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

#include "quack_logging.h"

#include <cctype>
#include <cinttypes>
#include <cstdio>
#include <cstdlib>
#include <mutex>
#include <string>

namespace adbc_driver_quack {
namespace {

std::mutex stderr_mutex;

void WriteLogEntry(void*, duckdb_timestamp* timestamp, char const* level,
                   char const* category, char const* message) noexcept {
  try {
    std::lock_guard<std::mutex> lock(stderr_mutex);
    // Unix microseconds preserve DuckDB's timestamp without locale conversion.
    std::fprintf(stderr, "[quack] %" PRId64 " %s %s: %s\n",
                 timestamp ? timestamp->micros : int64_t{0}, level ? level : "",
                 category ? category : "", message ? message : "");
    std::fflush(stderr);
  } catch (...) {
    // Diagnostics must never interrupt a query or escape a C callback.
  }
}

void Warn(char const* message) noexcept {
  WriteLogEntry(nullptr, nullptr, "WARNING", "Logging", message);
}

bool Execute(duckdb_connection connection, char const* sql) {
  duckdb_result result{};
  auto const status = duckdb_query(connection, sql, &result);
  duckdb_destroy_result(&result);
  return status == DuckDBSuccess;
}

}  // namespace

void InitializeQuackLogging(duckdb_database database,
                            duckdb_connection connection) noexcept {
  duckdb_log_storage storage = nullptr;
  try {
    auto const* value = std::getenv("ADBC_DRIVER_QUACK_LOG_LEVEL");
    if (value == nullptr || value[0] == '\0') return;
    std::string level(value);
    for (auto& ch : level) {
      ch = static_cast<char>(std::toupper(static_cast<unsigned char>(ch)));
    }
    if (level == "WARN") level = "WARNING";
    if (level != "INFO" && level != "DEBUG" && level != "WARNING" &&
        level != "ERROR") {
      Warn(
          "Invalid ADBC_DRIVER_QUACK_LOG_LEVEL; expected info, debug, warn, "
          "warning, or error. Logging is disabled.");
      return;
    }

    storage = duckdb_create_log_storage();
    if (storage == nullptr) {
      Warn("Could not create log storage. Logging is disabled.");
      return;
    }
    duckdb_log_storage_set_name(storage, "adbc_driver_quack_stderr");
    duckdb_log_storage_set_write_log_entry(storage, WriteLogEntry);
    auto const status = duckdb_register_log_storage(database, storage);
    duckdb_destroy_log_storage(&storage);
    if (status != DuckDBSuccess) {
      Warn("Could not register log storage. Logging is disabled.");
      return;
    }
    // Omitting category arguments enables all categories at this threshold.
    auto const sql = "CALL enable_logging(level = '" + level +
                     "', storage = 'adbc_driver_quack_stderr')";
    if (Execute(connection, sql.c_str())) return;
  } catch (...) {
    if (storage != nullptr) duckdb_destroy_log_storage(&storage);
  }
  try {
    Execute(connection, "SET enable_logging = false");
  } catch (...) {
  }
  Warn("Could not configure client logging. Logging is disabled.");
}

}  // namespace adbc_driver_quack
