#!/usr/bin/env bash
# Unit test for reproducible build concepts:
# 1. Deterministic ZIP packaging (normalized timestamps, permissions, order)
# 2. Mach-O LC_UUID invariance and deterministic signature stripping
set -euo pipefail

echo "=== [1/2] Testing Deterministic ZIP Packaging ==="
TMP_DIR=$(mktemp -d /tmp/repro-test-XXXXXX)
trap 'rm -rf "$TMP_DIR"' EXIT

DIR_A="$TMP_DIR/build_a"
DIR_B="$TMP_DIR/build_b"
mkdir -p "$DIR_A/src" "$DIR_B/src"

# Write identical file contents
echo 'console.log("hello world");' > "$DIR_A/src/index.js"
echo '{"name": "test-pkg", "version": "1.0.0"}' > "$DIR_A/package.json"

echo 'console.log("hello world");' > "$DIR_B/src/index.js"
echo '{"name": "test-pkg", "version": "1.0.0"}' > "$DIR_B/package.json"

# Intentionally alter timestamps between A and B
touch -t 202101010000 "$DIR_A/src/index.js" "$DIR_A/package.json"
touch -t 202405051200 "$DIR_B/src/index.js" "$DIR_B/package.json"

# Python function to pack deterministically
pack_deterministic() {
  local src_dir="$1"
  local out_zip="$2"
  python3 -c "
import os, sys, zipfile

src_dir = sys.argv[1]
out_zip = sys.argv[2]
fixed_time = (2026, 1, 1, 0, 0, 0)

with zipfile.ZipFile(out_zip, 'w', compression=zipfile.ZIP_DEFLATED) as zf:
    for root, dirs, files in os.walk(src_dir):
        dirs.sort()
        for f in sorted(files):
            full_path = os.path.join(root, f)
            arc_name = os.path.relpath(full_path, src_dir)
            with open(full_path, 'rb') as fp:
                data = fp.read()
            zi = zipfile.ZipInfo(arc_name, fixed_time)
            zi.external_attr = (0o644 << 16)
            zf.writestr(zi, data)
" "$src_dir" "$out_zip"
}

pack_deterministic "$DIR_A" "$TMP_DIR/a.zip"
pack_deterministic "$DIR_B" "$TMP_DIR/b.zip"

HASH_A=$(shasum -a 256 "$TMP_DIR/a.zip" | awk '{print $1}')
HASH_B=$(shasum -a 256 "$TMP_DIR/b.zip" | awk '{print $1}')

echo "Archive A SHA-256: $HASH_A"
echo "Archive B SHA-256: $HASH_B"

if [ "$HASH_A" != "$HASH_B" ]; then
  echo "FAIL: Deterministic ZIP packaging hash mismatch!"
  exit 1
fi
echo "SUCCESS: Deterministic ZIP packaging hashes match perfectly."

echo "=== [2/2] Testing Mach-O Signature Removal and LC_UUID Invariance ==="
mkdir -p "$TMP_DIR/node1" "$TMP_DIR/node2"
cat << 'EOF' > "$TMP_DIR/main.c"
#include <stdio.h>
int main(void) {
    puts("aether-node deterministic verification");
    return 0;
}
EOF

# Compile to object file
clang -c -O2 "$TMP_DIR/main.c" -o "$TMP_DIR/main.o"

# Link identically in two separate directory trees (simulating release engineer vs auditor)
LD="/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/ld"
$LD -dynamic -arch arm64 -platform_version macos 14.0 14.0 \
    -syslibroot /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk \
    -o "$TMP_DIR/node1/aether-bin" "$TMP_DIR/main.o" -lSystem >/dev/null 2>&1
$LD -dynamic -arch arm64 -platform_version macos 14.0 14.0 \
    -syslibroot /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk \
    -o "$TMP_DIR/node2/aether-bin" "$TMP_DIR/main.o" -lSystem >/dev/null 2>&1

# Official release signs node1 with official certificate (simulated by ad-hoc sign 1)
codesign -s - --force "$TMP_DIR/node1/aether-bin" >/dev/null 2>&1

# Independent auditor builds on their machine and ad-hoc signs node2
sleep 1
codesign -s - --force "$TMP_DIR/node2/aether-bin" >/dev/null 2>&1

# While signed, their raw hashes may differ due to timestamps and signing identities
HASH_SIGNED_1=$(shasum -a 256 "$TMP_DIR/node1/aether-bin" | awk '{print $1}')
HASH_SIGNED_2=$(shasum -a 256 "$TMP_DIR/node2/aether-bin" | awk '{print $1}')
echo "Official signed binary SHA-256: $HASH_SIGNED_1"
echo "Auditor signed binary SHA-256:  $HASH_SIGNED_2"

# 1. Compare LC_UUID
UUID_1=$(otool -l "$TMP_DIR/node1/aether-bin" | grep -A 2 LC_UUID | grep uuid | awk '{print $2}')
UUID_2=$(otool -l "$TMP_DIR/node2/aether-bin" | grep -A 2 LC_UUID | grep uuid | awk '{print $2}')
echo "Official LC_UUID: $UUID_1"
echo "Auditor  LC_UUID: $UUID_2"

if [ "$UUID_1" != "$UUID_2" ]; then
  echo "FAIL: LC_UUID mismatch between independent builds!"
  exit 1
fi
echo "SUCCESS: LC_UUID matches between official release and auditor build."

# 2. Strip signatures and compare bit-for-bit SHA-256
codesign --remove-signature "$TMP_DIR/node1/aether-bin"
codesign --remove-signature "$TMP_DIR/node2/aether-bin"

STRIPPED_1=$(shasum -a 256 "$TMP_DIR/node1/aether-bin" | awk '{print $1}')
STRIPPED_2=$(shasum -a 256 "$TMP_DIR/node2/aether-bin" | awk '{print $1}')
echo "Official stripped SHA-256: $STRIPPED_1"
echo "Auditor  stripped SHA-256: $STRIPPED_2"

if [ "$STRIPPED_1" != "$STRIPPED_2" ]; then
  echo "FAIL: Stripped binary hashes do not match!"
  exit 1
fi
echo "SUCCESS: Stripped binaries match 100% bit-for-bit."

echo "=== All Reproducibility Unit Tests Passed ==="
