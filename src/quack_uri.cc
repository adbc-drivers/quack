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

#include <uriparser/Uri.h>

#include <string>
#include <string_view>
#include <utility>

namespace adbc_driver_quack {
namespace {

std::string CopyRange(UriTextRangeA const& range) {
  if (range.first == nullptr || range.afterLast == nullptr ||
      range.afterLast < range.first) {
    return {};
  }
  return std::string(range.first, range.afterLast);
}

ParsedQuackUri Error(std::string message) {
  ParsedQuackUri result;
  result.error = std::move(message);
  return result;
}

}  // namespace

ParsedQuackUri ParseQuackUri(std::string_view uri_text) {
  std::string uri_storage(uri_text);
  UriUriA uri;
  char const* error_position = nullptr;

  if (uriParseSingleUriA(&uri, uri_storage.c_str(), &error_position) !=
      URI_SUCCESS) {
    return Error("failed to parse quack URI");
  }

  ParsedQuackUri result;
  auto const cleanup = [&uri]() { uriFreeUriMembersA(&uri); };

  std::string const scheme = CopyRange(uri.scheme);
  if (scheme != "quack") {
    cleanup();
    return Error("invalid quack URI scheme");
  }

  std::string const host = CopyRange(uri.hostText);
  if (host.empty()) {
    cleanup();
    return Error("quack URI host is required");
  }

  result.endpoint = "quack:" + host;
  std::string const port = CopyRange(uri.portText);
  if (!port.empty()) {
    result.endpoint += ":" + port;
  }

  if (uri.query.first != nullptr && uri.query.afterLast != nullptr) {
    UriQueryListA* query = nullptr;
    int item_count = 0;
    if (uriDissectQueryMallocA(&query, &item_count, uri.query.first,
                               uri.query.afterLast) != URI_SUCCESS) {
      cleanup();
      return Error("failed to parse quack URI query");
    }

    for (UriQueryListA const* item = query; item != nullptr;
         item = item->next) {
      if (item->key != nullptr && std::string_view(item->key) == "token") {
        result.token = item->value != nullptr ? item->value : "";
        break;
      }
    }
    uriFreeQueryListA(query);
  }

  result.ok = true;
  cleanup();
  return result;
}

}  // namespace adbc_driver_quack
