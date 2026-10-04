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
                             "purpose": prop("string", "why the owner asked for this payment"),
                             "dry_run": prop("boolean", "only check limits and fee")], required: ["to", "amount", "purpose"]), readOnly: false) { a in try send(a) },
        Spec(name: "aether_pay_many", description: "Pay several recipients in ONE transaction (all or nothing). Checked against the spending limits as one total.",
             schema: object(["payments": ["type": "array", "description": "list of {to, amount}",
                                          "items": object(["to": prop("string", "0x recipient"), "amount": prop("string", "AETH")], required: ["to", "amount"])],
                             "purpose": prop("string", "why the owner asked for these payments"),
                             "dry_run": prop("boolean", "only check limits")], required: ["payments", "purpose"]), readOnly: false) { a in try payMany(a) },
        Spec(name: "aether_pay_token", description: "Pay one owner-listed ERC-20 token to an allowed payee. Token caps are in that token's base units and enforced on chain. Requires a new-genesis account contract.",
             schema: object(["token": prop("string", "token 0x address"), "to": prop("string", "payee 0x address"),
                             "amount": prop("string", "decimal token amount"), "purpose": prop("string", "why the owner asked for this payment"),
                             "dry_run": prop("boolean", "only check policy")], required: ["token", "to", "amount", "purpose"]), readOnly: false) { a in try payToken(a) },
        Spec(name: "aether_receipt", description: "Whether a transaction is final, in which block, and if it succeeded.",
             schema: object(["hash": prop("string", "0x tx hash")], required: ["hash"]), readOnly: true) { a in try receiptTool(a) },
        Spec(name: "aether_history", description: "Payments this agent made (newest first).",
             schema: object(["limit": prop("integer", "max entries, default 20")]), readOnly: true) { a in try history(a) },
        Spec(name: "aether_get_test_tokens", description: "Testnet only: receive 10 test AETH (no value) into the agent's account (rate-limited by the network).",
             schema: object([:]), readOnly: false) { _ in try testTokens() },
    ] + Dex.specs

    // MARK: setup shared by every tool

    private static var validators: UInt32 = 4
    private static var configured = false

    /// The app's node on this Mac, when its switch is on (the wallet uses it too).
    static let appNodePort: UInt16 = 18545

    static func configure() {
        guard !configured else { return }
        configured = true
        for url in networkCandidates() {
            if let json = try? String(contentsOf: url, encoding: .utf8), let n = try? configureNetwork(networkJson: json) {
                validators = n
                break
            }
        }
        // AETHER_LOCAL_NODE=<port> reads through that node on 127.0.0.1. Otherwise, like
        // the wallet, ask the app's node when it runs (it verifies every block itself;
        // the certificates and proofs are still checked here).
        let env = ProcessInfo.processInfo.environment
        if let p = env["AETHER_LOCAL_NODE"], let port = UInt16(p) {
            useLocalNode(port: port)
        } else if env["AETHER_NO_LOCAL_NODE"] == nil, localNodeHeight(port: appNodePort) != nil {
            useLocalNode(port: appNodePort)
        }
    }

    /// Where the network file (validators and committee key) can be: an explicit
    /// path, the agent's own copy, then the one inside the Aether app this binary
    /// ships in (also through the ~/.local/bin symlink), then the installed app.
    static func networkCandidates() -> [URL] {
        var urls: [URL] = []
        if let env = ProcessInfo.processInfo.environment["AETHER_NETWORK"], !env.isEmpty { urls.append(URL(fileURLWithPath: env)) }
        urls.append(Paths.network)
        let exe = (Bundle.main.executableURL ?? URL(fileURLWithPath: CommandLine.arguments[0])).resolvingSymlinksInPath()
        // …/EastSea.app/Contents/Helpers/aether-agent -> …/EastSea.app/Contents/Resources/network.json
        urls.append(exe.deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Resources/network.json"))
        urls.append(URL(fileURLWithPath: "/Applications/EastSea.app/Contents/Resources/network.json"))
        urls.append(URL(fileURLWithPath: "/Applications/Aether.app/Contents/Resources/network.json"))
        return urls
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
         "active": s.expires == 0 || s.expires > UInt64(Date().timeIntervalSince1970),
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
        return try pay([(to, amount)], purpose: try purpose(a), dryRun: a["dry_run"] as? Bool ?? false)
    }

    static func payMany(_ a: Args) throws -> [String: Any] {
        guard let list = a["payments"] as? [[String: Any]], !list.isEmpty else { throw AgentError.input("payments must be a non-empty list") }
        let pairs = try list.map { p -> (String, String) in
            guard let to = p["to"] as? String, let amt = p["amount"] as? String else { throw AgentError.input("each payment needs to and amount") }
            return (to, amt)
        }
        return try pay(pairs, purpose: try purpose(a), dryRun: a["dry_run"] as? Bool ?? false)
    }

    private static func purpose(_ a: Args) throws -> String {
        guard let text = (a["purpose"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines),
              !text.isEmpty, text.count <= 500 else { throw AgentError.input("purpose is required (1..500 characters)") }
        return text
    }

    private static func checkPayee(_ to: String, session: SessionStatus, purpose: String, amount: String, asset: String) throws {
        guard ABI.isAddress(to) else { throw AgentError.input("\(to) is not a 0x address") }
        if !AgentPolicy.permits(to, allow: session.allow) {
            try requestPayee(to, purpose: purpose, amount: amount, asset: asset)
            throw AgentError.policy("\(to) is not approved. The wallet has a payee approval request; the owner can add it with Touch ID. Nothing was sent")
        }
    }

    private static func requestPayee(_ to: String, purpose: String, amount: String, asset: String) throws {
        guard ABI.isAddress(to) else { throw AgentError.input("\(to) is not a 0x address") }
        if try Payees.request(address: to, purpose: purpose, amount: amount, asset: asset) { notifyOwnerOfPayeeRequest() }
    }

    private static func notifyOwnerOfPayeeRequest() {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", "display notification \"새 수취인 승인이 필요합니다. 지갑의 보안 화면에서 확인하세요.\" with title \"Aether 비서\""]
        process.standardOutput = Pipe()
        process.standardError = Pipe()
        try? process.run()
    }

    /// Check against the on-chain limits (to fail early with a clear message; the
    /// contract enforces them regardless), sign with the session key, submit from
    /// the gas payer, wait for finality.
    private static func pay(_ payments: [(String, String)], purpose: String, dryRun: Bool) throws -> [String: Any] {
        configure()
        let id = try identity()
        var total = Wei.zero
        var parsed: [Payment] = []
        for (to, amt) in payments {
            guard ABI.isAddress(to) else { throw AgentError.input("\(to) is not a 0x address") }
            guard let w = Wei(aeth: amt), w.value > 0 else { throw AgentError.input("amount \"\(amt)\" is not a positive AETH amount") }
            parsed.append(Payment(to: to, valueWei: w.description))
            guard let t = total.adding(w) else { throw AgentError.input("the amounts add up to more than any balance") }
            total = t
        }
        guard let s = try session(id) else {
            for (to, amt) in payments { try requestPayee(to, purpose: purpose, amount: amt, asset: "AETH") }
            throw AgentError.policy("agent stopped or no active session; approval request saved for the owner. Nothing was sent")
        }
        guard s.expires == 0 || s.expires > UInt64(Date().timeIntervalSince1970) else {
            throw AgentError.policy("the session expired; the owner must run `aether-agent policy renew` with Touch ID")
        }
        for (to, amt) in payments {
            try checkPayee(to, session: s, purpose: purpose, amount: amt, asset: "AETH")
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
        try History.submit(PendingPayment(date: Date(), to: parsed.map(\.to), totalWei: total.description, hash: hash,
                                          purpose: purpose, payeeNames: parsed.map { Payees.name($0.to) ?? $0.to },
                                          asset: "AETH", amount: total.aeth))
        var out: [String: Any] = ["hash": hash, "paid_aeth": total.aeth, "recipients": parsed.count]
        if let r = waitForReceipt(hash) {
            out["final"] = true
            out["success"] = r.success
            out["block"] = r.height
            try History.finalize(hash: hash, success: r.success)
            if !r.success { out["note"] = "the account contract refused it (limits or recipients changed?); nothing was paid" }
        } else {
            out["final"] = false
            out["note"] = "not final after 30 s; check with aether_receipt"
        }
        return out
    }

    static func payToken(_ a: Args) throws -> [String: Any] {
        configure()
        guard let tokenAddress = a["token"] as? String, let to = a["to"] as? String,
              let amount = a["amount"] as? String else { throw AgentError.input("token, to and amount are required") }
        let why = try purpose(a)
        let amountText = amount.trimmingCharacters(in: .whitespaces)
        guard ABI.isAddress(tokenAddress),
              amountText.filter({ $0 == "." }).count <= 1,
              amountText.allSatisfy({ $0.isNumber || $0 == "." }),
              amountText.contains(where: { $0.isNumber && $0 != "0" }) else {
            throw AgentError.input("token address or amount is invalid")
        }
        let id = try identity()
        guard let s = try session(id) else {
            try requestPayee(to, purpose: why, amount: amount, asset: tokenAddress)
            throw AgentError.policy("agent stopped or no active session; approval request saved for the owner. Nothing was sent")
        }
        guard s.expires == 0 || s.expires > UInt64(Date().timeIntervalSince1970) else {
            throw AgentError.policy("the session expired; the owner must renew with Touch ID")
        }
        try checkPayee(to, session: s, purpose: why, amount: amount, asset: tokenAddress)
        let token = try Dex.Reader(Dex.deployment()).token(tokenAddress)
        guard let units = U256(units: amount, decimals: token.decimals), !units.isZero else {
            throw AgentError.input("invalid \(token.symbol) amount")
        }
        let cap = try sessionTokenStatus(account: id.account, token: token.address, validators: validatorCount)
        guard let perPayment = U256(decimal: cap.perPayment), let left = U256(decimal: cap.leftNow),
              !perPayment.isZero else { throw AgentError.policy("token is not approved by owner") }
        if perPayment < units { throw AgentError.policy("over this token's per-payment limit") }
        if left < units { throw AgentError.policy("over this token's 24-hour limit") }
        if a["dry_run"] as? Bool == true { return ["ok": true, "would_pay": amount, "token": token.address, "payee": Payees.name(to) ?? to] }
        let request = try prepareSessionTokenPayment(account: id.account, token: token.address, to: to, amount: units.description, validators: validatorCount)
        let agent = try Keys.agent()
        let sessionSig = try agent.signature(for: request.message)
        let prepared = try prepareSessionTokenSubmit(sessionPublicKey: id.agentKey, request: request, sessionSignature: sessionSig.rawRepresentation)
        let txSig = try agent.signature(for: prepared.signingMessage)
        let hash = try submitSigned(envelopeJson: prepared.envelopeJson, signature: txSig.rawRepresentation, p256PublicKey: id.agentKey)
        try History.submit(PendingPayment(date: Date(), to: [to], totalWei: "0", hash: hash, purpose: why,
                                          payeeNames: [Payees.name(to) ?? to], asset: "\(token.symbol) · \(token.address)", amount: amount))
        guard let r = waitForReceipt(hash) else { return ["hash": hash, "final": false, "note": "check aether_receipt"] }
        try History.finalize(hash: hash, success: r.success)
        return ["hash": hash, "final": true, "success": r.success, "block": r.height, "amount": amount, "token": token.address]
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
        try History.finalize(hash: h, success: r.success)
        return ["hash": h, "final": true, "success": r.success, "block": r.height, "gas_used": r.gasUsed,
                "tx_link": "aether://tx?hash=\(h)"]
    }

    static func history(_ a: Args) throws -> [String: Any] {
        configure()
        for item in History.pending() {
            if let r = try? receipt(txHash: item.hash) { try History.finalize(hash: item.hash, success: r.success) }
        }
        let limit = (a["limit"] as? Int) ?? Int(a["limit"] as? String ?? "") ?? 20
        let f = ISO8601DateFormatter()
        let items = History.load().prefix(max(1, limit)).map { e -> [String: Any] in
            ["date": f.string(from: e.date), "to": e.to, "payee_names": e.payeeNames ?? e.to,
             "amount": e.amount ?? aeth(e.totalWei), "asset": e.asset ?? "AETH", "status": e.status ?? "legacy (unverified)",
             "purpose": e.purpose ?? "not recorded", "hash": e.hash, "tx_link": e.txLink ?? "aether://tx?hash=\(e.hash)"]
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
