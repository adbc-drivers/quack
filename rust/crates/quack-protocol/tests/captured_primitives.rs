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

#[path = "fixtures/primitives/check.rs"]
mod check;
use quack_protocol::{codec, message::*};
fn wire(text: &str) -> Vec<u8> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .flat_map(str::split_whitespace)
        .map(|v| u8::from_str_radix(v, 16).unwrap())
        .collect()
}

#[test]
fn captured_primitive_transcripts() {
    assert!(!check::SQL.is_empty());
    assert!(!check::MULTI_SQL.is_empty());
    for (name, prepare, fetch, terminal, expected) in [
        (
            "mixed",
            include_str!("fixtures/primitives/mixed_inline_prepare.hex"),
            None,
            None,
            6,
        ),
        (
            "mixed",
            include_str!("fixtures/primitives/mixed_deferred_prepare.hex"),
            Some(include_str!(
                "fixtures/primitives/mixed_deferred_fetch_1.hex"
            )),
            Some(include_str!(
                "fixtures/primitives/mixed_deferred_fetch_2.hex"
            )),
            6,
        ),
        (
            "empty",
            include_str!("fixtures/primitives/empty_prepare.hex"),
            None,
            None,
            0,
        ),
        (
            "multi",
            include_str!("fixtures/primitives/multichunk_prepare.hex"),
            Some(include_str!("fixtures/primitives/multichunk_fetch_1.hex")),
            Some(include_str!("fixtures/primitives/multichunk_fetch_2.hex")),
            2050,
        ),
    ] {
        let mut offset = 0;
        for text in std::iter::once(prepare).chain(fetch).chain(terminal) {
            let bytes = wire(text);
            let message = codec::decode(bytes.clone()).unwrap();
            assert_eq!(codec::encode(&message).unwrap().to_vec(), bytes);
            let chunks = match message.body {
                MessageBody::PrepareResponse(p) => p.into_parts().results,
                MessageBody::FetchResponse(f) => f.decode_chunks().unwrap(),
                other => panic!("unexpected {other:?}"),
            };
            for chunk in chunks {
                if name == "multi" {
                    check::check_multi(&chunk, offset);
                } else {
                    check::check_mixed(&chunk, offset);
                }
                offset += chunk.rows();
            }
        }
        assert_eq!(offset, expected);
    }
}
