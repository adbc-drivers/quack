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

//! Owned messages for Quack revision `f96420ed46aee9c42064bdb99ddfd1230d6c0086`
//! (protocol 3, DuckDB serialization target V2_0_0).
//!
//! This codec targets little-endian peers. Protocol version alone does not
//! identify compatible schemas. HTTP transport and session rules are external.
//! PREPARE_RESPONSE metadata is decoded; nonempty inline results are unsupported.
//! Appended chunks remain opaque and unvalidated.
//! Decoding moves the input allocation into its opaque field without copying;
//! the field retains the entire allocation, including the parsed prefix.
//! Encoding borrows that field and allocates only the metadata prefix.

pub mod codec;
pub mod message;
pub mod state;

/// Protocol version of the supported revision, not an automatic negotiation policy.
pub const QUACK_VERSION: u64 = 3;
