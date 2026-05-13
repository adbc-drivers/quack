#pragma once

#include <string>
#include <string_view>

namespace adbc_driver_quack {

struct ParsedQuackUri {
  bool ok = false;
  std::string endpoint;
  std::string token;
  std::string error;
};

ParsedQuackUri ParseQuackUri(std::string_view uri);

}  // namespace adbc_driver_quack
