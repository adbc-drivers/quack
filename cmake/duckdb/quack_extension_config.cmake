if(NOT DEFINED adbc_driver_quack_quack_SOURCE_DIR)
  message(FATAL_ERROR "adbc_driver_quack_quack_SOURCE_DIR must be defined before loading Quack")
endif()

duckdb_extension_load(quack
  SOURCE_DIR "${adbc_driver_quack_quack_SOURCE_DIR}"
)

duckdb_extension_load(json)
duckdb_extension_load(autocomplete)

duckdb_extension_load(httpfs
  GIT_URL https://github.com/duckdb/duckdb-httpfs
  GIT_TAG 7e86e7a5e5a1f01f458361bebdfa9b0a9a73a619
)
