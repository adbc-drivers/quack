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

use quack_protocol::codec::*;
use quack_protocol::message::*;

fn hex(value: &str) -> Vec<u8> {
    value
        .split_whitespace()
        .map(|v| u8::from_str_radix(v, 16).unwrap())
        .collect()
}

// Independent wire fixture header: required type and unset client_query_id.
fn wire(id: u8, body: &str) -> Vec<u8> {
    let mut bytes = hex("01 00 00 03 00 ff ff ff ff ff ff ff ff ff 01 ff ff");
    bytes[2] = id;
    bytes.extend(hex(body));
    bytes
}

fn message(body: MessageBody) -> Message {
    Message {
        header: Header::default(),
        body,
    }
}

// The companion document's two-row INTEGER chunk [42, NULL].
const CHUNK: &str = "64 00 02 65 00 01 64 00 0d ff ff 66 00 01
    64 00 01 65 00 08 fd ff ff ff ff ff ff ff
    66 00 08 2a 00 00 00 00 00 00 80 ff ff ff ff";

fn fixtures() -> Vec<(Message, Vec<u8>)> {
    vec![
        (
            message(MessageBody::ConnectionRequest(ConnectionRequest {
                auth_string: "x".into(),
                client_duckdb_version: "d".into(),
                client_platform: "p".into(),
                min_supported_quack_version: 3,
                max_supported_quack_version: 3,
                client_id: "c".into(),
                heartbeat_timeout_seconds: 60,
            })),
            wire(
                1,
                "01 00 01 78 02 00 01 64 03 00 01 70 04 00 03 05 00 03 06 00 01 63 07 00 3c ff ff",
            ),
        ),
        (
            message(MessageBody::ConnectionResponse(ConnectionResponse {
                server_duckdb_version: "d".into(),
                server_platform: "p".into(),
                quack_version: 3,
                heartbeat_timeout_seconds: 60,
            })),
            wire(2, "01 00 01 64 02 00 01 70 03 00 03 04 00 3c ff ff"),
        ),
        (
            message(MessageBody::PrepareRequest(PrepareRequest {
                sql_query: "q".into(),
                query_uuid: HugeInt {
                    upper: -1,
                    lower: 128,
                },
                inline_rows: Some(0),
            })),
            wire(3, "01 00 01 71 02 00 7f 80 01 03 00 00 ff ff"),
        ),
        (
            message(MessageBody::PrepareResponse(
                PrepareResponse::from_body(
                    hex("05 00 00 ff ff ff ff ff ff ff ff ff 01 ff ff").into(),
                )
                .unwrap(),
            )),
            wire(4, "05 00 00 ff ff ff ff ff ff ff ff ff 01 ff ff"),
        ),
        (
            message(MessageBody::FetchRequest(FetchRequest {
                query_uuid: HugeInt {
                    upper: -65,
                    lower: 1,
                },
                batch_index: 128,
                ack_index: 127,
            })),
            wire(7, "01 00 bf 7f 01 02 00 80 01 03 00 7f ff ff"),
        ),
        (
            message(MessageBody::FetchResponse(FetchResponse {
                chunk_count: 1,
                batch_index: Some(1),
                payload: hex(CHUNK).into(),
                ..Default::default()
            })),
            wire(8, &format!("01 00 01 03 00 01 ff ff {CHUNK}")),
        ),
        (
            message(MessageBody::SendDataRequest(SendDataRequest {
                stream_id: "s".into(),
                chunk_count: 1,
                total_batches: Some(2),
                batch_index: Some(1),
                payload: hex(CHUNK).into(),
            })),
            wire(
                9,
                &format!("01 00 01 73 02 00 01 03 00 02 04 00 01 ff ff {CHUNK}"),
            ),
        ),
        (
            message(MessageBody::SuccessResponse(SuccessResponse)),
            wire(10, "ff ff"),
        ),
        (
            message(MessageBody::DisconnectMessage(DisconnectMessage)),
            wire(11, "ff ff"),
        ),
        (
            message(MessageBody::CancelRequest(CancelRequest {
                query_uuid: HugeInt {
                    upper: 64,
                    lower: 0,
                },
            })),
            wire(12, "01 00 c0 00 00 ff ff"),
        ),
        (
            message(MessageBody::SendDataResponse(SendDataResponse {
                accept_budget: Some(0),
            })),
            wire(14, "01 00 00 ff ff"),
        ),
        (
            message(MessageBody::Acknowledgement(Acknowledgement {
                query_uuid: HugeInt { upper: 0, lower: 1 },
            })),
            wire(15, "01 00 00 01 ff ff"),
        ),
        (
            message(MessageBody::HeartbeatRequest(HeartbeatRequest)),
            wire(16, "ff ff"),
        ),
        (
            message(MessageBody::ErrorResponse(ErrorResponse {
                message: vec![0xff, 0].into(),
                exception_type: "E".into(),
                extra_info: vec![("k".into(), "v".into()), ("k".into(), "w".into())],
                must_invalidate: true,
            })),
            wire(
                100,
                "01 00 02 ff 00 02 00 01 45 03 00 02
            00 00 01 6b 01 00 01 76 ff ff
            00 00 01 6b 01 00 01 77 ff ff 04 00 01 ff ff",
            ),
        ),
    ]
}

#[test]
fn all_message_kinds_match_independent_fixtures() {
    for (expected, bytes) in fixtures() {
        assert_eq!(encode(&expected).unwrap().to_vec(), bytes, "{expected:?}");
        let decoded = decode(bytes.clone()).unwrap();
        assert_eq!(decoded, expected);
        assert_eq!(encode(&decoded).unwrap().to_vec(), bytes);
    }
}

#[test]
fn documented_connection_request() {
    let bytes = hex("01 00 01 03 00 ff ff ff ff ff ff ff ff ff 01 ff ff
        01 00 01 78 04 00 03 05 00 03 07 00 3c ff ff");
    assert_eq!(bytes.len(), 32);
    let expected = message(MessageBody::ConnectionRequest(ConnectionRequest {
        auth_string: "x".into(),
        min_supported_quack_version: 3,
        max_supported_quack_version: 3,
        heartbeat_timeout_seconds: 60,
        ..Default::default()
    }));
    assert_eq!(decode(bytes.clone()).unwrap(), expected);
    assert_eq!(encode(&expected).unwrap().to_vec(), bytes);
}

#[test]
fn defaults_and_required_unset_fields() {
    let cases = [
        (
            message(MessageBody::ConnectionRequest(ConnectionRequest::default())),
            wire(1, "ff ff"),
        ),
        (
            message(MessageBody::ConnectionResponse(
                ConnectionResponse::default(),
            )),
            wire(2, "ff ff"),
        ),
        (
            message(MessageBody::PrepareRequest(PrepareRequest::default())),
            wire(3, "02 00 00 00 ff ff"),
        ),
        (
            message(MessageBody::FetchRequest(FetchRequest::default())),
            wire(7, "01 00 00 00 ff ff"),
        ),
        (
            message(MessageBody::FetchResponse(FetchResponse::default())),
            wire(8, "ff ff"),
        ),
        (
            message(MessageBody::SendDataRequest(SendDataRequest::default())),
            wire(9, "ff ff"),
        ),
        (
            message(MessageBody::CancelRequest(CancelRequest::default())),
            wire(12, "01 00 00 00 ff ff"),
        ),
        (
            message(MessageBody::SendDataResponse(SendDataResponse::default())),
            wire(14, "01 00 ff ff ff ff ff ff ff ff ff 01 ff ff"),
        ),
        (
            message(MessageBody::Acknowledgement(Acknowledgement::default())),
            wire(15, "ff ff"),
        ),
        (
            message(MessageBody::ErrorResponse(ErrorResponse::default())),
            wire(100, "ff ff"),
        ),
    ];
    for (expected, bytes) in cases {
        assert_eq!(decode(bytes.clone()).unwrap(), expected);
        assert_eq!(encode(&expected).unwrap().to_vec(), bytes);
    }
    // Explicit default properties are accepted, then canonically omitted.
    let explicit = decode(wire(
        1,
        "01 00 00 02 00 00 03 00 00 04 00 00 05 00 00 06 00 00 07 00 00 ff ff",
    ))
    .unwrap();
    assert_eq!(encode(&explicit).unwrap().to_vec(), wire(1, "ff ff"));
    let unset = decode(wire(
        3,
        "02 00 00 00 03 00 ff ff ff ff ff ff ff ff ff 01 ff ff",
    ))
    .unwrap();
    assert_eq!(
        encode(&unset).unwrap().to_vec(),
        wire(3, "02 00 00 00 ff ff")
    );
}

#[test]
fn nondefault_header_and_terminal_counts() {
    let bytes = hex("01 00 08 02 00 02 00 ff 03 00 00 ff ff 02 00 00 ff ff");
    let expected = Message {
        header: Header {
            connection_id: vec![0, 0xff].into(),
            client_query_id: Some(0),
        },
        body: MessageBody::FetchResponse(FetchResponse {
            total_batches: Some(0),
            ..Default::default()
        }),
    };
    assert_eq!(decode(bytes.clone()).unwrap(), expected);
    assert_eq!(encode(&expected).unwrap().to_vec(), bytes);
    let upload = message(MessageBody::SendDataRequest(SendDataRequest {
        total_batches: Some(0),
        ..Default::default()
    }));
    assert_eq!(encode(&upload).unwrap().to_vec(), wire(9, "03 00 00 ff ff"));
}

#[test]
fn signed_leb128_boundaries() {
    for (value, encoded) in [
        (i64::MIN, "80 80 80 80 80 80 80 80 80 7f"),
        (-65, "bf 7f"),
        (-64, "40"),
        (-1, "7f"),
        (0, "00"),
        (63, "3f"),
        (64, "c0 00"),
        (i64::MAX, "ff ff ff ff ff ff ff ff ff 00"),
    ] {
        let bytes = wire(12, &format!("01 00 {encoded} 00 ff ff"));
        let expected = message(MessageBody::CancelRequest(CancelRequest {
            query_uuid: HugeInt {
                upper: value,
                lower: 0,
            },
        }));
        assert_eq!(decode(bytes.clone()).unwrap(), expected);
        assert_eq!(encode(&expected).unwrap().to_vec(), bytes);
    }
}

#[test]
fn unsigned_leb128_boundaries() {
    for (value, encoded) in [
        (0, "00"),
        (127, "7f"),
        (128, "80 01"),
        (u64::MAX, "ff ff ff ff ff ff ff ff ff 01"),
    ] {
        let bytes = wire(12, &format!("01 00 00 {encoded} ff ff"));
        let expected = message(MessageBody::CancelRequest(CancelRequest {
            query_uuid: HugeInt {
                upper: 0,
                lower: value,
            },
        }));
        assert_eq!(decode(bytes.clone()).unwrap(), expected);
        assert_eq!(encode(&expected).unwrap().to_vec(), bytes);
    }
}

fn payload(message: &Message) -> &[u8] {
    match &message.body {
        MessageBody::PrepareResponse(v) => v.body().as_bytes(),
        MessageBody::FetchResponse(v) => v.payload.as_bytes(),
        MessageBody::SendDataRequest(v) => v.payload.as_bytes(),
        _ => &[],
    }
}

#[test]
fn opaque_payloads_reuse_the_input_allocation_and_encode_by_borrowing() {
    for (expected, bytes) in fixtures() {
        if payload(&expected).is_empty() {
            continue;
        }
        for spare in [0, 4096] {
            let mut input = Vec::with_capacity(bytes.len() + spare);
            input.extend_from_slice(&bytes);
            let original = input.as_ptr();
            let offset = bytes.len() - payload(&expected).len();
            let decoded = decode(input).unwrap();
            assert_eq!(payload(&decoded).as_ptr(), original.wrapping_add(offset));
            assert_eq!(payload(&decoded), payload(&expected));
            let encoded = encode(&decoded).unwrap();
            assert_eq!(encoded.payload.as_ptr(), payload(&decoded).as_ptr());
            assert_eq!(encoded.to_vec(), bytes);
            // The field owns its allocation independently of its enclosing message.
            let owned = match decoded.body {
                MessageBody::PrepareResponse(v) => v.into_body(),
                MessageBody::FetchResponse(v) => v.payload,
                MessageBody::SendDataRequest(v) => v.payload,
                _ => unreachable!(),
            };
            assert_eq!(owned.as_bytes().as_ptr(), original.wrapping_add(offset));
            assert_eq!(owned.as_bytes(), payload(&expected));
        }
    }
}

#[test]
fn opaque_equality_compares_visible_bytes() {
    let decoded = decode(wire(4, "05 00 00 00 ff ff")).unwrap();
    let expected = message(MessageBody::PrepareResponse(
        PrepareResponse::from_body(hex("05 00 00 00 ff ff").into()).unwrap(),
    ));
    assert_eq!(decoded, expected);
}

#[test]
fn all_truncations_of_fully_parsed_messages_fail() {
    for (expected, bytes) in fixtures() {
        if !payload(&expected).is_empty() {
            continue;
        }
        for length in 0..bytes.len() {
            assert!(
                decode(bytes[..length].to_vec()).is_err(),
                "ID {}, length {length}",
                expected.body.message_id()
            );
        }
    }
}

#[test]
fn malformed_fields_and_lengths_fail() {
    for (id, body) in [
        (1, "08 00 00 ff ff"),          // unknown field
        (1, "02 00 00 01 00 00 ff ff"), // reordered fields
        (1, "01 00 00 01 00 00 ff ff"), // duplicate field
        (3, "ff ff"),                   // missing UUID
        (7, "ff ff"),
        (12, "ff ff"),
        (14, "ff ff"),           // required unset field cannot be omitted
        (100, "04 00 02 ff ff"), // invalid boolean
        (100, "03 00 ff ff ff ff ff ff ff ff ff 01 ff ff"), // malicious map count
        (100, "03 00 01 01 00 00 00 00 00 ff ff ff ff"), // map entry order
        (1, "01 00 ff ff ff ff 0f ff ff"), // length exceeds input
        (1, "01 00 80 80 80 80 10 ff ff"), // ULEB32 overflow
        (1, "01 00 80 80 80 80 80 00 ff ff"), // overlong ULEB32
        (12, "01 00 80 80 80 80 80 80 80 80 80 01 00 ff ff"), // signed overflow
        (12, "01 00 ff ff ff ff ff ff ff ff ff 7e 00 ff ff"), // negative overflow
        (12, "01 00 80 80 80 80 80 80 80 80 80 80 00 00 ff ff"), // overlong signed
        (12, "01 00 00 ff ff ff ff ff ff ff ff ff 02 ff ff"), // unsigned overflow
        (12, "01 00 00 80 80 80 80 80 80 80 80 80 80 00 ff ff"), // overlong unsigned
        (10, "ff ff 00"),        // trailing byte
        (10, "00 00"),           // wrong terminator
        (4, "05 00 00 00"),      // incomplete opaque body
        (8, "ff ff 01"),         // zero chunks with data
        (8, "01 00 01 ff ff"),   // chunks with no data
        (9, "ff ff 01"),
        (9, "02 00 01 ff ff"),
    ] {
        let error = decode(wire(id, body)).unwrap_err();
        assert!(!error.context.is_empty());
        assert!(!error.to_string().is_empty());
    }
    for id in [0, 5, 6, 13, 17, 99, 255] {
        assert!(decode(wire(id, "ff ff")).is_err());
    }
    assert!(decode(hex("01 00 80 02")).is_err()); // ULEB8 overflow
    assert!(decode(hex("01 00 01 ff ff ff ff")).is_err()); // missing header correlation field
    let error = decode(hex("01 00 0a 03 00 80")).unwrap_err();
    assert_eq!(error.offset, 6);
    assert_eq!(error.context, "header.client_query_id");
}

#[test]
fn invalid_encoding_inputs_fail() {
    let mut cases = vec![
        message(MessageBody::PrepareRequest(PrepareRequest {
            inline_rows: Some(u64::MAX),
            ..Default::default()
        })),
        message(MessageBody::SendDataResponse(SendDataResponse {
            accept_budget: Some(u64::MAX),
        })),
        message(MessageBody::FetchResponse(FetchResponse {
            total_batches: Some(u64::MAX),
            ..Default::default()
        })),
        message(MessageBody::FetchResponse(FetchResponse {
            batch_index: Some(u64::MAX),
            ..Default::default()
        })),
        message(MessageBody::SendDataRequest(SendDataRequest {
            total_batches: Some(u64::MAX),
            ..Default::default()
        })),
        message(MessageBody::SendDataRequest(SendDataRequest {
            batch_index: Some(u64::MAX),
            ..Default::default()
        })),
        message(MessageBody::FetchResponse(FetchResponse {
            chunk_count: 1,
            ..Default::default()
        })),
        message(MessageBody::SendDataRequest(SendDataRequest {
            payload: vec![1].into(),
            ..Default::default()
        })),
    ];
    let mut header_sentinel = message(MessageBody::SuccessResponse(SuccessResponse));
    header_sentinel.header.client_query_id = Some(u64::MAX);
    cases.push(header_sentinel);
    for value in cases {
        let error = encode(&value).unwrap_err();
        assert!(!error.context.is_empty());
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn wire_strings_preserve_arbitrary_bytes() {
    let string = WireString::from(vec![0xff, 0, 0xff]);
    assert_eq!(string.as_bytes(), &[0xff, 0, 0xff]);
    assert!(string.as_str().is_err());
    assert_eq!(
        WireString::from(String::from("text")).as_str().unwrap(),
        "text"
    );
}

#[test]
fn prepare_metadata_and_original_body() {
    let bytes = wire(
        4,
        "01 00 01 64 00 0d ff ff 02 00 01 01 78 03 00 01 04 00 00 05 00 00 2a ff ff",
    );
    let message = decode(bytes.clone()).unwrap();
    let MessageBody::PrepareResponse(response) = &message.body else {
        panic!("wrong response")
    };
    assert_eq!(
        response.result_types()[0].id,
        duckdb_wiredata::logical_type::LogicalTypeId::Integer
    );
    assert_eq!(response.result_names(), &[WireString::from("x")]);
    assert!(response.needs_more_fetch());
    assert_eq!(
        response.query_uuid(),
        HugeInt {
            upper: 0,
            lower: 42
        }
    );
    assert_eq!(encode(&message).unwrap().to_vec(), bytes);
    for end in 0..response.body().as_bytes().len() {
        assert!(
            PrepareResponse::from_body(response.body().as_bytes()[..end].to_vec().into()).is_err()
        );
    }
}

#[test]
fn prepare_defaults_and_rejected_results() {
    let response = PrepareResponse::from_body(hex("05 00 00 00 ff ff").into()).unwrap();
    assert!(response.result_types().is_empty());
    assert!(response.result_names().is_empty());
    assert!(!response.needs_more_fetch());
    for body in [
        "ff ff",
        "05 00 00 00 ff ff 00",
        "00",
        "01 00 ff ff ff ff ff ff ff ff ff 01",
    ] {
        assert!(PrepareResponse::from_body(hex(body).into()).is_err());
    }
    let error = decode(wire(4, "04 00 01")).unwrap_err();
    assert_eq!(error.context, "PrepareResponse.results");
    assert!(error.message.contains("nonempty inline results"));
}

#[test]
fn prepare_nested_error_offset_is_message_relative() {
    let body = hex("01 00 01 64 00 7f ff ff 05 00 00 00 ff ff");
    let local = PrepareResponse::from_body(body.clone().into()).unwrap_err();
    let mut bytes = wire(4, "");
    let header_len = bytes.len();
    bytes.extend(body);
    let global = decode(bytes).unwrap_err();
    assert_eq!(global.offset, header_len + local.offset);
    assert_eq!(global.context, "LogicalType.id");
}
