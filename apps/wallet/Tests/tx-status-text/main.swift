// The activity row's sentence about a submitted transaction, in one language
// at a time (founder review of 0.7.0: Korean rows on an English screen).
//   swiftc -o ./tmp/tx-check apps/wallet/Sources/TxStatusText.swift apps/wallet/Tests/tx-status-text/main.swift && ./tmp/tx-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }
let hangul = { (s: String) in s.unicodeScalars.contains { (0xAC00...0xD7A3).contains($0.value) } }
let ko = "앞서 보낸 거래가 먼저 처리되기를 기다리고 있어요."

// Known backend states resolve through the catalog, keeping the reviewed Korean wording.
check(TxStatusText.sentence(state: "pending", reason: "nonce_gap", success: nil, message: ko, locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")) == ko,
      "Korean keeps the reviewed nonce-gap sentence")
// English never shows Hangul, for every state and reason the core names.
let cases: [(String, String?)] = [
    ("included", nil), ("pending", "state_price_above_cap"), ("pending", "nonce_gap"), ("pending", "fee_cap_below_base"),
    ("pending", nil), ("dropped", "state_price_above_cap"), ("dropped", "fee_cap_below_base"), ("dropped", "nonce_gap"),
    ("dropped", "expired"), ("dropped", "evicted"), ("dropped", "replaced"), ("dropped", "unaffordable"),
    ("dropped", "something new"), ("unknown", nil),
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
// Even a backend message in the wrong language cannot leak into the chosen UI language.
for (language, expected) in [("en", "Waiting for an earlier payment to go through first."),
                            ("ko", "앞서 보낸 거래가 먼저 처리되기를 기다리고 있어요."),
                            ("ja", "先に送った支払いの処理を待っています。")] {
    check(TxStatusText.sentence(state: "pending", reason: "nonce_gap", success: nil, message: "raw backend diagnostic",
                              locale: walletTestLocale(language), bundle: walletTestBundle(language)) == expected,
          "known backend status resolves in \(language)")
}
check(TxStatusText.unknown(locale: walletTestLocale("ja"), bundle: walletTestBundle("ja")) == "ネットワークにこの支払いの記録がありません。", "Japanese unknown sentence")
check(TxStatusText.timedOut(locale: walletTestLocale("ja"), bundle: walletTestBundle("ja")) == "長く待っても処理されませんでした。", "Japanese timed-out sentence")
check(TxStatusText.sentence(state: "pending", reason: "state_price_above_cap", success: nil,
                          message: "네트워크가 붐벼 지금 수수료가 이 거래에 허용한 최대치보다 높아요. 약 35초 뒤 내려가면 처리돼요. 10분 안에 내려가지 않으면 처리되지 않을 수 있고, 그때는 새 가격으로 다시 보낼 수 있어요.",
                          locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))
      == "네트워크가 붐벼 지금 수수료가 이 거래에 허용한 최대치보다 높아요. 약 35초 뒤 내려가면 처리돼요. 10분 안에 내려가지 않으면 처리되지 않을 수 있고, 그때는 새 가격으로 다시 보낼 수 있어요.", "Korean keeps the backend wait estimate")
let minuteEstimate = "네트워크가 붐벼 지금 수수료가 이 거래에 허용한 최대치보다 높아요. 약 3분 뒤 내려가면 처리돼요. 10분 안에 내려가지 않으면 처리되지 않을 수 있고, 그때는 새 가격으로 다시 보낼 수 있어요."
check(TxStatusText.sentence(state: "pending", reason: "state_price_above_cap", success: nil, message: minuteEstimate,
                          locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")) == minuteEstimate,
      "Korean keeps the estimated minutes, not the ten-minute limit")
for language in ["en", "ja", "zh-Hans", "zh-Hant"] {
    let result = TxStatusText.sentence(state: "pending", reason: "state_price_above_cap", success: nil, message: minuteEstimate,
                                       locale: walletTestLocale(language), bundle: walletTestBundle(language))
    check(result.contains("3") && !hangul(result), "the estimate resolves without backend Korean in \(language)")
}
let replacementMessage = "같은 순서 번호(42)로 보낸 다른 거래가 체인에 기록됐어요. 이 거래는 처리되지 않으며, 이 거래로 빠져나간 돈은 없어요."
check(TxStatusText.sentence(state: "replaced", reason: "nonce_used", success: nil, message: replacementMessage,
                          locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")) == replacementMessage,
      "Korean keeps the replaced payment number and reviewed wording")
check(TxStatusText.sentence(state: "unknown", reason: nil, success: nil, message: "raw backend diagnostic",
                          locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))
      == "아직 이 거래의 기록을 찾지 못했어요. 처리되지 않았어요 (아직 체인에 기록되지 않음).",
      "a known unknown status cannot pass through an untranslated backend message")
print("tx-status-text ok")
