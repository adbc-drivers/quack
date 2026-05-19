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

This repository contains a C++ ADBC driver for DuckDB's Quack remote protocol.

## Build

Use the repository root as the working directory. The supported local Linux
amd64 build path is:

```bash
./ci/scripts/build.sh test linux amd64
```

The build script requires a vcpkg toolchain via `CMAKE_TOOLCHAIN_FILE`,
`VCPKG_ROOT`, or `VCPKG_INSTALLATION_ROOT`. It configures the CI build under
`build/ci-test-linux-amd64`, builds the driver and C++ tests, and copies the
driver library to `build/libadbc_driver_quack.so`.

## Test

Run C++ tests after building:

```bash
./ci/scripts/test.sh linux amd64
```

Validation tests use the copied driver library and the Quack server defined in
`compose.yaml`:

```bash
pixi run validate --collect-only
pixi run validate -k connection
pixi run validate
```

Rebuild with `./ci/scripts/build.sh test linux amd64` before running
validation.
