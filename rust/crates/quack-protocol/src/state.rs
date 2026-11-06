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

enum QueryState {
    Awaiting,
    Outstanding,
    Complete,
    Failed,
}

pub struct Query {
    execution: std::sync::Arc<()>,
    request: Option<Message>,
    uuid: HugeInt,
    state: QueryState,
}

impl Query {
    pub fn request(&mut self) -> Result<Message, Error> {
        self.request
            .take()
            .ok_or(Error("query request already taken"))
    }

    pub fn response(&mut self, response: Message) -> Result<PrepareResponse, Error> {
        if !matches!(self.state, QueryState::Awaiting) || self.request.is_some() {
            return Err(Error("query is not awaiting a response"));
        }
        self.state = QueryState::Failed;
        match response.body {
            MessageBody::PrepareResponse(body) if body.query_uuid() == self.uuid => {
                self.state = if body.needs_more_fetch() {
                    QueryState::Outstanding
                } else {
                    QueryState::Complete
                };
                Ok(body)
            }
            _ => Err(Error("unexpected query response")),
        }
    }

    pub fn remote_error(&mut self, invalidate: bool) {
        self.state = if invalidate {
            QueryState::Failed
        } else {
            QueryState::Complete
        };
    }
}

impl Connection {
    pub fn prepare(&mut self, params: QueryParams) -> Result<Query, Error> {
        if self.state != ConnectionState::Connected {
            return Err(Error("connection is not idle"));
        }
        self.execution = std::sync::Arc::new(());
        self.state = ConnectionState::Busy;
        Ok(Query {
            execution: self.execution.clone(),
            uuid: params.uuid,
            state: QueryState::Awaiting,
            request: Some(Message {
                header: Header {
                    connection_id: self.id.clone(),
                    client_query_id: None,
                },
                body: MessageBody::PrepareRequest(PrepareRequest {
                    sql_query: params.query,
                    query_uuid: params.uuid,
                    inline_rows: params.inline_rows,
                }),
            }),
        })
    }

    pub fn finish_query(&mut self, query: Query) -> Result<Option<Message>, Error> {
        if !std::sync::Arc::ptr_eq(&self.execution, &query.execution) {
            return Err(Error("query belongs to another execution"));
        }
        if self.state == ConnectionState::Disconnected {
            return Ok(None);
        }
        match query.state {
            QueryState::Complete => {
                self.state = ConnectionState::Connected;
                Ok(None)
            }
            _ => {
                let request = self.pre_disconnect()?;
                self.disconnect();
                Ok(Some(request))
            }
        }
    }
}

#[cfg(test)]
mod query_tests {
    use super::*;
    fn connected() -> Connection {
        let mut connection = Connection::new();
        connection
            .pre_connect(ConnectionOptions::default())
            .unwrap();
        connection
            .connect(
                Header {
                    connection_id: "session".into(),
                    client_query_id: None,
                },
                ConnectionResponse {
                    quack_version: QUACK_VERSION,
                    heartbeat_timeout_seconds: 60,
                    ..Default::default()
                },
            )
            .unwrap();
        connection
    }
    fn params() -> QueryParams {
        QueryParams {
            query: "SELECT 42".into(),
            uuid: HugeInt { upper: 0, lower: 1 },
            inline_rows: Some(0),
        }
    }
    #[test]
    fn unfinished_query_disconnects() {
        let mut connection = connected();
        let mut query = connection.prepare(params()).unwrap();
        assert!(connection.prepare(params()).is_err());
        assert!(connection.heartbeat().is_ok());
        let request = query.request().unwrap();
        assert_eq!(request.header.connection_id, WireString::from("session"));
        assert!(query.request().is_err());
        assert!(connection.finish_query(query).unwrap().is_some());
        assert!(connection.prepare(params()).is_err());
    }
    #[test]
    fn recoverable_error_releases_connection() {
        let mut connection = connected();
        let mut query = connection.prepare(params()).unwrap();
        query.request().unwrap();
        query.remote_error(false);
        assert!(connection.finish_query(query).unwrap().is_none());
        assert!(connection.prepare(params()).is_ok());
    }
    #[test]
    fn wrong_connection_and_late_return() {
        let mut first = connected();
        let mut second = connected();
        let query = first.prepare(params()).unwrap();
        assert!(second.finish_query(query).is_err());
        let query = second.prepare(params()).unwrap();
        second.disconnect();
        assert!(second.finish_query(query).unwrap().is_none());
        assert!(second.heartbeat().is_err());
    }
}
