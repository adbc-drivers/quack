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
}

impl QuackStatement {
    pub(crate) fn new(session: Arc<Session>) -> Self {
        Self {
            session,
            sql: None,
            inline_rows: None,
        }
    }
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
    fn bind(&mut self, _batch: RecordBatch) -> Result<()> {
        Err(ErrorHelper::not_implemented()
            .message("bind not implemented")
            .to_adbc())
    }

    fn bind_stream(&mut self, _reader: Box<dyn RecordBatchReader + Send>) -> Result<()> {
        Err(ErrorHelper::not_implemented()
            .message("bind_stream not implemented")
            .to_adbc())
    }

    fn execute(&mut self) -> Result<Box<dyn RecordBatchReader + Send>> {
        let sql = self.sql.as_ref().ok_or_else(|| {
            ErrorHelper::invalid_state()
                .message("SQL query is required")
                .to_adbc()
        })?;
        let mut connection = self.session.connection()?;
        let mut query = self
            .session
            .runtime
            .block_on(connection.execute(quack_protocol::state::QueryParams {
                query: sql.clone().into(),
                uuid: query_uuid(),
                inline_rows: self.inline_rows.map(|value| value as u64),
            }))
            .map_err(|error| query_error(error.as_ref()))?;
        drop(connection);
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
        Err(ErrorHelper::not_implemented()
            .message("execute_update not implemented")
            .to_adbc())
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
        Err(ErrorHelper::get_unknown_option(&key).to_adbc())
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
