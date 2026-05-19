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

# Quack Transactions And Prepare-Only Statements

**Goal:** Implement ADBC transaction callbacks and parameterless statement prepare support for the Quack driver, while keeping bind parameters unsupported.

## Summary

- Implement ADBC connection transactions over the existing attached Quack catalog/session.
- Implement parameterless `AdbcStatementPrepare` for SQL statements while keeping bind parameters unsupported.
- Adjust bound-stream state handling so SQL and bound streams can coexist until execution, where unsupported parameter binding fails explicitly.

## Key Changes

- In `src/adbc_driver_quack.cc`, add transaction state to `ConnectionState`: autocommit enabled by default plus active remote transaction tracking.
- Add `DriverConnectionCommit` and `DriverConnectionRollback`, export `AdbcConnectionCommit` and `AdbcConnectionRollback`, and wire both callbacks in `InitDriver`.
- Extend `DriverConnectionSetOption`/`GetOption` for `ADBC_CONNECTION_OPTION_AUTOCOMMIT`, returning `"true"`/`"false"` and rejecting commit/rollback while autocommit is enabled.
- When autocommit is disabled, send remote `BEGIN TRANSACTION`; after commit or rollback, send `COMMIT`/`ROLLBACK` and begin a fresh remote transaction so repeated commit/rollback calls while autocommit is disabled remain valid.
- Implement `DriverStatementPrepare` without binding: require initialized statement, initialized connection, and non-empty `state->sql`; set a `prepared` flag.
- Change bound-stream behavior:
  - `DriverStatementBindStream` stores the stream but does not clear `state->sql`.
  - Setting `ADBC_INGEST_OPTION_TARGET_TABLE` clears `state->sql`, since that statement is now bulk ingest.
  - `DriverStatementExecuteQuery` with `has_bound_stream && !state->sql.empty()` returns `ADBC_STATUS_NOT_IMPLEMENTED` for parameterized SQL execution.
  - `DriverStatementExecuteQuery` with `has_bound_stream && state->sql.empty()` keeps the current bulk-ingest path.
- Keep `DriverStatementBind` returning `ADBC_STATUS_NOT_IMPLEMENTED`, and do not implement parameter schema.

## Validation Changes

- Update `validation/tests/quack.py`:
  - `connection_transactions=True`
  - `statement_prepare=True`
  - leave `statement_bind=False`
  - leave `statement_get_parameter_schema=False`
- Do not add `validation/queries/type/bind/*.txtcase` overrides unless a validation run shows bind tests are still collected despite `statement_bind=False`.

## Tests

- Update C++ symbol tests to expect exported/populated `ConnectionCommit` and `ConnectionRollback`.
- Replace the existing C++ "prepare is unsupported" assertion with prepare state tests:
  - uninitialized statement returns `ADBC_STATUS_INVALID_STATE`;
  - initialized statement with no SQL returns `ADBC_STATUS_INVALID_STATE`;
  - initialized SQL statement can be prepared;
  - bind remains `ADBC_STATUS_NOT_IMPLEMENTED`.
- Add C++ state-routing tests for bound streams:
  - `BindStream` after `SetSqlQuery` preserves SQL and execution fails with `ADBC_STATUS_NOT_IMPLEMENTED`;
  - setting `ADBC_INGEST_OPTION_TARGET_TABLE` clears SQL so the statement uses the bulk-ingest path.
- Run:
  - `./ci/scripts/build.sh test linux amd64`
  - `./ci/scripts/test.sh linux amd64`
  - `pixi run validate -k "transaction_toggle or prepare"`
  - `pixi run validate --collect-only` to confirm bind cases remain skipped or marked unsupported
  - `pre-commit run --all-files` outside the sandbox.

## Assumptions

- Prepared statements mean parameterless `prepare()` support only for now.
- No Quack protocol changes are included.
- Bind support remains blocked until Quack exposes a typed remote parameter mechanism.
