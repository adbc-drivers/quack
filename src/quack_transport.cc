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

#include "quack_transport.h"

#include "sql_escape.h"

namespace adbc_driver_quack {
namespace {

QuackTransportResult Execute(duckdb_connection connection,
                             std::string const& sql) {
  duckdb_result result = {};
  QuackTransportResult status;
  if (duckdb_query(connection, sql.c_str(), &result) == DuckDBError) {
    status.status = ADBC_STATUS_IO;
    char const* message = duckdb_result_error(&result);
    status.message = message != nullptr ? message : "DuckDB query failed";
    status.vendor_code =
        static_cast<int32_t>(duckdb_result_error_type(&result));
  }
  duckdb_destroy_result(&result);
  return status;
}

}  // namespace

QuackTransportResult InitializeQuackTransport(duckdb_connection connection,
                                              ParsedQuackUri const& uri,
                                              QuackTlsMode tls) {
  auto status = Execute(connection, "LOAD httpfs");
  if (status.status != ADBC_STATUS_OK) {
    return status;
  }
  std::string const verify = tls == QuackTlsMode::SkipVerify ? "false" : "true";
  status = Execute(connection,
                   "SET enable_curl_server_cert_verification = " + verify);
  if (status.status != ADBC_STATUS_OK) {
    return status;
  }
  status =
      Execute(connection, "SET enable_server_cert_verification = " + verify);
  if (status.status != ADBC_STATUS_OK) {
    return status;
  }

  std::string attach = "ATTACH " + DuckDbSqlStringLiteral(uri.endpoint) +
                       " AS remote (disable_ssl " +
                       (tls == QuackTlsMode::Disable ? "true" : "false");
  if (!uri.token.empty()) {
    attach += ", token " + DuckDbSqlStringLiteral(uri.token);
  }
  attach += ")";
  return Execute(connection, attach);
}

}  // namespace adbc_driver_quack
