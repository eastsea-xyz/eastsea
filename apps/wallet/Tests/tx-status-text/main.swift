// The activity row's sentence about a submitted transaction, in one language
// at a time (founder review of 0.7.0: Korean rows on an English screen).
//   swiftc -o ./tmp/tx-check apps/wallet/Sources/TxStatusText.swift apps/wallet/Tests/tx-status-text/main.swift && ./tmp/tx-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }
let hangul = { (s: String) in s.unicodeScalars.contains { (0xAC00...0xD7A3).contains($0.value) } }
let ko = "앞서 보낸 거래가 먼저 처리되기를 기다리고 있어요."

// Korean passes the core's own sentence through.
check(TxStatusText.sentence(state: "pending", reason: "nonce_gap", success: nil, message: ko, ko: true) == ko,
      "Korean shows the core's sentence")
// English never shows Hangul, for every state and reason the core names.
let cases: [(String, String?)] = [
    ("included", nil), ("pending", "state_price_above_cap"), ("pending", "nonce_gap"), ("pending", "fee_cap_below_base"),
    ("pending", nil), ("dropped", "state_price_above_cap"), ("dropped", "fee_cap_below_base"), ("dropped", "nonce_gap"),
    ("dropped", "expired"), ("dropped", "evicted"), ("dropped", "replaced"), ("dropped", "unaffordable"),
    ("dropped", "something new"), ("unknown", nil),
]
for (state, reason) in cases {
    let en = TxStatusText.sentence(state: state, reason: reason, success: false, message: ko, ko: false)
    check(!en.isEmpty && !hangul(en), "English \(state)/\(reason ?? "-") has no Korean: \(en)")
    check(!en.contains("nonce") && !en.contains("mempool") && !en.contains("TTL"), "no jargon: \(en)")
}
check(TxStatusText.sentence(state: "included", reason: nil, success: true, message: ko, ko: false) == "Done.", "success reads Done.")
check(TxStatusText.sentence(state: "dropped", reason: "expired", success: nil, message: ko, ko: false).contains("send it again"),
      "a resendable drop says it can be sent again")
check(!TxStatusText.sentence(state: "dropped", reason: "replaced", success: nil, message: ko, ko: false).contains("send it again"),
      "a replaced one does not offer a resend")
check(hangul(TxStatusText.unknown(ko: true)) && !hangul(TxStatusText.unknown(ko: false)), "unknown in each language")
check(hangul(TxStatusText.timedOut(ko: true)) && !hangul(TxStatusText.timedOut(ko: false)), "timed out in each language")
print("tx-status-text ok")
