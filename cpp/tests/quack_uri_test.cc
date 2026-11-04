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

#include "quack_uri.h"

#include <gtest/gtest.h>

TEST(QuackUriTest, ParsesHostOnlyEndpoint) {
  auto parsed = adbc_driver_quack::ParseQuackUri("quack://localhost/");
  ASSERT_TRUE(parsed.ok) << parsed.error;
  EXPECT_EQ(parsed.endpoint, "quack:localhost");
  EXPECT_EQ(parsed.token, "");
}

TEST(QuackUriTest, ParsesHostPortEndpoint) {
  auto parsed = adbc_driver_quack::ParseQuackUri(
      "quack://db.example.com:9842/?token=secret");
  ASSERT_TRUE(parsed.ok) << parsed.error;
  EXPECT_EQ(parsed.endpoint, "quack:db.example.com:9842");
  EXPECT_EQ(parsed.token, "secret");
}

TEST(QuackUriTest, DecodesToken) {
  auto parsed = adbc_driver_quack::ParseQuackUri("quack://db/?token=a%20b%27c");
  ASSERT_TRUE(parsed.ok) << parsed.error;
  EXPECT_EQ(parsed.endpoint, "quack:db");
  EXPECT_EQ(parsed.token, "a b'c");
}

TEST(QuackUriTest, RejectsInvalidScheme) {
  auto parsed = adbc_driver_quack::ParseQuackUri("http://db/?token=secret");
  EXPECT_FALSE(parsed.ok);
  EXPECT_NE(parsed.error.find("scheme"), std::string::npos);
}

TEST(QuackUriTest, RejectsMissingHost) {
  auto parsed = adbc_driver_quack::ParseQuackUri("quack:///?token=secret");
  EXPECT_FALSE(parsed.ok);
  EXPECT_NE(parsed.error.find("host"), std::string::npos);
}

TEST(QuackUriTest, DefaultsToVerifiedTls) {
  auto const parsed = adbc_driver_quack::ParseQuackUri("quack://localhost/");
  ASSERT_TRUE(parsed.ok) << parsed.error;
  EXPECT_EQ(parsed.tls, adbc_driver_quack::QuackTlsMode::Verify);
}

TEST(QuackUriTest, ParsesTlsModesAndTokenInEitherOrder) {
  using adbc_driver_quack::QuackTlsMode;
  struct Case {
    char const* value;
    QuackTlsMode mode;
  };
  for (auto const& test : {Case{"true", QuackTlsMode::Verify},
                           Case{"false", QuackTlsMode::Disable},
                           Case{"skip_verify", QuackTlsMode::SkipVerify},
                           Case{"skip-verify", QuackTlsMode::SkipVerify}}) {
    for (bool const token_first : {false, true}) {
      std::string const query =
          token_first ? "token=a%20b%27c&tls=" + std::string(test.value)
                      : "tls=" + std::string(test.value) + "&token=a%20b%27c";
      auto const parsed =
          adbc_driver_quack::ParseQuackUri("quack://db/?" + query);
      ASSERT_TRUE(parsed.ok) << parsed.error;
      EXPECT_EQ(parsed.tls, test.mode);
      EXPECT_EQ(parsed.token, "a b'c");
      EXPECT_EQ(parsed.endpoint, "quack:db");
    }
  }
}

TEST(QuackUriTest, RejectsInvalidAndDuplicateTlsParameters) {
  for (char const* query :
       {"tls", "tls=", "tls=TRUE", "tls=invalid", "tls=false&tls=false",
        "tls=true&tls=false", "token=secret&tls=invalid"}) {
    auto const parsed =
        adbc_driver_quack::ParseQuackUri("quack://db/?" + std::string(query));
    EXPECT_FALSE(parsed.ok) << query;
    EXPECT_NE(parsed.error.find("tls"), std::string::npos);
    EXPECT_EQ(parsed.error.find("secret"), std::string::npos);
  }
}

TEST(QuackUriTest, PreservesFirstTokenIncludingEmptyTokens) {
  for (char const* first : {"token", "token=", "token=first"}) {
    auto const parsed = adbc_driver_quack::ParseQuackUri(
        "quack://db/?" + std::string(first) + "&tls=false&token=second");
    ASSERT_TRUE(parsed.ok) << parsed.error;
    EXPECT_EQ(parsed.token, std::string(first) == "token=first" ? "first" : "");
    EXPECT_EQ(parsed.tls, adbc_driver_quack::QuackTlsMode::Disable);
  }
}
