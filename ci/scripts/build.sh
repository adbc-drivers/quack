#!/usr/bin/env bash
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

set -euo pipefail

if [[ $# -ne 3 ]]; then
  printf 'usage: %s <test|release> <linux|macos|windows> <amd64|arm64>\n' "$0" >&2
  exit 2
fi

mode="$1"
platform="$2"
arch="$3"

case "$mode" in
  test)
    cmake_config="Debug"
    ;;
  release)
    cmake_config="Release"
    ;;
  *)
    printf 'unsupported build mode: %s\n' "$mode" >&2
    exit 2
    ;;
esac

case "$platform" in
  linux)
    library_ext="so"
    ;;
  macos)
    library_ext="dylib"
    ;;
  windows)
    library_ext="dll"
    ;;
  *)
    printf 'unsupported platform: %s\n' "$platform" >&2
    exit 2
    ;;
esac

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../.." && pwd)"
build_dir="${repo_root}/build/ci-${mode}-${platform}-${arch}"
output_library="${repo_root}/build/libadbc_driver_quack.${library_ext}"

export PIXI_CACHE_DIR="${PIXI_CACHE_DIR:-/tmp/adbc-driver-quack-pixi-cache}"

cmake=(cmake)
generator_args=()
if command -v pixi >/dev/null 2>&1; then
  cmake=(pixi exec -s cmake -s ninja cmake)
  if [[ "$platform" != "windows" ]]; then
    generator_args=(-G Ninja)
  fi
elif [[ "$platform" != "windows" ]] && command -v ninja >/dev/null 2>&1; then
  generator_args=(-G Ninja)
fi

configure_args=(
  -S "$repo_root"
  -B "$build_dir"
  -DCMAKE_BUILD_TYPE="$cmake_config"
  -DBUILD_TESTING=ON
)

"${cmake[@]}" "${configure_args[@]}" "${generator_args[@]}"
"${cmake[@]}" --build "$build_dir" --config "$cmake_config" --parallel

built_library="$(
  find "$build_dir" \
    -type f \
    \( -name "libadbc_driver_quack.${library_ext}" -o -name "adbc_driver_quack.${library_ext}" \) \
    -print \
    -quit
)"

if [[ -z "$built_library" ]]; then
  printf 'could not find built quack driver in %s\n' "$build_dir" >&2
  exit 1
fi

mkdir -p "${repo_root}/build"
cp "$built_library" "$output_library"
chmod 755 "$output_library"
printf 'Built %s\n' "$output_library"
