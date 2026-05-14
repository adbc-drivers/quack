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

TEST(AdbcDriverTest, ParameterBindingApisReturnNotImplemented) {
  AdbcStatement statement = {};
  AdbcError error = ADBC_ERROR_INIT;

  EXPECT_EQ(AdbcStatementPrepare(&statement, &error),
            ADBC_STATUS_NOT_IMPLEMENTED);
  EXPECT_EQ(AdbcStatementBind(&statement, nullptr, nullptr, &error),
            ADBC_STATUS_NOT_IMPLEMENTED);
  EXPECT_EQ(AdbcStatementBindStream(&statement, nullptr, &error),
            ADBC_STATUS_NOT_IMPLEMENTED);
}
