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

include(FetchContent)
include(GoogleTest)

FetchContent_Declare(
  googletest
  GIT_REPOSITORY https://github.com/google/googletest.git
  GIT_TAG v1.15.2)

set(gtest_force_shared_crt
    ON
    CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(googletest)

add_executable(
  adbc_driver_quack_tests
  quack_uri_test.cc sql_escape_test.cc duckdb_arrow_stream_test.cc
  adbc_driver_test.cc exported_symbols_test.cc)

target_link_libraries(
  adbc_driver_quack_tests PRIVATE adbc_driver_quack adbc_driver_quack_helpers
                                  GTest::gtest_main ${CMAKE_DL_LIBS})

target_compile_definitions(
  adbc_driver_quack_tests
  PRIVATE ADBC_DRIVER_QUACK_LIBRARY_PATH="$<TARGET_FILE:adbc_driver_quack>")

gtest_discover_tests(adbc_driver_quack_tests)
