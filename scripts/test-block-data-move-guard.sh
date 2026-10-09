#!/usr/bin/env bash
# Run the real readiness fixture with only its answered guard removed.
# The mutant and all compiler artifacts stay in this worktree's tmp.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
mkdir -p "$root/tmp/swift-module-cache"
export TMPDIR="$root/tmp"
fixture="$(mktemp -d "$TMPDIR/move-fresh-mutant.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT
export AETHER_AGENT_TEST_TMP="$fixture"
export WALLET_TEST_BUNDLE="$TMPDIR/wallet-languages/WalletLocalizations.bundle"
/usr/bin/python3 scripts/wallet-l10n.py prepare-tests --out "$WALLET_TEST_BUNDLE"
/usr/bin/python3 - "$fixture/BlockDataMove.swift" <<'PY'
from pathlib import Path
import sys
source = Path("apps/wallet/Sources/BlockDataMove.swift").read_text()
guard = "              answered, let height else { return false }"
assert source.count(guard) == 1, "the answered guard must be a unique mutation point"
Path(sys.argv[1]).write_text(source.replace(guard, "              let height else { return false }"))
PY
W=apps/wallet/Sources
"$HOME/.claude/playbooks/aether-team/wait-compile.sh" && swiftc -Onone \
  -module-cache-path "$TMPDIR/swift-module-cache" -o "$fixture/test" \
  "$W/Brand.swift" "$W/Clock.swift" "$W/NodeWatchdog.swift" "$W/NodeStopReason.swift" \
  "$W/UnattendedDecision.swift" "$W/ArchiveMeasurement.swift" "$W/BlockDataLocation.swift" \
  "$W/KeySafety.swift" "$W/DataMigration.swift" "$fixture/BlockDataMove.swift" "$W/NodeStorageMove.swift" \
  "$W/AppLanguage.swift" apps/wallet/Tests/LocalizationTestSupport.swift apps/wallet/Tests/block-data/main.swift
if "$fixture/test" >"$fixture/result.out" 2>&1; then
  echo "FAIL answered-guard mutant passed; the deletion regression is not protected"
  exit 1
fi
if ! rg -q 'FAIL delete-only-after-answer: no answer must retain old data' "$fixture/result.out"; then
  cat "$fixture/result.out"
  echo "FAIL mutant did not fail at the intended deletion guard"
  exit 1
fi
cat "$fixture/result.out"
echo "PASS removing the answered guard fails the old-data deletion regression"
