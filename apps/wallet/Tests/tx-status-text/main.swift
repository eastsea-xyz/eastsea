// The activity row's sentence about a submitted transaction, in one language
// at a time (founder review of 0.7.0: Korean rows on an English screen).
//   swiftc -o ./tmp/tx-check apps/wallet/Sources/TxStatusText.swift apps/wallet/Tests/tx-status-text/main.swift && ./tmp/tx-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }
let hangul = { (s: String) in s.unicodeScalars.contains { (0xAC00...0xD7A3).contains($0.value) } }
let ko = "앞서 보낸 거래가 먼저 처리되기를 기다리고 있어요."

// Korean passes the core's own sentence through.
check(TxStatusText.sentence(state: "pending", reason: "nonce_gap", success: nil, message: ko, locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")) == ko,
      "Korean shows the core's sentence")
// English never shows Hangul, for every state and reason the core names.
let cases: [(String, String?)] = [
    ("included", nil), ("pending", "state_price_above_cap"), ("pending", "nonce_gap"), ("pending", "fee_cap_below_base"),
    ("pending", nil), ("dropped", "state_price_above_cap"), ("dropped", "fee_cap_below_base"), ("dropped", "nonce_gap"),
    ("dropped", "expired"), ("dropped", "evicted"), ("dropped", "replaced"), ("dropped", "unaffordable"),
    ("dropped", "something new"), ("unknown", nil), ("replaced", "nonce_used"),
]
for (state, reason) in cases {
    let en = TxStatusText.sentence(state: state, reason: reason, success: false, message: ko, locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
    check(!en.isEmpty && !hangul(en), "English \(state)/\(reason ?? "-") has no Korean: \(en)")
    check(!en.contains("nonce") && !en.contains("mempool") && !en.contains("TTL"), "no jargon: \(en)")
}
check(TxStatusText.sentence(state: "included", reason: nil, success: true, message: ko, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == "Done.", "success reads Done.")
check(TxStatusText.sentence(state: "dropped", reason: "expired", success: nil, message: ko, locale: walletTestLocale("en"), bundle: walletTestBundle("en")).contains("send it again"),
      "a resendable drop says it can be sent again")
check(!TxStatusText.sentence(state: "dropped", reason: "replaced", success: nil, message: ko, locale: walletTestLocale("en"), bundle: walletTestBundle("en")).contains("send it again"),
      "a replaced one does not offer a resend")
check(hangul(TxStatusText.unknown(locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))) && !hangul(TxStatusText.unknown(locale: walletTestLocale("en"), bundle: walletTestBundle("en"))), "unknown in each language")
check(hangul(TxStatusText.timedOut(locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))) && !hangul(TxStatusText.timedOut(locale: walletTestLocale("en"), bundle: walletTestBundle("en"))), "timed out in each language")

// A local queue result is not a final chain fact. It must not promise that the
// transaction was cancelled, failed permanently, or could not spend money.
let finalClaims = ["cancel", "no money", "nothing was spent", "did not go through", "didn't go through", "will not", "failed"]
for reason in ["state_price_above_cap", "nonce_gap", "fee_cap_below_base", "something new"] {
    let en = TxStatusText.sentence(state: "pending", reason: reason, success: nil, message: ko, locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
    check(!finalClaims.contains { en.lowercased().contains($0) }, "pending copy keeps the outcome unresolved: \(reason): \(en)")
}
for reason in ["state_price_above_cap", "fee_cap_below_base", "nonce_gap", "expired", "evicted", "replaced", "unaffordable", "something new"] {
    let en = TxStatusText.sentence(state: "dropped", reason: reason, success: nil, message: ko, locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
    check(en.contains("not recorded") && en.contains("yet"), "a local drop says it is not recorded yet: \(reason): \(en)")
    check(!finalClaims.contains { en.lowercased().contains($0) }, "local drop copy does not promise a final outcome: \(reason): \(en)")
    let unresolvedKo = "이 거래는 아직 체인에 기록되지 않았어요."
    check(TxStatusText.sentence(state: "dropped", reason: reason, success: nil, message: unresolvedKo, locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")) == unresolvedKo,
          "Korean local-drop copy keeps the core's unresolved sentence: \(reason)")
}
let localReplacement = TxStatusText.sentence(state: "dropped", reason: "replaced", success: nil, message: ko, locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
check(!localReplacement.contains("went through instead") && !localReplacement.contains("used on chain"),
      "a local replacement hint does not claim a proven chain replacement")
let timedOutEn = TxStatusText.timedOut(locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
check(!finalClaims.contains { timedOutEn.lowercased().contains($0) }, "a timeout does not claim the payment failed: \(timedOutEn)")
check(timedOutEn.contains("unconfirmed") || timedOutEn.contains("yet"), "a timeout leaves the outcome unconfirmed")
check(TxStatusText.timedOut(locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")).contains("아직"), "Korean timeout copy also leaves the outcome unconfirmed")

// A top-level replacement is a proven use of the sender's number on chain,
// unlike a local dropped/replaced hint. It says nothing about other uses of
// the transaction's authorizations or the account's money.
let confirmedReplacementKo = "보낸 계정의 같은 순서 번호(7)를 다른 거래가 체인에서 사용했어요. 이 거래는 더 이상 처리될 수 없어요."
let confirmedReplacementEn = TxStatusText.sentence(state: "replaced", reason: "nonce_used", success: nil,
                                                  message: confirmedReplacementKo, locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
check(confirmedReplacementEn != TxStatusText.unknown(locale: walletTestLocale("en"), bundle: walletTestBundle("en")),
      "top-level replaced has a distinct English sentence: \(confirmedReplacementEn)")
check(confirmedReplacementEn.contains("Another") && confirmedReplacementEn.contains("account") &&
      confirmedReplacementEn.contains("number") && confirmedReplacementEn.contains("on chain"),
      "confirmed replacement identifies another transaction's use of the sending account's number on chain")
check(!confirmedReplacementEn.contains("yet") && !confirmedReplacementEn.contains("may"),
      "proven replacement is final, rather than an unresolved local hint")
check(!["no money", "nothing was spent", "nothing left"].contains { confirmedReplacementEn.lowercased().contains($0) },
      "a proven replacement does not make a broader no-spend promise")
let confirmedReplacementKoText = TxStatusText.sentence(state: "replaced", reason: "nonce_used", success: nil,
                                                      message: confirmedReplacementKo, locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))
check(confirmedReplacementKoText == "보낸 계정의 같은 순서 번호를 다른 거래가 체인에서 사용했어요. 이 거래는 더 이상 처리될 수 없어요.",
      "Korean proven-replacement copy preserves the core's chain fact")
check(localReplacement.contains("yet") && localReplacement != confirmedReplacementEn,
      "local dropped/replaced remains unresolved and distinct from proven replacement")
for language in ["en", "ko", "ja", "zh-Hans", "zh-Hant"] {
    let result = TxStatusText.sentence(state: "pending", reason: "nonce_gap", success: nil,
                                      message: "raw backend diagnostic",
                                      locale: walletTestLocale(language), bundle: walletTestBundle(language))
    check(!result.contains("raw backend"), "backend text cannot bypass the catalog in \(language)")
}
print("tx-status-text ok")
