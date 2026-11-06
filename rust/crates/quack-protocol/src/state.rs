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

use crate::{QUACK_VERSION, message::*};

pub struct ConnectionOptions {
    pub auth_string: WireString,
    pub client_duckdb_version: WireString,
    pub client_platform: WireString,
    pub client_id: WireString,
    pub heartbeat_timeout_seconds: u64,
}

impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            auth_string: WireString::default(),
            client_duckdb_version: WireString::default(),
            client_platform: WireString::default(),
            client_id: WireString::default(),
            heartbeat_timeout_seconds: 60,
        }
    }
}

#[derive(Debug)]
pub struct Error(pub &'static str);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Error {}

#[derive(PartialEq)]
enum ConnectionState {
    Init,
    Connecting,
    Connected,
    Busy,
    Disconnected,
}

pub struct Connection {
    state: ConnectionState,
    id: WireString,
    timeout: u64,
    execution: std::sync::Arc<()>,
}

impl Default for Connection {
    fn default() -> Self {
        Self::new()
    }
}

impl Connection {
    pub fn new() -> Self {
        Self {
            state: ConnectionState::Init,
            id: WireString::default(),
            timeout: 0,
            execution: std::sync::Arc::new(()),
        }
    }

    pub fn pre_connect(&mut self, options: ConnectionOptions) -> Result<Message, Error> {
        if self.state != ConnectionState::Init {
            return Err(Error("connection is not initial"));
        }
        if options.heartbeat_timeout_seconds == 0
            || options.heartbeat_timeout_seconds > i64::MAX as u64 / 1000
        {
            return Err(Error("invalid heartbeat timeout"));
        }

        let request = ConnectionRequest {
            auth_string: options.auth_string,
            client_duckdb_version: options.client_duckdb_version,
            client_platform: options.client_platform,
            client_id: options.client_id,
            heartbeat_timeout_seconds: options.heartbeat_timeout_seconds,
            min_supported_quack_version: QUACK_VERSION,
            max_supported_quack_version: QUACK_VERSION,
        };

        self.state = ConnectionState::Connecting;

        Ok(Message {
            header: Header::default(),
            body: MessageBody::ConnectionRequest(request),
        })
    }

    pub fn connect(&mut self, header: Header, response: ConnectionResponse) -> Result<(), Error> {
        if self.state != ConnectionState::Connecting {
            return Err(Error("connection is not connecting"));
        }
        if header.connection_id.0.is_empty()
            || response.quack_version != QUACK_VERSION
            || response.heartbeat_timeout_seconds == 0
            || response.heartbeat_timeout_seconds > i64::MAX as u64 / 1000
        {
            self.disconnect();
            return Err(Error("invalid handshake response"));
        }

        self.id = header.connection_id;
        self.timeout = response.heartbeat_timeout_seconds;
        self.state = ConnectionState::Connected;

        Ok(())
    }

    pub fn heartbeat(&self) -> Result<Message, Error> {
        if self.state != ConnectionState::Connected && self.state != ConnectionState::Busy {
            return Err(Error("connection is not connected"));
        }

        Ok(Message {
            header: Header {
                connection_id: self.id.clone(),
                client_query_id: None,
            },
            body: MessageBody::HeartbeatRequest(HeartbeatRequest),
        })
    }

    pub fn pre_disconnect(&self) -> Result<Message, Error> {
        if self.state != ConnectionState::Connected && self.state != ConnectionState::Busy {
            return Err(Error("connection is not connected"));
        }

        Ok(Message {
            header: Header {
                connection_id: self.id.clone(),
                client_query_id: None,
            },
            body: MessageBody::DisconnectMessage(DisconnectMessage),
        })
    }

    pub fn heartbeat_timeout(&self) -> u64 {
        self.timeout
    }

    pub fn disconnect(&mut self) {
        self.state = ConnectionState::Disconnected;
    }

    pub fn fatal_transport_error(&mut self, _: &dyn std::error::Error) {
        self.disconnect();
    }
}

pub struct QueryParams {
    pub query: WireString,
    pub uuid: HugeInt,
    pub inline_rows: Option<u64>,
}

/// Query failures retain decoding and remote error details for adapters.
#[derive(Debug)]
pub enum QueryError {
    State(Error),
    Decode(crate::codec::DecodeError),
    Remote(ErrorResponse),
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::State(error) => error.fmt(f),
            Self::Decode(error) => error.fmt(f),
            Self::Remote(error) => write!(
                f,
                "{}: {}",
                String::from_utf8_lossy(error.exception_type.as_bytes()),
                String::from_utf8_lossy(error.message.as_bytes())
            ),
        }
    }
}
impl std::error::Error for QueryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::State(error) => Some(error),
            Self::Decode(error) => Some(error),
            Self::Remote(_) => None,
        }
    }
}
impl From<crate::codec::DecodeError> for QueryError {
    fn from(error: crate::codec::DecodeError) -> Self {
        Self::Decode(error)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum QueryState {
    ReadyPrepare,
    AwaitingPrepare,
    Delivering,
    ReadyFetch,
    AwaitingFetch { index: u64 },
    ReadyAck,
    AwaitingAck,
    AwaitingCancel,
    AwaitingUpload { delivering: bool },
    Complete,
    Released,
    Failed,
}

pub struct Query {
    execution: std::sync::Arc<()>,
    prepare_request: Option<PrepareRequest>,
    connection_id: WireString,
    uuid: HugeInt,
    state: QueryState,
    result_types: Vec<duckdb_wiredata::logical_type::LogicalType>,
    result_names: Vec<WireString>,
    pending: std::collections::VecDeque<duckdb_wiredata::chunk::DataChunk>,
    next_fetch: u64,
    accepted_batches: u64,
    terminal: bool,
    uploaded_batches: u64,
    upload_finished: bool,
}

impl Query {
    /// Build an upload request tied to this active execution.
    pub fn upload_request(&mut self, data: SendDataRequest) -> Result<Message, Error> {
        match self.state {
            QueryState::Delivering | QueryState::ReadyFetch => {}
            _ => return Err(Error("query is not accepting upload data")),
        }
        if self.upload_finished {
            return Err(Error("upload is already finished"));
        }
        if data.chunk_count == 0 {
            if data.batch_index.is_some() || data.total_batches != Some(self.uploaded_batches) {
                return Err(Error("invalid terminal upload count"));
            }
            self.upload_finished = true;
        } else {
            let next = self
                .uploaded_batches
                .checked_add(1)
                .ok_or(Error("upload batch index overflow"))?;
            if data.batch_index != Some(next) || data.total_batches.is_some() {
                return Err(Error("upload batches must be consecutive"));
            }
            self.uploaded_batches = next;
        }
        self.state = QueryState::AwaitingUpload {
            delivering: self.state == QueryState::Delivering,
        };
        Ok(Message {
            header: Header {
                connection_id: self.connection_id.clone(),
                client_query_id: None,
            },
            body: MessageBody::SendDataRequest(data),
        })
    }

    /// Supply known result metadata when PREPARE omitted it.
    pub fn set_expected_result_schema(
        &mut self,
        types: Vec<duckdb_wiredata::logical_type::LogicalType>,
        names: Vec<WireString>,
    ) -> Result<(), Error> {
        if types.len() != names.len() {
            return Err(Error("result schema lengths differ"));
        }
        if self.result_types.is_empty() && self.state == QueryState::ReadyFetch {
            self.result_types = types;
            self.result_names = names;
            Ok(())
        } else if self.result_types == types && self.result_names == names {
            Ok(())
        } else {
            Err(Error("conflicting result schema"))
        }
    }

    pub fn result_types(&self) -> &[duckdb_wiredata::logical_type::LogicalType] {
        &self.result_types
    }
    pub fn result_names(&self) -> &[WireString] {
        &self.result_names
    }
    pub fn query_uuid(&self) -> HugeInt {
        self.uuid
    }
    pub fn is_complete(&self) -> bool {
        matches!(self.state, QueryState::Complete | QueryState::Released)
    }
    /// Whether a failure of the outstanding exchange is cleanup-only.
    pub fn is_acknowledging(&self) -> bool {
        self.state == QueryState::AwaitingAck
    }
    pub fn is_failed(&self) -> bool {
        self.state == QueryState::Failed
    }

    /// Cancel this execution and discard buffered results.
    pub fn cancel_request(&mut self) -> Result<Message, Error> {
        match self.state {
            QueryState::Delivering | QueryState::ReadyFetch | QueryState::ReadyAck => {}
            _ => return Err(Error("query cannot be cancelled in its current state")),
        }
        self.pending.clear();
        self.state = QueryState::AwaitingCancel;
        Ok(Message {
            header: Header {
                connection_id: self.connection_id.clone(),
                client_query_id: None,
            },
            body: MessageBody::CancelRequest(CancelRequest {
                query_uuid: self.uuid,
            }),
        })
    }

    /// Produce the next legal request. Buffered chunks must be drained first.
    pub fn request(&mut self) -> Result<Message, Error> {
        let body = match self.state {
            QueryState::ReadyPrepare => {
                let request = self
                    .prepare_request
                    .take()
                    .ok_or(Error("missing prepare request"))?;
                self.state = QueryState::AwaitingPrepare;
                MessageBody::PrepareRequest(request)
            }
            QueryState::ReadyFetch => {
                self.state = QueryState::AwaitingFetch {
                    index: self.next_fetch,
                };
                MessageBody::FetchRequest(FetchRequest {
                    query_uuid: self.uuid,
                    batch_index: self.next_fetch,
                    ack_index: self.accepted_batches,
                })
            }
            QueryState::ReadyAck => {
                self.state = QueryState::AwaitingAck;
                MessageBody::Acknowledgement(Acknowledgement {
                    query_uuid: self.uuid,
                })
            }
            _ => return Err(Error("query is not ready to send a request")),
        };
        Ok(Message {
            header: Header {
                connection_id: self.connection_id.clone(),
                client_query_id: None,
            },
            body,
        })
    }

    /// Move a buffered chunk out. `None` means send a request or observe completion.
    pub fn next_chunk(&mut self) -> Result<Option<duckdb_wiredata::chunk::DataChunk>, Error> {
        match self.state {
            QueryState::Delivering => {
                let chunk = self
                    .pending
                    .pop_front()
                    .ok_or(Error("missing buffered chunk"))?;
                self.advance_delivery();
                Ok(Some(chunk))
            }
            QueryState::ReadyFetch
            | QueryState::ReadyAck
            | QueryState::Complete
            | QueryState::Released => Ok(None),
            _ => Err(Error("query is not ready to deliver chunks")),
        }
    }

    fn advance_delivery(&mut self) {
        self.state = if !self.pending.is_empty() {
            QueryState::Delivering
        } else if self.terminal {
            QueryState::ReadyAck
        } else {
            QueryState::ReadyFetch
        };
    }

    /// Accept exactly one outstanding response, retaining no copies of result buffers.
    pub fn response(&mut self, response: Message) -> Result<(), QueryError> {
        if self.state == QueryState::AwaitingCancel {
            self.state = QueryState::Failed;
            return match response.body {
                MessageBody::SuccessResponse(_) => {
                    self.state = QueryState::Complete;
                    Ok(())
                }
                MessageBody::ErrorResponse(error) => Err(QueryError::Remote(error)),
                _ => Err(QueryError::State(Error("unexpected cancellation response"))),
            };
        }
        match self.state {
            QueryState::AwaitingPrepare
            | QueryState::AwaitingFetch { .. }
            | QueryState::AwaitingAck
            | QueryState::AwaitingUpload { .. } => {}
            _ => return Err(QueryError::State(Error("query is not awaiting a response"))),
        }
        if let MessageBody::ErrorResponse(error) = response.body {
            self.remote_error(error.must_invalidate)
                .map_err(QueryError::State)?;
            return Err(QueryError::Remote(error));
        }
        let awaiting = std::mem::replace(&mut self.state, QueryState::Failed);
        // A failed validation leaves the query failed, so it cannot release the session.
        match (awaiting, response.body) {
            (QueryState::AwaitingUpload { delivering }, MessageBody::SendDataResponse(_)) => {
                self.state = if delivering {
                    QueryState::Delivering
                } else {
                    QueryState::ReadyFetch
                };
                Ok(())
            }
            (QueryState::AwaitingPrepare, MessageBody::PrepareResponse(body)) => {
                if body.query_uuid() != self.uuid {
                    return Err(QueryError::State(Error("query UUID does not match")));
                }
                let parts = body.into_parts();
                self.result_types = parts.result_types;
                self.result_names = parts.result_names;
                self.pending = parts.results.into();
                self.terminal = !parts.needs_more_fetch;
                self.advance_delivery();
                Ok(())
            }
            (QueryState::AwaitingFetch { index }, MessageBody::FetchResponse(body)) => {
                let chunks = body.decode_chunks()?;
                if body.chunk_count == 0 {
                    if body.batch_index.is_some()
                        || body
                            .total_batches
                            .is_some_and(|total| total != self.accepted_batches)
                    {
                        return Err(QueryError::State(Error("invalid terminal fetch response")));
                    }
                    self.terminal = true;
                } else {
                    if body.batch_index != Some(index) {
                        return Err(QueryError::State(Error("fetch batch index does not match")));
                    }
                    for chunk in &chunks {
                        crate::codec::validate_chunk_schema(&self.result_types, chunk)?;
                    }
                    self.next_fetch = index
                        .checked_add(1)
                        .ok_or(QueryError::State(Error("fetch batch index overflow")))?;
                    self.accepted_batches = index;
                }
                self.pending = chunks.into();
                self.advance_delivery();
                Ok(())
            }
            (QueryState::AwaitingAck, MessageBody::SuccessResponse(_)) => {
                self.state = QueryState::Complete;
                Ok(())
            }
            _ => Err(QueryError::State(Error("unexpected query response"))),
        }
    }

    /// Apply a decoded remote error only to an outstanding exchange.
    pub fn remote_error(&mut self, invalidate: bool) -> Result<(), Error> {
        match self.state {
            QueryState::AwaitingPrepare
            | QueryState::AwaitingFetch { .. }
            | QueryState::AwaitingAck
            | QueryState::AwaitingUpload { .. } => {}
            _ => return Err(Error("query is not awaiting a response")),
        }
        self.pending.clear();
        self.state = if invalidate {
            QueryState::Failed
        } else {
            QueryState::Complete
        };
        Ok(())
    }

    /// Invalidate an execution after transport failure or abandonment.
    pub fn fail(&mut self) {
        self.pending.clear();
        self.prepare_request = None;
        self.state = QueryState::Failed;
    }
}

impl Connection {
    pub fn prepare(&mut self, params: QueryParams) -> Result<Query, Error> {
        if self.state != ConnectionState::Connected {
            return Err(Error("connection is not idle"));
        }
        if params.uuid == HugeInt::default() {
            return Err(Error("query UUID must be nonzero"));
        }
        self.execution = std::sync::Arc::new(());
        self.state = ConnectionState::Busy;
        Ok(Query {
            execution: self.execution.clone(),
            uuid: params.uuid,
            state: QueryState::ReadyPrepare,
            connection_id: self.id.clone(),
            prepare_request: Some(PrepareRequest {
                sql_query: params.query,
                query_uuid: params.uuid,
                inline_rows: params.inline_rows,
            }),
            result_types: Vec::new(),
            result_names: Vec::new(),
            pending: std::collections::VecDeque::new(),
            next_fetch: 1,
            accepted_batches: 0,
            terminal: false,
            uploaded_batches: 0,
            upload_finished: false,
        })
    }

    /// Check execution identity and that the session has not been disconnected.
    pub fn check_query(&self, query: &Query) -> Result<(), Error> {
        if !std::sync::Arc::ptr_eq(&self.execution, &query.execution) {
            return Err(Error("query belongs to another execution"));
        }
        if self.state != ConnectionState::Busy {
            return Err(Error("connection has no active query"));
        }
        Ok(())
    }

    pub fn finish_query(&mut self, query: &mut Query) -> Result<Option<Message>, Error> {
        if !std::sync::Arc::ptr_eq(&self.execution, &query.execution) {
            return Err(Error("query belongs to another execution"));
        }
        if self.state == ConnectionState::Disconnected {
            query.fail();
            return Ok(None);
        }
        self.check_query(query)?;
        match query.state {
            QueryState::Complete => {
                self.state = ConnectionState::Connected;
                query.state = QueryState::Released;
                Ok(None)
            }
            _ => {
                let request = self.pre_disconnect()?;
                self.disconnect();
                query.fail();
                Ok(Some(request))
            }
        }
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod query_tests;
