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
use duckdb_wiredata::{chunk::VectorEncoding, decode::DecodeErrorKind};
use quack_protocol::{codec, message::*};

fn fixture(text: &str) -> Vec<u8> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .flat_map(str::split_whitespace)
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect()
}
fn prepare(bytes: impl Into<Bytes>) -> PrepareResponse {
    let MessageBody::PrepareResponse(response) = codec::decode(bytes).unwrap().body else {
        panic!("expected PREPARE")
    };
    response
}
fn fetch(bytes: impl Into<Bytes>) -> FetchResponse {
    let MessageBody::FetchResponse(response) = codec::decode(bytes).unwrap().body else {
        panic!("expected FETCH")
    };
    response
}
fn default_wire() -> Vec<u8> {
    fixture(include_str!(
        "fixtures/integer/select42_default_prepare.hex"
    ))
}
fn deferred_wire() -> Vec<u8> {
    fixture(include_str!(
        "fixtures/integer/select42_deferred_fetch_1.hex"
    ))
}

#[test]
fn captured_inline_values_round_trip_and_transfer() {
    for (text, expected, more) in [
        (
            include_str!("fixtures/integer/select42_default_prepare.hex"),
            vec![vec![Some(42)]],
            false,
        ),
        (
            include_str!("fixtures/integer/select42_inline_prepare.hex"),
            vec![vec![Some(42)]],
            true,
        ),
        (
            include_str!("fixtures/integer/nullable_prepare.hex"),
            vec![vec![Some(42), None, Some(-7)]],
            false,
        ),
        (
            include_str!("fixtures/integer/columns_prepare.hex"),
            vec![vec![Some(42)], vec![Some(-7)]],
            false,
        ),
    ] {
        let bytes = fixture(text);
        let message = codec::decode(bytes.clone()).unwrap();
        assert_eq!(codec::encode(&message).unwrap().to_vec(), bytes);
        let MessageBody::PrepareResponse(response) = message.body else {
            panic!("expected PREPARE")
        };
        assert_eq!(response.needs_more_fetch(), more);
        assert_eq!(response.results().len(), 1);
        let parts = response.into_parts();
        assert_eq!(parts.result_names.len(), expected.len());
        let chunk = &parts.results[0];
        assert_eq!(chunk.columns().len(), expected.len());
        for (column, expected) in chunk.columns().iter().zip(expected) {
            assert_eq!(
                (0..column.len())
                    .map(|i| column.get(i).unwrap())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
    let empty = prepare(fixture(include_str!("fixtures/integer/empty_prepare.hex")));
    assert_eq!(empty.result_names(), &["answer".into()]);
    assert!(empty.results().is_empty());
}

#[test]
fn captured_fetch_values_multiple_chunks_and_terminal() {
    let response = fetch(deferred_wire());
    let chunks = response.decode_chunks().unwrap();
    assert_eq!(chunks[0].columns()[0].get(0), Some(Some(42)));
    let schema = prepare(fixture(include_str!(
        "fixtures/integer/select42_deferred_prepare.hex"
    )));
    schema.validate_chunk(&chunks[0]).unwrap();

    let response = fetch(fixture(include_str!(
        "fixtures/integer/fetch_rows_fetch_1.hex"
    )));
    let chunks = response.decode_chunks().unwrap();
    assert_eq!(
        chunks.iter().map(|c| c.rows()).collect::<Vec<_>>(),
        [2048, 2048, 904]
    );
    let mut value = 0;
    for chunk in chunks {
        for row in 0..chunk.rows() {
            assert_eq!(chunk.columns()[0].get(row), Some(Some(value)));
            value += 1;
        }
    }
    assert_eq!(value, 5000);
    let terminal = fetch(fixture(include_str!(
        "fixtures/integer/fetch_rows_fetch_2.hex"
    )));
    assert!(terminal.decode_chunks().unwrap().is_empty());
}

#[test]
fn inline_and_fetch_buffers_share_input_and_survive_owners() {
    for wire in [default_wire(), deferred_wire()] {
        let owner = Bytes::from(wire);
        let message = codec::decode(owner.clone()).unwrap();
        let chunks = match message.body {
            MessageBody::PrepareResponse(response) => response.into_parts().results,
            MessageBody::FetchResponse(response) => response.decode_chunks().unwrap(),
            _ => panic!("unexpected response"),
        };
        assert_eq!(chunks[0].types().len(), 1);
        let VectorEncoding::Flat { values, .. } = chunks[0].columns()[0].encoding() else {
            panic!("expected flat")
        };
        let start = owner.as_ptr() as usize;
        assert!((start..start + owner.len()).contains(&(values.as_ptr() as usize)));
        let value_ptr = values.as_ptr();
        drop(owner);
        assert_eq!(chunks[0].columns()[0].get(0), Some(Some(42)));
        let VectorEncoding::Flat { values, .. } = chunks[0].columns()[0].encoding() else {
            panic!("expected flat")
        };
        assert_eq!(value_ptr, values.as_ptr());
    }
}

#[test]
fn malformed_inline_wrappers_and_schema_fail() {
    let wire = default_wire();
    let pointer = wire
        .windows(6)
        .position(|w| w == [4, 0, 1, 1, 44, 1])
        .unwrap()
        + 3;
    for (offset, value) in [(pointer, 0), (pointer, 2), (pointer + 1, 43)] {
        let mut bad = wire.clone();
        bad[offset] = value;
        assert!(codec::decode(bad).is_err());
    }
    let chunk_type = pointer + 3 + 6;
    assert_eq!(&wire[chunk_type..chunk_type + 3], &[100, 0, 13]);
    let mut unsupported = wire.clone();
    unsupported[chunk_type + 2] = 27;
    let error = codec::decode(unsupported).unwrap_err();
    assert_eq!(error.kind, DecodeErrorKind::Unsupported);

    let mut bad_schema = wire.clone();
    // Advertised type BOOLEAN, while chunk still carries INTEGER.
    let advertised = bad_schema
        .windows(3)
        .position(|w| w == [100, 0, 13])
        .unwrap();
    bad_schema[advertised + 2] = 10;
    assert!(
        codec::decode(bad_schema)
            .unwrap_err()
            .message
            .contains("chunk types differ")
    );

    let response = prepare(wire.clone());
    let body = response.body().bytes().clone();
    for end in 0..body.len() {
        assert!(PrepareResponse::from_body(body.slice(..end).into()).is_err());
    }
    // Removing the wrapper terminator must not consume the PREPARE query UUID as END.
    let mut bad = wire;
    let wrapper_end = bad.len() - 8;
    assert_eq!(&bad[wrapper_end..wrapper_end + 2], &[255, 255]);
    bad.drain(wrapper_end..wrapper_end + 2);
    let global = codec::decode(bad.clone()).unwrap_err();
    let local = PrepareResponse::from_body(bad[17..].to_vec().into()).unwrap_err();
    assert_eq!(global.offset, 17 + local.offset);
}

#[test]
fn fetch_rejects_count_mismatch_trailing_truncation_and_schema_mismatch() {
    let response = fetch(deferred_wire());
    let payload = response.payload.bytes().clone();
    for end in 0..payload.len() {
        let response = FetchResponse {
            chunk_count: 1,
            payload: payload.slice(..end).into(),
            ..Default::default()
        };
        assert!(response.decode_chunks().is_err());
    }
    for count in [0, 2, u64::MAX] {
        let response = FetchResponse {
            chunk_count: count,
            payload: payload.clone().into(),
            ..Default::default()
        };
        assert!(response.decode_chunks().is_err());
    }
    let mut trailing = payload.to_vec();
    trailing.push(0);
    assert!(
        FetchResponse {
            chunk_count: 1,
            payload: trailing.into(),
            ..Default::default()
        }
        .decode_chunks()
        .is_err()
    );
    let schema = prepare(fixture(include_str!(
        "fixtures/integer/columns_prepare.hex"
    )));
    assert!(
        schema
            .validate_chunk(&response.decode_chunks().unwrap()[0])
            .is_err()
    );
}
