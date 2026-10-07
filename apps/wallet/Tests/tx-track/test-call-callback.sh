#!/usr/bin/env bash
# Compile WalletModel's real callback adapter with fake local dependencies.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
TEST_TMP="$REPO_ROOT/tmp/tx-track-call-callback"
mkdir -p "$TEST_TMP"
export TMPDIR="$TEST_TMP"
python3 - "$REPO_ROOT" "$TEST_TMP/main.swift" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1])
source = (root / "apps/wallet/Sources/WalletModel.swift").read_text()
fixture = (root / "apps/wallet/Tests/tx-track/call-callback-fixture.swift").read_text()

def method(signature):
    start = source.index(signature)
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        if source[end] == "{":
            depth += 1
        elif source[end] == "}":
            depth -= 1
        end += 1
    return source[start:end]

for marker, signature in (
    ("INSERT_APPROVE_CALL", "    func approveCall()"),
    ("INSERT_REPLY", "    private func reply("),
    ("INSERT_TRACK", "    private func track("),
):
    fixture = fixture.replace("    // " + marker, method(signature))
Path(sys.argv[2]).write_text(fixture)
PY
swiftc -parse-as-library -module-cache-path "$TEST_TMP/module-cache" \
  -o "$TEST_TMP/check" "$REPO_ROOT/apps/wallet/Sources/TxTrack.swift" "$TEST_TMP/main.swift"
"$TEST_TMP/check"
