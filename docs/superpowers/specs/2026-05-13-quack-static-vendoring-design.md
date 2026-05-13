# Static Quack Vendoring Design

## Goal

Make `libadbc_driver_quack` self-contained with respect to the DuckDB Quack
extension. The driver should not download, install, or locate the Quack
extension at runtime. Quack should be built into the DuckDB library that the
driver links against.

Binary size and build time should be improved where that does not conflict
with static Quack vendoring. The first priority is removing the runtime
extension dependency.

## Current State

The root CMake project fetches DuckDB `v1.5.2` and uriparser with
`FetchContent`. It patches DuckDB to disable DuckDB's own ADBC module to avoid
exported symbol conflicts with this driver.

`AdbcConnectionInit` currently opens an in-memory DuckDB instance and executes:

```sql
INSTALL quack;
LOAD quack;
```

Then it creates a Quack secret when needed and attaches the remote endpoint:

```sql
CREATE SECRET (TYPE quack, TOKEN '<token>');
ATTACH 'quack:<host>[:<port>]' AS remote;
```

This means a connection can fail because runtime extension installation or
loading is unavailable, slow, unsigned, offline, or pointed at the wrong
extension repository.

## Upstream Quack Shape

Quack is an out-of-tree DuckDB extension. Its upstream build follows DuckDB's
standard extension build flow:

- the extension source is registered with `duckdb_extension_load(quack
  SOURCE_DIR ...)`;
- `build_static_extension(quack ...)` produces a static extension library;
- `build_loadable_extension(quack ...)` produces `quack.duckdb_extension`;
- DuckDB's generated static extension loader can link statically linked
  extensions into DuckDB and load them by name.

Quack's upstream extension config also registers supporting extensions:

- `json`
- `autocomplete`
- `httpfs`

`httpfs` is an out-of-tree dependency fetched from `duckdb/duckdb-httpfs`.
Quack also declares vcpkg dependencies on `openssl` and `curl`.

## Build Design

The project will always vendor Quack. There will be no
`ADBC_DRIVER_QUACK_VENDOR_QUACK` option.

The root CMake project will declare Quack as a pinned dependency with
`FetchContent_Declare`. The pin must be explicit, using a branch, tag, or commit
known to work with the selected DuckDB version. A commit is preferred once we
verify one against DuckDB `v1.5.2`.

The project will provide a CMake extension config file, for example:

```text
cmake/duckdb/quack_extension_config.cmake
```

That config will register Quack as a statically linked DuckDB extension using
the fetched Quack source directory:

```cmake
duckdb_extension_load(quack
  SOURCE_DIR "${adbc_driver_quack_quack_SOURCE_DIR}")
```

The top-level project will pass that config to DuckDB through
`DUCKDB_EXTENSION_CONFIGS` before DuckDB is made available. The config must be
included before DuckDB's base extension config so it can establish the desired
Quack registration.

Supporting Quack extensions are handled in one of two ways:

1. Prefer Quack's own extension config if it proves necessary for correctness.
   This preserves upstream behavior and lowers integration risk.
2. If Quack builds and passes tests without all helper extensions, explicitly
   register only the required support set. This can trim the build without
   weakening the static-vendoring goal.

The first implementation will start with the upstream support set unless local
verification proves a smaller set works.

## Minimal DuckDB Build Settings

The DuckDB build should disable unused developer artifacts:

```cmake
set(BUILD_SHELL OFF CACHE BOOL "" FORCE)
set(BUILD_UNITTESTS OFF CACHE BOOL "" FORCE)
set(ENABLE_UNITTEST_CPP_TESTS OFF CACHE BOOL "" FORCE)
set(BUILD_BENCHMARKS OFF CACHE BOOL "" FORCE)
set(ENABLE_SANITIZER OFF CACHE BOOL "" FORCE)
set(ENABLE_UBSAN OFF CACHE BOOL "" FORCE)
```

`DISABLE_EXTENSION_LOAD` should not be enabled until verified. Even with static
extensions, DuckDB may still use the extension loading path to activate a
statically linked extension by name.

`DISABLE_BUILTIN_EXTENSIONS` should not be enabled in the first pass because
DuckDB's base config links `core_functions` and `parquet`, and Quack's upstream
config may rely on normal extension registration semantics. After static Quack
works, this can be revisited with a separate measurement.

`SMALLER_BINARY` may be enabled only if the test suite continues to pass and the
performance tradeoff is acceptable. It is not required for this change.

## Runtime Design

Connection startup will no longer execute `INSTALL quack`.

The driver will either:

1. execute `LOAD quack`, relying on DuckDB's generated static extension loader
   to activate the linked extension by name; or
2. skip both `INSTALL` and `LOAD` if verification shows Quack is auto-loaded
   and `ATTACH 'quack:...'` works immediately.

The first implementation should keep `LOAD quack` and remove only
`INSTALL quack`. This directly removes runtime download/install while using the
normal DuckDB activation path for statically linked extensions.

After loading Quack, the existing secret and attach logic remains unchanged.

## Tests

Tests should prove the behavior change without requiring a live Quack server.

Required coverage:

- connection initialization no longer attempts `INSTALL quack`;
- `LOAD quack` succeeds from the statically linked extension in the local
  DuckDB build;
- the ADBC shared library still exports only the expected ADBC entry points;
- existing URI parsing, SQL escaping, and Arrow stream tests continue to pass.

If `LOAD quack` requires vcpkg-provided `curl`/`openssl` libraries that are not
available in the local environment, the build should fail at configure/build
time with a clear dependency message rather than falling back to runtime
installation.

## Failure Modes

If Quack and DuckDB pins are incompatible, CMake configure or compile should
fail. The fix is to update the Quack pin or DuckDB pin together, not to re-enable
runtime `INSTALL quack`.

If Quack's transitive extension dependencies fail to build, the first fallback
is to use Quack's upstream extension config exactly. If that still fails, the
implementation should document the missing external dependency and stop.

If `LOAD quack` succeeds but `ATTACH 'quack:...'` fails without a server, the
driver should continue to return a normal ADBC error. The no-server path should
not crash or leak connection state.

## Non-Goals

- Runtime installation from `core_nightly`, `community`, or any other extension
  repository.
- A user-facing option to disable Quack vendoring.
- Packaging a separate `quack.duckdb_extension` next to the ADBC driver.
- Solving parameter binding or broader optional ADBC APIs.
- Reworking the URI contract.
