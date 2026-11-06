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

//! Validated, shared views of little-endian DuckDB scalar chunks.

use crate::{
    decode::{DecodeError, Reader},
    logical_type::{LogicalType, LogicalTypeId, TypeInfoKind},
};
use bytes::Bytes;

#[derive(Debug, PartialEq, Eq)]
pub struct DataChunk {
    rows: usize,
    types: Vec<LogicalType>,
    columns: Vec<Vector>,
}

impl DataChunk {
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn types(&self) -> &[LogicalType] {
        &self.types
    }
    pub fn columns(&self) -> &[Vector] {
        &self.columns
    }
    pub fn into_columns(self) -> Vec<Vector> {
        self.columns
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Vector {
    len: usize,
    logical_type: LogicalTypeId,
    descriptor: LogicalType,
    storage: Storage,
}

#[derive(Debug, PartialEq, Eq)]
enum Storage {
    Variable {
        values: Vec<Bytes>,
        validity: Option<Bytes>,
    },
    Flat {
        values: Bytes,
        validity: Option<Bytes>,
    },
    Constant(Box<Vector>),
    Dictionary {
        selection: Bytes,
        dictionary: Box<Vector>,
    },
    Sequence {
        start: i64,
        increment: i64,
    },
}

/// Borrowed encoding information; all byte buffers are validated shared views.
#[derive(Debug)]
pub enum VectorEncoding<'a> {
    Variable {
        values: &'a [Bytes],
        validity: Option<&'a Bytes>,
    },
    Flat {
        values: &'a Bytes,
        validity: Option<&'a Bytes>,
    },
    Constant {
        value: &'a Vector,
    },
    Dictionary {
        selection: &'a Bytes,
        dictionary: &'a Vector,
    },
    Sequence {
        start: i64,
        increment: i64,
    },
}

impl Vector {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn encoding(&self) -> VectorEncoding<'_> {
        match &self.storage {
            Storage::Variable { values, validity } => VectorEncoding::Variable {
                values,
                validity: validity.as_ref(),
            },
            Storage::Flat { values, validity } => VectorEncoding::Flat {
                values,
                validity: validity.as_ref(),
            },
            Storage::Constant(value) => VectorEncoding::Constant { value },
            Storage::Dictionary {
                selection,
                dictionary,
            } => VectorEncoding::Dictionary {
                selection,
                dictionary,
            },
            Storage::Sequence { start, increment } => VectorEncoding::Sequence {
                start: *start,
                increment: *increment,
            },
        }
    }
    pub fn descriptor(&self) -> &LogicalType {
        &self.descriptor
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
        self.get_physical(index)
    }

    /// Read a compatible native storage value without logical-type coercion.
    pub fn get_physical<T: Primitive>(&self, index: usize) -> Option<Option<T>> {
        let compatible = match self.logical_type {
            LogicalTypeId::Date => T::LOGICAL_TYPE == LogicalTypeId::Integer,
            LogicalTypeId::TimeNs
            | LogicalTypeId::TimestampTzNs
            | LogicalTypeId::Time
            | LogicalTypeId::TimestampSec
            | LogicalTypeId::TimestampMs
            | LogicalTypeId::TimestampNs
            | LogicalTypeId::Timestamp
            | LogicalTypeId::TimestampTz => T::LOGICAL_TYPE == LogicalTypeId::BigInt,
            LogicalTypeId::Decimal => match decimal_width(&self.descriptor) {
                Some(2) => T::LOGICAL_TYPE == LogicalTypeId::SmallInt,
                Some(4) => T::LOGICAL_TYPE == LogicalTypeId::Integer,
                Some(8) => T::LOGICAL_TYPE == LogicalTypeId::BigInt,
                Some(16) => T::LOGICAL_TYPE == LogicalTypeId::HugeInt,
                _ => false,
            },
            id => id == T::LOGICAL_TYPE,
        };
        if index >= self.len || !compatible {
            return None;
        }
        match &self.storage {
            Storage::Variable { .. } => None,
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
            Storage::Constant(value) => value.get_physical(0),
            Storage::Dictionary {
                selection,
                dictionary,
            } => {
                let offset = index.checked_mul(4)?;
                let selected = u32::from_le_bytes(*selection.get(offset..)?.first_chunk::<4>()?);
                dictionary.get_physical(usize::try_from(selected).ok()?)
            }
            Storage::Sequence { start, increment } => {
                let value = i128::from(*start) + i128::from(*increment) * index as i128;
                Some(Some(T::from_sequence(value)?))
            }
        }
    }
    /// Nullable bytes for VARCHAR or BLOB; outer None indicates an invalid index or type.
    pub fn get_bytes(&self, index: usize) -> Option<Option<&[u8]>> {
        if index >= self.len {
            return None;
        }
        match &self.storage {
            Storage::Variable { values, validity } => {
                if validity
                    .as_ref()
                    .is_some_and(|mask| mask[index / 8] & (1 << (index % 8)) == 0)
                {
                    Some(None)
                } else {
                    Some(Some(values.get(index)?.as_ref()))
                }
            }
            Storage::Constant(value) => value.get_bytes(0),
            Storage::Dictionary {
                selection,
                dictionary,
            } => {
                let offset = index.checked_mul(4)?;
                let selected = u32::from_le_bytes(*selection.get(offset..)?.first_chunk::<4>()?);
                dictionary.get_bytes(usize::try_from(selected).ok()?)
            }
            _ => None,
        }
    }

    /// Read a decimal's unscaled coefficient, sign-extended to 128 bits.
    pub fn get_decimal(&self, index: usize) -> Option<Option<i128>> {
        match decimal_width(&self.descriptor)? {
            2 => self.get_physical::<i16>(index).map(|v| v.map(i128::from)),
            4 => self.get_physical::<i32>(index).map(|v| v.map(i128::from)),
            8 => self.get_physical::<i64>(index).map(|v| v.map(i128::from)),
            16 => self.get_physical::<i128>(index),
            _ => None,
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
primitive!(i128, HugeInt, 16, Some);
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

fn decimal_width(logical_type: &LogicalType) -> Option<usize> {
    if logical_type.id != LogicalTypeId::Decimal {
        return None;
    }
    match &logical_type.info.as_ref()?.kind {
        TypeInfoKind::Decimal { width, scale } if *width > 0 && *width <= 38 && scale <= width => {
            Some(match width {
                1..=4 => 2,
                5..=9 => 4,
                10..=18 => 8,
                _ => 16,
            })
        }
        _ => None,
    }
}

fn physical_width(id: LogicalTypeId) -> Option<usize> {
    match id {
        LogicalTypeId::Date => Some(4),
        LogicalTypeId::TimeNs
        | LogicalTypeId::TimestampTzNs
        | LogicalTypeId::Time
        | LogicalTypeId::TimestampSec
        | LogicalTypeId::TimestampMs
        | LogicalTypeId::TimestampNs
        | LogicalTypeId::Timestamp
        | LogicalTypeId::TimestampTz => Some(8),
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
            if logical_type.id == LogicalTypeId::Decimal && decimal_width(&logical_type).is_none() {
                return Err(self.reader.error("invalid decimal precision or scale"));
            }
            if physical_width(logical_type.id).is_none()
                && decimal_width(&logical_type).is_none()
                && logical_type.id != LogicalTypeId::Varchar
                && logical_type.id != LogicalTypeId::Blob
            {
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
            columns.push(self.vector(rows, 0, logical_type)?);
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
    fn strings(&mut self, len: usize, validity: Option<&Bytes>) -> Result<Vec<Bytes>, DecodeError> {
        let mut values = Vec::new();
        if self.reader.field(102, "Vector.values", false)? {
            let count = self.reader.count(1)?;
            if count != len {
                return Err(self.reader.error("string count differs from row count"));
            }
            values
                .try_reserve_exact(len)
                .map_err(|_| self.reader.error("cannot allocate string views"))?;
            for _ in 0..len {
                let size = usize::try_from(self.reader.uint(32)?)
                    .map_err(|_| self.reader.error("string length overflow"))?;
                let start = self.reader.pos;
                self.reader.take(size)?;
                values.push(self.owner.slice(start..self.reader.pos));
            }
        } else {
            self.reader.field(107, "Vector.byte_data_length", true)?;
            let size = usize::try_from(self.reader.uint(64)?)
                .map_err(|_| self.reader.error("string data length overflow"))?;
            self.reader.field(108, "Vector.length_data", true)?;
            let lengths_size = len
                .checked_mul(4)
                .ok_or_else(|| self.reader.error("string lengths size overflow"))?;
            let lengths = self.blob(lengths_size)?;
            self.reader.field(109, "Vector.byte_data", true)?;
            let data = self.blob(size)?;
            values
                .try_reserve_exact(len)
                .map_err(|_| self.reader.error("cannot allocate string views"))?;
            let mut offset = 0usize;
            for (index, bytes) in lengths.as_chunks::<4>().0.iter().enumerate() {
                let length = usize::try_from(u32::from_le_bytes(*bytes))
                    .map_err(|_| self.reader.error("string length overflow"))?;
                let valid = validity.is_none_or(|mask| mask[index / 8] & (1 << (index % 8)) != 0);
                let end = offset
                    .checked_add(if valid { length } else { 0 })
                    .ok_or_else(|| self.reader.error("string offset overflow"))?;
                if end > data.len() {
                    return Err(self.reader.error("string lengths exceed payload"));
                }
                values.push(data.slice(offset..end));
                offset = end;
            }
            if offset != data.len() {
                return Err(self.reader.error("string lengths differ from payload"));
            }
        }
        Ok(values)
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
        descriptor: &LogicalType,
    ) -> Result<Vector, DecodeError> {
        let logical_type = descriptor.id;
        if depth >= 64 {
            return Err(self.reader.error("vector nesting exceeds 64"));
        }
        let tag = if self.reader.field(90, "Vector.encoding", false)? {
            self.reader.uint(8)?
        } else {
            0
        };
        let storage = match tag {
            0 => {
                self.reader.field(100, "Vector.has_validity", true)?;
                let validity = if self.reader.boolean()? {
                    self.reader.field(101, "Vector.validity", true)?;
                    let size = len
                        .div_ceil(64)
                        .checked_mul(8)
                        .ok_or_else(|| self.reader.error("validity size overflow"))?;
                    Some(self.blob(size)?)
                } else {
                    None
                };
                if logical_type == LogicalTypeId::Varchar || logical_type == LogicalTypeId::Blob {
                    let values = self.strings(len, validity.as_ref())?;
                    return Ok(Vector {
                        len,
                        logical_type,
                        descriptor: descriptor.clone(),
                        storage: Storage::Variable { values, validity },
                    });
                }
                self.reader.field(102, "Vector.values", true)?;
                let size = len
                    .checked_mul(
                        physical_width(logical_type)
                            .or_else(|| decimal_width(descriptor))
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
            2 => Storage::Constant(Box::new(self.vector(1, depth + 1, descriptor)?)),
            3 => {
                self.reader.field(91, "Vector.selection", true)?;
                let size = len
                    .checked_mul(4)
                    .ok_or_else(|| self.reader.error("selection size overflow"))?;
                let selection = self.blob(size)?;
                self.reader.field(92, "Vector.dictionary_count", true)?;
                let count = usize::try_from(self.reader.uint(64)?)
                    .map_err(|_| self.reader.error("dictionary count overflow"))?;
                for selected in selection.as_chunks::<4>().0 {
                    let selected =
                        u32::from_le_bytes([selected[0], selected[1], selected[2], selected[3]]);
                    if u64::from(selected) >= count as u64 {
                        return Err(self.reader.error("dictionary index out of bounds"));
                    }
                }
                let dictionary = Box::new(self.vector(count, depth + 1, descriptor)?);
                Storage::Dictionary {
                    selection,
                    dictionary,
                }
            }
            4 => {
                self.reader.field(91, "Vector.sequence_start", true)?;
                let start = self.signed()?;
                self.reader.field(92, "Vector.sequence_increment", true)?;
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
        Ok(Vector {
            len,
            logical_type,
            descriptor: descriptor.clone(),
            storage,
        })
    }
}

#[cfg(test)]
#[path = "chunk_tests.rs"]
mod tests;
