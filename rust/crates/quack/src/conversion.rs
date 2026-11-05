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
    chunk::{DataChunk, Primitive, PrimitiveEncoding, PrimitiveVector},
    logical_type::{LogicalType, LogicalTypeId},
};
use quack_protocol::message::WireString;

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
        fields.push(Field::new(name, data_type(logical_type.id)?, true));
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
        if &data_type(logical_type.id)? != field.data_type() {
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

fn numeric_array<T: ArrowPrimitiveType>(
    vector: &PrimitiveVector,
) -> Result<PrimitiveArray<T>, ArrowError>
where
    T::Native: Primitive,
{
    if let PrimitiveEncoding::Flat { values, validity } = vector.encoding() {
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
        let value = vector.get::<T::Native>(index).ok_or_else(|| {
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

fn data_type(id: LogicalTypeId) -> Result<DataType, ArrowError> {
    match id {
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

fn array(vector: &PrimitiveVector) -> Result<ArrayRef, ArrowError> {
    match vector.logical_type() {
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
