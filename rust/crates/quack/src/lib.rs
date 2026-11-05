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

//! Minimal synchronous ADBC driver for Quack.
//!
//! Configure `uri` as `quack://HOST[:PORT]/?token=TOKEN&tls=MODE`.
//! The `quack.tls` database option overrides URI TLS settings. TLS defaults
//! to certificate verification; `false` selects HTTP and `skip_verify`
//! (or `skip-verify`) disables certificate verification.
//!
//! Set SQL on a statement and execute it to obtain a streaming Arrow reader.
//! INTEGER results, including nulls and flat/constant/dictionary/sequence
//! encodings, map to nullable Arrow Int32 columns. The integer statement option
//! `quack.inline_rows` controls PREPARE delivery: unset uses the server default,
//! zero forces FETCH, and positive values request inline rows. A positive
//! threshold may still require a terminal FETCH after delivering inline rows.
//!
//! One query may be active per session. Consume the reader through EOF before
//! executing another query. Dropping an unfinished reader waits up to two seconds to cancel its query.
//! Successful cancellation preserves the session; failure disconnects it. Readers retain session ownership when statements or connection
//! wrappers are dropped; returned batches own their buffers independently.
//!
//! Aligned flat INTEGER values on little-endian hosts and flat validity masks
//! share the received DuckDB buffers directly with Arrow. Other values are
//! materialized into final Arrow allocations. HTTP aggregation may copy before
//! decoding, and retaining a small buffer can retain the whole response body.
//!
//! Connections, statements, and readers share a background Tokio runtime.
//! Call this synchronous driver outside a Tokio runtime context.

mod connection;
mod conversion;
mod database;
mod driver;
mod error;
mod statement;

use adbc_ffi::export_driver;

export_driver!(AdbcDriverQuackInit, driver::QuackDriver);

#[cfg(test)]
mod primitive_tests;
