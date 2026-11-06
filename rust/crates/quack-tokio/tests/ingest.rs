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

use duckdb_wiredata::{
    encode::Encoder,
    logical_type::{LogicalType, LogicalTypeId},
};
use quack_protocol::{
    message::{HugeInt, SendDataRequest},
    state,
};
use quack_tokio::{Connection, ConnectionOptions, Error, TlsMode};

#[tokio::test]
async fn segmented_upload_empty_stream_and_cancellation() -> Result<(), Error> {
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
    for run in 0..3 {
        let stream = format!("test_upload_{run}");
        let mut query = connection.execute(state::QueryParams {
            query: format!("SELECT count(*) FROM scan_data_from_quack_client('{stream}', NULL::STRUCT(value INTEGER), ordered := true)").into(),
            uuid: HugeInt { upper: 0, lower: run + 1 }, inline_rows: Some(0),
        }).await?;
        query.set_expected_result_schema(
            vec![LogicalType {
                id: LogicalTypeId::BigInt,
                info: None,
            }],
            vec!["Count".into()],
        )?;
        if run == 0 {
            query.cancel().await?;
            continue;
        }
        let batches = if run == 1 { 0 } else { 1 };
        if batches == 1 {
            let mut encoder = Encoder::default();
            encoder.field(100);
            encoder.uint(1);
            encoder.field(101);
            encoder.uint(1);
            encoder.logical_type(&LogicalType {
                id: LogicalTypeId::Integer,
                info: None,
            })?;
            encoder.field(102);
            encoder.uint(1);
            encoder.field(100);
            encoder.boolean(false);
            encoder.field(102);
            encoder.blob(bytes::Bytes::from_static(&[42, 0, 0, 0]));
            encoder.end();
            encoder.end();
            let payload = encoder.finish();
            let payload = payload
                .iter()
                .cloned()
                .chain(payload.iter().cloned())
                .collect();
            query
                .send_data(
                    SendDataRequest {
                        stream_id: stream.clone().into(),
                        chunk_count: 2,
                        batch_index: Some(1),
                        total_batches: None,
                        payload: Default::default(),
                    },
                    payload,
                )
                .await?;
        }
        query
            .send_data(
                SendDataRequest {
                    stream_id: stream.into(),
                    chunk_count: 0,
                    batch_index: None,
                    total_batches: Some(batches),
                    payload: Default::default(),
                },
                vec![],
            )
            .await?;
        let chunk = query.next_chunk().await?.ok_or("missing count")?;
        assert_eq!(
            chunk.columns()[0].get::<i64>(0),
            Some(Some(2 * batches as i64))
        );
        while query.next_chunk().await?.is_some() {}
    }
    connection.disconnect().await
}
