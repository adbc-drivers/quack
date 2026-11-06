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

//! Bounded prefix decoding of DuckDB logical type objects.

use crate::logical_type::{LogicalType, LogicalTypeId, TypeInfo, TypeInfoKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeErrorKind {
    Malformed,
    Unsupported,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DecodeError {
    pub kind: DecodeErrorKind,
    pub offset: usize,
    pub context: &'static str,
    pub message: String,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} at byte {}: {}",
            self.context, self.offset, self.message
        )
    }
}
impl std::error::Error for DecodeError {}

/// Decode one object, leaving subsequent bytes to the enclosing message parser.
/// At most 64 nested LogicalType objects are accepted.
pub fn logical_type(bytes: &[u8]) -> Result<(LogicalType, usize), DecodeError> {
    let mut reader = Reader {
        bytes,
        pos: 0,
        context: "LogicalType",
    };
    let value = reader.logical_type(0)?;
    Ok((value, reader.pos))
}

pub(crate) struct Reader<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) pos: usize,
    pub(crate) context: &'static str,
}
impl Reader<'_> {
    pub(crate) fn error(&self, message: impl Into<String>) -> DecodeError {
        DecodeError {
            kind: DecodeErrorKind::Malformed,
            offset: self.pos,
            context: self.context,
            message: message.into(),
        }
    }
    pub(crate) fn unsupported(&self, message: impl Into<String>) -> DecodeError {
        DecodeError {
            kind: DecodeErrorKind::Unsupported,
            ..self.error(message)
        }
    }
    pub(crate) fn take(&mut self, len: usize) -> Result<&[u8], DecodeError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| self.error("length overflow"))?;
        if end > self.bytes.len() {
            return Err(self.error("truncated input"));
        }
        let start = self.pos;
        self.pos = end;
        Ok(&self.bytes[start..end])
    }
    fn peek(&self) -> Result<u16, DecodeError> {
        let pair = self
            .bytes
            .get(self.pos..)
            .and_then(|v| v.first_chunk::<2>())
            .ok_or_else(|| self.error("truncated field ID"))?;
        Ok(u16::from_le_bytes(*pair))
    }
    pub(crate) fn field(
        &mut self,
        id: u16,
        context: &'static str,
        required: bool,
    ) -> Result<bool, DecodeError> {
        self.context = context;
        let actual = self.peek()?;
        if actual == id {
            self.pos += 2;
            Ok(true)
        } else if required {
            Err(self.error(format!("expected field {id}, found {actual}")))
        } else {
            Ok(false)
        }
    }
    pub(crate) fn end(&mut self) -> Result<(), DecodeError> {
        self.field(u16::MAX, self.context, true)?;
        Ok(())
    }
    pub(crate) fn uint(&mut self, width: u32) -> Result<u64, DecodeError> {
        let mut value = 0u128;
        for index in 0..width.div_ceil(7) {
            let byte = self.take(1)?[0];
            value |= u128::from(byte & 0x7f) << (index * 7);
            if byte & 0x80 == 0 {
                if value >= 1u128 << width {
                    return Err(self.error("unsigned LEB128 overflow"));
                }
                return Ok(value as u64);
            }
        }
        Err(self.error("unsigned LEB128 exceeds encoded width"))
    }
    pub(crate) fn boolean(&mut self) -> Result<bool, DecodeError> {
        match self.take(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(self.error("invalid boolean")),
        }
    }
    fn string(&mut self) -> Result<Vec<u8>, DecodeError> {
        let len =
            usize::try_from(self.uint(32)?).map_err(|_| self.error("string length overflow"))?;
        Ok(self.take(len)?.to_vec())
    }
    pub(crate) fn count(&mut self, minimum: usize) -> Result<usize, DecodeError> {
        let count = usize::try_from(self.uint(64)?).map_err(|_| self.error("count overflow"))?;
        if count > (self.bytes.len() - self.pos) / minimum {
            return Err(self.error("count exceeds remaining input"));
        }
        Ok(count)
    }
    fn optional_uint(
        &mut self,
        id: u16,
        context: &'static str,
        width: u32,
    ) -> Result<u64, DecodeError> {
        if self.field(id, context, false)? {
            self.uint(width)
        } else {
            Ok(0)
        }
    }
    fn optional_string(&mut self, id: u16, context: &'static str) -> Result<Vec<u8>, DecodeError> {
        if self.field(id, context, false)? {
            self.string()
        } else {
            Ok(Vec::new())
        }
    }
    fn empty_values(&mut self, id: u16, context: &'static str) -> Result<(), DecodeError> {
        if self.field(id, context, false)? && self.uint(64)? != 0 {
            return Err(self.unsupported("unsupported Value-bearing metadata"));
        }
        Ok(())
    }
    pub(crate) fn logical_type(&mut self, depth: usize) -> Result<LogicalType, DecodeError> {
        self.context = "LogicalType";
        if depth >= 64 {
            return Err(self.error("type nesting exceeds 64"));
        }
        self.field(100, "LogicalType.id", true)?;
        let id = self.uint(8)? as u8;
        let id = LogicalTypeId::try_from(id)
            .map_err(|id| self.unsupported(format!("unsupported logical type ID {id}")))?;
        let info = if self.field(101, "LogicalType.type_info", false)? && self.boolean()? {
            Some(self.type_info(depth)?)
        } else {
            None
        };
        self.context = "LogicalType";
        self.end()?;
        Ok(LogicalType { id, info })
    }
    fn child(&mut self, depth: usize) -> Result<Box<LogicalType>, DecodeError> {
        self.field(200, "TypeInfo.child_type", true)?;
        Ok(Box::new(self.logical_type(depth + 1)?))
    }
    fn type_info(&mut self, depth: usize) -> Result<TypeInfo, DecodeError> {
        self.field(100, "TypeInfo.type", true)?;
        let tag = self.uint(8)?;
        let alias = self.optional_string(101, "TypeInfo.alias")?;
        self.empty_values(102, "TypeInfo.modifiers")?;
        let extension_info =
            if self.field(103, "TypeInfo.extension_info", false)? && self.boolean()? {
                self.empty_values(100, "ExtensionTypeInfo.modifiers")?;
                self.empty_values(101, "ExtensionTypeInfo.properties")?;
                self.end()?;
                true
            } else {
                false
            };
        let kind = match tag {
            1 => TypeInfoKind::Generic,
            2 => TypeInfoKind::Decimal {
                width: self.optional_uint(200, "Decimal.width", 8)? as u8,
                scale: self.optional_uint(201, "Decimal.scale", 8)? as u8,
            },
            3 => TypeInfoKind::String {
                collation: self.optional_string(200, "String.collation")?,
            },
            4 => TypeInfoKind::List {
                child: self.child(depth)?,
            },
            5 => {
                let mut children = Vec::new();
                if self.field(200, "Struct.child_types", false)? {
                    let count = self.count(12)?;
                    for _ in 0..count {
                        self.field(0, "Struct.child.name", true)?;
                        let name = self.string()?;
                        self.field(1, "Struct.child.type", true)?;
                        let child = self.logical_type(depth + 1)?;
                        self.end()?;
                        children.push((name, child));
                    }
                }
                TypeInfoKind::Struct { children }
            }
            6 => {
                self.field(200, "Enum.values_count", true)?;
                let expected = self.uint(64)?;
                if expected > u64::from(u32::MAX) {
                    return Err(self.error("enum dictionary exceeds uint32 size"));
                }
                self.field(201, "Enum.values", true)?;
                let count = self.count(1)?;
                if expected != count as u64 {
                    return Err(self.error("enum values_count does not match dictionary"));
                }
                let mut values = Vec::new();
                for _ in 0..count {
                    values.push(self.string()?);
                }
                TypeInfoKind::Enum { values }
            }
            9 => TypeInfoKind::Array {
                child: self.child(depth)?,
                size: self.optional_uint(201, "Array.size", 32)? as u32,
            },
            10 => TypeInfoKind::Any {
                target: self.child(depth)?,
                cast_score: self.optional_uint(201, "Any.cast_score", 64)?,
            },
            12 => TypeInfoKind::Template {
                name: self.optional_string(200, "Template.name")?,
            },
            13 => {
                self.field(200, "Geometry.crs", true)?;
                let crs = self.optional_string(100, "CoordinateReferenceSystem.definition")?;
                self.end()?;
                TypeInfoKind::Geometry { crs }
            }
            7 => return Err(self.unsupported("unsupported parsed-expression type metadata")),
            11 => {
                return Err(self.unsupported("unsupported Value-bearing INTEGER_LITERAL metadata"));
            }
            _ => return Err(self.unsupported(format!("unsupported type-info discriminator {tag}"))),
        };
        self.context = "TypeInfo";
        self.end()?;
        Ok(TypeInfo {
            alias,
            extension_info,
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(10, LogicalTypeId::Boolean)]
    #[case(13, LogicalTypeId::Integer)]
    #[case(25, LogicalTypeId::Varchar)]
    #[case(109, LogicalTypeId::Variant)]
    fn primitive_prefix(#[case] id: u8, #[case] expected: LogicalTypeId) {
        let bytes = [100, 0, id, 255, 255, 42];
        assert_eq!(
            logical_type(&bytes),
            Ok((
                LogicalType {
                    id: expected,
                    info: None
                },
                5
            ))
        );
    }

    #[rstest]
    #[case(&[])]
    #[case(&[100])]
    #[case(&[100, 0, 255])]
    #[case(&[100, 0, 13, 255])]
    #[case(&[100, 0, 127, 255, 255])]
    fn malformed(#[case] bytes: &[u8]) {
        assert!(logical_type(bytes).is_err());
    }
}
