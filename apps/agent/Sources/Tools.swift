import Foundation

/// The agent's actions. Each returns a JSON object (for MCP and the CLI alike).
/// Reads verify on this Mac (light client); payments are signed by the agent's
/// Secure Enclave key after the owner-signed policy allows them.
enum Tools {
    typealias Args = [String: Any]

    struct Spec {
        let name: String
        let description: String
        let schema: [String: Any]
        let readOnly: Bool
        let run: (Args) throws -> [String: Any]
    }

    static let all: [Spec] = [
        Spec(name: "aether_status", description: "Network status: latest block, validators, the fee of a plain transfer, and how this Mac reaches the network.",
             schema: object([:]), readOnly: true) { _ in try status() },
        Spec(name: "aether_wallet", description: "The agent's account: address, balance (verified on this Mac against the validators' signature), gas balance, the spending limits the account contract enforces, and how much is left today.",
             schema: object([:]), readOnly: true) { _ in try wallet() },
        Spec(name: "aether_balance", description: "Verified balance and nonce of any address.",
             schema: object(["address": prop("string", "0x address")], required: ["address"]), readOnly: true) { a in try balance(a) },
        Spec(name: "aether_send", description: "Pay AETH from the agent's account and wait for finality (~seconds). The account contract enforces the owner's limits (per payment, per 24 h, recipients). Use dry_run to check without paying.",
             schema: object(["to": prop("string", "0x recipient"), "amount": prop("string", "AETH, decimal, e.g. \"0.5\""),
                             "dry_run": prop("boolean", "only check limits and fee")], required: ["to", "amount"]), readOnly: false) { a in try send(a) },
        Spec(name: "aether_pay_many", description: "Pay several recipients in ONE transaction (all or nothing). Checked against the spending limits as one total.",
             schema: object(["payments": ["type": "array", "description": "list of {to, amount}",
                                          "items": object(["to": prop("string", "0x recipient"), "amount": prop("string", "AETH")], required: ["to", "amount"])],
                             "dry_run": prop("boolean", "only check limits")], required: ["payments"]), readOnly: false) { a in try payMany(a) },
        Spec(name: "aether_receipt", description: "Whether a transaction is final, in which block, and if it succeeded.",
             schema: object(["hash": prop("string", "0x tx hash")], required: ["hash"]), readOnly: true) { a in try receiptTool(a) },
        Spec(name: "aether_history", description: "Payments this agent made (newest first).",
             schema: object(["limit": prop("integer", "max entries, default 20")]), readOnly: true) { a in try history(a) },
        Spec(name: "aether_get_test_tokens", description: "Testnet only: receive 10 test AETH (no value) into the agent's account (rate-limited by the network).",
             schema: object([:]), readOnly: false) { _ in try testTokens() },
    ]

    // MARK: setup shared by every tool

    private static var validators: UInt32 = 4
    private static var configured = false

    static func configure() {
        guard !configured else { return }
        configured = true
        let env = ProcessInfo.processInfo.environment["AETHER_NETWORK"]
        let url = env.map { URL(fileURLWithPath: $0) } ?? Paths.network
        if let json = try? String(contentsOf: url, encoding: .utf8), let n = try? configureNetwork(networkJson: json) {
            validators = n
        }
    }

    static var validatorCount: UInt32 { configure(); return validators }

    /// The agent's account (owner key's address) and its gas payer (agent key's address).
    struct Identity {
        let account: String
        let gasPayer: String
        let agentKey: Data
        let agentCode: String
    }

    static func identity() throws -> Identity {
        let owner = Keys.publicKey(try Keys.owner())
        let agent = Keys.publicKey(try Keys.agent())
        return Identity(account: try accountAddress(p256PublicKey: owner), gasPayer: try accountAddress(p256PublicKey: agent),
                        agentKey: agent, agentCode: try recoveryKeyCode(p256PublicKey: agent))
    }

    private static func aeth(_ wei: String) -> String { Wei(decimal: wei)?.aeth ?? wei }

    /// The on-chain session of the agent's account, if it is this agent's key.
    static func session(_ id: Identity) throws -> SessionStatus? {
        let s = try sessionStatus(account: id.account, validators: validators)
        return s.exists && s.keyCode == id.agentCode ? s : nil
    }

    static func describe(_ s: SessionStatus) -> [String: Any] {
        ["per_payment_aeth": aeth(s.perPaymentWei), "per_day_aeth": aeth(s.perDayWei),
         "left_now_aeth": aeth(s.leftWei),
         "daily_rule": "at most per_day in any 24 hours (today plus yesterday, UTC)",
         "allowed_recipients": s.allow.isEmpty ? ["anyone"] : s.allow,
         "expires": s.expires == 0 ? "never" : ISO8601DateFormatter().string(from: Date(timeIntervalSince1970: TimeInterval(s.expires))),
         "enforced_by": "the account contract on chain"]
    }

    // MARK: tools

    static func status() throws -> [String: Any] {
        configure()
        let s = try chainStatus()
        return ["chain_id": s.chainId, "height": s.height, "validators": validators, "pending_txs": s.mempool,
                "transfer_fee_aeth": aeth(s.transferFeeWei), "connection": connection()]
    }

    static func wallet() throws -> [String: Any] {
        configure()
        let id = try identity()
        let acc = try verifiedAccount(address: id.account, validators: validators)
        let gas = try verifiedAccount(address: id.gasPayer, validators: validators)
        var out: [String: Any] = ["address": id.account, "balance_aeth": aeth(acc.balanceWei), "verified_at_block": acc.certifiedBlock,
                                  "gas_payer": id.gasPayer, "gas_balance_aeth": aeth(gas.balanceWei),
                                  "keys": "Secure Enclave (cannot be exported); the owner key needs Touch ID"]
        if let s = try session(id) {
            out["limits"] = describe(s)
        } else {
            out["limits"] = "not set: payments are disabled until the owner runs `aether-agent policy set` (Touch ID)"
        }
        return out
    }

    static func balance(_ a: Args) throws -> [String: Any] {
        configure()
        guard let addr = a["address"] as? String else { throw AgentError.input("address") }
        let acc = try verifiedAccount(address: addr, validators: validators)
        return ["address": acc.address, "balance_aeth": aeth(acc.balanceWei), "nonce": acc.nonce, "verified_at_block": acc.certifiedBlock]
    }

    static func send(_ a: Args) throws -> [String: Any] {
        guard let to = a["to"] as? String, let amount = a["amount"] as? String else { throw AgentError.input("to and amount are required") }
        return try pay([(to, amount)], dryRun: a["dry_run"] as? Bool ?? false)
    }

    static func payMany(_ a: Args) throws -> [String: Any] {
        guard let list = a["payments"] as? [[String: Any]], !list.isEmpty else { throw AgentError.input("payments must be a non-empty list") }
        let pairs = try list.map { p -> (String, String) in
            guard let to = p["to"] as? String, let amt = p["amount"] as? String else { throw AgentError.input("each payment needs to and amount") }
            return (to, amt)
        }
        return try pay(pairs, dryRun: a["dry_run"] as? Bool ?? false)
    }

    /// Check against the on-chain limits (to fail early with a clear message; the
    /// contract enforces them regardless), sign with the session key, submit from
    /// the gas payer, wait for finality.
    private static func pay(_ payments: [(String, String)], dryRun: Bool) throws -> [String: Any] {
        configure()
        let id = try identity()
        guard let s = try session(id) else { throw AgentError.policy("no limits set for this agent; the owner must run `aether-agent policy set`") }
        var total = Wei.zero
        var parsed: [Payment] = []
        for (to, amt) in payments {
            guard let w = Wei(aeth: amt), w.value > 0 else { throw AgentError.input("amount \"\(amt)\" is not a positive AETH amount") }
            guard to.hasPrefix("0x"), to.count == 42 else { throw AgentError.input("\(to) is not a 0x address") }
            if !s.allow.isEmpty && !s.allow.contains(where: { $0.lowercased() == to.lowercased() }) { throw AgentError.policy("\(to) is not an allowed recipient") }
            parsed.append(Payment(to: to, valueWei: w.description))
            guard let t = total.adding(w) else { throw AgentError.input("the amounts add up to more than any balance") }
            total = t
        }
        let perPayment = Wei(decimal: s.perPaymentWei) ?? .zero
        if perPayment < total { throw AgentError.policy("\(total.aeth) AETH is over the per-payment limit of \(aeth(s.perPaymentWei)) AETH") }
        let left = Wei(decimal: s.leftWei) ?? .zero
        if left < total { throw AgentError.policy("only \(left.aeth) AETH may be paid now (limit \(aeth(s.perDayWei)) AETH in any 24 h)") }
        let acc = try verifiedAccount(address: id.account, validators: validators)
        if (Wei(decimal: acc.balanceWei) ?? .zero) < total { throw AgentError.policy("the account holds \(aeth(acc.balanceWei)) AETH") }
        if dryRun {
            return ["ok": true, "would_pay_aeth": total.aeth, "left_after_aeth": Wei(value: left.value - total.value).aeth]
        }
        let request = try prepareSessionPayment(account: id.account, payments: parsed, validators: validators)
        let agent = try Keys.agent()
        let sessionSig = try agent.signature(for: request.message)
        let prepared = try prepareSessionSubmit(sessionPublicKey: id.agentKey, request: request, sessionSignature: sessionSig.rawRepresentation)
        let txSig = try agent.signature(for: prepared.signingMessage)
        let hash = try submitSigned(envelopeJson: prepared.envelopeJson, signature: txSig.rawRepresentation, p256PublicKey: id.agentKey)
        History.append(HistoryEntry(date: Date(), to: parsed.map(\.to), totalWei: total.description, hash: hash))
        var out: [String: Any] = ["hash": hash, "paid_aeth": total.aeth, "recipients": parsed.count]
        if let r = waitForReceipt(hash) {
            out["final"] = true
            out["success"] = r.success
            out["block"] = r.height
            if !r.success { out["note"] = "the account contract refused it (limits or recipients changed?); nothing was paid" }
        } else {
            out["final"] = false
            out["note"] = "not final after 30 s; check with aether_receipt"
        }
        return out
    }

    private static func waitForReceipt(_ hash: String) -> TxReceipt? {
        for _ in 0..<60 {
            if let r = try? receipt(txHash: hash) { return r }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return nil
    }

    static func receiptTool(_ a: Args) throws -> [String: Any] {
        configure()
        guard let h = a["hash"] as? String else { throw AgentError.input("hash") }
        guard let r = try receipt(txHash: h) else { return ["hash": h, "final": false] }
        return ["hash": h, "final": true, "success": r.success, "block": r.height, "gas_used": r.gasUsed]
    }

    static func history(_ a: Args) throws -> [String: Any] {
        let limit = (a["limit"] as? Int) ?? Int(a["limit"] as? String ?? "") ?? 20
        let f = ISO8601DateFormatter()
        let items = History.load().prefix(max(1, limit)).map { e -> [String: Any] in
            ["date": f.string(from: e.date), "to": e.to, "total_aeth": aeth(e.totalWei), "hash": e.hash]
        }
        return ["payments": Array(items)]
    }

    static func testTokens() throws -> [String: Any] {
        configure()
        let id = try identity()
        let hash = try devnetFaucet(to: id.account, valueWei: "0")
        let r = waitForReceipt(hash)
        return ["hash": hash, "to": id.account, "received_aeth": "10", "final": r != nil,
                "next": "Test tokens are in the agent account. Its gas payer needs gas too: the owner's `aether-agent policy set` sends some."]
    }

    // MARK: JSON schema helpers

    static func object(_ props: [String: Any], required: [String] = []) -> [String: Any] {
        ["type": "object", "properties": props, "required": required, "additionalProperties": false]
    }

    static func prop(_ type: String, _ description: String) -> [String: Any] {
        ["type": type, "description": description]
    }
}
