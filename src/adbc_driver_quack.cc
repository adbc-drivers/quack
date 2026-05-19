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

#include <cctype>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <new>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

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
AdbcStatusCode DriverConnectionGetOption(AdbcConnection* connection,
                                         char const* key, char* value,
                                         size_t* length, AdbcError* error);
AdbcStatusCode DriverConnectionGetObjects(
    AdbcConnection* connection, int depth, char const* catalog,
    char const* db_schema, char const* table_name, char const** table_type,
    char const* column_name, ArrowArrayStream* out, AdbcError* error);
AdbcStatusCode DriverConnectionNew(AdbcConnection* connection,
                                   AdbcError* error);
AdbcStatusCode DriverConnectionRelease(AdbcConnection* connection,
                                       AdbcError* error);
AdbcStatusCode DriverConnectionSetOption(AdbcConnection* connection,
                                         char const* key, char const* value,
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
AdbcStatusCode DriverStatementSetOption(AdbcStatement* statement,
                                        char const* key, char const* value,
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
  uint64_t next_ingest_id = 0;
  bool initialized = false;
};

struct StatementState {
  ConnectionState* connection = nullptr;
  std::string sql;
  std::string ingest_target_table;
  std::string ingest_target_catalog;
  std::string ingest_target_schema;
  std::string ingest_mode = ADBC_INGEST_OPTION_MODE_CREATE;
  bool ingest_temporary = false;
  ArrowArrayStream bound_stream = {};
  bool has_bound_stream = false;
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

AdbcStatusCode NotFound(AdbcError* error, std::string message,
                        int32_t vendor_code = 0) {
  return SetError(error, ADBC_STATUS_NOT_FOUND, std::move(message),
                  vendor_code);
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

void ReleaseBoundStream(StatementState* state) {
  if (state == nullptr || !state->has_bound_stream) {
    return;
  }
  if (state->bound_stream.release != nullptr) {
    state->bound_stream.release(&state->bound_stream);
  }
  state->bound_stream = {};
  state->has_bound_stream = false;
}

std::string QuoteIdentifier(std::string_view value) {
  std::string escaped;
  escaped.reserve(value.size() + 2);
  escaped.push_back('"');
  for (char c : value) {
    if (c == '"') {
      escaped.push_back('"');
    }
    escaped.push_back(c);
  }
  escaped.push_back('"');
  return escaped;
}

std::string QualifiedRemoteTableName(StatementState const* state) {
  std::string table = "remote.";
  table += QuoteIdentifier(state->ingest_target_schema.empty()
                               ? std::string_view{"main"}
                               : std::string_view{state->ingest_target_schema});
  table += ".";
  table += QuoteIdentifier(state->ingest_target_table);
  return table;
}

std::string QualifiedServerTableName(StatementState const* state) {
  std::string table;
  if (!state->ingest_target_schema.empty()) {
    table += QuoteIdentifier(state->ingest_target_schema);
    table += ".";
  }
  table += QuoteIdentifier(state->ingest_target_table);
  return table;
}

std::string LikeFilter(char const* value) {
  if (value == nullptr) {
    return "'%'";
  }
  return adbc_driver_quack::DuckDbSqlStringLiteral(value);
}

AdbcStatusCode BuildTableTypeCondition(char const** table_type,
                                       std::string* condition,
                                       AdbcError* error) {
  condition->clear();
  if (table_type == nullptr || table_type[0] == nullptr) {
    return Ok(error);
  }

  *condition = " AND table_type IN (";
  for (int i = 0; table_type[i] != nullptr; ++i) {
    if (std::strcmp(table_type[i], "LOCAL TABLE") != 0 &&
        std::strcmp(table_type[i], "BASE TABLE") != 0 &&
        std::strcmp(table_type[i], "VIEW") != 0) {
      return InvalidArgument(error,
                             "table type must be \"LOCAL TABLE\", \"BASE "
                             "TABLE\" or \"VIEW\"");
    }
    if (i > 0) {
      *condition += ", ";
    }
    *condition += LikeFilter(table_type[i]);
  }
  *condition += ")";
  return Ok(error);
}

std::string BuildGetObjectsQuery(int depth, std::string const& catalog_filter,
                                 std::string const& db_schema_filter,
                                 std::string const& table_name_filter,
                                 std::string const& table_type_condition,
                                 std::string const& column_name_filter) {
  switch (depth) {
    case ADBC_OBJECT_DEPTH_CATALOGS:
      return R"(
SELECT
  catalog_name,
  []::STRUCT(
    db_schema_name VARCHAR,
    db_schema_tables STRUCT(
      table_name VARCHAR,
      table_type VARCHAR,
      table_columns STRUCT(
        column_name VARCHAR,
        ordinal_position INTEGER,
        remarks VARCHAR,
        xdbc_data_type SMALLINT,
        xdbc_type_name VARCHAR,
        xdbc_column_size INTEGER,
        xdbc_decimal_digits SMALLINT,
        xdbc_num_prec_radix SMALLINT,
        xdbc_nullable SMALLINT,
        xdbc_column_def VARCHAR,
        xdbc_sql_data_type SMALLINT,
        xdbc_datetime_sub SMALLINT,
        xdbc_char_octet_length INTEGER,
        xdbc_is_nullable VARCHAR,
        xdbc_scope_catalog VARCHAR,
        xdbc_scope_schema VARCHAR,
        xdbc_scope_table VARCHAR,
        xdbc_is_autoincrement BOOLEAN,
        xdbc_is_generatedcolumn BOOLEAN
      )[],
      table_constraints STRUCT(
        constraint_name VARCHAR,
        constraint_type VARCHAR,
        constraint_column_names VARCHAR[],
        constraint_column_usage STRUCT(fk_catalog VARCHAR, fk_db_schema VARCHAR, fk_table VARCHAR, fk_column_name VARCHAR)[]
      )[]
    )[]
  )[] catalog_db_schemas
FROM information_schema.schemata
WHERE catalog_name LIKE )" +
             catalog_filter +
             R"(
GROUP BY catalog_name
)";
    case ADBC_OBJECT_DEPTH_DB_SCHEMAS:
      return R"(
WITH db_schemas AS (
  SELECT
    catalog_name,
    schema_name,
  FROM information_schema.schemata
  WHERE schema_name LIKE )" +
             db_schema_filter +
             R"(
)

SELECT
  catalog_name,
  COALESCE(LIST({
    db_schema_name: schema_name,
    db_schema_tables: []::STRUCT(
      table_name VARCHAR,
      table_type VARCHAR,
      table_columns STRUCT(
        column_name VARCHAR,
        ordinal_position INTEGER,
        remarks VARCHAR,
        xdbc_data_type SMALLINT,
        xdbc_type_name VARCHAR,
        xdbc_column_size INTEGER,
        xdbc_decimal_digits SMALLINT,
        xdbc_num_prec_radix SMALLINT,
        xdbc_nullable SMALLINT,
        xdbc_column_def VARCHAR,
        xdbc_sql_data_type SMALLINT,
        xdbc_datetime_sub SMALLINT,
        xdbc_char_octet_length INTEGER,
        xdbc_is_nullable VARCHAR,
        xdbc_scope_catalog VARCHAR,
        xdbc_scope_schema VARCHAR,
        xdbc_scope_table VARCHAR,
        xdbc_is_autoincrement BOOLEAN,
        xdbc_is_generatedcolumn BOOLEAN
      )[],
      table_constraints STRUCT(
        constraint_name VARCHAR,
        constraint_type VARCHAR,
        constraint_column_names VARCHAR[],
        constraint_column_usage STRUCT(fk_catalog VARCHAR, fk_db_schema VARCHAR, fk_table VARCHAR, fk_column_name VARCHAR)[]
      )[]
    )[],
  }) FILTER (dbs.schema_name is not null), []) catalog_db_schemas
FROM information_schema.schemata
LEFT JOIN db_schemas dbs
USING (catalog_name, schema_name)
WHERE catalog_name LIKE )" +
             catalog_filter +
             R"(
GROUP BY catalog_name
)";
    case ADBC_OBJECT_DEPTH_TABLES:
      return R"(
WITH tables AS (
  SELECT
    table_catalog catalog_name,
    table_schema schema_name,
    LIST({
      table_name: table_name,
      table_type: table_type,
      table_columns: []::STRUCT(
        column_name VARCHAR,
        ordinal_position INTEGER,
        remarks VARCHAR,
        xdbc_data_type SMALLINT,
        xdbc_type_name VARCHAR,
        xdbc_column_size INTEGER,
        xdbc_decimal_digits SMALLINT,
        xdbc_num_prec_radix SMALLINT,
        xdbc_nullable SMALLINT,
        xdbc_column_def VARCHAR,
        xdbc_sql_data_type SMALLINT,
        xdbc_datetime_sub SMALLINT,
        xdbc_char_octet_length INTEGER,
        xdbc_is_nullable VARCHAR,
        xdbc_scope_catalog VARCHAR,
        xdbc_scope_schema VARCHAR,
        xdbc_scope_table VARCHAR,
        xdbc_is_autoincrement BOOLEAN,
        xdbc_is_generatedcolumn BOOLEAN
      )[],
      table_constraints: []::STRUCT(
        constraint_name VARCHAR,
        constraint_type VARCHAR,
        constraint_column_names VARCHAR[],
        constraint_column_usage STRUCT(fk_catalog VARCHAR, fk_db_schema VARCHAR, fk_table VARCHAR, fk_column_name VARCHAR)[]
      )[],
    }) db_schema_tables
  FROM information_schema.tables
  WHERE table_name LIKE )" +
             table_name_filter + table_type_condition + R"(
  GROUP BY table_catalog, table_schema
),
db_schemas AS (
  SELECT
    catalog_name,
    schema_name,
    COALESCE(db_schema_tables, []) AS db_schema_tables,
  FROM information_schema.schemata
  LEFT JOIN tables
  USING (catalog_name, schema_name)
  WHERE schema_name LIKE )" +
             db_schema_filter +
             R"(
)

SELECT
  catalog_name,
  COALESCE(LIST({
    db_schema_name: schema_name,
    db_schema_tables: db_schema_tables,
  }) FILTER (dbs.schema_name is not null), []) catalog_db_schemas
FROM information_schema.schemata
LEFT JOIN db_schemas dbs
USING (catalog_name, schema_name)
WHERE catalog_name LIKE )" +
             catalog_filter +
             R"(
GROUP BY catalog_name
)";
    case ADBC_OBJECT_DEPTH_COLUMNS:
      return R"(
WITH columns AS (
  SELECT
    table_catalog,
    table_schema,
    table_name,
    LIST({
      column_name: column_name,
      ordinal_position: ordinal_position,
      remarks: '',
      xdbc_data_type: NULL::SMALLINT,
      xdbc_type_name: NULL::VARCHAR,
      xdbc_column_size: NULL::INTEGER,
      xdbc_decimal_digits: NULL::SMALLINT,
      xdbc_num_prec_radix: NULL::SMALLINT,
      xdbc_nullable: NULL::SMALLINT,
      xdbc_column_def: NULL::VARCHAR,
      xdbc_sql_data_type: NULL::SMALLINT,
      xdbc_datetime_sub: NULL::SMALLINT,
      xdbc_char_octet_length: NULL::INTEGER,
      xdbc_is_nullable: NULL::VARCHAR,
      xdbc_scope_catalog: NULL::VARCHAR,
      xdbc_scope_schema: NULL::VARCHAR,
      xdbc_scope_table: NULL::VARCHAR,
      xdbc_is_autoincrement: NULL::BOOLEAN,
      xdbc_is_generatedcolumn: NULL::BOOLEAN,
    }) table_columns
  FROM information_schema.columns
  WHERE column_name LIKE )" +
             column_name_filter +
             R"(
  GROUP BY table_catalog, table_schema, table_name
),
constraints AS (
  SELECT
    database_name AS table_catalog,
    schema_name AS table_schema,
    table_name,
    LIST({
      constraint_name: constraint_name,
      constraint_type: constraint_type,
      constraint_column_names: constraint_column_names,
      constraint_column_usage: list_transform(
        referenced_column_names,
        lambda name: {
          fk_catalog: database_name,
          fk_db_schema: schema_name,
          fk_table: referenced_table,
          fk_column_name: name,
        }
      )
    }) table_constraints
  FROM duckdb_constraints()
  WHERE
    constraint_type NOT IN ('NOT NULL') AND
    list_has_any(
      constraint_column_names,
      list_filter(
        constraint_column_names,
        lambda name: name LIKE )" +
             column_name_filter +
             R"(
      )
    )
  GROUP BY database_name, schema_name, table_name
),
tables AS (
  SELECT
    table_catalog catalog_name,
    table_schema schema_name,
    LIST({
      table_name: table_name,
      table_type: table_type,
      table_columns: COALESCE(table_columns, []),
      table_constraints: COALESCE(table_constraints, []),
    }) db_schema_tables
  FROM information_schema.tables
  LEFT JOIN columns
  USING (table_catalog, table_schema, table_name)
  LEFT JOIN constraints
  USING (table_catalog, table_schema, table_name)
  WHERE table_name LIKE )" +
             table_name_filter + table_type_condition + R"(
  GROUP BY table_catalog, table_schema
),
db_schemas AS (
  SELECT
    catalog_name,
    schema_name,
    COALESCE(db_schema_tables, []) AS db_schema_tables,
  FROM information_schema.schemata
  LEFT JOIN tables
  USING (catalog_name, schema_name)
  WHERE schema_name LIKE )" +
             db_schema_filter +
             R"(
)

SELECT
  catalog_name,
  COALESCE(LIST({
    db_schema_name: schema_name,
    db_schema_tables: db_schema_tables,
  }) FILTER (dbs.schema_name is not null), []) catalog_db_schemas
FROM information_schema.schemata
LEFT JOIN db_schemas dbs
USING (catalog_name, schema_name)
WHERE catalog_name LIKE )" +
             catalog_filter +
             R"(
GROUP BY catalog_name
)";
    default:
      return {};
  }
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

std::string Lowercase(std::string_view value) {
  std::string lowered(value);
  for (char& c : lowered) {
    c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
  }
  return lowered;
}

bool IsNotFoundDuckDbError(duckdb_error_type error_type,
                           std::string_view message) {
  std::string const lower_message = Lowercase(message);
  return (error_type == DUCKDB_ERROR_CATALOG &&
          lower_message.find("not found") != std::string::npos) ||
         lower_message.find("does not exist") != std::string::npos ||
         lower_message.find("not found") != std::string::npos ||
         lower_message.find("no such table") != std::string::npos ||
         lower_message.find("no catalog + schema named") != std::string::npos;
}

AdbcStatusCode RunDuckDbQueryAllowNotFound(ConnectionState* state,
                                           std::string const& sql,
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
    auto const result_error_type = duckdb_result_error_type(&result);
    duckdb_destroy_result(&result);
    if (IsNotFoundDuckDbError(result_error_type, message)) {
      return NotFound(error, std::move(message), error_type);
    }
    return IoError(error, std::move(message), error_type);
  }
  duckdb_destroy_result(&result);
  return Ok(error);
}

AdbcStatusCode RunRemoteQuery(ConnectionState* state, std::string const& sql,
                              AdbcError* error) {
  return RunDuckDbQuery(state, adbc_driver_quack::BuildRemoteQuerySql(sql),
                        error);
}

AdbcStatusCode RunRemoteQueryAllowNotFound(ConnectionState* state,
                                           std::string const& sql,
                                           AdbcError* error) {
  return RunDuckDbQueryAllowNotFound(
      state, adbc_driver_quack::BuildRemoteQuerySql(sql), error);
}

AdbcStatusCode GetColumnDefinitions(ConnectionState* state,
                                    std::string const& table_name,
                                    std::vector<std::string>* definitions,
                                    AdbcError* error) {
  duckdb_result result;
  std::string const sql =
      "SELECT name, type FROM pragma_table_info(" +
      adbc_driver_quack::DuckDbSqlStringLiteral(table_name) + ") ORDER BY cid";
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

  idx_t const column_count = duckdb_row_count(&result);
  if (column_count == 0) {
    duckdb_destroy_result(&result);
    return InvalidData(error, "bulk ingest input has no columns");
  }

  definitions->clear();
  definitions->reserve(static_cast<size_t>(column_count));
  for (idx_t row = 0; row < column_count; row++) {
    if (duckdb_value_is_null(&result, 0, row) ||
        duckdb_value_is_null(&result, 1, row)) {
      duckdb_destroy_result(&result);
      return InvalidData(error,
                         "bulk ingest input has invalid column metadata");
    }
    char* name = duckdb_value_varchar(&result, 0, row);
    char* type = duckdb_value_varchar(&result, 1, row);
    if (name == nullptr || type == nullptr) {
      duckdb_free(name);
      duckdb_free(type);
      duckdb_destroy_result(&result);
      return InvalidData(error,
                         "bulk ingest input has invalid column metadata");
    }
    definitions->push_back(QuoteIdentifier(name) + " " + type);
    duckdb_free(name);
    duckdb_free(type);
  }
  duckdb_destroy_result(&result);
  return Ok(error);
}

std::string JoinColumnDefinitions(std::vector<std::string> const& definitions) {
  std::string result;
  for (size_t i = 0; i < definitions.size(); i++) {
    if (i != 0) {
      result += ", ";
    }
    result += definitions[i];
  }
  return result;
}

AdbcStatusCode ExecuteBulkIngest(StatementState* state, int64_t* rows_affected,
                                 AdbcError* error) {
  if (state->ingest_target_table.empty()) {
    return InvalidState(error, "bulk ingest target table is not set");
  }
  if (!state->ingest_target_catalog.empty() &&
      state->ingest_target_catalog != "remote") {
    return NotImplemented(error, "bulk ingest catalogs are not implemented");
  }
  if (state->ingest_temporary) {
    return NotImplemented(error, "temporary bulk ingest is not implemented");
  }
  if (!state->has_bound_stream) {
    return InvalidState(error, "bulk ingest data is not bound");
  }

  uint64_t const ingest_id = state->connection->next_ingest_id++;
  std::string const view_name =
      "adbc_quack_ingest_view_" + std::to_string(ingest_id);
  std::string const data_name =
      "adbc_quack_ingest_data_" + std::to_string(ingest_id);
  std::string const quoted_view = QuoteIdentifier(view_name);
  std::string const quoted_data = QuoteIdentifier(data_name);
  if (duckdb_arrow_scan(state->connection->connection, view_name.c_str(),
                        reinterpret_cast<duckdb_arrow_stream>(
                            &state->bound_stream)) == DuckDBError) {
    ReleaseBoundStream(state);
    return IoError(error, "failed to scan Arrow stream for bulk ingest");
  }

  AdbcStatusCode status = RunDuckDbQuery(
      state->connection,
      "CREATE TEMP TABLE " + quoted_data + " AS SELECT * FROM " + quoted_view,
      error);
  RunDuckDbQuery(state->connection, "DROP VIEW IF EXISTS " + quoted_view,
                 nullptr);
  ReleaseBoundStream(state);
  if (status != ADBC_STATUS_OK) {
    RunDuckDbQuery(state->connection, "DROP TABLE IF EXISTS " + quoted_data,
                   nullptr);
    return status;
  }

  std::string const target = QualifiedRemoteTableName(state);
  std::string const server_target = QualifiedServerTableName(state);
  std::vector<std::string> column_definitions;
  status = GetColumnDefinitions(state->connection, data_name,
                                &column_definitions, error);
  if (status != ADBC_STATUS_OK) {
    RunDuckDbQuery(state->connection, "DROP TABLE IF EXISTS " + quoted_data,
                   nullptr);
    return status;
  }
  std::string const create_columns =
      " (" + JoinColumnDefinitions(column_definitions) + ")";
  if (state->ingest_mode == ADBC_INGEST_OPTION_MODE_CREATE) {
    status =
        RunRemoteQuery(state->connection,
                       "CREATE TABLE " + server_target + create_columns, error);
    if (status == ADBC_STATUS_OK) {
      status = RunDuckDbQuery(state->connection,
                              "SELECT * FROM quack_clear_cache()", error);
    }
    if (status == ADBC_STATUS_OK) {
      status = RunDuckDbQuery(
          state->connection,
          "INSERT INTO " + target + " SELECT * FROM " + quoted_data, error);
    }
  } else if (state->ingest_mode == ADBC_INGEST_OPTION_MODE_APPEND) {
    status = RunDuckDbQuery(state->connection,
                            "SELECT * FROM quack_clear_cache()", error);
    if (status == ADBC_STATUS_OK) {
      status = RunDuckDbQueryAllowNotFound(
          state->connection,
          "INSERT INTO " + target + " SELECT * FROM " + quoted_data, error);
    }
  } else if (state->ingest_mode == ADBC_INGEST_OPTION_MODE_REPLACE) {
    status = RunRemoteQuery(state->connection,
                            "DROP TABLE IF EXISTS " + server_target, error);
    if (status == ADBC_STATUS_OK) {
      status = RunRemoteQuery(state->connection,
                              "CREATE TABLE " + server_target + create_columns,
                              error);
    }
    if (status == ADBC_STATUS_OK) {
      status = RunDuckDbQuery(state->connection,
                              "SELECT * FROM quack_clear_cache()", error);
    }
    if (status == ADBC_STATUS_OK) {
      status = RunDuckDbQuery(
          state->connection,
          "INSERT INTO " + target + " SELECT * FROM " + quoted_data, error);
    }
  } else if (state->ingest_mode == ADBC_INGEST_OPTION_MODE_CREATE_APPEND) {
    status = RunRemoteQuery(
        state->connection,
        "CREATE TABLE IF NOT EXISTS " + server_target + create_columns, error);
    if (status == ADBC_STATUS_OK) {
      status = RunDuckDbQuery(state->connection,
                              "SELECT * FROM quack_clear_cache()", error);
    }
    if (status == ADBC_STATUS_OK) {
      status = RunDuckDbQuery(
          state->connection,
          "INSERT INTO " + target + " SELECT * FROM " + quoted_data, error);
    }
  } else {
    status = InvalidArgument(error, "unsupported bulk ingest mode");
  }

  RunDuckDbQuery(state->connection, "DROP TABLE IF EXISTS " + quoted_data,
                 nullptr);
  if (status == ADBC_STATUS_OK) {
    RunDuckDbQuery(state->connection, "COMMIT", nullptr);
  }
  if (status == ADBC_STATUS_OK && rows_affected != nullptr) {
    *rows_affected = -1;
  }
  return status;
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

AdbcStatusCode CopyOptionString(std::string const& option_value, char* value,
                                size_t* length, AdbcError* error) {
  if (length == nullptr) {
    return InvalidArgument(error, "option length must not be null");
  }
  size_t const required_length = option_value.size() + 1;
  size_t const available_length = *length;
  *length = required_length;
  if (available_length < required_length) {
    return Ok(error);
  }
  if (value == nullptr) {
    return InvalidArgument(error, "option value buffer must not be null");
  }
  std::memcpy(value, option_value.c_str(), required_length);
  return Ok(error);
}

AdbcStatusCode QueryRemoteStringValue(ConnectionState* state,
                                      std::string const& sql,
                                      std::string* value, AdbcError* error) {
  duckdb_result result;
  std::string const remote_sql = adbc_driver_quack::BuildRemoteQuerySql(sql);
  duckdb_state const query_state =
      duckdb_query(state->connection, remote_sql.c_str(), &result);
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
                       "remote DuckDB query did not return one string value");
  }

  char* result_value = duckdb_value_varchar(&result, 0, 0);
  if (result_value == nullptr) {
    duckdb_destroy_result(&result);
    return InvalidData(error,
                       "remote DuckDB query did not return one string value");
  }

  *value = result_value;
  duckdb_free(result_value);
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
  driver->ConnectionGetOption = DriverConnectionGetOption;
  driver->ConnectionGetObjects = DriverConnectionGetObjects;
  driver->ConnectionNew = DriverConnectionNew;
  driver->ConnectionRelease = DriverConnectionRelease;
  driver->ConnectionSetOption = DriverConnectionSetOption;

  driver->StatementBind = DriverStatementBind;
  driver->StatementBindStream = DriverStatementBindStream;
  driver->StatementExecuteQuery = DriverStatementExecuteQuery;
  driver->StatementNew = DriverStatementNew;
  driver->StatementPrepare = DriverStatementPrepare;
  driver->StatementRelease = DriverStatementRelease;
  driver->StatementSetOption = DriverStatementSetOption;
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

AdbcStatusCode DriverConnectionGetOption(AdbcConnection* connection,
                                         char const* key, char* value,
                                         size_t* length, AdbcError* error) {
  ConnectionState* state = GetConnection(connection);
  if (state == nullptr || !state->initialized) {
    return InvalidState(error, "connection is not initialized");
  }
  if (key == nullptr) {
    return InvalidArgument(error, "connection option key must not be null");
  }

  std::string option_value;
  if (std::strcmp(key, ADBC_CONNECTION_OPTION_CURRENT_CATALOG) == 0) {
    AdbcStatusCode status = QueryRemoteStringValue(
        state, "SELECT current_database()", &option_value, error);
    if (status != ADBC_STATUS_OK) {
      return status;
    }
  } else if (std::strcmp(key, ADBC_CONNECTION_OPTION_CURRENT_DB_SCHEMA) == 0) {
    AdbcStatusCode status = QueryRemoteStringValue(
        state, "SELECT current_schema()", &option_value, error);
    if (status != ADBC_STATUS_OK) {
      return status;
    }
  } else {
    return NotFound(error, std::string{"connection option not found: "} + key);
  }

  return CopyOptionString(option_value, value, length, error);
}

AdbcStatusCode DriverConnectionGetObjects(
    AdbcConnection* connection, int depth, char const* catalog,
    char const* db_schema, char const* table_name, char const** table_type,
    char const* column_name, ArrowArrayStream* out, AdbcError* error) {
  ConnectionState* state = GetConnection(connection);
  if (state == nullptr || !state->initialized) {
    return InvalidState(error, "connection is not initialized");
  }

  std::string table_type_condition;
  AdbcStatusCode status =
      BuildTableTypeCondition(table_type, &table_type_condition, error);
  if (status != ADBC_STATUS_OK) {
    return status;
  }

  std::string const query = BuildGetObjectsQuery(
      depth, LikeFilter(catalog), LikeFilter(db_schema), LikeFilter(table_name),
      table_type_condition, LikeFilter(column_name));
  if (query.empty()) {
    return InvalidArgument(error, "invalid GetObjects depth");
  }

  auto const result = adbc_driver_quack::ExecuteDuckDbArrowQuery(
      state->connection, adbc_driver_quack::BuildRemoteQuerySql(query), out,
      nullptr);
  if (result.status != ADBC_STATUS_OK) {
    return SetError(error, result.status, result.message, result.vendor_code);
  }
  return Ok(error);
}

AdbcStatusCode DriverConnectionSetOption(AdbcConnection* connection,
                                         char const* key, char const* value,
                                         AdbcError* error) {
  ConnectionState* state = GetConnection(connection);
  if (state == nullptr || !state->initialized) {
    return InvalidState(error, "connection is not initialized");
  }
  if (key == nullptr || value == nullptr) {
    return InvalidArgument(error,
                           "connection option key and value must not be null");
  }

  if (std::strcmp(key, ADBC_CONNECTION_OPTION_CURRENT_CATALOG) == 0 ||
      std::strcmp(key, ADBC_CONNECTION_OPTION_CURRENT_DB_SCHEMA) == 0) {
    std::string const sql = "USE " + QuoteIdentifier(value);
    return RunRemoteQueryAllowNotFound(state, sql, error);
  }

  return NotImplemented(error, "unsupported connection option");
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
  ReleaseBoundStream(state);
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
  if (state->has_bound_stream) {
    if (out != nullptr) {
      return InvalidArgument(error,
                             "bulk ingest does not produce a result stream");
    }
    return ExecuteBulkIngest(state, rows_affected, error);
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

AdbcStatusCode DriverStatementBindStream(AdbcStatement* statement,
                                         ArrowArrayStream* stream,
                                         AdbcError* error) {
  StatementState* state = GetStatement(statement);
  if (state == nullptr) {
    return InvalidState(error, "statement is not initialized");
  }
  if (stream == nullptr) {
    return InvalidArgument(error, "Arrow stream must not be null");
  }
  ReleaseBoundStream(state);
  state->sql.clear();
  state->bound_stream = *stream;
  state->has_bound_stream = true;
  stream->release = nullptr;
  return Ok(error);
}

AdbcStatusCode DriverStatementRelease(AdbcStatement* statement,
                                      AdbcError* error) {
  if (statement == nullptr) {
    return Ok(error);
  }
  StatementState* state = GetStatement(statement);
  ReleaseBoundStream(state);
  delete state;
  statement->private_data = nullptr;
  return Ok(error);
}

AdbcStatusCode DriverStatementSetOption(AdbcStatement* statement,
                                        char const* key, char const* value,
                                        AdbcError* error) {
  StatementState* state = GetStatement(statement);
  if (state == nullptr) {
    return InvalidState(error, "statement is not initialized");
  }
  if (key == nullptr || value == nullptr) {
    return InvalidArgument(error,
                           "statement option key and value must not be null");
  }
  if (std::strcmp(key, ADBC_INGEST_OPTION_TARGET_TABLE) == 0) {
    state->ingest_target_table = value;
    return Ok(error);
  }
  if (std::strcmp(key, ADBC_INGEST_OPTION_TARGET_CATALOG) == 0) {
    state->ingest_target_catalog = value;
    return Ok(error);
  }
  if (std::strcmp(key, ADBC_INGEST_OPTION_TARGET_DB_SCHEMA) == 0) {
    state->ingest_target_schema = value;
    return Ok(error);
  }
  if (std::strcmp(key, ADBC_INGEST_OPTION_MODE) == 0) {
    state->ingest_mode = value;
    return Ok(error);
  }
  if (std::strcmp(key, ADBC_INGEST_OPTION_TEMPORARY) == 0) {
    if (std::strcmp(value, ADBC_OPTION_VALUE_ENABLED) == 0) {
      state->ingest_temporary = true;
      return Ok(error);
    }
    if (std::strcmp(value, ADBC_OPTION_VALUE_DISABLED) == 0) {
      state->ingest_temporary = false;
      return Ok(error);
    }
    return InvalidArgument(error, "invalid temporary bulk ingest option value");
  }
  return NotImplemented(error, "unsupported statement option");
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

ADBC_EXPORT AdbcStatusCode AdbcConnectionSetOption(AdbcConnection* connection,
                                                   char const* key,
                                                   char const* value,
                                                   AdbcError* error) {
  return DriverConnectionSetOption(connection, key, value, error);
}

ADBC_EXPORT AdbcStatusCode AdbcConnectionGetInfo(AdbcConnection* connection,
                                                 uint32_t const* info_codes,
                                                 size_t info_codes_length,
                                                 ArrowArrayStream* out,
                                                 AdbcError* error) {
  return DriverConnectionGetInfo(connection, info_codes, info_codes_length, out,
                                 error);
}

ADBC_EXPORT AdbcStatusCode AdbcConnectionGetObjects(
    AdbcConnection* connection, int depth, char const* catalog,
    char const* db_schema, char const* table_name, char const** table_type,
    char const* column_name, ArrowArrayStream* out, AdbcError* error) {
  return DriverConnectionGetObjects(connection, depth, catalog, db_schema,
                                    table_name, table_type, column_name, out,
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

ADBC_EXPORT AdbcStatusCode AdbcStatementSetOption(AdbcStatement* statement,
                                                  char const* key,
                                                  char const* value,
                                                  AdbcError* error) {
  return DriverStatementSetOption(statement, key, value, error);
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
