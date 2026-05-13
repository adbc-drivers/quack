#include "sql_escape.h"

#include <gtest/gtest.h>

TEST(SqlEscapeTest, EscapesSingleQuotes) {
  EXPECT_EQ(adbc_driver_quack::DuckDbSqlStringLiteral("SELECT 'x'"),
            "'SELECT ''x'''");
}

TEST(SqlEscapeTest, BuildsRemoteQueryWrapper) {
  EXPECT_EQ(adbc_driver_quack::BuildRemoteQuerySql("SELECT 1"),
            "SELECT * FROM remote.query('SELECT 1')");
}

TEST(SqlEscapeTest, BuildsRemoteQueryWrapperWithEscapedSql) {
  EXPECT_EQ(adbc_driver_quack::BuildRemoteQuerySql("SELECT 'a'"),
            "SELECT * FROM remote.query('SELECT ''a''')");
}
