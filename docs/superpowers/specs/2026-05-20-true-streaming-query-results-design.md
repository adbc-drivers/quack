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

# True Streaming Query Results Design

## Goal

Make result-producing driver APIs stream DuckDB result chunks instead of
materializing the full result set through `duckdb_query_arrow`.

This applies to both:

- `AdbcStatementExecuteQuery` for SQL query execution.
- `AdbcConnectionGetObjects` for validation metadata results.

## Selected Approach

Use DuckDB's C++ streaming query API:

```cpp
duckdb::Connection::SendQuery(sql, duckdb::QueryResultOutputType::ALLOW_STREAMING)
```

The returned `duckdb::QueryResult` may be a `StreamQueryResult` or a
`MaterializedQueryResult`, depending on DuckDB and statement shape. The helper
will not call `duckdb_query_arrow`, so it will not request DuckDB's deprecated
materialized Arrow result API. Result consumption will use `QueryResult::Fetch`
or `QueryResult::TryFetch` and convert each `duckdb::DataChunk` to Arrow as it
is requested by `ArrowArrayStream::get_next`.

The deprecated DuckDB C streaming APIs are intentionally not used.

## Architecture

Replace the internals of `src/duckdb_arrow_stream.cc` with a streaming
implementation while keeping a narrow helper boundary for callers. The helper
will own a `duckdb::QueryResult` in `ArrowArrayStream::private_data`, expose the
same Arrow C Stream callbacks, and release the query result when the stream is
released.

The helper API should be renamed from `ExecuteDuckDbArrowQuery` to
`ExecuteDuckDbStreamingArrowQuery` so call sites and tests clearly distinguish
the new behavior from the old materializing DuckDB Arrow API. Both
`DriverStatementExecuteQuery` and `DriverConnectionGetObjects` will call the
new helper.

## Stream State

The stream state will contain:

```cpp
std::unique_ptr<duckdb::QueryResult> result;
duckdb::unordered_map<duckdb::idx_t,
                      const duckdb::shared_ptr<duckdb::ArrowTypeExtensionData>>
    extension_types;
std::string last_error;
```

`extension_types` is computed once after a successful query using
`duckdb::ArrowTypeExtensionData::GetExtensionTypes`, matching DuckDB's own C
Arrow adapter behavior.

## Data Flow

1. The caller builds the remote Quack SQL exactly as it does today.
2. The streaming helper calls `SendQuery(..., ALLOW_STREAMING)`.
3. If DuckDB returns an error result, the helper converts it to
   `ADBC_STATUS_IO` and no stream is installed.
4. If `rows_affected` is requested for result-producing execution, the helper
   sets it to `-1`. It does not call APIs that require materializing the result
   only to compute a row count.
5. `get_schema` converts `result->types` and `result->names` with
   `duckdb::ArrowConverter::ToArrowSchema`.
6. `get_next` fetches one `duckdb::DataChunk`. If no chunk remains, it leaves
   `ArrowArray::release` null. If a chunk is available, it converts that chunk
   with `duckdb::ArrowConverter::ToArrowArray`.
7. `release` destroys the stored `QueryResult` and clears all callbacks.

## Error Handling

The helper should catch `duckdb::Exception`, `std::exception`, and unknown
exceptions around query execution, schema conversion, and chunk conversion.
Errors from callbacks are stored in `last_error` and reported through
`get_last_error`.

Callback return codes should remain POSIX-style integers:

- `EINVAL` for null stream state or null output pointers.
- `EIO` for DuckDB query/fetch/conversion failures.

Driver entry points continue to convert helper failures into `AdbcError` with
the existing `StatusError` path.

## Concurrency Constraint

DuckDB documents that there can be only one active `StreamQueryResult` per
connection and that starting another query invalidates any existing streaming
result on that connection. The driver will not add connection-level
synchronization in this change. It will rely on the existing ADBC stream
lifetime contract: callers must consume or release the stream before issuing
another query on the same connection.

Tests should cover the normal stream lifecycle and the behavior expected from
chunk-by-chunk consumption. The connection-level single-active-stream rule
should be documented in code comments where the streaming query is created.

## Testing

Update the existing helper test to use the renamed streaming helper. Add tests
that:

- Fetch a simple one-row query through the Arrow C Stream callbacks.
- Fetch a large `range(...)` query and observe more than one Arrow array.
- Return an empty end-of-stream Arrow array with `release == nullptr`.
- Surface syntax or catalog errors from query execution.
- Surface invalid callback arguments with `EINVAL`.

Run the CI-style build and test scripts after implementation:

```bash
./ci/scripts/build.sh test linux amd64
./ci/scripts/test.sh linux amd64
pixi run validate --collect-only
pixi run validate -k get_objects
pre-commit run --all-files
```

Codex must run `pre-commit` outside the sandbox.
