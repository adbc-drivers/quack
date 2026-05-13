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
