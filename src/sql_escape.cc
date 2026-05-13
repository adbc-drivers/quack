#include "sql_escape.h"

namespace adbc_driver_quack {

std::string DuckDbSqlStringLiteral(std::string_view value) {
  std::string escaped;
  escaped.reserve(value.size() + 2);
  escaped.push_back('\'');
  for (char c : value) {
    if (c == '\'') {
      escaped.push_back('\'');
    }
    escaped.push_back(c);
  }
  escaped.push_back('\'');
  return escaped;
}

std::string BuildRemoteQuerySql(std::string_view caller_sql) {
  return "SELECT * FROM remote.query(" + DuckDbSqlStringLiteral(caller_sql) + ")";
}

}  // namespace adbc_driver_quack
