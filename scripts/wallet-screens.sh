#!/usr/bin/env bash
# Product QA renders every wallet screen with isolated fixtures: all five
# languages in light mode, plus English/Korean dark mode. Each PNG has visible
# text from Vision OCR next to it for check-wallet-screens-language.py.
#   scripts/wallet-screens.sh [screen-prefix]
# Account states: switcher, two-accounts, retire-blocked, menubar-qr.
# Browser: browser-start, browser-tabs (three tabs), browser-tabs-narrow,
# browser-permissions, browser-find. Fixtures never load an external page.
# Never launches EastSea.app or reads its real data, node, or keychain.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
mkdir -p "$root/tmp"
export TMPDIR="$root/tmp"
[ -f target/release/libaether_ffi.a ] || { echo "no target/release/libaether_ffi.a: run scripts/build-wallet.sh first" >&2; exit 1; }
only=${1:-}
out="$root/tmp/screens"

cd "$root/apps/wallet"
xcodegen generate >/dev/null
xcodebuild -project AetherWallet.xcodeproj -scheme WalletScreens -configuration Debug \
  -derivedDataPath "$root/tmp/wallet-screens-build" \
  CODE_SIGN_IDENTITY=- DEVELOPMENT_TEAM= build >"$root/tmp/wallet-screens-build.log" 2>&1 || {
    tail -80 "$root/tmp/wallet-screens-build.log" >&2
    exit 1
  }
bin="$root/tmp/wallet-screens-build/Build/Products/Debug/WalletScreens.app/Contents/MacOS/WalletScreens"
[ -x "$bin" ] || { echo "the screens renderer did not build" >&2; exit 1; }
mkdir -p "$out"
if [ -z "$only" ]; then
  shopt -s nullglob
  rm -f "$out"/*.png "$out"/*.text.json "$out"/*.txt
fi
fixture_home=$(mktemp -d "$root/tmp/wallet-screens-home.XXXXXX")
trap 'rm -rf "$fixture_home"' EXIT
for lang in en ko ja zh-Hans zh-Hant; do
  case "$lang" in
    en) locale=en_US ;;
    ko) locale=ko_KR ;;
    ja) locale=ja_JP ;;
    zh-Hans) locale=zh_CN ;;
    zh-Hant) locale=zh_TW ;;
  esac
  args=(-AppleLanguages "($lang)" -AppleLocale "$locale" -out "$out")
  [ -n "$only" ] && args+=(-only "$only")
  # CoreFoundation/NSHomeDirectory/UserDefaults use the throwaway home;
  # HOME itself is not reassigned. WALLET_SCREENS compiles out real-data work.
  WALLET_SCREEN_FIXTURE_ROOT="$fixture_home" CFFIXED_USER_HOME="$fixture_home" "$bin" "${args[@]}"
done
cd "$root"
check_args=(--out "$out")
[ -n "$only" ] && check_args+=(--only "$only")
/usr/bin/python3 scripts/check-wallet-screens-language.py "${check_args[@]}"
