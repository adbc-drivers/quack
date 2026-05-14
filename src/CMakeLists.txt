add_library(adbc_driver_quack_helpers STATIC
  duckdb_arrow_stream.cc
  quack_uri.cc
  sql_escape.cc)

set(ADBC_DRIVER_QUACK_DUCKDB_LIBRARIES)
if(TARGET duckdb_generated_extension_loader)
  list(APPEND ADBC_DRIVER_QUACK_DUCKDB_LIBRARIES
    "$<LINK_LIBRARY:WHOLE_ARCHIVE,duckdb_generated_extension_loader>")
endif()
foreach(extension_target
    quack_extension
    json_extension
    autocomplete_extension
    httpfs_extension
    core_functions_extension
    jemalloc_extension)
  if(TARGET ${extension_target})
    list(APPEND ADBC_DRIVER_QUACK_DUCKDB_LIBRARIES ${extension_target})
  endif()
endforeach()
list(APPEND ADBC_DRIVER_QUACK_DUCKDB_LIBRARIES duckdb_static)

target_include_directories(adbc_driver_quack_helpers
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/../third_party/arrow-adbc/include
    ${ADBC_DRIVER_QUACK_DUCKDB_INCLUDE_DIR})

target_link_libraries(adbc_driver_quack_helpers
  PUBLIC
    ${ADBC_DRIVER_QUACK_DUCKDB_LIBRARIES}
    uriparser)

set_target_properties(adbc_driver_quack_helpers PROPERTIES
  CXX_VISIBILITY_PRESET hidden
  VISIBILITY_INLINES_HIDDEN YES)

add_library(adbc_driver_quack SHARED
  adbc_driver_quack.cc)

target_link_libraries(adbc_driver_quack
  PRIVATE
    adbc_driver_quack_helpers)

target_include_directories(adbc_driver_quack
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/../third_party/arrow-adbc/include
  PRIVATE
    ${ADBC_DRIVER_QUACK_DUCKDB_INCLUDE_DIR})

set_target_properties(adbc_driver_quack PROPERTIES
  OUTPUT_NAME adbc_driver_quack
  CXX_VISIBILITY_PRESET hidden
  VISIBILITY_INLINES_HIDDEN YES)
