// Copyright (c) 2026 ADBC Drivers Contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//         http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

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
  return "SELECT * FROM remote.query(" + DuckDbSqlStringLiteral(caller_sql) +
         ")";
}

}  // namespace adbc_driver_quack
