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

//! Tokio connection orchestration over HTTP.

use quack_protocol::{codec, message::*, state};
use std::{sync::Arc, time::Duration};
use tokio::{sync::Mutex, task::JoinHandle, time::Instant};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

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
            String::from_utf8_lossy(&self.0.exception_type.0),
            String::from_utf8_lossy(&self.0.message.0)
        )
    }
}

impl std::error::Error for RemoteError {}

async fn exchange(
    client: &reqwest::Client,
    endpoint: &reqwest::Url,
    request: Message,
) -> Result<Message, Error> {
    let bytes = codec::encode(&request)?.to_vec();
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
    Ok(codec::decode(response.bytes().await?.to_vec())?)
}

pub struct Connection {
    state: Arc<Mutex<state::Connection>>,
    client: reqwest::Client,
    endpoint: reqwest::Url,
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
            state: Arc::new(Mutex::new(state::Connection::new())),
            client,
            endpoint,
            heartbeat: None,
        };
        connection.connect(options.protocol).await?;
        Ok(connection)
    }

    async fn connect(&mut self, options: state::ConnectionOptions) -> Result<(), Error> {
        let request = self.state.lock().await.pre_connect(options)?;
        let sent = Instant::now();
        let response = match tokio::time::timeout(
            Duration::from_secs(30),
            exchange(&self.client, &self.endpoint, request),
        )
        .await
        .map_err(|error| -> Error { Box::new(error) })
        .and_then(|result| result)
        {
            Ok(response) => response,
            Err(error) => {
                self.state
                    .lock()
                    .await
                    .fatal_transport_error(error.as_ref());
                return Err(error);
            }
        };
        let body = match response.body {
            MessageBody::ConnectionResponse(body) => body,
            body => {
                self.state.lock().await.disconnect();
                return Err(match body {
                    MessageBody::ErrorResponse(error) => Box::new(RemoteError(error)),
                    _ => Box::new(state::Error("unexpected handshake response")),
                });
            }
        };
        let timeout = {
            let mut state = self.state.lock().await;
            state.connect(response.header, body)?;
            Duration::from_secs(state.heartbeat_timeout())
        };
        let state = Arc::clone(&self.state);
        let client = self.client.clone();
        let endpoint = self.endpoint.clone();
        self.heartbeat = Some(tokio::spawn(async move {
            let interval = timeout / 3;
            let mut renewed = sent;
            loop {
                let deadline = renewed + timeout;
                tokio::time::sleep_until((Instant::now() + interval).min(deadline)).await;
                if Instant::now() >= deadline {
                    state.lock().await.disconnect();
                    break;
                }
                let request = match state.lock().await.heartbeat() {
                    Ok(request) => request,
                    Err(_) => break,
                };
                let sent = Instant::now();
                match tokio::time::timeout_at(deadline, exchange(&client, &endpoint, request)).await
                {
                    Ok(Ok(Message {
                        body: MessageBody::SuccessResponse(_),
                        ..
                    })) => renewed = sent,
                    Ok(Ok(Message {
                        body: MessageBody::ErrorResponse(error),
                        ..
                    })) if error.must_invalidate => {
                        state.lock().await.disconnect();
                        break;
                    }
                    Err(_) => {
                        state.lock().await.disconnect();
                        break;
                    }
                    Ok(Err(error))
                        if error.is::<reqwest::Error>() || error.is::<std::io::Error>() => {}
                    Ok(Ok(Message {
                        body: MessageBody::ErrorResponse(_),
                        ..
                    })) => {}
                    _ => {
                        state.lock().await.disconnect();
                        break;
                    }
                }
            }
        }));
        Ok(())
    }

    pub async fn prepare(&mut self, params: state::QueryParams) -> Result<PrepareResponse, Error> {
        let mut query = self.state.lock().await.prepare(params)?;
        let request = query.request()?;
        let result = match tokio::time::timeout(
            Duration::from_secs(30),
            exchange(&self.client, &self.endpoint, request),
        )
        .await
        {
            Ok(Ok(Message {
                body: MessageBody::ErrorResponse(error),
                ..
            })) => {
                query.remote_error(error.must_invalidate);
                Err(Box::new(RemoteError(error)) as Error)
            }
            Ok(Ok(response)) => query.response(response).map_err(|e| Box::new(e) as Error),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(Box::new(error) as Error),
        };
        let disconnect = self.state.lock().await.finish_query(query)?;
        if let Some(request) = disconnect {
            if let Some(task) = self.heartbeat.take() {
                task.abort();
                let _ = task.await;
            }
            let cleanup = tokio::time::timeout(
                Duration::from_secs(2),
                exchange(&self.client, &self.endpoint, request),
            )
            .await;
            if result.is_ok() {
                cleanup??;
            }
        }
        result
    }

    pub async fn disconnect(&mut self) -> Result<(), Error> {
        if let Some(task) = self.heartbeat.take() {
            task.abort();
            let _ = task.await;
        }
        let request = {
            let mut state = self.state.lock().await;
            let request = state.pre_disconnect().ok();
            state.disconnect();
            request
        };
        if let Some(request) = request {
            let response = tokio::time::timeout(
                Duration::from_secs(2),
                exchange(&self.client, &self.endpoint, request),
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
        if let Some(task) = &self.heartbeat {
            task.abort();
        }
    }
}
