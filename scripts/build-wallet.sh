#!/usr/bin/env bash
# Build the wallet apps: Rust core (UniFFI) for macOS and iOS, Swift bindings, Xcode projects.
#   scripts/build-wallet.sh            macOS app (Release)
#   scripts/build-wallet.sh ios-sim    iOS Simulator app (Debug; software key, no Secure Enclave)
#   scripts/build-wallet.sh ios        iPhone app (Release; unsigned unless you pass signing settings)
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
target=${1:-macos}
# Release artifacts must be byte-identical wherever the checkout lives (gap G5).
. scripts/repro-env.sh
aether_repro_rustflags
# The Darwin linker otherwise records the section order it happened to pick,
# and a UUID that follows the build directory (see aether_repro_link_flags).
aether_repro_link_flags
# The macOS node pins the proving program (the sidecar's embedded guest ELF), so
# a node only verifies proofs of the program the protocol names.
# A macOS build without it would verify any program: refuse.
if [ "$target" = macos ]; then
  AETHER_PROVER_PROGRAM=$(scripts/prover-program.sh)
  export AETHER_PROVER_PROGRAM
  echo "proving program $AETHER_PROVER_PROGRAM"
fi
case "$target" in
  macos)  MACOSX_DEPLOYMENT_TARGET=14.0 cargo build -p aether-ffi -p aether-node --release --locked ;;
  ios-sim) IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build -p aether-ffi --release --locked --target aarch64-apple-ios-sim ;;
  ios)    IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build -p aether-ffi --release --locked --target aarch64-apple-ios ;;
  *) echo "usage: $0 [macos|ios-sim|ios]"; exit 1 ;;
esac
# Release gate (audit 6): the app bundles Resources/network.json, and on a
# new-genesis chain that file must ship with the coordinator's
# ceremony-check.json beside it, pinning its exact bytes — no app build may
# hand a consumer Mac an unchecked genesis. The 7780 testnet bundle ships no
# record and passes (not a new genesis).
if gate=$(cargo run -q --release --locked -p aether-node --bin aether -- \
  mainnet-rules --bundle --network apps/wallet/Resources/network.json 2>&1); then
  echo "bundled ceremony record gate: $(printf '%s\n' "$gate" | tail -1)"
else
  printf '%s\n' "$gate" | sed 's/^/  /' >&2
  echo "the app bundle fails the ceremony-record gate — see docs/ops/mainnet-launch.md 6단계" >&2
  exit 1
fi
# Release gate (audit 7 note): the drill and test seams must be compiled out of
# the node the app ships. A dev-drill build has the hidden `dev-b3` subcommand
# and the AETHER_DEV_* variable names; a test-seam build has set_test_readings.
if [ "$target" = macos ]; then
  if target/release/aether dev-b3 /dev/null >/dev/null 2>&1 \
    || strings target/release/aether | grep -qE 'AETHER_DEV_(PROTOCOL|UPGRADE_NOTICE)|set_test_readings'; then
    echo "target/release/aether contains dev-drill or test-seam code — a shipped node must not (crates/node/Cargo.toml [features])" >&2
    exit 1
  fi
  echo "dev feature gate: node binary has no dev-drill / test-seam code"
fi
# The node carries the linker's UUID, which follows the build directory: rewrite
# it from the code, so the copy the app embeds is the same bytes anywhere.
[ "$target" = macos ] && aether_repro_fix_uuid target/release/aether
# Bindings come from the host (macOS) build of the same crate.
[ -f target/release/libaether_ffi.dylib ] || MACOSX_DEPLOYMENT_TARGET=14.0 cargo build -p aether-ffi --release --locked
cargo run -q --locked -p aether-ffi --features bindgen --bin uniffi-bindgen -- generate --library target/release/libaether_ffi.dylib --language swift --out-dir apps/wallet/Generated
mv -f apps/wallet/Generated/aether_ffiFFI.modulemap apps/wallet/Generated/module.modulemap
# The macOS app embeds the node and the agent CLI (Contents/Helpers).
[ "$target" = macos ] && scripts/build-agent.sh >/dev/null
cd apps/wallet && xcodegen generate >/dev/null
swift_flags=()
if [ -n "${OTHER_SWIFT_FLAGS:-}" ]; then swift_flags+=("OTHER_SWIFT_FLAGS=$OTHER_SWIFT_FLAGS"); fi
case "$target" in
  macos)  xcodebuild -project AetherWallet.xcodeproj -scheme AetherWallet -configuration Release -derivedDataPath build "${swift_flags[@]}" build | grep -E "BUILD|error:" ;;
  ios-sim) xcodebuild -project AetherWallet.xcodeproj -scheme AetherWalletIOS -sdk iphonesimulator -configuration Debug -derivedDataPath build CODE_SIGNING_ALLOWED=NO build | grep -E "BUILD|error:" ;;
  ios)    xcodebuild -project AetherWallet.xcodeproj -scheme AetherWalletIOS -sdk iphoneos -destination 'generic/platform=iOS' -configuration Release -derivedDataPath build CODE_SIGNING_ALLOWED=NO build | grep -E "BUILD|error:" ;;
esac
