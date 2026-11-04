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
use adbc_core::error::Result;
use adbc_core::options::{OptionStatement, OptionValue};
use adbc_core::{Optionable, PartitionedResult, Statement};
use arrow_array::{RecordBatch, RecordBatchReader};
use arrow_schema::Schema;
use driverbase::error::ErrorHelper as _;

pub struct QuackStatement {}

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
        Err(ErrorHelper::not_implemented()
            .message("execute not implemented")
            .to_adbc())
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

    fn set_sql_query(&mut self, _query: impl AsRef<str>) -> Result<()> {
        Err(ErrorHelper::not_implemented()
            .message("set_sql_query not implemented")
            .to_adbc())
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

    fn set_option(&mut self, key: Self::Option, _value: OptionValue) -> Result<()> {
        Err(ErrorHelper::set_unknown_option(&key).to_adbc())
    }

    fn get_option_string(&self, key: Self::Option) -> Result<String> {
        Err(ErrorHelper::get_unknown_option(&key).to_adbc())
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
