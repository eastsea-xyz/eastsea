#!/usr/bin/env bash
# Build the wallet apps: Rust core (UniFFI) for macOS and iOS, Swift bindings, Xcode projects.
#   scripts/build-wallet.sh            macOS app (Release)
#   scripts/build-wallet.sh ios-sim    iOS Simulator app (Debug; software key, no Secure Enclave)
#   scripts/build-wallet.sh ios        iPhone app (Release; unsigned unless you pass signing settings)
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
target=${1:-macos}
case "$target" in
  macos)  MACOSX_DEPLOYMENT_TARGET=14.0 cargo build -p aether-ffi --release ;;
  ios-sim) IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build -p aether-ffi --release --target aarch64-apple-ios-sim ;;
  ios)    IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build -p aether-ffi --release --target aarch64-apple-ios ;;
  *) echo "usage: $0 [macos|ios-sim|ios]"; exit 1 ;;
esac
# Bindings come from the host (macOS) build of the same crate.
[ -f target/release/libaether_ffi.dylib ] || MACOSX_DEPLOYMENT_TARGET=14.0 cargo build -p aether-ffi --release
cargo run -q -p aether-ffi --bin uniffi-bindgen -- generate --library target/release/libaether_ffi.dylib --language swift --out-dir apps/wallet/Generated
mv -f apps/wallet/Generated/aether_ffiFFI.modulemap apps/wallet/Generated/module.modulemap
cd apps/wallet && xcodegen generate >/dev/null
case "$target" in
  macos)  xcodebuild -project AetherWallet.xcodeproj -scheme AetherWallet -configuration Release -derivedDataPath build build | grep -E "BUILD|error:" ;;
  ios-sim) xcodebuild -project AetherWallet.xcodeproj -scheme AetherWalletIOS -sdk iphonesimulator -configuration Debug -derivedDataPath build CODE_SIGNING_ALLOWED=NO build | grep -E "BUILD|error:" ;;
  ios)    xcodebuild -project AetherWallet.xcodeproj -scheme AetherWalletIOS -sdk iphoneos -destination 'generic/platform=iOS' -configuration Release -derivedDataPath build CODE_SIGNING_ALLOWED=NO build | grep -E "BUILD|error:" ;;
esac
