// Checks the send sheet's displayed fee maximum (pre-audit 7, M1 — the exact
// wei the signature re-checks against a fresh quote at send time):
//   swiftc -o /tmp/fee-confirm-check apps/wallet/Sources/EarningsModel.swift apps/wallet/Tests/fee-confirm/main.swift && /tmp/fee-confirm-check
// No app, no node, no key — pure string arithmetic.
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

// The quote wins over the status maximum: what was shown is what is re-checked.
check(WeiMath.shownFeeWei(quoteWei: "5", statusWei: "9", recipients: 1) == "5", "quote preferred")

// No quote (several recipients, or a token send's sheet): the status maximum.
check(WeiMath.shownFeeWei(quoteWei: nil, statusWei: "9", recipients: 1) == "9", "status fallback")

// Nothing displayed (no quote and no status yet): nothing to re-check.
check(WeiMath.shownFeeWei(quoteWei: nil, statusWei: nil, recipients: 1) == nil, "nil when nothing shown")

// Each recipient can add its own account charge: the maximum scales with the
// count (a batch quotes each × n).
check(WeiMath.shownFeeWei(quoteWei: "3", statusWei: nil, recipients: 4) == "12", "scales with recipients")
check(WeiMath.shownFeeWei(quoteWei: nil, statusWei: "7", recipients: 0) == "7", "empty recipient list is one send")

// Exact wei, never a double: 1.000000000000000001 × 3 needs the last digit —
// a double would round 3.000000000000000003e18 off at the 16th significant
// digit and the re-check would demand a confirmation for a fee that did not
// change (or miss one that did).
let big = WeiMath.shownFeeWei(quoteWei: "1000000000000000001", statusWei: nil, recipients: 3)
check(big == "3000000000000000003", "exact at 19 digits: \(big ?? "nil")")

print("ok")
