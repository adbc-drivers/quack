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

//! Resolve DuckDB's typed columns and one-based references into the untyped remainder.
use super::*;
use arrow_array::{BooleanArray, PrimitiveArray, types::*};
use duckdb_wiredata::logical_type::{LogicalType, LogicalTypeId as Id, TypeInfoKind};

#[derive(Clone, Copy)]
struct Typed<'a> {
    values: &'a ArrayRef,
    ty: &'a LogicalType,
    row: usize,
}

enum Value<'a> {
    Null,
    Raw(usize),
    Typed(Typed<'a>, Option<usize>),
}

impl<'a> Typed<'a> {
    fn children(self) -> Result<Vec<(&'a str, Self)>, ArrowError> {
        let a = downcast::<StructArray>(self.values)?;
        let Some(TypeInfoKind::Struct { children }) = self.ty.info.as_ref().map(|i| &i.kind) else {
            return Err(invalid());
        };
        if children.len() != a.num_columns() {
            return Err(invalid());
        }
        children
            .iter()
            .zip(a.columns())
            .map(|((name, ty), values)| {
                Ok((
                    std::str::from_utf8(name).map_err(|_| invalid())?,
                    Self {
                        values,
                        ty,
                        row: self.row,
                    },
                ))
            })
            .collect()
    }

    fn resolve(self) -> Result<Option<Value<'a>>, ArrowError> {
        if self.ty.id != Id::Struct {
            return Ok(Some(if self.values.is_null(self.row) {
                Value::Null
            } else {
                Value::Typed(self, None)
            }));
        }
        let fields = self.children()?;
        let Some(("typed_value", typed)) = fields.first().copied() else {
            return Err(invalid());
        };
        let overlay = match &fields[1..] {
            [] => None,
            [("untyped_value_index", index)] => {
                let a = downcast::<UInt32Array>(index.values)?;
                a.is_valid(self.row).then(|| a.value(self.row) as usize)
            }
            _ => return Err(invalid()),
        };
        if typed.values.is_valid(self.row) {
            return Ok(Some(Value::Typed(
                typed,
                overlay.and_then(|i| i.checked_sub(1)),
            )));
        }
        Ok(match overlay {
            None => Some(Value::Null),
            Some(0) => None,
            Some(n) => Some(Value::Raw(n - 1)),
        })
    }

    fn primitive(self) -> Result<Vec<u8>, ArrowError> {
        macro_rules! fixed {
            ($ty:ty,$tag:expr) => {
                scalar(
                    $tag,
                    &downcast::<PrimitiveArray<$ty>>(self.values)?
                        .value(self.row)
                        .to_le_bytes(),
                )
            };
        }
        match self.ty.id {
            Id::Boolean => Ok(vec![
                if downcast::<BooleanArray>(self.values)?.value(self.row) {
                    4
                } else {
                    8
                },
            ]),
            Id::TinyInt => fixed!(Int8Type, 3),
            Id::SmallInt => fixed!(Int16Type, 4),
            Id::Integer => fixed!(Int32Type, 5),
            Id::BigInt => fixed!(Int64Type, 6),
            Id::HugeInt => fixed!(Decimal128Type, 7),
            Id::UHugeInt => fixed!(Decimal128Type, 12),
            Id::UTinyInt => fixed!(UInt8Type, 8),
            Id::USmallInt => fixed!(UInt16Type, 9),
            Id::UInteger => fixed!(UInt32Type, 10),
            Id::UBigInt => fixed!(UInt64Type, 11),
            Id::Float => fixed!(Float32Type, 13),
            Id::Double => fixed!(Float64Type, 14),
            Id::Date => fixed!(Date32Type, 19),
            Id::Time => fixed!(Time64MicrosecondType, 20),
            Id::Timestamp => fixed!(TimestampMicrosecondType, 24),
            Id::TimestampNs => fixed!(TimestampNanosecondType, 25),
            Id::TimestampTz => fixed!(TimestampMicrosecondType, 27),
            Id::Varchar | Id::Blob => {
                let text = self.ty.id == Id::Varchar;
                let bytes = if text {
                    downcast::<StringArray>(self.values)?
                        .value(self.row)
                        .as_bytes()
                } else {
                    downcast::<BinaryArray>(self.values)?.value(self.row)
                };
                let len = u32::try_from(bytes.len()).map_err(|_| invalid())?;
                let mut value = if text && len < 64 {
                    vec![((len as u8) << 2) | 1]
                } else {
                    let mut v = vec![if text { 64 } else { 60 }];
                    v.extend(len.to_le_bytes());
                    v
                };
                value.extend(bytes);
                Ok(value)
            }
            Id::Decimal => {
                let Some(TypeInfoKind::Decimal { width, scale }) =
                    self.ty.info.as_ref().map(|i| &i.kind)
                else {
                    return Err(invalid());
                };
                let mut bytes = vec![*width, *scale];
                bytes.extend(
                    downcast::<PrimitiveArray<Decimal128Type>>(self.values)?
                        .value(self.row)
                        .to_le_bytes(),
                );
                scalar(15, &bytes)
            }
            Id::Uuid => {
                let id =
                    uuid::Uuid::parse_str(downcast::<StringArray>(self.values)?.value(self.row))
                        .map_err(|_| invalid())?;
                let mut bytes = vec![80];
                bytes.extend(id.as_bytes());
                Ok(bytes)
            }
            id => Err(ArrowError::CastError(format!(
                "cannot export shredded VARIANT of type {id:?}"
            ))),
        }
    }
}

#[derive(Clone, Copy)]
struct Key<'a> {
    name: &'a str,
    index: Option<u32>,
}
type Children<'a> = Vec<(Option<Key<'a>>, Value<'a>)>;

enum Contents<'a> {
    Scalar(Vec<u8>),
    Children(bool, Children<'a>),
}

fn raw_children<'a>(core: &Nodes<'a>, index: usize) -> Result<Children<'a>, ArrowError> {
    if index >= core.tags.len() {
        return Err(invalid());
    }
    let tag = core.tags.value(index);
    if tag != 29 && tag != 30 {
        return Err(invalid());
    }
    let mut data = core
        .data
        .get(core.offsets.value(index) as usize..)
        .ok_or_else(invalid)?;
    let count = varint(&mut data)? as usize;
    let start = if count == 0 {
        0
    } else {
        varint(&mut data)? as usize
    };
    let end = start
        .checked_add(count)
        .filter(|end| *end <= core.value_ids.len())
        .ok_or_else(invalid)?;
    (start..end)
        .map(|i| {
            let key = if tag == 29 {
                let k = core.key_ids.value(i) as usize;
                if k >= core.keys.len() {
                    return Err(invalid());
                }
                Some(Key {
                    name: core.keys.value(k),
                    index: Some(core.key_ids.value(i)),
                })
            } else {
                None
            };
            Ok((key, Value::Raw(core.value_ids.value(i) as usize)))
        })
        .collect()
}

fn contents<'a>(
    value: Value<'a>,
    core: &Nodes<'a>,
    visited: &mut [bool],
) -> Result<Contents<'a>, ArrowError> {
    match value {
        Value::Null => Ok(Contents::Scalar(vec![0])),
        Value::Raw(i) => {
            if std::mem::replace(visited.get_mut(i).ok_or_else(invalid)?, true) {
                return Err(invalid());
            }
            let tag = core.tags.value(i);
            if tag == 29 || tag == 30 {
                return Ok(Contents::Children(tag == 29, raw_children(core, i)?));
            }
            Ok(Contents::Scalar(scalar(
                tag,
                core.data
                    .get(core.offsets.value(i) as usize..)
                    .ok_or_else(invalid)?,
            )?))
        }
        Value::Typed(node, overlay) => match node.ty.id {
            Id::Struct => {
                let mut children = Vec::new();
                for (name, child) in node.children()? {
                    if let Some(value) = child.resolve()? {
                        children.push((Some(Key { name, index: None }), value))
                    }
                }
                if let Some(i) = overlay {
                    if i >= core.tags.len() || core.tags.value(i) != 29 {
                        return Err(invalid());
                    }
                    children.extend(raw_children(core, i)?);
                }
                Ok(Contents::Children(true, children))
            }
            Id::List => {
                let list = downcast::<ListArray>(node.values)?;
                let Some(TypeInfoKind::List { child }) = node.ty.info.as_ref().map(|i| &i.kind)
                else {
                    return Err(invalid());
                };
                let rows = list.value_offsets();
                let children = (rows[node.row]..rows[node.row + 1])
                    .map(|row| {
                        let value = Typed {
                            values: list.values(),
                            ty: child,
                            row: row as usize,
                        }
                        .resolve()?
                        .unwrap_or(Value::Null);
                        Ok((None, value))
                    })
                    .collect::<Result<Vec<_>, ArrowError>>()?;
                Ok(Contents::Children(false, children))
            }
            _ => Ok(Contents::Scalar(node.primitive()?)),
        },
    }
}

fn encode(
    root: Value<'_>,
    core: &Nodes<'_>,
    unshred: bool,
) -> Result<(Vec<u8>, Vec<u8>), ArrowError> {
    enum Work<'a> {
        Visit(Option<Key<'a>>, Value<'a>),
        Finish(bool, usize),
    }
    let mut work = vec![Work::Visit(None, root)];
    let mut values = Vec::<(Option<u32>, Vec<u8>)>::new();
    let mut parent_keys = Vec::new();
    let mut keys = Vec::new();
    let mut visited = vec![false; core.tags.len()];
    while let Some(item) = work.pop() {
        match item {
            Work::Visit(key, node) => {
                let id = if let Some(key) = key {
                    if unshred {
                        let id = u32::try_from(keys.len()).map_err(|_| invalid())?;
                        keys.push(key.name);
                        Some(id)
                    } else {
                        Some(key.index.ok_or_else(invalid)?)
                    }
                } else {
                    None
                };
                match contents(node, core, &mut visited)? {
                    Contents::Scalar(bytes) => values.push((id, bytes)),
                    Contents::Children(object, mut children) => {
                        if object && unshred {
                            children.sort_by_key(|(key, _)| key.map(|k| k.name));
                            if children
                                .windows(2)
                                .any(|p| p[0].0.map(|k| k.name) == p[1].0.map(|k| k.name))
                            {
                                return Err(invalid());
                            }
                        }
                        parent_keys.push(id);
                        work.push(Work::Finish(object, values.len()));
                        work.extend(children.into_iter().rev().map(|(k, v)| Work::Visit(k, v)));
                    }
                }
            }
            Work::Finish(object, start) => {
                let count = u32::try_from(values.len() - start).map_err(|_| invalid())?;
                let mut payload = Vec::new();
                let mut offsets = vec![0];
                let mut ids = Vec::new();
                for (id, value) in values.drain(start..) {
                    if let Some(id) = id {
                        ids.push(id)
                    }
                    payload.extend(value);
                    offsets.push(u32::try_from(payload.len()).map_err(|_| invalid())?);
                }
                values.push((
                    parent_keys.pop().ok_or_else(invalid)?,
                    container(object, count, ids, offsets, payload)?,
                ));
            }
        }
    }
    Ok((
        if unshred {
            metadata(&StringArray::from(keys))?
        } else {
            metadata(core.keys)?
        },
        values.pop().ok_or_else(invalid)?.1,
    ))
}

pub(super) fn encode_raw(core: &Nodes) -> Result<(Vec<u8>, Vec<u8>), ArrowError> {
    encode(Value::Raw(0), core, false)
}

pub(in crate::conversion) fn convert(physical: &Vector) -> Result<ArrayRef, ArrowError> {
    let data = array(physical)?;
    let data = downcast::<StructArray>(&data)?;
    let Some(TypeInfoKind::Struct { children }) =
        physical.descriptor().info.as_ref().map(|i| &i.kind)
    else {
        return Err(invalid());
    };
    let [(_, _), (_, ty)] = children.as_slice() else {
        return Err(invalid());
    };
    let untyped = child::<StructArray>(data, "unshredded")?;
    let typed = data.column_by_name("shredded").ok_or_else(invalid)?;
    encode_rows(untyped.columns(), data.len(), |row, core| {
        if typed.is_null(row) {
            return Ok(None);
        }
        match (Typed {
            ty,
            values: typed,
            row,
        })
        .resolve()?
        {
            None | Some(Value::Null) => Ok(None),
            Some(root) => encode(root, core, true).map(Some),
        }
    })
}
