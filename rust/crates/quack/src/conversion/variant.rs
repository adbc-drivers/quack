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

use super::{ArrayRef, ArrowError, DataType, Field, Vector, array};
use arrow_array::{
    Array, BinaryArray, ListArray, StringArray, StructArray, UInt8Array, UInt32Array,
    builder::BinaryBuilder,
};
use arrow_buffer::NullBuffer;

use std::sync::Arc;

pub(super) fn storage_type() -> DataType {
    DataType::Struct(
        vec![
            Field::new("metadata", DataType::Binary, false),
            Field::new("value", DataType::Binary, true),
        ]
        .into(),
    )
}

fn invalid() -> ArrowError {
    ArrowError::ParseError("invalid DuckDB VARIANT representation".into())
}

fn downcast<T: Array + 'static>(a: &ArrayRef) -> Result<&T, ArrowError> {
    a.as_any().downcast_ref().ok_or_else(invalid)
}

fn child<'a, T: Array + 'static>(a: &'a StructArray, name: &str) -> Result<&'a T, ArrowError> {
    downcast(a.column_by_name(name).ok_or_else(invalid)?)
}

struct Nodes<'a> {
    keys: &'a StringArray,
    key_ids: &'a UInt32Array,
    value_ids: &'a UInt32Array,
    tags: &'a UInt8Array,
    offsets: &'a UInt32Array,
    data: &'a [u8],
}

// Parquet VARIANT stores integers in its offset and field-id tables in 1–4 bytes.
fn width(value: u32) -> usize {
    (32 - value.leading_zeros()).div_ceil(8).max(1) as usize
}
fn append_number(out: &mut Vec<u8>, value: u32, width: usize) {
    out.extend_from_slice(&value.to_le_bytes()[..width]);
}

fn metadata(keys: &StringArray) -> Result<Vec<u8>, ArrowError> {
    let mut payload = Vec::new();
    let mut offsets = vec![0];
    for key in keys {
        payload.extend_from_slice(key.ok_or_else(invalid)?.as_bytes());
        offsets.push(u32::try_from(payload.len()).map_err(|_| invalid())?);
    }
    let size = width(u32::try_from(payload.len()).map_err(|_| invalid())?);
    let mut output = vec![1 | ((size as u8 - 1) << 6)];
    append_number(
        &mut output,
        u32::try_from(keys.len()).map_err(|_| invalid())?,
        size,
    );
    for offset in offsets {
        append_number(&mut output, offset, size)
    }
    output.extend(payload);
    Ok(output)
}

fn container(
    object: bool,
    count: u32,
    keys: Vec<u32>,
    offsets: Vec<u32>,
    payload: Vec<u8>,
) -> Result<Vec<u8>, ArrowError> {
    let key_width = width(keys.iter().copied().max().unwrap_or(0));
    let offset_width = width(u32::try_from(payload.len()).map_err(|_| invalid())?);
    let large = count > 255;
    let header = if object {
        2 | ((key_width as u8 - 1) << 4) | (u8::from(large) << 6)
    } else {
        3 | (u8::from(large) << 4)
    };
    let mut output = vec![header | ((offset_width as u8 - 1) << 2)];
    append_number(&mut output, count, if large { 4 } else { 1 });
    for key in keys {
        append_number(&mut output, key, key_width)
    }
    for offset in offsets {
        append_number(&mut output, offset, offset_width)
    }
    output.extend(payload);
    Ok(output)
}

mod shredded;
pub(super) use shredded::convert as shredded;

fn varint(data: &mut &[u8]) -> Result<u32, ArrowError> {
    let mut n = 0;
    for shift in (0..32).step_by(7) {
        let (v, rest) = data.split_first().ok_or_else(invalid)?;
        *data = rest;
        if shift == 28 && *v > 15 {
            return Err(invalid());
        }
        n |= u32::from(v & 127) << shift;
        if v & 128 == 0 {
            return Ok(n);
        }
    }
    Err(invalid())
}

fn scalar(tag: u8, mut data: &[u8]) -> Result<Vec<u8>, ArrowError> {
    let (header, width) = match tag {
        0..=2 => (tag * 4, 0),
        3..=6 => ((tag) * 4, 1 << (tag - 3)),
        7 | 11 | 12 => {
            let n = match tag {
                7 => i64::try_from(i128::from_le_bytes(
                    *data.first_chunk().ok_or_else(invalid)?,
                ))
                .ok(),
                11 => {
                    i64::try_from(u64::from_le_bytes(*data.first_chunk().ok_or_else(invalid)?)).ok()
                }
                _ => i64::try_from(u128::from_le_bytes(
                    *data.first_chunk().ok_or_else(invalid)?,
                ))
                .ok(),
            }
            .ok_or_else(|| ArrowError::CastError("VARIANT integer exceeds int64".into()))?;
            let mut value = vec![24];
            value.extend(n.to_le_bytes());
            return Ok(value);
        }
        8..=10 => {
            let width = 1 << (tag - 8);
            let mut value = vec![(tag - 4) * 4];
            value.extend(data.get(..width).ok_or_else(invalid)?);
            value.resize(1 + 2 * width, 0);
            return Ok(value);
        }
        13 => (56, 4),
        14 => (28, 8),
        15 => {
            let precision = varint(&mut data)?;
            let scale = u8::try_from(varint(&mut data)?).map_err(|_| invalid())?;
            let (header, width) = match precision {
                1..=4 => (32, 2),
                5..=9 => (32, 4),
                10..=18 => (36, 8),
                19..=38 => (40, 16),
                _ => return Err(invalid()),
            };
            let bytes = data.get(..width).ok_or_else(invalid)?;
            let mut value = vec![header, scale];
            value.extend(bytes);
            if width == 2 {
                value.resize(6, if bytes[1] & 128 == 0 { 0 } else { 255 })
            }
            return Ok(value);
        }
        16 | 17 => {
            let len = varint(&mut data)?;
            let payload = data.get(..len as usize).ok_or_else(invalid)?;
            let mut value = if tag == 16 && len < 64 {
                vec![((len as u8) << 2) | 1]
            } else {
                let mut v = vec![if tag == 16 { 64 } else { 60 }];
                v.extend(len.to_le_bytes());
                v
            };
            value.extend(payload);
            return Ok(value);
        }
        18 => {
            let uuid = u128::from_le_bytes(*data.first_chunk().ok_or_else(invalid)?) ^ (1 << 127);
            let mut v = vec![80];
            v.extend(uuid.to_be_bytes());
            return Ok(v);
        }
        19 => (44, 4),
        20 => (68, 8),
        24 => (52, 8),
        25 => (76, 8),
        27 => (48, 8),
        _ => {
            return Err(ArrowError::CastError(format!(
                "DuckDB VARIANT tag {tag} cannot be exported as Parquet VARIANT"
            )));
        }
    };
    let mut value = vec![header];
    value.extend(data.get(..width).ok_or_else(invalid)?);
    Ok(value)
}

pub(super) fn convert(
    children: &[Vector],
    nulls: Option<NullBuffer>,
    len: usize,
) -> Result<ArrayRef, ArrowError> {
    let arrays = children.iter().map(array).collect::<Result<Vec<_>, _>>()?;
    encode_rows(&arrays, len, |row, core| {
        if nulls.as_ref().is_some_and(|mask| mask.is_null(row)) {
            Ok(None)
        } else {
            shredded::encode_raw(core).map(Some)
        }
    })
}

type EncodedRow = (Vec<u8>, Vec<u8>);

// Both physical representations use the same four columns for untyped data.
// A shredded row can refer to any node in these columns by its one-based index.
fn encode_rows(
    arrays: &[ArrayRef],
    len: usize,
    encode: impl Fn(usize, &Nodes) -> Result<Option<EncodedRow>, ArrowError>,
) -> Result<ArrayRef, ArrowError> {
    let [keys, edges, nodes, blobs] = arrays else {
        return Err(invalid());
    };
    let keys = downcast::<ListArray>(keys)?;
    let edges = downcast::<ListArray>(edges)?;
    let nodes = downcast::<ListArray>(nodes)?;
    let blobs = downcast::<BinaryArray>(blobs)?;
    let mut metadata_builder = BinaryBuilder::new();
    let mut values = BinaryBuilder::new();
    let mut valid = Vec::with_capacity(len);
    for row in 0..len {
        let keys = keys.value(row);
        let edges = edges.value(row);
        let nodes = nodes.value(row);
        let edges = downcast::<StructArray>(&edges)?;
        let nodes = downcast::<StructArray>(&nodes)?;
        let tree = Nodes {
            keys: downcast(&keys)?,
            key_ids: child(edges, "keys_index")?,
            value_ids: child(edges, "values_index")?,
            tags: child(nodes, "type_id")?,
            offsets: child(nodes, "byte_offset")?,
            data: blobs.value(row),
        };
        if let Some((meta, value)) = encode(row, &tree)? {
            if meta
                .len()
                .saturating_add(metadata_builder.values_slice().len())
                > i32::MAX as usize
                || value.len().saturating_add(values.values_slice().len()) > i32::MAX as usize
            {
                return Err(ArrowError::ComputeError(
                    "VARIANT binary offsets exceed i32".into(),
                ));
            }
            metadata_builder.append_value(meta);
            values.append_value(value);
            valid.push(true);
        } else {
            metadata_builder.append_value([1, 0, 0]);
            values.append_null();
            valid.push(false);
        }
    }
    let DataType::Struct(fields) = storage_type() else {
        return Err(invalid());
    };
    Ok(Arc::new(StructArray::try_new(
        fields,
        vec![
            Arc::new(metadata_builder.finish()),
            Arc::new(values.finish()),
        ],
        Some(NullBuffer::from(valid)),
    )?))
}
