import Foundation

enum AgentPolicy {
    static let defaultExpiryDays: TimeInterval = 7

    static func defaultExpires(now: Date = Date()) -> UInt64 {
        UInt64(now.timeIntervalSince1970 + defaultExpiryDays * 86_400)
    }

    /// An empty on-chain list means the owner explicitly chose any recipient.
    /// Initialization never creates a session with an empty list.
    static func permits(_ to: String, allow: [String]) -> Bool {
        allow.isEmpty || allow.contains { $0.caseInsensitiveCompare(to) == .orderedSame }
    }
}
