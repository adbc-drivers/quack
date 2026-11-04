<!--
Copyright (c) 2026 ADBC Drivers Contributors

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

        http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-->

# Captured INTEGER responses

Captured on 2026-11-04 using the existing `quack-quack-server-1`
(`duckdb/duckdb:2.0.0`) and plaintext proxy on localhost:9494. The equivalent
fixture configuration is `../cpp/compose.yaml` from the Rust workspace.
The handshake negotiated Quack 3, DuckDB v2.0.0, linux_amd64; chunk encoding
uses V2_0_0. Each hex file records its SQL, inline threshold, and UUID.
Comment lines are metadata; all other tokens are exact response bytes.

| Prefix | Expected rows / delivery |
| --- | --- |
| `select42_default` | One INTEGER named `42`, value 42, entirely inline |
| `select42_deferred` | Same result; PREPARE has no chunks; FETCH 1 has one chunk; FETCH 2 terminates with total 1 |
| `select42_inline` | One inline row, but PREPARE still reports `needs_more_fetch=true`; reaching the threshold does not establish EOF |
| `nullable` | INTEGER `answer`: 42, NULL, -7, entirely inline |
| `empty` | INTEGER `answer`, zero rows, no fetch needed |
| `columns` | One row: INTEGER `answer`=42, INTEGER `other`=-7, entirely inline |
| `fetch_rows` | INTEGER `answer`: 0 through 4999; FETCH 1 contains three chunks (2048, 2048, 904 rows); FETCH 2 terminates with total 1 |

These fixtures establish real framing and flat INTEGER data. Synthetic
constant, dictionary, sequence, and mixed-delivery cases belong with the
new decoder and state-machine tests, so they can target each encoding
without depending on a server optimizer choice. The positive-threshold
PREPARE fixture intentionally captures only that response, not a complete
query transcript.
