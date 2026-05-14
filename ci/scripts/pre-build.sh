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

platform="$2"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../.." && pwd)"
env_file="${repo_root}/.env.build"

{
  if [[ "$platform" == "macos" ]]; then
    printf 'MACOSX_DEPLOYMENT_TARGET=11.0\n'
  fi
} >"$env_file"
