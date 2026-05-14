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

if [[ $# -ne 2 ]]; then
  printf 'usage: %s <linux|macos|windows> <amd64|arm64>\n' "$0" >&2
  exit 2
fi

platform="$1"
arch="$2"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../.." && pwd)"
build_dir="${repo_root}/build/ci-test-${platform}-${arch}"

export PIXI_CACHE_DIR="${PIXI_CACHE_DIR:-/tmp/adbc-driver-quack-pixi-cache}"

ctest=(ctest)
if command -v pixi >/dev/null 2>&1; then
  ctest=(pixi exec -s cmake ctest)
fi

"${ctest[@]}" --test-dir "$build_dir" --build-config Debug --output-on-failure
