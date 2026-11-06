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

use duckdb_wiredata::chunk::DataChunk;

pub const SQL: &str = include_str!("mixed.sql");

pub fn check(chunk: &DataChunk, offset: usize) {
    assert_eq!(chunk.columns().len(), 10);
    for row in 0..chunk.rows() {
        let phase = (offset + row) % 3;
        let columns = chunk.columns();
        let string = if phase == 2 {
            None
        } else if phase == 1 {
            Some(&b""[..])
        } else {
            Some("こんにちは".as_bytes())
        };
        assert_eq!(columns[0].get_bytes(row), Some(string));
        let binary = if phase == 2 {
            None
        } else if phase == 1 {
            Some(&b""[..])
        } else {
            Some(&[0, 255][..])
        };
        assert_eq!(columns[1].get_bytes(row), Some(binary));
        for (index, column) in columns.iter().enumerate().take(6).skip(2) {
            let coefficient = if index == 5 {
                -12345678901234567890123456789012345678i128
            } else {
                -123
            };
            assert_eq!(
                column.get_decimal(row),
                Some(if phase == 2 { None } else { Some(coefficient) })
            );
        }
        assert_eq!(
            columns[6].get_physical::<i32>(row),
            Some(if phase == 2 { None } else { Some(-1) })
        );
        for (index, column) in columns.iter().enumerate().take(10).skip(7) {
            assert_eq!(
                column.get_physical::<i64>(row),
                Some(if phase == 2 {
                    None
                } else if index == 7 {
                    Some(43200123456)
                } else {
                    Some(-1)
                })
            );
        }
    }
}
