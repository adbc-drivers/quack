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

#include "duckdb_arrow_stream.h"

#include <cerrno>
#include <cstring>
#include <new>
#include <string>
#include <utility>

namespace adbc_driver_quack {
namespace {

struct DuckDbArrowStreamState {
  duckdb_arrow result = nullptr;
  std::string last_error;
};

DuckDbArrowQueryResult Error(AdbcStatusCode status, std::string message,
                             int32_t vendor_code = 0) {
  DuckDbArrowQueryResult result;
  result.status = status;
  result.message = std::move(message);
  result.vendor_code = vendor_code;
  return result;
}

void ResetStream(ArrowArrayStream* stream) {
  if (stream == nullptr) {
    return;
  }
  stream->get_schema = nullptr;
  stream->get_next = nullptr;
  stream->get_last_error = nullptr;
  stream->release = nullptr;
  stream->private_data = nullptr;
}

DuckDbArrowStreamState* GetState(ArrowArrayStream* stream) {
  if (stream == nullptr) {
    return nullptr;
  }
  return static_cast<DuckDbArrowStreamState*>(stream->private_data);
}

std::string DuckDbArrowError(duckdb_arrow result, std::string fallback) {
  if (result == nullptr) {
    return fallback;
  }
  char const* error = duckdb_query_arrow_error(result);
  if (error == nullptr || error[0] == '\0') {
    return fallback;
  }
  return error;
}

int StreamGetSchema(ArrowArrayStream* stream, ArrowSchema* out) {
  auto* state = GetState(stream);
  if (state == nullptr || state->result == nullptr || out == nullptr) {
    return EINVAL;
  }
  std::memset(out, 0, sizeof(*out));
  auto* schema = out;
  if (duckdb_query_arrow_schema(
          state->result, reinterpret_cast<duckdb_arrow_schema*>(&schema)) !=
      DuckDBSuccess) {
    state->last_error =
        DuckDbArrowError(state->result, "failed to get DuckDB Arrow schema");
    return EIO;
  }
  state->last_error.clear();
  return 0;
}

int StreamGetNext(ArrowArrayStream* stream, ArrowArray* out) {
  auto* state = GetState(stream);
  if (state == nullptr || state->result == nullptr || out == nullptr) {
    return EINVAL;
  }
  std::memset(out, 0, sizeof(*out));
  auto* array = out;
  if (duckdb_query_arrow_array(state->result,
                               reinterpret_cast<duckdb_arrow_array*>(&array)) !=
      DuckDBSuccess) {
    state->last_error =
        DuckDbArrowError(state->result, "failed to get DuckDB Arrow array");
    return EIO;
  }
  state->last_error.clear();
  return 0;
}

char const* StreamGetLastError(ArrowArrayStream* stream) {
  auto* state = GetState(stream);
  if (state == nullptr) {
    return "DuckDB Arrow stream is released";
  }
  if (state->last_error.empty()) {
    return nullptr;
  }
  return state->last_error.c_str();
}

void StreamRelease(ArrowArrayStream* stream) {
  auto* state = GetState(stream);
  if (state == nullptr) {
    ResetStream(stream);
    return;
  }
  duckdb_destroy_arrow(&state->result);
  delete state;
  ResetStream(stream);
}

}  // namespace

DuckDbArrowQueryResult ExecuteDuckDbArrowQuery(duckdb_connection connection,
                                               std::string_view sql,
                                               ArrowArrayStream* out,
                                               int64_t* rows_affected) {
  ResetStream(out);
  if (connection == nullptr) {
    return Error(ADBC_STATUS_INVALID_STATE, "connection is not initialized");
  }

  std::string sql_storage(sql);
  duckdb_arrow result = nullptr;
  if (duckdb_query_arrow(connection, sql_storage.c_str(), &result) !=
      DuckDBSuccess) {
    std::string const message =
        DuckDbArrowError(result, "DuckDB Arrow query failed");
    duckdb_destroy_arrow(&result);
    return Error(ADBC_STATUS_IO, message);
  }

  if (rows_affected != nullptr) {
    idx_t const rows_changed = duckdb_arrow_rows_changed(result);
    *rows_affected = rows_changed > 0 ? static_cast<int64_t>(rows_changed) : -1;
  }

  if (out == nullptr) {
    duckdb_destroy_arrow(&result);
    return {};
  }

  auto* state = new (std::nothrow) DuckDbArrowStreamState{result, {}};
  if (state == nullptr) {
    duckdb_destroy_arrow(&result);
    return Error(ADBC_STATUS_UNKNOWN, "failed to allocate DuckDB Arrow stream");
  }

  out->private_data = state;
  out->get_schema = StreamGetSchema;
  out->get_next = StreamGetNext;
  out->get_last_error = StreamGetLastError;
  out->release = StreamRelease;
  return {};
}

}  // namespace adbc_driver_quack
