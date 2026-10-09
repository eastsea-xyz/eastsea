#!/usr/bin/env bash
# Delete only idle targets managed by build-cache.py, oldest family first.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
exec python3 "$root/scripts/build-cache.py" prune "$@"
