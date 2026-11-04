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

//! Deterministic HTTP orchestration tests; no external server required.
use quack_protocol::{QUACK_VERSION, codec, message::*, state};
use quack_tokio::{Connection, ConnectionOptions, TlsMode};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
    task::{JoinHandle, JoinSet},
};

enum Reply {
    Message(Message),
    Bytes(Vec<u8>),
    Hang,
    Close,
}
struct Server {
    endpoint: String,
    requests: Arc<Mutex<Vec<Message>>>,
    notify: Arc<Notify>,
    task: JoinHandle<()>,
}
impl Server {
    async fn new(handler: impl Fn(&Message) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/quack", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let notify = Arc::new(Notify::new());
        let records = Arc::clone(&requests);
        let signal = Arc::clone(&notify);
        let handler = Arc::new(handler);
        let task = tokio::spawn(async move {
            let mut children = JoinSet::new();
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let handler = Arc::clone(&handler);
                let records = Arc::clone(&records);
                let signal = Arc::clone(&signal);
                children.spawn(async move {
                    let mut bytes = Vec::new();
                    let (start, len) = loop {
                        let byte = stream.read_u8().await.unwrap();
                        bytes.push(byte);
                        if bytes.ends_with(b"\r\n\r\n") {
                            let text = std::str::from_utf8(&bytes).unwrap();
                            let len = text
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().unwrap())
                                })
                                .unwrap();
                            break (bytes.len(), len);
                        }
                    };
                    bytes.resize(start + len, 0);
                    stream.read_exact(&mut bytes[start..]).await.unwrap();
                    let message = codec::decode(bytes.split_off(start)).unwrap();
                    let reply = handler(&message);
                    records.lock().unwrap().push(message);
                    signal.notify_waiters();
                    let body = match reply {
                        Reply::Message(message) => codec::encode(&message).unwrap().into_vec(),
                        Reply::Bytes(bytes) => bytes,
                        Reply::Hang => {
                            std::future::pending::<()>().await;
                            unreachable!()
                        }
                        Reply::Close => return,
                    };
                    let header = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                });
            }
        });
        Self {
            endpoint,
            requests,
            notify,
            task,
        }
    }
    async fn wait_for(&self, id: u8, count: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let notification = self.notify.notified();
                if self
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|m| m.body.message_id() == id)
                    .count()
                    >= count
                {
                    return;
                }
                notification.await;
            }
        })
        .await
        .unwrap();
    }
    fn ids(&self) -> Vec<u8> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|m| m.body.message_id())
            .collect()
    }
    async fn connect(&self) -> Connection {
        Connection::new(ConnectionOptions {
            endpoint: self.endpoint.clone(),
            tls: TlsMode::Disabled,
            protocol: state::ConnectionOptions::default(),
        })
        .await
        .unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn message(body: MessageBody) -> Message {
    Message {
        header: Header::default(),
        body,
    }
}
fn success() -> Reply {
    Reply::Message(message(MessageBody::SuccessResponse(SuccessResponse)))
}
fn handshake(timeout: u64) -> Reply {
    Reply::Message(Message {
        header: Header {
            connection_id: "session".into(),
            client_query_id: None,
        },
        body: MessageBody::ConnectionResponse(ConnectionResponse {
            quack_version: QUACK_VERSION,
            heartbeat_timeout_seconds: timeout,
            ..Default::default()
        }),
    })
}
fn fixture(text: &str) -> Message {
    codec::decode(
        text.lines()
            .filter(|l| !l.starts_with('#'))
            .flat_map(str::split_whitespace)
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect::<Vec<_>>(),
    )
    .unwrap()
}
fn prepare(uuid: HugeInt, inline: bool) -> Reply {
    let MessageBody::PrepareResponse(body) = fixture(if inline {
        include_str!("../../quack-protocol/tests/fixtures/integer/select42_default_prepare.hex")
    } else {
        include_str!("../../quack-protocol/tests/fixtures/integer/select42_deferred_prepare.hex")
    })
    .body
    else {
        panic!("prepare")
    };
    let mut bytes = body.into_body().as_bytes().to_vec();
    let position = bytes.len() - 3;
    bytes[position] = uuid.lower as u8;
    Reply::Message(message(MessageBody::PrepareResponse(
        PrepareResponse::from_body(bytes.into()).unwrap(),
    )))
}
fn data() -> Reply {
    Reply::Message(fixture(include_str!(
        "../../quack-protocol/tests/fixtures/integer/select42_deferred_fetch_1.hex"
    )))
}
fn terminal() -> Reply {
    Reply::Message(message(MessageBody::FetchResponse(FetchResponse {
        total_batches: Some(1),
        ..Default::default()
    })))
}
fn remote(invalidate: bool) -> Reply {
    Reply::Message(message(MessageBody::ErrorResponse(ErrorResponse {
        message: "query failed".into(),
        must_invalidate: invalidate,
        ..Default::default()
    })))
}
fn params(uuid: u64) -> state::QueryParams {
    state::QueryParams {
        query: "SELECT 42".into(),
        uuid: HugeInt {
            upper: 0,
            lower: uuid,
        },
        inline_rows: Some(0),
    }
}

#[tokio::test]
async fn sequential_fetch_ack_and_reuse() {
    let server = Server::new(|request| match &request.body {
        MessageBody::ConnectionRequest(_) => handshake(60),
        MessageBody::PrepareRequest(p) => prepare(p.query_uuid, false),
        MessageBody::FetchRequest(f) if f.batch_index == 1 => {
            assert_eq!(f.ack_index, 0);
            data()
        }
        MessageBody::FetchRequest(f) => {
            assert_eq!((f.batch_index, f.ack_index), (2, 1));
            terminal()
        }
        _ => success(),
    })
    .await;
    let mut connection = server.connect().await;
    let mut query = connection.execute(params(2)).await.unwrap();
    assert_eq!(server.ids(), [1, 3]);
    assert!(connection.execute(params(3)).await.is_err());
    assert_eq!(
        query.next_chunk().await.unwrap().unwrap().columns()[0].get(0),
        Some(Some(42))
    );
    assert_eq!(server.ids(), [1, 3, 7]);
    assert!(query.next_chunk().await.unwrap().is_none());
    assert_eq!(server.ids(), [1, 3, 7, 7, 15]);
    assert!(query.next_chunk().await.unwrap().is_none());
    let mut next = connection.execute(params(3)).await.unwrap();
    assert!(next.next_chunk().await.unwrap().is_some());
    assert!(next.next_chunk().await.unwrap().is_none());
    connection.disconnect().await.unwrap();
}

#[tokio::test]
async fn inline_chunks_precede_fetched_chunks() {
    let server = Server::new(|request| match &request.body {
        MessageBody::ConnectionRequest(_) => handshake(60),
        MessageBody::PrepareRequest(p) => {
            let MessageBody::PrepareResponse(body) = fixture(include_str!(
                "../../quack-protocol/tests/fixtures/integer/select42_inline_prepare.hex"
            ))
            .body
            else {
                panic!("prepare")
            };
            let mut bytes = body.into_body().as_bytes().to_vec();
            let position = bytes.len() - 3;
            bytes[position] = p.query_uuid.lower as u8;
            Reply::Message(message(MessageBody::PrepareResponse(
                PrepareResponse::from_body(bytes.into()).unwrap(),
            )))
        }
        MessageBody::FetchRequest(f) if f.batch_index == 1 => {
            assert_eq!(f.ack_index, 0);
            data()
        }
        MessageBody::FetchRequest(f) => {
            assert_eq!((f.batch_index, f.ack_index), (2, 1));
            terminal()
        }
        _ => success(),
    })
    .await;
    let mut connection = server.connect().await;
    let mut query = connection.execute(params(3)).await.unwrap();
    let inline = query.next_chunk().await.unwrap().unwrap();
    assert_eq!(inline.columns()[0].get(0), Some(Some(42)));
    assert_eq!(server.ids(), [1, 3]);
    let fetched = query.next_chunk().await.unwrap().unwrap();
    assert_eq!(fetched.columns()[0].get(0), Some(Some(42)));
    assert_eq!(server.ids(), [1, 3, 7]);
    assert!(query.next_chunk().await.unwrap().is_none());
    assert_eq!(server.ids(), [1, 3, 7, 7, 15]);
    connection.disconnect().await.unwrap();
    drop(query);
    drop(connection);
    assert_eq!(inline.columns()[0].get(0), Some(Some(42)));
    assert_eq!(fetched.columns()[0].get(0), Some(Some(42)));
}

#[tokio::test]
async fn multiple_chunks_do_not_prefetch() {
    let server = Server::new(|request| match &request.body {
        MessageBody::ConnectionRequest(_) => handshake(60),
        MessageBody::PrepareRequest(p) => prepare(p.query_uuid, false),
        MessageBody::FetchRequest(f) if f.batch_index == 1 => Reply::Message(fixture(
            include_str!("../../quack-protocol/tests/fixtures/integer/fetch_rows_fetch_1.hex"),
        )),
        MessageBody::FetchRequest(_) => terminal(),
        _ => success(),
    })
    .await;
    let mut connection = server.connect().await;
    let mut query = connection.execute(params(2)).await.unwrap();
    for expected in [2048, 2048, 904] {
        assert_eq!(query.next_chunk().await.unwrap().unwrap().rows(), expected);
        assert_eq!(server.ids(), [1, 3, 7]);
    }
    assert!(query.next_chunk().await.unwrap().is_none());
    connection.disconnect().await.unwrap();
}

#[tokio::test]
async fn dropping_query_and_explicit_disconnect_invalidate_handles() {
    let server = Server::new(|request| match &request.body {
        MessageBody::ConnectionRequest(_) => handshake(60),
        MessageBody::PrepareRequest(p) => prepare(p.query_uuid, true),
        _ => success(),
    })
    .await;
    let mut connection = server.connect().await;
    let query = connection.execute(params(1)).await.unwrap();
    drop(query);
    assert!(connection.execute(params(2)).await.is_err());
    server.wait_for(11, 1).await;
    let mut connection = server.connect().await;
    let mut query = connection.execute(params(1)).await.unwrap();
    connection.disconnect().await.unwrap();
    assert!(query.next_chunk().await.is_err());
    assert!(query.next_chunk().await.unwrap().is_none());
}

#[tokio::test]
async fn cancelling_prepare_disconnects_immediately() {
    let server = Server::new(|request| match request.body {
        MessageBody::ConnectionRequest(_) => handshake(60),
        MessageBody::PrepareRequest(_) => Reply::Hang,
        _ => success(),
    })
    .await;
    let mut connection = server.connect().await;
    let mut execution = Box::pin(connection.execute(params(2)));
    tokio::select! { _ = &mut execution => panic!("request unexpectedly finished"), _ = server.wait_for(3,1) => {} }
    drop(execution);
    assert!(connection.execute(params(3)).await.is_err());
    server.wait_for(11, 1).await;
}

#[tokio::test]
async fn cancelling_fetch_or_ack_disconnects_immediately() {
    for inline in [false, true] {
        let server = Server::new(move |request| match &request.body {
            MessageBody::ConnectionRequest(_) => handshake(60),
            MessageBody::PrepareRequest(p) => prepare(p.query_uuid, inline),
            MessageBody::FetchRequest(_) | MessageBody::Acknowledgement(_) => Reply::Hang,
            _ => success(),
        })
        .await;
        let mut connection = server.connect().await;
        let mut query = connection.execute(params(1)).await.unwrap();
        if inline {
            assert!(query.next_chunk().await.unwrap().is_some());
        }
        let mut fetch = Box::pin(query.next_chunk());
        tokio::select! { _ = &mut fetch => panic!("request unexpectedly finished"), _ = server.wait_for(if inline {15} else {7},1) => {} }
        drop(fetch);
        assert!(query.next_chunk().await.unwrap().is_none());
        assert!(connection.execute(params(2)).await.is_err());
        server.wait_for(11, 1).await;
    }
}

#[tokio::test]
async fn errors_once_and_recoverable_remote_errors_release_session() {
    for invalidate in [false, true] {
        let server = Server::new(move |request| match &request.body {
            MessageBody::ConnectionRequest(_) => handshake(60),
            MessageBody::PrepareRequest(p) => prepare(p.query_uuid, false),
            MessageBody::FetchRequest(_) => remote(invalidate),
            _ => success(),
        })
        .await;
        let mut connection = server.connect().await;
        let mut query = connection.execute(params(2)).await.unwrap();
        assert!(
            query
                .next_chunk()
                .await
                .unwrap_err()
                .is::<quack_tokio::RemoteError>()
        );
        assert!(query.next_chunk().await.unwrap().is_none());
        let next = connection.execute(params(3)).await;
        assert_eq!(next.is_err(), invalidate);
        drop(next);
    }
}

#[tokio::test]
async fn malformed_and_transport_fetch_errors_disconnect() {
    for close in [false, true] {
        let server = Server::new(move |request| match &request.body {
            MessageBody::ConnectionRequest(_) => handshake(60),
            MessageBody::PrepareRequest(p) => prepare(p.query_uuid, false),
            MessageBody::FetchRequest(_) => {
                if close {
                    Reply::Close
                } else {
                    Reply::Bytes(vec![0])
                }
            }
            _ => success(),
        })
        .await;
        let mut connection = server.connect().await;
        let mut query = connection.execute(params(2)).await.unwrap();
        assert!(query.next_chunk().await.is_err());
        assert!(query.next_chunk().await.unwrap().is_none());
        assert!(connection.execute(params(3)).await.is_err());
    }
}

#[tokio::test]
async fn acknowledgement_failure_is_best_effort() {
    for mode in 0..4 {
        let server = Server::new(move |request| match &request.body {
            MessageBody::ConnectionRequest(_) => handshake(60),
            MessageBody::PrepareRequest(p) => prepare(p.query_uuid, true),
            MessageBody::Acknowledgement(_) => match mode {
                0 => remote(false),
                1 => remote(true),
                2 => Reply::Close,
                _ => Reply::Bytes(vec![0]),
            },
            _ => success(),
        })
        .await;
        let mut connection = server.connect().await;
        let mut query = connection.execute(params(1)).await.unwrap();
        assert!(query.next_chunk().await.unwrap().is_some());
        assert!(query.next_chunk().await.unwrap().is_none());
        let next = connection.execute(params(2)).await;
        assert_eq!(next.is_ok(), mode == 0);
        drop(next);
    }
}

#[tokio::test]
async fn heartbeat_continues_while_query_is_busy() {
    let server = Server::new(|request| match &request.body {
        MessageBody::ConnectionRequest(_) => handshake(1),
        MessageBody::PrepareRequest(p) => prepare(p.query_uuid, true),
        _ => success(),
    })
    .await;
    let mut connection = server.connect().await;
    let mut query = connection.execute(params(1)).await.unwrap();
    server.wait_for(16, 1).await;
    assert!(query.next_chunk().await.unwrap().is_some());
    assert!(query.next_chunk().await.unwrap().is_none());
    connection.disconnect().await.unwrap();
}

#[tokio::test]
async fn fetch_timeout_disconnects() {
    let server = Server::new(|request| match &request.body {
        MessageBody::ConnectionRequest(_) => handshake(60),
        MessageBody::PrepareRequest(p) => prepare(p.query_uuid, false),
        MessageBody::FetchRequest(_) => Reply::Hang,
        _ => success(),
    })
    .await;
    let mut connection = server.connect().await;
    let mut query = connection.execute(params(2)).await.unwrap();
    let mut fetch = Box::pin(query.next_chunk());
    tokio::select! { _ = &mut fetch => panic!("request unexpectedly finished"), _ = server.wait_for(7,1) => {} }
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(fetch.await.unwrap_err().is::<tokio::time::error::Elapsed>());
    assert!(query.next_chunk().await.unwrap().is_none());
    assert!(connection.execute(params(3)).await.is_err());
}
