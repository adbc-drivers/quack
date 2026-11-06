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

use arrow_array::{ArrayRef, Int32Array, RecordBatch, RecordBatchOptions};
use arrow_buffer::{BooleanBuffer, Buffer, NullBuffer, ScalarBuffer};
use arrow_schema::{ArrowError, DataType, Field, Schema, SchemaRef};
use duckdb_wiredata::{
    chunk::{DataChunk, IntegerEncoding, IntegerVector},
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
        if logical_type.id != LogicalTypeId::Integer {
            return Err(ArrowError::NotYetImplemented(format!(
                "unsupported result type {:?}",
                logical_type.id
            )));
        }
        let name = name
            .as_str()
            .map_err(|error| ArrowError::ExternalError(Box::new(error)))?;
        fields.push(Field::new(name, DataType::Int32, true));
    }
    Ok(Arc::new(Schema::new(fields)))
}

pub(crate) fn record_batch(chunk: DataChunk, schema: SchemaRef) -> Result<RecordBatch, ArrowError> {
    if schema.fields().len() != chunk.columns().len()
        || schema
            .fields()
            .iter()
            .any(|field| field.data_type() != &DataType::Int32)
    {
        return Err(ArrowError::SchemaError(
            "chunk does not match INTEGER Arrow schema".into(),
        ));
    }
    let rows = chunk.rows();
    let mut arrays: Vec<ArrayRef> = Vec::new();
    arrays
        .try_reserve_exact(chunk.columns().len())
        .map_err(|error| ArrowError::ExternalError(Box::new(error)))?;
    for column in chunk.into_columns() {
        arrays.push(Arc::new(integer_array(&column)?));
    }
    RecordBatch::try_new_with_options(
        schema,
        arrays,
        &RecordBatchOptions::new().with_row_count(Some(rows)),
    )
}

fn integer_array(vector: &IntegerVector) -> Result<Int32Array, ArrowError> {
    if let IntegerEncoding::Flat { values, validity } = vector.encoding() {
        let expected = vector
            .len()
            .checked_mul(size_of::<i32>())
            .ok_or_else(|| ArrowError::ComputeError("INTEGER values size overflow".into()))?;
        if values.len() != expected {
            return Err(ArrowError::ComputeError(
                "INTEGER values size differs from row count".into(),
            ));
        }
        let values = if cfg!(target_endian = "little")
            && values.as_ptr().align_offset(align_of::<i32>()) == 0
        {
            // Length and native alignment are checked before ScalarBuffer's constructor.
            ScalarBuffer::from(Buffer::from(values.clone()))
        } else {
            let mut decoded = Vec::new();
            decoded
                .try_reserve_exact(vector.len())
                .map_err(|error| ArrowError::ExternalError(Box::new(error)))?;
            for value in values.as_chunks::<4>().0 {
                decoded.push(i32::from_le_bytes([value[0], value[1], value[2], value[3]]));
            }
            ScalarBuffer::from(decoded)
        };
        let nulls = validity
            .map(|mask| {
                let expected = vector.len().div_ceil(64).checked_mul(8).ok_or_else(|| {
                    ArrowError::ComputeError("INTEGER validity size overflow".into())
                })?;
                if mask.len() != expected {
                    return Err(ArrowError::ComputeError(
                        "INTEGER validity size differs from row count".into(),
                    ));
                }
                Ok(NullBuffer::new(BooleanBuffer::new(
                    Buffer::from(mask.clone()),
                    0,
                    vector.len(),
                )))
            })
            .transpose()?;
        return Int32Array::try_new(values, nulls);
    }

    // Compressed values expand directly into their final Arrow allocation.
    // Create validity only on the first null, without an intermediate bool array.
    let mut values = Vec::new();
    values
        .try_reserve_exact(vector.len())
        .map_err(|error| ArrowError::ExternalError(Box::new(error)))?;
    let mut validity: Option<Vec<u8>> = None;
    for index in 0..vector.len() {
        let value = vector.get(index).ok_or_else(|| {
            ArrowError::ComputeError("validated INTEGER vector has no value at row".into())
        })?;
        match value {
            Some(value) => values.push(value),
            None => {
                values.push(0);
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
    Int32Array::try_new(values.into(), nulls)
}

#[cfg(test)]
#[path = "conversion_tests.rs"]
mod tests;
