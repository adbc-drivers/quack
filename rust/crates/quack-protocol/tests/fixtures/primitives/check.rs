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

use duckdb_wiredata::chunk::{DataChunk, Primitive};

pub const SQL: &str = include_str!("mixed.sql");
pub const MULTI_SQL: &str = "SELECT i::BIGINT AS i64, (i % 256)::UTINYINT AS u8, (i % 2 = 0)::BOOLEAN AS b FROM range(2050) t(i) ORDER BY i";

fn column<T: Primitive + std::fmt::Debug + PartialEq>(
    chunk: &DataChunk,
    column: usize,
    offset: usize,
    expected: &[Option<T>],
) {
    for row in 0..chunk.rows() {
        assert_eq!(
            chunk.columns()[column].get::<T>(row),
            Some(expected[offset + row])
        );
    }
}

pub fn check_mixed(chunk: &DataChunk, offset: usize) {
    assert_eq!(chunk.columns().len(), 11);
    column(
        chunk,
        0,
        offset,
        &[
            Some(false),
            Some(true),
            None,
            Some(true),
            Some(true),
            Some(true),
        ],
    );
    column::<i8>(
        chunk,
        1,
        offset,
        &[
            Some(i8::MIN),
            Some(i8::MAX),
            None,
            Some(-7),
            Some(-7),
            Some(-7),
        ],
    );
    column::<i16>(
        chunk,
        2,
        offset,
        &[
            Some(i16::MIN),
            Some(i16::MAX),
            None,
            Some(-7),
            Some(-7),
            Some(-7),
        ],
    );
    column::<i32>(
        chunk,
        3,
        offset,
        &[
            Some(i32::MIN),
            Some(i32::MAX),
            None,
            Some(-7),
            Some(-7),
            Some(-7),
        ],
    );
    column::<i64>(
        chunk,
        4,
        offset,
        &[
            Some(i64::MIN),
            Some(i64::MAX),
            None,
            Some(-7),
            Some(-7),
            Some(-7),
        ],
    );
    column::<u8>(
        chunk,
        5,
        offset,
        &[
            Some(u8::MIN),
            Some(u8::MAX),
            None,
            Some(7),
            Some(7),
            Some(7),
        ],
    );
    column::<u16>(
        chunk,
        6,
        offset,
        &[
            Some(u16::MIN),
            Some(u16::MAX),
            None,
            Some(7),
            Some(7),
            Some(7),
        ],
    );
    column::<u32>(
        chunk,
        7,
        offset,
        &[
            Some(u32::MIN),
            Some(u32::MAX),
            None,
            Some(7),
            Some(7),
            Some(7),
        ],
    );
    column::<u64>(
        chunk,
        8,
        offset,
        &[
            Some(u64::MIN),
            Some(u64::MAX),
            None,
            Some(7),
            Some(7),
            Some(7),
        ],
    );
    for row in 0..chunk.rows() {
        let value = chunk.columns()[9].get::<f32>(row).unwrap();
        match offset + row {
            0 => assert_eq!(value.unwrap().to_bits(), (-0.0f32).to_bits()),
            1 => assert_eq!(value, Some(1.5)),
            2 => assert_eq!(value, None),
            3 => assert_eq!(value, Some(f32::INFINITY)),
            4 => assert_eq!(value, Some(f32::NEG_INFINITY)),
            5 => assert!(value.unwrap().is_nan()),
            _ => panic!("unexpected row"),
        }
    }
    for row in 0..chunk.rows() {
        let value = chunk.columns()[10].get::<f64>(row).unwrap();
        match offset + row {
            0 => assert_eq!(value.unwrap().to_bits(), (-0.0f64).to_bits()),
            1 => assert_eq!(value, Some(1.5)),
            2 => assert_eq!(value, None),
            3 => assert_eq!(value, Some(f64::INFINITY)),
            4 => assert_eq!(value, Some(f64::NEG_INFINITY)),
            5 => assert!(value.unwrap().is_nan()),
            _ => panic!("unexpected row"),
        }
    }
}

pub fn check_multi(chunk: &DataChunk, offset: usize) {
    for row in 0..chunk.rows() {
        let i = offset + row;
        assert_eq!(chunk.columns()[0].get::<i64>(row), Some(Some(i as i64)));
        assert_eq!(
            chunk.columns()[1].get::<u8>(row),
            Some(Some((i % 256) as u8))
        );
        assert_eq!(
            chunk.columns()[2].get::<bool>(row),
            Some(Some(i.is_multiple_of(2)))
        );
    }
}
