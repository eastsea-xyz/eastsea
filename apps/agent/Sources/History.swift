import Foundation

/// Finalized payments only. Pending context is held separately until finality:
/// a receipt, or the payment's nonce used by another transaction. A node-local
/// drop is not finality (B5 review round 2, finding 5) — another node may still
/// include the same transaction — so it only annotates the pending entry.
struct HistoryEntry: Codable {
    let date: Date
    let to: [String]
    let totalWei: String
    let hash: String
    let status: String?
    let purpose: String?
    let payeeNames: [String]?
    let asset: String?
    let amount: String?
    let txLink: String?
}

struct PendingPayment: Codable {
    let date: Date
    let to: [String]
    let totalWei: String
    let hash: String
    let purpose: String
    let payeeNames: [String]
    let asset: String
    let amount: String
    /// Who signed it and at which nonce: what reconciles it later (the nonce
    /// used on chain without this hash's receipt means another tx took it).
    /// Absent in entries written before round 2.
    var sender: String? = nil
    var nonce: UInt64? = nil
    /// The last node-local drop reason, while it is not on chain yet.
    var notIncluded: String? = nil
}

enum History {
    private static func read<T: Decodable>(_ type: T.Type, from url: URL) -> [T] {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .secondsSince1970
        return (try? Data(contentsOf: url)).flatMap { try? d.decode([T].self, from: $0) } ?? []
    }

    private static func write<T: Encodable>(_ items: [T], to url: URL) throws {
        let e = JSONEncoder()
        e.outputFormatting = [.prettyPrinted, .sortedKeys]
        e.dateEncodingStrategy = .secondsSince1970
        try Paths.write(e.encode(items), to: url)
    }

    static func load() -> [HistoryEntry] { read(HistoryEntry.self, from: Paths.history) }
    static func pending() -> [PendingPayment] { read(PendingPayment.self, from: Paths.pending) }

    static func submit(_ item: PendingPayment) throws {
        try write(Array(([item] + pending()).prefix(1_000)), to: Paths.pending)
    }

    /// A node said it dropped the payment: keep it pending (with the reason),
    /// so a later receipt — from that node or another — still finalizes it.
    static func markNotIncluded(hash: String, why: String) throws {
        let items = pending()
        guard items.contains(where: { $0.hash.caseInsensitiveCompare(hash) == .orderedSame }) else { return }
        try write(items.map { item -> PendingPayment in
            guard item.hash.caseInsensitiveCompare(hash) == .orderedSame else { return item }
            var noted = item
            noted.notIncluded = why
            return noted
        }, to: Paths.pending)
    }

    static func finalize(hash: String, success: Bool) throws {
        guard let item = pending().first(where: { $0.hash.caseInsensitiveCompare(hash) == .orderedSame }) else { return }
        let final = HistoryEntry(date: Date(), to: item.to, totalWei: item.totalWei, hash: item.hash,
                                 status: success ? "confirmed" : "failed", purpose: item.purpose,
                                 payeeNames: item.payeeNames, asset: item.asset, amount: item.amount,
                                 txLink: "aether://tx?hash=\(item.hash)")
        let older = load().filter { $0.hash.caseInsensitiveCompare(hash) != .orderedSame }
        try write(Array(([final] + older).prefix(1_000)), to: Paths.history)
        try write(pending().filter { $0.hash.caseInsensitiveCompare(hash) != .orderedSame }, to: Paths.pending)
    }
}

struct NamedPayee: Codable {
    let name: String
    let address: String
}

struct PayeeRequest: Codable {
    let address: String
    let purpose: String
    let amount: String?
    let asset: String?
    let date: Date
}

enum Payees {
    static func load() -> [NamedPayee] {
        (try? Data(contentsOf: Paths.payees)).flatMap { try? JSONDecoder().decode([NamedPayee].self, from: $0) } ?? []
    }

    static func requests() -> [PayeeRequest] {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .secondsSince1970
        return (try? Data(contentsOf: Paths.payeeRequests)).flatMap { try? d.decode([PayeeRequest].self, from: $0) } ?? []
    }

    static func name(_ address: String) -> String? {
        load().first { $0.address.caseInsensitiveCompare(address) == .orderedSame }?.name
    }

    static func add(name: String, address: String) throws {
        let items = load().filter { $0.address.caseInsensitiveCompare(address) != .orderedSame } + [NamedPayee(name: name, address: address)]
        try Paths.write(JSONEncoder().encode(items), to: Paths.payees)
        let left = requests().filter { $0.address.caseInsensitiveCompare(address) != .orderedSame }
        let e = JSONEncoder()
        e.dateEncodingStrategy = .secondsSince1970
        try Paths.write(e.encode(left), to: Paths.payeeRequests)
    }

    @discardableResult static func request(address: String, purpose: String, amount: String, asset: String) throws -> Bool {
        guard !requests().contains(where: { $0.address.caseInsensitiveCompare(address) == .orderedSame }) else { return false }
        let e = JSONEncoder()
        e.dateEncodingStrategy = .secondsSince1970
        try Paths.write(e.encode(requests() + [PayeeRequest(address: address, purpose: purpose, amount: amount, asset: asset, date: Date())]), to: Paths.payeeRequests)
        return true
    }
}

extension Data {
    var hex: String { map { String(format: "%02x", $0) }.joined() }
}
