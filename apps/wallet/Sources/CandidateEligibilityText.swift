import Foundation

/// This Mac's eligibility verdict, as reported by the local node. The wallet
/// displays it without reimplementing the network's eligibility rules.
struct CandidateEligibility: Decodable, Equatable, Sendable {
    enum WhyNot: String, Decodable, Equatable, Sendable {
        case streak
        case uptime
        case lastEpoch = "last_epoch"
        case v3Stability = "v3_stability"
        case none
    }

    let eligibleNextDraw: Bool
    let whyNot: WhyNot
    let hoursToEligible: Double?

    private enum CodingKeys: String, CodingKey {
        case eligibleNextDraw = "eligible_next_draw"
        case whyNot = "why_not"
        case hoursToEligible = "hours_to_eligible"
    }

    /// An old helper, a failed RPC or an unknown verdict supplies no new line.
    /// Decode the boolean strictly: JSON numbers must not masquerade as bools.
    static func fromRPC(_ response: Any?, validatorKey: String) -> CandidateEligibility? {
        let key = normalizedHex(validatorKey)
        guard !key.isEmpty,
              let result = response as? [String: Any],
              let rows = result["candidates"] as? [[String: Any]],
              let row = rows.first(where: {
                  guard let rowKey = $0["validator_key"] as? String else { return false }
                  return normalizedHex(rowKey) == key
              }),
              let data = try? JSONSerialization.data(withJSONObject: row),
              let verdict = try? JSONDecoder().decode(CandidateEligibility.self, from: data),
              verdict.eligibleNextDraw == (verdict.whyNot == .none) else { return nil }
        return verdict
    }

    private static func normalizedHex(_ value: String) -> String {
        let hex = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return hex.hasPrefix("0x") ? String(hex.dropFirst(2)) : hex
    }
}

/// One calm line for a registered Mac that has not yet been seated.
enum CandidateEligibilityText {
    static func line(_ verdict: CandidateEligibility?,
                     ko: Bool = Bundle.main.preferredLocalizations.first?.hasPrefix("ko") ?? false) -> String? {
        guard let verdict else { return nil }
        if verdict.eligibleNextDraw {
            return ko ? "검증자 후보: 자격을 갖췄어요. 다음 추첨을 기다려요" : "Validator candidate: eligible, waiting for the next draw"
        }
        if let hours = verdict.hoursToEligible, hours.isFinite, hours > 0 {
            let estimate = String(format: "%.0f", hours.rounded(.up))
            return ko ? "검증자 후보: 자격까지 약 \(estimate)시간" : "Validator candidate: eligible in about \(estimate) h"
        }
        switch verdict.whyNot {
        case .streak:
            return ko ? "검증자 후보: 온라인 상태를 더 오래 유지해야 해요" : "Validator candidate: needs more time online"
        case .uptime:
            return ko ? "검증자 후보: 놓친 온라인 확인을 만회할 시간이 필요해요" : "Validator candidate: needs more time online after missed check-ins"
        case .lastEpoch:
            return ko ? "검증자 후보: 최근 온라인 확인을 기다리고 있어요" : "Validator candidate: waiting for the latest online check-in"
        case .v3Stability:
            return ko ? "검증자 후보: 안정적인 온라인 활동을 더 확인해야 해요" : "Validator candidate: needs more time to show stable online activity"
        case .none:
            return nil
        }
    }
}
