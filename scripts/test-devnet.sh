#!/usr/bin/env bash
# Compile on the shared SSD cache, then execute devnet scenarios serially on RAM.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
FILTER=${1:-'binary(devnet) | binary(catchup_devnet) | binary(dkg) | binary(dkg_matrix) | binary(liveness) | binary(no_fork) | binary(robustness) | binary(selfheal)'}
if (( $# > 1 )); then echo 'usage: scripts/test-devnet.sh [nextest filter]' >&2; exit 2; fi
scripts/run-rust-tests.sh --ram -- -p aether-node -E "$FILTER" --test-threads 1
