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
use arrow_array::{Array, Int32Array};
use bytes::Bytes;
use duckdb_wiredata::chunk::decode;

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

fn integer_schema(names: &[&str]) -> SchemaRef {
    let types = names
        .iter()
        .map(|_| LogicalType {
            id: LogicalTypeId::Integer,
            info: None,
        })
        .collect::<Vec<_>>();
    schema(
        &types,
        &names
            .iter()
            .map(|name| WireString::from(*name))
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

// Add an ignored prefix to select the actual alignment of the values view.
fn positioned(wire: Vec<u8>, aligned: bool) -> (Bytes, DataChunk) {
    for prefix in 0..4 {
        let mut storage = vec![0; prefix];
        storage.extend_from_slice(&wire);
        let owner = Bytes::from(storage);
        let (decoded, _) = decode(owner.slice(prefix..)).unwrap();
        let PrimitiveEncoding::Flat { values, .. } = decoded.columns()[0].encoding() else {
            panic!("flat required")
        };
        if (values.as_ptr().align_offset(4) == 0) == aligned {
            return (owner, decoded);
        }
    }
    panic!("alignment unavailable")
}

#[test]
fn aligned_values_and_padded_validity_share_owner() {
    let mask = [5, 255, 255, 255, 255, 255, 255, 255];
    let (owner, decoded) = positioned(chunk(3, vec![flat(&[42, 0, -7], Some(&mask))]), true);
    let PrimitiveEncoding::Flat { values, validity } = decoded.columns()[0].encoding() else {
        panic!("flat required")
    };
    let values_ptr = values.as_ptr();
    let mask_ptr = validity.unwrap().as_ptr();
    let result = record_batch(decoded, integer_schema(&["answer"])).unwrap();
    drop(owner);
    let array = result
        .column(0)
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    if cfg!(target_endian = "little") {
        assert_eq!(array.values().inner().as_ptr(), values_ptr);
    }
    assert_eq!(array.nulls().unwrap().inner().inner().as_ptr(), mask_ptr);
    assert_eq!(
        array.iter().collect::<Vec<_>>(),
        vec![Some(42), None, Some(-7)]
    );
    assert_eq!(array.null_count(), 1);
}

#[test]
fn unaligned_values_materialize_but_validity_still_shared() {
    let (owner, decoded) = positioned(chunk(3, vec![flat(&[42, 0, -7], Some(&[5; 8]))]), false);
    let PrimitiveEncoding::Flat { values, validity } = decoded.columns()[0].encoding() else {
        panic!("flat required")
    };
    let values_ptr = values.as_ptr();
    let mask_ptr = validity.unwrap().as_ptr();
    let result = record_batch(decoded, integer_schema(&["answer"])).unwrap();
    let array = result
        .column(0)
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    assert_ne!(array.values().inner().as_ptr(), values_ptr);
    assert_eq!(array.values().inner().as_ptr().align_offset(4), 0);
    assert_eq!(array.nulls().unwrap().inner().inner().as_ptr(), mask_ptr);
    drop(owner);
    assert_eq!(
        array.iter().collect::<Vec<_>>(),
        vec![Some(42), None, Some(-7)]
    );
}

#[test]
fn compressed_columns_expand_into_arrow() {
    let (decoded, _) = decode(chunk(
        4,
        vec![
            constant(flat(&[42], None)),
            constant(flat(&[0], Some(&[0; 8]))),
            dictionary(&[1, 0, 1, 0], 2, flat(&[42, -7], Some(&[1; 8]))),
            sequence(-7, 3),
            dictionary(&[0, 1, 0, 1], 2, constant(flat(&[17], None))),
        ],
    ))
    .unwrap();
    let shared_schema = integer_schema(&["a", "b", "c", "d", "e"]);
    let result = record_batch(decoded, shared_schema.clone()).unwrap();
    assert!(Arc::ptr_eq(&result.schema(), &shared_schema));
    let expected = [
        vec![Some(42); 4],
        vec![None; 4],
        vec![None, Some(42), None, Some(42)],
        vec![Some(-7), Some(-4), Some(-1), Some(2)],
        vec![Some(17); 4],
    ];
    for (column, expected) in result.columns().iter().zip(expected) {
        let array = column.as_any().downcast_ref::<Int32Array>().unwrap();
        assert_eq!(array.iter().collect::<Vec<_>>(), expected);
        if array.null_count() == 0 {
            assert!(array.nulls().is_none());
        }
    }
}

#[test]
fn empty_columns_and_zero_column_row_count() {
    let (decoded, _) = decode(chunk(0, vec![flat(&[], None), sequence(42, 1)])).unwrap();
    let result = record_batch(decoded, integer_schema(&["a", "b"])).unwrap();
    assert_eq!(result.num_rows(), 0);
    assert_eq!(result.num_columns(), 2);
    let (decoded, _) = decode(chunk(3, vec![])).unwrap();
    let result = record_batch(decoded, integer_schema(&[])).unwrap();
    assert_eq!(result.num_rows(), 3);
}

#[test]
fn sibling_batches_share_one_response_and_survive_drop() {
    let first = chunk(1, vec![flat(&[42], None)]);
    let second = chunk(1, vec![flat(&[-7], None)]);
    let owner = Bytes::from([first.as_slice(), second.as_slice()].concat());
    let allocation_start = owner.as_ptr() as usize;
    let allocation_end = allocation_start + owner.len();
    let (first_chunk, consumed) = decode(owner.clone()).unwrap();
    let (second_chunk, _) = decode(owner.slice(consumed..)).unwrap();
    let shared_schema = integer_schema(&["answer"]);
    let first_batch = record_batch(first_chunk, shared_schema.clone()).unwrap();
    let second_batch = record_batch(second_chunk, shared_schema).unwrap();
    drop(owner);
    drop(first_batch);
    let array = second_batch
        .column(0)
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    if cfg!(target_endian = "little") {
        assert!((allocation_start..allocation_end).contains(&(array.values().as_ptr() as usize)));
    }
    assert_eq!(array.value(0), -7);
}

#[test]
fn schema_validation_and_mismatch_are_errors() {
    let integer = LogicalType {
        id: LogicalTypeId::Integer,
        info: None,
    };
    let varchar = LogicalType {
        id: LogicalTypeId::Varchar,
        info: None,
    };
    assert!(schema(std::slice::from_ref(&integer), &[]).is_err());
    assert!(matches!(
        schema(&[varchar], &["a".into()]),
        Err(ArrowError::NotYetImplemented(_))
    ));
    assert!(schema(&[integer], &[WireString(vec![255])]).is_err());
    let (decoded, _) = decode(chunk(1, vec![flat(&[42], None)])).unwrap();
    assert!(record_batch(decoded, integer_schema(&[])).is_err());
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
fn numeric_alignment_compression_and_bit_preservation() {
    macro_rules! check {
        ($native:ty, $arrow:ty, $id:ident, $bits:expr) => {{
            let bytes = $bits.to_le_bytes();
            let wire = typed_chunk(LogicalTypeId::$id, 1, raw_flat(&bytes, Some(&[1; 8])));
            for prefix in 0..size_of::<$native>() {
                let mut storage = vec![0; prefix];
                storage.extend_from_slice(&wire);
                let owner = Bytes::from(storage);
                let (chunk, _) = decode(owner.slice(prefix..)).unwrap();
                let PrimitiveEncoding::Flat { values, validity } = chunk.columns()[0].encoding()
                else {
                    panic!("flat")
                };
                let pointer = values.as_ptr();
                let validity_pointer = validity.unwrap().as_ptr();
                let array = numeric_array::<$arrow>(&chunk.columns()[0]).unwrap();
                drop(chunk);
                drop(owner);
                assert_eq!(array.value(0).to_le_bytes(), bytes);
                assert_eq!(
                    array.nulls().unwrap().inner().inner().as_ptr(),
                    validity_pointer
                );
                if cfg!(target_endian = "little")
                    && pointer.align_offset(align_of::<$native>()) == 0
                {
                    assert_eq!(array.values().inner().as_ptr(), pointer);
                } else {
                    assert_ne!(array.values().inner().as_ptr(), pointer);
                }
            }
            for vector in [
                constant(raw_flat(&bytes, None)),
                dictionary(&[0, 0, 0], 1, raw_flat(&bytes, None)),
            ] {
                let (chunk, _) = decode(typed_chunk(LogicalTypeId::$id, 3, vector)).unwrap();
                let array = numeric_array::<$arrow>(&chunk.columns()[0]).unwrap();
                for row in 0..3 {
                    assert_eq!(array.value(row).to_le_bytes(), bytes);
                }
            }
            let (chunk, _) = decode(typed_chunk(
                LogicalTypeId::$id,
                3,
                constant(raw_flat(&bytes, Some(&[0; 8]))),
            ))
            .unwrap();
            assert_eq!(
                numeric_array::<$arrow>(&chunk.columns()[0])
                    .unwrap()
                    .null_count(),
                3
            );
        }};
    }
    check!(i8, Int8Type, TinyInt, i8::MIN);
    check!(i16, Int16Type, SmallInt, i16::MIN);
    check!(i32, Int32Type, Integer, i32::MIN);
    check!(i64, Int64Type, BigInt, i64::MIN);
    check!(u8, UInt8Type, UTinyInt, u8::MAX);
    check!(u16, UInt16Type, USmallInt, u16::MAX);
    check!(u32, UInt32Type, UInteger, u32::MAX);
    check!(u64, UInt64Type, UBigInt, u64::MAX);
    check!(f32, Float32Type, Float, 0x7fc01234u32);
    check!(f64, Float64Type, Double, 0x7ff8000000001234u64);
}

#[test]
fn boolean_arrow_bitmap_boundaries_and_compression() {
    let mut bytes = vec![1; 65];
    bytes[0] = 0;
    bytes[64] = 255;
    let mut mask = [255; 16];
    mask[8] = 254;
    for vector in [
        raw_flat(&bytes, Some(&mask)),
        dictionary(
            &(0..65).collect::<Vec<_>>(),
            65,
            raw_flat(&bytes, Some(&mask)),
        ),
    ] {
        let (chunk, _) = decode(typed_chunk(LogicalTypeId::Boolean, 65, vector)).unwrap();
        let array = array(&chunk.columns()[0]).unwrap();
        let boolean = array.as_any().downcast_ref::<BooleanArray>().unwrap();
        assert!(!boolean.value(0));
        assert!(boolean.value(63));
        assert!(boolean.is_null(64));
        assert_eq!(boolean.null_count(), 1);
    }
    for (bytes, mask, nulls) in [([1], None, 0), ([255], Some([0; 8]), 65)] {
        let (chunk, _) = decode(typed_chunk(
            LogicalTypeId::Boolean,
            65,
            constant(raw_flat(&bytes, mask.as_ref().map(|m| m.as_slice()))),
        ))
        .unwrap();
        let array = array(&chunk.columns()[0]).unwrap();
        assert_eq!(array.null_count(), nulls);
    }
}

#[test]
fn same_width_schema_mismatch_is_rejected() {
    let (chunk, _) = decode(typed_chunk(
        LogicalTypeId::UInteger,
        1,
        raw_flat(&42u32.to_le_bytes(), None),
    ))
    .unwrap();
    assert!(matches!(
        record_batch(chunk, integer_schema(&["x"])),
        Err(ArrowError::SchemaError(_))
    ));
}
