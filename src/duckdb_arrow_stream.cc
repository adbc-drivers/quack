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
#include <duckdb/main/client_context.hpp>
#include <duckdb/main/connection.hpp>
#include <duckdb/main/pending_query_result.hpp>
#include <duckdb/main/query_result.hpp>
#include <new>
#include <string>
#include <utility>

namespace adbc_driver_quack {
namespace {

struct DuckDbArrowStreamState {
  duckdb::unique_ptr<duckdb::PendingQueryResult> pending_result;
  duckdb::unique_ptr<duckdb::QueryResult> result;
  duckdb::vector<duckdb::LogicalType> types;
  duckdb::vector<duckdb::string> names;
  duckdb::ClientProperties client_properties;
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

std::string DuckDbErrorMessage(duckdb::PendingQueryResult const* result,
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

int EnsureResultReady(DuckDbArrowStreamState* state) {
  if (state->result != nullptr) {
    return 0;
  }
  if (state->pending_result == nullptr) {
    state->last_error = "DuckDB query result is not available";
    return EINVAL;
  }
  try {
    state->result = state->pending_result->Execute();
    state->pending_result.reset();
  } catch (duckdb::Exception const& ex) {
    state->last_error = ex.what();
    return EIO;
  } catch (std::exception const& ex) {
    state->last_error = ex.what();
    return EIO;
  } catch (...) {
    state->last_error = "unknown DuckDB query execution error";
    return EIO;
  }
  if (state->result == nullptr) {
    state->last_error = "DuckDB query returned no result";
    return EIO;
  }
  if (state->result->HasError()) {
    state->last_error =
        DuckDbErrorMessage(state->result.get(), "DuckDB query failed");
    return EIO;
  }
  return 0;
}

int StreamGetSchema(ArrowArrayStream* stream, ArrowSchema* out) {
  auto* state = GetState(stream);
  if (state == nullptr || out == nullptr) {
    return EINVAL;
  }
  std::memset(out, 0, sizeof(*out));
  try {
    duckdb::ArrowConverter::ToArrowSchema(out, state->types, state->names,
                                          state->client_properties);
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
  if (state == nullptr || out == nullptr) {
    return EINVAL;
  }
  std::memset(out, 0, sizeof(*out));

  int const result_status = EnsureResultReady(state);
  if (result_status != 0) {
    return result_status;
  }

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
    duckdb::ArrowConverter::ToArrowArray(*chunk, out, state->client_properties,
                                         state->extension_types);
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
  duckdb::unique_ptr<duckdb::PendingQueryResult> pending_result;
  try {
    // DuckDB allows one active StreamQueryResult per connection. ADBC callers
    // must consume or release the returned ArrowArrayStream before issuing
    // another query on the same connection.
    pending_result = duckdb_connection_ptr->PendingQuery(
        std::string(sql), duckdb::QueryResultOutputType::ALLOW_STREAMING);
  } catch (duckdb::Exception const& ex) {
    return Error(ADBC_STATUS_IO, ex.what());
  } catch (std::exception const& ex) {
    return Error(ADBC_STATUS_IO, ex.what());
  } catch (...) {
    return Error(ADBC_STATUS_IO, "unknown DuckDB query error");
  }

  if (pending_result == nullptr) {
    return Error(ADBC_STATUS_IO, "DuckDB query returned no pending result");
  }
  if (pending_result->HasError()) {
    return Error(
        ADBC_STATUS_IO,
        DuckDbErrorMessage(pending_result.get(), "DuckDB query failed"),
        static_cast<int32_t>(pending_result->GetErrorType()));
  }

  if (rows_affected != nullptr) {
    *rows_affected = -1;
  }

  if (out == nullptr) {
    return {};
  }

  duckdb::ClientProperties client_properties =
      duckdb_connection_ptr->context->GetClientProperties();

  duckdb::unordered_map<
      duckdb::idx_t, duckdb::shared_ptr<duckdb::ArrowTypeExtensionData> const>
      extension_types;
  try {
    extension_types = duckdb::ArrowTypeExtensionData::GetExtensionTypes(
        *client_properties.client_context, pending_result->types);
  } catch (duckdb::Exception const& ex) {
    return Error(ADBC_STATUS_IO, ex.what());
  } catch (std::exception const& ex) {
    return Error(ADBC_STATUS_IO, ex.what());
  } catch (...) {
    return Error(ADBC_STATUS_IO,
                 "unknown error while preparing DuckDB Arrow conversions");
  }

  duckdb::vector<duckdb::LogicalType> types = pending_result->types;
  duckdb::vector<duckdb::string> names = pending_result->names;
  auto* state =
      new (std::nothrow) DuckDbArrowStreamState{std::move(pending_result),
                                                nullptr,
                                                std::move(types),
                                                std::move(names),
                                                std::move(client_properties),
                                                std::move(extension_types),
                                                {}};
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
