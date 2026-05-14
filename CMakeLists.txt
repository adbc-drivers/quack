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
  adbc_driver_quack_quack
  GIT_REPOSITORY https://github.com/duckdb/duckdb-quack.git
  GIT_TAG v1.5-variegata
  GIT_SUBMODULES ""
  PATCH_COMMAND ${CMAKE_COMMAND} -E chdir <SOURCE_DIR> git apply --ignore-whitespace ${CMAKE_CURRENT_LIST_DIR}/cmake/patches/quack-parent-duckdb-includes.patch
  SOURCE_SUBDIR cmake/skip-add-subdirectory
)

FetchContent_Declare(
  uriparser
  GIT_REPOSITORY https://github.com/uriparser/uriparser.git
  GIT_TAG uriparser-0.9.8
)

set(URIPARSER_BUILD_DOCS OFF CACHE BOOL "" FORCE)
set(URIPARSER_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(URIPARSER_BUILD_TOOLS OFF CACHE BOOL "" FORCE)
set(ENABLE_SANITIZER OFF CACHE BOOL "" FORCE)
set(ENABLE_UBSAN OFF CACHE BOOL "" FORCE)
set(BUILD_SHELL OFF CACHE BOOL "" FORCE)
set(BUILD_UNITTESTS OFF CACHE BOOL "" FORCE)
set(ENABLE_UNITTEST_CPP_TESTS OFF CACHE BOOL "" FORCE)
set(BUILD_BENCHMARKS OFF CACHE BOOL "" FORCE)
set(SKIP_EXTENSIONS
  parquet
  CACHE STRING "" FORCE)
set(DUCKDB_EXTENSION_CONFIGS
  "${CMAKE_CURRENT_LIST_DIR}/cmake/duckdb/quack_extension_config.cmake"
  CACHE STRING "" FORCE)

FetchContent_MakeAvailable(adbc_driver_quack_quack duckdb uriparser)

set(ADBC_DRIVER_QUACK_DUCKDB_INCLUDE_DIR "${duckdb_SOURCE_DIR}/src/include")

add_subdirectory(src)

include(CTest)
if(BUILD_TESTING)
  add_subdirectory(tests)
endif()
