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

//! Tokio connection orchestration and incremental query delivery over HTTP.
//!
//! [`Connection::execute`] completes PREPARE and returns a [`Query`] with result
//! schema accessors. Call [`Query::next_chunk`] through EOF to consume inline
//! chunks followed by sequential FETCH batches and final acknowledgement.
//! A connection accepts one active query; dropping an unfinished query or
//! cancelling an execution/fetch future invalidates that session immediately.
//! Remote cleanup is best effort when a Tokio runtime is available.
//!
//! Chunks retain shared views into completed HTTP response bodies, preserving
//! compressed INTEGER vectors until a consumer needs to materialize them.

use duckdb_wiredata::{chunk::DataChunk, logical_type::LogicalType};
use quack_protocol::{codec, message::*, state};
use std::{
    sync::{Arc, Mutex, MutexGuard, Weak},
    time::Duration,
};
use tokio::{
    task::{AbortHandle, JoinHandle},
    time::Instant,
};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub enum TlsMode {
    #[default]
    Verify,
    Disabled,
    SkipVerify,
}

pub struct ConnectionOptions {
    pub endpoint: String,
    pub tls: TlsMode,
    pub protocol: state::ConnectionOptions,
}

/// A remote Quack error, retaining all protocol metadata.
#[derive(Debug)]
pub struct RemoteError(pub ErrorResponse);
impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}",
            String::from_utf8_lossy(self.0.exception_type.as_bytes()),
            String::from_utf8_lossy(self.0.message.as_bytes())
        )
    }
}
impl std::error::Error for RemoteError {}

async fn exchange(
    client: &reqwest::Client,
    endpoint: &reqwest::Url,
    request: Message,
) -> Result<Message, Error> {
    let bytes = codec::encode(&request)?.into_vec();
    let response = client
        .post(endpoint.clone())
        .header(reqwest::header::CONTENT_TYPE, "application/vnd.duckdb")
        .body(bytes)
        .send()
        .await?;
    let status = response.status();
    let response = response.error_for_status()?;
    if status != reqwest::StatusCode::OK {
        return Err(std::io::Error::other(format!("unexpected HTTP status: {status}")).into());
    }
    Ok(codec::decode(response.bytes().await?)?)
}

struct Core {
    state: Mutex<state::Connection>,
    client: reqwest::Client,
    endpoint: reqwest::Url,
    heartbeat: Mutex<Option<AbortHandle>>,
}
impl Core {
    fn state(&self) -> Result<MutexGuard<'_, state::Connection>, Error> {
        self.state
            .lock()
            .map_err(|_| Box::new(state::Error("connection state lock poisoned")) as Error)
    }
    fn stop_heartbeat(&self) {
        let handle = match self.heartbeat.lock() {
            Ok(mut handle) => handle.take(),
            Err(error) => error.into_inner().take(),
        };
        if let Some(handle) = handle {
            handle.abort();
        }
    }
    /// Also used from Drop: even a poisoned mutex must not leave a session usable.
    fn invalidate(&self) -> Option<Message> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(error) => error.into_inner(),
        };
        let request = state.pre_disconnect().ok();
        state.disconnect();
        drop(state);
        self.stop_heartbeat();
        request
    }
    fn cleanup(self: &Arc<Self>, request: Option<Message>) {
        if let (Some(request), Ok(runtime)) = (request, tokio::runtime::Handle::try_current()) {
            let core = Arc::clone(self);
            runtime.spawn(async move {
                let _ = tokio::time::timeout(
                    CLEANUP_TIMEOUT,
                    exchange(&core.client, &core.endpoint, request),
                )
                .await;
            });
        }
    }
    async fn exchange(&self, request: Message) -> Result<Message, Error> {
        tokio::time::timeout(
            REQUEST_TIMEOUT,
            exchange(&self.client, &self.endpoint, request),
        )
        .await?
    }
}

pub struct Connection {
    core: Arc<Core>,
    heartbeat: Option<JoinHandle<()>>,
}
impl Connection {
    /// Establish a session and start heartbeats before returning.
    pub async fn new(options: ConnectionOptions) -> Result<Self, Error> {
        let endpoint = reqwest::Url::parse(&options.endpoint)?;
        let (scheme, skip_verify) = match options.tls {
            TlsMode::Verify => ("https", false),
            TlsMode::SkipVerify => ("https", true),
            TlsMode::Disabled => ("http", false),
        };
        if endpoint.scheme() != scheme || endpoint.host_str().is_none() {
            return Err(
                state::Error("endpoint scheme conflicts with TLS mode or lacks a host").into(),
            );
        }
        let client = reqwest::Client::builder()
            .tls_danger_accept_invalid_certs(skip_verify)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()?;
        let mut connection = Self {
            core: Arc::new(Core {
                state: Mutex::new(state::Connection::new()),
                client,
                endpoint,
                heartbeat: Mutex::new(None),
            }),
            heartbeat: None,
        };
        let request = connection.core.state()?.pre_connect(options.protocol)?;
        let sent = Instant::now();
        let response = connection.core.exchange(request).await?;
        let body = match response.body {
            MessageBody::ConnectionResponse(body) => body,
            MessageBody::ErrorResponse(error) => return Err(Box::new(RemoteError(error))),
            _ => return Err(state::Error("unexpected handshake response").into()),
        };
        let timeout = {
            let mut state = connection.core.state()?;
            state.connect(response.header, body)?;
            Duration::from_secs(state.heartbeat_timeout())
        };
        let task = tokio::spawn(heartbeat(Arc::downgrade(&connection.core), timeout, sent));
        *connection
            .core
            .heartbeat
            .lock()
            .map_err(|_| state::Error("heartbeat lock poisoned"))? = Some(task.abort_handle());
        connection.heartbeat = Some(task);
        Ok(connection)
    }
    /// Start execution; the returned handle owns the active execution until EOF.
    pub async fn execute(&mut self, params: state::QueryParams) -> Result<Query, Error> {
        let protocol = self.core.state()?.prepare(params)?;
        let mut query = Query {
            core: Arc::clone(&self.core),
            protocol,
            done: false,
        };
        // The query's Drop guard exists before the first network await.
        query.exchange().await?;
        Ok(query)
    }
    pub async fn disconnect(&mut self) -> Result<(), Error> {
        let request = self.core.invalidate();
        if let Some(task) = self.heartbeat.take() {
            let _ = task.await;
        }
        if let Some(request) = request {
            let response = tokio::time::timeout(
                CLEANUP_TIMEOUT,
                exchange(&self.core.client, &self.core.endpoint, request),
            )
            .await??;
            match response.body {
                MessageBody::SuccessResponse(_) => {}
                MessageBody::ErrorResponse(error) => return Err(Box::new(RemoteError(error))),
                _ => return Err(state::Error("unexpected disconnect response").into()),
            }
        }
        Ok(())
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        let request = self.core.invalidate();
        self.core.cleanup(request);
    }
}

/// An active query. Dropping it before EOF disconnects its session.
pub struct Query {
    core: Arc<Core>,
    protocol: state::Query,
    done: bool,
}
impl Query {
    /// Send a segmented payload, retaining each buffer until HTTP consumes it.
    /// A zero-chunk terminal request closes the upload before fetching the result.
    pub async fn send_data(
        &mut self,
        data: quack_protocol::message::SendDataRequest,
        payload: Vec<bytes::Bytes>,
    ) -> Result<(), Error> {
        self.core.state()?.check_query(&self.protocol)?;
        let payload_len = payload
            .iter()
            .try_fold(0usize, |n, b| n.checked_add(b.len()))
            .ok_or(state::Error("upload length overflow"))?;
        if (data.chunk_count == 0) != (payload_len == 0) || !data.payload.as_bytes().is_empty() {
            return Err(Box::new(state::Error("invalid segmented upload payload")));
        }
        let mut operation = Operation {
            query: self,
            armed: true,
        };
        let result: Result<(), Error> = async {
            let request = operation.query.protocol.upload_request(data)?;
            let prefix = bytes::Bytes::from(codec::encode_upload_header(&request)?);
            let length = prefix
                .len()
                .checked_add(payload_len)
                .ok_or(state::Error("upload length overflow"))?;
            let segments = std::iter::once(prefix).chain(payload);
            let body = reqwest::Body::wrap_stream(futures_util::stream::iter(
                segments.map(Ok::<_, std::io::Error>),
            ));

            let response = tokio::time::timeout(REQUEST_TIMEOUT, async {
                let response = operation
                    .query
                    .core
                    .client
                    .post(operation.query.core.endpoint.clone())
                    .header(reqwest::header::CONTENT_TYPE, "application/vnd.duckdb")
                    .header(reqwest::header::CONTENT_LENGTH, length)
                    .body(body)
                    .send()
                    .await?
                    .error_for_status()?;
                response.bytes().await
            })
            .await??;
            operation
                .query
                .core
                .state()?
                .check_query(&operation.query.protocol)?;
            operation
                .query
                .protocol
                .response(codec::decode(response)?)
                .map_err(|error| -> Error {
                    match error {
                        state::QueryError::Remote(error) => Box::new(RemoteError(error)),
                        error => Box::new(error),
                    }
                })
        }
        .await;
        if result.is_err() {
            if operation.query.protocol.is_complete() {
                operation.query.finish()?;
            } else {
                operation.query.abandon();
            }
        }
        operation.armed = false;
        result
    }

    /// Supply known result metadata when PREPARE omitted it.
    pub fn set_expected_result_schema(
        &mut self,
        types: Vec<LogicalType>,
        names: Vec<WireString>,
    ) -> Result<(), Error> {
        self.protocol.set_expected_result_schema(types, names)?;
        Ok(())
    }

    pub fn result_types(&self) -> &[LogicalType] {
        self.protocol.result_types()
    }
    pub fn result_names(&self) -> &[WireString] {
        self.protocol.result_names()
    }
    pub fn query_uuid(&self) -> HugeInt {
        self.protocol.query_uuid()
    }

    /// Cancel an unfinished query, waiting at most two seconds for confirmation.
    /// Failure or interruption invalidates the session.
    pub async fn cancel(&mut self) -> Result<(), Error> {
        if self.done {
            return Ok(());
        }
        let mut operation = Operation {
            query: self,
            armed: true,
        };
        let result = operation.query.cancel_inner().await;
        if result.is_err() {
            operation.query.abandon();
        }
        operation.armed = false;
        result
    }

    async fn cancel_inner(&mut self) -> Result<(), Error> {
        self.core.state()?.check_query(&self.protocol)?;
        let request = self.protocol.cancel_request()?;
        let response = tokio::time::timeout(
            CLEANUP_TIMEOUT,
            exchange(&self.core.client, &self.core.endpoint, request),
        )
        .await??;
        self.core.state()?.check_query(&self.protocol)?;
        self.protocol.response(response).map_err(|error| -> Error {
            match error {
                state::QueryError::Remote(error) => Box::new(RemoteError(error)),
                error => Box::new(error),
            }
        })?;
        self.finish()
    }

    fn finish(&mut self) -> Result<(), Error> {
        let request = self.core.state()?.finish_query(&mut self.protocol)?;
        self.done = true;
        if request.is_some() {
            self.core.stop_heartbeat();
            self.core.cleanup(request);
        }
        Ok(())
    }
    fn abandon(&mut self) {
        if !self.done {
            self.protocol.fail();
            self.done = true;
            let request = self.core.invalidate();
            self.core.cleanup(request);
        }
    }
    async fn exchange(&mut self) -> Result<(), Error> {
        self.core.state()?.check_query(&self.protocol)?;
        let request = self.protocol.request()?;
        let cleanup = self.protocol.is_acknowledging();
        let result = match self.core.exchange(request).await {
            Ok(response) => {
                let active = self.core.state().and_then(|state| {
                    state
                        .check_query(&self.protocol)
                        .map_err(|error| Box::new(error) as Error)
                });
                match active {
                    Ok(()) => self.protocol.response(response).map_err(|error| -> Error {
                        match error {
                            state::QueryError::Remote(error) => Box::new(RemoteError(error)),
                            error => Box::new(error),
                        }
                    }),
                    Err(error) => {
                        self.protocol.fail();
                        Err(error)
                    }
                }
            }
            Err(error) => {
                self.protocol.fail();
                Err(error)
            }
        };
        if cleanup || result.is_err() || self.protocol.is_complete() {
            // A failed ACK affects session health, but not successful result consumption.
            self.finish()?;
        }
        if cleanup { Ok(()) } else { result }
    }
    /// Move one chunk out, fetching only when the current response has been drained.
    /// Errors are returned once; subsequent calls return EOF.
    pub async fn next_chunk(&mut self) -> Result<Option<DataChunk>, Error> {
        if self.done {
            return Ok(None);
        }
        let mut operation = Operation {
            query: self,
            armed: true,
        };
        let result = operation.query.next_chunk_inner().await;
        if result.is_err() {
            operation.query.abandon();
        }
        operation.armed = false;
        result
    }
    async fn next_chunk_inner(&mut self) -> Result<Option<DataChunk>, Error> {
        loop {
            self.core.state()?.check_query(&self.protocol)?;
            if let Some(chunk) = self.protocol.next_chunk()? {
                return Ok(Some(chunk));
            }
            self.exchange().await?;
            if self.done {
                return Ok(None);
            }
        }
    }
}
impl Drop for Query {
    fn drop(&mut self) {
        self.abandon();
    }
}
struct Operation<'a> {
    query: &'a mut Query,
    armed: bool,
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.query.abandon();
        }
    }
}

async fn heartbeat(core: Weak<Core>, timeout: Duration, sent: Instant) {
    let interval = timeout / 3;
    let mut renewed = sent;
    loop {
        let deadline = renewed + timeout;
        tokio::time::sleep_until((Instant::now() + interval).min(deadline)).await;
        let Some(core) = core.upgrade() else {
            break;
        };
        if Instant::now() >= deadline {
            core.invalidate();
            break;
        }
        let request = match core
            .state()
            .and_then(|state| state.heartbeat().map_err(|error| Box::new(error) as Error))
        {
            Ok(request) => request,
            Err(_) => break,
        };
        let sent = Instant::now();
        match tokio::time::timeout_at(deadline, exchange(&core.client, &core.endpoint, request))
            .await
        {
            Ok(Ok(Message {
                body: MessageBody::SuccessResponse(_),
                ..
            })) => renewed = sent,
            Ok(Err(error)) if error.is::<reqwest::Error>() || error.is::<std::io::Error>() => {}
            Ok(Ok(Message {
                body: MessageBody::ErrorResponse(error),
                ..
            })) if !error.must_invalidate => {}
            _ => {
                core.invalidate();
                break;
            }
        }
    }
}
