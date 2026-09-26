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
        Spec(name: "aether_wallet", description: "The agent's own account: address, balance (verified on this Mac against the validators' signature), spending limits, and how much is left today.",
             schema: object([:]), readOnly: true) { _ in try wallet() },
        Spec(name: "aether_balance", description: "Verified balance and nonce of any address.",
             schema: object(["address": prop("string", "0x address")], required: ["address"]), readOnly: true) { a in try balance(a) },
        Spec(name: "aether_send", description: "Pay AETH from the agent's account and wait for finality (~seconds). Checked against the owner's spending limits first. Use dry_run to check without paying.",
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
        Spec(name: "aether_get_test_tokens", description: "Testnet only: receive 10 test AETH (no value) into the agent's account.",
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

    private static func agentAddress() throws -> (String, Data) {
        let pk = Keys.publicKey(try Keys.agent())
        return (try accountAddress(p256PublicKey: pk), pk)
    }

    // MARK: tools

    static func status() throws -> [String: Any] {
        configure()
        let s = try chainStatus()
        return ["chain_id": s.chainId, "height": s.height, "validators": validators, "pending_txs": s.mempool,
                "transfer_fee_aeth": Wei(decimal: s.transferFeeWei)?.aeth ?? s.transferFeeWei, "connection": connection()]
    }

    static func wallet() throws -> [String: Any] {
        configure()
        let (addr, _) = try agentAddress()
        let acc = try verifiedAccount(address: addr, validators: validators)
        var out: [String: Any] = ["address": addr, "balance_aeth": Wei(decimal: acc.balanceWei)?.aeth ?? acc.balanceWei,
                                  "verified_at_block": acc.certifiedBlock, "key": "Secure Enclave (cannot be exported)"]
        if let p = try? PolicyStore.load() {
            let spent = (try? Ledger.spentLastDay(Ledger.load())) ?? .zero
            let day = Wei(aeth: p.maxPerDay) ?? .zero
            out["limits"] = ["per_tx_aeth": p.maxPerTx, "per_day_aeth": p.maxPerDay, "allowed_recipients": p.allow.isEmpty ? ["anyone"] : p.allow]
            out["left_today_aeth"] = spent < day ? Wei(value: day.value - spent.value).aeth : "0"
        } else {
            out["limits"] = "not set: payments are disabled until the owner runs `aether-agent init`"
        }
        return out
    }

    static func balance(_ a: Args) throws -> [String: Any] {
        configure()
        guard let addr = a["address"] as? String else { throw AgentError.input("address") }
        let acc = try verifiedAccount(address: addr, validators: validators)
        return ["address": acc.address, "balance_aeth": Wei(decimal: acc.balanceWei)?.aeth ?? acc.balanceWei, "nonce": acc.nonce, "verified_at_block": acc.certifiedBlock]
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

    /// Policy check, sign in the Secure Enclave, submit, wait for finality, log.
    private static func pay(_ payments: [(String, String)], dryRun: Bool) throws -> [String: Any] {
        configure()
        let policy = try PolicyStore.load()
        let ledger = try Ledger.load()
        let (addr, pk) = try agentAddress()
        let acc = try verifiedAccount(address: addr, validators: validators)
        // Only this binary holds the key, so the on-chain nonce counts its payments.
        guard acc.nonce <= UInt64(ledger.count) else {
            throw AgentError.policy("ledger.json is missing entries (chain nonce \(acc.nonce), ledger \(ledger.count)); the owner must run `aether-agent policy reset-ledger`")
        }
        var total = Wei.zero
        var parsed: [(String, Wei)] = []
        for (to, amt) in payments {
            guard let w = Wei(aeth: amt), w.value > 0 else { throw AgentError.input("amount \"\(amt)\" is not a positive AETH amount") }
            guard to.hasPrefix("0x"), to.count == 42 else { throw AgentError.input("\(to) is not a 0x address") }
            guard let perTx = Wei(aeth: policy.maxPerTx), !(perTx < w) else { throw AgentError.policy("\(amt) AETH is over the per-payment limit of \(policy.maxPerTx) AETH") }
            if !policy.allow.isEmpty && !policy.allow.contains(to.lowercased()) { throw AgentError.policy("\(to) is not on the allowed recipient list") }
            parsed.append((to, w))
            total = total + w
        }
        let spent = Ledger.spentLastDay(ledger)
        guard let day = Wei(aeth: policy.maxPerDay), !(day < spent + total) else {
            throw AgentError.policy("would exceed the daily limit: spent \(spent.aeth) + \(total.aeth) > \(policy.maxPerDay) AETH in 24 h")
        }
        guard let bal = Wei(decimal: acc.balanceWei), !(bal < total) else { throw AgentError.policy("balance \(Wei(decimal: acc.balanceWei)?.aeth ?? "?") AETH is less than \(total.aeth)") }
        if dryRun {
            return ["ok": true, "would_pay_aeth": total.aeth, "left_today_after_aeth": Wei(value: day.value - spent.value - total.value).aeth]
        }

        let prepared = parsed.count == 1
            ? try prepareTransfer(p256PublicKey: pk, to: parsed[0].0, valueWei: parsed[0].1.description)
            : try prepareBatch(p256PublicKey: pk, payments: parsed.map { Payment(to: $0.0, valueWei: $0.1.description) })
        let sig = try Keys.agent().signature(for: prepared.signingMessage)
        let hash = try submitSigned(envelopeJson: prepared.envelopeJson, signature: sig.rawRepresentation, p256PublicKey: pk)
        try Ledger.save([LedgerEntry(date: Date(), to: parsed.map(\.0), totalWei: total.description, hash: hash, nonce: prepared.nonce)] + ledger)
        var out: [String: Any] = ["hash": hash, "paid_aeth": total.aeth, "recipients": parsed.count]
        if let r = waitForReceipt(hash) {
            out["final"] = true
            out["success"] = r.success
            out["block"] = r.height
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
        let items = try Ledger.load().prefix(max(1, limit)).map { e -> [String: Any] in
            ["date": f.string(from: e.date), "to": e.to, "total_aeth": Wei(decimal: e.totalWei)?.aeth ?? e.totalWei, "hash": e.hash]
        }
        return ["payments": Array(items)]
    }

    static func testTokens() throws -> [String: Any] {
        configure()
        let (addr, _) = try agentAddress()
        let hash = try devnetFaucet(to: addr, valueWei: Wei(aeth: "10")!.description)
        let r = waitForReceipt(hash)
        return ["hash": hash, "received_aeth": "10", "final": r != nil]
    }

    // MARK: JSON schema helpers

    static func object(_ props: [String: Any], required: [String] = []) -> [String: Any] {
        ["type": "object", "properties": props, "required": required, "additionalProperties": false]
    }

    static func prop(_ type: String, _ description: String) -> [String: Any] {
        ["type": type, "description": description]
    }
}
