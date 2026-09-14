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

# ADBC Driver for DuckDB Quack

This repository contains [ADBC drivers](https://arrow.apache.org/adbc/) for
[DuckDB Quack](https://duckdb.org/quack/), implemented in C++.

This project is not associated with DuckDB Labs.

## Installation

Pre-packaged builds of the drivers in this repo have been made available for
various platforms from the [Columnar](https://columnar.tech) CDN. These can be
installed by any tool that supports [ADBC](https://arrow.apache.org/adbc/)
Driver Manifests, such as [dbc](https://columnar.tech/dbc):

```sh
dbc install quack --pre
```

See [Building](#building) if you would rather build the drivers yourself.

## Connecting

Set the ADBC database option `uri` to
`quack://HOST[:PORT]/?token=TOKEN`. Connections require TLS and verify the
server certificate by default, including connections to localhost.

The database option `quack.tls` or the URI query parameter `tls` selects the
transport:

| Value | Behavior |
| --- | --- |
| `true` (default) | Require TLS and verify the server certificate and hostname. |
| `false` | Use plaintext HTTP. |
| `skip_verify` or `skip-verify` | Require TLS without verifying the server certificate or hostname. |

For the local development server, start `docker compose up -d --wait test-service`
and use `quack://localhost:9496/?token=quack-secret&tls=skip_verify`.
The Compose fixture includes a Caddy TLS proxy with a locally issued certificate.
The plaintext endpoint is also available at
`quack://localhost:9494/?token=quack-secret&tls=false`.

An explicit `quack.tls` database option overrides the URI setting regardless
of the order in which options are set. Changes affect subsequently initialized
connections; existing connections retain their transport settings. Empty,
null, unknown, and duplicate URI `tls` values are rejected.

## Building

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).
