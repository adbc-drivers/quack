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

//! Run against Compose with QUACK_HTTP_ENDPOINT=http://localhost:9494/quack.

use quack_protocol::state;
use quack_tokio::{Connection, ConnectionOptions, Error, TlsMode};

#[tokio::test(flavor = "current_thread")]
async fn connect_and_disconnect() -> Result<(), Error> {
    let endpoint = match std::env::var("QUACK_HTTP_ENDPOINT") {
        Ok(endpoint) => endpoint,
        Err(std::env::VarError::NotPresent) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut connection = Connection::new(ConnectionOptions {
        endpoint,
        tls: TlsMode::Disabled,
        protocol: state::ConnectionOptions {
            auth_string: "quack-secret".into(),
            ..Default::default()
        },
    })
    .await?;
    connection.disconnect().await
}

#[tokio::test(flavor = "current_thread")]
async fn query_initial_response() -> Result<(), Error> {
    let endpoint = match std::env::var("QUACK_HTTP_ENDPOINT") {
        Ok(endpoint) => endpoint,
        Err(std::env::VarError::NotPresent) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut connection = Connection::new(ConnectionOptions {
        endpoint,
        tls: TlsMode::Disabled,
        protocol: state::ConnectionOptions {
            auth_string: "quack-secret".into(),
            ..Default::default()
        },
    })
    .await?;
    let uuid = quack_protocol::message::HugeInt { upper: 0, lower: 1 };
    let mut response = connection
        .execute(state::QueryParams {
            query: "SELECT CAST(42 AS INTEGER) AS answer".into(),
            uuid,
            inline_rows: Some(0),
        })
        .await?;
    assert_eq!(response.result_types().len(), 1);
    assert_eq!(
        response.result_types()[0].id,
        duckdb_wiredata::logical_type::LogicalTypeId::Integer
    );
    assert_eq!(response.result_names(), &["answer".into()]);
    assert_eq!(response.query_uuid(), uuid);
    let chunk = response.next_chunk().await?.ok_or("missing result chunk")?;
    assert_eq!(chunk.rows(), 1);
    assert_eq!(chunk.columns()[0].get(0), Some(Some(42)));
    assert!(response.next_chunk().await?.is_none());
    for (index, inline_rows) in [None, Some(0), Some(1)].into_iter().enumerate() {
        let mut query = connection
            .execute(state::QueryParams {
                query: "SELECT 42".into(),
                uuid: quack_protocol::message::HugeInt {
                    upper: 0,
                    lower: 2 + index as u64,
                },
                inline_rows,
            })
            .await?;
        assert_eq!(query.result_names(), &["42".into()]);
        let chunk = query.next_chunk().await?.ok_or("missing result chunk")?;
        assert_eq!(chunk.columns()[0].get(0), Some(Some(42)));
        assert!(query.next_chunk().await?.is_none());
    }
    for (index, sql, expected) in [
        ("SELECT NULL::INTEGER AS answer", vec![None]),
        ("SELECT 42::INTEGER AS answer WHERE false", vec![]),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (sql, expected))| (index, sql, expected))
    {
        let mut query = connection
            .execute(state::QueryParams {
                query: sql.into(),
                uuid: quack_protocol::message::HugeInt {
                    upper: 0,
                    lower: 10 + index as u64,
                },
                inline_rows: None,
            })
            .await?;
        let mut values = Vec::new();
        while let Some(chunk) = query.next_chunk().await? {
            for row in 0..chunk.rows() {
                values.push(chunk.columns()[0].get(row).ok_or("row out of bounds")?);
            }
        }
        assert_eq!(values, expected);
    }
    connection.disconnect().await
}
