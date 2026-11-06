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

//! Validated, shared views of little-endian DuckDB INTEGER chunks.

use crate::{
    decode::{DecodeError, Reader},
    logical_type::{LogicalType, LogicalTypeId},
};
use bytes::Bytes;

#[derive(Debug, PartialEq, Eq)]
pub struct DataChunk {
    rows: usize,
    types: Vec<LogicalType>,
    columns: Vec<IntegerVector>,
}

impl DataChunk {
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn types(&self) -> &[LogicalType] {
        &self.types
    }
    pub fn columns(&self) -> &[IntegerVector] {
        &self.columns
    }
    pub fn into_columns(self) -> Vec<IntegerVector> {
        self.columns
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct IntegerVector {
    len: usize,
    storage: Storage,
}

#[derive(Debug, PartialEq, Eq)]
enum Storage {
    Flat {
        values: Bytes,
        validity: Option<Bytes>,
    },
    Constant(Box<IntegerVector>),
    Dictionary {
        selection: Bytes,
        dictionary: Box<IntegerVector>,
    },
    Sequence {
        start: i64,
        increment: i64,
    },
}

/// Borrowed encoding information; all byte buffers are validated shared views.
#[derive(Debug)]
pub enum IntegerEncoding<'a> {
    Flat {
        values: &'a Bytes,
        validity: Option<&'a Bytes>,
    },
    Constant {
        value: &'a IntegerVector,
    },
    Dictionary {
        selection: &'a Bytes,
        dictionary: &'a IntegerVector,
    },
    Sequence {
        start: i64,
        increment: i64,
    },
}

impl IntegerVector {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn encoding(&self) -> IntegerEncoding<'_> {
        match &self.storage {
            Storage::Flat { values, validity } => IntegerEncoding::Flat {
                values,
                validity: validity.as_ref(),
            },
            Storage::Constant(value) => IntegerEncoding::Constant { value },
            Storage::Dictionary {
                selection,
                dictionary,
            } => IntegerEncoding::Dictionary {
                selection,
                dictionary,
            },
            Storage::Sequence { start, increment } => IntegerEncoding::Sequence {
                start: *start,
                increment: *increment,
            },
        }
    }
    /// `None` means out of bounds; `Some(None)` is a SQL NULL.
    pub fn get(&self, index: usize) -> Option<Option<i32>> {
        if index >= self.len {
            return None;
        }
        match &self.storage {
            Storage::Flat { values, validity } => {
                if validity
                    .as_ref()
                    .is_some_and(|mask| mask[index / 8] & (1 << (index % 8)) == 0)
                {
                    return Some(None);
                }
                let offset = index.checked_mul(4)?;
                let value = values.get(offset..)?.first_chunk::<4>()?;
                Some(Some(i32::from_le_bytes(*value)))
            }
            Storage::Constant(value) => value.get(0),
            Storage::Dictionary {
                selection,
                dictionary,
            } => {
                let offset = index.checked_mul(4)?;
                let selected = u32::from_le_bytes(*selection.get(offset..)?.first_chunk::<4>()?);
                dictionary.get(usize::try_from(selected).ok()?)
            }
            Storage::Sequence { start, increment } => {
                let value = i128::from(*start) + i128::from(*increment) * index as i128;
                Some(Some(i32::try_from(value).ok()?))
            }
        }
    }
}

/// Decode one chunk, leaving following chunk or wrapper bytes unconsumed.
/// Values, validity, and dictionary selections share the input allocation.
pub fn decode(bytes: impl Into<Bytes>) -> Result<(DataChunk, usize), DecodeError> {
    let bytes = bytes.into();
    let reader = Reader {
        bytes: &bytes,
        pos: 0,
        context: "DataChunk",
    };
    let mut parser = Parser {
        owner: &bytes,
        reader,
    };
    let chunk = parser.chunk()?;
    Ok((chunk, parser.reader.pos))
}

struct Parser<'a> {
    owner: &'a Bytes,
    reader: Reader<'a>,
}

impl Parser<'_> {
    fn chunk(&mut self) -> Result<DataChunk, DecodeError> {
        self.reader.field(100, "DataChunk.rows", true)?;
        let rows = usize::try_from(self.reader.uint(32)?)
            .map_err(|_| self.reader.error("row count overflow"))?;
        self.reader.field(101, "DataChunk.types", true)?;
        let count = self.reader.count(5)?;
        let mut types = Vec::new();
        types
            .try_reserve_exact(count)
            .map_err(|_| self.reader.error("cannot allocate type descriptors"))?;
        for _ in 0..count {
            let logical_type = self.reader.logical_type(0)?;
            if logical_type.id != LogicalTypeId::Integer {
                return Err(self.reader.unsupported(format!(
                    "unsupported chunk logical type {:?}",
                    logical_type.id
                )));
            }
            types.push(logical_type);
        }
        self.reader.field(102, "DataChunk.columns", true)?;
        let columns_count = self.reader.count(2)?;
        if columns_count != count {
            return Err(self.reader.error("column count differs from type count"));
        }
        let mut columns = Vec::new();
        columns
            .try_reserve_exact(count)
            .map_err(|_| self.reader.error("cannot allocate columns"))?;
        for _ in 0..count {
            columns.push(self.vector(rows, 0)?);
            self.reader.end()?;
        }
        self.reader.context = "DataChunk";
        self.reader.end()?;
        Ok(DataChunk {
            rows,
            types,
            columns,
        })
    }
    fn blob(&mut self, expected: usize) -> Result<Bytes, DecodeError> {
        let count = usize::try_from(self.reader.uint(64)?)
            .map_err(|_| self.reader.error("blob length overflow"))?;
        if count != expected {
            return Err(self.reader.error("blob length differs from expected size"));
        }
        let start = self.reader.pos;
        self.reader.take(count)?;
        Ok(self.owner.slice(start..self.reader.pos))
    }
    fn signed(&mut self) -> Result<i64, DecodeError> {
        let mut value = 0i128;
        for index in 0..10 {
            let byte = self.reader.take(1)?[0];
            let shift = index * 7;
            value |= i128::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                if byte & 0x40 != 0 {
                    value |= !0i128 << (shift + 7);
                }
                return i64::try_from(value)
                    .map_err(|_| self.reader.error("signed LEB128 overflow"));
            }
        }
        Err(self.reader.error("signed LEB128 exceeds encoded width"))
    }
    fn vector(&mut self, len: usize, depth: usize) -> Result<IntegerVector, DecodeError> {
        if depth >= 64 {
            return Err(self.reader.error("vector nesting exceeds 64"));
        }
        let tag = if self.reader.field(90, "IntegerVector.encoding", false)? {
            self.reader.uint(8)?
        } else {
            0
        };
        let storage = match tag {
            0 => {
                self.reader.field(100, "IntegerVector.has_validity", true)?;
                let validity = if self.reader.boolean()? {
                    self.reader.field(101, "IntegerVector.validity", true)?;
                    let size = len
                        .div_ceil(64)
                        .checked_mul(8)
                        .ok_or_else(|| self.reader.error("validity size overflow"))?;
                    Some(self.blob(size)?)
                } else {
                    None
                };
                self.reader.field(102, "IntegerVector.values", true)?;
                let size = len
                    .checked_mul(4)
                    .ok_or_else(|| self.reader.error("values size overflow"))?;
                let values = self.blob(size)?;
                Storage::Flat { values, validity }
            }
            2 => Storage::Constant(Box::new(self.vector(1, depth + 1)?)),
            3 => {
                self.reader.field(91, "IntegerVector.selection", true)?;
                let size = len
                    .checked_mul(4)
                    .ok_or_else(|| self.reader.error("selection size overflow"))?;
                let selection = self.blob(size)?;
                self.reader
                    .field(92, "IntegerVector.dictionary_count", true)?;
                let count = usize::try_from(self.reader.uint(64)?)
                    .map_err(|_| self.reader.error("dictionary count overflow"))?;
                for selected in selection.as_chunks::<4>().0 {
                    let selected =
                        u32::from_le_bytes([selected[0], selected[1], selected[2], selected[3]]);
                    if u64::from(selected) >= count as u64 {
                        return Err(self.reader.error("dictionary index out of bounds"));
                    }
                }
                let dictionary = Box::new(self.vector(count, depth + 1)?);
                Storage::Dictionary {
                    selection,
                    dictionary,
                }
            }
            4 => {
                self.reader
                    .field(91, "IntegerVector.sequence_start", true)?;
                let start = self.signed()?;
                self.reader
                    .field(92, "IntegerVector.sequence_increment", true)?;
                let increment = self.signed()?;
                if len > 0 {
                    let end = i128::from(start) + i128::from(increment) * (len - 1) as i128;
                    if i32::try_from(start).is_err() || i32::try_from(end).is_err() {
                        return Err(self.reader.error("sequence exceeds INTEGER range"));
                    }
                }
                Storage::Sequence { start, increment }
            }
            _ => {
                return Err(self
                    .reader
                    .unsupported(format!("unsupported vector encoding {tag}")));
            }
        };
        Ok(IntegerVector { len, storage })
    }
}

#[cfg(test)]
#[path = "chunk_tests.rs"]
mod tests;
