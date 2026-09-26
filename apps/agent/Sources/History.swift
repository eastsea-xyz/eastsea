import Foundation

/// The agent's own payments, for `history` only. Limits are not enforced from
/// this file: the account contract enforces them on chain.
struct HistoryEntry: Codable {
    let date: Date
    let to: [String]
    let totalWei: String
    let hash: String
}

enum History {
    static func load() -> [HistoryEntry] {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .secondsSince1970
        return (try? Data(contentsOf: Paths.history)).flatMap { try? d.decode([HistoryEntry].self, from: $0) } ?? []
    }

    static func append(_ e: HistoryEntry) {
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        enc.dateEncodingStrategy = .secondsSince1970
        if let data = try? enc.encode(Array(([e] + load()).prefix(1_000))) { try? Paths.write(data, to: Paths.history) }
    }
}

extension Data {
    var hex: String { map { String(format: "%02x", $0) }.joined() }
}
