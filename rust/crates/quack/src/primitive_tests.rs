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

use arrow_array::{
    Array, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    RecordBatch, UInt8Array, UInt16Array, UInt32Array, UInt64Array,
};

pub(crate) const SQL: &str =
    include_str!("../../quack-protocol/tests/fixtures/primitives/mixed.sql");
pub(crate) const MULTI_SQL: &str = "SELECT i::BIGINT AS i64, (i % 256)::UTINYINT AS u8, (i % 2 = 0)::BOOLEAN AS b FROM range(2050) t(i) ORDER BY i";

pub(crate) fn check_mixed(batch: &RecordBatch, offset: usize) {
    assert_eq!(batch.num_columns(), 11);
    let names = [
        "b", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64",
    ];
    for (field, name) in batch.schema().fields().iter().zip(names) {
        assert_eq!(field.name(), name);
        assert!(field.is_nullable());
    }
    let boolean = batch
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    for row in 0..batch.num_rows() {
        assert_eq!(boolean.is_null(row), offset + row == 2);
        if !boolean.is_null(row) {
            assert_eq!(boolean.value(row), offset + row != 0);
        }
    }
    let array = batch
        .column(1)
        .as_any()
        .downcast_ref::<Int8Array>()
        .unwrap();
    for row in 0..batch.num_rows() {
        assert_eq!(array.is_null(row), offset + row == 2);
        if !array.is_null(row) {
            assert_eq!(
                array.value(row),
                match offset + row {
                    0 => i8::MIN,
                    1 => i8::MAX,
                    _ => -7,
                }
            );
        }
    }
    let array = batch
        .column(2)
        .as_any()
        .downcast_ref::<Int16Array>()
        .unwrap();
    for row in 0..batch.num_rows() {
        assert_eq!(array.is_null(row), offset + row == 2);
        if !array.is_null(row) {
            assert_eq!(
                array.value(row),
                match offset + row {
                    0 => i16::MIN,
                    1 => i16::MAX,
                    _ => -7,
                }
            );
        }
    }
    let array = batch
        .column(3)
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    for row in 0..batch.num_rows() {
        assert_eq!(array.is_null(row), offset + row == 2);
        if !array.is_null(row) {
            assert_eq!(
                array.value(row),
                match offset + row {
                    0 => i32::MIN,
                    1 => i32::MAX,
                    _ => -7,
                }
            );
        }
    }
    let array = batch
        .column(4)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    for row in 0..batch.num_rows() {
        assert_eq!(array.is_null(row), offset + row == 2);
        if !array.is_null(row) {
            assert_eq!(
                array.value(row),
                match offset + row {
                    0 => i64::MIN,
                    1 => i64::MAX,
                    _ => -7,
                }
            );
        }
    }
    let array = batch
        .column(5)
        .as_any()
        .downcast_ref::<UInt8Array>()
        .unwrap();
    for row in 0..batch.num_rows() {
        assert_eq!(array.is_null(row), offset + row == 2);
        if !array.is_null(row) {
            assert_eq!(
                array.value(row),
                match offset + row {
                    0 => u8::MIN,
                    1 => u8::MAX,
                    _ => 7,
                }
            );
        }
    }
    let array = batch
        .column(6)
        .as_any()
        .downcast_ref::<UInt16Array>()
        .unwrap();
    for row in 0..batch.num_rows() {
        assert_eq!(array.is_null(row), offset + row == 2);
        if !array.is_null(row) {
            assert_eq!(
                array.value(row),
                match offset + row {
                    0 => u16::MIN,
                    1 => u16::MAX,
                    _ => 7,
                }
            );
        }
    }
    let array = batch
        .column(7)
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    for row in 0..batch.num_rows() {
        assert_eq!(array.is_null(row), offset + row == 2);
        if !array.is_null(row) {
            assert_eq!(
                array.value(row),
                match offset + row {
                    0 => u32::MIN,
                    1 => u32::MAX,
                    _ => 7,
                }
            );
        }
    }
    let array = batch
        .column(8)
        .as_any()
        .downcast_ref::<UInt64Array>()
        .unwrap();
    for row in 0..batch.num_rows() {
        assert_eq!(array.is_null(row), offset + row == 2);
        if !array.is_null(row) {
            assert_eq!(
                array.value(row),
                match offset + row {
                    0 => u64::MIN,
                    1 => u64::MAX,
                    _ => 7,
                }
            );
        }
    }
    let array = batch
        .column(9)
        .as_any()
        .downcast_ref::<Float32Array>()
        .unwrap();
    for row in 0..batch.num_rows() {
        assert_eq!(array.is_null(row), offset + row == 2);
        match offset + row {
            0 => assert_eq!(array.value(row).to_bits(), (-0.0f32).to_bits()),
            1 => assert_eq!(array.value(row), 1.5),
            2 => {}
            3 => assert_eq!(array.value(row), f32::INFINITY),
            4 => assert_eq!(array.value(row), f32::NEG_INFINITY),
            5 => assert!(array.value(row).is_nan()),
            _ => panic!("unexpected row"),
        }
    }
    let array = batch
        .column(10)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    for row in 0..batch.num_rows() {
        assert_eq!(array.is_null(row), offset + row == 2);
        match offset + row {
            0 => assert_eq!(array.value(row).to_bits(), (-0.0f64).to_bits()),
            1 => assert_eq!(array.value(row), 1.5),
            2 => {}
            3 => assert_eq!(array.value(row), f64::INFINITY),
            4 => assert_eq!(array.value(row), f64::NEG_INFINITY),
            5 => assert!(array.value(row).is_nan()),
            _ => panic!("unexpected row"),
        }
    }
}
pub(crate) fn check_multi(batch: &RecordBatch, offset: usize) {
    let integer = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    let unsigned = batch
        .column(1)
        .as_any()
        .downcast_ref::<UInt8Array>()
        .unwrap();
    let boolean = batch
        .column(2)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    for row in 0..batch.num_rows() {
        let i = offset + row;
        assert_eq!(integer.value(row), i as i64);
        assert_eq!(unsigned.value(row), (i % 256) as u8);
        assert_eq!(boolean.value(row), i.is_multiple_of(2));
    }
}

fn wire(text: &str) -> Vec<u8> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .flat_map(str::split_whitespace)
        .map(|v| u8::from_str_radix(v, 16).unwrap())
        .collect()
}

#[test]
fn captured_arrow_results() {
    use quack_protocol::{codec, message::MessageBody};
    for (prepare, fetch, expected) in [
        (
            include_str!("../../quack-protocol/tests/fixtures/primitives/mixed_inline_prepare.hex"),
            None,
            6,
        ),
        (
            include_str!(
                "../../quack-protocol/tests/fixtures/primitives/mixed_deferred_prepare.hex"
            ),
            Some(include_str!(
                "../../quack-protocol/tests/fixtures/primitives/mixed_deferred_fetch_1.hex"
            )),
            6,
        ),
        (
            include_str!("../../quack-protocol/tests/fixtures/primitives/empty_prepare.hex"),
            None,
            0,
        ),
        (
            include_str!("../../quack-protocol/tests/fixtures/primitives/multichunk_prepare.hex"),
            Some(include_str!(
                "../../quack-protocol/tests/fixtures/primitives/multichunk_fetch_1.hex"
            )),
            2050,
        ),
    ] {
        let MessageBody::PrepareResponse(prepare) = codec::decode(wire(prepare)).unwrap().body
        else {
            panic!("prepare")
        };
        let schema =
            crate::conversion::schema(prepare.result_types(), prepare.result_names()).unwrap();
        assert_eq!(schema.fields().len(), if expected == 2050 { 3 } else { 11 });
        let mut chunks = prepare.into_parts().results;
        if let Some(fetch) = fetch {
            let MessageBody::FetchResponse(fetch) = codec::decode(wire(fetch)).unwrap().body else {
                panic!("fetch")
            };
            chunks.extend(fetch.decode_chunks().unwrap());
        }
        let batches: Vec<_> = chunks
            .into_iter()
            .map(|c| crate::conversion::record_batch(c, schema.clone()).unwrap())
            .collect();
        let mut rows = 0;
        for batch in batches {
            if expected == 2050 {
                check_multi(&batch, rows);
            } else {
                check_mixed(&batch, rows);
            }
            rows += batch.num_rows();
        }
        assert_eq!(rows, expected);
    }
}
