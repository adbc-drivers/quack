#include <arrow-adbc/adbc.h>

#include <gtest/gtest.h>

TEST(AdbcDriverTest, ParameterBindingApisReturnNotImplemented) {
  AdbcStatement statement = {};
  AdbcError error = ADBC_ERROR_INIT;

  EXPECT_EQ(AdbcStatementPrepare(&statement, &error), ADBC_STATUS_NOT_IMPLEMENTED);
  EXPECT_EQ(AdbcStatementBind(&statement, nullptr, nullptr, &error),
            ADBC_STATUS_NOT_IMPLEMENTED);
  EXPECT_EQ(AdbcStatementBindStream(&statement, nullptr, &error),
            ADBC_STATUS_NOT_IMPLEMENTED);
}
