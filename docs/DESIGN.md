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

# Quack Driver Design

## Purpose

This repository builds an ADBC C driver for a remote DuckDB server exposed
through the Quack protocol. The driver is implemented in C++ at the repository
root with CMake. There is no `rust/` or `go/` implementation directory.

## Project Shape

- `CMakeLists.txt` is the root project. It requires CMake 4, uses C++20,
  finds ZLIB, OpenSSL, and CURL, fetches DuckDB, DuckDB Quack, nanoarrow, and
  uriparser with `FetchContent`, configures generated build metadata, and adds
  `src/` and `tests/`.
- `src/CMakeLists.txt` builds `adbc_driver_quack_helpers` and the shared
  library `adbc_driver_quack`.
- `src/adbc_driver_quack.cc` owns the exported ADBC C API entry points, driver
  table initialization, database/connection/statement state, query execution,
  and bulk ingest.
- `src/get_info_stream.cc` builds the Arrow C Stream result for
  `AdbcConnectionGetInfo` with nanoarrow.
- `src/duckdb_arrow_stream.cc` adapts DuckDB streaming query results to the
  Arrow C Stream interface.
- `src/quack_uri.cc` parses `quack://...` connection URIs.
- `src/sql_escape.cc` builds escaped DuckDB SQL string literals and remote
  query wrappers.
- `tests/` contains Googletest coverage for helpers, exported symbols, and
  core ADBC driver callback behavior.
- `validation/tests/quack.py` defines the validation-suite driver quirks and
  feature flags.
- `validation/queries/` contains txtcase overrides for SQL dialect or result
  differences.

## Runtime Model

`AdbcDatabaseSetOption` accepts a `uri` option in this form:

```text
quack://HOST[:PORT]/?token=TOKEN
```

`AdbcConnectionInit` opens an in-memory local DuckDB database, loads the
statically linked Quack extension, attaches the remote endpoint as the fixed
catalog name `remote`, and stores the live DuckDB connection in
`ConnectionState`.

Validation defaults to a Quack server at:

```text
quack://localhost:9494/?token=quack-secret
```

`compose.yaml` defines a local DuckDB Quack service for that endpoint.

Statement execution wraps caller SQL with Quack remote execution through
`BuildRemoteQuerySql`. Bulk ingest scans an Arrow stream into local DuckDB,
creates or updates the remote table through Quack, clears Quack metadata cache,
and inserts into `remote.<schema>.<table>`.

Result-producing statement execution and GetObjects use DuckDB streaming query
results and convert each fetched DuckDB chunk to Arrow on demand. Callers must
consume or release a returned Arrow stream before issuing another query on the
same connection because DuckDB permits only one active streaming query result
per connection.

## Metadata APIs

Implemented metadata support currently includes `AdbcConnectionGetInfo`.
`GetInfo` returns driver and vendor metadata, including the remote DuckDB
version queried through Quack.

Optional metadata APIs should be added deliberately and wired in three places:

1. exported C function declarations and definitions in `src/adbc_driver_quack.cc`;
2. `AdbcDriver` callback table initialization in `InitDriver`;
3. validation feature flags in `validation/tests/quack.py`.

When adding Arrow-valued metadata results, prefer a focused helper file similar
to `get_info_stream.cc` instead of expanding `adbc_driver_quack.cc` further.

## Build and Validation

The CI-style local build is:

```bash
./ci/scripts/build.sh test linux amd64
```

That script configures and builds under `build/ci-test-linux-amd64`, then
copies the shared library to `build/libadbc_driver_quack.so` for validation.
It requires a vcpkg toolchain via `CMAKE_TOOLCHAIN_FILE`, `VCPKG_ROOT`, or
`VCPKG_INSTALLATION_ROOT`.

The C++ test wrapper is:

```bash
./ci/scripts/test.sh linux amd64
```

Direct CMake iteration is also supported from the repository root:

```bash
cmake -S . -B build -DCMAKE_BUILD_TYPE=Debug
cmake --build build
ctest --test-dir build --output-on-failure
```

Validation uses Pixi:

```bash
pixi run validate --collect-only
pixi run validate -k get_objects
pixi run validate
```

`pixi.toml` defines `validate`, `gendocs`, and `release`; it does not define a
repository-local `pixi run make` task.

Run `./ci/scripts/build.sh test linux amd64` before validation so the copied
driver library is current.

## Design Constraints

- Keep the public ADBC surface in `src/adbc_driver_quack.cc`, but move complex
  Arrow stream construction into helper files.
- Preserve the fixed local DuckDB attachment alias `remote` unless a feature
  explicitly requires changing the connection contract.
- Use existing helper functions for SQL escaping and remote query wrapping.
- Do not edit `.pixi/` dependency files.
- Do not introduce Rust or Go build assumptions; this driver is C++/CMake.
