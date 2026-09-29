import Foundation

// The node's finalized activity index is display data. VerifiedAccount remains
// the only source of the AETH balance shown by the wallet.
struct ChainHistoryPage: Decodable, Sendable {
    let entries: [ChainHistoryEntry]
    let nextCursor: String?
    let historyStart: UInt64
    let indexedHeight: UInt64

    static func decode(_ json: String) throws -> Self {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(Self.self, from: Data(json.utf8))
    }
}

struct ChainTokenMove: Decodable, Sendable {
    let token: String
    let from: String
    let to: String
    let amount: String
}

struct ChainPairSwap: Decodable, Sendable {
    let pair: String
    let amount0In: String
    let amount1In: String
    let amount0Out: String
    let amount1Out: String
}

struct ChainHistoryEntry: Decodable, Sendable {
    let address: String
    let height: UInt64
    let txIndex: UInt32
    let txHash: String
    let timestampMs: UInt64
    let direction: String
    let kind: String
    let from: String?
    let to: String?
    let valueWei: String
    let method: String?
    let approvalAmount: String?
    let approvalSpender: String?
    let contractAddress: String?
    let success: Bool
    let tokens: [ChainTokenMove]
    let pairSwaps: [ChainPairSwap]?
    let nativeReceivedWei: String?
    let nativePayoutSource: String?
}

struct ChainTokenName: Sendable {
    let symbol: String
    let decimals: Int
    let origin: String?
}

struct ChainNames: Sendable {
    var router: String?
    var launchpad: String?
    var tokenFactory: String?
    var waeth: String?
    var tokens: [String: ChainTokenName] = [:]
}

enum ChainActivity {
    static func short(_ address: String) -> String {
        address.count > 12 ? "\(address.prefix(6))…\(address.suffix(4))" : address
    }

    static func tokenShort(_ address: String) -> String {
        let body = address.hasPrefix("0x") ? String(address.dropFirst(2)) : address
        return body.count > 8 ? "0x\(body.prefix(4))…\(body.suffix(4))" : address
    }

    static func units(_ raw: String, decimals: Int = 18) -> String {
        guard decimals >= 0, raw.allSatisfy(\.isNumber), !raw.isEmpty else { return "?" }
        let padded = String(repeating: "0", count: max(0, decimals + 1 - raw.count)) + raw
        guard decimals > 0 else {
            let whole = padded.drop(while: { $0 == "0" })
            return whole.isEmpty ? "0" : String(whole)
        }
        let whole = padded.dropLast(decimals).drop(while: { $0 == "0" })
        let fraction = String(padded.suffix(decimals)).replacingOccurrences(of: "0+$", with: "", options: .regularExpression)
        return "\(whole.isEmpty ? "0" : String(whole))\(fraction.isEmpty ? "" : ".\(fraction.prefix(6))")"
    }

    static func title(_ row: ChainHistoryEntry, names: ChainNames) -> String {
        let me = row.address.lowercased()
        let from = row.from?.lowercased()
        let to = row.to?.lowercased()
        let incoming = row.tokens.filter { $0.to.lowercased() == me && $0.from.lowercased() != me }
        let outgoing = row.tokens.filter { $0.from.lowercased() == me && $0.to.lowercased() != me }
        let amount = units(row.valueWei)
        let contract = names.router.map({ to == $0.lowercased() }) == true ? "router" :
            names.launchpad.map({ to == $0.lowercased() }) == true ? "launchpad" :
            names.tokenFactory.map({ to == $0.lowercased() }) == true ? "factory" : ""
        func tokenText(_ move: ChainTokenMove) -> String {
            let token = names.tokens[move.token.lowercased()]
            let symbol = token?.symbol ?? "Token"
            let name = "\(symbol) · \(tokenShort(move.token))"
            let warning = token?.origin == "launchpad" ? " (Launchpad · unverified)" : token == nil ? " (unverified)" : ""
            let amount = token.map { units(move.amount, decimals: $0.decimals) } ?? "\(move.amount) base units"
            return "\(amount) \(name)\(warning)"
        }
        if row.kind == "node_reward" { return "Node reward \(amount) \(Brand.coinTicker)" }
        if row.kind == "proof_reward" { return "Proof reward \(amount) \(Brand.coinTicker)" }
        if row.kind == "registration" { return "Registered a voting node" }
        if !row.success { return "Failed call \(short(row.to ?? row.address)) (method \(row.method ?? "0x"))" }
        if row.kind == "deploy" { return "Deployed contract \(short(row.contractAddress ?? row.address))" }
        if contract == "router", row.pairSwaps?.isEmpty == false,
           ["0x38ed1739", "0xac344b4d", "0x3f070ce1"].contains(row.method ?? "") {
            let spent = outgoing.first.map(tokenText) ?? "\(amount) \(Brand.coinTicker)"
            let nativeIn = names.waeth.flatMap { waeth in
                row.nativePayoutSource?.lowercased() == waeth.lowercased()
                    ? row.nativeReceivedWei.map { "\(units($0)) \(Brand.coinTicker)" } : nil
            }
            let got = incoming.last.map(tokenText) ?? nativeIn ?? "\(Brand.coinTicker)"
            return "Swapped \(spent) → \(got)"
        }
        if contract == "router", ["0xe8e33700", "0xcf2df7c6"].contains(row.method ?? "") { return "Added liquidity" }
        if contract == "router", ["0xbaa2abde", "0x0fb9ca68"].contains(row.method ?? "") { return "Removed liquidity" }
        if contract == "launchpad", row.method == "0x42a81515" { return "Launched a token" }
        if contract == "launchpad", row.method == "0xcce7ec13" { return "Bought on launchpad · \(incoming.first.map(tokenText) ?? "\(amount) \(Brand.coinTicker)")" }
        if contract == "launchpad", row.method == "0x6a272462" { return "Sold on launchpad · \(outgoing.first.map(tokenText) ?? "token")" }
        if contract == "factory", ["0x3ca6d100", "0xc7ff321d"].contains(row.method ?? "") { return "Created a token" }
        if row.method == "0x095ea7b3" {
            let action = row.approvalAmount == "0" ? "Revoked" : "Approved"
            return "\(action) token \(tokenShort(row.to ?? row.address))\(row.approvalSpender.map { " for \(short($0))" } ?? "")"
        }
        if row.kind == "native_transfer", from != me { return "Received \(amount) \(Brand.coinTicker) from \(short(row.from ?? ""))" }
        if row.kind == "native_transfer" { return "Sent \(amount) \(Brand.coinTicker) to \(short(row.to ?? ""))" }
        if let receipt = incoming.first, from != me { return "Received \(tokenText(receipt)) from \(short(receipt.from))" }
        if let sent = outgoing.first, row.method == "0xa9059cbb" { return "Sent \(tokenText(sent)) to \(short(sent.to))" }
        let deltas = outgoing.map { "−\(tokenText($0))" } + incoming.map { "+\(tokenText($0))" }
        let native = row.valueWei != "0" ? ["−\(amount) \(Brand.coinTicker)"] : []
        let changes = (native + deltas).isEmpty ? "" : " · \((native + deltas).joined(separator: ", "))"
        return "Contract call \(short(row.to ?? row.address)) (method \(row.method ?? "0x"))\(changes)"
    }

    static func validLinkedAddress(_ value: String, own: String, existing: [String]) -> String? {
        let address = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard address.hasPrefix("0x"), address.count == 42, address.dropFirst(2).allSatisfy(\.isHexDigit), address != own.lowercased(),
              !existing.contains(where: { $0.lowercased() == address }) else { return nil }
        return address
    }

    /// Exact decimal subtraction for the verified-balance fallback.
    static func rise(_ newer: String, over older: String) -> String? {
        guard newer.allSatisfy(\.isNumber), older.allSatisfy(\.isNumber) else { return nil }
        let a = Array(newer.reversed()).compactMap(\.wholeNumberValue)
        let b = Array(older.reversed()).compactMap(\.wholeNumberValue)
        var borrow = 0, digits: [Int] = []
        for i in 0..<max(a.count, b.count) {
            var n = (i < a.count ? a[i] : 0) - (i < b.count ? b[i] : 0) - borrow
            borrow = n < 0 ? 1 : 0
            if n < 0 { n += 10 }
            digits.append(n)
        }
        guard borrow == 0 else { return nil }
        while digits.count > 1 && digits.last == 0 { digits.removeLast() }
        return digits.allSatisfy({ $0 == 0 }) ? nil : digits.reversed().map { String($0) }.joined()
    }
}

struct IncomingNoticeState: Codable {
    var height: UInt64
    var hashes: Set<String>

    mutating func consume(_ entries: [ChainHistoryEntry]) -> [ChainHistoryEntry] {
        let incoming = entries.filter { $0.success && $0.direction == "in" && ($0.kind == "native_transfer" || $0.kind == "erc20_transfer") }
        let fresh = incoming.filter { $0.height > height || ($0.height == height && !hashes.contains($0.txHash.lowercased())) }
        if let newest = entries.map(\.height).max(), newest > height { height = newest; hashes = [] }
        hashes.formUnion(entries.filter { $0.height == height }.map { $0.txHash.lowercased() })
        return fresh
    }
}
