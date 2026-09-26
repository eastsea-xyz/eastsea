import Foundation

/// Human-only commands: every change is signed by the owner key, which asks
/// for Touch ID or the login password.
enum Owner {
    static func initialize() throws -> [String: Any] {
        let agent = try Keys.agent(create: true)
        _ = try Keys.owner(create: true)
        if (try? PolicyStore.load()) == nil {
            try PolicyStore.save(.defaults(agent: Keys.publicKey(agent)))
        }
        if (try? Ledger.load()) == nil || !FileManager.default.fileExists(atPath: Paths.ledger.path) {
            try Ledger.save([])
        }
        Tools.configure()
        let address = try accountAddress(p256PublicKey: Keys.publicKey(agent))
        return ["address": address, "policy": try describe(PolicyStore.load()), "keys": Paths.dir.path,
                "next": "Fund this address from your wallet (that balance is the most the agent can ever spend), then run `aether-agent setup all`."]
    }

    static func policy(_ args: [String]) throws -> [String: Any] {
        switch args.first ?? "show" {
        case "show":
            return try describe(PolicyStore.load())
        case "set":
            let f = flags(args.dropFirst())
            let agentKey = try Keys.agent()
            var p = (try? PolicyStore.load()) ?? .defaults(agent: Keys.publicKey(agentKey))
            if let v = f["per_tx"] {
                guard Wei(aeth: v) != nil else { throw AgentError.input("--per-tx \(v)") }
                p.maxPerTx = v
            }
            if let v = f["per_day"] {
                guard Wei(aeth: v) != nil else { throw AgentError.input("--per-day \(v)") }
                p.maxPerDay = v
            }
            if let v = f["allow"] {
                p.allow = v == "anyone" ? [] : v.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces).lowercased() }
            }
            p.updated = Date()
            try PolicyStore.save(p)
            return try describe(PolicyStore.load())
        case "reset-ledger":
            // Owner presence first, then restart the log from what the chain shows.
            _ = try Keys.owner().signature(for: Data("aether-agent reset-ledger".utf8))
            try Ledger.save([])
            return ["ok": true, "note": "Spending log cleared. The on-chain nonce check now counts from the next payment."]
        default:
            throw AgentError.input("policy show | set | reset-ledger")
        }
    }

    static func describe(_ p: Policy) throws -> [String: Any] {
        ["per_tx_aeth": p.maxPerTx, "per_day_aeth": p.maxPerDay, "allowed_recipients": p.allow.isEmpty ? ["anyone"] : p.allow,
         "updated": ISO8601DateFormatter().string(from: p.updated), "signed_by": "owner (Touch ID)"]
    }
}
