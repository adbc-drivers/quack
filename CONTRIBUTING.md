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

# How to Contribute

All contributors are expected to follow the [Code of
Conduct](https://github.com/adbc-drivers/quack?tab=coc-ov-file#readme).

## Reporting Issues and Making Feature Requests

Please file issues and feature requests on the GitHub issue tracker:
https://github.com/adbc-drivers/quack/issues

Potential security vulnerabilities should be reported to
[security@adbc-drivers.org](mailto:security@adbc-drivers.org) instead.  See
the [Security
Policy](https://github.com/adbc-drivers/quack?tab=security-ov-file#readme).

## Build and Test

### Build

Build the driver by passing the configuration, platform, and architecture to
the CI build script:

```bash
./ci/scripts/build.sh [test|release] [linux|windows|macos] [amd64|arm64]
```

The build script requires a vcpkg toolchain via `CMAKE_TOOLCHAIN_FILE`,
`VCPKG_ROOT`, or `VCPKG_INSTALLATION_ROOT`. It configures the CI build under
`build/ci-test-linux-amd64`, builds the driver and C++ tests, and copies the
driver library to `build/libadbc_driver_quack.so`.

### Test

C++ unit tests can be run after building:

```bash
./ci/scripts/test.sh linux amd64
```

Also, the validation suite can be run via [pixi](https://pixi.prefix.dev/latest/).
The driver must be built with `./ci/scripts/build.sh` before running validation.
Also, the Quack server defined in `compose.yaml` must be started:

```bash
docker compose up test-service
export QUACK_URI="quack://localhost:9494/?token=quack-secret"

pixi run validate --collect-only
pixi run validate -k connection
pixi run validate
```

This will produce a test report, which can be rendered into a documentation
page (using MyST Markdown):

```shell
$ pixi run gendocs --output generated/
```

Then look at `./generated/quack.md`.

## Opening a Pull Request

Before opening a pull request:

- Review your changes and make sure no stray files, etc. are included.
- Ensure the Apache license header is at the top of all files.
- Check if there is an existing issue.  If not, please file one, unless the
  change is trivial.
- Assign the issue to yourself by commenting just the word `take`.
- Run the static checks by installing [pre-commit](https://pre-commit.com/),
  then running `pre-commit run --all-files` from inside the repository.  Make
  sure all your changes are staged/committed (unstaged changes will be
  ignored).

When writing the pull request description:

- Ensure the title follows [Conventional
  Commits](https://www.conventionalcommits.org/en/v1.0.0/) format.  The
  component should be a directory path relative to the repo root,
  e.g. `src/auth` would also be valid if that directory existed).  Example
  titles:

  - `feat: support GEOGRAPHY data type`
  - `chore: update action versions`
  - `fix!: return us instead of ms`

  Ensure that breaking changes are appropriately flagged with a `!` as seen
  in the last example above.
- Make sure the description ends with `Closes #NNN`, `Fixes #NNN`, or
  similar, so that the issue will be linked to your pull request.
