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

# Quack ADBC Driver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a CMake 4 C++ project that produces `libadbc_driver_quack`, an ADBC C API shared library that delegates unbound SQL to a remote DuckDB server through Quack.

**Architecture:** The root CMake project fetches DuckDB 1.5.2, uriparser, and Googletest. The driver vendors Apache Arrow ADBC's `adbc.h`, parses `uri=quack://host:port/?token=...` with uriparser, opens a local in-memory DuckDB client, loads Quack, attaches the remote endpoint as `remote`, and executes caller SQL through `remote.query('<caller-sql>')`.

**Tech Stack:** CMake 4, C++20, DuckDB C API, Apache Arrow ADBC C header, uriparser, Googletest, CTest.

---

## File Structure

- Create `CMakeLists.txt`: root project, CMake 4 requirement, FetchContent setup, DuckDB patch application, dependency targets, test enablement.
- Create `cmake/patches/duckdb-disable-adbc.patch`: patch that removes DuckDB's own ADBC extension/module build registration.
- Create `src/CMakeLists.txt`: shared library target and private object library for testable helpers.
- Create `third_party/arrow-adbc/include/arrow-adbc/adbc.h`: vendored Apache Arrow ADBC C API header from the `apache-arrow-adbc-23` tag.
- Create `third_party/arrow-adbc/LICENSE.txt`: Apache Arrow ADBC license file from the same tag as the vendored header.
- Create `third_party/arrow-adbc/NOTICE.txt`: Apache Arrow ADBC notice file from the same tag as the vendored header.
- Create `src/adbc_driver_quack.cc`: exported ADBC lifecycle, connection, statement, and error functions.
- Create `src/quack_uri.h`: parsed URI model and parser declaration.
- Create `src/quack_uri.cc`: uriparser-backed parser and percent decoding.
- Create `src/sql_escape.h`: SQL string literal escaping declarations.
- Create `src/sql_escape.cc`: DuckDB SQL literal escaping and `remote.query` wrapper construction.
- Create `tests/CMakeLists.txt`: Googletest FetchContent and CTest registration.
- Create `tests/quack_uri_test.cc`: URI parser tests.
- Create `tests/sql_escape_test.cc`: SQL escaping and `remote.query` wrapping tests.
- Create `tests/adbc_driver_test.cc`: API behavior tests for parameter binding status and exported C function availability.
- Create `tests/exported_symbols_test.cc`: dynamic library symbol lookup test.

## Task 1: Root CMake and Dependency Skeleton

**Files:**
- Create: `CMakeLists.txt`
- Create: `cmake/patches/duckdb-disable-adbc.patch`
- Create: `src/CMakeLists.txt`
- Create: `tests/CMakeLists.txt`

- [ ] **Step 1: Write the failing configure test**

Run:

```bash
cmake -S . -B build
```

Expected: FAIL because no root `CMakeLists.txt` exists or no configured targets exist.

- [ ] **Step 2: Add root CMake project**

Create `CMakeLists.txt` with:

```cmake
cmake_minimum_required(VERSION 4.0)

project(adbc_driver_quack
  VERSION 0.1.0
  DESCRIPTION "ADBC driver for remote DuckDB via Quack"
  LANGUAGES C CXX)

include(FetchContent)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

FetchContent_Declare(
  duckdb
  GIT_REPOSITORY https://github.com/duckdb/duckdb.git
  GIT_TAG v1.5.2
  PATCH_COMMAND ${CMAKE_COMMAND} -E chdir <SOURCE_DIR> git apply --ignore-whitespace ${CMAKE_CURRENT_LIST_DIR}/cmake/patches/duckdb-disable-adbc.patch
)

FetchContent_Declare(
  uriparser
  GIT_REPOSITORY https://github.com/uriparser/uriparser.git
  GIT_TAG uriparser-0.9.8
)

set(URIPARSER_BUILD_DOCS OFF CACHE BOOL "" FORCE)
set(URIPARSER_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(URIPARSER_BUILD_TOOLS OFF CACHE BOOL "" FORCE)

FetchContent_MakeAvailable(duckdb uriparser)

add_subdirectory(src)

include(CTest)
if(BUILD_TESTING)
  add_subdirectory(tests)
endif()
```

- [ ] **Step 3: Add initial DuckDB patch**

Create `cmake/patches/duckdb-disable-adbc.patch` with the exact patch needed for the DuckDB 1.5.2 tree. During implementation, inspect the fetched DuckDB source after the first configure failure and patch the file that registers the `adbc` extension or module. The finished patch must remove only that registration and must apply with `git apply --ignore-whitespace`.

- [ ] **Step 4: Add source target skeleton**

Create `src/CMakeLists.txt` with:

```cmake
add_library(adbc_driver_quack_helpers
  quack_uri.cc
  sql_escape.cc)

target_include_directories(adbc_driver_quack_helpers
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/../third_party/arrow-adbc/include)

target_link_libraries(adbc_driver_quack_helpers
  PUBLIC
    uriparser::uriparser)

add_library(adbc_driver_quack SHARED
  adbc_driver_quack.cc)

target_link_libraries(adbc_driver_quack
  PRIVATE
    adbc_driver_quack_helpers
    duckdb)

target_include_directories(adbc_driver_quack
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/../third_party/arrow-adbc/include)

set_target_properties(adbc_driver_quack PROPERTIES
  OUTPUT_NAME adbc_driver_quack
  CXX_VISIBILITY_PRESET hidden
  VISIBILITY_INLINES_HIDDEN YES)
```

- [ ] **Step 5: Add test CMake skeleton**

Create `tests/CMakeLists.txt` with:

```cmake
include(FetchContent)
include(GoogleTest)

FetchContent_Declare(
  googletest
  GIT_REPOSITORY https://github.com/google/googletest.git
  GIT_TAG v1.15.2
)

set(gtest_force_shared_crt ON CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(googletest)

add_executable(adbc_driver_quack_tests
  quack_uri_test.cc
  sql_escape_test.cc
  adbc_driver_test.cc
  exported_symbols_test.cc)

target_link_libraries(adbc_driver_quack_tests
  PRIVATE
    adbc_driver_quack
    adbc_driver_quack_helpers
    GTest::gtest_main
    ${CMAKE_DL_LIBS})

target_compile_definitions(adbc_driver_quack_tests
  PRIVATE
    ADBC_DRIVER_QUACK_LIBRARY_PATH="$<TARGET_FILE:adbc_driver_quack>")

gtest_discover_tests(adbc_driver_quack_tests)
```

- [ ] **Step 6: Run configure**

Run:

```bash
cmake -S . -B build -DCMAKE_BUILD_TYPE=Debug
```

Expected: FAIL until source files are added in later tasks, or PASS once the source and test files from the following tasks exist. If the DuckDB patch path is wrong, inspect `build/_deps/duckdb-src` and correct only `cmake/patches/duckdb-disable-adbc.patch`.

## Task 2: URI Parser Red-Green

**Files:**
- Create: `src/quack_uri.h`
- Create: `src/quack_uri.cc`
- Create: `tests/quack_uri_test.cc`

- [ ] **Step 1: Write failing URI tests**

Create `tests/quack_uri_test.cc` with:

```cpp
#include "quack_uri.h"

#include <gtest/gtest.h>

TEST(QuackUriTest, ParsesHostOnlyEndpoint) {
  auto parsed = adbc_driver_quack::ParseQuackUri("quack://localhost/");
  ASSERT_TRUE(parsed.ok) << parsed.error;
  EXPECT_EQ(parsed.endpoint, "quack:localhost");
  EXPECT_EQ(parsed.token, "");
}

TEST(QuackUriTest, ParsesHostPortEndpoint) {
  auto parsed = adbc_driver_quack::ParseQuackUri("quack://db.example.com:9842/?token=secret");
  ASSERT_TRUE(parsed.ok) << parsed.error;
  EXPECT_EQ(parsed.endpoint, "quack:db.example.com:9842");
  EXPECT_EQ(parsed.token, "secret");
}

TEST(QuackUriTest, DecodesToken) {
  auto parsed = adbc_driver_quack::ParseQuackUri("quack://db/?token=a%20b%27c");
  ASSERT_TRUE(parsed.ok) << parsed.error;
  EXPECT_EQ(parsed.endpoint, "quack:db");
  EXPECT_EQ(parsed.token, "a b'c");
}

TEST(QuackUriTest, RejectsInvalidScheme) {
  auto parsed = adbc_driver_quack::ParseQuackUri("http://db/?token=secret");
  EXPECT_FALSE(parsed.ok);
  EXPECT_NE(parsed.error.find("scheme"), std::string::npos);
}

TEST(QuackUriTest, RejectsMissingHost) {
  auto parsed = adbc_driver_quack::ParseQuackUri("quack:///?token=secret");
  EXPECT_FALSE(parsed.ok);
  EXPECT_NE(parsed.error.find("host"), std::string::npos);
}
```

- [ ] **Step 2: Run tests and verify red**

Run:

```bash
cmake --build build --target adbc_driver_quack_tests
```

Expected: FAIL because `quack_uri.h` and `ParseQuackUri` do not exist.

- [ ] **Step 3: Implement URI parser**

Create `src/quack_uri.h` with:

```cpp
#pragma once

#include <string>
#include <string_view>

namespace adbc_driver_quack {

struct ParsedQuackUri {
  bool ok = false;
  std::string endpoint;
  std::string token;
  std::string error;
};

ParsedQuackUri ParseQuackUri(std::string_view uri);

}  // namespace adbc_driver_quack
```

Create `src/quack_uri.cc` using uriparser's `uriParseSingleUriA`, `UriUriA`, and `uriFreeUriMembersA`. Convert the incoming `std::string_view` to a null-terminated `std::string` for uriparser. Extract scheme, host text, port text, and query text with range copies. Reject non-`quack` schemes and empty hosts. Parse `token` from query pairs using `uriDissectQueryMallocA`, URL-decoding query values through uriparser APIs. Build `endpoint` as `quack:<host>` plus `:<port>` when a port is present.

- [ ] **Step 4: Run URI tests and verify green**

Run:

```bash
cmake --build build --target adbc_driver_quack_tests
ctest --test-dir build --output-on-failure -R QuackUriTest
```

Expected: PASS for all `QuackUriTest` cases.

## Task 3: SQL Escaping and Delegation Wrapper

**Files:**
- Create: `src/sql_escape.h`
- Create: `src/sql_escape.cc`
- Create: `tests/sql_escape_test.cc`

- [ ] **Step 1: Write failing SQL escaping tests**

Create `tests/sql_escape_test.cc` with:

```cpp
#include "sql_escape.h"

#include <gtest/gtest.h>

TEST(SqlEscapeTest, EscapesSingleQuotes) {
  EXPECT_EQ(adbc_driver_quack::DuckDbSqlStringLiteral("SELECT 'x'"),
            "'SELECT ''x'''");
}

TEST(SqlEscapeTest, BuildsRemoteQueryWrapper) {
  EXPECT_EQ(adbc_driver_quack::BuildRemoteQuerySql("SELECT 1"),
            "SELECT * FROM remote.query('SELECT 1')");
}

TEST(SqlEscapeTest, BuildsRemoteQueryWrapperWithEscapedSql) {
  EXPECT_EQ(adbc_driver_quack::BuildRemoteQuerySql("SELECT 'a'"),
            "SELECT * FROM remote.query('SELECT ''a''')");
}
```

- [ ] **Step 2: Run tests and verify red**

Run:

```bash
cmake --build build --target adbc_driver_quack_tests
```

Expected: FAIL because `sql_escape.h` and wrapper helpers do not exist.

- [ ] **Step 3: Implement SQL helpers**

Create `src/sql_escape.h` with:

```cpp
#pragma once

#include <string>
#include <string_view>

namespace adbc_driver_quack {

std::string DuckDbSqlStringLiteral(std::string_view value);
std::string BuildRemoteQuerySql(std::string_view caller_sql);

}  // namespace adbc_driver_quack
```

Create `src/sql_escape.cc` with:

```cpp
#include "sql_escape.h"

namespace adbc_driver_quack {

std::string DuckDbSqlStringLiteral(std::string_view value) {
  std::string escaped;
  escaped.reserve(value.size() + 2);
  escaped.push_back('\'');
  for (char c : value) {
    if (c == '\'') {
      escaped.push_back('\'');
    }
    escaped.push_back(c);
  }
  escaped.push_back('\'');
  return escaped;
}

std::string BuildRemoteQuerySql(std::string_view caller_sql) {
  return "SELECT * FROM remote.query(" + DuckDbSqlStringLiteral(caller_sql) + ")";
}

}  // namespace adbc_driver_quack
```

- [ ] **Step 4: Run SQL helper tests and verify green**

Run:

```bash
cmake --build build --target adbc_driver_quack_tests
ctest --test-dir build --output-on-failure -R SqlEscapeTest
```

Expected: PASS for all `SqlEscapeTest` cases.

## Task 4: Vendor Apache Arrow ADBC Header

**Files:**
- Create: `third_party/arrow-adbc/include/arrow-adbc/adbc.h`
- Create: `third_party/arrow-adbc/LICENSE.txt`
- Create: `third_party/arrow-adbc/NOTICE.txt`
- Modify: `src/CMakeLists.txt`
- Modify: `tests/CMakeLists.txt`

- [ ] **Step 1: Fetch the vendored header and license files**

Create the vendor directory:

```bash
mkdir -p third_party/arrow-adbc/include/arrow-adbc
```

Fetch these files from the `apache-arrow-adbc-23` tag:

```bash
curl -L -o third_party/arrow-adbc/include/arrow-adbc/adbc.h https://raw.githubusercontent.com/apache/arrow-adbc/apache-arrow-adbc-23/c/include/arrow-adbc/adbc.h
curl -L -o third_party/arrow-adbc/LICENSE.txt https://raw.githubusercontent.com/apache/arrow-adbc/apache-arrow-adbc-23/LICENSE.txt
curl -L -o third_party/arrow-adbc/NOTICE.txt https://raw.githubusercontent.com/apache/arrow-adbc/apache-arrow-adbc-23/NOTICE.txt
```

- [ ] **Step 2: Verify the vendored header is usable**

Run:

```bash
test -s third_party/arrow-adbc/include/arrow-adbc/adbc.h
test -s third_party/arrow-adbc/LICENSE.txt
test -s third_party/arrow-adbc/NOTICE.txt
rg -n "struct AdbcDatabase|AdbcStatementExecuteQuery|ADBC_STATUS_NOT_IMPLEMENTED" third_party/arrow-adbc/include/arrow-adbc/adbc.h
```

Expected: PASS and show declarations for the ADBC database, statement execute function, and not-implemented status.

- [ ] **Step 3: Ensure CMake include paths expose vendored header**

Confirm `src/CMakeLists.txt` exposes `${CMAKE_CURRENT_SOURCE_DIR}/../third_party/arrow-adbc/include` on both `adbc_driver_quack_helpers` and `adbc_driver_quack`. Confirm `tests/CMakeLists.txt` links tests to `adbc_driver_quack`, which carries the public vendored include path.

## Task 5: Driver API Tests

**Files:**
- Create: `tests/adbc_driver_test.cc`

- [ ] **Step 1: Write failing ADBC API tests**

Create `tests/adbc_driver_test.cc` with:

```cpp
#include <arrow-adbc/adbc.h>

#include <gtest/gtest.h>

TEST(AdbcDriverTest, ParameterBindingApisReturnNotImplemented) {
  AdbcStatement statement = {};
  AdbcError error = {};

  EXPECT_EQ(AdbcStatementPrepare(&statement, &error), ADBC_STATUS_NOT_IMPLEMENTED);
  EXPECT_EQ(AdbcStatementBind(&statement, nullptr, nullptr, &error), ADBC_STATUS_NOT_IMPLEMENTED);
  EXPECT_EQ(AdbcStatementBindStream(&statement, nullptr, &error), ADBC_STATUS_NOT_IMPLEMENTED);
}
```

- [ ] **Step 2: Run tests and verify red**

Run:

```bash
cmake --build build --target adbc_driver_quack_tests
```

Expected: FAIL at link time because Apache Arrow ADBC declares the ADBC functions but `adbc_driver_quack.cc` does not define them yet.

## Task 6: ADBC Driver Implementation

**Files:**
- Create: `src/adbc_driver_quack.cc`

- [ ] **Step 1: Implement exported functions**

Create `src/adbc_driver_quack.cc` with:

- hidden C++ private structs `DatabaseState`, `ConnectionState`, and `StatementState`
- exported C functions marked with `__attribute__((visibility("default")))` on Unix-like builds
- `AdbcDatabaseNew`: zero-initialize private state
- `AdbcDatabaseSetOption`: accept only key `uri`; parse and store the raw URI and parsed endpoint/token
- `AdbcDatabaseInit`: require a parsed URI
- `AdbcConnectionNew`: initialize empty connection state
- `AdbcConnectionInit`: open DuckDB in-memory, run `INSTALL quack`, `LOAD quack`, optional `CREATE SECRET`, and `ATTACH '<endpoint>' AS remote`
- `AdbcStatementNew`: require an initialized connection
- `AdbcStatementSetSqlQuery`: store the caller SQL
- `AdbcStatementExecuteQuery`: call DuckDB with `BuildRemoteQuerySql(stored_sql)` and return `ADBC_STATUS_OK` on success
- `AdbcStatementPrepare`, `AdbcStatementBind`, `AdbcStatementBindStream`: return `ADBC_STATUS_NOT_IMPLEMENTED`
- release functions: free private state and set `private_data` to `nullptr`
- error helper: allocate `AdbcError::message`, set `vendor_code`, install release callback

Use DuckDB C API types from `duckdb.h`. If DuckDB result-to-Arrow stream export is not available from the linked C API target, return query success with `stream == nullptr` support first and return `ADBC_STATUS_NOT_IMPLEMENTED` when a non-null stream is requested; then add a focused note in the code comment explaining that Arrow stream export depends on the DuckDB C API surface available in 1.5.2.

- [ ] **Step 2: Run ADBC behavior test**

Run:

```bash
cmake --build build --target adbc_driver_quack_tests
ctest --test-dir build --output-on-failure -R AdbcDriverTest
```

Expected: PASS for parameter binding status without requiring a running Quack server.

## Task 7: Exported Symbol Test

**Files:**
- Create: `tests/exported_symbols_test.cc`

- [ ] **Step 1: Write failing exported symbol test**

Create `tests/exported_symbols_test.cc` with:

```cpp
#include <gtest/gtest.h>

#if defined(_WIN32)
#include <windows.h>
#else
#include <dlfcn.h>
#endif

namespace {

void* OpenLibrary(const char* path) {
#if defined(_WIN32)
  return reinterpret_cast<void*>(LoadLibraryA(path));
#else
  return dlopen(path, RTLD_NOW | RTLD_LOCAL);
#endif
}

void* FindSymbol(void* library, const char* name) {
#if defined(_WIN32)
  return reinterpret_cast<void*>(GetProcAddress(reinterpret_cast<HMODULE>(library), name));
#else
  return dlsym(library, name);
#endif
}

void CloseLibrary(void* library) {
#if defined(_WIN32)
  FreeLibrary(reinterpret_cast<HMODULE>(library));
#else
  dlclose(library);
#endif
}

}  // namespace

TEST(ExportedSymbolsTest, ExportsRequiredAdbcEntryPoints) {
  void* library = OpenLibrary(ADBC_DRIVER_QUACK_LIBRARY_PATH);
  ASSERT_NE(library, nullptr);

  const char* symbols[] = {
      "AdbcDatabaseNew",
      "AdbcDatabaseSetOption",
      "AdbcDatabaseInit",
      "AdbcDatabaseRelease",
      "AdbcConnectionNew",
      "AdbcConnectionInit",
      "AdbcConnectionRelease",
      "AdbcStatementNew",
      "AdbcStatementSetSqlQuery",
      "AdbcStatementExecuteQuery",
      "AdbcStatementPrepare",
      "AdbcStatementBind",
      "AdbcStatementBindStream",
      "AdbcStatementRelease",
  };

  for (const char* symbol : symbols) {
    EXPECT_NE(FindSymbol(library, symbol), nullptr) << symbol;
  }

  CloseLibrary(library);
}
```

- [ ] **Step 2: Run exported symbol test**

Run:

```bash
cmake --build build --target adbc_driver_quack_tests
ctest --test-dir build --output-on-failure -R ExportedSymbolsTest
```

Expected: PASS after exported visibility attributes are applied to all required ADBC functions.

## Task 8: Build Verification and CTest Integration

**Files:**
- Modify: `CMakeLists.txt`
- Modify: `src/CMakeLists.txt`
- Modify: `tests/CMakeLists.txt`

- [ ] **Step 1: Configure clean build**

Run:

```bash
cmake -S . -B build -DCMAKE_BUILD_TYPE=Debug
```

Expected: PASS with DuckDB, uriparser, and Googletest populated through FetchContent.

- [ ] **Step 2: Build driver and tests**

Run:

```bash
cmake --build build --target adbc_driver_quack adbc_driver_quack_tests
```

Expected: PASS and produce a shared library named `libadbc_driver_quack` on Linux.

- [ ] **Step 3: Run full CTest suite**

Run:

```bash
ctest --test-dir build --output-on-failure
```

Expected: PASS for URI parsing, SQL wrapper, ADBC API behavior, and exported symbol tests.

- [ ] **Step 4: Inspect library output name**

Run:

```bash
cmake --build build --target adbc_driver_quack
ls build/src/libadbc_driver_quack*
```

Expected: output includes `build/src/libadbc_driver_quack.so` on Linux or the platform-equivalent shared library suffix.

## Task 9: Final Review

**Files:**
- Review all created files.

- [ ] **Step 1: Check git diff**

Run:

```bash
git diff --stat
git diff -- CMakeLists.txt cmake src tests third_party docs/superpowers/plans/2026-05-12-quack-adbc-driver.md docs/superpowers/specs/2026-05-12-quack-adbc-driver-design.md
```

Expected: changes are limited to the CMake project, source files, tests, vendored ADBC header and license files, DuckDB patch, spec update, and this plan.

- [ ] **Step 2: Re-run verification before completion**

Run:

```bash
cmake -S . -B build -DCMAKE_BUILD_TYPE=Debug
cmake --build build --target adbc_driver_quack adbc_driver_quack_tests
ctest --test-dir build --output-on-failure
```

Expected: all commands pass, or failures are documented with the exact failing command and first actionable error.

## Self-Review

- Spec coverage: root CMake 4 project, C++20, DuckDB 1.5.2 FetchContent, DuckDB ADBC patch, uriparser FetchContent, vendored Apache Arrow ADBC header, Googletest FetchContent, CTest, shared library naming, ADBC exports, Quack attach, `remote.query` delegation, and parameter binding non-support all map to tasks.
- Placeholder scan: the plan contains no open behavioral requirements. The DuckDB patch task necessarily depends on the fetched DuckDB 1.5.2 file layout and constrains the implementation to a single registration removal.
- Type consistency: helper names, test names, and ADBC function names are consistent across tasks.
