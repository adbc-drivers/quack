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
  const auto result = adbc_driver_quack::ExecuteDuckDbArrowQuery(
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
