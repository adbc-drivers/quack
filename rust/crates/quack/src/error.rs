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

#[derive(Clone, Copy, Debug)]
pub struct ErrorHelper {}

impl driverbase::error::ErrorHelper for ErrorHelper {
    const NAME: &'static str = "quack";
}

/// Map transport/protocol failures while retaining server metadata for ADBC callers.
pub(crate) fn query_error(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> adbc_core::error::Error {
    use driverbase::error::ErrorHelper as _;
    use duckdb_wiredata::decode::DecodeErrorKind;
    use quack_protocol::{
        codec::DecodeError,
        state::{Error as StateError, QueryError},
    };
    let helper = if error.is::<tokio::time::error::Elapsed>() {
        ErrorHelper::timeout()
    } else if let Some(error) = error.downcast_ref::<DecodeError>() {
        match error.kind {
            DecodeErrorKind::Unsupported => ErrorHelper::not_implemented(),
            DecodeErrorKind::Malformed => ErrorHelper::invalid_data(),
        }
    } else if let Some(error) = error.downcast_ref::<QueryError>() {
        match error {
            QueryError::Decode(error) if error.kind == DecodeErrorKind::Unsupported => {
                ErrorHelper::not_implemented()
            }
            QueryError::Remote(_) => ErrorHelper::unknown(),
            _ => ErrorHelper::invalid_data(),
        }
    } else if error.is::<StateError>() {
        ErrorHelper::invalid_state()
    } else if error.is::<quack_tokio::RemoteError>() {
        ErrorHelper::unknown()
    } else {
        ErrorHelper::io()
    };
    let mut result = helper
        .context("execute Quack query")
        .message(error.to_string())
        .to_adbc();
    let remote = error
        .downcast_ref::<quack_tokio::RemoteError>()
        .map(|error| &error.0)
        .or_else(|| match error.downcast_ref::<QueryError>() {
            Some(QueryError::Remote(error)) => Some(error),
            _ => None,
        });
    if let Some(remote) = remote {
        let mut details = vec![
            (
                "quack.exception_type".into(),
                remote.exception_type.as_bytes().to_vec(),
            ),
            ("quack.message".into(), remote.message.as_bytes().to_vec()),
            (
                "quack.must_invalidate".into(),
                remote.must_invalidate.to_string().into_bytes(),
            ),
        ];
        details.extend(remote.extra_info.iter().map(|(key, value)| {
            (
                String::from_utf8_lossy(key.as_bytes()).into_owned(),
                value.as_bytes().to_vec(),
            )
        }));
        result.details = Some(details);
    }
    result
}
