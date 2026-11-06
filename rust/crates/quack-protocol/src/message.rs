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

use bytes::Bytes;
use duckdb_wiredata::chunk::DataChunk;

/// A wire string is bytes, not necessarily UTF-8.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WireString(pub Vec<u8>);

impl WireString {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn as_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.0)
    }
}

impl From<&str> for WireString {
    fn from(value: &str) -> Self {
        Self(value.as_bytes().to_vec())
    }
}

impl From<String> for WireString {
    fn from(value: String) -> Self {
        Self(value.into_bytes())
    }
}

impl From<Vec<u8>> for WireString {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

/// An independently owned view into an allocation. Intentionally not Clone.
///
/// Slices retain the underlying allocation until its last owner is dropped.
#[derive(Default)]
pub struct OpaqueBytes {
    buffer: Bytes,
}

impl OpaqueBytes {
    /// Take ownership of a complete opaque body or appended chunk payload.
    pub fn new(buffer: impl Into<Bytes>) -> Self {
        Self {
            buffer: buffer.into(),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer
    }

    /// Share the visible bytes without copying their contents.
    pub fn bytes(&self) -> &Bytes {
        &self.buffer
    }

    pub fn into_bytes(self) -> Bytes {
        self.buffer
    }

    pub(crate) fn tail(buffer: Bytes, start: usize) -> Self {
        Self {
            buffer: buffer.slice(start..),
        }
    }
}

impl From<Vec<u8>> for OpaqueBytes {
    fn from(buffer: Vec<u8>) -> Self {
        Self::new(buffer)
    }
}

impl From<Bytes> for OpaqueBytes {
    fn from(buffer: Bytes) -> Self {
        Self::new(buffer)
    }
}

impl std::fmt::Debug for OpaqueBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpaqueBytes")
            .field("len", &self.buffer.len())
            .finish_non_exhaustive()
    }
}

/// DuckDB hugeint wire halves. Conventional UUIDs additionally flip the upper
/// sign bit; these fields are not RFC-4122 network-order UUID bytes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HugeInt {
    pub upper: i64,
    pub lower: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub connection_id: WireString,
    pub client_query_id: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Message {
    pub header: Header,
    pub body: MessageBody,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MessageBody {
    ConnectionRequest(ConnectionRequest),
    ConnectionResponse(ConnectionResponse),
    PrepareRequest(PrepareRequest),
    PrepareResponse(PrepareResponse),
    FetchRequest(FetchRequest),
    FetchResponse(FetchResponse),
    SendDataRequest(SendDataRequest),
    SuccessResponse(SuccessResponse),
    DisconnectMessage(DisconnectMessage),
    CancelRequest(CancelRequest),
    SendDataResponse(SendDataResponse),
    Acknowledgement(Acknowledgement),
    HeartbeatRequest(HeartbeatRequest),
    ErrorResponse(ErrorResponse),
}

impl MessageBody {
    pub fn message_id(&self) -> u8 {
        match self {
            Self::ConnectionRequest(_) => 1,
            Self::ConnectionResponse(_) => 2,
            Self::PrepareRequest(_) => 3,
            Self::PrepareResponse(_) => 4,
            Self::FetchRequest(_) => 7,
            Self::FetchResponse(_) => 8,
            Self::SendDataRequest(_) => 9,
            Self::SuccessResponse(_) => 10,
            Self::DisconnectMessage(_) => 11,
            Self::CancelRequest(_) => 12,
            Self::SendDataResponse(_) => 14,
            Self::Acknowledgement(_) => 15,
            Self::HeartbeatRequest(_) => 16,
            Self::ErrorResponse(_) => 100,
        }
    }
}

// Defaults here are wire defaults, not necessarily usable session settings.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ConnectionRequest {
    pub auth_string: WireString,
    pub client_duckdb_version: WireString,
    pub client_platform: WireString,
    pub min_supported_quack_version: u64,
    pub max_supported_quack_version: u64,
    pub client_id: WireString,
    pub heartbeat_timeout_seconds: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ConnectionResponse {
    pub server_duckdb_version: WireString,
    pub server_platform: WireString,
    pub quack_version: u64,
    pub heartbeat_timeout_seconds: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PrepareRequest {
    pub sql_query: WireString,
    pub query_uuid: HugeInt,
    pub inline_rows: Option<u64>,
}

/// Owned PREPARE metadata and decoded chunk views.
#[derive(Debug, PartialEq, Eq)]
pub struct PrepareResponseParts {
    pub result_types: Vec<duckdb_wiredata::logical_type::LogicalType>,
    pub result_names: Vec<WireString>,
    pub needs_more_fetch: bool,
    pub query_uuid: HugeInt,
    pub results: Vec<DataChunk>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PrepareResponse {
    body: OpaqueBytes,
    result_types: Vec<duckdb_wiredata::logical_type::LogicalType>,
    result_names: Vec<WireString>,
    needs_more_fetch: bool,
    query_uuid: HugeInt,
    results: Vec<duckdb_wiredata::chunk::DataChunk>,
}

impl PrepareResponse {
    /// Validate and decode a complete body, including inline chunk views.
    /// Error offsets are relative to the beginning of this body.
    pub fn from_body(body: OpaqueBytes) -> Result<Self, crate::codec::DecodeError> {
        let PrepareResponseParts {
            result_types,
            result_names,
            needs_more_fetch,
            query_uuid,
            results,
        } = crate::codec::decode_prepare_body(body.bytes())?;
        Ok(Self {
            body,
            result_types,
            result_names,
            needs_more_fetch,
            query_uuid,
            results,
        })
    }

    pub fn result_types(&self) -> &[duckdb_wiredata::logical_type::LogicalType] {
        &self.result_types
    }
    pub fn result_names(&self) -> &[WireString] {
        &self.result_names
    }
    pub fn needs_more_fetch(&self) -> bool {
        self.needs_more_fetch
    }
    pub fn query_uuid(&self) -> HugeInt {
        self.query_uuid
    }
    pub fn results(&self) -> &[duckdb_wiredata::chunk::DataChunk] {
        &self.results
    }
    /// Move schema and decoded chunks out without cloning them or their buffers.
    pub fn into_parts(self) -> PrepareResponseParts {
        PrepareResponseParts {
            result_types: self.result_types,
            result_names: self.result_names,
            needs_more_fetch: self.needs_more_fetch,
            query_uuid: self.query_uuid,
            results: self.results,
        }
    }
    pub fn validate_chunk(
        &self,
        chunk: &duckdb_wiredata::chunk::DataChunk,
    ) -> Result<(), crate::codec::DecodeError> {
        crate::codec::validate_chunk_schema(&self.result_types, chunk)
    }
    /// Original validated body, retained for byte-preserving re-encoding.
    pub fn body(&self) -> &OpaqueBytes {
        &self.body
    }
    pub fn into_body(self) -> OpaqueBytes {
        self.body
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct FetchRequest {
    pub query_uuid: HugeInt,
    pub batch_index: u64,
    pub ack_index: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct FetchResponse {
    /// Declared count; opaque payload contents are not checked against it.
    pub chunk_count: u64,
    pub total_batches: Option<u64>,
    pub batch_index: Option<u64>,
    pub payload: OpaqueBytes,
}

impl FetchResponse {
    /// Decode exactly the declared bare chunks, sharing this payload's allocation.
    /// Error offsets are relative to the beginning of the payload.
    pub fn decode_chunks(
        &self,
    ) -> Result<Vec<duckdb_wiredata::chunk::DataChunk>, crate::codec::DecodeError> {
        crate::codec::decode_fetch_chunks(self.payload.bytes(), self.chunk_count)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SendDataRequest {
    pub stream_id: WireString,
    pub chunk_count: u64,
    pub total_batches: Option<u64>,
    pub batch_index: Option<u64>,
    pub payload: OpaqueBytes,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SendDataResponse {
    pub accept_budget: Option<u64>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CancelRequest {
    pub query_uuid: HugeInt,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Acknowledgement {
    pub query_uuid: HugeInt,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ErrorResponse {
    pub message: WireString,
    pub exception_type: WireString,
    /// Wire map entries in received order; duplicate keys are preserved.
    pub extra_info: Vec<(WireString, WireString)>,
    pub must_invalidate: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SuccessResponse;
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DisconnectMessage;
#[derive(Debug, Default, PartialEq, Eq)]
pub struct HeartbeatRequest;

impl PartialEq for OpaqueBytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for OpaqueBytes {}
