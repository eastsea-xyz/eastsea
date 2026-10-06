#!/bin/bash
# Guarded, serialized builds; every temporary artifact stays under this tree.
set -euo pipefail
CONTRACTS_ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$CONTRACTS_ROOT"
mkdir -p "$CONTRACTS_ROOT/tmp"
export TMPDIR="$CONTRACTS_ROOT/tmp"
export CARGO_TARGET_DIR="$CONTRACTS_ROOT/tmp/target.noindex"
export PATH="$HOME/.cargo/bin:$PATH"
export RAYON_NUM_THREADS=4
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CONTRACTS_ONCHAIN_METRICS="$CONTRACTS_ROOT/tmp/contracts-onchain-metrics.jsonl"
WITH_NODE=false
case "${1:-}" in
    '') ;;
    --with-node) WITH_NODE=true ;;
    *) echo 'Usage: scripts/run-contracts-onchain.sh [--with-node]' >&2; exit 64 ;;
esac

resource_guard() {
    # A denied process lookup is not evidence that the host is idle.
    while true; do
        set +e
        pgrep -x rustc > "$TMPDIR/rustc-processes" 2> "$TMPDIR/rustc-process-error"
        rustc_status=$?
        pgrep -f 'cargo (build|test)' > "$TMPDIR/cargo-processes" 2> "$TMPDIR/cargo-process-error"
        cargo_status=$?
        set -e
        if (( rustc_status > 1 || cargo_status > 1 )); then
            cat "$TMPDIR/rustc-process-error" "$TMPDIR/cargo-process-error" >&2
            echo 'Resource guard cannot read host process state; Cargo was not launched.' >&2
            exit 75
        fi
        free=$(memory_pressure | tail -1 | sed -n 's/.*: \([0-9][0-9]*\)%.*/\1/p')
        if [[ -z "$free" ]]; then
            echo 'Resource guard cannot read memory pressure; Cargo was not launched.' >&2
            exit 75
        fi
        if (( rustc_status == 1 && cargo_status == 1 && free >= 25 )); then return; fi
        echo "Waiting for idle Cargo/rustc and >=25% free memory (free=$free%)." >&2
        sleep 60
    done
}

resource_guard
python3 scripts/generate-contract-fixtures.py --offline
resource_guard
: > "$CONTRACTS_ONCHAIN_METRICS"
cargo test -p aether-contracts-onchain -j 4 --test contracts_onchain -- --test-threads=1 --nocapture 2>&1 | tee "$TMPDIR/contracts-onchain-test.txt"
if "$WITH_NODE"; then
    resource_guard
    cargo test -p aether-node -j 4 --test state_budget --test zero_fee -- --test-threads=1 2>&1 | tee "$TMPDIR/contracts-onchain-node-test.txt"
fi
FORGE=$(command -v forge || true)
if [[ -z "$FORGE" ]]; then FORGE="$HOME/.foundry/bin/forge"; fi
"$FORGE" test --root "$CONTRACTS_ROOT/contracts" --offline --threads 4 2>&1 | tee "$TMPDIR/contracts-onchain-forge-core.txt"
"$FORGE" test --root "$CONTRACTS_ROOT/crates/contracts-onchain/fixtures/toolbox" --offline --threads 4 2>&1 | tee "$TMPDIR/contracts-onchain-forge-toolbox.txt"
python3 scripts/contracts-onchain-report.py --metrics "$CONTRACTS_ONCHAIN_METRICS"
# Preserve the checked-in report and gitignored Foundry artifacts; remove all
# task scratch and Rust build products only after the report has been written.
python3 - <<'PY'
from pathlib import Path
import shutil
root = Path.cwd().resolve()
scratch = root / 'tmp'
assert scratch.parent == root and scratch.name == 'tmp' and not scratch.is_symlink()
shutil.rmtree(scratch)
PY
