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

//! Explicit fixture regeneration; never runs as part of the normal test suite.
use quack_protocol::{QUACK_VERSION, codec, message::*};

async fn exchange(
    client: &reqwest::Client,
    endpoint: &str,
    connection: &WireString,
    body: MessageBody,
) -> Vec<u8> {
    let message = Message {
        header: Header {
            connection_id: connection.clone(),
            client_query_id: None,
        },
        body,
    };
    client
        .post(endpoint)
        .body(codec::encode(&message).unwrap().into_vec())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes()
        .await
        .unwrap()
        .to_vec()
}

#[tokio::test]
#[ignore = "explicitly regenerates captured fixtures from a configured live server"]
async fn capture_scalars() {
    let endpoint = std::env::var("QUACK_HTTP_ENDPOINT").unwrap();
    let client = reqwest::Client::new();
    let response = exchange(
        &client,
        &endpoint,
        &WireString::default(),
        MessageBody::ConnectionRequest(ConnectionRequest {
            auth_string: "quack-secret".into(),
            min_supported_quack_version: QUACK_VERSION,
            max_supported_quack_version: QUACK_VERSION,
            heartbeat_timeout_seconds: 60,
            ..Default::default()
        }),
    )
    .await;
    let response = codec::decode(response).unwrap();
    let connection = response.header.connection_id;
    let MessageBody::ConnectionResponse(server) = response.body else {
        panic!("handshake: {:?}", response.body)
    };
    let mixed = include_str!("../../quack-protocol/tests/fixtures/scalars/mixed.sql");
    let empty = format!("SELECT * FROM ({mixed}) q WHERE false");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../quack-protocol/tests/fixtures/scalars");
    for (index, (name, sql, inline)) in [
        ("mixed_inline", mixed, None),
        ("mixed_deferred", mixed, Some(0)),
        ("empty", empty.as_str(), None),
    ]
    .into_iter()
    .enumerate()
    {
        let uuid = HugeInt {
            upper: 0,
            lower: index as u64 + 100,
        };
        let metadata = format!(
            "# DuckDB: {}; platform: {}; Quack: {}\n# SQL: {}\n# inline_rows: {inline:?}; UUID: {uuid:?}\n",
            server.server_duckdb_version.as_str().unwrap(),
            server.server_platform.as_str().unwrap(),
            server.quack_version,
            sql.trim().replace('\n', " ")
        );
        let save = |suffix: &str, bytes: &[u8]| {
            let license = include_str!(
                "../../quack-protocol/tests/fixtures/integer/select42_default_prepare.hex"
            )
            .split("# Captured")
            .next()
            .unwrap();
            let mut text = format!(
                "{license}# Captured from live server, V2_0_0 little-endian; exact response bytes.\n{metadata}"
            );
            for line in bytes.chunks(32) {
                text.push_str(
                    &line
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                text.push('\n');
            }
            std::fs::write(root.join(format!("{name}_{suffix}.hex")), text).unwrap();
        };
        let bytes = exchange(
            &client,
            &endpoint,
            &connection,
            MessageBody::PrepareRequest(PrepareRequest {
                sql_query: sql.into(),
                query_uuid: uuid,
                inline_rows: inline,
            }),
        )
        .await;
        save("prepare", &bytes);
        let MessageBody::PrepareResponse(prepare) = codec::decode(bytes).unwrap().body else {
            panic!("prepare")
        };
        if prepare.needs_more_fetch() {
            for index in 1.. {
                let bytes = exchange(
                    &client,
                    &endpoint,
                    &connection,
                    MessageBody::FetchRequest(FetchRequest {
                        query_uuid: uuid,
                        batch_index: index,
                        ack_index: index - 1,
                    }),
                )
                .await;
                save(&format!("fetch_{index}"), &bytes);
                let MessageBody::FetchResponse(fetch) = codec::decode(bytes).unwrap().body else {
                    panic!("fetch")
                };
                fetch.decode_chunks().unwrap();
                if fetch.chunk_count == 0 {
                    break;
                }
            }
        }
        let ack = exchange(
            &client,
            &endpoint,
            &connection,
            MessageBody::Acknowledgement(Acknowledgement { query_uuid: uuid }),
        )
        .await;
        assert!(matches!(
            codec::decode(ack).unwrap().body,
            MessageBody::SuccessResponse(_)
        ));
    }
    exchange(
        &client,
        &endpoint,
        &connection,
        MessageBody::DisconnectMessage(DisconnectMessage),
    )
    .await;
}
