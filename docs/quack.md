---
# Copyright (c) 2026 ADBC Drivers Contributors
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#         http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
{}
---

{{ cross_reference|safe }}
# DuckDB Quack Driver {{ version }}

{{ heading|safe }}

This driver provides ADBC access to a DuckDB server exposed through the Quack
remote protocol.

:::{note}
This project is not associated with DuckDB Labs.
:::

## Installation & Quickstart

The driver can be installed with [dbc](https://docs.columnar.tech/dbc):

```bash
dbc install --pre quack
```

## Connecting

Connections require verified TLS by default. Start the local development
fixture with `docker compose up -d --wait test-service`. It includes a Caddy
TLS proxy with a locally issued certificate, so the example skips verification:

```python
from adbc_driver_manager import dbapi

dbapi.connect(
    driver="quack",
    db_kwargs={"uri": "quack://localhost:9496/?token=quack-secret&tls=skip_verify"},
)
```

The plaintext endpoint is also available at
`quack://localhost:9494/?token=quack-secret&tls=false`.

## Connection String Format

Quack URI syntax:

```text
quack://HOST[:PORT]/?token=TOKEN
```

Components:

- `Scheme`: `quack://` (required)
- `HOST`: Quack server host (required)
- `PORT`: Quack server port (optional)
- `token`: shared Quack authentication token (optional)
- `tls`: `true` (default) verifies the server certificate and hostname;
  `false` uses plaintext HTTP; `skip_verify` or `skip-verify` requires TLS
  without certificate or hostname verification.

The database option `quack.tls` accepts the same values and overrides the URI
setting regardless of setter order. Changes apply to subsequently initialized
connections only. Empty, null, unknown, and duplicate URI `tls` values are
rejected.

## Feature & Type Support

{{ features|safe }}

### Types

{{ types|safe }}

{{ footnotes|safe }}

## Compatibility

{{ compatibility_info|safe }}
