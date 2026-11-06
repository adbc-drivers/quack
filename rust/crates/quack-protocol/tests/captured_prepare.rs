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

// Exact curl response captured 2026-11-03 from duckdb/duckdb:2.0.0, linux_amd64.
// Handshake: v2.0.0 / Quack 3; storage serialization target V2_0_0.
// PREPARE used inline_rows=0, query UUID (upper=0, lower=1).
// Server response has no session ID; no bytes were redacted or rewritten.
// SQL: SELECT NULL::BOOLEAN AS boolean_col, NULL::INTEGER AS integer_col, NULL::VARCHAR AS varchar_col, NULL::DECIMAL(18,4) AS decimal_col, NULL::ENUM('red','green','blue') AS enum_col, NULL::INTEGER[] AS list_col, NULL::INTEGER[3] AS array_col, NULL::MAP(VARCHAR,INTEGER) AS map_col, NULL::STRUCT(a INTEGER,b VARCHAR[]) AS struct_col, NULL::UNION(i INTEGER,s VARCHAR) AS union_col, 'x' COLLATE nocase AS collation_col, NULL::UUID AS uuid_col, NULL::TIMESTAMP_NS AS timestamp_col, NULL::VARIANT AS variant_col

use duckdb_wiredata::logical_type::LogicalTypeId as Id;
use quack_protocol::{
    codec,
    message::{HugeInt, MessageBody},
};

const RESPONSE: &str = "01 00 04 03 00 ff ff ff ff ff ff ff ff ff 01 ff ff 01 00 0e 64 00 0a ff ff 64 00 0d ff ff 64 00 19 ff ff 64 00 15 65 00 01 64 00 02 c8 00 12 c9 00 04 ff ff ff ff 64 00 68 65 00 01 64 00 06 c8 00 03 c9 00 03 03 72 65 64 05 67 72 65 65 6e 04 62 6c 75 65 ff ff ff ff 64 00 65 65 00 01 64 00 04 c8 00 64 00 0d ff ff ff ff ff ff 64 00 6c 65 00 01 64 00 09 c8 00 64 00 0d ff ff c9 00 03 ff ff ff ff 64 00 66 65 00 01 64 00 04 c8 00 64 00 64 65 00 01 64 00 05 c8 00 02 00 00 03 6b 65 79 01 00 64 00 19 ff ff ff ff 00 00 05 76 61 6c 75 65 01 00 64 00 0d ff ff ff ff ff ff ff ff ff ff ff ff 64 00 64 65 00 01 64 00 05 c8 00 02 00 00 01 61 01 00 64 00 0d ff ff ff ff 00 00 01 62 01 00 64 00 65 65 00 01 64 00 04 c8 00 64 00 19 ff ff ff ff ff ff ff ff ff ff ff ff 64 00 6b 65 00 01 64 00 05 c8 00 03 00 00 00 01 00 64 00 1c ff ff ff ff 00 00 01 69 01 00 64 00 0d ff ff ff ff 00 00 01 73 01 00 64 00 19 ff ff ff ff ff ff ff ff 64 00 19 65 00 01 64 00 03 c8 00 06 6e 6f 63 61 73 65 ff ff ff ff 64 00 36 ff ff 64 00 14 ff ff 64 00 6d 65 00 01 64 00 05 c8 00 04 00 00 04 6b 65 79 73 01 00 64 00 65 65 00 01 64 00 04 c8 00 64 00 19 ff ff ff ff ff ff ff ff 00 00 08 63 68 69 6c 64 72 65 6e 01 00 64 00 65 65 00 01 64 00 04 c8 00 64 00 64 65 00 01 64 00 05 c8 00 02 00 00 0a 6b 65 79 73 5f 69 6e 64 65 78 01 00 64 00 1e ff ff ff ff 00 00 0c 76 61 6c 75 65 73 5f 69 6e 64 65 78 01 00 64 00 1e ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 00 06 76 61 6c 75 65 73 01 00 64 00 65 65 00 01 64 00 04 c8 00 64 00 64 65 00 01 64 00 05 c8 00 02 00 00 07 74 79 70 65 5f 69 64 01 00 64 00 1c ff ff ff ff 00 00 0b 62 79 74 65 5f 6f 66 66 73 65 74 01 00 64 00 1e ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 00 04 64 61 74 61 01 00 64 00 1a ff ff ff ff ff ff ff ff 02 00 0e 0b 62 6f 6f 6c 65 61 6e 5f 63 6f 6c 0b 69 6e 74 65 67 65 72 5f 63 6f 6c 0b 76 61 72 63 68 61 72 5f 63 6f 6c 0b 64 65 63 69 6d 61 6c 5f 63 6f 6c 08 65 6e 75 6d 5f 63 6f 6c 08 6c 69 73 74 5f 63 6f 6c 09 61 72 72 61 79 5f 63 6f 6c 07 6d 61 70 5f 63 6f 6c 0a 73 74 72 75 63 74 5f 63 6f 6c 09 75 6e 69 6f 6e 5f 63 6f 6c 0d 63 6f 6c 6c 61 74 69 6f 6e 5f 63 6f 6c 08 75 75 69 64 5f 63 6f 6c 0d 74 69 6d 65 73 74 61 6d 70 5f 63 6f 6c 0b 76 61 72 69 61 6e 74 5f 63 6f 6c 03 00 01 05 00 00 01 ff ff";

fn bytes() -> Vec<u8> {
    RESPONSE
        .split_whitespace()
        .map(|v| u8::from_str_radix(v, 16).unwrap())
        .collect()
}
#[test]
fn captured_prepare_metadata() {
    let wire = bytes();
    let message = codec::decode(wire.clone()).unwrap();
    let MessageBody::PrepareResponse(response) = &message.body else {
        panic!("expected prepare");
    };
    assert_eq!(
        response
            .result_types()
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![
            Id::Boolean,
            Id::Integer,
            Id::Varchar,
            Id::Decimal,
            Id::Enum,
            Id::List,
            Id::Array,
            Id::Map,
            Id::Struct,
            Id::Union,
            Id::Varchar,
            Id::Uuid,
            Id::TimestampNs,
            Id::Variant
        ]
    );
    assert_eq!(
        response
            .result_names()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "boolean_col",
            "integer_col",
            "varchar_col",
            "decimal_col",
            "enum_col",
            "list_col",
            "array_col",
            "map_col",
            "struct_col",
            "union_col",
            "collation_col",
            "uuid_col",
            "timestamp_col",
            "variant_col"
        ]
    );
    assert!(response.needs_more_fetch());
    assert_eq!(response.query_uuid(), HugeInt { upper: 0, lower: 1 });
    assert_eq!(codec::encode(&message).unwrap().to_vec(), wire);
}
#[test]
fn captured_prepare_truncations() {
    let wire = bytes();
    for end in 0..wire.len() {
        assert!(
            codec::decode(wire[..end].to_vec()).is_err(),
            "truncation {end}"
        );
    }
}
