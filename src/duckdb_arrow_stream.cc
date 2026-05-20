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
#include <duckdb/common/arrow/arrow_converter.hpp>
#include <duckdb/common/types/data_chunk.hpp>
#include <duckdb/function/table/arrow/arrow_duck_schema.hpp>
#include <duckdb/main/connection.hpp>
#include <duckdb/main/query_result.hpp>
#include <new>
#include <string>
#include <utility>

namespace adbc_driver_quack {
namespace {

struct DuckDbArrowStreamState {
  duckdb::unique_ptr<duckdb::QueryResult> result;
  duckdb::unordered_map<
      duckdb::idx_t, duckdb::shared_ptr<duckdb::ArrowTypeExtensionData> const>
      extension_types;
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

std::string DuckDbErrorMessage(duckdb::QueryResult const* result,
                               std::string fallback) {
  if (result == nullptr || !result->HasError()) {
    return fallback;
  }
  std::string const& error = result->GetError();
  if (error.empty()) {
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
  try {
    duckdb::ArrowConverter::ToArrowSchema(out, state->result->types,
                                          state->result->names,
                                          state->result->client_properties);
  } catch (duckdb::Exception const& ex) {
    state->last_error = ex.what();
    return EIO;
  } catch (std::exception const& ex) {
    state->last_error = ex.what();
    return EIO;
  } catch (...) {
    state->last_error = "unknown error while getting DuckDB Arrow schema";
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

  duckdb::ErrorData& fetch_error = state->result->GetErrorObject();
  duckdb::unique_ptr<duckdb::DataChunk> chunk;
  try {
    if (!state->result->TryFetch(chunk, fetch_error)) {
      state->last_error = fetch_error.Message();
      if (state->last_error.empty()) {
        state->last_error = DuckDbErrorMessage(
            state->result.get(), "failed to fetch DuckDB result chunk");
      }
      return EIO;
    }
  } catch (duckdb::Exception const& ex) {
    state->last_error = ex.what();
    return EIO;
  } catch (std::exception const& ex) {
    state->last_error = ex.what();
    return EIO;
  } catch (...) {
    state->last_error = "unknown error while fetching DuckDB result chunk";
    return EIO;
  }
  if (chunk == nullptr || chunk->size() == 0) {
    state->last_error.clear();
    return 0;
  }

  try {
    duckdb::ArrowConverter::ToArrowArray(
        *chunk, out, state->result->client_properties, state->extension_types);
  } catch (duckdb::Exception const& ex) {
    state->last_error = ex.what();
    return EIO;
  } catch (std::exception const& ex) {
    state->last_error = ex.what();
    return EIO;
  } catch (...) {
    state->last_error = "unknown error while converting DuckDB chunk to Arrow";
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
  delete state;
  ResetStream(stream);
}

}  // namespace

DuckDbArrowQueryResult ExecuteDuckDbStreamingArrowQuery(
    duckdb_connection connection, std::string_view sql, ArrowArrayStream* out,
    int64_t* rows_affected) {
  ResetStream(out);
  if (connection == nullptr) {
    return Error(ADBC_STATUS_INVALID_STATE, "connection is not initialized");
  }

  auto* duckdb_connection_ptr =
      reinterpret_cast<duckdb::Connection*>(connection);
  duckdb::unique_ptr<duckdb::QueryResult> query_result;
  try {
    // DuckDB allows one active StreamQueryResult per connection. ADBC callers
    // must consume or release the returned ArrowArrayStream before issuing
    // another query on the same connection.
    query_result = duckdb_connection_ptr->SendQuery(
        std::string(sql), duckdb::QueryResultOutputType::ALLOW_STREAMING);
  } catch (duckdb::Exception const& ex) {
    return Error(ADBC_STATUS_IO, ex.what());
  } catch (std::exception const& ex) {
    return Error(ADBC_STATUS_IO, ex.what());
  } catch (...) {
    return Error(ADBC_STATUS_IO, "unknown DuckDB query error");
  }

  if (query_result == nullptr) {
    return Error(ADBC_STATUS_IO, "DuckDB query returned no result");
  }
  if (query_result->HasError()) {
    return Error(ADBC_STATUS_IO,
                 DuckDbErrorMessage(query_result.get(), "DuckDB query failed"),
                 static_cast<int32_t>(query_result->GetErrorType()));
  }

  if (rows_affected != nullptr) {
    *rows_affected = -1;
  }

  if (out == nullptr) {
    return {};
  }

  duckdb::unordered_map<
      duckdb::idx_t, duckdb::shared_ptr<duckdb::ArrowTypeExtensionData> const>
      extension_types;
  try {
    extension_types = duckdb::ArrowTypeExtensionData::GetExtensionTypes(
        *query_result->client_properties.client_context, query_result->types);
  } catch (duckdb::Exception const& ex) {
    return Error(ADBC_STATUS_IO, ex.what());
  } catch (std::exception const& ex) {
    return Error(ADBC_STATUS_IO, ex.what());
  } catch (...) {
    return Error(ADBC_STATUS_IO,
                 "unknown error while preparing DuckDB Arrow conversions");
  }

  auto* state = new (std::nothrow) DuckDbArrowStreamState{
      std::move(query_result), std::move(extension_types), {}};
  if (state == nullptr) {
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
