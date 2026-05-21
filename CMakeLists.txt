# Copyright (c) 2026 ADBC Drivers Contributors
#
# Licensed under the Apache License, Version 2.0 (the "License"); you may not
# use this file except in compliance with the License. You may obtain a copy of
# the License at
#
# http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
# WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the
# License for the specific language governing permissions and limitations under
# the License.

cmake_minimum_required(VERSION 4.0)

project(
  adbc_driver_quack
  VERSION 0.1.0
  DESCRIPTION "ADBC driver for remote DuckDB via Quack"
  LANGUAGES C CXX)

set(ADBC_DRIVER_QUACK_DUCKDB_VERSION
    "1.5.3"
    CACHE STRING "DuckDB version used by the Quack driver")
set(ADBC_DRIVER_QUACK_DUCKDB_TAG "v${ADBC_DRIVER_QUACK_DUCKDB_VERSION}")
set(ADBC_DRIVER_QUACK_NANOARROW_VERSION
    "0.8.0"
    CACHE STRING "Apache Arrow nanoarrow version used by the Quack driver")
set(ADBC_DRIVER_QUACK_NANOARROW_TAG
    "apache-arrow-nanoarrow-${ADBC_DRIVER_QUACK_NANOARROW_VERSION}")

configure_file(
  ${CMAKE_CURRENT_LIST_DIR}/src/adbc_driver_quack_config.h.in
  ${CMAKE_CURRENT_BINARY_DIR}/generated/adbc_driver_quack_config.h @ONLY)

include(FetchContent)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
set(CMAKE_FIND_PACKAGE_TARGETS_GLOBAL TRUE)
if(MSVC)
  set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()

find_package(ZLIB REQUIRED GLOBAL)
if(TARGET ZLIB::ZLIB)
  set_target_properties(ZLIB::ZLIB PROPERTIES IMPORTED_GLOBAL TRUE)
else()
  add_library(ZLIB::ZLIB UNKNOWN IMPORTED GLOBAL)
  set_target_properties(
    ZLIB::ZLIB
    PROPERTIES IMPORTED_CONFIGURATIONS "DEBUG;RELEASE"
               IMPORTED_LOCATION_DEBUG "${ZLIB_LIBRARY_DEBUG}"
               IMPORTED_LOCATION_RELEASE "${ZLIB_LIBRARY_RELEASE}"
               INTERFACE_INCLUDE_DIRECTORIES "${ZLIB_INCLUDE_DIRS}")
endif()
find_package(OpenSSL REQUIRED GLOBAL)
find_package(CURL REQUIRED GLOBAL)

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

FetchContent_Declare(
  duckdb
  GIT_REPOSITORY https://github.com/duckdb/duckdb.git
  GIT_TAG ${ADBC_DRIVER_QUACK_DUCKDB_TAG}
  PATCH_COMMAND
    ${CMAKE_COMMAND} -E chdir <SOURCE_DIR> git apply --ignore-whitespace
    ${CMAKE_CURRENT_LIST_DIR}/cmake/patches/duckdb-disable-adbc.patch COMMAND
    ${CMAKE_COMMAND} -E chdir <SOURCE_DIR> git apply --ignore-whitespace
    ${CMAKE_CURRENT_LIST_DIR}/cmake/patches/duckdb-msvc-uncaught-exception.patch
)

FetchContent_Declare(
  uriparser
  GIT_REPOSITORY https://github.com/uriparser/uriparser.git
  GIT_TAG uriparser-0.9.8)

FetchContent_Declare(
  nanoarrow
  GIT_REPOSITORY https://github.com/apache/arrow-nanoarrow.git
  GIT_TAG ${ADBC_DRIVER_QUACK_NANOARROW_TAG})

set(URIPARSER_BUILD_DOCS
    OFF
    CACHE BOOL "" FORCE)
set(URIPARSER_SHARED_LIBS
    OFF
    CACHE BOOL "" FORCE)
set(URIPARSER_BUILD_TESTS
    OFF
    CACHE BOOL "" FORCE)
set(URIPARSER_BUILD_TOOLS
    OFF
    CACHE BOOL "" FORCE)
set(ENABLE_SANITIZER
    OFF
    CACHE BOOL "" FORCE)
set(ENABLE_UBSAN
    OFF
    CACHE BOOL "" FORCE)
set(BUILD_SHELL
    OFF
    CACHE BOOL "" FORCE)
set(BUILD_UNITTESTS
    OFF
    CACHE BOOL "" FORCE)
set(ENABLE_UNITTEST_CPP_TESTS
    OFF
    CACHE BOOL "" FORCE)
set(BUILD_BENCHMARKS
    OFF
    CACHE BOOL "" FORCE)
set(BUILD_EXTENSIONS
    "quack;httpfs"
    CACHE STRING "" FORCE)
set(SKIP_EXTENSIONS
    parquet
    CACHE STRING "" FORCE)

FetchContent_MakeAvailable(duckdb nanoarrow uriparser)

set(ADBC_DRIVER_QUACK_DUCKDB_INCLUDE_DIR "${duckdb_SOURCE_DIR}/src/include")

add_subdirectory(src)

include(CTest)
if(BUILD_TESTING)
  add_subdirectory(tests)
endif()
