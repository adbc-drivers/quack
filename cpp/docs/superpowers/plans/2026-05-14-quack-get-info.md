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

# Quack GetInfo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement ADBC `ConnectionGetInfo` for the Quack C++ driver and unskip the validation suite GetInfo tests.

**Architecture:** Add CMake-owned build metadata, fetch Apache Arrow nanoarrow with renamed symbols, generate a config header, add a focused nanoarrow-backed Arrow C Stream producer for the ADBC GetInfo result shape, then wire it into the driver callback table and exported C API. The validation suite remains the red test: first remove the local skips, rebuild, and verify `test_get_info` fails because the driver does not implement GetInfo yet.

**Tech Stack:** C++20, Arrow C Data Interface, Apache Arrow nanoarrow, ADBC C API, DuckDB C API, CMake, GoogleTest, Pixi validation suite, pre-commit.

---

## File Structure

- Modify `validation/tests/test_connection.py`: remove the two local skip overrides so inherited validation tests run.
- Modify `CMakeLists.txt`: define reusable DuckDB/driver version variables, use the DuckDB variable for `FetchContent`, and generate a config header.
- Modify `license.tpl`: attribute Apache Arrow nanoarrow.
- Create `src/adbc_driver_quack_config.h.in`: template for build-time metadata consumed by the driver.
- Modify `src/adbc_driver_quack.cc`: declare, wire, and export `DriverConnectionGetInfo`/`AdbcConnectionGetInfo`.
- Create `src/get_info_stream.h`: expose a small helper that builds an `ArrowArrayStream` for GetInfo rows.
- Create `src/get_info_stream.cc`: build the Arrow C Data schema/array/stream through nanoarrow and handle info-code filtering.
- Modify `src/CMakeLists.txt`: compile the new helper, include the generated header directory, and link nanoarrow privately.
- Modify `tests/exported_symbols_test.cc`: update existing callback/export expectations now that `ConnectionGetInfo` is supported.
- Do not add a new red unit test. The unskipped validation tests are the failing test for this change.
- Do not create validation `.txtcase` files for this task. GetInfo tests are not query-based and do not need SQL dialect overrides.

## Supported Info Codes

Return rows for these recognized ADBC info codes:

- `ADBC_INFO_VENDOR_NAME`: string `"DuckDB Quack"`
- `ADBC_INFO_VENDOR_VERSION`: string from the remote DuckDB server, fetched through Quack with `SELECT version()` and normalized by removing one leading `v`, expected to match validation quirk value `"1.5.2"` for the default test environment
- `ADBC_INFO_VENDOR_SQL`: bool `true`
- `ADBC_INFO_VENDOR_SUBSTRAIT`: bool `false`
- `ADBC_INFO_DRIVER_NAME`: string `"ADBC Driver for DuckDB Quack"`
- `ADBC_INFO_DRIVER_VERSION`: string from generated config value `ADBC_DRIVER_QUACK_VERSION`, templated from CMake project version
- `ADBC_INFO_DRIVER_ARROW_VERSION`: string from generated config value `ADBC_DRIVER_QUACK_DUCKDB_TAG`, matching the embedded DuckDB Arrow export provider version format expected by validation
- `ADBC_INFO_DRIVER_ADBC_VERSION`: int64 `ADBC_VERSION_1_1_0`

When `info_codes == nullptr`, return all supported rows in the order above. When a list is provided, return only requested supported codes, preserving the supported-order output. Omit unsupported or unrecognized requested codes.

Do not use `duckdb_library_version()` for `ADBC_INFO_VENDOR_VERSION`: that reports the embedded local DuckDB client. In this driver, the ADBC database vendor is the remote DuckDB server reached through Quack.

## Arrow Result Shape

Produce an `ArrowArrayStream` with this schema from `adbc.h`:

```text
struct<
  info_name: uint32 not null,
  info_value: dense_union<
    string_value: utf8 = 0,
    bool_value: bool = 1,
    int64_value: int64 = 2,
    int32_bitmask: int32 = 3,
    string_list: list<utf8> = 4,
    int32_to_int32_list_map: map<int32, list<int32>> = 5
  >
>
```

The initial implementation only emits union children for string, bool, and int64 values. The remaining children must still exist in the schema and array because the ADBC schema defines the full union.

### Task 1: Unskip Validation GetInfo Tests and Capture RED

**Files:**
- Modify: `validation/tests/test_connection.py`

- [ ] **Step 1: Remove local skip overrides**

Replace the file body after imports with:

```python
import adbc_drivers_validation.tests.connection as connection_tests

from .quack import get_quirks


def pytest_generate_tests(metafunc) -> None:
    quirks = [get_quirks(metafunc.config.getoption("vendor_version"))]
    return connection_tests.generate_tests(quirks, metafunc)


class TestConnection(connection_tests.TestConnection):
    pass
```

- [ ] **Step 2: Rebuild before running validation**

Run:

```bash
./ci/scripts/build.sh test linux amd64
```

Expected: build completes successfully.

- [ ] **Step 3: Run the unskipped red validation tests**

Run:

```bash
pixi run validate -k get_info
```

Expected: `test_get_info` and/or `test_get_info_arrow_version` fail because the driver does not implement `AdbcConnectionGetInfo` yet. The exact failure may come from a null driver callback or an ADBC `NOT_IMPLEMENTED` error.

### Task 2: Add GetInfo Stream Helper

**Files:**
- Modify: `CMakeLists.txt`
- Modify: `license.tpl`
- Create: `src/adbc_driver_quack_config.h.in`
- Create: `src/get_info_stream.h`
- Create: `src/get_info_stream.cc`
- Modify: `src/CMakeLists.txt`

- [ ] **Step 1: Move version metadata into CMake variables**

Modify the root `CMakeLists.txt` immediately after the `project(...)` block:

```cmake
set(ADBC_DRIVER_QUACK_DUCKDB_VERSION
    "1.5.2"
    CACHE STRING "DuckDB version used by the Quack driver")
set(ADBC_DRIVER_QUACK_DUCKDB_TAG
    "v${ADBC_DRIVER_QUACK_DUCKDB_VERSION}"
    CACHE STRING "DuckDB Git tag used by the Quack driver")
set(ADBC_DRIVER_QUACK_NANOARROW_TAG
    "apache-arrow-nanoarrow-0.8.0"
    CACHE STRING "Apache Arrow nanoarrow Git tag used by the Quack driver")
```

Change the DuckDB `FetchContent_Declare` block from:

```cmake
  GIT_TAG v1.5.2
```

to:

```cmake
  GIT_TAG ${ADBC_DRIVER_QUACK_DUCKDB_TAG}
```

Before the `FetchContent_Declare(...)` calls, set nanoarrow options so symbols are renamed and extra components are disabled:

```cmake
set(NANOARROW_NAMESPACE
    "AdbcDriverQuack"
    CACHE STRING "Prefix for nanoarrow symbols embedded in the Quack driver")
set(NANOARROW_BUILD_APPS
    OFF
    CACHE BOOL "" FORCE)
set(NANOARROW_BUILD_TESTS
    OFF
    CACHE BOOL "" FORCE)
set(NANOARROW_BUILD_BENCHMARKS
    OFF
    CACHE BOOL "" FORCE)
set(NANOARROW_BUILD_INTEGRATION_TESTS
    OFF
    CACHE BOOL "" FORCE)
set(NANOARROW_INSTALL_SHARED
    OFF
    CACHE BOOL "" FORCE)
```

Add nanoarrow as a pinned `FetchContent` dependency:

```cmake
FetchContent_Declare(
  nanoarrow
  GIT_REPOSITORY https://github.com/apache/arrow-nanoarrow.git
  GIT_TAG ${ADBC_DRIVER_QUACK_NANOARROW_TAG})
```

Update dependency materialization:

```cmake
FetchContent_MakeAvailable(adbc_driver_quack_quack duckdb nanoarrow uriparser)
```

Add this `configure_file` call after the version variables and before `add_subdirectory(src)`:

```cmake
configure_file(
  ${CMAKE_CURRENT_LIST_DIR}/src/adbc_driver_quack_config.h.in
  ${CMAKE_CURRENT_BINARY_DIR}/generated/adbc_driver_quack_config.h @ONLY)
```

- [ ] **Step 2: Attribute nanoarrow in the license template**

Add this third-party entry under `Third Party Licenses` in `license.tpl`:

```text
Apache Arrow nanoarrow:

Apache Arrow nanoarrow
Copyright 2023 The Apache Software Foundation

This product includes software developed at
The Apache Software Foundation (http://www.apache.org/).
```

The repo already carries the Apache License text at the top of `license.tpl`; do not duplicate the full Apache 2.0 license text for nanoarrow.

- [ ] **Step 3: Add the generated config header template**

Create `src/adbc_driver_quack_config.h.in`:

```cpp
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

#pragma once

#define ADBC_DRIVER_QUACK_VERSION "@PROJECT_VERSION@"
#define ADBC_DRIVER_QUACK_DUCKDB_VERSION "@ADBC_DRIVER_QUACK_DUCKDB_VERSION@"
#define ADBC_DRIVER_QUACK_DUCKDB_TAG "@ADBC_DRIVER_QUACK_DUCKDB_TAG@"
#define ADBC_DRIVER_QUACK_NANOARROW_TAG "@ADBC_DRIVER_QUACK_NANOARROW_TAG@"
```

- [ ] **Step 4: Add the public helper header**

Create `src/get_info_stream.h`:

```cpp
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

#pragma once

#include <arrow-adbc/adbc.h>

#include <cstddef>
#include <cstdint>
#include <string>

namespace adbc_driver_quack {

struct GetInfoResult {
  AdbcStatusCode status = ADBC_STATUS_OK;
  std::string message;
};

GetInfoResult BuildGetInfoStream(std::string const& remote_vendor_version,
                                  uint32_t const* info_codes,
                                  std::size_t info_codes_length,
                                  ArrowArrayStream* out);

}  // namespace adbc_driver_quack
```

- [ ] **Step 5: Add the helper implementation**

Create `src/get_info_stream.cc` using nanoarrow instead of manually allocating Arrow C Data buffers. Include:

```cpp
#include <nanoarrow/nanoarrow.h>
```

Responsibilities:

- Define a private `InfoRow` model with `uint32_t code`, value kind, and value storage.
- Use nanoarrow to construct and own the schema, array, and stream callbacks.
- Use `ArrowBasicArrayStreamInit(out, &schema, 1)` and `ArrowBasicArrayStreamSetArray(out, 0, &array)` so nanoarrow owns release logic.
- Return `ADBC_STATUS_INVALID_ARGUMENT` if `out == nullptr`.
- Return `ADBC_STATUS_UNKNOWN` with a useful nanoarrow error message if a nanoarrow call fails.

Use these exact constants and helper behavior:

```cpp
#include "adbc_driver_quack_config.h"

namespace {

constexpr char kDriverName[] = "ADBC Driver for DuckDB Quack";
constexpr char kVendorName[] = "DuckDB Quack";
constexpr char kDriverArrowVersion[] = ADBC_DRIVER_QUACK_DUCKDB_TAG;

std::string NormalizeDuckDbVersion(char const* version) {
  if (version == nullptr || version[0] == '\0') {
    return "unknown";
  }
  std::string normalized(version);
  if (!normalized.empty() && normalized[0] == 'v') {
    normalized.erase(0, 1);
  }
  return normalized;
}

}  // namespace
```

Use `NormalizeDuckDbVersion(remote_vendor_version.c_str())` as the value for `ADBC_INFO_VENDOR_VERSION`; do not call `duckdb_library_version()` in this helper.

Build the schema with nanoarrow:

- Top-level: `ArrowSchemaInit(&schema)` then `ArrowSchemaSetTypeStruct(&schema, 2)`.
- `schema.children[0]`: `ArrowSchemaSetType(..., NANOARROW_TYPE_UINT32)` and `ArrowSchemaSetName(..., "info_name")`; clear nullable flags for this field.
- `schema.children[1]`: `ArrowSchemaSetTypeUnion(..., NANOARROW_TYPE_DENSE_UNION, 6)` and `ArrowSchemaSetName(..., "info_value")`.
- Union children: `string_value` (`NANOARROW_TYPE_STRING`), `bool_value` (`NANOARROW_TYPE_BOOL`), `int64_value` (`NANOARROW_TYPE_INT64`), `int32_bitmask` (`NANOARROW_TYPE_INT32`), `string_list` (`NANOARROW_TYPE_LIST` with string item), and `int32_to_int32_list_map` (`NANOARROW_TYPE_MAP` with int32 key and list<int32> value).

Build the array with nanoarrow:

- `ArrowArrayInitFromSchema(&array, &schema, &error)`.
- `ArrowArrayStartAppending(&array)`.
- For each row, append the code to `array.children[0]`.
- For a string row, append the string to `array.children[1]->children[0]`, then call `ArrowArrayFinishUnionElement(array.children[1], 0)`.
- For a bool row, append `0` or `1` to `array.children[1]->children[1]`, then call `ArrowArrayFinishUnionElement(array.children[1], 1)`.
- For an int64 row, append the value to `array.children[1]->children[2]`, then call `ArrowArrayFinishUnionElement(array.children[1], 2)`.
- After appending the row children, call `ArrowArrayFinishElement(&array)` for the top-level struct row.
- Do not append to union children 3, 4, or 5; nanoarrow should keep them as existing empty child arrays because the schema contains them.
- Finish with `ArrowArrayFinishBuildingDefault(&array, &error)`.

- [ ] **Step 6: Add helper, generated header directory, and nanoarrow linkage to the build**

Modify `src/CMakeLists.txt`:

```cmake
add_library(adbc_driver_quack_helpers STATIC duckdb_arrow_stream.cc
                                             get_info_stream.cc quack_uri.cc
                                             sql_escape.cc)
```

Add `${CMAKE_CURRENT_BINARY_DIR}/../generated` to the helper include directories, so `get_info_stream.cc` can include the configured header:

```cmake
target_include_directories(
  adbc_driver_quack_helpers
  PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}
         ${CMAKE_CURRENT_SOURCE_DIR}/../third_party/arrow-adbc/include
         ${ADBC_DRIVER_QUACK_DUCKDB_INCLUDE_DIR}
         ${CMAKE_CURRENT_BINARY_DIR}/../generated)
```

Link nanoarrow privately into the helper target:

```cmake
target_link_libraries(adbc_driver_quack_helpers
                      PUBLIC ${ADBC_DRIVER_QUACK_DUCKDB_LIBRARIES} uriparser
                      PRIVATE nanoarrow_static)
```

Keep nanoarrow headers private to this driver; do not add them to public driver APIs.

- [ ] **Step 7: Build to catch C++ and Arrow C Data mistakes**

Run:

```bash
cmake --build build/ci-test-linux-amd64
```

Expected: compile errors, if any, point only at the new helper or CMake wiring. Fix them before continuing.

### Task 3: Wire GetInfo Into the Driver

**Files:**
- Modify: `src/adbc_driver_quack.cc`

- [ ] **Step 1: Include the helper**

Add:

```cpp
#include "get_info_stream.h"
```

- [ ] **Step 2: Declare the driver callback**

In the `extern "C"` declarations near the other connection functions, add:

```cpp
AdbcStatusCode DriverConnectionGetInfo(AdbcConnection* connection,
                                       uint32_t const* info_codes,
                                       size_t info_codes_length,
                                       ArrowArrayStream* out,
                                       AdbcError* error);
```

- [ ] **Step 3: Populate the driver table**

In `InitDriver`, after connection callbacks are assigned, add:

```cpp
driver->ConnectionGetInfo = DriverConnectionGetInfo;
```

- [ ] **Step 4: Implement the callback**

Add this helper near `RunDuckDbQuery`:

```cpp
AdbcStatusCode QueryRemoteDuckDbVersion(ConnectionState* state,
                                        std::string* version,
                                        AdbcError* error) {
  if (state == nullptr || state->connection == nullptr || version == nullptr) {
    return InvalidState(error, "connection is not initialized");
  }

  duckdb_result result;
  std::string const remote_sql =
      adbc_driver_quack::BuildRemoteQuerySql("SELECT version()");
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

  if (duckdb_row_count(&result) < 1 || duckdb_column_count(&result) < 1) {
    duckdb_destroy_result(&result);
    return IoError(error, "remote DuckDB version query returned no rows");
  }

  char* raw_version = duckdb_value_varchar(&result, 0, 0);
  if (raw_version == nullptr) {
    duckdb_destroy_result(&result);
    return IoError(error, "remote DuckDB version query returned null");
  }

  *version = raw_version;
  duckdb_free(raw_version);
  duckdb_destroy_result(&result);
  return ADBC_STATUS_OK;
}
```

Then add this implementation near the other connection functions:

```cpp
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
      QueryRemoteDuckDbVersion(state, &remote_vendor_version, error);
  if (status != ADBC_STATUS_OK) {
    return status;
  }

  auto result = adbc_driver_quack::BuildGetInfoStream(
      remote_vendor_version, info_codes, info_codes_length, out);
  if (result.status != ADBC_STATUS_OK) {
    return SetError(error, result.status, std::move(result.message));
  }
  return Ok(error);
}
```

- [ ] **Step 5: Export the direct C API symbol**

Add this exported wrapper near the other `ADBC_EXPORT` connection wrappers:

```cpp
ADBC_EXPORT AdbcStatusCode AdbcConnectionGetInfo(
    AdbcConnection* connection, uint32_t const* info_codes,
    size_t info_codes_length, ArrowArrayStream* out, AdbcError* error) {
  return DriverConnectionGetInfo(connection, info_codes, info_codes_length, out,
                                 error);
}
```

- [ ] **Step 6: Rebuild**

Run:

```bash
cmake --build build/ci-test-linux-amd64
```

Expected: the driver library links successfully.

### Task 4: Update Existing C++ Expectations

**Files:**
- Modify: `tests/exported_symbols_test.cc`

- [ ] **Step 1: Add exported symbol expectation**

In `ExportsRequiredAdbcEntryPoints`, add `"AdbcConnectionGetInfo"` to the `symbols` array next to the other connection symbols:

```cpp
"AdbcConnectionGetInfo",
```

- [ ] **Step 2: Require callback population**

In `DriverInitFunctionsPopulateDriverTable`, add:

```cpp
EXPECT_NE(driver.ConnectionGetInfo, nullptr) << init_symbol;
```

- [ ] **Step 3: Remove unsupported callback expectation**

In `DriverInitFunctionsClearUnsupportedCallbacks`, remove:

```cpp
EXPECT_EQ(driver.ConnectionGetInfo, nullptr) << init_symbol;
```

- [ ] **Step 4: Run C++ unit tests**

Run:

```bash
cmake --build build/ci-test-linux-amd64
```

Then run:

```bash
ctest --test-dir build/ci-test-linux-amd64 --output-on-failure
```

Expected: all C++ tests pass.

### Task 5: Run GREEN Validation

**Files:**
- No new files.

- [ ] **Step 1: Rebuild through project workflow before validation**

Run:

```bash
./ci/scripts/build.sh test linux amd64
```

Expected: build completes successfully.

- [ ] **Step 2: Run targeted GetInfo validation**

Run:

```bash
pixi run validate -k get_info
```

Expected: `test_get_info` and `test_get_info_arrow_version` pass.

- [ ] **Step 3: Run full validation if targeted tests pass**

Run:

```bash
pixi run validate
```

Expected: existing unsupported features remain skipped or xfailed according to `validation/tests/quack.py`; no new GetInfo failures remain.

### Task 6: Final Formatting and Repository Checks

**Files:**
- No planned source changes beyond formatting.

- [ ] **Step 1: Run formatter and hooks outside sandbox**

Run:

```bash
pre-commit run --all-files
```

Expected: all hooks pass. If hooks modify files, inspect the diff and rerun until clean.

- [ ] **Step 2: Check final diff**

Run:

```bash
git diff -- CMakeLists.txt license.tpl validation/tests/test_connection.py src/adbc_driver_quack_config.h.in src/adbc_driver_quack.cc src/get_info_stream.h src/get_info_stream.cc src/CMakeLists.txt tests/CMakeLists.txt tests/exported_symbols_test.cc docs/superpowers/plans/2026-05-14-quack-get-info.md
```

Expected: diff contains only CMake version/header generation, nanoarrow FetchContent/linkage, nanoarrow license attribution, GetInfo implementation, validation unskip, existing test expectation updates, the GoogleTest discovery timeout build fix, build wiring, and this plan.

- [ ] **Step 3: Do not stage or commit**

No `git add` or `git commit` should be run unless the user explicitly asks for a commit.

## Plan Self-Review

- Spec coverage: the plan unskips both validation GetInfo tests, implements the driver callback, returns remote DuckDB vendor metadata, templates driver/DuckDB/nanoarrow build metadata through CMake, uses nanoarrow with renamed symbols for Arrow array/stream construction, attributes nanoarrow in `license.tpl`, updates existing C++ expectations, rebuilds before validation, and runs final formatting.
- Placeholder scan: no planned step depends on unspecified files or future decisions.
- Type consistency: all callback signatures match `third_party/arrow-adbc/include/arrow-adbc/adbc.h`; version and name values match `validation/tests/quack.py`.
