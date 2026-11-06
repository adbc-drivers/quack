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
use adbc_core::{Connection, Optionable, Statement};
use arrow_array::RecordBatchReader;
use arrow_array::{Array, StringArray};
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
    pub(crate) manual: bool,
    pub(crate) session: std::sync::Arc<Session>,
}

impl QuackConnection {
    fn namespace(&self, function: &str) -> Result<String> {
        let mut statement = QuackStatement::new(self.session.clone());
        statement.set_sql_query(format!("SELECT {function}()"))?;
        let mut values = Vec::new();
        for batch in statement.execute()? {
            let batch = batch.map_err(|error| {
                ErrorHelper::invalid_data()
                    .context("read current namespace")
                    .message(error.to_string())
                    .to_adbc()
            })?;
            let array = batch
                .columns()
                .first()
                .and_then(|array| array.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| {
                    ErrorHelper::invalid_data()
                        .message("expected namespace string")
                        .to_adbc()
                })?;
            for index in 0..array.len() {
                if array.is_null(index) {
                    return Err(ErrorHelper::invalid_data()
                        .message("null namespace")
                        .to_adbc());
                }
                values.push(array.value(index).to_owned());
            }
        }
        if values.len() != 1 {
            return Err(ErrorHelper::invalid_data()
                .message("expected one namespace value")
                .to_adbc());
        }
        Ok(values.remove(0))
    }

    fn set_namespace(&self, catalog: bool, value: String) -> Result<()> {
        let (catalog, schema) = if catalog {
            (value, "main".to_owned())
        } else {
            (self.namespace("current_database")?, value)
        };
        let mut statement = QuackStatement::new(self.session.clone());
        statement.set_sql_query(format!(
            "USE {}.{}",
            crate::ingest::quote(&catalog),
            crate::ingest::quote(&schema)
        ))?;
        statement.execute_update().map(|_| ()).map_err(|mut error| {
            if error.status == adbc_core::error::Status::Unknown
                && (error.message.to_lowercase().contains("does not exist")
                    || error
                        .message
                        .contains("SET schema: No catalog + schema named"))
            {
                error.status = adbc_core::error::Status::NotFound;
            }
            error
        })
    }

    fn control(&self, sql: &str) -> Result<()> {
        let mut statement = QuackStatement::new(self.session.clone());
        statement.set_sql_query(sql)?;
        statement.execute_update()?;
        Ok(())
    }
    fn finish_transaction(&self, sql: &str) -> Result<()> {
        if !self.manual {
            return Err(ErrorHelper::invalid_state()
                .message("connection is in autocommit mode")
                .to_adbc());
        }
        self.control(sql)?;
        self.control("BEGIN TRANSACTION")
    }
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
        codes: Option<HashSet<InfoCode>>,
    ) -> Result<Box<dyn RecordBatchReader + Send>> {
        let mut info = driverbase::InfoRegistry::new();
        info.add_string(InfoCode::DriverName, "ADBC Driver for DuckDB Quack");
        info.add_string(InfoCode::VendorName, "DuckDB Quack");
        info.add_string(InfoCode::DriverVersion, env!("CARGO_PKG_VERSION"));
        info.add_string(
            InfoCode::DriverArrowVersion,
            format!("v{}", arrow_array::ARROW_VERSION),
        );
        info.add_bool(InfoCode::VendorSql, true);
        info.add_bool(InfoCode::VendorSubstrait, false);
        info.add_int64(InfoCode::DriverAdbcVersion, 1_001_000);
        if codes
            .as_ref()
            .is_none_or(|codes| codes.contains(&InfoCode::VendorVersion))
        {
            let mut statement = QuackStatement::new(self.session.clone());
            statement.set_sql_query("SELECT version()")?;
            let mut version = None;
            for batch in statement.execute()? {
                let batch = batch
                    .map_err(|e| ErrorHelper::invalid_data().message(e.to_string()).to_adbc())?;
                let values = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| {
                        ErrorHelper::invalid_data()
                            .message("invalid version result")
                            .to_adbc()
                    })?;
                if values.len() == 1 && !values.is_null(0) {
                    version = Some(values.value(0).trim_start_matches('v').to_owned());
                }
            }
            info.add_string(
                InfoCode::VendorVersion,
                version.ok_or_else(|| {
                    ErrorHelper::invalid_data()
                        .message("missing vendor version")
                        .to_adbc()
                })?,
            );
        }
        Ok(info.get_info(codes).build())
    }

    fn get_objects(
        &self,
        depth: ObjectDepth,
        catalog: Option<&str>,
        db_schema: Option<&str>,
        table_name: Option<&str>,
        table_type: Option<Vec<&str>>,
        column_name: Option<&str>,
    ) -> Result<Box<dyn RecordBatchReader + Send>> {
        Ok(driverbase::get_objects::get_objects(
            crate::objects::Objects(
                std::sync::Arc::clone(&self.session),
                column_name.map(str::to_owned),
            ),
            depth,
            catalog,
            db_schema,
            table_name,
            table_type,
            column_name,
        ))
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
        self.finish_transaction("COMMIT")
    }

    fn rollback(&mut self) -> Result<()> {
        self.finish_transaction("ROLLBACK")
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
            (OptionConnection::CurrentCatalog, OptionValue::String(value)) => {
                self.set_namespace(true, value)
            }
            (OptionConnection::CurrentSchema, OptionValue::String(value)) => {
                self.set_namespace(false, value)
            }
            (OptionConnection::CurrentCatalog | OptionConnection::CurrentSchema, _) => {
                Err(ErrorHelper::invalid_argument()
                    .message("namespace must be a string")
                    .to_adbc())
            }
            (OptionConnection::AutoCommit, OptionValue::String(value))
                if value == "true" || value == "false" =>
            {
                let manual = value == "false";
                if manual != self.manual {
                    self.control(if manual {
                        "BEGIN TRANSACTION"
                    } else {
                        "COMMIT"
                    })?;
                    self.manual = manual;
                }
                Ok(())
            }
            (OptionConnection::AutoCommit, _) => Err(ErrorHelper::invalid_argument()
                .message("autocommit must be true or false")
                .to_adbc()),
            _ => Err(ErrorHelper::set_unknown_option(&key).to_adbc()),
        }
    }

    fn get_option_string(&self, key: Self::Option) -> Result<String> {
        match key {
            OptionConnection::CurrentCatalog => self.namespace("current_database"),
            OptionConnection::CurrentSchema => self.namespace("current_schema"),
            OptionConnection::AutoCommit => Ok((!self.manual).to_string()),
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
