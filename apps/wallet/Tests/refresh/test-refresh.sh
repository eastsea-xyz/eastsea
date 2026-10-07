#!/usr/bin/env bash
# Exercise WalletModel's actual refresh admission and completion statements.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
REFRESH_TMP="$ROOT/tmp/wallet-refresh"
mkdir -p "$REFRESH_TMP"
export TMPDIR="$REFRESH_TMP"
python3 - "$ROOT" "$REFRESH_TMP/main.swift" <<'EXTRACT'
from pathlib import Path
import sys
root = Path(sys.argv[1])
source = (root / 'apps/wallet/Sources/WalletModel.swift').read_text()
start = source.index('    func refresh() {') + len('    func refresh() {')
end = source.index('        Task.detached {', start)
admission = source[start:end]
start = source.index('            await MainActor.run {',
    source.index('            let verified = acc, readError = err', end))
start += len('            await MainActor.run {')
end = source.index('                // Published only', start)
completion = source[start:end]
fixture = r'''
import Foundation
final class RefreshFixture {
    var enclave: Int? = 1
    var lastKeyAttempt = Date()
    var address = "wallet"
    var validators: UInt32 = 4
    var networkGeneration: UInt64 = 0
    var refreshes = 0
    var refreshInFlight = false
    var jobs = 0
    func loadKey() {}
    func reconcileUnresolved() {}
    func refresh() { ADMISSION
        _ = (addr, n, generation, checkRecovery)
        jobs += 1
    }
    func complete(generation: UInt64) { COMPLETION }
}
func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL " + message); exit(1) }
}
let wallet = RefreshFixture()
for _ in 0..<20 { wallet.refresh() }
check(wallet.jobs == 1, "R19: timer queued \(wallet.jobs) refresh jobs while the first was still running")
wallet.complete(generation: 0)
wallet.refresh()
check(wallet.jobs == 2, "R19: a finished refresh releases the next refresh")
wallet.networkGeneration = 1
wallet.complete(generation: 0)
wallet.refresh()
check(wallet.jobs == 3, "R19: stale network completion also releases the refresh gate")
print("OK refresh single flight and completion")
'''
Path(sys.argv[2]).write_text(fixture.replace('ADMISSION', admission).replace('COMPLETION', completion))
EXTRACT
swiftc -module-cache-path "$REFRESH_TMP/module-cache" -o "$REFRESH_TMP/check" "$REFRESH_TMP/main.swift"
"$REFRESH_TMP/check"
