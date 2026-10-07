#!/usr/bin/env bash
# Product QA: render every Mac wallet screen and sheet to PNG with the design
# preview's sample data, in Korean and English, light and dark:
#   tmp/screens/<screen>-<ko|en>-<light|dark>.png
#   scripts/wallet-screens.sh            all screens
#   scripts/wallet-screens.sh sheet-     only the screens whose name starts so
# The renderer is its own app (WalletScreens, built with WALLET_SCREENS): it
# never starts a node, never reads or moves the real data folder, never opens
# the keychain, and runs with HOME pointed at a throwaway folder. It never
# launches EastSea.app. Needs the core library: scripts/build-wallet.sh first.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f target/release/libaether_ffi.a ] || { echo "no target/release/libaether_ffi.a: run scripts/build-wallet.sh first" >&2; exit 1; }
only=${1:-}
out="$PWD/tmp/screens"
cd apps/wallet && xcodegen generate >/dev/null
log="$out.build.log"; mkdir -p "$(dirname "$log")"
xcodebuild -project AetherWallet.xcodeproj -scheme WalletScreens -configuration Debug -derivedDataPath build \
  CODE_SIGN_IDENTITY=- DEVELOPMENT_TEAM= build > "$log" 2>&1 || true
# Never render with a stale renderer: a failed build stops here.
grep -q '\*\* BUILD SUCCEEDED \*\*' "$log" || { grep -E "error:" "$log" | head -20 >&2; echo "the screens renderer did not build ($log)" >&2; exit 1; }
bin=build/Build/Products/Debug/WalletScreens.app/Contents/MacOS/WalletScreens
[ -n "$only" ] || rm -rf "$out"
mkdir -p "$out"
home=$(mktemp -d "${TMPDIR:-/tmp}/wallet-screens-home.XXXXXX")
trap 'rm -rf "$home"' EXIT
for lang in en ko; do
  locale=$([ "$lang" = ko ] && echo ko_KR || echo en_US)
  args=(-AppleLanguages "($lang)" -AppleLocale "$locale" -out "$out")
  [ -n "$only" ] && args+=(-only "$only")
  HOME="$home" CFFIXED_USER_HOME="$home" "$bin" "${args[@]}"
done
echo "screens: $(ls "$out"/*.png | wc -l | tr -d ' ') PNGs in tmp/screens"
# Every Korean render must read Korean (scripts/check-wallet-screens-language.py).
/usr/bin/python3 ../../scripts/check-wallet-screens-language.py "$out"
