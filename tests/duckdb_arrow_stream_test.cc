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

#include "duckdb_arrow_stream.h"

#include <arrow-adbc/adbc.h>
#include <duckdb.h>
#include <gtest/gtest.h>

TEST(DuckDbArrowStreamTest, ExecutesQueryAsArrowStream) {
  duckdb_database database = nullptr;
  duckdb_connection connection = nullptr;
  ASSERT_EQ(duckdb_open(nullptr, &database), DuckDBSuccess);
  ASSERT_EQ(duckdb_connect(database, &connection), DuckDBSuccess);

  ArrowArrayStream stream = {};
  int64_t rows_affected = 0;
  auto const result = adbc_driver_quack::ExecuteDuckDbArrowQuery(
      connection, "SELECT 42 AS answer", &stream, &rows_affected);

  EXPECT_EQ(result.status, ADBC_STATUS_OK);
  EXPECT_TRUE(result.message.empty());
  EXPECT_EQ(rows_affected, -1);
  ASSERT_NE(stream.get_schema, nullptr);
  ASSERT_NE(stream.get_next, nullptr);
  ASSERT_NE(stream.release, nullptr);

  ArrowSchema schema = {};
  EXPECT_EQ(stream.get_schema(&stream, &schema), 0);
  ASSERT_NE(schema.release, nullptr);
  EXPECT_STREQ(schema.name, "duckdb_query_result");
  schema.release(&schema);

  ArrowArray array = {};
  EXPECT_EQ(stream.get_next(&stream, &array), 0);
  EXPECT_EQ(array.length, 1);
  ASSERT_NE(array.release, nullptr);
  array.release(&array);

  ArrowArray end = {};
  EXPECT_EQ(stream.get_next(&stream, &end), 0);
  EXPECT_EQ(end.release, nullptr);

  stream.release(&stream);
  EXPECT_EQ(stream.release, nullptr);

  duckdb_disconnect(&connection);
  duckdb_close(&database);
}
