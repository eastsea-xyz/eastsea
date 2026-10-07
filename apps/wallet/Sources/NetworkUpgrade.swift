import Foundation

/// A finalized notice stamped by the wallet's verified head.
struct NetworkUpgrade: Decodable, Equatable, Identifiable {
    let `protocol`: UInt32
    let activateAt: UInt64
    let emergency: Bool
    let notes: String
    let certifiedHeight: UInt64
    let certifiedTimestampMs: UInt64

    var id: UInt32 { `protocol` }

    enum CodingKeys: String, CodingKey {
        case `protocol`, emergency, notes
        case activateAt = "activate_at"
        case certifiedHeight = "certified_height"
        case certifiedTimestampMs = "certified_timestamp_ms"
    }

    static func parse(_ json: String, height: UInt64, now: Date = Date()) -> [NetworkUpgrade] {
        guard let data = json.data(using: .utf8),
              let notices = try? JSONDecoder().decode([NetworkUpgrade].self, from: data) else { return [] }
        return notices.filter {
            let age = now.timeIntervalSince1970 - TimeInterval($0.certifiedTimestampMs) / 1_000
            return $0.certifiedHeight >= height && $0.activateAt > $0.certifiedHeight &&
                age >= -600 && age <= 600
        }.sorted { $0.activateAt < $1.activateAt }
    }

    func daysLeft(height _: UInt64) -> UInt64 {
        let remaining = activateAt - min(certifiedHeight, activateAt)
        return remaining / 86_400 + (remaining % 86_400 == 0 ? 0 : 1)
    }

    func notice(height: UInt64) -> String {
        String(localized: "Network upgrade to protocol \(`protocol`) in \(daysLeft(height: height)) days: \(notes)")
    }

    func requiresAppUpdate(supportedProtocol: UInt32) -> Bool {
        `protocol` > supportedProtocol
    }

    func updateDeadline(height _: UInt64, now _: Date) -> String {
        let remaining = activateAt - min(certifiedHeight, activateAt)
        let estimated = Date(timeIntervalSince1970:
            TimeInterval(certifiedTimestampMs) / 1_000 + TimeInterval(remaining))
        return String(localized: "Update \(Brand.name) before \(estimated.formatted(date: .abbreviated, time: .shortened)) (estimated)")
    }
}
