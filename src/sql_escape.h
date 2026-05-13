#pragma once

#include <string>
#include <string_view>

namespace adbc_driver_quack {

std::string DuckDbSqlStringLiteral(std::string_view value);
std::string BuildRemoteQuerySql(std::string_view caller_sql);

}  // namespace adbc_driver_quack
