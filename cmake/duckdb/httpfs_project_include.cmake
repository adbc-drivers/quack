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

foreach(previous_project_include IN
        LISTS ADBC_DRIVER_QUACK_PREVIOUS_HTTPFS_PROJECT_INCLUDE)
  if(NOT previous_project_include STREQUAL "")
    include("${previous_project_include}")
  endif()
endforeach()

function(adbc_driver_quack_link_httpfs_zlib)
  find_package(ZLIB REQUIRED)

  if(ZLIB_LIBRARIES)
    set(zlib_libraries ${ZLIB_LIBRARIES})
  else()
    set(zlib_libraries ZLIB::ZLIB)
  endif()

  if(TARGET httpfs_loadable_extension)
    target_link_libraries(httpfs_loadable_extension ${zlib_libraries})
  endif()

  if(TARGET httpfs_extension)
    target_link_libraries(httpfs_extension ${zlib_libraries})
  endif()
endfunction()

cmake_language(DEFER CALL adbc_driver_quack_link_httpfs_zlib)
