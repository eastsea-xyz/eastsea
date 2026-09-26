import CryptoKit
import Foundation

/// Spending limits, signed by the owner key (Touch ID). An agent that edits the
/// file breaks the signature and can then spend nothing.
struct Policy: Codable, Equatable {
    var maxPerTx: String        // AETH, decimal
    var maxPerDay: String       // AETH over any rolling 24 h
    var allow: [String]         // lowercase 0x addresses; empty = anyone
    var agentPublicKey: String  // binds the policy to this agent key (hex)
    var updated: Date

    static func defaults(agent: Data) -> Policy {
        Policy(maxPerTx: "1", maxPerDay: "10", allow: [], agentPublicKey: agent.hex, updated: Date())
    }
}

private struct SignedPolicy: Codable {
    let policy: Policy
    let ownerPublicKey: String
    let signature: String
}

enum PolicyStore {
    private static func encode(_ p: Policy) throws -> Data {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys]
        e.dateEncodingStrategy = .secondsSince1970
        return try e.encode(p)
    }

    /// Sign and save (prompts for Touch ID / password).
    static func save(_ p: Policy) throws {
        let owner = try Keys.owner()
        let sig = try owner.signature(for: encode(p))
        let signed = SignedPolicy(policy: p, ownerPublicKey: Keys.publicKey(owner).hex, signature: sig.rawRepresentation.hex)
        let e = JSONEncoder()
        e.outputFormatting = [.prettyPrinted, .sortedKeys]
        e.dateEncodingStrategy = .secondsSince1970
        try Paths.write(e.encode(signed), to: Paths.policy)
    }

    /// Load and verify against the owner key and this agent key.
    static func load() throws -> Policy {
        guard let data = try? Data(contentsOf: Paths.policy) else { throw AgentError.notInitialized }
        let d = JSONDecoder()
        d.dateDecodingStrategy = .secondsSince1970
        guard let signed = try? d.decode(SignedPolicy.self, from: data) else { throw AgentError.policy("policy.json is unreadable; the owner must run `aether-agent policy set`") }
        let owner = try Keys.owner()
        guard Keys.publicKey(owner).hex == signed.ownerPublicKey,
              let sig = try? P256.Signing.ECDSASignature(rawRepresentation: Data(hex: signed.signature) ?? Data()),
              owner.publicKey.isValidSignature(sig, for: try encode(signed.policy))
        else { throw AgentError.policy("policy.json was modified without the owner's Touch ID; the owner must run `aether-agent policy set`") }
        guard signed.policy.agentPublicKey == Keys.publicKey(try Keys.agent()).hex else {
            throw AgentError.policy("policy belongs to a different agent key")
        }
        return signed.policy
    }
}

/// The agent's own payments. Signed by the agent key (no prompt) so edits are
/// detected, and cross-checked with the account nonce on chain so deleting it
/// is detected too (only this binary holds the key, so nonce == payments made).
struct LedgerEntry: Codable {
    let date: Date
    let to: [String]
    let totalWei: String
    let hash: String
    let nonce: UInt64
}

private struct SignedLedger: Codable {
    let entries: [LedgerEntry]
    let signature: String
}

enum Ledger {
    private static func encode(_ e: [LedgerEntry]) throws -> Data {
        let enc = JSONEncoder()
        enc.outputFormatting = [.sortedKeys]
        enc.dateEncodingStrategy = .secondsSince1970
        return try enc.encode(e)
    }

    static func load() throws -> [LedgerEntry] {
        guard let data = try? Data(contentsOf: Paths.ledger) else { return [] }
        let d = JSONDecoder()
        d.dateDecodingStrategy = .secondsSince1970
        let agent = try Keys.agent()
        guard let signed = try? d.decode(SignedLedger.self, from: data),
              let sig = try? P256.Signing.ECDSASignature(rawRepresentation: Data(hex: signed.signature) ?? Data()),
              agent.publicKey.isValidSignature(sig, for: try encode(signed.entries))
        else { throw AgentError.policy("ledger.json was modified; the owner must run `aether-agent policy reset-ledger`") }
        return signed.entries
    }

    static func save(_ entries: [LedgerEntry]) throws {
        let agent = try Keys.agent()
        let sig = try agent.signature(for: encode(entries))
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        enc.dateEncodingStrategy = .secondsSince1970
        try Paths.write(enc.encode(SignedLedger(entries: entries, signature: sig.rawRepresentation.hex)), to: Paths.ledger)
    }

    static func spentLastDay(_ entries: [LedgerEntry]) -> Wei {
        let from = Date().addingTimeInterval(-86_400)
        return entries.filter { $0.date >= from }.reduce(Wei.zero) { $0 + (Wei(decimal: $1.totalWei) ?? .zero) }
    }
}

extension Data {
    var hex: String { map { String(format: "%02x", $0) }.joined() }

    init?(hex: String) {
        let s = hex.hasPrefix("0x") ? String(hex.dropFirst(2)) : hex
        guard s.count % 2 == 0 else { return nil }
        var out = Data(capacity: s.count / 2)
        var i = s.startIndex
        while i < s.endIndex {
            let j = s.index(i, offsetBy: 2)
            guard let b = UInt8(s[i..<j], radix: 16) else { return nil }
            out.append(b)
            i = j
        }
        self = out
    }
}
