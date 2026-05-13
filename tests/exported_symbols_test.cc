#include <arrow-adbc/adbc.h>

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

template <typename Function>
Function FindFunction(void* library, const char* name) {
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
      "AdbcDriverInit",
      "AdbcDriverQuackInit",
  };

  for (const char* symbol : symbols) {
    EXPECT_NE(FindSymbol(library, symbol), nullptr) << symbol;
  }

  CloseLibrary(library);
}

TEST(ExportedSymbolsTest, DriverInitFunctionsPopulateDriverTable) {
  void* library = OpenLibrary(ADBC_DRIVER_QUACK_LIBRARY_PATH);
  ASSERT_NE(library, nullptr);

  const char* init_symbols[] = {
      "AdbcDriverInit",
      "AdbcDriverQuackInit",
  };

  for (const char* init_symbol : init_symbols) {
    const auto init = FindFunction<AdbcDriverInitFunc>(library, init_symbol);
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
