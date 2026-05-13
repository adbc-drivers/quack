#include "quack_uri.h"

#include <gtest/gtest.h>

TEST(QuackUriTest, ParsesHostOnlyEndpoint) {
  auto parsed = adbc_driver_quack::ParseQuackUri("quack://localhost/");
  ASSERT_TRUE(parsed.ok) << parsed.error;
  EXPECT_EQ(parsed.endpoint, "quack:localhost");
  EXPECT_EQ(parsed.token, "");
}

TEST(QuackUriTest, ParsesHostPortEndpoint) {
  auto parsed =
      adbc_driver_quack::ParseQuackUri("quack://db.example.com:9842/?token=secret");
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
