#!/usr/bin/env bash
# R11: fixture helpers only. No app launch or production-node/data access.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p tmp/swift-module-cache
root=$(pwd -P)
export TMPDIR="$root/tmp"
work=$(mktemp -d "$root/tmp/R11-node-identity.XXXXXX")
phase=prepare
gate="$root/scripts/compile-gate.sh"
report_failure() {
    local fixture_status=$1
    if [ "$fixture_status" -ne 0 ]; then
        printf 'FAIL R11 daemon fixture: phase=%s exit=%s work=%s\n' "$phase" "$fixture_status" "$work" >&2
    fi
}
trap 'report_failure "$?"' EXIT
cat > "$work/R11-helper.c" <<'C'
#include <stdio.h>
#include <unistd.h>
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    char pending[4096];
    if (snprintf(pending, sizeof(pending), "%s.%d", argv[1], getpid()) >= sizeof(pending)) return 3;
    FILE *ready = fopen(pending, "w");
    if (!ready) { perror("R11 daemon readiness open"); return 4; }
    fprintf(ready, "%d %s\n", getpid(), R11_LABEL);
    if (fclose(ready) != 0 || rename(pending, argv[1]) != 0) { perror("R11 daemon readiness publish"); return 5; }
    for (;;) pause();
}
C
phase='compile old helper'
"$gate" /usr/bin/clang -DR11_LABEL='"old"' "$work/R11-helper.c" -o "$work/R11-old"
phase='compile new helper'
"$gate" /usr/bin/clang -DR11_LABEL='"new"' "$work/R11-helper.c" -o "$work/R11-new"
phase='sign helpers'
/usr/bin/codesign --force --sign - --identifier com.pipln.eastsea.R11.fixture "$work/R11-old" "$work/R11-new"
phase='compile Swift checker'
printf 'R11 daemon fixture: %s (work=%s)\n' "$phase" "$work" >&2
python3 scripts/swift-test-cache.py --output "$work/R11-check" -- apps/wallet/Sources/NodeReleaseIdentity.swift apps/wallet/Tests/update-daemon/main.swift
phase='run Swift checker'
"$work/R11-check" "$work/R11-old" "$work/R11-new" "$work"
