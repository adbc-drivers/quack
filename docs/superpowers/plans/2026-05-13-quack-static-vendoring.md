# Static Quack Vendoring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build Quack into the DuckDB library used by `libadbc_driver_quack` and remove runtime `INSTALL quack`.

**Architecture:** The root CMake project fetches Quack before configuring DuckDB, passes a local DuckDB extension config through `DUCKDB_EXTENSION_CONFIGS`, and links the driver against the resulting DuckDB target. Runtime connection setup keeps `LOAD quack` as static-extension activation and then runs the existing secret/attach flow.

**Tech Stack:** CMake 4, DuckDB `FetchContent`, DuckDB extension build system, C++20, Googletest, CTest.

---

## File Structure

- Modify `CMakeLists.txt`: fetch Quack, apply smaller DuckDB build flags, pass the Quack extension config into DuckDB before `FetchContent_MakeAvailable(duckdb uriparser)`.
- Create `cmake/duckdb/quack_extension_config.cmake`: register the fetched Quack source as a statically linked DuckDB extension and include Quack's upstream support extensions.
- Modify `src/adbc_driver_quack.cc`: add a small startup SQL helper and remove runtime `INSTALL quack` from connection initialization.
- Modify `tests/adbc_driver_test.cc`: add focused tests for startup SQL so the no-runtime-install contract is covered without a live server.
- Run existing tests through `ctest`.

## Task 1: Add A Testable Quack Startup SQL Seam

**Files:**
- Modify: `src/adbc_driver_quack.cc`
- Modify: `tests/adbc_driver_test.cc`

- [ ] **Step 1: Write the failing test**

Append these tests to `tests/adbc_driver_test.cc`:

```cpp
extern "C" const char* AdbcDriverQuackStartupSqlForTesting(size_t index);

TEST(AdbcDriverQuackTest, StartupSqlDoesNotInstallQuack) {
  for (size_t index = 0;; ++index) {
    const char* sql = AdbcDriverQuackStartupSqlForTesting(index);
    if (sql == nullptr) {
      break;
    }
    EXPECT_STRNE(sql, "INSTALL quack");
  }
}

TEST(AdbcDriverQuackTest, StartupSqlLoadsQuack) {
  ASSERT_STREQ(AdbcDriverQuackStartupSqlForTesting(0), "LOAD quack");
  ASSERT_EQ(AdbcDriverQuackStartupSqlForTesting(1), nullptr);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cmake --build build --target adbc_driver_quack_tests
```

Expected: link failure mentioning `AdbcDriverQuackStartupSqlForTesting`, because the test-only helper does not exist yet.

- [ ] **Step 3: Add the startup SQL helper and use it**

In `src/adbc_driver_quack.cc`, add this include near the other standard includes:

```cpp
#include <array>
```

Add this helper in the anonymous namespace after `RunDuckDbQuery`:

```cpp
constexpr std::array<const char*, 1> kQuackStartupSql = {"LOAD quack"};

AdbcStatusCode LoadQuackExtension(ConnectionState* state, AdbcError* error) {
  for (const char* sql : kQuackStartupSql) {
    AdbcStatusCode status = RunDuckDbQuery(state, sql, error);
    if (status != ADBC_STATUS_OK) {
      return status;
    }
  }
  return Ok(error);
}
```

Replace this block in `AdbcConnectionInit`:

```cpp
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
```

with:

```cpp
  AdbcStatusCode status = LoadQuackExtension(connection_state, error);
  if (status != ADBC_STATUS_OK) {
    CloseConnectionState(connection_state);
    return status;
  }
```

Add this exported test helper near the bottom of `src/adbc_driver_quack.cc`, inside `extern "C"` and before `AdbcDriverInit`:

```cpp
const char* AdbcDriverQuackStartupSqlForTesting(size_t index) {
  if (index >= kQuackStartupSql.size()) {
    return nullptr;
  }
  return kQuackStartupSql[index];
}
```

- [ ] **Step 4: Run test to verify it passes**

Run:

```bash
cmake --build build --target adbc_driver_quack_tests
build/tests/adbc_driver_quack_tests --gtest_filter='AdbcDriverQuackTest.StartupSql*'
```

Expected: both `AdbcDriverQuackTest.StartupSqlDoesNotInstallQuack` and `AdbcDriverQuackTest.StartupSqlLoadsQuack` pass.

- [ ] **Step 5: Commit**

Run:

```bash
git add src/adbc_driver_quack.cc tests/adbc_driver_test.cc
git commit -m "feat: remove runtime quack install"
```

## Task 2: Register Quack As A Static DuckDB Extension

**Files:**
- Modify: `CMakeLists.txt`
- Create: `cmake/duckdb/quack_extension_config.cmake`

- [ ] **Step 1: Write the extension config**

Create `cmake/duckdb/quack_extension_config.cmake` with:

```cmake
if(NOT DEFINED adbc_driver_quack_quack_SOURCE_DIR)
  message(FATAL_ERROR "adbc_driver_quack_quack_SOURCE_DIR must be defined before loading Quack")
endif()

duckdb_extension_load(quack
  SOURCE_DIR "${adbc_driver_quack_quack_SOURCE_DIR}"
)

duckdb_extension_load(json)
duckdb_extension_load(autocomplete)

duckdb_extension_load(httpfs
  GIT_URL https://github.com/duckdb/duckdb-httpfs
  GIT_TAG 7e86e7a5e5a1f01f458361bebdfa9b0a9a73a619
  APPLY_PATCHES
)
```

- [ ] **Step 2: Update root CMake dependency configuration**

In `CMakeLists.txt`, add this `FetchContent_Declare` after the DuckDB declaration:

```cmake
FetchContent_Declare(
  adbc_driver_quack_quack
  GIT_REPOSITORY https://github.com/duckdb/duckdb-quack.git
  GIT_TAG v1.5-variegata
)
```

Add this block before `FetchContent_MakeAvailable(duckdb uriparser)`:

```cmake
FetchContent_GetProperties(adbc_driver_quack_quack)
if(NOT adbc_driver_quack_quack_POPULATED)
  FetchContent_Populate(adbc_driver_quack_quack)
endif()

set(BUILD_SHELL OFF CACHE BOOL "" FORCE)
set(BUILD_UNITTESTS OFF CACHE BOOL "" FORCE)
set(ENABLE_UNITTEST_CPP_TESTS OFF CACHE BOOL "" FORCE)
set(BUILD_BENCHMARKS OFF CACHE BOOL "" FORCE)
set(DUCKDB_EXTENSION_CONFIGS
  "${CMAKE_CURRENT_LIST_DIR}/cmake/duckdb/quack_extension_config.cmake"
  CACHE STRING "" FORCE)
```

Keep the existing sanitizer and uriparser flags.

- [ ] **Step 3: Configure from a clean build directory**

Run:

```bash
cmake -S . -B build-static-quack -G Ninja
```

Expected: configure succeeds and the output includes Quack in the DuckDB extension list. If CMake reports missing `curl` or `openssl` dependencies, install or configure vcpkg dependencies and rerun the same command; do not remove static Quack registration.

- [ ] **Step 4: Build the test target**

Run:

```bash
cmake --build build-static-quack --target adbc_driver_quack_tests
```

Expected: build succeeds. If Quack fails because its submodule paths are missing, adjust the Quack fetch declaration to include required submodules:

```cmake
FetchContent_Declare(
  adbc_driver_quack_quack
  GIT_REPOSITORY https://github.com/duckdb/duckdb-quack.git
  GIT_TAG v1.5-variegata
  GIT_SUBMODULES duckdb extension-ci-tools
)
```

Then rerun configure and build.

- [ ] **Step 5: Commit**

Run:

```bash
git add CMakeLists.txt cmake/duckdb/quack_extension_config.cmake
git commit -m "build: vendor quack extension"
```

## Task 3: Verify Static Quack Activation

**Files:**
- Modify: `tests/duckdb_arrow_stream_test.cc`

- [ ] **Step 1: Write the static load test**

Append this test to `tests/duckdb_arrow_stream_test.cc`:

```cpp
TEST(DuckDbArrowStreamTest, LoadsStaticallyLinkedQuackExtension) {
  duckdb_database database = nullptr;
  ASSERT_EQ(duckdb_open(nullptr, &database), DuckDBSuccess);

  duckdb_connection connection = nullptr;
  ASSERT_EQ(duckdb_connect(database, &connection), DuckDBSuccess);

  duckdb_result result;
  ASSERT_EQ(duckdb_query(connection, "LOAD quack", &result), DuckDBSuccess)
      << duckdb_result_error(&result);
  duckdb_destroy_result(&result);

  duckdb_disconnect(&connection);
  duckdb_close(&database);
}
```

- [ ] **Step 2: Run test to verify it passes in the static build**

Run:

```bash
cmake --build build-static-quack --target adbc_driver_quack_tests
build-static-quack/tests/adbc_driver_quack_tests --gtest_filter='DuckDbArrowStreamTest.LoadsStaticallyLinkedQuackExtension'
```

Expected: test passes. A failure that mentions extension repository access means Quack was not statically registered into DuckDB.

- [ ] **Step 3: Commit**

Run:

```bash
git add tests/duckdb_arrow_stream_test.cc
git commit -m "test: verify static quack load"
```

## Task 4: Full Verification And Cleanup

**Files:**
- Verify: `CMakeLists.txt`
- Verify: `cmake/duckdb/quack_extension_config.cmake`
- Verify: `src/adbc_driver_quack.cc`
- Verify: `tests/adbc_driver_test.cc`
- Verify: `tests/duckdb_arrow_stream_test.cc`

- [ ] **Step 1: Run the full test suite**

Run:

```bash
ctest --test-dir build-static-quack --output-on-failure
```

Expected: all registered tests pass.

- [ ] **Step 2: Verify runtime install text is gone from source**

Run:

```bash
rg -n "INSTALL quack" CMakeLists.txt cmake src tests
```

Expected: no matches.

- [ ] **Step 3: Verify exported symbols remain scoped**

Run:

```bash
build-static-quack/tests/adbc_driver_quack_tests --gtest_filter='ExportedSymbolsTest.*'
```

Expected: exported symbol tests pass, including `AdbcDriverInit` and `AdbcDriverQuackInit`.

- [ ] **Step 4: Inspect local size impact**

Run:

```bash
du -sh build-static-quack/src/libadbc_driver_quack.so
```

Expected: command prints the shared library size. Record the size in the final response; do not fail the task based on size unless the build is obviously broken.

- [ ] **Step 5: Commit any verification-only test adjustments**

If Task 4 required source changes, run:

```bash
git add CMakeLists.txt cmake src tests
git commit -m "test: complete static quack verification"
```

If Task 4 required no source changes, skip this commit.

## Self-Review

- Spec coverage: The plan vendors Quack unconditionally, removes runtime `INSTALL quack`, preserves `LOAD quack` for static activation, keeps existing URI/secret/attach behavior, disables unused DuckDB build artifacts, and verifies exported symbols.
- Placeholder scan: No `TBD`, `TODO`, or unspecified test steps remain.
- Type consistency: The helper is declared and defined as `const char* AdbcDriverQuackStartupSqlForTesting(size_t index)`, and all tests use that signature.
