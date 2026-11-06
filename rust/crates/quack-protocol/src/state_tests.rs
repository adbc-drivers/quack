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
fn params(uuid: u64) -> QueryParams {
    QueryParams {
        query: "SELECT 42".into(),
        uuid: HugeInt {
            upper: 0,
            lower: uuid,
        },
        inline_rows: Some(0),
    }
}
fn fixture(text: &str) -> Message {
    let bytes = text
        .lines()
        .filter(|line| !line.starts_with('#'))
        .flat_map(str::split_whitespace)
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect::<Vec<_>>();
    crate::codec::decode(bytes).unwrap()
}
fn response(body: MessageBody) -> Message {
    Message {
        header: Header::default(),
        body,
    }
}
fn prepare(uuid: u64, inline: bool, more: bool) -> Message {
    let mut message = fixture(if inline {
        include_str!("../tests/fixtures/integer/select42_default_prepare.hex")
    } else {
        include_str!("../tests/fixtures/integer/select42_deferred_prepare.hex")
    });
    let MessageBody::PrepareResponse(body) = message.body else {
        panic!("prepare")
    };
    let mut bytes = body.into_body().as_bytes().to_vec();
    // Prepare fixture has UUID lower 1 or 2 as a one-byte varint at end.
    let len = bytes.len();
    bytes[len - 3] = uuid as u8;
    if let Some(position) = bytes.windows(3).position(|bytes| bytes == [3, 0, 1]) {
        bytes[position + 2] = u8::from(more);
    } else if more {
        let position = bytes.windows(2).position(|bytes| bytes == [4, 0]).unwrap();
        bytes.splice(position..position, [3, 0, 1]);
    }
    message.body = MessageBody::PrepareResponse(PrepareResponse::from_body(bytes.into()).unwrap());
    message
}
fn fetch(index: u64) -> Message {
    let mut message = fixture(include_str!(
        "../tests/fixtures/integer/select42_deferred_fetch_1.hex"
    ));
    let MessageBody::FetchResponse(ref mut body) = message.body else {
        panic!("fetch")
    };
    body.batch_index = Some(index);
    message
}
fn terminal(total: Option<u64>) -> Message {
    response(MessageBody::FetchResponse(FetchResponse {
        total_batches: total,
        ..Default::default()
    }))
}
fn ack(query: &mut Query) {
    assert!(matches!(
        query.request().unwrap().body,
        MessageBody::Acknowledgement(_)
    ));
    assert!(query.is_acknowledging());
    assert!(!query.is_complete());
    query
        .response(response(MessageBody::SuccessResponse(SuccessResponse)))
        .unwrap();
    assert!(query.is_complete());
}

#[test]
fn inline_requires_delivery_and_ack_before_release() {
    let mut connection = connected();
    let mut query = connection.prepare(params(1)).unwrap();
    assert!(query.next_chunk().is_err());
    assert!(query.response(prepare(1, true, false)).is_err());
    let request = query.request().unwrap();
    assert_eq!(request.header.connection_id, WireString::from("session"));
    assert!(query.request().is_err());
    query.response(prepare(1, true, false)).unwrap();
    assert_eq!(query.result_names(), &[WireString::from("42")]);
    assert_eq!(query.result_types().len(), 1);
    assert!(query.request().is_err());
    assert!(connection.prepare(params(2)).is_err());
    assert!(connection.heartbeat().is_ok());
    let chunk = query.next_chunk().unwrap().unwrap();
    assert_eq!(chunk.columns()[0].get(0), Some(Some(42)));
    assert!(query.next_chunk().unwrap().is_none());
    ack(&mut query);
    assert!(connection.finish_query(&mut query).unwrap().is_none());
    assert!(query.request().is_err());
    assert!(connection.finish_query(&mut query).is_err());
    assert!(connection.prepare(params(2)).is_ok());
}

#[test]
fn sequential_fetch_and_ack_watermarks_exclude_inline() {
    let mut connection = connected();
    let mut query = connection.prepare(params(1)).unwrap();
    query.request().unwrap();
    query.response(prepare(1, true, true)).unwrap();
    query.next_chunk().unwrap().unwrap();
    for index in 1..=2 {
        let MessageBody::FetchRequest(request) = query.request().unwrap().body else {
            panic!("fetch")
        };
        assert_eq!(request.batch_index, index);
        assert_eq!(request.ack_index, index - 1);
        assert_eq!(request.query_uuid, query.query_uuid());
        assert!(query.request().is_err());
        assert!(query.next_chunk().is_err());
        query.response(fetch(index)).unwrap();
        assert!(query.request().is_err());
        query.next_chunk().unwrap().unwrap();
    }
    let MessageBody::FetchRequest(request) = query.request().unwrap().body else {
        panic!("fetch")
    };
    assert_eq!((request.batch_index, request.ack_index), (3, 2));
    query.response(terminal(Some(2))).unwrap();
    ack(&mut query);
    connection.finish_query(&mut query).unwrap();
}

#[test]
fn multiple_chunks_are_delivered_before_next_fetch() {
    let mut connection = connected();
    let mut query = connection.prepare(params(2)).unwrap();
    query.request().unwrap();
    query.response(prepare(2, false, true)).unwrap();
    query.request().unwrap();
    query
        .response(fixture(include_str!(
            "../tests/fixtures/integer/fetch_rows_fetch_1.hex"
        )))
        .unwrap();
    for rows in [2048, 2048, 904] {
        assert!(query.request().is_err());
        let chunk = query.next_chunk().unwrap().unwrap();
        assert_eq!(chunk.rows(), rows);
    }
    query.request().unwrap();
    query.response(terminal(Some(1))).unwrap();
    ack(&mut query);
    connection.finish_query(&mut query).unwrap();
}

#[test]
fn empty_fetch_accepts_zero_or_absent_total() {
    for total in [Some(0), None] {
        let mut connection = connected();
        let mut query = connection.prepare(params(2)).unwrap();
        query.request().unwrap();
        query.response(prepare(2, false, true)).unwrap();
        query.request().unwrap();
        query.response(terminal(total)).unwrap();
        assert!(query.next_chunk().unwrap().is_none());
        ack(&mut query);
        connection.finish_query(&mut query).unwrap();
    }
}

#[test]
fn invalid_responses_fail_and_disconnect() {
    for bad in [
        fetch(2),
        terminal(Some(1)),
        response(MessageBody::SuccessResponse(SuccessResponse)),
    ] {
        let mut connection = connected();
        let mut query = connection.prepare(params(2)).unwrap();
        query.request().unwrap();
        query.response(prepare(2, false, true)).unwrap();
        query.request().unwrap();
        assert!(query.response(bad).is_err());
        assert!(query.is_failed());
        assert!(query.request().is_err());
        assert!(connection.finish_query(&mut query).unwrap().is_some());
        assert!(connection.prepare(params(3)).is_err());
    }
    let mut connection = connected();
    let mut query = connection.prepare(params(2)).unwrap();
    query.request().unwrap();
    assert!(query.response(prepare(1, true, false)).is_err());
    assert!(connection.finish_query(&mut query).unwrap().is_some());
}

#[test]
fn schema_and_malformed_payload_fail_before_acknowledgement() {
    for malformed in [false, true] {
        let mut connection = connected();
        let mut query = connection.prepare(params(2)).unwrap();
        query.request().unwrap();
        query.response(prepare(2, false, true)).unwrap();
        query.request().unwrap();
        let mut message = if malformed {
            fetch(1)
        } else {
            fixture(include_str!(
                "../tests/fixtures/integer/columns_prepare.hex"
            ))
        };
        if malformed {
            let MessageBody::FetchResponse(ref mut body) = message.body else {
                panic!("fetch")
            };
            body.payload = vec![0].into();
        } else {
            let MessageBody::PrepareResponse(body) = message.body else {
                panic!("prepare")
            };
            // Reuse the two-column chunk from inline wrapper as a FETCH payload.
            let raw = body.body().as_bytes();
            let start = raw
                .windows(3)
                .position(|bytes| bytes == [100, 0, 1])
                .unwrap();
            let (_, consumed) =
                duckdb_wiredata::chunk::decode(body.body().bytes().slice(start..)).unwrap();
            message.body = MessageBody::FetchResponse(FetchResponse {
                chunk_count: 1,
                batch_index: Some(1),
                payload: body.body().bytes().slice(start..start + consumed).into(),
                ..Default::default()
            });
        }
        assert!(matches!(
            query.response(message),
            Err(QueryError::Decode(_))
        ));
        assert!(connection.finish_query(&mut query).unwrap().is_some());
    }
}

#[test]
fn remote_error_release_depends_on_invalidation() {
    for invalidate in [false, true] {
        let mut connection = connected();
        let mut query = connection.prepare(params(2)).unwrap();
        assert!(query.remote_error(invalidate).is_err());
        query.request().unwrap();
        let error = ErrorResponse {
            message: "bad SQL".into(),
            must_invalidate: invalidate,
            ..Default::default()
        };
        assert!(matches!(
            query.response(response(MessageBody::ErrorResponse(error))),
            Err(QueryError::Remote(_))
        ));
        assert_eq!(
            connection.finish_query(&mut query).unwrap().is_some(),
            invalidate
        );
        assert_eq!(connection.prepare(params(3)).is_ok(), !invalidate);
    }
}

#[test]
fn recoverable_ack_error_completes_but_invalidation_disconnects() {
    for invalidate in [false, true] {
        let mut connection = connected();
        let mut query = connection.prepare(params(1)).unwrap();
        query.request().unwrap();
        query.response(prepare(1, true, false)).unwrap();
        query.next_chunk().unwrap();
        query.request().unwrap();
        assert!(query.is_acknowledging());
        query.remote_error(invalidate).unwrap();
        assert_eq!(
            connection.finish_query(&mut query).unwrap().is_some(),
            invalidate
        );
    }
}

#[test]
fn unfinished_and_failed_executions_disconnect() {
    for failed in [false, true] {
        let mut connection = connected();
        let mut query = connection.prepare(params(2)).unwrap();
        if failed {
            query.fail();
        }
        assert!(connection.finish_query(&mut query).unwrap().is_some());
        assert!(query.request().is_err());
        assert!(connection.heartbeat().is_err());
    }
}

#[test]
fn wrong_connection_stale_and_late_returns_cannot_release_execution() {
    let mut first = connected();
    let mut second = connected();
    let mut query = first.prepare(params(1)).unwrap();
    assert!(second.finish_query(&mut query).is_err());
    query.request().unwrap();
    query.remote_error(false).unwrap();
    first.finish_query(&mut query).unwrap();
    let mut newer = first.prepare(params(2)).unwrap();
    assert!(first.finish_query(&mut query).is_err());
    assert!(first.check_query(&newer).is_ok());
    first.disconnect();
    assert!(first.check_query(&newer).is_err());
    assert!(first.finish_query(&mut newer).unwrap().is_none());
    assert!(newer.request().is_err());
}

#[test]
fn zero_uuid_is_rejected_without_claiming_connection() {
    let mut connection = connected();
    assert!(connection.prepare(params(0)).is_err());
    assert!(connection.prepare(params(1)).is_ok());
}
