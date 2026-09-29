#!/usr/bin/env bash
# Tests for the reproducible-build scripts (gap G5). No cargo/Xcode build: these
# check the pieces the release scripts rely on, so a regression fails fast.
#
#   scripts/test-repro-scripts.sh
#
# 1. repro-env.sh pins SOURCE_DATE_EPOCH/ZERO_AR_DATE and derives the remap list
#    from the checkout and target dir (so the flags follow the build location).
# 2. deterministic-zip.py packs the same content to the same bytes whatever the
#    mtimes and order, and to different bytes when the content changes.
# 3. repro-app-check.sh calls two identically built bundles reproducible when
#    only their signatures differ, and not reproducible when the code differs.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/aether-repro-test-XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "ok: $*"; }

echo "=== [1/3] repro-env.sh ==="
# Fresh shell so the parent environment cannot leak in.
env -u SOURCE_DATE_EPOCH -u ZERO_AR_DATE -u RUSTFLAGS -u AETHER_REMAP_FLAGS \
  CARGO_TARGET_DIR="$tmp/target-one" bash -c '
    set -euo pipefail
    . "$1/scripts/repro-env.sh"
    case "$SOURCE_DATE_EPOCH" in ""|*[!0-9]*) echo "bad SOURCE_DATE_EPOCH: $SOURCE_DATE_EPOCH"; exit 1 ;; esac
    [ "$ZERO_AR_DATE" = 1 ] || { echo "ZERO_AR_DATE not 1"; exit 1; }
    for p in "$1=/aether-node" "$2=/aether-target"; do
      case "$AETHER_REMAP_FLAGS" in *"$p"*) ;; *) echo "missing remap $p in: $AETHER_REMAP_FLAGS"; exit 1 ;; esac
    done
    RUSTFLAGS="--cfg keep_me"
    aether_repro_rustflags "--remap-path-prefix=/x=/y"
    case "$RUSTFLAGS" in
      "--remap-path-prefix=/x=/y --cfg keep_me"*) ;;
      *) echo "aether_repro_rustflags lost the caller flags: $RUSTFLAGS"; exit 1 ;;
    esac
  ' _ "$repo" "$tmp/target-one" || fail "repro-env.sh"
pass "pins the epoch, zeroes archive dates, keeps caller RUSTFLAGS"

echo "=== [2/3] deterministic-zip.py ==="
src_a="$tmp/src_a" src_b="$tmp/src_b"
mkdir -p "$src_a/src" "$src_a/wasm"
printf 'console.log("hi");\n' > "$src_a/src/a.js"
printf 'wasm-bytes\n' > "$src_a/wasm/a.wasm"
printf '{"v":1}\n' > "$src_a/manifest.json"
printf 'test\n' > "$src_a/src/a.test.js"
# Same content, different mtimes, written in a different order.
mkdir -p "$src_b"
cp "$src_a/manifest.json" "$src_b/manifest.json"
cp -R "$src_a/wasm" "$src_b/wasm"
cp -R "$src_a/src" "$src_b/src"
touch -t 202001010000 "$src_a"/manifest.json "$src_a"/src/*.js "$src_a"/src/*.test.js "$src_a"/wasm/*
touch -t 202406060606 "$src_b"/manifest.json "$src_b"/src/*.js "$src_b"/src/*.test.js "$src_b"/wasm/*

SOURCE_DATE_EPOCH=1767225600 python3 scripts/deterministic-zip.py "$tmp/a.zip" "$src_a" manifest.json src wasm
SOURCE_DATE_EPOCH=1767225600 python3 scripts/deterministic-zip.py "$tmp/b.zip" "$src_b" manifest.json src wasm
ha=$(shasum -a 256 "$tmp/a.zip" | awk '{print $1}')
hb=$(shasum -a 256 "$tmp/b.zip" | awk '{print $1}')
[ "$ha" = "$hb" ] || fail "zip differs between identical content: $ha vs $hb"
python3 - "$tmp/a.zip" <<'PY' || fail "zip contents"
import sys, zipfile
names = sorted(zipfile.ZipFile(sys.argv[1]).namelist())
want = ["manifest.json", "src/a.js", "wasm/a.wasm"]  # a.test.js excluded
assert names == want, f"{names} != {want}"
PY
# A content change must change the archive (guards against a packer that
# silently drops files).
printf 'changed\n' > "$src_b/src/a.js"
SOURCE_DATE_EPOCH=1767225600 python3 scripts/deterministic-zip.py "$tmp/c.zip" "$src_b" manifest.json src wasm
hc=$(shasum -a 256 "$tmp/c.zip" | awk '{print $1}')
[ "$ha" != "$hc" ] || fail "zip did not change with the content"
pass "same content -> same bytes, .test. left out, changes show up"

echo "=== [3/3] repro-app-check.sh ==="
cat > "$tmp/main.c" <<'EOF'
#include <stdio.h>
int main(void) { puts("aether reproducible bundle"); return 0; }
EOF
mk_bundle() { # mk_bundle DIR
  mkdir -p "$1/Contents/MacOS"
  clang -O2 "$tmp/main.c" -o "$1/Contents/MacOS/AetherWallet"
}
mk_bundle "$tmp/A.app"
mk_bundle "$tmp/B.app"
# Only the signature differs (different identifiers), as in a release vs an
# independent rebuild.
codesign -s - -i com.aether.test.one --force "$tmp/A.app/Contents/MacOS/AetherWallet" >/dev/null 2>&1
sleep 1
codesign -s - -i com.aether.test.two --force "$tmp/B.app/Contents/MacOS/AetherWallet" >/dev/null 2>&1
sa=$(shasum -a 256 "$tmp/A.app/Contents/MacOS/AetherWallet" | awk '{print $1}')
sb=$(shasum -a 256 "$tmp/B.app/Contents/MacOS/AetherWallet" | awk '{print $1}')
[ "$sa" != "$sb" ] || fail "the two signed copies should differ before stripping"
scripts/repro-app-check.sh "$tmp/A.app" "$tmp/B.app" >/dev/null || fail "repro-app-check rejected identically built bundles"
pass "signature-only difference reported reproducible"

# Different code must be reported as a difference (exit 1).
cat > "$tmp/other.c" <<'EOF'
#include <stdio.h>
int main(void) { puts("aether bundle with a change"); return 0; }
EOF
mkdir -p "$tmp/C.app/Contents/MacOS"
clang -O2 "$tmp/other.c" -o "$tmp/C.app/Contents/MacOS/AetherWallet"
if scripts/repro-app-check.sh "$tmp/A.app" "$tmp/C.app" >"$tmp/c.log" 2>&1; then
  cat "$tmp/c.log" >&2
  fail "repro-app-check accepted two different bundles"
fi
pass "a real code difference is reported (exit 1)"

echo
echo "all reproducible-build script tests passed"
