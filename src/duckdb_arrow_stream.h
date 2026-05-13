#pragma once

#include <arrow-adbc/adbc.h>
#include <duckdb.h>

#include <cstdint>
#include <string>
#include <string_view>

namespace adbc_driver_quack {

struct DuckDbArrowQueryResult {
  AdbcStatusCode status = ADBC_STATUS_OK;
  std::string message;
  int32_t vendor_code = 0;
};

DuckDbArrowQueryResult ExecuteDuckDbArrowQuery(duckdb_connection connection,
                                               std::string_view sql,
                                               ArrowArrayStream* out,
                                               int64_t* rows_affected);

}  // namespace adbc_driver_quack
