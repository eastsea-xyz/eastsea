#!/usr/bin/env bash
# Keep the semaphore owner alive for the entire build, with a bounded queue wait.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
mkdir -p "$root/tmp"
exec python3 "$root/scripts/compile-gate.py" "$@"
