import Foundation

var checks = 0
func check(_ condition: Bool, _ message: String) {
    checks += 1
    if !condition { print("FAIL", message); exit(1) }
}

let key = String(repeating: "ab", count: 32)
func response(eligible: Any = false, whyNot: Any = "streak", hours: Any = 4) -> [String: Any] {
    ["epoch": 40, "next_draw_epoch": 48, "open_seats": 3, "candidates": [
        ["validator_key": String(repeating: "cd", count: 32), "eligible_next_draw": true, "why_not": "none"],
        ["validator_key": key, "missed": 2, "eligible_next_draw": eligible, "why_not": whyNot, "hours_to_eligible": hours]
    ]]
}

let verdict = CandidateEligibility.fromRPC(response(), validatorKey: "0x" + key.uppercased())
check(verdict?.eligibleNextDraw == false && verdict?.whyNot == .streak && verdict?.hoursToEligible == 4,
      "matches this Mac's validator key after hex normalization")
check(CandidateEligibilityText.line(verdict, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == "Validator candidate: eligible in about 4 h", "English ETA")
check(CandidateEligibilityText.line(verdict, locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")) == "검증자 후보: 자격까지 약 4시간", "Korean ETA")

var prefixed = response()
var rows = prefixed["candidates"] as! [[String: Any]]
rows[1]["validator_key"] = "0X" + key.uppercased()
prefixed["candidates"] = rows
check(CandidateEligibility.fromRPC(prefixed, validatorKey: key)?.whyNot == .streak,
      "normalizes the RPC key as well as the local key")
check(CandidateEligibility.fromRPC(response(), validatorKey: String(repeating: "ef", count: 32)) == nil,
      "another candidate never supplies this Mac's verdict")
check(CandidateEligibility.fromRPC(nil, validatorKey: key) == nil, "RPC failure gives no verdict")
check(CandidateEligibility.fromRPC(["candidates": [["validator_key": key, "streak": 4]]], validatorKey: key) == nil,
      "old helpers do not invent eligibility from the old streak field")
check(CandidateEligibilityText.line(nil, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil && CandidateEligibilityText.line(nil, locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")) == nil,
      "missing data adds no guessed reason or ETA")

let reasons: [(CandidateEligibility.WhyNot, String, String)] = [
    (.streak, "Validator candidate: needs more time online", "검증자 후보: 온라인 상태를 더 오래 유지해야 해요"),
    (.uptime, "Validator candidate: needs more time online after missed check-ins", "검증자 후보: 놓친 온라인 확인을 만회할 시간이 필요해요"),
    (.lastEpoch, "Validator candidate: waiting for the latest online check-in", "검증자 후보: 최근 온라인 확인을 기다리고 있어요"),
    (.v3Stability, "Validator candidate: needs more time to show stable online activity", "검증자 후보: 안정적인 온라인 활동을 더 확인해야 해요")
]
for (reason, en, ko) in reasons {
    let decoded = CandidateEligibility.fromRPC(response(whyNot: reason.rawValue, hours: NSNull()), validatorKey: key)
    check(decoded?.whyNot == reason && decoded?.hoursToEligible == nil, "decodes \(reason.rawValue) with unknown ETA")
    check(CandidateEligibilityText.line(decoded, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == en, "plain English \(reason.rawValue)")
    check(CandidateEligibilityText.line(decoded, locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")) == ko, "plain Korean \(reason.rawValue)")
    for language in ["en", "ko", "ja", "zh-Hans", "zh-Hant"] {
        let text = CandidateEligibilityText.line(decoded, locale: walletTestLocale(language), bundle: walletTestBundle(language))!
        check(!text.contains("RPC") && !text.contains("epoch") && !text.contains("v3") && !text.contains("why_not"),
              "no protocol jargon in \(reason.rawValue) (\(language))")
        check(!text.contains("\n"), "one calm line for \(reason.rawValue) (\(language))")
    }
}

let ready = CandidateEligibility.fromRPC(response(eligible: true, whyNot: "none", hours: NSNull()), validatorKey: key)
check(ready?.whyNot == CandidateEligibility.WhyNot.none && ready?.eligibleNextDraw == true, "none is the eligible verdict")
check(CandidateEligibilityText.line(ready, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == "Validator candidate: eligible, waiting for the next draw", "eligible English waiting line")
check(CandidateEligibilityText.line(ready, locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")) == "검증자 후보: 자격을 갖췄어요. 다음 추첨을 기다려요", "eligible Korean waiting line")

let fractional = CandidateEligibility(eligibleNextDraw: false, whyNot: .streak, hoursToEligible: 0.25)
check(CandidateEligibilityText.line(fractional, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == "Validator candidate: eligible in about 1 h", "rounds a partial hour up")
let rounded = CandidateEligibility(eligibleNextDraw: false, whyNot: .uptime, hoursToEligible: 4.1)
check(CandidateEligibilityText.line(rounded, locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")) == "검증자 후보: 자격까지 약 5시간", "rounds ETA conservatively in Korean")
for hours in [0, -1, Double.nan, Double.infinity] {
    let invalid = CandidateEligibility(eligibleNextDraw: false, whyNot: .lastEpoch, hoursToEligible: hours)
    check(CandidateEligibilityText.line(invalid, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == reasons[2].1, "invalid ETA \(hours) uses the known reason")
}
let readyWithHours = CandidateEligibility(eligibleNextDraw: true, whyNot: .none, hoursToEligible: 2)
check(CandidateEligibilityText.line(readyWithHours, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == "Validator candidate: eligible, waiting for the next draw", "an eligible candidate is not shown as counting down")

check(CandidateEligibility.fromRPC(response(eligible: "true", whyNot: "none"), validatorKey: key) == nil,
      "malformed eligibility does not become a waiting line")
check(CandidateEligibility.fromRPC(response(eligible: 1, whyNot: "none"), validatorKey: key) == nil,
      "a numeric field is not an eligibility boolean")
check(CandidateEligibility.fromRPC(response(whyNot: "future_reason"), validatorKey: key) == nil,
      "unknown reasons do not become a guessed reason")
check(CandidateEligibility.fromRPC(response(hours: "4"), validatorKey: key) == nil,
      "text is not a numeric ETA")
check(CandidateEligibility.fromRPC(response(eligible: false, whyNot: "none"), validatorKey: key) == nil,
      "an inconsistent verdict does not claim eligibility")
check(CandidateEligibility.fromRPC(response(eligible: true, whyNot: "uptime"), validatorKey: key) == nil,
      "an inconsistent reason does not claim eligibility")

var noHours = response()
var noHoursRows = noHours["candidates"] as! [[String: Any]]
noHoursRows[1].removeValue(forKey: "hours_to_eligible")
noHours["candidates"] = noHoursRows
check(CandidateEligibility.fromRPC(noHours, validatorKey: key)?.hoursToEligible == nil,
      "omitted ETA preserves the reported reason")
for field in ["eligible_next_draw", "why_not"] {
    var missing = response()
    var missingRows = missing["candidates"] as! [[String: Any]]
    missingRows[1].removeValue(forKey: field)
    missing["candidates"] = missingRows
    check(CandidateEligibility.fromRPC(missing, validatorKey: key) == nil, "missing \(field) has no verdict")
}

print("OK candidate-eligibility (\(checks) checks)")
