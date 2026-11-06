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

use crate::{
    decode::logical_type,
    logical_type::{LogicalType, LogicalTypeId as Id, TypeInfo, TypeInfoKind as Kind},
};
use rstest::rstest;
#[path = "captured_types.rs"]
mod captured;
fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|v| u8::from_str_radix(v, 16).unwrap())
        .collect()
}
fn plain(id: Id) -> LogicalType {
    LogicalType { id, info: None }
}
fn typed(id: Id, kind: Kind) -> LogicalType {
    LogicalType {
        id,
        info: Some(TypeInfo {
            alias: vec![],
            extension_info: false,
            kind,
        }),
    }
}
fn list(child: LogicalType) -> LogicalType {
    typed(
        Id::List,
        Kind::List {
            child: Box::new(child),
        },
    )
}
fn structure(id: Id, fields: Vec<(&str, LogicalType)>) -> LogicalType {
    typed(
        id,
        Kind::Struct {
            children: fields
                .into_iter()
                .map(|(n, t)| (n.as_bytes().to_vec(), t))
                .collect(),
        },
    )
}
fn variant() -> LogicalType {
    structure(
        Id::Variant,
        vec![
            ("keys", list(plain(Id::Varchar))),
            (
                "children",
                list(structure(
                    Id::Struct,
                    vec![
                        ("keys_index", plain(Id::UInteger)),
                        ("values_index", plain(Id::UInteger)),
                    ],
                )),
            ),
            (
                "values",
                list(structure(
                    Id::Struct,
                    vec![
                        ("type_id", plain(Id::UTinyInt)),
                        ("byte_offset", plain(Id::UInteger)),
                    ],
                )),
            ),
            ("data", plain(Id::Blob)),
        ],
    )
}
#[rstest]
#[case(captured::BOOLEAN, plain(Id::Boolean))]
#[case(captured::INTEGER, plain(Id::Integer))]
#[case(captured::VARCHAR, plain(Id::Varchar))]
#[case(captured::DECIMAL, typed(Id::Decimal,Kind::Decimal { width:18,scale:4 }))]
#[case(captured::ENUM, typed(Id::Enum,Kind::Enum { values:vec![b"red".to_vec(),b"green".to_vec(),b"blue".to_vec()] }))]
#[case(captured::LIST, list(plain(Id::Integer)))]
#[case(captured::ARRAY, typed(Id::Array,Kind::Array { child:Box::new(plain(Id::Integer)),size:3 }))]
#[case(captured::MAP, typed(Id::Map,Kind::List { child:Box::new(structure(Id::Struct,vec![("key",plain(Id::Varchar)),("value",plain(Id::Integer))])) }))]
#[case(captured::STRUCT, structure(Id::Struct,vec![("a",plain(Id::Integer)),("b",list(plain(Id::Varchar)))]))]
#[case(captured::UNION, structure(Id::Union,vec![("",plain(Id::UTinyInt)),("i",plain(Id::Integer)),("s",plain(Id::Varchar))]))]
#[case(captured::COLLATION, typed(Id::Varchar,Kind::String { collation:b"nocase".to_vec() }))]
#[case(captured::UUID, plain(Id::Uuid))]
#[case(captured::TIMESTAMP, plain(Id::TimestampNs))]
#[case(captured::VARIANT, variant())]
fn captured_types(#[case] fixture: &[u8], #[case] expected: LogicalType) {
    let bytes = fixture;
    assert_eq!(
        logical_type(bytes).unwrap(),
        (expected.clone(), bytes.len())
    );
    for end in 0..bytes.len() {
        assert!(logical_type(&bytes[..end]).is_err(), "truncation {end}");
    }
    let mut trailing = bytes.to_vec();
    trailing.extend([0xde, 0xad]);
    assert_eq!(logical_type(&trailing).unwrap(), (expected, bytes.len()));
}
// Constructed fixtures for metadata not exposed by this capture query.
#[rstest]
#[case("64 00 0d 65 00 00 ff ff", plain(Id::Integer))]
#[case("64 00 0d 65 00 01 64 00 01 65 00 02 78 ff ff ff ff ff", LogicalType { id:Id::Integer,info:Some(TypeInfo {alias:vec![b'x',255],extension_info:false,kind:Kind::Generic}) })]
#[case("64 00 03 65 00 01 64 00 0a c8 00 64 00 0d ff ff ff ff ff ff",typed(Id::Any,Kind::Any{target:Box::new(plain(Id::Integer)),cast_score:0}))]
#[case("64 00 05 65 00 01 64 00 0c c8 00 01 54 ff ff ff ff",typed(Id::Template,Kind::Template{name:b"T".to_vec()}))]
#[case("64 00 3c 65 00 01 64 00 0d c8 00 64 00 04 45 50 53 47 ff ff ff ff ff ff",typed(Id::Geometry,Kind::Geometry{crs:b"EPSG".to_vec()}))]
#[case("64 00 0d 65 00 01 64 00 01 66 00 00 67 00 01 64 00 00 65 00 00 ff ff ff ff ff ff",LogicalType { id:Id::Integer,info:Some(TypeInfo {alias:vec![],extension_info:true,kind:Kind::Generic}) })]
fn synthetic_metadata(#[case] fixture: &str, #[case] expected: LogicalType) {
    let bytes = hex(fixture);
    assert_eq!(logical_type(&bytes).unwrap(), (expected, bytes.len()));
}
#[rstest]
#[case("64 00 80 02 ff ff", "overflow")]
#[case("64 00 80 80", "encoded width")]
#[case("64 00 7f ff ff", "unsupported logical")]
#[case("64 00 0d 65 00 02 ff ff", "boolean")]
#[case("64 00 0d 64 00 0d ff ff", "expected field")]
#[case("64 00 0d 65 00 01 64 00 0e ff ff ff ff", "discriminator")]
#[case("64 00 0d 65 00 01 64 00 01 66 00 01", "Value-bearing")]
#[case("64 00 0d 65 00 01 64 00 01 67 00 01 64 00 01", "Value-bearing")]
#[case("64 00 0d 65 00 01 64 00 01 67 00 01 65 00 01", "Value-bearing")]
#[case("64 00 26 65 00 01 64 00 0b ff ff", "INTEGER_LITERAL")]
#[case("64 00 04 65 00 01 64 00 07 ff ff", "parsed-expression")]
#[case(
    "64 00 64 65 00 01 64 00 05 c8 00 ff ff ff ff ff ff ff ff ff 01",
    "count"
)]
#[case("64 00 19 65 00 01 64 00 03 c8 00 ff ff ff ff 0f", "truncated")]
#[case(
    "64 00 68 65 00 01 64 00 06 c8 00 02 c9 00 01 00 ff ff ff ff",
    "dictionary"
)]
fn malformed(#[case] fixture: &str, #[case] message: &str) {
    let bytes = hex(fixture);
    let error = logical_type(&bytes).unwrap_err();
    assert!(error.message.contains(message), "{error}");
    assert!(error.offset <= bytes.len());
}
#[test]
fn nesting_limit() {
    let mut bytes = captured::INTEGER.to_vec();
    for _ in 1..64 {
        let mut outer = hex("64 00 65 65 00 01 64 00 04 c8 00");
        outer.extend(bytes);
        outer.extend([255; 4]);
        bytes = outer;
    }
    assert!(logical_type(&bytes).is_ok());
    let mut outer = hex("64 00 65 65 00 01 64 00 04 c8 00");
    outer.extend(bytes);
    outer.extend([255; 4]);
    assert!(
        logical_type(&outer)
            .unwrap_err()
            .message
            .contains("nesting")
    );
}
