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

#include <cerrno>
#include <string>

struct ArrowArrayStreamGuard {
  ArrowArrayStream stream = {};

  ~ArrowArrayStreamGuard() { Release(); }

  ArrowArrayStream* get() { return &stream; }

  void Release() {
    if (stream.release != nullptr) {
      stream.release(&stream);
    }
  }
};

struct ArrowSchemaGuard {
  ArrowSchema schema = {};

  ~ArrowSchemaGuard() { Release(); }

  ArrowSchema* get() { return &schema; }

  void Release() {
    if (schema.release != nullptr) {
      schema.release(&schema);
    }
  }
};

struct ArrowArrayGuard {
  ArrowArray array = {};

  ~ArrowArrayGuard() { Release(); }

  ArrowArray* get() { return &array; }

  void Release() {
    if (array.release != nullptr) {
      array.release(&array);
    }
  }
};

class DuckDbArrowStreamTest : public ::testing::Test {
 protected:
  void SetUp() override {
    ASSERT_EQ(duckdb_open(nullptr, &database_), DuckDBSuccess);
    ASSERT_EQ(duckdb_connect(database_, &connection_), DuckDBSuccess);
  }

  void TearDown() override {
    if (connection_ != nullptr) {
      duckdb_disconnect(&connection_);
    }
    if (database_ != nullptr) {
      duckdb_close(&database_);
    }
  }

  duckdb_connection connection() { return connection_; }

 private:
  duckdb_database database_ = nullptr;
  duckdb_connection connection_ = nullptr;
};

TEST_F(DuckDbArrowStreamTest, ExecutesQueryAsArrowStream) {
  ArrowArrayStreamGuard stream;
  int64_t rows_affected = 0;
  auto const result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      connection(), "SELECT 42 AS answer", stream.get(), &rows_affected);

  EXPECT_EQ(result.status, ADBC_STATUS_OK);
  EXPECT_TRUE(result.message.empty());
  EXPECT_EQ(rows_affected, -1);
  ASSERT_NE(stream.stream.get_schema, nullptr);
  ASSERT_NE(stream.stream.get_next, nullptr);
  ASSERT_NE(stream.stream.release, nullptr);

  ArrowSchemaGuard schema;
  EXPECT_EQ(stream.stream.get_schema(stream.get(), schema.get()), 0);
  ASSERT_NE(schema.schema.release, nullptr);
  EXPECT_STREQ(schema.schema.name, "duckdb_query_result");
  schema.Release();

  ArrowArrayGuard array;
  EXPECT_EQ(stream.stream.get_next(stream.get(), array.get()), 0);
  EXPECT_EQ(array.array.length, 1);
  ASSERT_NE(array.array.release, nullptr);
  array.Release();

  ArrowArrayGuard end;
  EXPECT_EQ(stream.stream.get_next(stream.get(), end.get()), 0);
  EXPECT_EQ(end.array.release, nullptr);

  stream.Release();
  EXPECT_EQ(stream.stream.release, nullptr);
}

TEST_F(DuckDbArrowStreamTest, FetchesLargeQueryInMultipleArrays) {
  ArrowArrayStreamGuard stream;
  auto const result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      connection(), "SELECT range AS value FROM range(10000)", stream.get(),
      nullptr);

  ASSERT_EQ(result.status, ADBC_STATUS_OK);
  ASSERT_NE(stream.stream.get_next, nullptr);

  int array_count = 0;
  int64_t row_count = 0;
  while (true) {
    ArrowArrayGuard array;
    ASSERT_EQ(stream.stream.get_next(stream.get(), array.get()), 0);
    if (array.array.release == nullptr) {
      break;
    }
    ++array_count;
    row_count += array.array.length;
  }

  EXPECT_GT(array_count, 1);
  EXPECT_EQ(row_count, 10000);
}

TEST_F(DuckDbArrowStreamTest, ReportsQueryErrors) {
  ArrowArrayStreamGuard stream;
  auto const result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      connection(), "SELECT * FROM missing_table", stream.get(), nullptr);

  EXPECT_EQ(result.status, ADBC_STATUS_IO);
  EXPECT_FALSE(result.message.empty());
  EXPECT_NE(result.vendor_code, 0);
  EXPECT_EQ(stream.stream.release, nullptr);
}

TEST_F(DuckDbArrowStreamTest, PreservesColumnNamesForEmptyResults) {
  ArrowArrayStreamGuard stream;
  auto const result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      connection(), "SELECT 42 AS \"MiXeD Name\" WHERE false", stream.get(),
      nullptr);
  ASSERT_EQ(result.status, ADBC_STATUS_OK) << result.message;

  ArrowSchemaGuard schema;
  ASSERT_EQ(stream.stream.get_schema(stream.get(), schema.get()), 0);
  ASSERT_EQ(schema.schema.n_children, 1);
  EXPECT_STREQ(schema.schema.children[0]->name, "MiXeD Name");
  EXPECT_STREQ(schema.schema.children[0]->format, "i");

  for (int i = 0; i < 2; ++i) {
    ArrowArrayGuard end;
    EXPECT_EQ(stream.stream.get_next(stream.get(), end.get()), 0);
    EXPECT_EQ(end.array.release, nullptr);
    EXPECT_EQ(stream.stream.get_last_error(stream.get()), nullptr);
  }
}

TEST_F(DuckDbArrowStreamTest, ReportsFetchErrors) {
  ArrowArrayStreamGuard stream;
  auto const result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      connection(),
      "SELECT CASE WHEN range = 9999 THEN error('stream failure') ELSE range "
      "END "
      "FROM range(10000)",
      stream.get(), nullptr);
  ASSERT_EQ(result.status, ADBC_STATUS_OK) << result.message;

  int status = 0;
  do {
    ArrowArrayGuard array;
    status = stream.stream.get_next(stream.get(), array.get());
    ASSERT_TRUE(status != 0 || array.array.release != nullptr)
        << "execution error was reported as end-of-stream";
  } while (status == 0);
  EXPECT_EQ(status, EIO);
  ASSERT_NE(stream.stream.get_last_error(stream.get()), nullptr);
  EXPECT_NE(std::string(stream.stream.get_last_error(stream.get()))
                .find("stream failure"),
            std::string::npos);

  ArrowArrayGuard retry;
  EXPECT_EQ(stream.stream.get_next(stream.get(), retry.get()), EIO);
  EXPECT_EQ(retry.array.release, nullptr);
}

TEST_F(DuckDbArrowStreamTest, ReusesConnectionAfterEarlyRelease) {
  ArrowArrayStreamGuard stream;
  auto result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      connection(), "SELECT range FROM range(1000000)", stream.get(), nullptr);
  ASSERT_EQ(result.status, ADBC_STATUS_OK) << result.message;
  {
    ArrowArrayGuard array;
    ASSERT_EQ(stream.stream.get_next(stream.get(), array.get()), 0);
    ASSERT_NE(array.array.release, nullptr);
    EXPECT_GT(array.array.length, 0);
    EXPECT_LT(array.array.length, 1000000);
  }
  stream.Release();
  EXPECT_EQ(stream.stream.private_data, nullptr);

  result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      connection(), "SELECT 42", stream.get(), nullptr);
  ASSERT_EQ(result.status, ADBC_STATUS_OK) << result.message;
  ArrowArrayGuard array;
  ASSERT_EQ(stream.stream.get_next(stream.get(), array.get()), 0);
  ASSERT_NE(array.array.release, nullptr);
  ASSERT_EQ(array.array.n_children, 1);
  ASSERT_NE(array.array.children[0]->buffers[1], nullptr);
  EXPECT_EQ(static_cast<int32_t const*>(array.array.children[0]->buffers[1])[0],
            42);
  for (int i = 0; i < 2; ++i) {
    ArrowArrayGuard end;
    EXPECT_EQ(stream.stream.get_next(stream.get(), end.get()), 0);
    EXPECT_EQ(end.array.release, nullptr);
  }
}

TEST_F(DuckDbArrowStreamTest, ResolvesDeferredMetadata) {
  ASSERT_EQ(duckdb_query(connection(),
                         "CREATE TABLE deferred_metadata AS SELECT 1 AS value",
                         nullptr),
            DuckDBSuccess);

  // A volatile default expands into multiple statements, so Submit cannot
  // provide the output metadata until execution has advanced.
  ArrowArrayStreamGuard stream;
  auto result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      connection(),
      "ALTER TABLE deferred_metadata ADD COLUMN extra DOUBLE DEFAULT random()",
      stream.get(), nullptr);
  ASSERT_EQ(result.status, ADBC_STATUS_OK) << result.message;
  ArrowSchemaGuard schema;
  ASSERT_EQ(stream.stream.get_schema(stream.get(), schema.get()), 0);
  ASSERT_NE(schema.schema.release, nullptr);
  while (true) {
    ArrowArrayGuard array;
    ASSERT_EQ(stream.stream.get_next(stream.get(), array.get()), 0)
        << stream.stream.get_last_error(stream.get());
    if (array.array.release == nullptr) {
      break;
    }
  }
  stream.Release();

  result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      connection(), "SELECT extra FROM deferred_metadata", stream.get(),
      nullptr);
  ASSERT_EQ(result.status, ADBC_STATUS_OK) << result.message;
  ArrowArrayGuard array;
  ASSERT_EQ(stream.stream.get_next(stream.get(), array.get()), 0);
  ASSERT_NE(array.array.release, nullptr);
  EXPECT_EQ(array.array.length, 1);
}

TEST_F(DuckDbArrowStreamTest, RejectsInvalidCallbackArguments) {
  ArrowArrayStreamGuard stream;
  auto const result = adbc_driver_quack::ExecuteDuckDbStreamingArrowQuery(
      connection(), "SELECT 1", stream.get(), nullptr);
  ASSERT_EQ(result.status, ADBC_STATUS_OK);
  ASSERT_NE(stream.stream.get_schema, nullptr);
  ASSERT_NE(stream.stream.get_next, nullptr);

  {
    ArrowSchemaGuard schema;
    EXPECT_EQ(stream.stream.get_schema(nullptr, schema.get()), EINVAL);
    EXPECT_EQ(schema.schema.release, nullptr);
  }

  {
    ArrowSchemaGuard schema;
    EXPECT_EQ(stream.stream.get_schema(stream.get(), nullptr), EINVAL);
    EXPECT_EQ(schema.schema.release, nullptr);
  }

  {
    ArrowArrayGuard array;
    EXPECT_EQ(stream.stream.get_next(nullptr, array.get()), EINVAL);
    EXPECT_EQ(array.array.release, nullptr);
  }

  {
    ArrowArrayGuard array;
    EXPECT_EQ(stream.stream.get_next(stream.get(), nullptr), EINVAL);
    EXPECT_EQ(array.array.release, nullptr);
  }
}
