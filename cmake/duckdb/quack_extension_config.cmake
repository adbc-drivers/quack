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

if(NOT DEFINED adbc_driver_quack_quack_SOURCE_DIR)
  message(
    FATAL_ERROR
      "adbc_driver_quack_quack_SOURCE_DIR must be defined before loading Quack")
endif()

duckdb_extension_load(quack SOURCE_DIR "${adbc_driver_quack_quack_SOURCE_DIR}")

duckdb_extension_load(json)
duckdb_extension_load(autocomplete)

find_package(ZLIB REQUIRED)
duckdb_extension_load(httpfs GIT_URL https://github.com/duckdb/duckdb-httpfs
                      GIT_TAG 7e86e7a5e5a1f01f458361bebdfa9b0a9a73a619)
