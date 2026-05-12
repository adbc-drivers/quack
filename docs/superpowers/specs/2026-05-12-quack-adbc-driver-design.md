# Quack ADBC Driver Design

## Goal

Build a CMake 4 C++ project that produces a shared library named
`libadbc_driver_quack`. The library exports the ADBC C API and uses a local
DuckDB client, loaded with the Quack extension, to connect to and query a
remote DuckDB instance. Project C++ sources use C++20.

## Project Layout

- `CMakeLists.txt` at the repository root defines the project, requires CMake
  4, fetches DuckDB 1.5.2 and uriparser with `FetchContent`, applies the DuckDB
  patch before configuring DuckDB, enables testing, and adds the `src` and
  `tests` subdirectories.
- `cmake/patches/duckdb-disable-adbc.patch` disables DuckDB's own ADBC
  module/build entry so this driver can export the ADBC C API without duplicate
  DuckDB ADBC symbols.
- `src/CMakeLists.txt` builds a shared library target named
  `adbc_driver_quack` with output name `adbc_driver_quack`.
- `src/adbc_driver_quack.cc` owns the exported ADBC C API functions and the
  DuckDB-backed driver lifecycle.
- `src/quack_uri.h` and `src/quack_uri.cc` parse the connection URI. Helper
  APIs use `std::string_view` for read-only string inputs where practical.
- `third_party/arrow-adbc/include/arrow-adbc/adbc.h` vendors the ADBC C API
  header from Apache Arrow ADBC.
- `third_party/arrow-adbc/LICENSE.txt` and `third_party/arrow-adbc/NOTICE.txt`
  preserve the vendored header's Apache project license and notice files.
- `tests/CMakeLists.txt` fetches Googletest with `FetchContent`, builds unit
  tests, and registers them with CTest.
- `tests/quack_uri_test.cc` verifies URI parsing behavior.
- `tests/exported_symbols_test.cc` verifies the library exports the expected
  ADBC entry points.

## Dependencies

The root project depends on DuckDB 1.5.2 and uriparser via `FetchContent`.
DuckDB is patched during the dependency population step before DuckDB is added
to the build. Tests depend on Googletest via `FetchContent` and are registered
with CTest using `gtest_discover_tests`.

The driver includes the vendored Apache Arrow ADBC C header and links to
DuckDB's C API and uriparser. It does not embed or expose DuckDB's ADBC
implementation. DuckDB's own ADBC module is disabled by patch to avoid symbol
conflicts with this driver's exported ADBC functions.

## Connection Contract

The ADBC database option `uri` configures the remote endpoint:

```text
uri=quack://host:port/?token=...
```

URI parsing is implemented with uriparser. The parser accepts:

- scheme: exactly `quack`
- host: non-empty DNS name, IPv4 address, IPv6 literal in brackets, or other
  authority text DuckDB Quack can consume
- port: optional decimal port
- query parameter `token`: optional token string

The parser converts the URI authority into the Quack attach endpoint:

```text
quack:host
quack:host:port
```

The token is URL-decoded before use. Missing or malformed `uri` values cause
ADBC initialization to fail with a descriptive error.

## Runtime Behavior

`AdbcDatabaseNew`, `AdbcDatabaseSetOption`, `AdbcDatabaseInit`, and
`AdbcDatabaseRelease` manage database-level configuration. The database stores
the parsed Quack URI and no live DuckDB connection.

`AdbcConnectionInit` opens an in-memory local DuckDB database and configures it
as a Quack client:

```sql
INSTALL quack;
LOAD quack;
CREATE SECRET (TYPE quack, TOKEN '<token>');
ATTACH '<quack-endpoint>' AS remote;
```

`CREATE SECRET` is skipped when the URI has no token. SQL string construction
uses escaping for token and endpoint values. The attached database alias is
fixed as `remote`.

`AdbcStatementNew`, `AdbcStatementSetSqlQuery`, `AdbcStatementExecuteQuery`,
and `AdbcStatementRelease` delegate query execution through Quack's
`remote.query` function. The driver wraps caller SQL in a local DuckDB query
that sends the original SQL text to the remote instance:

```sql
SELECT * FROM remote.query('<caller-sql>');
```

SQL string construction escapes the caller SQL before embedding it in the
`remote.query` call. Callers can submit ordinary DuckDB SQL without rewriting
table references to include the `remote` attached database alias.

`AdbcStatementPrepare`, `AdbcStatementBind`, and `AdbcStatementBindStream`
return `ADBC_STATUS_NOT_IMPLEMENTED` in the first implementation. The driver
does not interpolate bound parameter values into SQL because that would require
complete, type-aware DuckDB literal serialization and would be easy to make
incorrect. A future implementation can add parameter support by using an
explicit remote parameter mechanism if Quack exposes one, or by implementing
well-tested type-aware serialization.

Optional ADBC APIs that are not required for database lifecycle, connection
lifecycle, statement lifecycle, query execution, or error release return
`ADBC_STATUS_NOT_IMPLEMENTED`.

## Error Handling

The driver fills `AdbcError` with:

- `message`: a stable heap-allocated string that explains the failure
- `vendor_code`: DuckDB error code when available, otherwise zero
- `release`: a function that frees driver-owned error memory

Failures from URI parsing, DuckDB open/connect, extension load, secret
creation, attach, and query execution are translated into the closest ADBC
status code.

## Testing

Tests use Googletest and CTest.

Unit tests cover:

- valid URI parsing with host only
- valid URI parsing with host and port
- valid URI parsing with URL-encoded token
- invalid scheme rejection
- missing host rejection
- SQL delegation wrapper escaping for `remote.query`
- parameter binding APIs returning `ADBC_STATUS_NOT_IMPLEMENTED`
- exported symbols for the required ADBC entry points

Integration testing is limited to deterministic local behavior in the first
iteration. If no Quack server is running, connection attempts should fail
cleanly with an ADBC error instead of crashing or leaking obvious resources.

## Non-Goals

- SQL rewriting to automatically qualify table references with `remote`.
- Parameter binding support.
- Implementing every optional ADBC metadata and partitioning API in the first
  iteration.
- Running or provisioning a remote DuckDB Quack server as part of the default
  test suite.
