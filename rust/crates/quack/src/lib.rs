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
//! Connections own a share of a background Tokio runtime. Call this
//! synchronous driver outside a Tokio runtime context.

mod connection;
mod database;
mod driver;
mod error;
mod statement;

use adbc_ffi::export_driver;

export_driver!(AdbcDriverQuackInit, driver::QuackDriver);
