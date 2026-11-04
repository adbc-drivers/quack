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

#include "get_info_stream.h"

#include <nanoarrow/nanoarrow.h>

#include <cstring>
#include <string>
#include <utility>
#include <vector>

#include "adbc_driver_quack_config.h"

namespace adbc_driver_quack {
namespace {

constexpr char kDriverName[] = "ADBC Driver for DuckDB Quack";
constexpr char kVendorName[] = "DuckDB Quack";
constexpr char kDriverArrowVersion[] = "v" ADBC_DRIVER_QUACK_NANOARROW_VERSION;

enum class ValueKind { kString, kBool, kInt64 };

struct InfoRow {
  uint32_t code = 0;
  ValueKind kind = ValueKind::kString;
  std::string string_value;
  bool bool_value = false;
  int64_t int64_value = 0;
};

struct SchemaHolder {
  ArrowSchema schema = {};

  SchemaHolder() { ArrowSchemaInit(&schema); }

  ~SchemaHolder() {
    if (schema.release != nullptr) {
      ArrowSchemaRelease(&schema);
    }
  }

  SchemaHolder(SchemaHolder const&) = delete;
  SchemaHolder& operator=(SchemaHolder const&) = delete;
};

struct ArrayHolder {
  ArrowArray array = {};

  ArrayHolder() = default;

  ~ArrayHolder() {
    if (array.release != nullptr) {
      ArrowArrayRelease(&array);
    }
  }

  ArrayHolder(ArrayHolder const&) = delete;
  ArrayHolder& operator=(ArrayHolder const&) = delete;
};

std::string NormalizeDuckDbVersion(char const* version) {
  if (version == nullptr || version[0] == '\0') {
    return "unknown";
  }
  std::string normalized(version);
  if (!normalized.empty() && normalized[0] == 'v') {
    normalized.erase(0, 1);
  }
  return normalized;
}

GetInfoResult Error(AdbcStatusCode status, std::string message) {
  GetInfoResult result;
  result.status = status;
  result.message = std::move(message);
  return result;
}

void ResetStream(ArrowArrayStream* stream) {
  std::memset(stream, 0, sizeof(*stream));
}

std::string NanoarrowErrorMessage(std::string const& context,
                                  ArrowErrorCode code,
                                  ArrowError* error = nullptr) {
  std::string message = context;
  char const* detail = ArrowErrorMessage(error);
  if (detail != nullptr && detail[0] != '\0') {
    message += ": ";
    message += detail;
  } else if (code != NANOARROW_OK) {
    message += ": ";
    message += std::strerror(code);
  }
  return message;
}

GetInfoResult NanoarrowError(std::string const& context, ArrowErrorCode code,
                             ArrowError* error = nullptr) {
  return Error(ADBC_STATUS_UNKNOWN,
               NanoarrowErrorMessage(context, code, error));
}

bool ShouldInclude(uint32_t code, uint32_t const* info_codes,
                   std::size_t info_codes_length) {
  if (info_codes == nullptr) {
    return true;
  }
  for (std::size_t i = 0; i < info_codes_length; ++i) {
    if (info_codes[i] == code) {
      return true;
    }
  }
  return false;
}

void AddStringRow(std::vector<InfoRow>* rows, uint32_t code,
                  std::string value) {
  rows->push_back(
      InfoRow{code, ValueKind::kString, std::move(value), false, 0});
}

void AddBoolRow(std::vector<InfoRow>* rows, uint32_t code, bool value) {
  rows->push_back(InfoRow{code, ValueKind::kBool, {}, value, 0});
}

void AddInt64Row(std::vector<InfoRow>* rows, uint32_t code, int64_t value) {
  rows->push_back(InfoRow{code, ValueKind::kInt64, {}, false, value});
}

std::vector<InfoRow> BuildRows(std::string const& remote_vendor_version,
                               uint32_t const* info_codes,
                               std::size_t info_codes_length) {
  std::vector<InfoRow> rows;
  rows.reserve(8);

  if (ShouldInclude(ADBC_INFO_VENDOR_NAME, info_codes, info_codes_length)) {
    AddStringRow(&rows, ADBC_INFO_VENDOR_NAME, kVendorName);
  }
  if (ShouldInclude(ADBC_INFO_VENDOR_VERSION, info_codes, info_codes_length)) {
    AddStringRow(&rows, ADBC_INFO_VENDOR_VERSION,
                 NormalizeDuckDbVersion(remote_vendor_version.c_str()));
  }
  if (ShouldInclude(ADBC_INFO_VENDOR_SQL, info_codes, info_codes_length)) {
    AddBoolRow(&rows, ADBC_INFO_VENDOR_SQL, true);
  }
  if (ShouldInclude(ADBC_INFO_VENDOR_SUBSTRAIT, info_codes,
                    info_codes_length)) {
    AddBoolRow(&rows, ADBC_INFO_VENDOR_SUBSTRAIT, false);
  }
  if (ShouldInclude(ADBC_INFO_DRIVER_NAME, info_codes, info_codes_length)) {
    AddStringRow(&rows, ADBC_INFO_DRIVER_NAME, kDriverName);
  }
  if (ShouldInclude(ADBC_INFO_DRIVER_VERSION, info_codes, info_codes_length)) {
    AddStringRow(&rows, ADBC_INFO_DRIVER_VERSION, ADBC_DRIVER_QUACK_VERSION);
  }
  if (ShouldInclude(ADBC_INFO_DRIVER_ARROW_VERSION, info_codes,
                    info_codes_length)) {
    AddStringRow(&rows, ADBC_INFO_DRIVER_ARROW_VERSION, kDriverArrowVersion);
  }
  if (ShouldInclude(ADBC_INFO_DRIVER_ADBC_VERSION, info_codes,
                    info_codes_length)) {
    AddInt64Row(&rows, ADBC_INFO_DRIVER_ADBC_VERSION, ADBC_VERSION_1_1_0);
  }

  return rows;
}

ArrowErrorCode SetNamedType(ArrowSchema* schema, ArrowType type,
                            char const* name) {
  ArrowErrorCode code = ArrowSchemaSetType(schema, type);
  if (code != NANOARROW_OK) {
    return code;
  }
  return ArrowSchemaSetName(schema, name);
}

ArrowErrorCode BuildGetInfoSchema(ArrowSchema* schema) {
  ArrowErrorCode code = ArrowSchemaSetTypeStruct(schema, 2);
  if (code != NANOARROW_OK) {
    return code;
  }

  code = SetNamedType(schema->children[0], NANOARROW_TYPE_UINT32, "info_name");
  if (code != NANOARROW_OK) {
    return code;
  }
  schema->children[0]->flags &= ~ARROW_FLAG_NULLABLE;

  code = ArrowSchemaSetTypeUnion(schema->children[1],
                                 NANOARROW_TYPE_DENSE_UNION, 6);
  if (code != NANOARROW_OK) {
    return code;
  }
  code = ArrowSchemaSetName(schema->children[1], "info_value");
  if (code != NANOARROW_OK) {
    return code;
  }

  code = SetNamedType(schema->children[1]->children[0], NANOARROW_TYPE_STRING,
                      "string_value");
  if (code != NANOARROW_OK) {
    return code;
  }
  code = SetNamedType(schema->children[1]->children[1], NANOARROW_TYPE_BOOL,
                      "bool_value");
  if (code != NANOARROW_OK) {
    return code;
  }
  code = SetNamedType(schema->children[1]->children[2], NANOARROW_TYPE_INT64,
                      "int64_value");
  if (code != NANOARROW_OK) {
    return code;
  }
  code = SetNamedType(schema->children[1]->children[3], NANOARROW_TYPE_INT32,
                      "int32_bitmask");
  if (code != NANOARROW_OK) {
    return code;
  }

  ArrowSchema* string_list = schema->children[1]->children[4];
  code = SetNamedType(string_list, NANOARROW_TYPE_LIST, "string_list");
  if (code != NANOARROW_OK) {
    return code;
  }
  code = ArrowSchemaSetType(string_list->children[0], NANOARROW_TYPE_STRING);
  if (code != NANOARROW_OK) {
    return code;
  }

  ArrowSchema* map = schema->children[1]->children[5];
  code = SetNamedType(map, NANOARROW_TYPE_MAP, "int32_to_int32_list_map");
  if (code != NANOARROW_OK) {
    return code;
  }
  code =
      ArrowSchemaSetType(map->children[0]->children[0], NANOARROW_TYPE_INT32);
  if (code != NANOARROW_OK) {
    return code;
  }

  ArrowSchema* map_value = map->children[0]->children[1];
  code = ArrowSchemaSetType(map_value, NANOARROW_TYPE_LIST);
  if (code != NANOARROW_OK) {
    return code;
  }
  return ArrowSchemaSetType(map_value->children[0], NANOARROW_TYPE_INT32);
}

ArrowStringView StringView(std::string const& value) {
  return ArrowStringView{value.data(), static_cast<int64_t>(value.size())};
}

ArrowErrorCode AppendRow(ArrowArray* array, InfoRow const& row) {
  ArrowErrorCode code = ArrowArrayAppendUInt(array->children[0], row.code);
  if (code != NANOARROW_OK) {
    return code;
  }

  ArrowArray* union_array = array->children[1];
  switch (row.kind) {
    case ValueKind::kString:
      code = ArrowArrayAppendString(union_array->children[0],
                                    StringView(row.string_value));
      if (code != NANOARROW_OK) {
        return code;
      }
      code = ArrowArrayFinishUnionElement(union_array, 0);
      break;
    case ValueKind::kBool:
      code =
          ArrowArrayAppendInt(union_array->children[1], row.bool_value ? 1 : 0);
      if (code != NANOARROW_OK) {
        return code;
      }
      code = ArrowArrayFinishUnionElement(union_array, 1);
      break;
    case ValueKind::kInt64:
      code = ArrowArrayAppendInt(union_array->children[2], row.int64_value);
      if (code != NANOARROW_OK) {
        return code;
      }
      code = ArrowArrayFinishUnionElement(union_array, 2);
      break;
  }

  if (code != NANOARROW_OK) {
    return code;
  }
  return ArrowArrayFinishElement(array);
}

GetInfoResult BuildArray(ArrowSchema* schema, std::vector<InfoRow> const& rows,
                         ArrowArray* array) {
  ArrowError error;
  ArrowErrorInit(&error);
  ArrowErrorCode code = ArrowArrayInitFromSchema(array, schema, &error);
  if (code != NANOARROW_OK) {
    return NanoarrowError("initialize GetInfo array from schema", code, &error);
  }

  code = ArrowArrayStartAppending(array);
  if (code != NANOARROW_OK) {
    return NanoarrowError("start appending GetInfo array", code);
  }

  for (InfoRow const& row : rows) {
    code = AppendRow(array, row);
    if (code != NANOARROW_OK) {
      return NanoarrowError("append GetInfo row", code);
    }
  }

  ArrowErrorInit(&error);
  code = ArrowArrayFinishBuildingDefault(array, &error);
  if (code != NANOARROW_OK) {
    return NanoarrowError("finish building GetInfo array", code, &error);
  }
  return {};
}

}  // namespace

GetInfoResult BuildGetInfoStream(std::string const& remote_vendor_version,
                                 uint32_t const* info_codes,
                                 std::size_t info_codes_length,
                                 ArrowArrayStream* out) {
  if (out == nullptr) {
    return Error(ADBC_STATUS_INVALID_ARGUMENT,
                 "GetInfo output stream must not be null");
  }

  ResetStream(out);

  SchemaHolder schema;
  ArrowErrorCode code = BuildGetInfoSchema(&schema.schema);
  if (code != NANOARROW_OK) {
    return NanoarrowError("build GetInfo schema", code);
  }

  std::vector<InfoRow> rows =
      BuildRows(remote_vendor_version, info_codes, info_codes_length);
  ArrayHolder array;
  GetInfoResult result = BuildArray(&schema.schema, rows, &array.array);
  if (result.status != ADBC_STATUS_OK) {
    return result;
  }

  code = ArrowBasicArrayStreamInit(out, &schema.schema, 1);
  if (code != NANOARROW_OK) {
    return NanoarrowError("initialize GetInfo array stream", code);
  }

  ArrowBasicArrayStreamSetArray(out, 0, &array.array);
  return {};
}

}  // namespace adbc_driver_quack
