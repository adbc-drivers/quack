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

# Captured primitive responses

Captured on 2026-11-05 from the running `duckdb/duckdb:2.0.0` fixture
configured by `cpp/compose.yaml`. The handshake reports DuckDB v2.0.0,
linux_amd64, Quack 3; serialization is V2_0_0 little-endian. Hex files contain
exact response bytes with no rewriting. Comment lines record SQL, inline
threshold, query UUID, and server version.

`mixed.sql` returns six rows with BOOLEAN; signed and unsigned 8-, 16-, 32-,
and 64-bit integers; FLOAT; and DOUBLE. Row 0 contains false, signed minima,
unsigned zero, and floating negative zero. Row 1 contains true, integer maxima,
and 1.5. Row 2 is entirely NULL. Rows 3–5 contain true, signed -7, unsigned 7,
and respectively positive infinity, negative infinity, and NaN.

- `mixed_inline`: six rows in PREPARE, without a fetch.
- `mixed_deferred`: metadata-only PREPARE, one FETCH containing six rows,
  and a terminal FETCH.
- `empty`: the same eleven-column schema with zero rows.
- `multichunk`: 2,050 rows of BIGINT `i`, UTINYINT `i % 256`, and BOOLEAN
  `i % 2 = 0`; deferred FETCH contains chunks of 2,048 and 2 rows, followed
  by a terminal FETCH.

Offline protocol and Arrow tests replay these bytes. Synthetic tests cover
compressed encodings, alignment, malformed values, and NaN payload bits;
these cases must not depend on server optimizer decisions.

To regenerate from the Rust workspace (this deliberately rewrites fixtures):

```bash
docker compose -f ../cpp/compose.yaml up -d --wait test-service
QUACK_HTTP_ENDPOINT=http://localhost:9494/quack cargo test -p quack-tokio --test capture_primitives -- --ignored
```

The capture test is ignored during normal runs. Live Tokio and ADBC tests
use the same endpoint variable and skip only when it is absent.
