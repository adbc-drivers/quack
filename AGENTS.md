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

# Repository Instructions

This repository is the ADBC Driver for DuckDB Quack.

## Layout

This is a C++/CMake driver repository at the repository root. Do not assume
there is a `rust/` or `go/` subdirectory, and do not run commands from those
nonexistent directories.

Important paths:

- `CMakeLists.txt`: root CMake project.
- `src/`: C++ driver implementation and helper libraries.
- `tests/`: C++ Googletest tests registered with CTest.
- `validation/tests/`: Python validation-suite adapter and local quirks.
- `validation/queries/`: validation txtcase overrides.
- `third_party/arrow-adbc/include/arrow-adbc/adbc.h`: vendored ADBC C API
  header.
- `.pixi/`: Python and validation dependencies. Prefer this over exploring
  unrelated filesystem locations.
- `docs/DESIGN.md`: current architecture notes for future agents.

## Build and Test

Use the repository root as the working directory unless a command below says
otherwise.

For the CI-style local C++ build on Linux amd64:

```bash
./ci/scripts/build.sh test linux amd64
```

For C++ tests after that build:

```bash
./ci/scripts/test.sh linux amd64
```

The build script requires a vcpkg toolchain via `CMAKE_TOOLCHAIN_FILE`,
`VCPKG_ROOT`, or `VCPKG_INSTALLATION_ROOT`.

Prefer `./ci/scripts/build.sh test linux amd64` over direct CMake invocation.
That script configures the vcpkg toolchain, uses the expected CI build
directory, builds the tests, and copies the driver library to
`build/libadbc_driver_quack.so` for validation.

Validation is configured as a Pixi task and expects the driver library at
`build/libadbc_driver_quack.so` on Linux:

```bash
pixi run validate --collect-only
pixi run validate -k get_objects
pixi run validate
```

There is no repository-local `pixi run make` task in `pixi.toml`.

Always rebuild the driver before running validation:

```bash
./ci/scripts/build.sh test linux amd64
```

For final formatting and repository checks:

```bash
pre-commit run --all-files
```

Codex must run `gh` and `pre-commit` outside the sandbox.

## Development Notes

- Use C++20 and the existing style in `src/` and `tests/`.
- `panic`, Rust `unwrap`, and Rust `expect` guidance from broader ADBC
  instructions is not relevant here because this repository is not Rust.
- Assertions in C++ tests use Googletest.
- Validation features are declared in `validation/tests/quack.py`.
- Validation defaults to `QUACK_URI` or
  `quack://localhost:9494/?token=quack-secret`.
- `compose.yaml` defines the local Quack server used by validation.
- Validation query overrides use txtcase files under `validation/queries/`.
- Do not modify files under `.pixi/`; inspect them only when needed to
  understand installed validation dependencies.
- Do not leave temporary files behind. If a temporary file is necessary, prefix
  it with `temp_` and remove it before finishing.
- Do not commit unless explicitly requested. If committing is requested, stage
  files explicitly and include a `Generated-by:` trailer.
