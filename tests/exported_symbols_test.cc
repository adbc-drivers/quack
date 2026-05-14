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

#include <arrow-adbc/adbc.h>
#include <gtest/gtest.h>

#include <cstring>

#if defined(_WIN32)
#include <windows.h>
#else
#include <dlfcn.h>
#endif

namespace {

void* OpenLibrary(char const* path) {
#if defined(_WIN32)
  return reinterpret_cast<void*>(LoadLibraryA(path));
#else
  return dlopen(path, RTLD_NOW | RTLD_LOCAL);
#endif
}

void* FindSymbol(void* library, char const* name) {
#if defined(_WIN32)
  return reinterpret_cast<void*>(
      GetProcAddress(reinterpret_cast<HMODULE>(library), name));
#else
  return dlsym(library, name);
#endif
}

template <typename Function>
Function FindFunction(void* library, char const* name) {
  return reinterpret_cast<Function>(FindSymbol(library, name));
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

  char const* symbols[] = {
      "AdbcDatabaseNew",          "AdbcDatabaseSetOption",
      "AdbcDatabaseInit",         "AdbcDatabaseRelease",
      "AdbcConnectionNew",        "AdbcConnectionInit",
      "AdbcConnectionRelease",    "AdbcStatementNew",
      "AdbcStatementSetSqlQuery", "AdbcStatementExecuteQuery",
      "AdbcStatementPrepare",     "AdbcStatementBind",
      "AdbcStatementBindStream",  "AdbcStatementRelease",
      "AdbcDriverInit",           "AdbcDriverQuackInit",
  };

  for (char const* symbol : symbols) {
    EXPECT_NE(FindSymbol(library, symbol), nullptr) << symbol;
  }

  CloseLibrary(library);
}

TEST(ExportedSymbolsTest, DriverInitFunctionsPopulateDriverTable) {
  void* library = OpenLibrary(ADBC_DRIVER_QUACK_LIBRARY_PATH);
  ASSERT_NE(library, nullptr);

  char const* init_symbols[] = {
      "AdbcDriverInit",
      "AdbcDriverQuackInit",
  };

  for (char const* init_symbol : init_symbols) {
    auto const init = FindFunction<AdbcDriverInitFunc>(library, init_symbol);
    ASSERT_NE(init, nullptr) << init_symbol;

    AdbcDriver driver = {};
    AdbcError error = ADBC_ERROR_INIT;
    EXPECT_EQ(init(ADBC_VERSION_1_1_0, &driver, &error), ADBC_STATUS_OK)
        << init_symbol;
    EXPECT_NE(driver.release, nullptr) << init_symbol;
    EXPECT_NE(driver.DatabaseNew, nullptr) << init_symbol;
    EXPECT_NE(driver.DatabaseSetOption, nullptr) << init_symbol;
    EXPECT_NE(driver.DatabaseInit, nullptr) << init_symbol;
    EXPECT_NE(driver.DatabaseRelease, nullptr) << init_symbol;
    EXPECT_NE(driver.ConnectionNew, nullptr) << init_symbol;
    EXPECT_NE(driver.ConnectionInit, nullptr) << init_symbol;
    EXPECT_NE(driver.ConnectionRelease, nullptr) << init_symbol;
    EXPECT_NE(driver.StatementNew, nullptr) << init_symbol;
    EXPECT_NE(driver.StatementSetSqlQuery, nullptr) << init_symbol;
    EXPECT_NE(driver.StatementExecuteQuery, nullptr) << init_symbol;
    EXPECT_NE(driver.StatementPrepare, nullptr) << init_symbol;
    EXPECT_NE(driver.StatementBind, nullptr) << init_symbol;
    EXPECT_NE(driver.StatementBindStream, nullptr) << init_symbol;
    EXPECT_NE(driver.StatementRelease, nullptr) << init_symbol;
    EXPECT_EQ(driver.release(&driver, &error), ADBC_STATUS_OK) << init_symbol;
  }

  CloseLibrary(library);
}

TEST(ExportedSymbolsTest, DriverInitFunctionsClearUnsupportedCallbacks) {
  void* library = OpenLibrary(ADBC_DRIVER_QUACK_LIBRARY_PATH);
  ASSERT_NE(library, nullptr);

  char const* init_symbols[] = {
      "AdbcDriverInit",
      "AdbcDriverQuackInit",
  };

  for (char const* init_symbol : init_symbols) {
    auto const init = FindFunction<AdbcDriverInitFunc>(library, init_symbol);
    ASSERT_NE(init, nullptr) << init_symbol;

    AdbcDriver driver;
    std::memset(&driver, 0xAB, sizeof(driver));
    AdbcError error = ADBC_ERROR_INIT;
    EXPECT_EQ(init(ADBC_VERSION_1_1_0, &driver, &error), ADBC_STATUS_OK)
        << init_symbol;

    EXPECT_EQ(driver.DatabaseSetOptionInt, nullptr) << init_symbol;
    EXPECT_EQ(driver.ConnectionGetInfo, nullptr) << init_symbol;
    EXPECT_EQ(driver.StatementExecuteSchema, nullptr) << init_symbol;

    EXPECT_EQ(driver.release(&driver, &error), ADBC_STATUS_OK) << init_symbol;
  }

  CloseLibrary(library);
}
