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

use super::*;
use crate::decode::DecodeErrorKind;

fn uint(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let byte = (value & 127) as u8;
        value >>= 7;
        bytes.push(byte | if value == 0 { 0 } else { 128 });
        if value == 0 {
            return bytes;
        }
    }
}
fn signed(mut value: i64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let byte = (value & 127) as u8;
        value >>= 7;
        let done = (value == 0 && byte & 64 == 0) || (value == -1 && byte & 64 != 0);
        bytes.push(byte | if done { 0 } else { 128 });
        if done {
            return bytes;
        }
    }
}
fn field(id: u16, value: impl AsRef<[u8]>) -> Vec<u8> {
    let mut bytes = id.to_le_bytes().to_vec();
    bytes.extend_from_slice(value.as_ref());
    bytes
}
fn blob(bytes: &[u8]) -> Vec<u8> {
    let mut output = uint(bytes.len() as u64);
    output.extend_from_slice(bytes);
    output
}
fn flat(values: &[i32], validity: Option<&[u8]>) -> Vec<u8> {
    let mut bytes = field(100, [u8::from(validity.is_some())]);
    if let Some(validity) = validity {
        bytes.extend(field(101, blob(validity)));
    }
    let values: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    bytes.extend(field(102, blob(&values)));
    bytes
}
fn constant(child: Vec<u8>) -> Vec<u8> {
    let mut bytes = field(90, [2]);
    bytes.extend(child);
    bytes
}
fn dictionary(selection: &[u32], count: u64, child: Vec<u8>) -> Vec<u8> {
    let mut bytes = field(90, [3]);
    let selection: Vec<u8> = selection.iter().flat_map(|v| v.to_le_bytes()).collect();
    bytes.extend(field(91, blob(&selection)));
    bytes.extend(field(92, uint(count)));
    bytes.extend(child);
    bytes
}
fn sequence(start: i64, increment: i64) -> Vec<u8> {
    let mut bytes = field(90, [4]);
    bytes.extend(field(91, signed(start)));
    bytes.extend(field(92, signed(increment)));
    bytes
}
fn chunk(rows: u32, columns: Vec<Vec<u8>>) -> Vec<u8> {
    let mut bytes = field(100, uint(u64::from(rows)));
    let mut types = uint(columns.len() as u64);
    for _ in &columns {
        types.extend([100, 0, 13, 255, 255]);
    }
    bytes.extend(field(101, types));
    let mut vectors = uint(columns.len() as u64);
    for vector in columns {
        vectors.extend(vector);
        vectors.extend([255, 255]);
    }
    bytes.extend(field(102, vectors));
    bytes.extend([255, 255]);
    bytes
}
fn values(vector: &PrimitiveVector) -> Vec<Option<i32>> {
    (0..vector.len())
        .map(|index| vector.get::<i32>(index).unwrap())
        .collect()
}

#[test]
fn flat_shared_buffers_and_prefix() {
    let mask = [5, 0, 0, 0, 0, 0, 0, 0];
    let mut wire = chunk(
        3,
        vec![flat(&[42, 0, -7], Some(&mask)), flat(&[1, 2, 3], None)],
    );
    let size = wire.len();
    wire.extend([17, 18]);
    let input = Bytes::from(wire);
    let (decoded, consumed) = decode(input.clone()).unwrap();
    assert_eq!(consumed, size);
    assert_eq!(decoded.rows(), 3);
    assert_eq!(decoded.types().len(), 2);
    assert_eq!(
        values(&decoded.columns()[0]),
        vec![Some(42), None, Some(-7)]
    );
    assert_eq!(decoded.columns()[0].get::<i32>(3), None);
    let PrimitiveEncoding::Flat { values, validity } = decoded.columns()[0].encoding() else {
        panic!("expected flat")
    };
    let start = input.as_ptr() as usize;
    assert!((start..start + input.len()).contains(&(values.as_ptr() as usize)));
    assert!((start..start + input.len()).contains(&(validity.unwrap().as_ptr() as usize)));
    drop(input);
    assert_eq!(
        values.as_ref(),
        [42, 0, 0, 0, 0, 0, 0, 0, 249, 255, 255, 255]
    );
}

#[test]
fn compressed_vectors_remain_compressed() {
    let (decoded, _) = decode(chunk(
        4,
        vec![
            constant(flat(&[42], None)),
            constant(flat(&[0], Some(&[0; 8]))),
            dictionary(
                &[1, 0, 1, 0],
                2,
                flat(&[42, -7], Some(&[1, 0, 0, 0, 0, 0, 0, 0])),
            ),
            sequence(-7, 3),
        ],
    ))
    .unwrap();
    assert!(matches!(
        decoded.columns()[0].encoding(),
        PrimitiveEncoding::Constant { .. }
    ));
    assert!(matches!(
        decoded.columns()[2].encoding(),
        PrimitiveEncoding::Dictionary { .. }
    ));
    assert!(matches!(
        decoded.columns()[3].encoding(),
        PrimitiveEncoding::Sequence { .. }
    ));
    assert_eq!(values(&decoded.columns()[0]), vec![Some(42); 4]);
    assert_eq!(values(&decoded.columns()[1]), vec![None; 4]);
    assert_eq!(
        values(&decoded.columns()[2]),
        vec![None, Some(42), None, Some(42)]
    );
    assert_eq!(
        values(&decoded.columns()[3]),
        vec![Some(-7), Some(-4), Some(-1), Some(2)]
    );
}

#[test]
fn nested_dictionary_constant_and_empty() {
    let (decoded, _) = decode(chunk(
        3,
        vec![dictionary(&[0, 1, 0], 2, constant(flat(&[42], None)))],
    ))
    .unwrap();
    assert_eq!(values(&decoded.columns()[0]), vec![Some(42); 3]);
    for vector in [
        flat(&[], None),
        flat(&[], Some(&[])),
        dictionary(&[], 0, flat(&[], None)),
        sequence(i64::MAX, i64::MIN),
    ] {
        let (decoded, _) = decode(chunk(0, vec![vector])).unwrap();
        assert!(decoded.columns()[0].is_empty());
        assert_eq!(decoded.columns()[0].get::<i32>(0), None);
    }
}

#[test]
fn validity_word_boundary_and_padding() {
    let mut mask = [255; 16];
    mask[8] = 254;
    let (decoded, _) = decode(chunk(65, vec![flat(&[42; 65], Some(&mask))])).unwrap();
    assert_eq!(decoded.columns()[0].get::<i32>(63), Some(Some(42)));
    assert_eq!(decoded.columns()[0].get::<i32>(64), Some(None));
}

#[test]
fn huge_constant_has_no_row_allocation() {
    let (decoded, _) = decode(chunk(u32::MAX, vec![constant(flat(&[42], None))])).unwrap();
    assert_eq!(decoded.rows(), u32::MAX as usize);
    assert_eq!(
        decoded.columns()[0].get::<i32>(decoded.rows() - 1),
        Some(Some(42))
    );
}

#[test]
fn malformed_lengths_indices_sequences_and_recursion() {
    for wire in [
        chunk(2, vec![flat(&[42], None)]),
        chunk(1, vec![flat(&[42], Some(&[1]))]),
        chunk(2, vec![dictionary(&[0], 1, flat(&[42], None))]),
        chunk(1, vec![dictionary(&[1], 1, flat(&[42], None))]),
        chunk(1, vec![dictionary(&[0], 0, flat(&[], None))]),
        chunk(2, vec![sequence(i64::from(i32::MAX), 1)]),
        chunk(2, vec![sequence(i64::from(i32::MIN), -1)]),
        chunk(1, vec![sequence(i64::MAX, 0)]),
    ] {
        assert_eq!(decode(wire).unwrap_err().kind, DecodeErrorKind::Malformed);
    }
    let mut nested = flat(&[42], None);
    for _ in 0..64 {
        nested = constant(nested);
    }
    assert!(decode(chunk(1, vec![nested])).is_err());
    let mut sequence_bad = field(90, [4]);
    sequence_bad.extend(field(91, [255; 10]));
    assert!(decode(chunk(1, vec![sequence_bad])).is_err());
}

#[test]
fn unsupported_is_distinct() {
    for tag in [1, 5, 255] {
        assert_eq!(
            decode(chunk(1, vec![field(90, uint(tag))]))
                .unwrap_err()
                .kind,
            DecodeErrorKind::Unsupported
        );
    }
    let mut wire = chunk(1, vec![flat(&[42], None)]);
    let index = wire
        .windows(5)
        .position(|v| v == [100, 0, 13, 255, 255])
        .unwrap();
    wire[index + 2] = 25;
    assert_eq!(decode(wire).unwrap_err().kind, DecodeErrorKind::Unsupported);
}

#[test]
fn truncations_and_column_mismatch() {
    let wire = chunk(
        3,
        vec![dictionary(
            &[0, 1, 0],
            2,
            flat(&[42, 0], Some(&[1, 0, 0, 0, 0, 0, 0, 0])),
        )],
    );
    for end in 0..wire.len() {
        assert!(decode(wire[..end].to_vec()).is_err(), "truncation {end}");
    }
    let mut wire = chunk(1, vec![flat(&[42], None)]);
    let index = wire.windows(3).position(|v| v == [102, 0, 1]).unwrap();
    wire[index + 2] = 0;
    assert!(decode(wire).is_err());
}

fn fixture(text: &str) -> Bytes {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .flat_map(str::split_whitespace)
        .map(|v| u8::from_str_radix(v, 16).unwrap())
        .collect::<Vec<_>>()
        .into()
}

#[test]
fn captured_bare_chunks() {
    for (text, expected) in [
        (
            include_str!(
                "../../quack-protocol/tests/fixtures/integer/select42_deferred_fetch_1.hex"
            ),
            vec![42],
        ),
        (
            include_str!("../../quack-protocol/tests/fixtures/integer/fetch_rows_fetch_1.hex"),
            (0..5000).collect(),
        ),
    ] {
        let wire = fixture(text);
        let mut offset = wire
            .windows(3)
            .position(|v| v == [100, 0, 1] || v == [100, 0, 128])
            .unwrap();
        let mut actual = Vec::new();
        while offset < wire.len() {
            let (decoded, consumed) = decode(wire.slice(offset..)).unwrap();
            actual.extend(
                values(&decoded.columns()[0])
                    .into_iter()
                    .map(Option::unwrap),
            );
            offset += consumed;
        }
        assert_eq!(actual, expected);
    }
}

fn typed_chunk(id: LogicalTypeId, rows: u32, vector: Vec<u8>) -> Vec<u8> {
    let mut wire = chunk(rows, vec![vector]);
    let index = wire
        .windows(5)
        .position(|v| v == [100, 0, 13, 255, 255])
        .unwrap();
    wire[index + 2] = id as u8;
    wire
}

fn raw_flat(bytes: &[u8], validity: Option<&[u8]>) -> Vec<u8> {
    let mut wire = field(100, [u8::from(validity.is_some())]);
    if let Some(mask) = validity {
        wire.extend(field(101, blob(mask)));
    }
    wire.extend(field(102, blob(bytes)));
    wire
}

#[test]
fn primitive_widths_compression_and_truncations() {
    macro_rules! check {
        ($native:ty, $id:ident, $value:expr) => {{
            let value: $native = $value;
            let bytes = value.to_le_bytes();
            for (rows, vector) in [
                (1, raw_flat(&bytes, None)),
                (3, constant(raw_flat(&bytes, None))),
                (3, dictionary(&[0, 0, 0], 1, raw_flat(&bytes, None))),
                (
                    3,
                    dictionary(&[0, 1, 0], 2, constant(raw_flat(&bytes, None))),
                ),
            ] {
                let wire = typed_chunk(LogicalTypeId::$id, rows, vector);
                let (decoded, _) = decode(wire.clone()).unwrap();
                for row in 0..rows as usize {
                    assert_eq!(decoded.columns()[0].get::<$native>(row), Some(Some(value)));
                }
                assert_eq!(decoded.columns()[0].get::<$native>(rows as usize), None);
                for end in 0..wire.len() {
                    assert!(decode(wire[..end].to_vec()).is_err());
                }
            }
            let (decoded, _) = decode(typed_chunk(
                LogicalTypeId::$id,
                3,
                constant(raw_flat(&bytes, Some(&[0; 8]))),
            ))
            .unwrap();
            assert_eq!(decoded.columns()[0].get::<$native>(2), Some(None));
            assert!(decode(typed_chunk(LogicalTypeId::$id, 2, raw_flat(&bytes, None))).is_err());
        }};
    }
    check!(i8, TinyInt, i8::MIN);
    check!(i16, SmallInt, i16::MIN);
    check!(i32, Integer, i32::MIN);
    check!(i64, BigInt, i64::MIN);
    check!(u8, UTinyInt, u8::MAX);
    check!(u16, USmallInt, u16::MAX);
    check!(u32, UInteger, u32::MAX);
    check!(u64, UBigInt, u64::MAX);
    check!(f32, Float, f32::INFINITY);
    check!(f64, Double, f64::NEG_INFINITY);
}

#[test]
fn primitive_sequences_check_destination_range() {
    macro_rules! check {
        ($native:ty, $id:ident) => {{
            let (decoded, _) = decode(typed_chunk(LogicalTypeId::$id, 3, sequence(3, -1))).unwrap();
            assert_eq!(decoded.columns()[0].get::<$native>(2), Some(Some(1)));
            for (start, step) in [(<$native>::MIN as i64, -1), (<$native>::MAX as i64, 1)] {
                assert!(decode(typed_chunk(LogicalTypeId::$id, 2, sequence(start, step))).is_err());
            }
        }};
    }
    check!(i8, TinyInt);
    check!(i16, SmallInt);
    check!(i32, Integer);
    check!(i64, BigInt);
    check!(u8, UTinyInt);
    check!(u16, USmallInt);
    check!(u32, UInteger);
    let (decoded, _) = decode(typed_chunk(
        LogicalTypeId::UBigInt,
        3,
        sequence(i64::MAX, 1),
    ))
    .unwrap();
    assert_eq!(
        decoded.columns()[0].get::<u64>(2),
        Some(Some(i64::MAX as u64 + 2))
    );
    for vector in [sequence(-1, 0), sequence(i64::MAX, i64::MAX)] {
        assert!(decode(typed_chunk(LogicalTypeId::UBigInt, 3, vector)).is_err());
    }
    for id in [
        LogicalTypeId::Boolean,
        LogicalTypeId::Float,
        LogicalTypeId::Double,
    ] {
        assert_eq!(
            decode(typed_chunk(id, 0, sequence(0, 1))).unwrap_err().kind,
            DecodeErrorKind::Unsupported
        );
    }
}

#[test]
fn boolean_validity_compression_and_invalid_payloads() {
    let (decoded, _) = decode(typed_chunk(
        LogicalTypeId::Boolean,
        3,
        raw_flat(&[0, 255, 1], Some(&[5; 8])),
    ))
    .unwrap();
    assert_eq!(decoded.columns()[0].get::<bool>(0), Some(Some(false)));
    assert_eq!(decoded.columns()[0].get::<bool>(1), Some(None));
    assert_eq!(decoded.columns()[0].get::<bool>(2), Some(Some(true)));
    assert_eq!(decoded.columns()[0].get::<u8>(0), None);
    for vector in [
        raw_flat(&[2], None),
        constant(raw_flat(&[255], None)),
        dictionary(&[0], 1, raw_flat(&[2], None)),
    ] {
        assert_eq!(
            decode(typed_chunk(LogicalTypeId::Boolean, 1, vector))
                .unwrap_err()
                .kind,
            DecodeErrorKind::Malformed
        );
    }
    let (decoded, _) = decode(typed_chunk(
        LogicalTypeId::Boolean,
        3,
        dictionary(&[1, 0, 1], 2, raw_flat(&[0, 1], None)),
    ))
    .unwrap();
    assert_eq!(decoded.columns()[0].get::<bool>(0), Some(Some(true)));
    assert_eq!(decoded.columns()[0].get::<bool>(1), Some(Some(false)));
    let (decoded, _) = decode(typed_chunk(
        LogicalTypeId::Boolean,
        65,
        constant(raw_flat(&[1], None)),
    ))
    .unwrap();
    assert_eq!(decoded.columns()[0].get::<bool>(64), Some(Some(true)));
}

#[test]
fn float_payload_bits_survive_views() {
    for bits in [0x80000000u32, 0x7fc01234, 1, 0x7f7fffff] {
        let (decoded, _) = decode(typed_chunk(
            LogicalTypeId::Float,
            1,
            raw_flat(&bits.to_le_bytes(), None),
        ))
        .unwrap();
        assert_eq!(
            decoded.columns()[0]
                .get::<f32>(0)
                .unwrap()
                .unwrap()
                .to_bits(),
            bits
        );
    }
    for bits in [
        0x8000000000000000u64,
        0x7ff8000000001234,
        1,
        0x7fefffffffffffff,
    ] {
        let (decoded, _) = decode(typed_chunk(
            LogicalTypeId::Double,
            1,
            raw_flat(&bits.to_le_bytes(), None),
        ))
        .unwrap();
        assert_eq!(
            decoded.columns()[0]
                .get::<f64>(0)
                .unwrap()
                .unwrap()
                .to_bits(),
            bits
        );
    }
}
