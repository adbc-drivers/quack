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

//! Validated, shared views of little-endian DuckDB primitive chunks.

use crate::{
    decode::{DecodeError, Reader},
    logical_type::{LogicalType, LogicalTypeId},
};
use bytes::Bytes;

#[derive(Debug, PartialEq, Eq)]
pub struct DataChunk {
    rows: usize,
    types: Vec<LogicalType>,
    columns: Vec<PrimitiveVector>,
}

impl DataChunk {
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn types(&self) -> &[LogicalType] {
        &self.types
    }
    pub fn columns(&self) -> &[PrimitiveVector] {
        &self.columns
    }
    pub fn into_columns(self) -> Vec<PrimitiveVector> {
        self.columns
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PrimitiveVector {
    len: usize,
    logical_type: LogicalTypeId,
    storage: Storage,
}

#[derive(Debug, PartialEq, Eq)]
enum Storage {
    Flat {
        values: Bytes,
        validity: Option<Bytes>,
    },
    Constant(Box<PrimitiveVector>),
    Dictionary {
        selection: Bytes,
        dictionary: Box<PrimitiveVector>,
    },
    Sequence {
        start: i64,
        increment: i64,
    },
}

/// Borrowed encoding information; all byte buffers are validated shared views.
#[derive(Debug)]
pub enum PrimitiveEncoding<'a> {
    Flat {
        values: &'a Bytes,
        validity: Option<&'a Bytes>,
    },
    Constant {
        value: &'a PrimitiveVector,
    },
    Dictionary {
        selection: &'a Bytes,
        dictionary: &'a PrimitiveVector,
    },
    Sequence {
        start: i64,
        increment: i64,
    },
}

impl PrimitiveVector {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn encoding(&self) -> PrimitiveEncoding<'_> {
        match &self.storage {
            Storage::Flat { values, validity } => PrimitiveEncoding::Flat {
                values,
                validity: validity.as_ref(),
            },
            Storage::Constant(value) => PrimitiveEncoding::Constant { value },
            Storage::Dictionary {
                selection,
                dictionary,
            } => PrimitiveEncoding::Dictionary {
                selection,
                dictionary,
            },
            Storage::Sequence { start, increment } => PrimitiveEncoding::Sequence {
                start: *start,
                increment: *increment,
            },
        }
    }
    pub fn logical_type(&self) -> LogicalTypeId {
        self.logical_type
    }

    /// `None` means out of bounds or a mismatched requested type;
    /// `Some(None)` is a SQL NULL. No numeric coercion is performed.
    pub fn get<T: Primitive>(&self, index: usize) -> Option<Option<T>> {
        if index >= self.len || self.logical_type != T::LOGICAL_TYPE {
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
                let offset = index.checked_mul(T::WIDTH)?;
                Some(Some(T::read(values.get(offset..offset + T::WIDTH)?)?))
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
                Some(Some(T::from_sequence(value)?))
            }
        }
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Native values supported by a primitive wire vector. Implementations are sealed.
pub trait Primitive: sealed::Sealed + Copy {
    const LOGICAL_TYPE: LogicalTypeId;
    const WIDTH: usize;
    fn read(bytes: &[u8]) -> Option<Self>;
    fn from_sequence(value: i128) -> Option<Self>;
}

macro_rules! primitive {
    ($native:ty, $id:ident, $width:literal, $sequence:expr) => {
        impl sealed::Sealed for $native {}
        impl Primitive for $native {
            const LOGICAL_TYPE: LogicalTypeId = LogicalTypeId::$id;
            const WIDTH: usize = $width;
            fn read(bytes: &[u8]) -> Option<Self> {
                Some(Self::from_le_bytes(bytes.try_into().ok()?))
            }
            fn from_sequence(value: i128) -> Option<Self> {
                ($sequence)(value)
            }
        }
    };
}
primitive!(i8, TinyInt, 1, |value| i8::try_from(value).ok());
primitive!(i16, SmallInt, 2, |value| i16::try_from(value).ok());
primitive!(i32, Integer, 4, |value| i32::try_from(value).ok());
primitive!(i64, BigInt, 8, |value| i64::try_from(value).ok());
primitive!(u8, UTinyInt, 1, |value| u8::try_from(value).ok());
primitive!(u16, USmallInt, 2, |value| u16::try_from(value).ok());
primitive!(u32, UInteger, 4, |value| u32::try_from(value).ok());
primitive!(u64, UBigInt, 8, |value| u64::try_from(value).ok());
primitive!(f32, Float, 4, |_| None);
primitive!(f64, Double, 8, |_| None);

impl sealed::Sealed for bool {}
impl Primitive for bool {
    const LOGICAL_TYPE: LogicalTypeId = LogicalTypeId::Boolean;
    const WIDTH: usize = 1;
    fn read(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [0] => Some(false),
            [1] => Some(true),
            _ => None,
        }
    }
    fn from_sequence(_: i128) -> Option<Self> {
        None
    }
}

fn physical_width(id: LogicalTypeId) -> Option<usize> {
    match id {
        LogicalTypeId::Boolean => Some(1),
        LogicalTypeId::TinyInt => Some(1),
        LogicalTypeId::SmallInt => Some(2),
        LogicalTypeId::Integer => Some(4),
        LogicalTypeId::BigInt => Some(8),
        LogicalTypeId::UTinyInt => Some(1),
        LogicalTypeId::USmallInt => Some(2),
        LogicalTypeId::UInteger => Some(4),
        LogicalTypeId::UBigInt => Some(8),
        LogicalTypeId::Float => Some(4),
        LogicalTypeId::Double => Some(8),
        _ => None,
    }
}

fn sequence_range(id: LogicalTypeId) -> Option<(i128, i128)> {
    match id {
        LogicalTypeId::TinyInt => Some((i128::from(i8::MIN), i128::from(i8::MAX))),
        LogicalTypeId::SmallInt => Some((i128::from(i16::MIN), i128::from(i16::MAX))),
        LogicalTypeId::Integer => Some((i128::from(i32::MIN), i128::from(i32::MAX))),
        LogicalTypeId::BigInt => Some((i128::from(i64::MIN), i128::from(i64::MAX))),
        LogicalTypeId::UTinyInt => Some((i128::from(u8::MIN), i128::from(u8::MAX))),
        LogicalTypeId::USmallInt => Some((i128::from(u16::MIN), i128::from(u16::MAX))),
        LogicalTypeId::UInteger => Some((i128::from(u32::MIN), i128::from(u32::MAX))),
        LogicalTypeId::UBigInt => Some((i128::from(u64::MIN), i128::from(u64::MAX))),
        _ => None,
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
            if physical_width(logical_type.id).is_none() {
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
        for logical_type in &types {
            columns.push(self.vector(rows, 0, logical_type.id)?);
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
    fn vector(
        &mut self,
        len: usize,
        depth: usize,
        logical_type: LogicalTypeId,
    ) -> Result<PrimitiveVector, DecodeError> {
        if depth >= 64 {
            return Err(self.reader.error("vector nesting exceeds 64"));
        }
        let tag = if self.reader.field(90, "PrimitiveVector.encoding", false)? {
            self.reader.uint(8)?
        } else {
            0
        };
        let storage = match tag {
            0 => {
                self.reader
                    .field(100, "PrimitiveVector.has_validity", true)?;
                let validity = if self.reader.boolean()? {
                    self.reader.field(101, "PrimitiveVector.validity", true)?;
                    let size = len
                        .div_ceil(64)
                        .checked_mul(8)
                        .ok_or_else(|| self.reader.error("validity size overflow"))?;
                    Some(self.blob(size)?)
                } else {
                    None
                };
                self.reader.field(102, "PrimitiveVector.values", true)?;
                let size = len
                    .checked_mul(
                        physical_width(logical_type)
                            .ok_or_else(|| self.reader.unsupported("unsupported primitive type"))?,
                    )
                    .ok_or_else(|| self.reader.error("values size overflow"))?;
                let values = self.blob(size)?;
                if logical_type == LogicalTypeId::Boolean {
                    for (index, value) in values.iter().enumerate() {
                        let valid = validity
                            .as_ref()
                            .is_none_or(|mask| mask[index / 8] & (1 << (index % 8)) != 0);
                        if valid && *value > 1 {
                            return Err(self.reader.error("invalid BOOLEAN value"));
                        }
                    }
                }
                Storage::Flat { values, validity }
            }
            2 => Storage::Constant(Box::new(self.vector(1, depth + 1, logical_type)?)),
            3 => {
                self.reader.field(91, "PrimitiveVector.selection", true)?;
                let size = len
                    .checked_mul(4)
                    .ok_or_else(|| self.reader.error("selection size overflow"))?;
                let selection = self.blob(size)?;
                self.reader
                    .field(92, "PrimitiveVector.dictionary_count", true)?;
                let count = usize::try_from(self.reader.uint(64)?)
                    .map_err(|_| self.reader.error("dictionary count overflow"))?;
                for selected in selection.as_chunks::<4>().0 {
                    let selected =
                        u32::from_le_bytes([selected[0], selected[1], selected[2], selected[3]]);
                    if u64::from(selected) >= count as u64 {
                        return Err(self.reader.error("dictionary index out of bounds"));
                    }
                }
                let dictionary = Box::new(self.vector(count, depth + 1, logical_type)?);
                Storage::Dictionary {
                    selection,
                    dictionary,
                }
            }
            4 => {
                self.reader
                    .field(91, "PrimitiveVector.sequence_start", true)?;
                let start = self.signed()?;
                self.reader
                    .field(92, "PrimitiveVector.sequence_increment", true)?;
                let increment = self.signed()?;
                let (min, max) = sequence_range(logical_type)
                    .ok_or_else(|| self.reader.unsupported("sequence requires an integer type"))?;
                if len > 0 {
                    let end = i128::from(start) + i128::from(increment) * (len - 1) as i128;
                    if !(min..=max).contains(&i128::from(start)) || !(min..=max).contains(&end) {
                        return Err(self.reader.error("sequence exceeds integer range"));
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
        Ok(PrimitiveVector {
            len,
            logical_type,
            storage,
        })
    }
}

#[cfg(test)]
#[path = "chunk_tests.rs"]
mod tests;
