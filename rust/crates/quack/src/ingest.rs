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

//! Arrow upload conversion. Compatible fixed-width values retain Arrow ownership.
use arrow_array::{Array, ArrayRef, RecordBatch};
use arrow_schema::{ArrowError, DataType, TimeUnit};
use bytes::Bytes;
use duckdb_wiredata::{
    encode::Encoder,
    logical_type::{LogicalType, LogicalTypeId as Id, TypeInfo, TypeInfoKind},
};

pub(crate) fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub(crate) fn descriptor(dt: &DataType) -> Result<(LogicalType, String, DataType), ArrowError> {
    let (id, sql, normalized) = match dt {
        DataType::Boolean => (Id::Boolean, "BOOLEAN", dt.clone()),
        DataType::Int8 => (Id::TinyInt, "TINYINT", dt.clone()),
        DataType::Int16 => (Id::SmallInt, "SMALLINT", dt.clone()),
        DataType::Int32 => (Id::Integer, "INTEGER", dt.clone()),
        DataType::Int64 => (Id::BigInt, "BIGINT", dt.clone()),
        DataType::UInt8 => (Id::UTinyInt, "UTINYINT", dt.clone()),
        DataType::UInt16 => (Id::USmallInt, "USMALLINT", dt.clone()),
        DataType::UInt32 => (Id::UInteger, "UINTEGER", dt.clone()),
        DataType::UInt64 => (Id::UBigInt, "UBIGINT", dt.clone()),
        DataType::Float32 => (Id::Float, "FLOAT", dt.clone()),
        DataType::Float64 => (Id::Double, "DOUBLE", dt.clone()),
        DataType::Date32 | DataType::Date64 => (Id::Date, "DATE", DataType::Date32),
        DataType::Time64(TimeUnit::Nanosecond) => (Id::TimeNs, "TIME_NS", dt.clone()),
        DataType::Time32(_) | DataType::Time64(_) => {
            (Id::Time, "TIME", DataType::Time64(TimeUnit::Microsecond))
        }
        DataType::Timestamp(unit, None) => {
            let (id, name) = match unit {
                TimeUnit::Second => (Id::TimestampSec, "TIMESTAMP_S"),
                TimeUnit::Millisecond => (Id::TimestampMs, "TIMESTAMP_MS"),
                TimeUnit::Microsecond => (Id::Timestamp, "TIMESTAMP"),
                TimeUnit::Nanosecond => (Id::TimestampNs, "TIMESTAMP_NS"),
            };
            (id, name, dt.clone())
        }
        DataType::Timestamp(TimeUnit::Nanosecond, Some(_)) => (
            Id::TimestampTzNs,
            "TIMESTAMPTZ_NS",
            DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
        ),
        DataType::Timestamp(_, Some(_)) => (
            Id::TimestampTz,
            "TIMESTAMPTZ",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        ),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
            (Id::Varchar, "VARCHAR", dt.clone())
        }
        DataType::Binary
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::FixedSizeBinary(_) => (Id::Blob, "BLOB", dt.clone()),
        DataType::Dictionary(_, value) => return descriptor(value),
        DataType::Decimal128(precision, scale) => {
            let width = i16::from(*precision) + i16::from(*scale).min(0).abs();
            let scale = (*scale).max(0) as u8;
            if !(1..=38).contains(&width) || i16::from(scale) > width {
                return Err(ArrowError::NotYetImplemented(
                    "decimal outside DuckDB precision range".into(),
                ));
            }
            let width = width as u8;
            return Ok((
                LogicalType {
                    id: Id::Decimal,
                    info: Some(TypeInfo {
                        alias: vec![],
                        extension_info: false,
                        kind: TypeInfoKind::Decimal { width, scale },
                    }),
                },
                format!("DECIMAL({width},{scale})"),
                DataType::Decimal128(width, scale as i8),
            ));
        }
        _ => return Err(ArrowError::NotYetImplemented(format!("ingestion of {dt}"))),
    };
    Ok((LogicalType { id, info: None }, sql.into(), normalized))
}

fn shared(buffer: arrow_buffer::Buffer) -> Bytes {
    Bytes::from_owner(buffer)
}

pub(crate) fn encode(batch: &RecordBatch) -> Result<Vec<Bytes>, ArrowError> {
    let mut encoder = Encoder::default();
    encoder.field(100);
    encoder.uint(batch.num_rows() as u64);
    encoder.field(101);
    encoder.uint(batch.num_columns() as u64);
    let types = batch
        .schema()
        .fields()
        .iter()
        .map(|f| descriptor(f.data_type()))
        .collect::<Result<Vec<_>, _>>()?;
    for (dt, _, _) in &types {
        encoder
            .logical_type(dt)
            .map_err(|e| ArrowError::NotYetImplemented(e.into()))?;
    }
    encoder.field(102);
    encoder.uint(batch.num_columns() as u64);
    for (array, (_, _, normalized)) in batch.columns().iter().zip(&types) {
        encode_vector(&mut encoder, array, normalized)?;
    }
    encoder.end();
    Ok(encoder.finish())
}

fn encode_vector(
    encoder: &mut Encoder,
    array: &ArrayRef,
    normalized: &DataType,
) -> Result<(), ArrowError> {
    if let DataType::Dictionary(key_type, _) = array.data_type() {
        // Dictionary validity belongs to keys in Arrow but to values on the wire.
        // Preserve dictionaries without null keys; nullable keys use the cast path below.
        if array.null_count() == 0 {
            macro_rules! dictionary {
                ($t:ty) => {{
                    let dictionary = array
                        .as_any()
                        .downcast_ref::<arrow_array::DictionaryArray<$t>>()
                        .ok_or_else(|| {
                            ArrowError::InvalidArgumentError("invalid dictionary array".into())
                        })?;
                    let keys = arrow_cast::cast(dictionary.keys(), &DataType::UInt32)?;
                    let data = keys.to_data();
                    let selection = shared(
                        data.buffers()[0].slice_with_length(data.offset() * 4, keys.len() * 4),
                    );
                    encoder.field(90);
                    encoder.uint(3);
                    encoder.field(91);
                    encoder.blob(selection);
                    encoder.field(92);
                    encoder.uint(dictionary.values().len() as u64);
                    return encode_vector(encoder, dictionary.values(), normalized);
                }};
            }
            use arrow_array::types::*;
            match key_type.as_ref() {
                DataType::Int8 => dictionary!(Int8Type),
                DataType::Int16 => dictionary!(Int16Type),
                DataType::Int32 => dictionary!(Int32Type),
                DataType::Int64 => dictionary!(Int64Type),
                DataType::UInt8 => dictionary!(UInt8Type),
                DataType::UInt16 => dictionary!(UInt16Type),
                DataType::UInt32 => dictionary!(UInt32Type),
                DataType::UInt64 => dictionary!(UInt64Type),
                _ => {
                    return Err(ArrowError::NotYetImplemented(
                        "unsupported dictionary key".into(),
                    ));
                }
            }
        }
    }
    let array: ArrayRef = if array.data_type() == normalized {
        array.clone()
    } else {
        arrow_cast::cast_with_options(
            array,
            normalized,
            &arrow_cast::CastOptions {
                safe: false,
                ..Default::default()
            },
        )?
    };
    encoder.field(100);
    encoder.boolean(array.null_count() != 0);
    if let Some(nulls) = array.nulls().filter(|n| n.null_count() != 0) {
        let size = array.len().div_ceil(64) * 8;
        let bytes = if nulls.offset() % 8 == 0 && nulls.buffer().len() >= nulls.offset() / 8 + size
        {
            shared(nulls.buffer().slice_with_length(nulls.offset() / 8, size))
        } else {
            let mut mask = vec![255u8; size];
            for i in 0..array.len() {
                if array.is_null(i) {
                    mask[i / 8] &= !(1 << (i % 8));
                }
            }
            Bytes::from(mask)
        };
        encoder.field(101);
        encoder.blob(bytes);
    }
    encoder.field(102);
    match array.data_type() {
        DataType::Boolean => {
            let a = array
                .as_any()
                .downcast_ref::<arrow_array::BooleanArray>()
                .ok_or_else(|| ArrowError::InvalidArgumentError("invalid Boolean array".into()))?;
            encoder.blob(Bytes::from(
                (0..a.len())
                    .map(|i| u8::from(a.value(i)))
                    .collect::<Vec<_>>(),
            ));
        }
        DataType::Utf8
        | DataType::LargeUtf8
        | DataType::Utf8View
        | DataType::Binary
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::FixedSizeBinary(_) => {
            encoder.uint(array.len() as u64);
            let data = array.to_data();
            for i in 0..array.len() {
                let value: &[u8] = match array.data_type() {
                    DataType::Utf8 => array
                        .as_any()
                        .downcast_ref::<arrow_array::StringArray>()
                        .map(|a| a.value(i).as_bytes()),
                    DataType::LargeUtf8 => array
                        .as_any()
                        .downcast_ref::<arrow_array::LargeStringArray>()
                        .map(|a| a.value(i).as_bytes()),
                    DataType::Utf8View => array
                        .as_any()
                        .downcast_ref::<arrow_array::StringViewArray>()
                        .map(|a| a.value(i).as_bytes()),
                    DataType::Binary => array
                        .as_any()
                        .downcast_ref::<arrow_array::BinaryArray>()
                        .map(|a| a.value(i)),
                    DataType::LargeBinary => array
                        .as_any()
                        .downcast_ref::<arrow_array::LargeBinaryArray>()
                        .map(|a| a.value(i)),
                    DataType::BinaryView => array
                        .as_any()
                        .downcast_ref::<arrow_array::BinaryViewArray>()
                        .map(|a| a.value(i)),
                    DataType::FixedSizeBinary(_) => array
                        .as_any()
                        .downcast_ref::<arrow_array::FixedSizeBinaryArray>()
                        .map(|a| a.value(i)),
                    _ => None,
                }
                .ok_or_else(|| ArrowError::InvalidArgumentError("invalid variable array".into()))?;
                if value.len() > u32::MAX as usize {
                    return Err(ArrowError::InvalidArgumentError(
                        "value exceeds wire size".into(),
                    ));
                }
                encoder.uint(value.len() as u64);
                // Retain a matching Arrow buffer; inline view values are copied.
                let bytes = data
                    .buffers()
                    .iter()
                    .find_map(|buffer| {
                        let start =
                            (value.as_ptr() as usize).checked_sub(buffer.as_ptr() as usize)?;
                        (start.checked_add(value.len())? <= buffer.len())
                            .then(|| shared(buffer.slice_with_length(start, value.len())))
                    })
                    .unwrap_or_else(|| Bytes::copy_from_slice(value));
                encoder.raw(bytes);
            }
        }
        DataType::Decimal128(precision, _) if *precision <= 18 => {
            let a = array
                .as_any()
                .downcast_ref::<arrow_array::Decimal128Array>()
                .ok_or_else(|| ArrowError::InvalidArgumentError("invalid decimal array".into()))?;
            let width = if *precision <= 4 {
                2
            } else if *precision <= 9 {
                4
            } else {
                8
            };
            let mut bytes = Vec::with_capacity(a.len() * width);
            for i in 0..a.len() {
                bytes.extend_from_slice(&a.value(i).to_le_bytes()[..width]);
            }
            encoder.blob(bytes.into());
        }
        dt => {
            let width = match dt {
                DataType::Int8 | DataType::UInt8 => 1,
                DataType::Int16 | DataType::UInt16 => 2,
                DataType::Int32 | DataType::UInt32 | DataType::Float32 | DataType::Date32 => 4,
                DataType::Decimal128(_, _) => 16,
                _ => 8,
            };
            let data = array.to_data();
            let buffer = data
                .buffers()
                .first()
                .ok_or_else(|| ArrowError::InvalidArgumentError("missing value buffer".into()))?;
            let bytes = buffer.slice_with_length(data.offset() * width, array.len() * width);
            if cfg!(target_endian = "little") {
                encoder.blob(shared(bytes));
            } else {
                encoder.blob(Bytes::from(
                    bytes
                        .chunks_exact(width)
                        .flat_map(|b| b.iter().rev().copied())
                        .collect::<Vec<_>>(),
                ));
            }
        }
    }
    encoder.end();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{BooleanArray, Int32Array, StringArray};
    use arrow_schema::{Field, Schema};
    use std::sync::Arc;

    #[test]
    fn timestamp_normalization_rejects_overflow() {
        for unit in [TimeUnit::Second, TimeUnit::Millisecond] {
            for value in [i64::MIN, i64::MAX] {
                let array: ArrayRef = match unit {
                    TimeUnit::Second => Arc::new(
                        arrow_array::TimestampSecondArray::from(vec![Some(value), None])
                            .with_timezone("UTC"),
                    ),
                    _ => Arc::new(
                        arrow_array::TimestampMillisecondArray::from(vec![Some(value), None])
                            .with_timezone("UTC"),
                    ),
                };
                let batch = RecordBatch::try_new(
                    Arc::new(Schema::new(vec![Field::new(
                        "ts",
                        array.data_type().clone(),
                        true,
                    )])),
                    vec![array],
                )
                .unwrap();
                assert!(encode(&batch).is_err());
            }
        }
    }

    #[test]
    fn sliced_values_share_storage_and_survive_input_drop() {
        let array = Int32Array::from(vec![Some(11), None, Some(33), Some(44)]).slice(1, 3);
        let pointer = array.values().as_ptr().cast::<u8>();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, true)])),
            vec![Arc::new(array)],
        )
        .unwrap();
        let segments = encode(&batch).unwrap();
        assert!(
            segments
                .iter()
                .any(|b| b.as_ptr() == pointer && b.len() == 12)
        );
        drop(batch);
        let bytes: Vec<_> = segments.iter().flat_map(|b| b.iter().copied()).collect();
        let (chunk, _) = duckdb_wiredata::chunk::decode(bytes).unwrap();
        assert_eq!(chunk.columns()[0].get::<i32>(0), Some(None));
        assert_eq!(chunk.columns()[0].get::<i32>(1), Some(Some(33)));
        assert_eq!(chunk.columns()[0].get::<i32>(2), Some(Some(44)));
    }

    #[test]
    fn dictionary_values_are_shared() {
        let values = Arc::new(StringArray::from(vec!["hello", "world"]));
        let pointer = values.value(0).as_ptr();
        let dictionary = arrow_array::DictionaryArray::<arrow_array::types::Int8Type>::try_new(
            arrow_array::Int8Array::from(vec![1, 0, 1]),
            values,
        )
        .unwrap();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "d",
            dictionary.data_type().clone(),
            true,
        )]));
        let batch = RecordBatch::try_new(schema, vec![Arc::new(dictionary)]).unwrap();
        let segments = encode(&batch).unwrap();
        assert!(segments.iter().any(|b| b.as_ptr() == pointer));
        drop(batch);
        let (chunk, _) = duckdb_wiredata::chunk::decode(
            segments
                .iter()
                .flat_map(|b| b.iter().copied())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(
            chunk.columns()[0].get_bytes(0),
            Some(Some(b"world".as_slice()))
        );
        assert_eq!(
            chunk.columns()[0].get_bytes(1),
            Some(Some(b"hello".as_slice()))
        );
    }

    #[test]
    fn strings_share_values_and_boolean_bitmap_is_converted() {
        let strings = StringArray::from(vec![Some("alpha"), None, Some("beta")]);
        let pointer = strings.value(0).as_ptr();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("s", DataType::Utf8, true),
                Field::new("b", DataType::Boolean, true),
            ])),
            vec![
                Arc::new(strings),
                Arc::new(BooleanArray::from(vec![Some(true), None, Some(false)])),
            ],
        )
        .unwrap();
        let segments = encode(&batch).unwrap();
        assert!(segments.iter().any(|b| b.as_ptr() == pointer));
        drop(batch);
        let (chunk, _) = duckdb_wiredata::chunk::decode(
            segments
                .iter()
                .flat_map(|b| b.iter().copied())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(
            chunk.columns()[0].get_bytes(0),
            Some(Some(b"alpha".as_slice()))
        );
        assert_eq!(chunk.columns()[0].get_bytes(1), Some(None));
        assert_eq!(chunk.columns()[1].get::<bool>(0), Some(Some(true)));
        assert_eq!(chunk.columns()[1].get::<bool>(1), Some(None));
    }
}
