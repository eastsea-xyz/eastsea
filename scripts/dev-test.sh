#!/usr/bin/env bash
# Affected edit/test loop; remote runs never enter the local compile semaphore.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
mkdir -p "$root/tmp"
export TMPDIR="$root/tmp" PATH="$HOME/.cargo/bin:$PATH"
exec python3 "$root/scripts/dev-test.py" "$@"
