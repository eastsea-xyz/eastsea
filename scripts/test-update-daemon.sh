#!/usr/bin/env bash
# R11: fixture helpers only. No app launch or production-node/data access.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p tmp/swift-module-cache
root=$(pwd -P)
export TMPDIR="$root/tmp"
work=$(mktemp -d "$root/tmp/R11-node-identity.XXXXXX")
cat > "$work/R11-helper.c" <<'C'
#include <stdio.h>
#include <unistd.h>
int main(void) { puts(R11_LABEL); fflush(stdout); for (;;) pause(); }
C
/usr/bin/clang -DR11_LABEL='"old"' "$work/R11-helper.c" -o "$work/R11-old"
/usr/bin/clang -DR11_LABEL='"new"' "$work/R11-helper.c" -o "$work/R11-new"
/usr/bin/codesign --force --sign - --identifier com.pipln.eastsea.R11.fixture "$work/R11-old" "$work/R11-new"
python3 scripts/swift-test-cache.py --output "$work/R11-check" -- apps/wallet/Sources/NodeReleaseIdentity.swift apps/wallet/Tests/update-daemon/main.swift
"$work/R11-check" "$work/R11-old" "$work/R11-new" "$work"
