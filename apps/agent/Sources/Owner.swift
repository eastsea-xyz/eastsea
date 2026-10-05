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
        if let session = try? Tools.session(id) {
            out["next"] = session.allow.isEmpty || session.expires == 0
                ? "Existing policy allows any recipient or never expires. Add a named payee with `aether-agent payee add --name NAME --address 0x...` (Touch ID) to restrict recipients and renew the session."
                : "Already set up. Renew with `aether-agent policy renew` (Touch ID) before expiry."
        } else if let min = Wei(aeth: "0.5"), !(balance < min) {
            out["next"] = "Payments are off until you add a named payee with `aether-agent payee add --name NAME --address 0x...` (Touch ID). Default limits: 1 DBLN/payment, 10 DBLN/day, 7 days."
        } else {
            out["next"] = "Fund \(id.account), then add a named payee with `aether-agent payee add --name NAME --address 0x...` (Touch ID)."
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
            if current == nil && f["allow"] == nil { throw AgentError.input("add a payee first, or explicitly choose --allow anyone (warning: an agent tricked into paying can send to any address within the caps)") }
            let allow = f["allow"].map { $0 == "anyone" ? [] : $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) } } ?? (current?.allow ?? [])
            if f["allow"] != nil && f["allow"] != "anyone" && allow.isEmpty {
                throw AgentError.input("--allow requires addresses, or explicitly --allow anyone")
            }
            if f["allow"] != nil && f["allow"] != "anyone" && allow.contains(where: { Payees.name($0) == nil }) {
                throw AgentError.input("name each recipient first with `aether-agent payee add --name NAME --address 0x...`")
            }
            if f["allow"] == "anyone" { fputs("WARNING: any recipient is allowed; a tricked agent can spend within the caps.\n", stderr) }
            // Legacy non-expiring or expired sessions receive a fresh term.
            let now = UInt64(Date().timeIntervalSince1970)
            let keepExpiry = current.map { $0.expires > now ? $0.expires : AgentPolicy.defaultExpires() } ?? AgentPolicy.defaultExpires()
            let gasBalance = (try? verifiedAccount(address: id.gasPayer, validators: Tools.validatorCount)).flatMap { Wei(decimal: $0.balanceWei) } ?? .zero
            let gas = f["gas"] ?? ((Wei(aeth: "0.05").map { gasBalance < $0 } ?? true) ? defaultGas : "0")
            let expires: UInt64
            if let raw = f["expires_days"] {
                guard let days = Double(raw), days.isFinite, days > 0, days <= 30 else {
                    throw AgentError.input("--expires-days must be 1..30")
                }
                expires = UInt64(Date().timeIntervalSince1970 + days * 86_400)
            } else { expires = keepExpiry }
            return try apply(perTx: perTx, perDay: perDay, allow: allow, expires: expires, gas: gas)
        case "renew":
            guard let s = try Tools.session(id) else { throw AgentError.policy("no session to renew") }
            let f = flags(args.dropFirst())
            let days: Double
            if let raw = f["days"] {
                guard let value = Double(raw) else { throw AgentError.input("renew --days must be 1..30") }
                days = value
            } else { days = AgentPolicy.defaultExpiryDays }
            guard days.isFinite, days > 0, days <= 30 else { throw AgentError.input("renew --days must be 1..30") }
            return try apply(perTx: Wei(decimal: s.perPaymentWei)?.aeth ?? "1", perDay: Wei(decimal: s.perDayWei)?.aeth ?? "10",
                             allow: s.allow, expires: UInt64(Date().timeIntervalSince1970 + days * 86_400), gas: "0")
        default:
            throw AgentError.input("policy show | set [--per-tx X] [--per-day Y] [--allow 0x..,0x..|anyone] [--expires-days 1..30] [--gas \(Tools.coinTicker)] | renew")
        }
    }

    static func stop() throws -> [String: Any] {
        Tools.configure()
        let id = try Tools.identity()
        guard try sessionStatus(account: id.account, validators: Tools.validatorCount).exists else {
            return ["stopped": true, "already_stopped": true]
        }
        let key = try Keys.owner()
        let pk = Keys.publicKey(key)
        let prepared = try prepareStopSessions(ownerPublicKey: pk, validators: Tools.validatorCount)
        let sig = try key.signature(for: prepared.signingMessage) // Touch ID / password
        let hash = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig.rawRepresentation, p256PublicKey: pk)
        for _ in 0..<60 {
            if let r = try? receipt(txHash: hash) {
                guard r.success else { throw AgentError.io("session removal failed (tx \(hash))") }
                return ["stopped": true, "final": true, "hash": hash]
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return ["stopped": false, "final": false, "hash": hash]
    }

    static func payee(_ args: [String]) throws -> [String: Any] {
        let f = flags(args.dropFirst())
        switch args.first ?? "list" {
        case "list": return ["payees": Payees.load().map { ["name": $0.name, "address": $0.address] }]
        case "pending": return ["requests": Payees.requests().map {
            ["address": $0.address, "purpose": $0.purpose, "amount": $0.amount ?? "", "asset": $0.asset ?? ""]
        }]
        case "add":
            guard let name = f["name"]?.trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty,
                  let address = f["address"], address.hasPrefix("0x"), address.count == 42 else {
                throw AgentError.input("payee add --name NAME --address 0x...")
            }
            Tools.configure()
            let id = try Tools.identity()
            let current = try Tools.session(id)
            let allow = Array(Set((current?.allow ?? []) + [address])).sorted()
            let now = UInt64(Date().timeIntervalSince1970)
            let expires = current.map { $0.expires > now ? $0.expires : AgentPolicy.defaultExpires() } ?? AgentPolicy.defaultExpires()
            let gasBalance = (try? verifiedAccount(address: id.gasPayer, validators: Tools.validatorCount)).flatMap { Wei(decimal: $0.balanceWei) } ?? .zero
            let gas = current == nil && gasBalance < (Wei(aeth: "0.05") ?? .zero) ? defaultGas : "0"
            let result = try apply(perTx: current.flatMap { Wei(decimal: $0.perPaymentWei)?.aeth } ?? "1",
                                   perDay: current.flatMap { Wei(decimal: $0.perDayWei)?.aeth } ?? "10",
                                   allow: allow, expires: expires, gas: gas)
            guard result["final"] as? Bool != false else { return result }
            try Payees.add(name: name, address: address)
            return result
        default: throw AgentError.input("payee list | pending | add --name NAME --address 0x...")
        }
    }

    static func token(_ args: [String]) throws -> [String: Any] {
        guard args.first == "allow" else { throw AgentError.input("token allow --address 0x... --per-tx UNITS --per-day UNITS") }
        let f = flags(args.dropFirst())
        guard let address = f["address"], let p = f["per_tx"], let d = f["per_day"] else {
            throw AgentError.input("token allow --address 0x... --per-tx UNITS --per-day UNITS")
        }
        Tools.configure()
        let key = try Keys.owner(), pk = Keys.publicKey(key)
        let prepared = try prepareSetSessionToken(ownerPublicKey: pk, token: address, perPayment: p, perDay: d, validators: Tools.validatorCount)
        let sig = try key.signature(for: prepared.signingMessage)
        let hash = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig.rawRepresentation, p256PublicKey: pk)
        for _ in 0..<60 {
            if let r = try? receipt(txHash: hash) {
                guard r.success else { throw AgentError.io("token policy failed (tx \(hash)); this needs a new-genesis account contract") }
                return ["hash": hash, "final": true, "token": address, "per_payment_units": p, "per_day_units": d]
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return ["hash": hash, "final": false]
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
                var out = (try Tools.session(id)).map(Tools.describe) ?? ["hash": hash]
                out["final"] = true
                out["hash"] = hash
                return out
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return ["hash": hash, "final": false]
    }
}
