import Foundation

/// Human-only commands. Each change is a transaction from the agent account
/// signed by the owner key, which asks for Touch ID or the login password; the
/// limits then live in the account contract, not in any local file.
enum Owner {
    static let defaultGas = "0.2"

    static func initialize() throws -> [String: Any] {
        _ = try Keys.agent(create: true)
        _ = try Keys.owner(create: true)
        Tools.configure()
        let id = try Tools.identity()
        var out: [String: Any] = ["account": id.account, "gas_payer": id.gasPayer, "keys": Paths.dir.path]
        let balance = (try? verifiedAccount(address: id.account, validators: Tools.validatorCount)).flatMap { Wei(decimal: $0.balanceWei) } ?? .zero
        if (try? Tools.session(id)) != nil {
            out["next"] = "Already set up. Change limits with `aether-agent policy set` (Touch ID)."
        } else if let min = Wei(aeth: "0.5"), !(balance < min) {
            out["limits"] = try apply(perTx: "1", perDay: "10", allow: [], expires: 0, gas: defaultGas)
            out["next"] = "Ready. Register with your agent tools: `aether-agent setup all --apply`."
        } else {
            out["next"] = "Fund \(id.account) (that balance is the most the agent can ever spend; on the testnet: `aether-agent get-test-tokens`), then run `aether-agent policy set` (Touch ID) to set limits and give the agent gas."
        }
        return out
    }

    static func policy(_ args: [String]) throws -> [String: Any] {
        Tools.configure()
        let id = try Tools.identity()
        switch args.first ?? "show" {
        case "show":
            guard let s = try Tools.session(id) else { return ["limits": "not set", "account": id.account] }
            return Tools.describe(s)
        case "set":
            let f = flags(args.dropFirst())
            let current = try Tools.session(id)
            let wei = { (w: String?) in w.flatMap { Wei(decimal: $0)?.aeth } }
            let perTx = f["per_tx"] ?? wei(current?.perPaymentWei) ?? "1"
            let perDay = f["per_day"] ?? wei(current?.perDayWei) ?? "10"
            let allow = f["allow"].map { $0 == "anyone" ? [] : $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) } } ?? (current?.allow ?? [])
            // Unspecified settings keep their current values (an expiry is never silently removed).
            let keepExpiry = current.map { $0.expires } ?? 0
            let gasBalance = (try? verifiedAccount(address: id.gasPayer, validators: Tools.validatorCount)).flatMap { Wei(decimal: $0.balanceWei) } ?? .zero
            let gas = f["gas"] ?? ((Wei(aeth: "0.05").map { gasBalance < $0 } ?? true) ? defaultGas : "0")
            let expires: UInt64 = f["expires_days"].flatMap(Double.init).map { $0 > 0 ? UInt64(Date().timeIntervalSince1970 + $0 * 86_400) : 0 } ?? keepExpiry
            return try apply(perTx: perTx, perDay: perDay, allow: allow, expires: expires, gas: gas)
        default:
            throw AgentError.input("policy show | set [--per-tx X] [--per-day Y] [--allow 0x..,0x..|anyone] [--expires-days N (0 = never)] [--gas AETH]")
        }
    }

    /// One owner-signed transaction: replace the agent's session with these limits
    /// and top up its gas payer.
    private static func apply(perTx: String, perDay: String, allow: [String], expires: UInt64, gas: String) throws -> [String: Any] {
        guard let p = Wei(aeth: perTx), let d = Wei(aeth: perDay), p.value > 0, !(d < p) else { throw AgentError.input("need 0 < per-tx <= per-day") }
        guard let g = Wei(aeth: gas) else { throw AgentError.input("--gas \(gas)") }
        let id = try Tools.identity()
        let settings = SessionSettings(sessionCode: id.agentCode, perPaymentWei: p.description, perDayWei: d.description, expires: expires, allow: allow, gasWei: g.description)
        let owner = try Keys.owner()
        let ownerKey = Keys.publicKey(owner)
        let prepared = try prepareSetSession(ownerPublicKey: ownerKey, settings: settings, validators: Tools.validatorCount)
        let sig = try owner.signature(for: prepared.signingMessage)  // Touch ID / password
        let hash = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig.rawRepresentation, p256PublicKey: ownerKey)
        for _ in 0..<60 {
            if let r = try? receipt(txHash: hash) {
                guard r.success else { throw AgentError.io("the account refused the new limits (tx \(hash))") }
                return (try Tools.session(id)).map(Tools.describe) ?? ["hash": hash]
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return ["hash": hash, "final": false]
    }
}
