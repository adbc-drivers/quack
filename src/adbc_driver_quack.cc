#if defined(_WIN32)
#define ADBC_EXPORT __declspec(dllexport)
#else
#define ADBC_EXPORT __attribute__((visibility("default")))
#endif

#include <arrow-adbc/adbc.h>
#include <duckdb.h>

#include "duckdb_arrow_stream.h"
#include "quack_uri.h"
#include "sql_escape.h"

#include <cstdlib>
#include <cstring>
#include <memory>
#include <new>
#include <string>
#include <utility>

extern "C" {
AdbcStatusCode AdbcDatabaseInit(AdbcDatabase* database, AdbcError* error);
AdbcStatusCode AdbcDatabaseNew(AdbcDatabase* database, AdbcError* error);
AdbcStatusCode AdbcDatabaseSetOption(AdbcDatabase* database, const char* key,
                                     const char* value, AdbcError* error);
AdbcStatusCode AdbcDatabaseRelease(AdbcDatabase* database, AdbcError* error);
AdbcStatusCode AdbcConnectionInit(AdbcConnection* connection, AdbcDatabase* database,
                                  AdbcError* error);
AdbcStatusCode AdbcConnectionNew(AdbcConnection* connection, AdbcError* error);
AdbcStatusCode AdbcConnectionRelease(AdbcConnection* connection, AdbcError* error);
AdbcStatusCode AdbcStatementBind(AdbcStatement* statement, ArrowArray* values,
                                 ArrowSchema* schema, AdbcError* error);
AdbcStatusCode AdbcStatementBindStream(AdbcStatement* statement,
                                       ArrowArrayStream* stream, AdbcError* error);
AdbcStatusCode AdbcStatementExecuteQuery(AdbcStatement* statement,
                                         ArrowArrayStream* out,
                                         int64_t* rows_affected,
                                         AdbcError* error);
AdbcStatusCode AdbcStatementNew(AdbcConnection* connection, AdbcStatement* statement,
                                AdbcError* error);
AdbcStatusCode AdbcStatementPrepare(AdbcStatement* statement, AdbcError* error);
AdbcStatusCode AdbcStatementRelease(AdbcStatement* statement, AdbcError* error);
AdbcStatusCode AdbcStatementSetSqlQuery(AdbcStatement* statement, const char* query,
                                        AdbcError* error);
}

namespace {

struct DatabaseState {
  std::string uri;
  adbc_driver_quack::ParsedQuackUri parsed_uri;
  bool initialized = false;
};

struct ConnectionState {
  duckdb_database database = nullptr;
  duckdb_connection connection = nullptr;
  bool initialized = false;
};

struct StatementState {
  ConnectionState* connection = nullptr;
  std::string sql;
};

struct DriverState {
  int version = ADBC_VERSION_1_1_0;
};

void ReleaseError(AdbcError* error) {
  if (error == nullptr) {
    return;
  }
  std::free(error->message);
  error->message = nullptr;
  error->vendor_code = 0;
  std::memset(error->sqlstate, 0, sizeof(error->sqlstate));
  error->release = nullptr;
  error->private_data = nullptr;
  error->private_driver = nullptr;
}

void ClearError(AdbcError* error) {
  if (error == nullptr) {
    return;
  }
  if (error->release != nullptr) {
    error->release(error);
    return;
  }
  error->message = nullptr;
  error->vendor_code = 0;
  std::memset(error->sqlstate, 0, sizeof(error->sqlstate));
  error->private_data = nullptr;
  error->private_driver = nullptr;
}

AdbcStatusCode SetError(AdbcError* error, AdbcStatusCode status, std::string message,
                        int32_t vendor_code = 0) {
  if (error == nullptr) {
    return status;
  }
  ClearError(error);
  char* error_message = static_cast<char*>(std::malloc(message.size() + 1));
  if (error_message == nullptr) {
    return status;
  }
  std::memcpy(error_message, message.c_str(), message.size() + 1);
  error->message = error_message;
  error->vendor_code = vendor_code;
  std::memset(error->sqlstate, 0, sizeof(error->sqlstate));
  error->release = ReleaseError;
  error->private_data = nullptr;
  error->private_driver = nullptr;
  return status;
}

AdbcStatusCode Ok(AdbcError* error) {
  ClearError(error);
  return ADBC_STATUS_OK;
}

AdbcStatusCode InvalidArgument(AdbcError* error, std::string message) {
  return SetError(error, ADBC_STATUS_INVALID_ARGUMENT, std::move(message));
}

AdbcStatusCode InvalidState(AdbcError* error, std::string message) {
  return SetError(error, ADBC_STATUS_INVALID_STATE, std::move(message));
}

AdbcStatusCode NotImplemented(AdbcError* error, std::string message) {
  return SetError(error, ADBC_STATUS_NOT_IMPLEMENTED, std::move(message));
}

AdbcStatusCode IoError(AdbcError* error, std::string message, int32_t vendor_code = 0) {
  return SetError(error, ADBC_STATUS_IO, std::move(message), vendor_code);
}

AdbcStatusCode ReleaseDriver(AdbcDriver* driver, AdbcError* error) {
  if (driver == nullptr) {
    return Ok(error);
  }
  delete static_cast<DriverState*>(driver->private_data);
  driver->private_data = nullptr;
  driver->release = nullptr;
  return Ok(error);
}

DatabaseState* GetDatabase(AdbcDatabase* database) {
  if (database == nullptr) {
    return nullptr;
  }
  return static_cast<DatabaseState*>(database->private_data);
}

ConnectionState* GetConnection(AdbcConnection* connection) {
  if (connection == nullptr) {
    return nullptr;
  }
  return static_cast<ConnectionState*>(connection->private_data);
}

StatementState* GetStatement(AdbcStatement* statement) {
  if (statement == nullptr) {
    return nullptr;
  }
  return static_cast<StatementState*>(statement->private_data);
}

void CloseConnectionState(ConnectionState* state) {
  if (state == nullptr) {
    return;
  }
  if (state->connection != nullptr) {
    duckdb_disconnect(&state->connection);
  }
  if (state->database != nullptr) {
    duckdb_close(&state->database);
  }
  state->initialized = false;
}

AdbcStatusCode RunDuckDbQuery(ConnectionState* state, const std::string& sql,
                              AdbcError* error) {
  duckdb_result result;
  const duckdb_state query_state =
      duckdb_query(state->connection, sql.c_str(), &result);
  if (query_state == DuckDBError) {
    const char* result_error = duckdb_result_error(&result);
    std::string message = result_error != nullptr ? result_error : "DuckDB query failed";
    const auto error_type = static_cast<int32_t>(duckdb_result_error_type(&result));
    duckdb_destroy_result(&result);
    return IoError(error, std::move(message), error_type);
  }
  duckdb_destroy_result(&result);
  return Ok(error);
}

AdbcStatusCode InitDriver(int version, void* raw_driver, AdbcError* error) {
  if (raw_driver == nullptr) {
    return InvalidArgument(error, "driver must not be null");
  }
  if (version != ADBC_VERSION_1_0_0 && version != ADBC_VERSION_1_1_0) {
    return NotImplemented(error, "unsupported ADBC driver version");
  }

  auto* driver = static_cast<AdbcDriver*>(raw_driver);
  auto* driver_state = new (std::nothrow) DriverState{version};
  if (driver_state == nullptr) {
    return SetError(error, ADBC_STATUS_UNKNOWN, "failed to allocate driver state");
  }
  driver->private_data = driver_state;
  driver->release = ReleaseDriver;

  driver->DatabaseInit = AdbcDatabaseInit;
  driver->DatabaseNew = AdbcDatabaseNew;
  driver->DatabaseSetOption = AdbcDatabaseSetOption;
  driver->DatabaseRelease = AdbcDatabaseRelease;

  driver->ConnectionInit = AdbcConnectionInit;
  driver->ConnectionNew = AdbcConnectionNew;
  driver->ConnectionRelease = AdbcConnectionRelease;

  driver->StatementBind = AdbcStatementBind;
  driver->StatementBindStream = AdbcStatementBindStream;
  driver->StatementExecuteQuery = AdbcStatementExecuteQuery;
  driver->StatementNew = AdbcStatementNew;
  driver->StatementPrepare = AdbcStatementPrepare;
  driver->StatementRelease = AdbcStatementRelease;
  driver->StatementSetSqlQuery = AdbcStatementSetSqlQuery;

  return Ok(error);
}

}  // namespace

extern "C" {

AdbcStatusCode AdbcDatabaseNew(AdbcDatabase* database, AdbcError* error) {
  if (database == nullptr) {
    return InvalidArgument(error, "database must not be null");
  }
  database->private_data = new DatabaseState();
  database->private_driver = nullptr;
  return Ok(error);
}

AdbcStatusCode AdbcDatabaseSetOption(AdbcDatabase* database, const char* key,
                                     const char* value, AdbcError* error) {
  DatabaseState* state = GetDatabase(database);
  if (state == nullptr) {
    return InvalidState(error, "database is not initialized");
  }
  if (key == nullptr || value == nullptr) {
    return InvalidArgument(error, "database option key and value must not be null");
  }
  if (std::strcmp(key, "uri") != 0) {
    return NotImplemented(error, "unsupported database option");
  }

  auto parsed = adbc_driver_quack::ParseQuackUri(value);
  if (!parsed.ok) {
    return InvalidArgument(error, parsed.error);
  }
  state->uri = value;
  state->parsed_uri = std::move(parsed);
  return Ok(error);
}

AdbcStatusCode AdbcDatabaseInit(AdbcDatabase* database, AdbcError* error) {
  DatabaseState* state = GetDatabase(database);
  if (state == nullptr) {
    return InvalidState(error, "database is not initialized");
  }
  if (!state->parsed_uri.ok) {
    return InvalidArgument(error, "database option 'uri' is required");
  }
  state->initialized = true;
  return Ok(error);
}

AdbcStatusCode AdbcDatabaseRelease(AdbcDatabase* database, AdbcError* error) {
  if (database == nullptr) {
    return Ok(error);
  }
  delete GetDatabase(database);
  database->private_data = nullptr;
  database->private_driver = nullptr;
  return Ok(error);
}

AdbcStatusCode AdbcConnectionNew(AdbcConnection* connection, AdbcError* error) {
  if (connection == nullptr) {
    return InvalidArgument(error, "connection must not be null");
  }
  connection->private_data = new ConnectionState();
  connection->private_driver = nullptr;
  return Ok(error);
}

AdbcStatusCode AdbcConnectionInit(AdbcConnection* connection, AdbcDatabase* database,
                                  AdbcError* error) {
  ConnectionState* connection_state = GetConnection(connection);
  DatabaseState* database_state = GetDatabase(database);
  if (connection_state == nullptr) {
    return InvalidState(error, "connection is not initialized");
  }
  if (database_state == nullptr || !database_state->initialized) {
    return InvalidState(error, "database is not initialized");
  }

  if (duckdb_open(nullptr, &connection_state->database) == DuckDBError) {
    return IoError(error, "failed to open local DuckDB client");
  }
  if (duckdb_connect(connection_state->database, &connection_state->connection) ==
      DuckDBError) {
    CloseConnectionState(connection_state);
    return IoError(error, "failed to connect local DuckDB client");
  }

  const std::string install = "INSTALL quack";
  AdbcStatusCode status = RunDuckDbQuery(connection_state, install, error);
  if (status != ADBC_STATUS_OK) {
    CloseConnectionState(connection_state);
    return status;
  }
  status = RunDuckDbQuery(connection_state, "LOAD quack", error);
  if (status != ADBC_STATUS_OK) {
    CloseConnectionState(connection_state);
    return status;
  }

  if (!database_state->parsed_uri.token.empty()) {
    const std::string create_secret =
        "CREATE SECRET (TYPE quack, TOKEN " +
        adbc_driver_quack::DuckDbSqlStringLiteral(database_state->parsed_uri.token) +
        ")";
    status = RunDuckDbQuery(connection_state, create_secret, error);
    if (status != ADBC_STATUS_OK) {
      CloseConnectionState(connection_state);
      return status;
    }
  }

  const std::string attach =
      "ATTACH " +
      adbc_driver_quack::DuckDbSqlStringLiteral(database_state->parsed_uri.endpoint) +
      " AS remote";
  status = RunDuckDbQuery(connection_state, attach, error);
  if (status != ADBC_STATUS_OK) {
    CloseConnectionState(connection_state);
    return status;
  }

  connection_state->initialized = true;
  return Ok(error);
}

AdbcStatusCode AdbcConnectionRelease(AdbcConnection* connection, AdbcError* error) {
  if (connection == nullptr) {
    return Ok(error);
  }
  ConnectionState* state = GetConnection(connection);
  CloseConnectionState(state);
  delete state;
  connection->private_data = nullptr;
  connection->private_driver = nullptr;
  return Ok(error);
}

AdbcStatusCode AdbcStatementNew(AdbcConnection* connection, AdbcStatement* statement,
                                AdbcError* error) {
  if (statement == nullptr) {
    return InvalidArgument(error, "statement must not be null");
  }
  ConnectionState* connection_state = GetConnection(connection);
  if (connection_state == nullptr || !connection_state->initialized) {
    return InvalidState(error, "connection is not initialized");
  }
  statement->private_data = new StatementState{connection_state, {}};
  statement->private_driver = nullptr;
  return Ok(error);
}

AdbcStatusCode AdbcStatementSetSqlQuery(AdbcStatement* statement, const char* query,
                                        AdbcError* error) {
  StatementState* state = GetStatement(statement);
  if (state == nullptr) {
    return InvalidState(error, "statement is not initialized");
  }
  if (query == nullptr) {
    return InvalidArgument(error, "SQL query must not be null");
  }
  state->sql = query;
  return Ok(error);
}

AdbcStatusCode AdbcStatementExecuteQuery(AdbcStatement* statement,
                                         ArrowArrayStream* out,
                                         int64_t* rows_affected,
                                         AdbcError* error) {
  StatementState* state = GetStatement(statement);
  if (state == nullptr) {
    return InvalidState(error, "statement is not initialized");
  }
  if (state->connection == nullptr || !state->connection->initialized) {
    return InvalidState(error, "connection is not initialized");
  }
  if (state->sql.empty()) {
    return InvalidState(error, "SQL query is not set");
  }
  const std::string remote_sql = adbc_driver_quack::BuildRemoteQuerySql(state->sql);
  if (out != nullptr) {
    const auto result = adbc_driver_quack::ExecuteDuckDbArrowQuery(
        state->connection->connection, remote_sql, out, rows_affected);
    if (result.status != ADBC_STATUS_OK) {
      return SetError(error, result.status, result.message, result.vendor_code);
    }
    return Ok(error);
  }

  const AdbcStatusCode status = RunDuckDbQuery(state->connection, remote_sql, error);
  if (status == ADBC_STATUS_OK && rows_affected != nullptr) {
    *rows_affected = -1;
  }
  return status;
}

AdbcStatusCode AdbcStatementPrepare(AdbcStatement*, AdbcError* error) {
  return NotImplemented(error, "parameterized statements are not implemented");
}

AdbcStatusCode AdbcStatementBind(AdbcStatement*, ArrowArray*, ArrowSchema*,
                                 AdbcError* error) {
  return NotImplemented(error, "parameter binding is not implemented");
}

AdbcStatusCode AdbcStatementBindStream(AdbcStatement*, ArrowArrayStream*,
                                       AdbcError* error) {
  return NotImplemented(error, "parameter binding is not implemented");
}

AdbcStatusCode AdbcStatementRelease(AdbcStatement* statement, AdbcError* error) {
  if (statement == nullptr) {
    return Ok(error);
  }
  delete GetStatement(statement);
  statement->private_data = nullptr;
  statement->private_driver = nullptr;
  return Ok(error);
}

ADBC_EXPORT AdbcStatusCode AdbcDriverInit(int version, void* driver,
                                          AdbcError* error) {
  return InitDriver(version, driver, error);
}

ADBC_EXPORT AdbcStatusCode AdbcDriverQuackInit(int version, void* driver,
                                               AdbcError* error) {
  return InitDriver(version, driver, error);
}

}  // extern "C"
