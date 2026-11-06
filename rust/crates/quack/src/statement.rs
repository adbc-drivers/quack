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

use crate::{
    connection::Session,
    conversion,
    error::{ErrorHelper, query_error},
};
use adbc_core::error::Result;
use adbc_core::options::{OptionStatement, OptionValue};
use adbc_core::{Optionable, PartitionedResult, Statement};
use arrow_array::{RecordBatch, RecordBatchReader};
use arrow_schema::{ArrowError, Schema, SchemaRef};
use driverbase::error::ErrorHelper as _;
use std::sync::Arc;

const INLINE_ROWS: &str = "quack.inline_rows";

pub struct QuackStatement {
    session: Arc<Session>,
    sql: Option<String>,
    inline_rows: Option<i64>,
    target: Option<String>,
    mode: adbc_core::options::IngestMode,
    bound: Option<Box<dyn RecordBatchReader + Send>>,
}

impl QuackStatement {
    pub(crate) fn new(session: Arc<Session>) -> Self {
        Self {
            session,
            sql: None,
            inline_rows: None,
            target: None,
            mode: adbc_core::options::IngestMode::Create,
            bound: None,
        }
    }
    fn ingest(&mut self) -> Result<Option<i64>> {
        use crate::ingest::{descriptor, encode, quote};
        use adbc_core::options::IngestMode;
        let target = quote(self.target.as_deref().ok_or_else(|| {
            ErrorHelper::invalid_state()
                .message("missing ingestion target")
                .to_adbc()
        })?);
        let reader = self.bound.take().ok_or_else(|| {
            ErrorHelper::invalid_state()
                .message("ingestion data is not bound")
                .to_adbc()
        })?;
        let schema = reader.schema();
        if schema.fields().is_empty() {
            return Err(ErrorHelper::invalid_data()
                .message("ingestion input has no columns")
                .to_adbc());
        }
        let types = schema
            .fields()
            .iter()
            .map(|f| descriptor(f.data_type()))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(ingest_arrow_error)?;
        let columns = schema
            .fields()
            .iter()
            .zip(&types)
            .map(|(f, (_, sql, _))| {
                format!(
                    "{} {}{}",
                    quote(f.name()),
                    sql,
                    if f.is_nullable() { "" } else { " NOT NULL" }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let ddl = match self.mode {
            IngestMode::Create => Some(format!("CREATE TABLE {target} ({columns})")),
            IngestMode::Replace => Some(format!("CREATE OR REPLACE TABLE {target} ({columns})")),
            IngestMode::CreateAppend => {
                Some(format!("CREATE TABLE IF NOT EXISTS {target} ({columns})"))
            }
            IngestMode::Append => None,
        };
        if let Some(sql) = ddl {
            let mut query = self.query_sql(sql, None).map_err(ingest_query_error)?;
            while self
                .session
                .runtime
                .block_on(query.next_chunk())
                .map_err(|e| ingest_query_error(query_error(e.as_ref())))?
                .is_some()
            {}
        }
        let stream_id = uuid::Uuid::new_v4().to_string();
        let prototype = types
            .iter()
            .enumerate()
            .map(|(i, (_, sql, _))| format!("c{i} {sql}"))
            .collect::<Vec<_>>()
            .join(", ");
        // Append/CreateAppend insert positionally without checking destination schemas.
        // Rely on server mismatch errors; reordered or renamed fields may silently succeed.
        let sql = format!(
            "INSERT INTO {target} SELECT * FROM scan_data_from_quack_client('{stream_id}', NULL::STRUCT({prototype}), ordered := true)"
        );
        let mut query = self.query_sql(sql, Some(0)).map_err(ingest_query_error)?;
        query
            .set_expected_result_schema(
                vec![duckdb_wiredata::logical_type::LogicalType {
                    id: duckdb_wiredata::logical_type::LogicalTypeId::BigInt,
                    info: None,
                }],
                vec!["Count".into()],
            )
            .map_err(|e| ingest_query_error(query_error(e.as_ref())))?;
        let result = (|| {
            let mut index = 0u64;
            let mut segments = Vec::new();
            let mut size = 0usize;
            let mut chunks = 0u64;
            let mut flush = |payload: Vec<bytes::Bytes>, chunk_count| -> Result<()> {
                index += 1;
                self.session
                    .runtime
                    .block_on(query.send_data(
                        quack_protocol::message::SendDataRequest {
                            stream_id: stream_id.clone().into(),
                            chunk_count,
                            batch_index: Some(index),
                            total_batches: None,
                            payload: Default::default(),
                        },
                        payload,
                    ))
                    .map_err(|e| ingest_query_error(query_error(e.as_ref())))
            };
            for batch in reader {
                let batch = batch.map_err(ingest_arrow_error)?;
                if batch.schema() != schema {
                    return Err(ErrorHelper::invalid_data()
                        .message("ingestion stream schema changed")
                        .to_adbc());
                }
                for offset in (0..batch.num_rows()).step_by(2048) {
                    let batch = batch.slice(offset, (batch.num_rows() - offset).min(2048));
                    let payload = encode(&batch).map_err(ingest_arrow_error)?;
                    let length: usize = payload.iter().map(|segment| segment.len()).sum();
                    if chunks != 0
                        && (size.saturating_add(length) > 4 * 1024 * 1024 || chunks == 128)
                    {
                        flush(std::mem::take(&mut segments), chunks)?;
                        size = 0;
                        chunks = 0;
                    }
                    segments.extend(payload);
                    size = size.saturating_add(length);
                    chunks += 1;
                }
            }
            if chunks != 0 {
                flush(segments, chunks)?;
            }
            self.session
                .runtime
                .block_on(query.send_data(
                    quack_protocol::message::SendDataRequest {
                        stream_id: stream_id.into(),
                        chunk_count: 0,
                        batch_index: None,
                        total_batches: Some(index),
                        payload: Default::default(),
                    },
                    vec![],
                ))
                .map_err(|e| ingest_query_error(query_error(e.as_ref())))?;
            while self
                .session
                .runtime
                .block_on(query.next_chunk())
                .map_err(|e| ingest_query_error(query_error(e.as_ref())))?
                .is_some()
            {}
            Ok(None)
        })();
        if result.is_err() {
            let _ = self.session.runtime.block_on(query.cancel());
        }
        result
    }

    fn query_sql(&self, sql: String, inline_rows: Option<u64>) -> Result<quack_tokio::Query> {
        let mut connection = self.session.connection()?;
        self.session
            .runtime
            .block_on(connection.execute(quack_protocol::state::QueryParams {
                query: sql.into(),
                uuid: query_uuid(),
                inline_rows,
            }))
            .map_err(|e| query_error(e.as_ref()))
    }

    fn start_query(&self) -> Result<quack_tokio::Query> {
        if self.bound.is_some() {
            return Err(ErrorHelper::not_implemented()
                .message("SQL parameter binding is unsupported")
                .to_adbc());
        }
        let sql = self.sql.as_ref().ok_or_else(|| {
            ErrorHelper::invalid_state()
                .message("SQL query is required")
                .to_adbc()
        })?;
        let mut connection = self.session.connection()?;
        let query = self
            .session
            .runtime
            .block_on(connection.execute(quack_protocol::state::QueryParams {
                query: sql.clone().into(),
                uuid: query_uuid(),
                inline_rows: self.inline_rows.map(|value| value as u64),
            }))
            .map_err(|error| query_error(error.as_ref()))?;
        drop(connection);
        Ok(query)
    }
}

fn ingest_arrow_error(error: ArrowError) -> adbc_core::error::Error {
    let helper = match error {
        ArrowError::NotYetImplemented(_) => ErrorHelper::not_implemented(),
        _ => ErrorHelper::invalid_data(),
    };
    helper
        .context("encode ingestion input")
        .message(error.to_string())
        .to_adbc()
}

fn ingest_query_error(mut error: adbc_core::error::Error) -> adbc_core::error::Error {
    use adbc_core::error::Status;
    let message = error.message.to_lowercase();
    if message.contains("already exists")
        || message.contains("column") && message.contains("supplied")
    {
        error.status = Status::AlreadyExists;
    } else if message.contains("does not exist") {
        error.status = Status::NotFound;
    } else if message.contains("constraint") {
        error.status = Status::Integrity;
    }
    error
}

fn query_uuid() -> quack_protocol::message::HugeInt {
    let (upper, lower) = uuid::Uuid::new_v4().as_u64_pair();
    quack_protocol::message::HugeInt {
        upper: (upper ^ (1u64 << 63)) as i64,
        lower,
    }
}

struct QueryReader {
    query: Option<quack_tokio::Query>,
    schema: SchemaRef,
    session: Arc<Session>,
}

impl QueryReader {
    fn finish(&mut self) {
        if let Some(mut query) = self.query.take() {
            let _ = self.session.runtime.block_on(query.cancel());
        }
    }
}

impl Drop for QueryReader {
    fn drop(&mut self) {
        self.finish();
    }
}

impl Iterator for QueryReader {
    type Item = std::result::Result<RecordBatch, ArrowError>;
    fn next(&mut self) -> Option<Self::Item> {
        let query = self.query.as_mut()?;
        match self.session.runtime.block_on(query.next_chunk()) {
            Ok(Some(chunk)) => {
                let result = conversion::record_batch(chunk, Arc::clone(&self.schema));
                if result.is_err() {
                    self.finish();
                }
                Some(result)
            }
            Ok(None) => {
                self.finish();
                None
            }
            Err(error) => {
                self.finish();
                Some(Err(ArrowError::ExternalError(error)))
            }
        }
    }
}

impl RecordBatchReader for QueryReader {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

impl Statement for QuackStatement {
    fn bind(&mut self, batch: RecordBatch) -> Result<()> {
        self.bound = Some(Box::new(arrow_array::RecordBatchIterator::new(
            vec![Ok(batch.clone())],
            batch.schema(),
        )));
        Ok(())
    }

    fn bind_stream(&mut self, reader: Box<dyn RecordBatchReader + Send>) -> Result<()> {
        self.bound = Some(reader);
        Ok(())
    }

    fn execute(&mut self) -> Result<Box<dyn RecordBatchReader + Send>> {
        if self.target.is_some() {
            return Err(ErrorHelper::invalid_state()
                .message("ingestion does not produce a result stream")
                .to_adbc());
        }
        let mut query = self.start_query()?;
        let schema = match conversion::schema(query.result_types(), query.result_names()) {
            Ok(schema) => schema,
            Err(error) => {
                let _ = self.session.runtime.block_on(query.cancel());
                let helper = match error {
                    ArrowError::NotYetImplemented(_) => ErrorHelper::not_implemented(),
                    _ => ErrorHelper::invalid_data(),
                };
                return Err(helper
                    .context("read Quack result schema")
                    .message(error.to_string())
                    .to_adbc());
            }
        };
        Ok(Box::new(QueryReader {
            query: Some(query),
            schema,
            session: Arc::clone(&self.session),
        }))
    }

    fn execute_update(&mut self) -> Result<Option<i64>> {
        if self.target.is_some() {
            return self.ingest();
        }
        let mut query = self.start_query()?;
        while self
            .session
            .runtime
            .block_on(query.next_chunk())
            .map_err(|error| query_error(error.as_ref()))?
            .is_some()
        {}
        Ok(None)
    }

    fn execute_schema(&mut self) -> Result<Schema> {
        Err(ErrorHelper::not_implemented()
            .message("execute_schema not implemented")
            .to_adbc())
    }

    fn execute_partitions(&mut self) -> Result<PartitionedResult> {
        Err(ErrorHelper::not_implemented()
            .message("execute_partitions not implemented")
            .to_adbc())
    }

    fn get_parameter_schema(&self) -> Result<Schema> {
        Err(ErrorHelper::not_implemented()
            .message("get_parameter_schema not implemented")
            .to_adbc())
    }

    fn prepare(&mut self) -> Result<()> {
        Err(ErrorHelper::not_implemented()
            .message("prepare not implemented")
            .to_adbc())
    }

    fn set_sql_query(&mut self, query: impl AsRef<str>) -> Result<()> {
        self.target = None;
        self.bound = None;
        self.sql = Some(query.as_ref().to_owned());
        Ok(())
    }

    fn set_substrait_plan(&mut self, _plan: impl AsRef<[u8]>) -> Result<()> {
        Err(ErrorHelper::not_implemented()
            .message("set_substrait_plan not implemented")
            .to_adbc())
    }

    fn cancel(&mut self) -> Result<()> {
        Err(ErrorHelper::not_implemented()
            .message("cancel not implemented")
            .to_adbc())
    }
}

impl Optionable for QuackStatement {
    type Option = OptionStatement;

    fn set_option(&mut self, key: Self::Option, value: OptionValue) -> Result<()> {
        match (&key, value) {
            (OptionStatement::TargetTable, OptionValue::String(value)) if !value.is_empty() => {
                self.target = Some(value);
                self.sql = None;
                Ok(())
            }
            (OptionStatement::IngestMode, value) => {
                self.mode = adbc_core::options::IngestMode::try_from(&value)?;
                Ok(())
            }
            (OptionStatement::Temporary, OptionValue::String(value)) if value == "false" => Ok(()),
            (
                OptionStatement::TargetCatalog
                | OptionStatement::TargetDbSchema
                | OptionStatement::Temporary,
                _,
            ) => Err(ErrorHelper::not_implemented()
                .message("optional ingestion targeting is unsupported")
                .to_adbc()),
            (OptionStatement::TargetTable, _) => Err(ErrorHelper::invalid_argument()
                .message("target table must be a nonempty string")
                .to_adbc()),
            (OptionStatement::Other(name), OptionValue::Int(value))
                if name == INLINE_ROWS && value >= 0 =>
            {
                self.inline_rows = Some(value);
                Ok(())
            }
            (OptionStatement::Other(name), _) if name == INLINE_ROWS => {
                Err(ErrorHelper::invalid_argument()
                    .message("quack.inline_rows must be a nonnegative integer")
                    .to_adbc())
            }
            _ => Err(ErrorHelper::set_unknown_option(&key).to_adbc()),
        }
    }

    fn get_option_string(&self, key: Self::Option) -> Result<String> {
        match key {
            OptionStatement::TargetTable => self.target.clone().ok_or_else(|| {
                ErrorHelper::invalid_state()
                    .message("target table is unset")
                    .to_adbc()
            }),
            OptionStatement::IngestMode => Ok(self.mode.into()),
            _ => Err(ErrorHelper::get_unknown_option(&key).to_adbc()),
        }
    }

    fn get_option_bytes(&self, key: Self::Option) -> Result<Vec<u8>> {
        Err(ErrorHelper::get_unknown_option(&key).to_adbc())
    }

    fn get_option_int(&self, key: Self::Option) -> Result<i64> {
        match &key {
            OptionStatement::Other(name) if name == INLINE_ROWS => {
                self.inline_rows.ok_or_else(|| {
                    ErrorHelper::invalid_state()
                        .message("quack.inline_rows is unset (server default)")
                        .to_adbc()
                })
            }
            _ => Err(ErrorHelper::get_unknown_option(&key).to_adbc()),
        }
    }

    fn get_option_double(&self, key: Self::Option) -> Result<f64> {
        Err(ErrorHelper::get_unknown_option(&key).to_adbc())
    }
}

#[cfg(test)]
mod tests;
