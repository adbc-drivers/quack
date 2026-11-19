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

//! Conversion of validated DuckDB views into Arrow-owned or shared buffers.

use std::{mem::align_of, sync::Arc};

use arrow_array::{
    ArrayRef, BooleanArray, PrimitiveArray, RecordBatch, RecordBatchOptions, types::*,
};
use arrow_buffer::{BooleanBuffer, Buffer, NullBuffer, ScalarBuffer};
use arrow_schema::{ArrowError, DataType, Field, Schema, SchemaRef};
use duckdb_wiredata::{
    chunk::{DataChunk, Primitive, Vector, VectorEncoding},
    logical_type::{LogicalType, LogicalTypeId, TypeInfoKind},
};
use quack_protocol::message::WireString;

mod variant;

fn result_field(name: &str, ty: &LogicalType) -> Result<Field, ArrowError> {
    let field = Field::new(name, data_type(ty)?, true);
    Ok(if ty.id == LogicalTypeId::Variant {
        field.with_metadata(
            arrow_schema::Metadata::default()
                .with("ARROW:extension:name", "arrow.parquet.variant")
                .with("ARROW:extension:metadata", ""),
        )
    } else {
        field
    })
}

pub(crate) fn schema(types: &[LogicalType], names: &[WireString]) -> Result<SchemaRef, ArrowError> {
    if types.len() != names.len() {
        return Err(ArrowError::SchemaError(
            "result type and name counts differ".into(),
        ));
    }
    let mut fields = Vec::new();
    fields
        .try_reserve_exact(types.len())
        .map_err(|error| ArrowError::ExternalError(Box::new(error)))?;
    for (logical_type, name) in types.iter().zip(names) {
        let name = name
            .as_str()
            .map_err(|error| ArrowError::ExternalError(Box::new(error)))?;
        fields.push(result_field(name, logical_type)?);
    }
    Ok(Arc::new(Schema::new(fields)))
}

pub(crate) fn record_batch(chunk: DataChunk, schema: SchemaRef) -> Result<RecordBatch, ArrowError> {
    if schema.fields().len() != chunk.columns().len() {
        return Err(ArrowError::SchemaError(
            "chunk column count differs from Arrow schema".into(),
        ));
    }
    for (logical_type, field) in chunk.types().iter().zip(schema.fields()) {
        if &result_field(field.name(), logical_type)? != field.as_ref() {
            return Err(ArrowError::SchemaError(
                "chunk type differs from Arrow schema".into(),
            ));
        }
    }
    let rows = chunk.rows();
    let mut arrays: Vec<ArrayRef> = Vec::new();
    arrays
        .try_reserve_exact(chunk.columns().len())
        .map_err(|error| ArrowError::ExternalError(Box::new(error)))?;
    for column in chunk.into_columns() {
        arrays.push(array(&column)?);
    }
    RecordBatch::try_new_with_options(
        schema,
        arrays,
        &RecordBatchOptions::new().with_row_count(Some(rows)),
    )
}

fn numeric_array<T: ArrowPrimitiveType>(vector: &Vector) -> Result<PrimitiveArray<T>, ArrowError>
where
    T::Native: Primitive,
{
    if let VectorEncoding::Flat { values, validity } = vector.encoding() {
        let expected = vector
            .len()
            .checked_mul(size_of::<T::Native>())
            .ok_or_else(|| ArrowError::ComputeError("primitive values size overflow".into()))?;
        if values.len() != expected {
            return Err(ArrowError::ComputeError(
                "primitive values size differs from row count".into(),
            ));
        }
        let values =
            if cfg!(target_endian = "little")
                && values.as_ptr().align_offset(align_of::<T::Native>()) == 0
            {
                // Length and native alignment are checked before ScalarBuffer's constructor.
                ScalarBuffer::from(Buffer::from(values.clone()))
            } else {
                let mut decoded = Vec::new();
                decoded
                    .try_reserve_exact(vector.len())
                    .map_err(|error| ArrowError::ExternalError(Box::new(error)))?;
                for value in values.chunks_exact(size_of::<T::Native>()) {
                    decoded.push(T::Native::read(value).ok_or_else(|| {
                        ArrowError::ComputeError("invalid primitive value".into())
                    })?);
                }
                ScalarBuffer::from(decoded)
            };
        let nulls = validity
            .map(|mask| {
                let expected = vector.len().div_ceil(64).checked_mul(8).ok_or_else(|| {
                    ArrowError::ComputeError("primitive validity size overflow".into())
                })?;
                if mask.len() != expected {
                    return Err(ArrowError::ComputeError(
                        "primitive validity size differs from row count".into(),
                    ));
                }
                Ok(NullBuffer::new(BooleanBuffer::new(
                    Buffer::from(mask.clone()),
                    0,
                    vector.len(),
                )))
            })
            .transpose()?;
        return PrimitiveArray::<T>::try_new(values, nulls);
    }

    // Compressed values expand directly into their final Arrow allocation.
    // Create validity only on the first null, without an intermediate bool array.
    let mut values = Vec::new();
    values
        .try_reserve_exact(vector.len())
        .map_err(|error| ArrowError::ExternalError(Box::new(error)))?;
    let mut validity: Option<Vec<u8>> = None;
    for index in 0..vector.len() {
        let value = vector.get_physical::<T::Native>(index).ok_or_else(|| {
            ArrowError::ComputeError("validated primitive vector has no value at row".into())
        })?;
        match value {
            Some(value) => values.push(value),
            None => {
                values.push(T::Native::default());
                if validity.is_none() {
                    let mut mask = Vec::new();
                    let len = vector.len().div_ceil(8);
                    mask.try_reserve_exact(len)
                        .map_err(|error| ArrowError::ExternalError(Box::new(error)))?;
                    mask.resize(len, u8::MAX);
                    validity = Some(mask);
                }
                if let Some(mask) = &mut validity {
                    mask[index / 8] &= !(1 << (index % 8));
                }
            }
        }
    }
    let nulls = validity
        .map(|mask| NullBuffer::new(BooleanBuffer::new(Buffer::from_vec(mask), 0, vector.len())));
    PrimitiveArray::<T>::try_new(values.into(), nulls)
}

fn data_type(logical_type: &LogicalType) -> Result<DataType, ArrowError> {
    let id = logical_type.id;
    match id {
        LogicalTypeId::Struct
        | LogicalTypeId::Union
        | LogicalTypeId::List
        | LogicalTypeId::Array
        | LogicalTypeId::Map
        | LogicalTypeId::Enum => nested_type(logical_type),
        LogicalTypeId::Variant => Ok(variant::storage_type()),
        LogicalTypeId::SqlNull => Ok(DataType::Null),
        LogicalTypeId::HugeInt | LogicalTypeId::UHugeInt => Ok(DataType::Decimal128(38, 0)),
        LogicalTypeId::Interval => Ok(DataType::Interval(arrow_schema::IntervalUnit::MonthDayNano)),
        LogicalTypeId::Uuid | LogicalTypeId::Char | LogicalTypeId::Varchar => Ok(DataType::Utf8),
        LogicalTypeId::Bit | LogicalTypeId::Blob => Ok(DataType::Binary),
        LogicalTypeId::Decimal => match logical_type.info.as_ref().map(|info| &info.kind) {
            Some(TypeInfoKind::Decimal { width, scale })
                if *width > 0 && *width <= 38 && scale <= width =>
            {
                Ok(DataType::Decimal128(*width, *scale as i8))
            }
            _ => Err(ArrowError::SchemaError(
                "invalid decimal precision or scale".into(),
            )),
        },
        LogicalTypeId::Date => Ok(DataType::Date32),
        LogicalTypeId::TimeNs => Ok(DataType::Time64(arrow_schema::TimeUnit::Nanosecond)),
        LogicalTypeId::TimestampTzNs => Ok(DataType::Timestamp(
            arrow_schema::TimeUnit::Nanosecond,
            Some("UTC".into()),
        )),
        LogicalTypeId::TimeTz | LogicalTypeId::Time => {
            Ok(DataType::Time64(arrow_schema::TimeUnit::Microsecond))
        }
        LogicalTypeId::TimestampSec => {
            Ok(DataType::Timestamp(arrow_schema::TimeUnit::Second, None))
        }
        LogicalTypeId::TimestampMs => Ok(DataType::Timestamp(
            arrow_schema::TimeUnit::Millisecond,
            None,
        )),
        LogicalTypeId::TimestampNs => Ok(DataType::Timestamp(
            arrow_schema::TimeUnit::Nanosecond,
            None,
        )),
        LogicalTypeId::Timestamp => Ok(DataType::Timestamp(
            arrow_schema::TimeUnit::Microsecond,
            None,
        )),
        LogicalTypeId::TimestampTz => Ok(DataType::Timestamp(
            arrow_schema::TimeUnit::Microsecond,
            Some("UTC".into()),
        )),
        LogicalTypeId::Boolean => Ok(DataType::Boolean),
        LogicalTypeId::TinyInt => Ok(DataType::Int8),
        LogicalTypeId::SmallInt => Ok(DataType::Int16),
        LogicalTypeId::Integer => Ok(DataType::Int32),
        LogicalTypeId::BigInt => Ok(DataType::Int64),
        LogicalTypeId::UTinyInt => Ok(DataType::UInt8),
        LogicalTypeId::USmallInt => Ok(DataType::UInt16),
        LogicalTypeId::UInteger => Ok(DataType::UInt32),
        LogicalTypeId::UBigInt => Ok(DataType::UInt64),
        LogicalTypeId::Float => Ok(DataType::Float32),
        LogicalTypeId::Double => Ok(DataType::Float64),
        _ => Err(ArrowError::NotYetImplemented(format!(
            "unsupported result type {id:?}"
        ))),
    }
}

fn invalid_type() -> ArrowError {
    ArrowError::SchemaError("invalid nested type descriptor".into())
}

fn nested_type(ty: &LogicalType) -> Result<DataType, ArrowError> {
    match (ty.id, &ty.info.as_ref().ok_or_else(invalid_type)?.kind) {
        (LogicalTypeId::Enum, TypeInfoKind::Enum { values }) => Ok(DataType::Dictionary(
            Box::new(match values.len() {
                0..=255 => DataType::UInt8,
                256..=65535 => DataType::UInt16,
                _ => DataType::UInt32,
            }),
            Box::new(DataType::Utf8),
        )),
        (LogicalTypeId::List, TypeInfoKind::List { child }) => {
            Ok(DataType::List(Arc::new(result_field("l", child)?)))
        }
        (LogicalTypeId::Array, TypeInfoKind::Array { child, size }) => Ok(DataType::FixedSizeList(
            Arc::new(result_field("", child)?),
            i32::try_from(*size).map_err(|_| invalid_type())?,
        )),
        (LogicalTypeId::Map, TypeInfoKind::List { child }) => {
            let DataType::Struct(fields) = data_type(child)? else {
                return Err(invalid_type());
            };
            if fields.len() != 2 {
                return Err(invalid_type());
            }
            let entry = DataType::Struct(
                vec![
                    fields[0]
                        .as_ref()
                        .clone()
                        .with_name("key")
                        .with_nullable(false),
                    fields[1].as_ref().clone().with_name("value"),
                ]
                .into(),
            );
            Ok(DataType::Map(
                Arc::new(Field::new("entries", entry, false)),
                false,
            ))
        }
        (LogicalTypeId::Struct | LogicalTypeId::Union, TypeInfoKind::Struct { children }) => {
            let union = ty.id == LogicalTypeId::Union;
            let fields = children
                .iter()
                .skip(usize::from(union))
                .map(|(name, child)| {
                    result_field(
                        std::str::from_utf8(name).map_err(|_| invalid_type())?,
                        child,
                    )
                })
                .collect::<Result<Vec<_>, ArrowError>>()?;
            if union {
                if fields.is_empty() || fields.len() > 128 {
                    return Err(invalid_type());
                }
                Ok(DataType::Union(
                    arrow_schema::UnionFields::try_new((0..fields.len()).map(|i| i as i8), fields)?,
                    arrow_schema::UnionMode::Sparse,
                ))
            } else {
                Ok(DataType::Struct(fields.into()))
            }
        }
        _ => Err(invalid_type()),
    }
}

fn nested_array(vector: &Vector) -> Result<ArrayRef, ArrowError> {
    use arrow_array::{Array, StructArray, UInt32Array, UInt64Array};
    use arrow_select::take::take;
    let (children, entries, validity) = match vector.encoding() {
        VectorEncoding::Shredded(physical) => return variant::shredded(physical),
        VectorEncoding::Constant { value } => {
            return take(
                array(value)?.as_ref(),
                &UInt32Array::from(vec![0; vector.len()]),
                None,
            );
        }
        VectorEncoding::Dictionary {
            selection,
            dictionary,
        } => {
            let indices = UInt32Array::from_iter_values(
                selection
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .copied()
                    .map(u32::from_le_bytes),
            );
            return take(array(dictionary)?.as_ref(), &indices, None);
        }
        VectorEncoding::Nested {
            children,
            entries,
            validity,
        } => (children, entries, validity),
        _ => return Err(invalid_type()),
    };
    let nulls = validity
        .map(|v| NullBuffer::new(BooleanBuffer::new(Buffer::from(v.clone()), 0, vector.len())));
    if vector.logical_type() == LogicalTypeId::Variant {
        return variant::convert(children, nulls, vector.len());
    }
    let ty = nested_type(vector.descriptor())?;
    let mut arrays = children.iter().map(array).collect::<Result<Vec<_>, _>>()?;
    match ty {
        DataType::Struct(fields) => Ok(Arc::new(StructArray::try_new_with_length(
            fields,
            arrays,
            nulls,
            vector.len(),
        )?)),
        DataType::FixedSizeList(field, size) => {
            Ok(Arc::new(arrow_array::FixedSizeListArray::try_new(
                field,
                size,
                arrays.pop().ok_or_else(invalid_type)?,
                nulls,
            )?))
        }
        DataType::Union(fields, _) => {
            let tags = arrays
                .first()
                .and_then(|v| v.as_any().downcast_ref::<arrow_array::UInt8Array>())
                .ok_or_else(invalid_type)?;
            let tags = tags
                .iter()
                .map(|v| i8::try_from(v.unwrap_or(0)).map_err(|_| invalid_type()))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Arc::new(arrow_array::UnionArray::try_new(
                fields,
                tags.into(),
                None,
                arrays.split_off(1),
            )?))
        }
        DataType::Map(_, _) | DataType::List(_) => {
            let child = arrays.pop().ok_or_else(invalid_type)?;
            let mut offsets = vec![0i32];
            let mut total = 0usize;
            let mut contiguous = true;
            for &(start, len) in entries {
                contiguous &= len == 0 || start == total;
                total = total.checked_add(len).ok_or_else(invalid_type)?;
                offsets.push(
                    i32::try_from(total)
                        .map_err(|_| ArrowError::ComputeError("list offsets exceed i32".into()))?,
                );
            }
            let child = if contiguous {
                child.slice(0, total)
            } else {
                let indices = UInt64Array::from_iter_values(
                    entries
                        .iter()
                        .flat_map(|&(start, len)| (start..start + len).map(|i| i as u64)),
                );
                take(child.as_ref(), &indices, None)?
            };
            let offsets = arrow_buffer::OffsetBuffer::new(offsets.into());
            match ty {
                DataType::List(field) => Ok(Arc::new(arrow_array::ListArray::try_new(
                    field, offsets, child, nulls,
                )?)),
                DataType::Map(field, sorted) => {
                    let entries = child
                        .as_any()
                        .downcast_ref::<StructArray>()
                        .ok_or_else(invalid_type)?;
                    let DataType::Struct(fields) = field.data_type() else {
                        return Err(invalid_type());
                    };
                    let entries = StructArray::try_new(
                        fields.clone(),
                        entries.columns().to_vec(),
                        entries.nulls().cloned(),
                    )?;
                    Ok(Arc::new(arrow_array::MapArray::try_new(
                        field, offsets, entries, nulls, sorted,
                    )?))
                }
                _ => Err(invalid_type()),
            }
        }
        _ => Err(invalid_type()),
    }
}

fn array(vector: &Vector) -> Result<ArrayRef, ArrowError> {
    match vector.logical_type() {
        LogicalTypeId::List
        | LogicalTypeId::Array
        | LogicalTypeId::Struct
        | LogicalTypeId::Map
        | LogicalTypeId::Variant
        | LogicalTypeId::Union => nested_array(vector),
        LogicalTypeId::Enum => {
            let Some(TypeInfoKind::Enum { values }) =
                vector.descriptor().info.as_ref().map(|i| &i.kind)
            else {
                return Err(invalid_type());
            };
            let labels = values
                .iter()
                .map(|v| std::str::from_utf8(v).map_err(|_| invalid_type()))
                .collect::<Result<Vec<_>, _>>()?;
            let dictionary = Arc::new(arrow_array::StringArray::from(labels));
            match values.len() {
                0..=255 => Ok(Arc::new(arrow_array::DictionaryArray::try_new(
                    numeric_array::<UInt8Type>(vector)?,
                    dictionary,
                )?)),
                256..=65535 => Ok(Arc::new(arrow_array::DictionaryArray::try_new(
                    numeric_array::<UInt16Type>(vector)?,
                    dictionary,
                )?)),
                _ => Ok(Arc::new(arrow_array::DictionaryArray::try_new(
                    numeric_array::<UInt32Type>(vector)?,
                    dictionary,
                )?)),
            }
        }

        LogicalTypeId::SqlNull => Ok(Arc::new(arrow_array::NullArray::new(vector.len()))),
        LogicalTypeId::HugeInt | LogicalTypeId::UHugeInt => Ok(Arc::new(
            numeric_array::<Decimal128Type>(vector)?.with_precision_and_scale(38, 0)?,
        )),
        LogicalTypeId::Interval => {
            let raw = numeric_array::<Decimal128Type>(vector)?;
            Ok(Arc::new(arrow_array::IntervalMonthDayNanoArray::from_iter(
                raw.iter().map(|row| {
                    row.map(|bits| {
                        IntervalMonthDayNanoType::make_value(
                            bits as i32,
                            (bits >> 32) as i32,
                            ((bits >> 64) as i64).wrapping_mul(1000),
                        )
                    })
                }),
            )))
        }
        LogicalTypeId::TimeTz => {
            let raw = numeric_array::<UInt64Type>(vector)?;
            Ok(Arc::new(arrow_array::Time64MicrosecondArray::from_iter(
                raw.iter().map(|row| row.map(|v| (v >> 24) as i64)),
            )))
        }
        LogicalTypeId::Uuid => {
            let raw = numeric_array::<Decimal128Type>(vector)?;
            Ok(Arc::new(arrow_array::StringArray::from_iter(
                raw.iter().map(|row| {
                    row.map(|bits| uuid::Uuid::from_u128((bits as u128) ^ (1 << 127)).to_string())
                }),
            )))
        }
        LogicalTypeId::Char | LogicalTypeId::Varchar | LogicalTypeId::Bit | LogicalTypeId::Blob => {
            let mut builder = arrow_array::builder::BinaryBuilder::new();
            let mut size = 0usize;
            for index in 0..vector.len() {
                let value = vector
                    .get_bytes(index)
                    .ok_or_else(|| ArrowError::ComputeError("invalid variable-width row".into()))?;
                if let Some(value) = value {
                    size = size
                        .checked_add(value.len())
                        .filter(|size| *size <= i32::MAX as usize)
                        .ok_or_else(|| {
                            ArrowError::ComputeError("binary offsets exceed i32".into())
                        })?;
                    if matches!(
                        vector.logical_type(),
                        LogicalTypeId::Varchar | LogicalTypeId::Char
                    ) {
                        std::str::from_utf8(value)
                            .map_err(|error| ArrowError::ParseError(error.to_string()))?;
                    }
                }
                builder.append_option(value);
            }
            let binary = builder.finish();
            if matches!(
                vector.logical_type(),
                LogicalTypeId::Varchar | LogicalTypeId::Char
            ) {
                Ok(Arc::new(arrow_array::StringArray::from(binary)))
            } else {
                Ok(Arc::new(binary))
            }
        }
        LogicalTypeId::Decimal => {
            let DataType::Decimal128(precision, scale) = data_type(vector.descriptor())? else {
                return Err(ArrowError::SchemaError("invalid decimal type".into()));
            };
            let mut builder = arrow_array::Decimal128Array::builder(vector.len());
            for index in 0..vector.len() {
                builder.append_option(
                    vector
                        .get_decimal(index)
                        .ok_or_else(|| ArrowError::ComputeError("invalid decimal row".into()))?,
                );
            }
            let array = builder
                .finish()
                .with_precision_and_scale(precision, scale)?;
            array.validate_decimal_precision(precision)?;
            Ok(Arc::new(array))
        }
        LogicalTypeId::Date => Ok(Arc::new(numeric_array::<Date32Type>(vector)?)),
        LogicalTypeId::TimeNs => Ok(Arc::new(numeric_array::<Time64NanosecondType>(vector)?)),
        LogicalTypeId::TimestampTzNs => Ok(Arc::new(
            numeric_array::<TimestampNanosecondType>(vector)?.with_timezone("UTC"),
        )),
        LogicalTypeId::Time => Ok(Arc::new(numeric_array::<Time64MicrosecondType>(vector)?)),
        LogicalTypeId::TimestampSec => Ok(Arc::new(numeric_array::<TimestampSecondType>(vector)?)),
        LogicalTypeId::TimestampMs => {
            Ok(Arc::new(numeric_array::<TimestampMillisecondType>(vector)?))
        }
        LogicalTypeId::TimestampNs => {
            Ok(Arc::new(numeric_array::<TimestampNanosecondType>(vector)?))
        }
        LogicalTypeId::Timestamp => {
            Ok(Arc::new(numeric_array::<TimestampMicrosecondType>(vector)?))
        }
        LogicalTypeId::TimestampTz => Ok(Arc::new(
            numeric_array::<TimestampMicrosecondType>(vector)?.with_timezone("UTC"),
        )),

        LogicalTypeId::Boolean => {
            let mut builder = BooleanArray::builder(vector.len());
            for index in 0..vector.len() {
                builder.append_option(
                    vector
                        .get::<bool>(index)
                        .ok_or_else(|| ArrowError::ComputeError("invalid BOOLEAN row".into()))?,
                );
            }
            Ok(Arc::new(builder.finish()))
        }
        LogicalTypeId::TinyInt => Ok(Arc::new(numeric_array::<Int8Type>(vector)?)),
        LogicalTypeId::SmallInt => Ok(Arc::new(numeric_array::<Int16Type>(vector)?)),
        LogicalTypeId::Integer => Ok(Arc::new(numeric_array::<Int32Type>(vector)?)),
        LogicalTypeId::BigInt => Ok(Arc::new(numeric_array::<Int64Type>(vector)?)),
        LogicalTypeId::UTinyInt => Ok(Arc::new(numeric_array::<UInt8Type>(vector)?)),
        LogicalTypeId::USmallInt => Ok(Arc::new(numeric_array::<UInt16Type>(vector)?)),
        LogicalTypeId::UInteger => Ok(Arc::new(numeric_array::<UInt32Type>(vector)?)),
        LogicalTypeId::UBigInt => Ok(Arc::new(numeric_array::<UInt64Type>(vector)?)),
        LogicalTypeId::Float => Ok(Arc::new(numeric_array::<Float32Type>(vector)?)),
        LogicalTypeId::Double => Ok(Arc::new(numeric_array::<Float64Type>(vector)?)),
        id => Err(ArrowError::NotYetImplemented(format!(
            "unsupported result type {id:?}"
        ))),
    }
}

#[cfg(test)]
#[path = "conversion_tests.rs"]
mod tests;
