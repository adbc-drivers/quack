include(FetchContent)
include(GoogleTest)

FetchContent_Declare(
  googletest
  GIT_REPOSITORY https://github.com/google/googletest.git
  GIT_TAG v1.15.2
)

set(gtest_force_shared_crt ON CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(googletest)

add_executable(adbc_driver_quack_tests
  quack_uri_test.cc
  sql_escape_test.cc
  duckdb_arrow_stream_test.cc
  adbc_driver_test.cc
  exported_symbols_test.cc)

target_link_libraries(adbc_driver_quack_tests
  PRIVATE
    adbc_driver_quack
    adbc_driver_quack_helpers
    GTest::gtest_main
    ${CMAKE_DL_LIBS})

target_compile_definitions(adbc_driver_quack_tests
  PRIVATE
    ADBC_DRIVER_QUACK_LIBRARY_PATH="$<TARGET_FILE:adbc_driver_quack>")

gtest_discover_tests(adbc_driver_quack_tests)
