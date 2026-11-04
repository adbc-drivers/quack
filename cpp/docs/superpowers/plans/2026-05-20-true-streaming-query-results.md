<!--
Copyright (c) 2026 ADBC Drivers Contributors

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

        http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-->

# True Streaming Query Results Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace DuckDB's materialized Arrow query API with a true chunk-fetching Arrow C Stream adapter for statement queries and GetObjects.

**Architecture:** Keep streaming logic isolated in `src/duckdb_arrow_stream.*`. The helper will own a DuckDB C++ `QueryResult`, fetch one `DataChunk` per `get_next`, and convert each chunk to Arrow only when requested.

**Tech Stack:** C++20, DuckDB C++ API, Arrow C Data/Stream interfaces, Googletest, CMake, Pixi validation.

---

## File Structure

- Modify `src/duckdb_arrow_stream.h`: rename the helper declaration to `ExecuteDuckDbStreamingArrowQuery`.
- Modify `src/duckdb_arrow_stream.cc`: replace `duckdb_query_arrow` usage with `duckdb::Connection::SendQuery`, `duckdb::QueryResult::TryFetch`, `duckdb::ArrowConverter::ToArrowSchema`, and `duckdb::ArrowConverter::ToArrowArray`.
- Modify `src/adbc_driver_quack.cc`: update `DriverStatementExecuteQuery` and `DriverConnectionGetObjects` to call the renamed helper.
- Modify `tests/duckdb_arrow_stream_test.cc`: update the existing test and add streaming-specific coverage.
- Optionally modify `docs/DESIGN.md`: update the helper description from "DuckDB Arrow query results" to "DuckDB streaming query results" if the code rename makes the existing description stale.

## Task 1: Rename Helper API and Update Call Sites

**Files:**
- Modify: `src/duckdb_arrow_stream.h`
- Modify: `src/duckdb_arrow_stream.cc`
- Modify: `src/adbc_driver_quack.cc`
- Modify: `tests/duckdb_arrow_stream_test.cc`

- [ ] **Step 1: Rename the declaration**

In `src/duckdb_arrow_stream.h`, replace:

```cpp
DuckDbArrowQueryResult ExecuteDuckDbArrowQuery(duckdb_connection connection,
                                               std::string_view sql,
                                               ArrowArrayStream* out,
                                               int64_t* rows_affected);
```

with:

```cpp
DuckDbArrowQueryResult ExecuteDuckDbStreamingArrowQuery(
    duckdb_connection connection, std::string_view sql, ArrowArrayStream* out,
    int64_t* rows_affected);
```

- [ ] **Step 2: Rename the definition**

In `src/duckdb_arrow_stream.cc`, replace the function signature:

```cpp
DuckDbArrowQueryResult ExecuteDuckDbArrowQuery(duckdb_connection connection,
                                               std::string_view sql,
                                               ArrowArrayStream* out,
                                               int64_t* rows_affected) {
```

with:

```cpp
DuckDbArrowQueryResult ExecuteDuckDbStreamingArrowQuery(
    duckdb_connection connection, std::string_view sql, ArrowArrayStream* out,
    int64_t* rows_affected) {
```

- [ ] **Step 3: Update driver call sites**

In `src/adbc_driver_quack.cc`, replace both calls:

```cpp
adbc_driver_quack::ExecuteDuckDbArrowQuery(
```

with:

```cpp
adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
```

- [ ] **Step 4: Update test call sites**

In `tests/duckdb_arrow_stream_test.cc`, replace:

```cpp
adbc_driver_quack::ExecuteDuckDbArrowQuery(
```

with:

```cpp
adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
```

- [ ] **Step 5: Build to confirm the mechanical rename**

Run:

```bash
cmake --build build/ci-test-linux-amd64 --target adbc_driver_quack_helper_tests
```

Expected: the target either builds successfully or fails only because Task 2 has not yet replaced the helper internals.

## Task 2: Replace Materialized DuckDB Arrow State With Streaming Query State

**Files:**
- Modify: `src/duckdb_arrow_stream.cc`

- [ ] **Step 1: Update includes**

At the top of `src/duckdb_arrow_stream.cc`, add these includes:

```cpp
#include <duckdb/common/arrow/arrow_converter.hpp>
#include <duckdb/common/types/data_chunk.hpp>
#include <duckdb/function/table/arrow/arrow_duck_schema.hpp>
#include <duckdb/main/connection.hpp>
#include <duckdb/main/query_result.hpp>
```

Keep the existing standard headers.

- [ ] **Step 2: Change stream state ownership**

Replace:

```cpp
struct DuckDbArrowStreamState {
  duckdb_arrow result = nullptr;
  std::string last_error;
};
```

with:

```cpp
struct DuckDbArrowStreamState {
  duckdb::unique_ptr<duckdb::QueryResult> result;
  duckdb::unordered_map<
      duckdb::idx_t, const duckdb::shared_ptr<duckdb::ArrowTypeExtensionData>>
      extension_types;
  std::string last_error;
};
```

- [ ] **Step 3: Replace DuckDB Arrow error helper**

Replace `DuckDbArrowError(...)` with:

```cpp
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
```

- [ ] **Step 4: Update release callback**

Replace the body of `StreamRelease` with:

```cpp
void StreamRelease(ArrowArrayStream* stream) {
  auto* state = GetState(stream);
  if (state == nullptr) {
    ResetStream(stream);
    return;
  }
  delete state;
  ResetStream(stream);
}
```

The `duckdb::unique_ptr<duckdb::QueryResult>` destructor closes the underlying result.

## Task 3: Implement Streaming Schema and Chunk Fetching

**Files:**
- Modify: `src/duckdb_arrow_stream.cc`

- [ ] **Step 1: Rewrite `StreamGetSchema`**

Replace the callback body with:

```cpp
int StreamGetSchema(ArrowArrayStream* stream, ArrowSchema* out) {
  auto* state = GetState(stream);
  if (state == nullptr || state->result == nullptr || out == nullptr) {
    return EINVAL;
  }
  std::memset(out, 0, sizeof(*out));
  try {
    duckdb::ArrowConverter::ToArrowSchema(
        out, state->result->types, state->result->names,
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
```

- [ ] **Step 2: Rewrite `StreamGetNext`**

Replace the callback body with:

```cpp
int StreamGetNext(ArrowArrayStream* stream, ArrowArray* out) {
  auto* state = GetState(stream);
  if (state == nullptr || state->result == nullptr || out == nullptr) {
    return EINVAL;
  }
  std::memset(out, 0, sizeof(*out));

  duckdb::ErrorData fetch_error;
  duckdb::unique_ptr<duckdb::DataChunk> chunk;
  if (!state->result->TryFetch(chunk, fetch_error)) {
    state->last_error = fetch_error.Message();
    if (state->last_error.empty()) {
      state->last_error = "failed to fetch DuckDB result chunk";
    }
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
```

- [ ] **Step 3: Keep `StreamGetLastError` unchanged**

Verify it still returns `nullptr` when `last_error` is empty and a stable C string otherwise.

## Task 4: Implement Streaming Query Creation

**Files:**
- Modify: `src/duckdb_arrow_stream.cc`

- [ ] **Step 1: Replace materialized query execution**

Inside `ExecuteDuckDbStreamingArrowQuery`, replace the `duckdb_query_arrow` block and the old `duckdb_destroy_arrow` cleanup with:

```cpp
auto* duckdb_connection_ptr =
    reinterpret_cast<duckdb::Connection*>(connection);
duckdb::unique_ptr<duckdb::QueryResult> query_result;
try {
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
```

- [ ] **Step 2: Preserve row count behavior**

After the error check, add:

```cpp
if (rows_affected != nullptr) {
  *rows_affected = -1;
}
```

- [ ] **Step 3: Handle `out == nullptr`**

Keep a no-output branch that discards the result object:

```cpp
if (out == nullptr) {
  return {};
}
```

- [ ] **Step 4: Compute extension type conversions once**

Before allocating `DuckDbArrowStreamState`, add:

```cpp
duckdb::unordered_map<
    duckdb::idx_t, const duckdb::shared_ptr<duckdb::ArrowTypeExtensionData>>
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
```

- [ ] **Step 5: Install stream callbacks**

Replace the old state allocation with:

```cpp
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
```

- [ ] **Step 6: Add active stream comment**

Add this comment immediately above `SendQuery`:

```cpp
// DuckDB allows one active StreamQueryResult per connection. ADBC callers
// must consume or release the returned ArrowArrayStream before issuing another
// query on the same connection.
```

## Task 5: Update and Expand Helper Tests

**Files:**
- Modify: `tests/duckdb_arrow_stream_test.cc`

- [ ] **Step 1: Add RAII helpers for DuckDB setup**

Keep the existing simple setup if preferred. Add a local helper function near the top of the test file:

```cpp
struct DuckDbFixture {
  duckdb_database database = nullptr;
  duckdb_connection connection = nullptr;

  DuckDbFixture() {
    EXPECT_EQ(duckdb_open(nullptr, &database), DuckDBSuccess);
    EXPECT_EQ(duckdb_connect(database, &connection), DuckDBSuccess);
  }

  ~DuckDbFixture() {
    duckdb_disconnect(&connection);
    duckdb_close(&database);
  }
};
```

- [ ] **Step 2: Update the existing smoke test**

Replace manual open/connect/close in `ExecutesQueryAsArrowStream` with:

```cpp
DuckDbFixture fixture;

ArrowArrayStream stream = {};
int64_t rows_affected = 0;
auto const result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
    fixture.connection, "SELECT 42 AS answer", &stream, &rows_affected);
```

Keep the existing schema, first-array, end-of-stream, and release assertions.

- [ ] **Step 3: Add a multi-chunk streaming test**

Add:

```cpp
TEST(DuckDbArrowStreamTest, FetchesLargeQueryInMultipleArrays) {
  DuckDbFixture fixture;

  ArrowArrayStream stream = {};
  auto const result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      fixture.connection, "SELECT range AS value FROM range(10000)", &stream,
      nullptr);

  ASSERT_EQ(result.status, ADBC_STATUS_OK);
  ASSERT_NE(stream.get_next, nullptr);

  int array_count = 0;
  int64_t row_count = 0;
  while (true) {
    ArrowArray array = {};
    ASSERT_EQ(stream.get_next(&stream, &array), 0);
    if (array.release == nullptr) {
      break;
    }
    ++array_count;
    row_count += array.length;
    array.release(&array);
  }

  EXPECT_GT(array_count, 1);
  EXPECT_EQ(row_count, 10000);
  stream.release(&stream);
}
```

- [ ] **Step 4: Add a query error test**

Add:

```cpp
TEST(DuckDbArrowStreamTest, ReportsQueryErrors) {
  DuckDbFixture fixture;

  ArrowArrayStream stream = {};
  auto const result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      fixture.connection, "SELECT * FROM missing_table", &stream, nullptr);

  EXPECT_EQ(result.status, ADBC_STATUS_IO);
  EXPECT_FALSE(result.message.empty());
  EXPECT_EQ(stream.release, nullptr);
}
```

- [ ] **Step 5: Add callback argument validation test**

Add:

```cpp
TEST(DuckDbArrowStreamTest, RejectsInvalidCallbackArguments) {
  DuckDbFixture fixture;

  ArrowArrayStream stream = {};
  auto const result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      fixture.connection, "SELECT 1", &stream, nullptr);
  ASSERT_EQ(result.status, ADBC_STATUS_OK);

  ArrowSchema schema = {};
  ArrowArray array = {};
  EXPECT_EQ(stream.get_schema(nullptr, &schema), EINVAL);
  EXPECT_EQ(stream.get_schema(&stream, nullptr), EINVAL);
  EXPECT_EQ(stream.get_next(nullptr, &array), EINVAL);
  EXPECT_EQ(stream.get_next(&stream, nullptr), EINVAL);

  stream.release(&stream);
}
```

- [ ] **Step 6: Run helper tests**

Run:

```bash
cmake --build build/ci-test-linux-amd64 --target adbc_driver_quack_helper_tests
./build/ci-test-linux-amd64/tests/adbc_driver_quack_helper_tests
```

Expected: all helper tests pass.

## Task 6: Update Project Design Notes

**Files:**
- Modify: `docs/DESIGN.md`

- [ ] **Step 1: Update helper description**

In `docs/DESIGN.md`, replace:

```markdown
- `src/duckdb_arrow_stream.cc` adapts DuckDB Arrow query results to the Arrow C
  Stream interface.
```

with:

```markdown
- `src/duckdb_arrow_stream.cc` adapts DuckDB streaming query results to the
  Arrow C Stream interface.
```

- [ ] **Step 2: Add runtime note**

After the paragraph that starts with `Statement execution wraps caller SQL`, add:

```markdown
Result-producing statement execution and GetObjects use DuckDB streaming query
results and convert each fetched DuckDB chunk to Arrow on demand. Callers must
consume or release a returned Arrow stream before issuing another query on the
same connection because DuckDB permits only one active streaming query result
per connection.
```

## Task 7: Full Verification

**Files:**
- No source changes expected.

- [ ] **Step 1: Run CI-style build**

Run:

```bash
./ci/scripts/build.sh test linux amd64
```

Expected: build succeeds without compiler warnings.

- [ ] **Step 2: Run C++ tests**

Run:

```bash
./ci/scripts/test.sh linux amd64
```

Expected: all C++ tests pass.

- [ ] **Step 3: Run validation collection**

Run:

```bash
pixi run validate --collect-only
```

Expected: validation tests collect successfully.

- [ ] **Step 4: Run GetObjects validation subset**

Run:

```bash
pixi run validate -k get_objects
```

Expected: GetObjects validation passes.

- [ ] **Step 5: Run pre-commit outside the sandbox**

Run outside the sandbox:

```bash
pre-commit run --all-files
```

Expected: all hooks pass. Include any lockfile or formatting updates produced by hooks in the final diff if they are relevant.

## Self-Review

- Spec coverage: the plan covers both `AdbcStatementExecuteQuery` and `DriverConnectionGetObjects`, avoids deprecated C streaming APIs, converts one chunk per `get_next`, preserves Arrow C Stream behavior, and documents DuckDB's one-active-stream constraint.
- Placeholder scan: no task contains unresolved placeholders.
- Type consistency: the plan consistently uses `ExecuteDuckDbStreamingArrowQuery`, `DuckDbArrowQueryResult`, `duckdb::QueryResult`, `duckdb::DataChunk`, and Arrow C Stream callbacks.
