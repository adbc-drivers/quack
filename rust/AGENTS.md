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

# Rust Workspace Instructions

## Scope and workflow

This directory is the Rust workspace. Run Rust commands from `rust/`.
The parent repository also contains a C++ driver; its statements that this
Rust directory does not exist or that Rust guidance is irrelevant do not
apply here. Rust tests do not require a Pixi or C++ driver build.

Generally offer plan mode before implementing or fixing code. Keep changes
focused and avoid unsolicited scaffolding.

## Commits

Do not commit unless explicitly authorized. Stage only intended files,
including Cargo.lock changes when applicable, before running pre-commit.
Never stage temporary files or unrelated work.

Use one assistance trailer with the actual model name, without an email:

```text
Assisted-by: GPT-6
```

Do not use `Co-authored-by:` or `Generated-by:`, and do not combine assistance
trailers. This overrides the parent instruction to use `Generated-by:`.

## Rust conventions and architecture

- Do not use `unwrap` or `expect` outside test code. Propagate errors and use
  `map_err` when conversion is needed.
- Rename underscore-prefixed variables when they become used.
- Prefer an actual `match` over chains of `matches!` conditions.
- Use the existing Apache-2.0 header and ADBC Drivers Contributors copyright,
  with the appropriate year and comment syntax.
- Keep `quack-protocol` independent of Tokio and HTTP. Expose its modules
  directly rather than adding crate-root re-exports.
- Keep connection state opaque, with a private state enum.
- The Tokio connection owns its HTTP client. Its async constructor completes
  connection establishment before returning successfully.

## Live integration tests

Put Tokio integration tests under `crates/quack-tokio/tests`.
Read `QUACK_HTTP_ENDPOINT` from the environment. Return early only if it is
absent; malformed values or connection failures must fail configured tests.

Use the fixture in `../compose.yaml`. From this directory:

```bash
docker compose -f ../compose.yaml up -d --wait test-service
QUACK_HTTP_ENDPOINT=http://localhost:9494/quack cargo test -p quack-tokio --test connection
```

The plaintext fixture uses `TlsMode::Disabled` and token `quack-secret`.
The test should not manage Docker itself.

## Validation

Run the following checks and relevant Rust tests before finishing code changes:

```bash
cargo build
cargo +nightly clippy --all-targets
cargo fmt --all -- --check
pre-commit run --all-files
```

Fix compiler warnings and pre-commit failures. Keep Cargo.lock current and
include its updates in commits. Do not run `pixi run make` as a prerequisite
for Rust tests.

## Tool and temporary-file hygiene

- Codex must run `gh` and `pre-commit` outside the sandbox.
- Do not repeatedly download resources, unpack archives, or rerun expensive
  builds/test suites just to inspect their output. Capture output once and
  inspect the saved result. Rerun when changes or failures justify it.
- Run pre-commit directly; never redirect its output.
- Put temporary output directly in `/tmp/temp_codex`, name each file with the
  `temp_codex_` prefix, and remove it afterward. Recursive directory cleanup
  requires approval.
- Inspect Rust dependencies in `~/.cargo` and Python dependencies in the
  repository's `.pixi`. Do not search the entire filesystem for dependencies
  or modify `.pixi` files.
- Avoid Bash comments in code-review commands and do not run useless echo
  commands.
