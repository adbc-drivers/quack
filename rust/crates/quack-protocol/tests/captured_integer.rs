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

use quack_protocol::{codec, message::MessageBody};

fn bytes(fixture: &str) -> Vec<u8> {
    fixture
        .lines()
        .filter(|line| !line.starts_with('#'))
        .flat_map(str::split_whitespace)
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect()
}

#[test]
fn deferred_prepare_preserves_schema() {
    let wire = bytes(include_str!(
        "fixtures/integer/select42_deferred_prepare.hex"
    ));
    let message = codec::decode(wire.clone()).unwrap();
    let MessageBody::PrepareResponse(response) = &message.body else {
        panic!("expected PREPARE response");
    };
    assert_eq!(response.result_names(), &["42".into()]);
    assert_eq!(response.result_types().len(), 1);
    assert!(response.needs_more_fetch());
    assert_eq!(response.query_uuid().lower, 2);
    assert_eq!(codec::encode(&message).unwrap().to_vec(), wire);
}

#[test]
fn fetched_payload_and_terminal_round_trip() {
    for (fixture, chunks, index, total) in [
        (
            include_str!("fixtures/integer/select42_deferred_fetch_1.hex"),
            1,
            Some(1),
            None,
        ),
        (
            include_str!("fixtures/integer/select42_deferred_fetch_2.hex"),
            0,
            None,
            Some(1),
        ),
        (
            include_str!("fixtures/integer/fetch_rows_fetch_1.hex"),
            3,
            Some(1),
            None,
        ),
        (
            include_str!("fixtures/integer/fetch_rows_fetch_2.hex"),
            0,
            None,
            Some(1),
        ),
    ] {
        let wire = bytes(fixture);
        let message = codec::decode(wire.clone()).unwrap();
        let MessageBody::FetchResponse(response) = &message.body else {
            panic!("expected FETCH response");
        };
        assert_eq!(response.chunk_count, chunks);
        assert_eq!(response.batch_index, index);
        assert_eq!(response.total_batches, total);
        assert_eq!(response.payload.as_bytes().is_empty(), chunks == 0);
        assert_eq!(codec::encode(&message).unwrap().to_vec(), wire);
    }
}
