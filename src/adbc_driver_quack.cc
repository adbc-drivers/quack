// Copyright (c) 2026 ADBC Drivers Contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//         http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#if defined(_WIN32)
#define ADBC_EXPORT __declspec(dllexport)
#else
#define ADBC_EXPORT __attribute__((visibility("default")))
#endif

#include <arrow-adbc/adbc.h>
#include <duckdb.h>

#include <cstdlib>
#include <cstring>
#include <memory>
#include <new>
#include <string>
#include <utility>

#include "duckdb_arrow_stream.h"
#include "get_info_stream.h"
#include "quack_uri.h"
#include "sql_escape.h"

extern "C" {
AdbcStatusCode DriverDatabaseInit(AdbcDatabase* database, AdbcError* error);
AdbcStatusCode DriverDatabaseNew(AdbcDatabase* database, AdbcError* error);
AdbcStatusCode DriverDatabaseSetOption(AdbcDatabase* database, char const* key,
                                       char const* value, AdbcError* error);
AdbcStatusCode DriverDatabaseRelease(AdbcDatabase* database, AdbcError* error);
AdbcStatusCode DriverConnectionInit(AdbcConnection* connection,
                                    AdbcDatabase* database, AdbcError* error);
AdbcStatusCode DriverConnectionGetInfo(AdbcConnection* connection,
                                       uint32_t const* info_codes,
                                       size_t info_codes_length,
                                       ArrowArrayStream* out, AdbcError* error);
AdbcStatusCode DriverConnectionNew(AdbcConnection* connection,
                                   AdbcError* error);
AdbcStatusCode DriverConnectionRelease(AdbcConnection* connection,
                                       AdbcError* error);
AdbcStatusCode DriverStatementBind(AdbcStatement* statement, ArrowArray* values,
                                   ArrowSchema* schema, AdbcError* error);
AdbcStatusCode DriverStatementBindStream(AdbcStatement* statement,
                                         ArrowArrayStream* stream,
                                         AdbcError* error);
AdbcStatusCode DriverStatementExecuteQuery(AdbcStatement* statement,
                                           ArrowArrayStream* out,
                                           int64_t* rows_affected,
                                           AdbcError* error);
AdbcStatusCode DriverStatementNew(AdbcConnection* connection,
                                  AdbcStatement* statement, AdbcError* error);
AdbcStatusCode DriverStatementPrepare(AdbcStatement* statement,
                                      AdbcError* error);
AdbcStatusCode DriverStatementRelease(AdbcStatement* statement,
                                      AdbcError* error);
AdbcStatusCode DriverStatementSetSqlQuery(AdbcStatement* statement,
                                          char const* query, AdbcError* error);
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
  bool const preserve_private_driver =
      error->vendor_code == ADBC_ERROR_VENDOR_CODE_PRIVATE_DATA;
  AdbcDriver* const private_driver = error->private_driver;
  if (error->release != nullptr) {
    error->release(error);
  } else {
    error->message = nullptr;
    error->vendor_code = 0;
    std::memset(error->sqlstate, 0, sizeof(error->sqlstate));
    error->private_data = nullptr;
    error->private_driver = nullptr;
  }
  if (preserve_private_driver) {
    error->vendor_code = ADBC_ERROR_VENDOR_CODE_PRIVATE_DATA;
    error->private_driver = private_driver;
  }
}

AdbcStatusCode SetError(AdbcError* error, AdbcStatusCode status,
                        std::string message, int32_t vendor_code = 0) {
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

AdbcStatusCode InvalidData(AdbcError* error, std::string message) {
  return SetError(error, ADBC_STATUS_INVALID_DATA, std::move(message));
}

AdbcStatusCode InvalidState(AdbcError* error, std::string message) {
  return SetError(error, ADBC_STATUS_INVALID_STATE, std::move(message));
}

AdbcStatusCode NotImplemented(AdbcError* error, std::string message) {
  return SetError(error, ADBC_STATUS_NOT_IMPLEMENTED, std::move(message));
}

AdbcStatusCode IoError(AdbcError* error, std::string message,
                       int32_t vendor_code = 0) {
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

AdbcStatusCode RunDuckDbQuery(ConnectionState* state, std::string const& sql,
                              AdbcError* error) {
  duckdb_result result;
  duckdb_state const query_state =
      duckdb_query(state->connection, sql.c_str(), &result);
  if (query_state == DuckDBError) {
    char const* result_error = duckdb_result_error(&result);
    std::string message =
        result_error != nullptr ? result_error : "DuckDB query failed";
    auto const error_type =
        static_cast<int32_t>(duckdb_result_error_type(&result));
    duckdb_destroy_result(&result);
    return IoError(error, std::move(message), error_type);
  }
  duckdb_destroy_result(&result);
  return Ok(error);
}

AdbcStatusCode QueryRemoteVendorVersion(ConnectionState* state,
                                        std::string* remote_vendor_version,
                                        AdbcError* error) {
  duckdb_result result;
  std::string const sql =
      adbc_driver_quack::BuildRemoteQuerySql("SELECT version()");
  duckdb_state const query_state =
      duckdb_query(state->connection, sql.c_str(), &result);
  if (query_state == DuckDBError) {
    char const* result_error = duckdb_result_error(&result);
    std::string message =
        result_error != nullptr ? result_error : "DuckDB query failed";
    auto const error_type =
        static_cast<int32_t>(duckdb_result_error_type(&result));
    duckdb_destroy_result(&result);
    return IoError(error, std::move(message), error_type);
  }

  if (duckdb_column_count(&result) != 1 || duckdb_row_count(&result) != 1 ||
      duckdb_column_type(&result, 0) != DUCKDB_TYPE_VARCHAR ||
      duckdb_value_is_null(&result, 0, 0)) {
    duckdb_destroy_result(&result);
    return InvalidData(error,
                       "remote DuckDB version query did not return one string "
                       "value");
  }

  char* value = duckdb_value_varchar(&result, 0, 0);
  if (value == nullptr) {
    duckdb_destroy_result(&result);
    return InvalidData(error,
                       "remote DuckDB version query did not return one string "
                       "value");
  }

  *remote_vendor_version = value;
  duckdb_free(value);
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
  if (version >= ADBC_VERSION_1_1_0) {
    std::memset(driver, 0, ADBC_DRIVER_1_1_0_SIZE);
  } else {
    std::memset(driver, 0, ADBC_DRIVER_1_0_0_SIZE);
  }

  auto* driver_state = new (std::nothrow) DriverState{version};
  if (driver_state == nullptr) {
    return SetError(error, ADBC_STATUS_UNKNOWN,
                    "failed to allocate driver state");
  }
  driver->private_data = driver_state;
  driver->release = ReleaseDriver;

  driver->DatabaseInit = DriverDatabaseInit;
  driver->DatabaseNew = DriverDatabaseNew;
  driver->DatabaseSetOption = DriverDatabaseSetOption;
  driver->DatabaseRelease = DriverDatabaseRelease;

  driver->ConnectionInit = DriverConnectionInit;
  driver->ConnectionGetInfo = DriverConnectionGetInfo;
  driver->ConnectionNew = DriverConnectionNew;
  driver->ConnectionRelease = DriverConnectionRelease;

  driver->StatementBind = DriverStatementBind;
  driver->StatementBindStream = DriverStatementBindStream;
  driver->StatementExecuteQuery = DriverStatementExecuteQuery;
  driver->StatementNew = DriverStatementNew;
  driver->StatementPrepare = DriverStatementPrepare;
  driver->StatementRelease = DriverStatementRelease;
  driver->StatementSetSqlQuery = DriverStatementSetSqlQuery;

  return Ok(error);
}

}  // namespace

extern "C" {

AdbcStatusCode DriverDatabaseNew(AdbcDatabase* database, AdbcError* error) {
  if (database == nullptr) {
    return InvalidArgument(error, "database must not be null");
  }
  database->private_data = new DatabaseState();
  return Ok(error);
}

AdbcStatusCode DriverDatabaseSetOption(AdbcDatabase* database, char const* key,
                                       char const* value, AdbcError* error) {
  DatabaseState* state = GetDatabase(database);
  if (state == nullptr) {
    return InvalidState(error, "database is not initialized");
  }
  if (key == nullptr || value == nullptr) {
    return InvalidArgument(error,
                           "database option key and value must not be null");
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

AdbcStatusCode DriverDatabaseInit(AdbcDatabase* database, AdbcError* error) {
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

AdbcStatusCode DriverDatabaseRelease(AdbcDatabase* database, AdbcError* error) {
  if (database == nullptr) {
    return Ok(error);
  }
  delete GetDatabase(database);
  database->private_data = nullptr;
  return Ok(error);
}

AdbcStatusCode DriverConnectionNew(AdbcConnection* connection,
                                   AdbcError* error) {
  if (connection == nullptr) {
    return InvalidArgument(error, "connection must not be null");
  }
  connection->private_data = new ConnectionState();
  return Ok(error);
}

AdbcStatusCode DriverConnectionInit(AdbcConnection* connection,
                                    AdbcDatabase* database, AdbcError* error) {
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
  if (duckdb_connect(connection_state->database,
                     &connection_state->connection) == DuckDBError) {
    CloseConnectionState(connection_state);
    return IoError(error, "failed to connect local DuckDB client");
  }

  AdbcStatusCode status = RunDuckDbQuery(connection_state, "LOAD quack", error);
  if (status != ADBC_STATUS_OK) {
    CloseConnectionState(connection_state);
    return status;
  }

  std::string attach = "ATTACH " +
                       adbc_driver_quack::DuckDbSqlStringLiteral(
                           database_state->parsed_uri.endpoint) +
                       " AS remote (disable_ssl true";
  if (!database_state->parsed_uri.token.empty()) {
    attach += ", token ";
    attach += adbc_driver_quack::DuckDbSqlStringLiteral(
        database_state->parsed_uri.token);
  }
  attach += ")";
  status = RunDuckDbQuery(connection_state, attach, error);
  if (status != ADBC_STATUS_OK) {
    CloseConnectionState(connection_state);
    return status;
  }

  connection_state->initialized = true;
  return Ok(error);
}

AdbcStatusCode DriverConnectionRelease(AdbcConnection* connection,
                                       AdbcError* error) {
  if (connection == nullptr) {
    return Ok(error);
  }
  ConnectionState* state = GetConnection(connection);
  CloseConnectionState(state);
  delete state;
  connection->private_data = nullptr;
  return Ok(error);
}

AdbcStatusCode DriverConnectionGetInfo(AdbcConnection* connection,
                                       uint32_t const* info_codes,
                                       size_t info_codes_length,
                                       ArrowArrayStream* out,
                                       AdbcError* error) {
  ConnectionState* state = GetConnection(connection);
  if (state == nullptr || !state->initialized) {
    return InvalidState(error, "connection is not initialized");
  }

  std::string remote_vendor_version;
  AdbcStatusCode status =
      QueryRemoteVendorVersion(state, &remote_vendor_version, error);
  if (status != ADBC_STATUS_OK) {
    return status;
  }

  auto const result = adbc_driver_quack::BuildGetInfoStream(
      remote_vendor_version, info_codes, info_codes_length, out);
  if (result.status != ADBC_STATUS_OK) {
    return SetError(error, result.status, result.message);
  }
  return Ok(error);
}

AdbcStatusCode DriverStatementNew(AdbcConnection* connection,
                                  AdbcStatement* statement, AdbcError* error) {
  if (statement == nullptr) {
    return InvalidArgument(error, "statement must not be null");
  }
  ConnectionState* connection_state = GetConnection(connection);
  if (connection_state == nullptr || !connection_state->initialized) {
    return InvalidState(error, "connection is not initialized");
  }
  statement->private_data = new StatementState{connection_state, {}};
  return Ok(error);
}

AdbcStatusCode DriverStatementSetSqlQuery(AdbcStatement* statement,
                                          char const* query, AdbcError* error) {
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

AdbcStatusCode DriverStatementExecuteQuery(AdbcStatement* statement,
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
  std::string const remote_sql =
      adbc_driver_quack::BuildRemoteQuerySql(state->sql);
  if (out != nullptr) {
    auto const result = adbc_driver_quack::ExecuteDuckDbArrowQuery(
        state->connection->connection, remote_sql, out, rows_affected);
    if (result.status != ADBC_STATUS_OK) {
      return SetError(error, result.status, result.message, result.vendor_code);
    }
    return Ok(error);
  }

  AdbcStatusCode const status =
      RunDuckDbQuery(state->connection, remote_sql, error);
  if (status == ADBC_STATUS_OK && rows_affected != nullptr) {
    *rows_affected = -1;
  }
  return status;
}

AdbcStatusCode DriverStatementPrepare(AdbcStatement*, AdbcError* error) {
  return NotImplemented(error, "parameterized statements are not implemented");
}

AdbcStatusCode DriverStatementBind(AdbcStatement*, ArrowArray*, ArrowSchema*,
                                   AdbcError* error) {
  return NotImplemented(error, "parameter binding is not implemented");
}

AdbcStatusCode DriverStatementBindStream(AdbcStatement*, ArrowArrayStream*,
                                         AdbcError* error) {
  return NotImplemented(error, "parameter binding is not implemented");
}

AdbcStatusCode DriverStatementRelease(AdbcStatement* statement,
                                      AdbcError* error) {
  if (statement == nullptr) {
    return Ok(error);
  }
  delete GetStatement(statement);
  statement->private_data = nullptr;
  return Ok(error);
}

ADBC_EXPORT AdbcStatusCode AdbcDatabaseNew(AdbcDatabase* database,
                                           AdbcError* error) {
  return DriverDatabaseNew(database, error);
}

ADBC_EXPORT AdbcStatusCode AdbcDatabaseSetOption(AdbcDatabase* database,
                                                 char const* key,
                                                 char const* value,
                                                 AdbcError* error) {
  return DriverDatabaseSetOption(database, key, value, error);
}

ADBC_EXPORT AdbcStatusCode AdbcDatabaseInit(AdbcDatabase* database,
                                            AdbcError* error) {
  return DriverDatabaseInit(database, error);
}

ADBC_EXPORT AdbcStatusCode AdbcDatabaseRelease(AdbcDatabase* database,
                                               AdbcError* error) {
  return DriverDatabaseRelease(database, error);
}

ADBC_EXPORT AdbcStatusCode AdbcConnectionNew(AdbcConnection* connection,
                                             AdbcError* error) {
  return DriverConnectionNew(connection, error);
}

ADBC_EXPORT AdbcStatusCode AdbcConnectionInit(AdbcConnection* connection,
                                              AdbcDatabase* database,
                                              AdbcError* error) {
  return DriverConnectionInit(connection, database, error);
}

ADBC_EXPORT AdbcStatusCode AdbcConnectionRelease(AdbcConnection* connection,
                                                 AdbcError* error) {
  return DriverConnectionRelease(connection, error);
}

ADBC_EXPORT AdbcStatusCode AdbcConnectionGetInfo(AdbcConnection* connection,
                                                 uint32_t const* info_codes,
                                                 size_t info_codes_length,
                                                 ArrowArrayStream* out,
                                                 AdbcError* error) {
  return DriverConnectionGetInfo(connection, info_codes, info_codes_length, out,
                                 error);
}

ADBC_EXPORT AdbcStatusCode AdbcStatementNew(AdbcConnection* connection,
                                            AdbcStatement* statement,
                                            AdbcError* error) {
  return DriverStatementNew(connection, statement, error);
}

ADBC_EXPORT AdbcStatusCode AdbcStatementSetSqlQuery(AdbcStatement* statement,
                                                    char const* query,
                                                    AdbcError* error) {
  return DriverStatementSetSqlQuery(statement, query, error);
}

ADBC_EXPORT AdbcStatusCode AdbcStatementExecuteQuery(AdbcStatement* statement,
                                                     ArrowArrayStream* out,
                                                     int64_t* rows_affected,
                                                     AdbcError* error) {
  return DriverStatementExecuteQuery(statement, out, rows_affected, error);
}

ADBC_EXPORT AdbcStatusCode AdbcStatementPrepare(AdbcStatement* statement,
                                                AdbcError* error) {
  return DriverStatementPrepare(statement, error);
}

ADBC_EXPORT AdbcStatusCode AdbcStatementBind(AdbcStatement* statement,
                                             ArrowArray* values,
                                             ArrowSchema* schema,
                                             AdbcError* error) {
  return DriverStatementBind(statement, values, schema, error);
}

ADBC_EXPORT AdbcStatusCode AdbcStatementBindStream(AdbcStatement* statement,
                                                   ArrowArrayStream* stream,
                                                   AdbcError* error) {
  return DriverStatementBindStream(statement, stream, error);
}

ADBC_EXPORT AdbcStatusCode AdbcStatementRelease(AdbcStatement* statement,
                                                AdbcError* error) {
  return DriverStatementRelease(statement, error);
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
