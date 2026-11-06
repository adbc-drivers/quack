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

use crate::error::ErrorHelper;
use crate::statement::QuackStatement;
use adbc_core::error::Result;
use adbc_core::options::{InfoCode, ObjectDepth, OptionConnection, OptionValue};
use adbc_core::{Connection, Optionable};
use arrow_array::RecordBatchReader;
use arrow_schema::Schema;
use driverbase::error::ErrorHelper as _;
use std::collections::HashSet;

pub(crate) struct Session {
    pub(crate) connection: std::sync::Mutex<quack_tokio::Connection>,
    pub(crate) runtime: std::sync::Arc<tokio::runtime::Runtime>,
}

impl Session {
    pub(crate) fn connection(&self) -> Result<std::sync::MutexGuard<'_, quack_tokio::Connection>> {
        self.connection.lock().map_err(|error| {
            ErrorHelper::invalid_state()
                .context("access Quack session")
                .message(error.to_string())
                .to_adbc()
        })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Ok(connection) = self.connection.get_mut() {
            let _ = self.runtime.block_on(connection.disconnect());
        }
    }
}

pub struct QuackConnection {
    pub(crate) session: std::sync::Arc<Session>,
}

impl Connection for QuackConnection {
    type StatementType = QuackStatement;

    fn new_statement(&mut self) -> Result<Self::StatementType> {
        Ok(QuackStatement::new(std::sync::Arc::clone(&self.session)))
    }

    fn cancel(&mut self) -> Result<()> {
        Err(ErrorHelper::not_implemented()
            .message("cancel not implemented")
            .to_adbc())
    }

    fn get_info(
        &self,
        _codes: Option<HashSet<InfoCode>>,
    ) -> Result<Box<dyn RecordBatchReader + Send>> {
        Err(ErrorHelper::not_implemented()
            .message("get_info not implemented")
            .to_adbc())
    }

    fn get_objects(
        &self,
        _depth: ObjectDepth,
        _catalog: Option<&str>,
        _db_schema: Option<&str>,
        _table_name: Option<&str>,
        _table_type: Option<Vec<&str>>,
        _column_name: Option<&str>,
    ) -> Result<Box<dyn RecordBatchReader + Send>> {
        Err(ErrorHelper::not_implemented()
            .message("get_objects not implemented")
            .to_adbc())
    }

    fn get_table_schema(
        &self,
        _catalog: Option<&str>,
        _db_schema: Option<&str>,
        _table_name: &str,
    ) -> Result<Schema> {
        Err(ErrorHelper::not_implemented()
            .message("get_table_schema not implemented")
            .to_adbc())
    }

    fn get_table_types(&self) -> Result<Box<dyn RecordBatchReader + Send>> {
        Err(ErrorHelper::not_implemented()
            .message("get_table_types not implemented")
            .to_adbc())
    }

    fn get_statistic_names(&self) -> Result<Box<dyn RecordBatchReader + Send>> {
        Err(ErrorHelper::not_implemented()
            .message("get_statistic_names not implemented")
            .to_adbc())
    }

    fn get_statistics(
        &self,
        _catalog: Option<&str>,
        _db_schema: Option<&str>,
        _table_name: Option<&str>,
        _approximate: bool,
    ) -> Result<Box<dyn RecordBatchReader + Send>> {
        Err(ErrorHelper::not_implemented()
            .message("get_statistics not implemented")
            .to_adbc())
    }

    fn commit(&mut self) -> Result<()> {
        Err(ErrorHelper::not_implemented()
            .message("commit not implemented")
            .to_adbc())
    }

    fn rollback(&mut self) -> Result<()> {
        Err(ErrorHelper::not_implemented()
            .message("rollback not implemented")
            .to_adbc())
    }

    fn read_partition(
        &self,
        _partition: impl AsRef<[u8]>,
    ) -> Result<Box<dyn RecordBatchReader + Send>> {
        Err(ErrorHelper::not_implemented()
            .message("read_partition not implemented")
            .to_adbc())
    }
}

impl Optionable for QuackConnection {
    type Option = OptionConnection;

    fn set_option(&mut self, key: Self::Option, value: OptionValue) -> Result<()> {
        match (&key, value) {
            (OptionConnection::AutoCommit, OptionValue::String(value)) if value == "true" => Ok(()),
            (OptionConnection::AutoCommit, _) => Err(ErrorHelper::not_implemented()
                .message("manual transactions are unsupported")
                .to_adbc()),
            _ => Err(ErrorHelper::set_unknown_option(&key).to_adbc()),
        }
    }

    fn get_option_string(&self, key: Self::Option) -> Result<String> {
        match key {
            OptionConnection::AutoCommit => Ok("true".into()),
            _ => Err(ErrorHelper::get_unknown_option(&key).to_adbc()),
        }
    }

    fn get_option_bytes(&self, key: Self::Option) -> Result<Vec<u8>> {
        Err(ErrorHelper::get_unknown_option(&key).to_adbc())
    }

    fn get_option_int(&self, key: Self::Option) -> Result<i64> {
        Err(ErrorHelper::get_unknown_option(&key).to_adbc())
    }

    fn get_option_double(&self, key: Self::Option) -> Result<f64> {
        Err(ErrorHelper::get_unknown_option(&key).to_adbc())
    }
}
