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

extern "C" AdbcStatusCode AdbcDriverInit(int version, void* driver,
                                         AdbcError* error);

TEST(AdbcDriverTest, UnsupportedStatementApisReturnNotImplemented) {
  AdbcStatement statement = {};
  AdbcError error = ADBC_ERROR_INIT;

  EXPECT_EQ(AdbcStatementPrepare(&statement, &error),
            ADBC_STATUS_NOT_IMPLEMENTED);
  EXPECT_EQ(AdbcStatementBind(&statement, nullptr, nullptr, &error),
            ADBC_STATUS_NOT_IMPLEMENTED);
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

TEST(AdbcDriverTest, DatabaseInitHandlesManagerExtendedError) {
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
  ASSERT_EQ(AdbcDatabaseSetOption(&database, "uri",
                                  "quack://localhost:9494/?token=quack-secret",
                                  &error),
            ADBC_STATUS_OK);
  EXPECT_EQ(AdbcDatabaseInit(&database, &error), ADBC_STATUS_OK);
  EXPECT_EQ(database.private_driver, &driver);
  EXPECT_EQ(error.vendor_code, ADBC_ERROR_VENDOR_CODE_PRIVATE_DATA);
  EXPECT_EQ(error.private_driver, &driver);

  ASSERT_EQ(AdbcDatabaseRelease(&database, nullptr), ADBC_STATUS_OK);
  EXPECT_EQ(database.private_driver, &driver);
  ASSERT_EQ(driver.release(&driver, nullptr), ADBC_STATUS_OK);
}
