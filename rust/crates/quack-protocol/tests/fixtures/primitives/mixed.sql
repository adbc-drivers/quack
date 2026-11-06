-- Copyright (c) 2026 ADBC Drivers Contributors
--
-- Licensed under the Apache License, Version 2.0 (the "License");
-- you may not use this file except in compliance with the License.
-- You may obtain a copy of the License at
--
--         http:--www.apache.org/licenses/LICENSE-2.0
--
-- Unless required by applicable law or agreed to in writing, software
-- distributed under the License is distributed on an "AS IS" BASIS,
-- WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
-- See the License for the specific language governing permissions and
-- limitations under the License.


SELECT
  CASE i WHEN 0 THEN false WHEN 1 THEN true WHEN 2 THEN NULL ELSE true END::BOOLEAN AS b,
  CASE i WHEN 0 THEN -128 WHEN 1 THEN 127 WHEN 2 THEN NULL ELSE -7 END::TINYINT AS i8,
  CASE i WHEN 0 THEN -32768 WHEN 1 THEN 32767 WHEN 2 THEN NULL ELSE -7 END::SMALLINT AS i16,
  CASE i WHEN 0 THEN -2147483648 WHEN 1 THEN 2147483647 WHEN 2 THEN NULL ELSE -7 END::INTEGER AS i32,
  CASE i WHEN 0 THEN '-9223372036854775808' WHEN 1 THEN '9223372036854775807' WHEN 2 THEN NULL ELSE '-7' END::BIGINT AS i64,
  CASE i WHEN 0 THEN 0 WHEN 1 THEN 255 WHEN 2 THEN NULL ELSE 7 END::UTINYINT AS u8,
  CASE i WHEN 0 THEN 0 WHEN 1 THEN 65535 WHEN 2 THEN NULL ELSE 7 END::USMALLINT AS u16,
  CASE i WHEN 0 THEN 0 WHEN 1 THEN 4294967295 WHEN 2 THEN NULL ELSE 7 END::UINTEGER AS u32,
  CASE i WHEN 0 THEN '0' WHEN 1 THEN '18446744073709551615' WHEN 2 THEN NULL ELSE '7' END::UBIGINT AS u64,
  CASE i WHEN 0 THEN '-0.0' WHEN 1 THEN '1.5' WHEN 2 THEN NULL WHEN 3 THEN 'Infinity' WHEN 4 THEN '-Infinity' ELSE 'NaN' END::FLOAT AS f32,
  CASE i WHEN 0 THEN '-0.0' WHEN 1 THEN '1.5' WHEN 2 THEN NULL WHEN 3 THEN 'Infinity' WHEN 4 THEN '-Infinity' ELSE 'NaN' END::DOUBLE AS f64
FROM range(6) t(i) ORDER BY i
