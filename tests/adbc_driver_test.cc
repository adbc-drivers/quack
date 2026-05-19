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

#include <arrow-adbc/adbc.h>
#include <gtest/gtest.h>

#include <cstdlib>
#include <cstring>
#include <string>

extern "C" AdbcStatusCode AdbcDriverInit(int version, void* driver,
                                         AdbcError* error);

namespace {

void ExpectErrorMessage(AdbcError* error, char const* message) {
  ASSERT_NE(error->message, nullptr);
  EXPECT_STREQ(error->message, message);
  ASSERT_NE(error->release, nullptr);
  error->release(error);
}

void ReleaseTestStream(ArrowArrayStream* stream) { stream->release = nullptr; }

bool GetQuackUri(std::string* uri) {
  char const* value = std::getenv("QUACK_URI");
  if (value == nullptr || value[0] == '\0') {
    return false;
  }
  *uri = value;
  return true;
}

struct TestConnection {
  AdbcDatabase database = {};
  AdbcConnection connection = {};

  explicit TestConnection(std::string const& uri) {
    AdbcError error = ADBC_ERROR_INIT;
    EXPECT_EQ(AdbcDatabaseNew(&database, &error), ADBC_STATUS_OK);
    EXPECT_EQ(AdbcDatabaseSetOption(&database, "uri", uri.c_str(), &error),
              ADBC_STATUS_OK);
    EXPECT_EQ(AdbcDatabaseInit(&database, &error), ADBC_STATUS_OK);
    EXPECT_EQ(AdbcConnectionNew(&connection, &error), ADBC_STATUS_OK);
    EXPECT_EQ(AdbcConnectionInit(&connection, &database, &error),
              ADBC_STATUS_OK);
  }

  ~TestConnection() {
    EXPECT_EQ(AdbcConnectionRelease(&connection, nullptr), ADBC_STATUS_OK);
    EXPECT_EQ(AdbcDatabaseRelease(&database, nullptr), ADBC_STATUS_OK);
  }
};

class AdbcDriverLiveTest : public ::testing::Test {
 protected:
  void SetUp() override {
    if (!GetQuackUri(&quack_uri_)) {
      GTEST_SKIP() << "QUACK_URI is required for live Quack server tests";
    }
  }

  std::string quack_uri_;
};

}  // namespace

TEST(AdbcDriverTest, PrepareRejectsUninitializedStatement) {
  AdbcStatement statement = {};
  AdbcError error = ADBC_ERROR_INIT;

  EXPECT_EQ(AdbcStatementPrepare(&statement, &error),
            ADBC_STATUS_INVALID_STATE);
}

TEST_F(AdbcDriverLiveTest, PrepareRequiresSqlQuery) {
  TestConnection connection(quack_uri_);
  AdbcStatement statement = {};
  AdbcError error = ADBC_ERROR_INIT;

  ASSERT_EQ(AdbcStatementNew(&connection.connection, &statement, &error),
            ADBC_STATUS_OK);
  EXPECT_EQ(AdbcStatementPrepare(&statement, &error),
            ADBC_STATUS_INVALID_STATE);

  ASSERT_EQ(AdbcStatementRelease(&statement, nullptr), ADBC_STATUS_OK);
}

TEST_F(AdbcDriverLiveTest, PrepareAcceptsSqlQuery) {
  TestConnection connection(quack_uri_);
  AdbcStatement statement = {};
  AdbcError error = ADBC_ERROR_INIT;

  ASSERT_EQ(AdbcStatementNew(&connection.connection, &statement, &error),
            ADBC_STATUS_OK);
  ASSERT_EQ(AdbcStatementSetSqlQuery(&statement, "SELECT 1", &error),
            ADBC_STATUS_OK);
  EXPECT_EQ(AdbcStatementPrepare(&statement, &error), ADBC_STATUS_OK);

  ASSERT_EQ(AdbcStatementRelease(&statement, nullptr), ADBC_STATUS_OK);
}

TEST(AdbcDriverTest, BindRemainsNotImplemented) {
  AdbcStatement statement = {};
  AdbcError error = ADBC_ERROR_INIT;

  EXPECT_EQ(AdbcStatementBind(&statement, nullptr, nullptr, &error),
            ADBC_STATUS_NOT_IMPLEMENTED);
}

TEST_F(AdbcDriverLiveTest, BindStreamPreservesSqlUntilExecute) {
  TestConnection connection(quack_uri_);
  AdbcStatement statement = {};
  ArrowArrayStream stream = {};
  stream.release = ReleaseTestStream;
  AdbcError error = ADBC_ERROR_INIT;

  ASSERT_EQ(AdbcStatementNew(&connection.connection, &statement, &error),
            ADBC_STATUS_OK);
  ASSERT_EQ(AdbcStatementSetSqlQuery(&statement, "SELECT ?", &error),
            ADBC_STATUS_OK);
  ASSERT_EQ(AdbcStatementBindStream(&statement, &stream, &error),
            ADBC_STATUS_OK);
  EXPECT_EQ(AdbcStatementExecuteQuery(&statement, nullptr, nullptr, &error),
            ADBC_STATUS_NOT_IMPLEMENTED);

  ASSERT_EQ(AdbcStatementRelease(&statement, nullptr), ADBC_STATUS_OK);
}

TEST_F(AdbcDriverLiveTest, IngestTargetTableClearsSqlQuery) {
  TestConnection connection(quack_uri_);
  AdbcStatement statement = {};
  AdbcError error = ADBC_ERROR_INIT;

  ASSERT_EQ(AdbcStatementNew(&connection.connection, &statement, &error),
            ADBC_STATUS_OK);
  ASSERT_EQ(AdbcStatementSetSqlQuery(&statement, "SELECT 1", &error),
            ADBC_STATUS_OK);
  ASSERT_EQ(AdbcStatementSetOption(&statement, ADBC_INGEST_OPTION_TARGET_TABLE,
                                   "target", &error),
            ADBC_STATUS_OK);
  EXPECT_EQ(AdbcStatementPrepare(&statement, &error),
            ADBC_STATUS_INVALID_STATE);

  ASSERT_EQ(AdbcStatementRelease(&statement, nullptr), ADBC_STATUS_OK);
}

TEST_F(AdbcDriverLiveTest, AutocommitDefaultsToTrue) {
  TestConnection connection(quack_uri_);
  char value[8] = {};
  size_t length = sizeof(value);
  AdbcError error = ADBC_ERROR_INIT;

  EXPECT_EQ(AdbcConnectionGetOption(&connection.connection,
                                    ADBC_CONNECTION_OPTION_AUTOCOMMIT, value,
                                    &length, &error),
            ADBC_STATUS_OK);
  EXPECT_STREQ(value, ADBC_OPTION_VALUE_ENABLED);
}

TEST_F(AdbcDriverLiveTest, CommitRollbackRejectAutocommit) {
  TestConnection connection(quack_uri_);
  AdbcError error = ADBC_ERROR_INIT;

  EXPECT_EQ(AdbcConnectionCommit(&connection.connection, &error),
            ADBC_STATUS_INVALID_STATE);
  EXPECT_EQ(AdbcConnectionRollback(&connection.connection, &error),
            ADBC_STATUS_INVALID_STATE);
}

TEST(AdbcDriverTest, DriverExposesGetObjects) {
  AdbcDriver driver = {};
  ASSERT_EQ(AdbcDriverInit(ADBC_VERSION_1_1_0, &driver, nullptr),
            ADBC_STATUS_OK);

  EXPECT_NE(driver.ConnectionGetObjects, nullptr);

  ASSERT_EQ(driver.release(&driver, nullptr), ADBC_STATUS_OK);
}

TEST(AdbcDriverTest, DriverExposesConnectionGetOption) {
  AdbcDriver driver = {};
  ASSERT_EQ(AdbcDriverInit(ADBC_VERSION_1_1_0, &driver, nullptr),
            ADBC_STATUS_OK);

  EXPECT_NE(driver.ConnectionGetOption, nullptr);

  ASSERT_EQ(driver.release(&driver, nullptr), ADBC_STATUS_OK);
}

TEST(AdbcDriverTest, BindStreamRejectsUninitializedStatement) {
  AdbcStatement statement = {};
  AdbcError error = ADBC_ERROR_INIT;

  EXPECT_EQ(AdbcStatementBindStream(&statement, nullptr, &error),
            ADBC_STATUS_INVALID_STATE);
}

TEST(AdbcDriverTest, ErrorMessagesIncludeQuackPrefix) {
  AdbcStatement statement = {};
  AdbcError error = ADBC_ERROR_INIT;

  EXPECT_EQ(AdbcStatementBind(&statement, nullptr, nullptr, &error),
            ADBC_STATUS_NOT_IMPLEMENTED);
  ExpectErrorMessage(&error, "[quack] parameter binding is not implemented");
}

TEST(AdbcDriverTest, HelperErrorMessagesIncludeQuackPrefix) {
  AdbcStatement statement = {};
  AdbcError error = ADBC_ERROR_INIT;

  EXPECT_EQ(AdbcStatementBindStream(&statement, nullptr, &error),
            ADBC_STATUS_INVALID_STATE);
  ExpectErrorMessage(&error, "[quack] statement is not initialized");
}

TEST(AdbcDriverTest, PropagatedErrorMessagesIncludeQuackPrefix) {
  AdbcDatabase database = {};
  AdbcError error = ADBC_ERROR_INIT;

  ASSERT_EQ(AdbcDatabaseNew(&database, &error), ADBC_STATUS_OK);
  EXPECT_EQ(AdbcDatabaseSetOption(&database, "uri", "quack://", &error),
            ADBC_STATUS_INVALID_ARGUMENT);
  ASSERT_NE(error.message, nullptr);
  EXPECT_EQ(std::strncmp(error.message, "[quack] ", 8), 0);
  EXPECT_NE(std::strstr(error.message, "host"), nullptr);
  ASSERT_NE(error.release, nullptr);
  error.release(&error);

  ASSERT_EQ(AdbcDatabaseRelease(&database, nullptr), ADBC_STATUS_OK);
}

TEST_F(AdbcDriverLiveTest, DatabaseInitHandlesManagerExtendedError) {
  AdbcDriver driver = {};
  ASSERT_EQ(AdbcDriverInit(ADBC_VERSION_1_1_0, &driver, nullptr),
            ADBC_STATUS_OK);

  AdbcDatabase database = {};
  database.private_driver = &driver;
  AdbcError error = ADBC_ERROR_INIT;
  error.vendor_code = ADBC_ERROR_VENDOR_CODE_PRIVATE_DATA;
  error.private_driver = &driver;

  ASSERT_EQ(AdbcDatabaseNew(&database, &error), ADBC_STATUS_OK);
  EXPECT_EQ(database.private_driver, &driver);
  ASSERT_EQ(AdbcDatabaseSetOption(&database, "uri", quack_uri_.c_str(), &error),
            ADBC_STATUS_OK);
  EXPECT_EQ(AdbcDatabaseInit(&database, &error), ADBC_STATUS_OK);
  EXPECT_EQ(database.private_driver, &driver);
  EXPECT_EQ(error.vendor_code, ADBC_ERROR_VENDOR_CODE_PRIVATE_DATA);
  EXPECT_EQ(error.private_driver, &driver);

  ASSERT_EQ(AdbcDatabaseRelease(&database, nullptr), ADBC_STATUS_OK);
  EXPECT_EQ(database.private_driver, &driver);
  ASSERT_EQ(driver.release(&driver, nullptr), ADBC_STATUS_OK);
}
