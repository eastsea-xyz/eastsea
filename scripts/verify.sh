#!/usr/bin/env bash
# Whole-tree gate: run this on the merged tree before anything lands on main.
#   scripts/verify.sh            everything (about an hour on a loaded Mac)
#   scripts/verify.sh rust       Rust tests + clippy only
#   scripts/verify.sh apps       contracts, extension, fuzz build, bindings, Xcode builds
#   scripts/verify.sh rehearsal  the mainnet-genesis rehearsal with a pinned proving sidecar
# Why it exists: on 2026-09-30 five merged branches were checked only by their own
# test files, and the first whole-workspace run three days later found a broken
# macOS build, stale bindings and four failing tests. Every step prints PASS/FAIL;
# the exit status is the number of failed steps.
set -uo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
what=${1:-all}
fails=0
step() { # step <name> <command...>
  local name=$1; shift
  if "$@" >"tmp/verify-$name.log" 2>&1; then echo "PASS  $name"; else echo "FAIL  $name  (tmp/verify-$name.log)"; fails=$((fails + 1)); fi
}
mkdir -p tmp
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$PWD/target/verify"}

if [ "$what" = all ] || [ "$what" = rust ]; then
  step rust-tests cargo test --workspace --no-fail-fast
  step clippy cargo clippy --workspace --all-targets -- -D clippy::correctness
fi

if [ "$what" = all ] || [ "$what" = apps ]; then
  step forge bash -c 'cd contracts && forge test'
  step extension-build scripts/build-extension.sh
  step extension-tests bash -c 'cd apps/extension && node --test test/*.test.mjs'
  step fuzz-build bash -c 'cd fuzz && AETHER_PROVER_PROGRAM=$(../scripts/prover-program.sh) CARGO_TARGET_DIR=$PWD/../tmp/cargo-target cargo +nightly fuzz build'
  # Generated bindings must match the Rust API (whitespace aside).
  gen=$(mktemp -d)
  step bindings bash -c "cargo build -p aether-ffi && cargo run -q -p aether-ffi --features bindgen --bin uniffi-bindgen -- generate --library \$CARGO_TARGET_DIR/debug/libaether_ffi.dylib --language swift --out-dir $gen && diff -wB $gen/aether_ffi.swift apps/wallet/Generated/aether_ffi.swift && diff -wB $gen/aether_ffiFFI.h apps/wallet/Generated/aether_ffiFFI.h"
  # Static libraries the Xcode targets link (both stale-library failures we hit were silent until link time).
  step ffi-libs bash -c 'unset CARGO_TARGET_DIR; cargo build --release -p aether-ffi && cargo build --release -p aether-ffi --target aarch64-apple-ios-sim'
  step xcodegen bash -c 'cd apps/wallet && xcodegen generate && git diff --quiet -- AetherWallet.xcodeproj || { echo "project.pbxproj is stale: commit the regenerated project"; exit 1; }'
  step xcode-mac bash -c 'cd apps/wallet && xcodebuild -project AetherWallet.xcodeproj -scheme AetherWallet -configuration Debug build CODE_SIGNING_ALLOWED=NO | grep -q "BUILD SUCCEEDED"'
  step xcode-ios bash -c "cd apps/wallet && xcodebuild -project AetherWallet.xcodeproj -scheme AetherWalletIOS -destination 'generic/platform=iOS Simulator' -configuration Debug build CODE_SIGNING_ALLOWED=NO | grep -q 'BUILD SUCCEEDED'"
  # Pure Swift tests (the table of sources per test lives in the script).
  step swift-pure scripts/test-swift-pure.sh
  # Every Tests/<dir> must be in that table.
  step swift-pure-coverage bash -c 'for d in apps/wallet/Tests/*/; do n=$(basename $d); grep -q "^run $n " scripts/test-swift-pure.sh || { echo "missing in scripts/test-swift-pure.sh: $n"; exit 1; }; done'
fi

if [ "$what" = all ] || [ "$what" = rehearsal ]; then
  # The end-to-end check the unit and integration tests cannot give: the exact
  # mainnet genesis flags through `aether network` -> `aether dkg` -> `aether run`,
  # with the proving sidecar pinned like a release (a second Mac found, on
  # 2026-10-03, that dkg dropped the genesis protocol while every test passed).
  step rehearsal bash -c 'export AETHER_PROVER_PROGRAM=$(scripts/prover-program.sh) && cargo build --release -p aether-node --bin aether && d=$PWD/tmp/rehearsal-bin && mkdir -p $d && cp "$CARGO_TARGET_DIR/release/aether" $d/aether && cp "$CARGO_TARGET_DIR/release/aether-prover" $d/aether-prover && rm -rf tmp/rehearsal-run && AETHER_BIN=$d/aether scripts/mainnet-rehearsal.sh $PWD/tmp/rehearsal-run'
  # The launch ceremony tool, against the binary the rehearsal just built.
  step genesis-tool bash -c 'AETHER_BIN=$PWD/tmp/rehearsal-bin/aether scripts/test-mainnet-genesis.sh'
fi

echo "verify: $fails failed step(s)"
exit "$fails"
