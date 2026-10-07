import Foundation

/// A signed on-chain notice as reported by the selected node's status RPC.
struct NetworkUpgrade: Decodable, Equatable, Identifiable {
    let `protocol`: UInt32
    let activateAt: UInt64
    let emergency: Bool
    let notes: String

    var id: UInt32 { `protocol` }

    enum CodingKeys: String, CodingKey {
        case `protocol`, emergency, notes
        case activateAt = "activate_at"
    }

    static func parse(_ json: String, height: UInt64) -> [NetworkUpgrade] {
        guard let data = json.data(using: .utf8),
              let notices = try? JSONDecoder().decode([NetworkUpgrade].self, from: data) else { return [] }
        return notices.filter { $0.activateAt > height }.sorted { $0.activateAt < $1.activateAt }
    }

    func daysLeft(height: UInt64) -> UInt64 {
        let remaining = activateAt - min(height, activateAt)
        return remaining / 86_400 + (remaining % 86_400 == 0 ? 0 : 1)
    }

    func notice(height: UInt64) -> String {
        String(localized: "Network upgrade to protocol \(`protocol`) in \(daysLeft(height: height)) days: \(notes)")
    }

    func requiresAppUpdate(supportedProtocol: UInt32) -> Bool {
        `protocol` > supportedProtocol
    }

    func updateDeadline(height: UInt64, now: Date) -> String {
        let remaining = activateAt - min(height, activateAt)
        let estimated = now.addingTimeInterval(TimeInterval(remaining))
        return String(localized: "Update \(Brand.name) before \(estimated.formatted(date: .abbreviated, time: .shortened)) (estimated)")
    }
}
