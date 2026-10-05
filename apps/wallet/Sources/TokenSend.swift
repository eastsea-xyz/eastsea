import Foundation

// Sending ERC-20 tokens, and the checks the send flow runs around it
// (docs/research/token-spam-2026.md §6, adopted 2026-09-28). Everything here is
// pure: no node, no key, no UI. Storage rule: these checks read public chain
// data and settings on this device. Nothing new is written on chain — the
// derived sets below ("addresses I sent to", "tokens my own transactions
// touched") come from the receipts/activity the wallet already tracks, and the
// only things kept are the user's own choices (hidden or promoted tokens).

/// Exact decimal amounts for a token of any number of decimals. No floating
/// point anywhere: amounts are decimal digit strings of base units.
enum TokenAmount {
    /// "1.5" (with at most `decimals` fraction digits) → base units ("15…0");
    /// leading and trailing spaces are ignored, anything else is rejected.
    /// Returns "0" for zero, with leading zeros stripped.
    static func parse(_ text: String, decimals: Int) -> String? {
        guard decimals >= 0 else { return nil }
        let parts = text.trimmingCharacters(in: .whitespacesAndNewlines)
            .split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count <= 2, let whole = parts.first else { return nil }
        let frac = parts.count == 2 ? String(parts[1]) : ""
        guard (whole + frac).allSatisfy({ $0.isASCII && $0.isNumber }),
              !whole.isEmpty || !frac.isEmpty, frac.count <= decimals else { return nil }
        let digits = String(whole) + frac + String(repeating: "0", count: decimals - frac.count)
        let trimmed = digits.drop(while: { $0 == "0" })
        return trimmed.isEmpty ? "0" : String(trimmed)
    }

    /// All fraction digits, trailing zeros kept (the Max button fills the field
    /// with the exact balance, unlike `TokenUnits.format` which caps at 6).
    static func exact(_ raw: String, decimals: Int) -> String {
        let raw = raw.allSatisfy({ $0.isASCII && $0.isNumber }) ? raw : "0"
        let padded = String(repeating: "0", count: max(0, decimals + 1 - raw.count)) + raw
        let whole = padded.dropLast(decimals).drop(while: { $0 == "0" })
        guard decimals > 0 else { return whole.isEmpty ? "0" : String(whole) }
        return "\(whole.isEmpty ? "0" : String(whole)).\(padded.suffix(decimals))"
    }
}

/// The ERC-20 pieces a token send needs: `transfer(address,uint256)` calldata,
/// and recognizing one (the approval UI already labels the selector).
enum ERC20 {
    static let transferSelector = "a9059cbb"

    /// Calldata for `transfer(to, amount)`; `amount` is base units (decimal).
    /// Nil when the amount does not fit a uint256 or the address is malformed.
    static func transferCalldata(to: String, amount: String) -> String? {
        guard SendSafety.isValidAddress(to), let arg = EVMABI.word(uintDecimal: amount) else { return nil }
        return EVMABI.call(transferSelector, [EVMABI.word(address: to), arg])
    }
}

extension EVMABI {
    /// A 32-byte word from a decimal string (exact for any uint256); nil when
    /// the digits overflow or are not plain ASCII digits.
    static func word(uintDecimal v: String) -> String? {
        guard !v.isEmpty, v.allSatisfy({ $0.isASCII && $0.isNumber }) else { return nil }
        var digits = v.map { Int(String($0))! }
        var hex: [Character] = []
        while digits.count > 1 || digits[0] >= 1 {
            var carry = 0
            for i in digits.indices {
                let cur = carry * 10 + digits[i]
                digits[i] = cur / 16
                carry = cur % 16
            }
            hex.append(Character(String(carry, radix: 16)))
            while digits.count > 1, digits.first == 0 { digits.removeFirst() }
        }
        guard hex.count <= 64 else { return nil }
        return String(repeating: "0", count: 64 - hex.count) + String(hex.reversed())
    }
}

/// The send-flow safety heuristics of token-spam-2026.md §6.3, as pure
/// functions the sheet and the extension both mirror.
enum SendSafety {
    static func isValidAddress(_ s: String) -> Bool {
        let h = s.lowercased().dropFirst(s.lowercased().hasPrefix("0x") ? 2 : 0)
        return s.lowercased().hasPrefix("0x") && h.count == 40 && h.allSatisfy { $0.isHexDigit }
    }

    struct AddressRisk: Equatable {
        /// An address in the history whose first and last 4 hex chars match the
        /// recipient (but which is a different address): probable poisoning.
        let poisoningMatch: String?
        /// No send to this address has been recorded on this device.
        let firstSend: Bool
    }

    static func addressRisk(_ recipient: String, sentTo history: Set<String>) -> AddressRisk {
        let to = recipient.lowercased()
        guard isValidAddress(to) else { return AddressRisk(poisoningMatch: nil, firstSend: true) }
        let body = String(to.dropFirst(2))
        let prefix = body.prefix(4), suffix = body.suffix(4)
        var match: String?
        for h in history where h != to {
            let b = h.hasPrefix("0x") ? String(h.dropFirst(2)) : h
            if b.count == 40, b.prefix(4) == prefix, b.suffix(4) == suffix { match = h; break }
        }
        return AddressRisk(poisoningMatch: match, firstSend: !history.contains(to))
    }

    /// Lowercase ASCII alphanumerics only, accents folded, so look-alike checks
    /// see through "ÆTH", "aeth " and zero-width tricks.
    static func normalized(_ s: String) -> String {
        let folded = (s.applyingTransform(StringTransform("Latin-ASCII"), reverse: false) ?? s)
            .applyingTransform(.stripCombiningMarks, reverse: false) ?? s
        return folded.lowercased().filter { $0.isASCII && $0.isLetter || $0.isNumber }
    }

    /// Does `token`'s symbol or name equal or resemble an official one?
    static func looksLikeOfficial(symbol: String, name: String, official: [(symbol: String, name: String)]) -> Bool {
        let s = normalized(symbol), n = normalized(name)
        for o in official {
            let os = normalized(o.symbol), on = normalized(o.name)
            if os.isEmpty { continue }
            if resembles(s, os) || (!on.isEmpty && (resembles(s, on) || resembles(n, os) || resembles(n, on))) { return true }
        }
        return false
    }

    /// Equal, one edit away (for symbols of 3+ chars), or containing an
    /// official symbol of 4+ chars (wrapped-coin style names such as "WDBLN").
    private static func resembles(_ a: String, _ b: String) -> Bool {
        if a == b { return true }
        if b.count >= 4, a.contains(b) { return true }
        return min(a.count, b.count) >= 3 && editDistance(a, b) <= 1
    }

    static func editDistance(_ a: String, _ b: String) -> Int {
        let x = Array(a), y = Array(b)
        var prev = Array(0...y.count)
        for i in 1...max(1, x.count) {
            var cur = [i] + Array(repeating: 0, count: y.count)
            for j in 1...y.count {
                cur[j] = min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (x[i - 1] == y[j - 1] ? 0 : 1))
            }
            prev = cur
            if i >= x.count { break }
        }
        return prev[y.count]
    }

    /// The human-readable reason from a node error like
    /// "execution reverted: 0x08c379a0…" (`Error(string)`), or the message as is.
    static func revertReason(_ nodeError: String) -> String {
        guard let at = nodeError.range(of: "0x08c379a0") else { return nodeError }
        let hex = Array(nodeError[at.upperBound...].filter { $0.isHexDigit })
        let words = stride(from: 0, to: hex.count - hex.count % 64, by: 64).map { String(hex[$0..<($0 + 64)]) }
        guard words.count >= 3, Int(words[0], radix: 16) == 32, let len = Int(words[1], radix: 16),
              len > 0, len <= (words.count - 2) * 32 else { return nodeError }
        let body = words[2...].joined()
        let bytes = stride(from: 0, to: len * 2, by: 2).compactMap {
            UInt8(String(body[body.index(body.startIndex, offsetBy: $0)...body.index(body.startIndex, offsetBy: $0 + 1)]), radix: 16)
        }
        let text = String(decoding: bytes, as: UTF8.self)
        return text.allSatisfy({ !$0.isNewline }) && !text.isEmpty ? "\"\(text)\"" : nodeError
    }
}

/// A token is never shown by its symbol alone (anyone can deploy "USDT"):
/// rows and pickers say "SYMBOL · 0x8a9B…F41c".
enum TokenLabel {
    /// "0x8a9B…F41c" — 4 hex chars each side, as in the research doc.
    static func short(_ address: String) -> String {
        let a = address.hasPrefix("0x") ? String(address.dropFirst(2)) : address
        return a.count > 8 ? "0x\(a.prefix(4))…\(a.suffix(4))" : address
    }

    /// "NEB · 0x8a9B…F41c"
    static func row(_ token: TokenInfo) -> String {
        "\(token.symbol) · \(short(token.address))"
    }
}

/// What a pre-send dry-run found.
enum DryRunOutcome: Equatable {
    /// The call ran through (a token transfer returned its `true`).
    case ok
    /// The node refused it (revert, honeypot, blocked transfer): the reason.
    case reverted(String)
    /// Nothing could be tried (no local node answered; an iPhone or a Mac
    /// whose node is off). Not a failure — an honest "unchecked".
    case unchecked
}

/// An error whose text is all the dry-run needs from a call that refused.
struct DryRunError: LocalizedError {
    let reason: String
    var errorDescription: String? { reason }
}

/// Runs a send as a stateless `eth_call` from this account before anything is
/// signed. The FFI's `ethCall` cannot say who is calling (from 0x0 every token
/// transfer reverts on the balance check), so the full call goes to the local
/// node's loopback port; `plainCall` (the FFI) stands in for plain native-coin
/// transfers when there is no local node — a contract that cannot receive
/// plain transfers reverts the same way it would on chain. Storage rule: a
/// dry-run's result is never stored — these checks read public chain data and
/// settings on this device, and nothing new is written on chain.
enum SendDryRun {
    /// The local node's JSON-RPC port (the one the wallet switches to).
    static let port: UInt16 = 18_545

    static func check(from: String, to: String, valueWei: String, data: String,
                      port: UInt16 = SendDryRun.port,
                      plainCall: ((String, String) throws -> String)? = nil) async -> DryRunOutcome {
        guard let word = EVMABI.word(uintDecimal: valueWei) else { return .unchecked }
        var req = URLRequest(url: URL(string: "http://127.0.0.1:\(port)/")!, timeoutInterval: 4)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let stripped = word.drop(while: { $0 == "0" })
        req.httpBody = try? JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "id": 1, "method": "eth_call",
                                                                   "params": [["from": from, "to": to, "value": stripped.isEmpty ? "0x0" : "0x" + stripped, "data": data], "latest"]])
        guard let (bytes, resp) = try? await URLSession.shared.data(for: req),
              (resp as? HTTPURLResponse)?.statusCode == 200,
              let obj = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any] else {
            if (data.isEmpty || data == "0x"), let plainCall {
                do {
                    _ = try plainCall(to, "0x")
                    return .ok
                } catch {
                    let text = (error as? LocalizedError)?.errorDescription ?? "\(error)"
                    return .reverted(SendSafety.revertReason(text))
                }
            }
            return .unchecked
        }
        if let err = obj["error"] as? [String: Any], let message = err["message"] as? String {
            return .reverted(SendSafety.revertReason(message))
        }
        guard let result = obj["result"] as? String else { return .unchecked }
        // An ERC-20 transfer answers a bool true; a token that answers nothing
        // or false did not confirm the transfer.
        if !(data.isEmpty || data == "0x"), result != "0x" + String(repeating: "0", count: 63) + "1" {
            return .reverted("the token contract did not confirm the transfer")
        }
        return .ok
    }
}

/// Which tokens the main Assets list shows (token-spam-2026.md §6.1): the native
/// coin and tokens this wallet acquired or moved by its own signed action, or that are
/// official (the bundled seed list and wrapped AETH), or that the user chose to
/// show. Everything else someone sent in goes to the collapsed "Unverified"
/// section, out of any total. Only the user's choices are stored, on this
/// device; the derived sets come from the wallet's own tracked transactions.
struct TokenDisplayPolicy: Equatable {
    /// Tokens a transaction this wallet signed touched (send, approve, call).
    var touched: Set<String> = []
    /// Official tokens (bundled seed list + wrapped AETH); always shown.
    var official: Set<String> = []
    /// Hidden by the user (this device).
    var hidden: Set<String> = []
    /// Moved to the main list by the user (this device).
    var shown: Set<String> = []

    func isMain(_ address: String) -> Bool {
        let a = address.lowercased()
        if hidden.contains(a) { return false }
        return official.contains(a) || touched.contains(a) || shown.contains(a)
    }

    func split(_ holdings: [TokenHolding]) -> (main: [TokenHolding], unverified: [TokenHolding]) {
        (holdings.filter { isMain($0.token.address) }, holdings.filter { !isMain($0.token.address) })
    }
}
