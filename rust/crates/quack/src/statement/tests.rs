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

use super::*;
use adbc_core::error::Status;
use adbc_core::options::OptionDatabase;
use adbc_core::{Connection, Database};
use arrow_array::{Array, Int32Array};

fn database() -> Option<crate::database::QuackDatabase> {
    let endpoint = match std::env::var("QUACK_HTTP_ENDPOINT") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return None,
        Err(error) => panic!("{error}"),
    };
    let endpoint = url::Url::parse(&endpoint).unwrap();
    assert_eq!(endpoint.scheme(), "http");
    let uri = url::Url::parse(&format!(
        "quack://{}/?token=quack-secret&tls=false",
        &endpoint[url::Position::BeforeHost..url::Position::AfterPort]
    ))
    .unwrap();
    let mut database = crate::database::QuackDatabase::new().unwrap();
    database
        .set_option(OptionDatabase::Uri, OptionValue::String(uri.into()))
        .unwrap();
    Some(database)
}

#[test]
fn uuid_conversion() {
    let first = query_uuid();
    let second = query_uuid();
    assert_ne!(first, second);
    assert!(first.upper != 0 || first.lower != 0);
    let conventional_upper = first.upper as u64 ^ (1 << 63);
    assert_eq!((conventional_upper >> 12) & 15, 4);
    assert_eq!(first.lower >> 62, 2);
}

#[test]
fn query_error_categories_and_details() {
    use duckdb_wiredata::decode::DecodeErrorKind;
    use quack_protocol::{
        codec::DecodeError,
        message::ErrorResponse,
        state::{Error, QueryError},
    };
    assert_eq!(query_error(&Error("busy")).status, Status::InvalidState);
    assert_eq!(
        query_error(&QueryError::State(Error("bad index"))).status,
        Status::InvalidData
    );
    assert_eq!(
        query_error(&DecodeError {
            kind: DecodeErrorKind::Unsupported,
            offset: 1,
            context: "type",
            message: "unsupported".into()
        })
        .status,
        Status::NotImplemented
    );
    let remote = quack_tokio::RemoteError(ErrorResponse {
        message: "bad SQL".into(),
        exception_type: "Binder".into(),
        extra_info: vec![("detail".into(), "value".into())],
        must_invalidate: false,
    });
    let error = query_error(&remote);
    assert_eq!(error.status, Status::Unknown);
    assert!(
        error
            .details
            .unwrap()
            .iter()
            .any(|(key, value)| key == "detail" && value == b"value")
    );
}

#[test]
fn live_statement_streaming_and_ownership() {
    let Some(database) = database() else {
        return;
    };
    let mut connection = database.new_connection().unwrap();
    let mut statement = connection.new_statement().unwrap();
    assert_eq!(
        statement.execute().err().unwrap().status,
        Status::InvalidState
    );
    let option = OptionStatement::Other(INLINE_ROWS.into());
    assert_eq!(
        statement.get_option_int(option.clone()).unwrap_err().status,
        Status::InvalidState
    );
    for value in [
        OptionValue::Int(-1),
        OptionValue::String("0".into()),
        OptionValue::Double(0.0),
    ] {
        assert_eq!(
            statement
                .set_option(option.clone(), value)
                .unwrap_err()
                .status,
            Status::InvalidArguments
        );
    }
    statement.set_sql_query("SELECT 42").unwrap();
    for mode in [None, Some(0), Some(1)] {
        statement.inline_rows = mode;
        let mut reader = statement.execute().unwrap();
        assert_eq!(reader.schema().field(0).name(), "42");
        assert_eq!(
            statement.execute().err().unwrap().status,
            Status::InvalidState
        );
        let batch = reader.next().unwrap().unwrap();
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(values.value(0), 42);
        assert_eq!(values.null_count(), 0);
        assert!(reader.next().is_none());
    }
    statement
        .set_option(option.clone(), OptionValue::Int(0))
        .unwrap();
    assert_eq!(statement.get_option_int(option).unwrap(), 0);
    let mut reader = statement.execute().unwrap();
    drop(statement);
    drop(connection);
    drop(database);
    let batch = reader.next().unwrap().unwrap();
    assert!(reader.next().is_none());
    drop(reader);
    assert_eq!(
        batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap()
            .value(0),
        42
    );
}

#[test]
fn live_nullable_empty_and_multichunk_results() {
    let Some(database) = database() else {
        return;
    };
    let mut connection = database.new_connection().unwrap();
    let mut statement = connection.new_statement().unwrap();
    for mode in [None, Some(0), Some(1)] {
        statement.inline_rows = mode;
        statement
            .set_sql_query("SELECT 42::INTEGER AS answer, NULL::INTEGER AS missing")
            .unwrap();
        let mut reader = statement.execute().unwrap();
        let batch = reader.next().unwrap().unwrap();
        assert_eq!(batch.num_columns(), 2);
        assert_eq!(
            batch
                .column(0)
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap()
                .value(0),
            42
        );
        assert!(batch.column(1).is_null(0));
        assert!(reader.next().is_none());
        statement
            .set_sql_query("SELECT 42::INTEGER AS answer WHERE false")
            .unwrap();
        let mut reader = statement.execute().unwrap();
        assert_eq!(reader.schema().field(0).name(), "answer");
        assert_eq!(
            reader
                .by_ref()
                .map(|batch| batch.unwrap().num_rows())
                .sum::<usize>(),
            0
        );
    }
    statement.inline_rows = Some(0);
    statement
        .set_sql_query("SELECT i::INTEGER AS answer FROM range(5000) t(i)")
        .unwrap();
    let mut reader = statement.execute().unwrap();
    let mut batches = Vec::new();
    for batch in reader.by_ref() {
        batches.push(batch.unwrap());
    }
    assert!(batches.len() > 1);
    drop(reader);
    drop(statement);
    drop(connection);
    drop(database);
    let first = batches.remove(0);
    let mut expected = i32::try_from(first.num_rows()).unwrap();
    drop(first);
    for batch in batches {
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        for value in values.values() {
            assert_eq!(*value, expected);
            expected += 1;
        }
    }
    assert_eq!(expected, 5000);
}

#[test]
fn live_reader_abandonment_disconnects() {
    let Some(database) = database() else {
        return;
    };
    let mut connection = database.new_connection().unwrap();
    let mut statement = connection.new_statement().unwrap();
    statement.set_sql_query("SELECT 42").unwrap();
    statement
        .set_option(
            OptionStatement::Other(INLINE_ROWS.into()),
            OptionValue::Int(0),
        )
        .unwrap();
    drop(statement.execute().unwrap());
    assert_eq!(
        statement.execute().err().unwrap().status,
        Status::InvalidState
    );
}

#[test]
fn live_exported_adbc_query() {
    use adbc_core::constants::*;
    use adbc_ffi::{
        FFI_AdbcConnection, FFI_AdbcDatabase, FFI_AdbcDriver, FFI_AdbcError, FFI_AdbcStatement,
    };
    use arrow_array::ffi_stream::{FFI_ArrowArrayStream, RecordBatchStreamReader};
    use std::ffi::CString;
    let endpoint = match std::env::var("QUACK_HTTP_ENDPOINT") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return,
        Err(error) => panic!("{error}"),
    };
    let endpoint = url::Url::parse(&endpoint).unwrap();
    assert_eq!(endpoint.scheme(), "http");
    let uri = url::Url::parse(&format!(
        "quack://{}/?token=quack-secret&tls=false",
        &endpoint[url::Position::BeforeHost..url::Position::AfterPort]
    ))
    .unwrap();
    let uri = CString::new(uri.as_str()).unwrap();
    let mut driver = FFI_AdbcDriver::default();
    let mut error = FFI_AdbcError::default();
    let mut database = FFI_AdbcDatabase::default();
    let mut connection = FFI_AdbcConnection::default();
    let mut statement = FFI_AdbcStatement::default();
    // Each pointer refers to a live FFI handle; callbacks are supplied by this driver.
    unsafe {
        assert_eq!(
            crate::AdbcDriverQuackInit(
                ADBC_VERSION_1_1_0,
                (&mut driver as *mut FFI_AdbcDriver).cast(),
                &mut error
            ),
            ADBC_STATUS_OK
        );
        assert_eq!(
            driver.DatabaseNew.unwrap()(&mut database, &mut error),
            ADBC_STATUS_OK
        );
        assert_eq!(
            driver.DatabaseSetOption.unwrap()(
                &mut database,
                c"uri".as_ptr(),
                uri.as_ptr(),
                &mut error
            ),
            ADBC_STATUS_OK
        );
        assert_eq!(
            driver.DatabaseInit.unwrap()(&mut database, &mut error),
            ADBC_STATUS_OK
        );
        assert_eq!(
            driver.ConnectionNew.unwrap()(&mut connection, &mut error),
            ADBC_STATUS_OK
        );
        assert_eq!(
            driver.ConnectionInit.unwrap()(&mut connection, &mut database, &mut error),
            ADBC_STATUS_OK
        );
        assert_eq!(
            driver.StatementNew.unwrap()(&mut connection, &mut statement, &mut error),
            ADBC_STATUS_OK
        );
        let mut stream = FFI_ArrowArrayStream::empty();
        assert_eq!(
            driver.StatementExecuteQuery.unwrap()(
                &mut statement,
                &mut stream,
                std::ptr::null_mut(),
                &mut error
            ),
            ADBC_STATUS_INVALID_STATE
        );
        assert_eq!(
            driver.StatementSetSqlQuery.unwrap()(&mut statement, c"SELECT 42".as_ptr(), &mut error),
            ADBC_STATUS_OK
        );
        assert_eq!(
            driver.StatementSetOptionInt.unwrap()(
                &mut statement,
                c"quack.inline_rows".as_ptr(),
                -1,
                &mut error
            ),
            ADBC_STATUS_INVALID_ARGUMENT
        );
        for mode in [None, Some(0), Some(1)] {
            if let Some(mode) = mode {
                assert_eq!(
                    driver.StatementSetOptionInt.unwrap()(
                        &mut statement,
                        c"quack.inline_rows".as_ptr(),
                        mode,
                        &mut error
                    ),
                    ADBC_STATUS_OK
                );
            }
            assert_eq!(
                driver.StatementExecuteQuery.unwrap()(
                    &mut statement,
                    &mut stream,
                    std::ptr::null_mut(),
                    &mut error
                ),
                ADBC_STATUS_OK
            );
            let mut reader = RecordBatchStreamReader::try_new(stream).unwrap();
            assert_eq!(
                reader.schema().field(0).data_type(),
                &arrow_schema::DataType::Int32
            );
            let batch = reader.next().unwrap().unwrap();
            assert_eq!(
                batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .unwrap()
                    .value(0),
                42
            );
            assert!(reader.next().is_none());
            stream = FFI_ArrowArrayStream::empty();
        }
        assert_eq!(
            driver.StatementExecuteQuery.unwrap()(
                &mut statement,
                &mut stream,
                std::ptr::null_mut(),
                &mut error
            ),
            ADBC_STATUS_OK
        );
        let mut reader = RecordBatchStreamReader::try_new(stream).unwrap();
        assert_eq!(
            driver.StatementRelease.unwrap()(&mut statement, &mut error),
            ADBC_STATUS_OK
        );
        assert_eq!(
            driver.ConnectionRelease.unwrap()(&mut connection, &mut error),
            ADBC_STATUS_OK
        );
        assert_eq!(
            driver.DatabaseRelease.unwrap()(&mut database, &mut error),
            ADBC_STATUS_OK
        );
        let batch = reader.next().unwrap().unwrap();
        assert!(reader.next().is_none());
        drop(reader);
        assert_eq!(
            batch
                .column(0)
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap()
                .value(0),
            42
        );
    }
}

#[test]
fn live_remote_error_and_unsupported_schema() {
    let Some(database) = database() else {
        return;
    };
    let mut connection = database.new_connection().unwrap();
    let mut statement = connection.new_statement().unwrap();
    statement.set_sql_query("SELECT missing_column").unwrap();
    let error = statement.execute().err().unwrap();
    assert_eq!(error.status, Status::Unknown);
    assert!(
        error
            .details
            .unwrap()
            .iter()
            .any(|(key, _)| key == "quack.exception_type")
    );
    statement.set_sql_query("SELECT 42").unwrap();
    let mut reader = statement.execute().unwrap();
    assert!(reader.next().unwrap().is_ok());
    assert!(reader.next().is_none());
    statement.set_sql_query("SELECT TRUE").unwrap();
    assert_eq!(
        statement.execute().err().unwrap().status,
        Status::NotImplemented
    );
}
