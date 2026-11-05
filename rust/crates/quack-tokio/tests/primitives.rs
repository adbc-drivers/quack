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

#[path = "../../quack-protocol/tests/fixtures/primitives/check.rs"]
mod check;
use quack_protocol::{message::HugeInt, state};
use quack_tokio::{Connection, ConnectionOptions, TlsMode};

#[tokio::test]
async fn live_primitive_queries() {
    let endpoint = match std::env::var("QUACK_HTTP_ENDPOINT") {
        Ok(endpoint) => endpoint,
        Err(std::env::VarError::NotPresent) => return,
        Err(error) => panic!("{error}"),
    };
    let mut connection = Connection::new(ConnectionOptions {
        endpoint,
        tls: TlsMode::Disabled,
        protocol: state::ConnectionOptions {
            auth_string: "quack-secret".into(),
            ..Default::default()
        },
    })
    .await
    .unwrap();
    let empty = format!("SELECT * FROM ({}) q WHERE false", check::SQL);
    let mut uuid = 0;
    for inline_rows in [None, Some(0), Some(1)] {
        for (sql, expected) in [
            (check::SQL, 6),
            (empty.as_str(), 0),
            (check::MULTI_SQL, 2050),
        ] {
            uuid += 1;
            let mut query = connection
                .execute(state::QueryParams {
                    query: sql.into(),
                    uuid: HugeInt {
                        upper: 0,
                        lower: uuid,
                    },
                    inline_rows,
                })
                .await
                .unwrap();
            let mut chunks = Vec::new();
            while let Some(chunk) = query.next_chunk().await.unwrap() {
                chunks.push(chunk);
            }
            drop(query);
            let mut rows = 0;
            for chunk in chunks {
                if expected == 2050 {
                    check::check_multi(&chunk, rows);
                } else {
                    check::check_mixed(&chunk, rows);
                }
                rows += chunk.rows();
            }
            assert_eq!(rows, expected);
        }
    }
    connection.disconnect().await.unwrap();
}
