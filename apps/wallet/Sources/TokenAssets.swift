import Foundation

// ERC-20 tokens this wallet holds, found the way the agent's DEX tools find them
// (apps/agent/Sources/Dex.swift): the DEX token factory, its pools and the launchpad
// are enumerated with read-only `eth_call`s, then `balanceOf` is asked for each token.
// Unlike AETH, these balances are the node's answer, not light-client verified.
// Pure values and functions (the reader is passed in), so it is checked without a node
// (apps/wallet/Tests/assets).

/// Where to look for tokens on one chain (bundled `token-sources.json`).
struct TokenSources: Codable, Equatable {
    var network: String?
    var waeth: String?
    var router: String?
    var tokenFactory: String?
    var pairFactory: String?
    var launchpad: String?
    var seed: [String] = []

    private struct File: Decodable { let chains: [String: TokenSources] }

    static func parse(_ data: Data, chainId: UInt64) -> TokenSources? {
        (try? JSONDecoder().decode(File.self, from: data))?.chains[String(chainId)]
    }

    static func bundled(chainId: UInt64) -> TokenSources? {
        guard let url = Bundle.main.url(forResource: "token-sources", withExtension: "json"),
              let data = try? Data(contentsOf: url) else { return nil }
        return parse(data, chainId: chainId)
    }
}

struct TokenInfo: Codable, Equatable {
    let address: String
    let symbol: String
    let name: String
    let decimals: Int
    /// Which on-chain list the token was enumerated from: "seed" (the bundled
    /// official list), "dex" (the token factory), "pool" (a DEX pair side) or
    /// "launchpad" (the launchpad's own list — anyone can create those). Nil in
    /// catalogs written before this field existed.
    var origin: String?
}

/// A token with a non-zero balance.
struct TokenHolding: Codable, Equatable, Identifiable {
    let token: TokenInfo
    /// Base units, decimal string (exact).
    let balance: String
    var id: String { token.address }
    var amount: String { TokenUnits.format(balance, decimals: token.decimals) }
}

/// Tokens seen so far on a chain, and how far each list was read (only new entries
/// are read next time).
struct TokenCatalog: Codable, Equatable {
    var tokens: [String: TokenInfo] = [:]
    /// Addresses that are not tokens (no `decimals`): not asked again.
    var rejected: Set<String> = []
    var factoryRead: UInt64 = 0
    var pairsRead: UInt64 = 0
    var launchesRead: UInt64 = 0
}

enum TokenScanner {
    // Selectors (`forge inspect <Contract> methodIdentifiers`).
    enum Sel {
        static let allTokensLength = "dbb80e42", allTokens = "634282af"
        static let allPairsLength = "574f2ba3", allPairs = "1e3dd18b", token0 = "0dfe1681", token1 = "d21220a7"
        static let tokenCount = "9f181b5e", tokens = "4f64b2be"  // launchpad (CurveLaunch)
        static let symbol = "95d89b41", name = "06fdde03", decimals = "313ce567", balanceOf = "70a08231"
    }

    /// Caps per list, as in the agent.
    static let maxFactoryTokens: UInt64 = 500
    static let maxPools: UInt64 = 200
    static let maxLaunches: UInt64 = 500

    typealias Read = (_ to: String, _ data: String) throws -> String

    /// Read new tokens into `catalog`, then every known token's balance for `owner`.
    /// Returns the updated catalog and the non-zero holdings (by symbol); throws only
    /// when the balances cannot be read.
    static func scan(owner: String, sources: TokenSources, catalog: TokenCatalog, read: Read) throws -> (TokenCatalog, [TokenHolding]) {
        var cat = catalog
        discover(sources: sources, catalog: &cat, read: read)
        var held: [TokenHolding] = []
        for t in cat.tokens.values {
            let raw = try read(t.address, EVMABI.call(Sel.balanceOf, [EVMABI.word(address: owner)]))
            let bal = try EVMABI.uint(raw)
            if bal != "0" { held.append(TokenHolding(token: t, balance: bal)) }
        }
        held.sort { ($0.token.symbol.lowercased(), $0.token.address) < ($1.token.symbol.lowercased(), $1.token.address) }
        return (cat, held)
    }

    /// Enumerate the lists from where they were last read. A failure stops that list
    /// where it is (it resumes next time). Each address remembers the most
    /// specific list it was seen in (the launchpad's own list names its tokens).
    static func discover(sources: TokenSources, catalog cat: inout TokenCatalog, read: Read) {
        var found: [(address: String, origin: String)] = sources.seed.map { ($0, "seed") }
        found.append(contentsOf: [sources.waeth].compactMap { $0 }.map { ($0, "seed") })
        if let f = sources.tokenFactory {
            cat.factoryRead = list(f, count: Sel.allTokensLength, item: Sel.allTokens, from: cat.factoryRead, cap: maxFactoryTokens, read: read) { found.append(($0, "dex")) }
        }
        if let f = sources.pairFactory {
            cat.pairsRead = list(f, count: Sel.allPairsLength, item: Sel.allPairs, from: cat.pairsRead, cap: maxPools, read: read) { pair in
                for sel in [Sel.token0, Sel.token1] {
                    if let t = try? EVMABI.address(read(pair, EVMABI.call(sel))) { found.append((t, "pool")) }
                }
            }
        }
        if let l = sources.launchpad {
            cat.launchesRead = list(l, count: Sel.tokenCount, item: Sel.tokens, from: cat.launchesRead, cap: maxLaunches, read: read) { found.append(($0, "launchpad")) }
        }
        for (a, origin) in found.map({ ($0.address.lowercased(), $0.origin) }) {
            if let known = cat.tokens[a] {
                if rank(origin) > rank(known.origin ?? "unknown") { cat.tokens[a]?.origin = origin }
            } else if !cat.rejected.contains(a) {
                if var t = info(a, read: read) {
                    t.origin = origin
                    cat.tokens[a] = t
                } else {
                    cat.rejected.insert(a)
                }
            }
        }
    }

    /// The more specific provenance wins, so a launchpad token keeps its badge
    /// even after it graduates into a DEX pool.
    private static func rank(_ origin: String) -> Int {
        ["seed": 1, "pool": 2, "dex": 3, "launchpad": 4][origin] ?? 0
    }

    /// Entries `from..<min(length, cap)` of an on-chain address list; returns how far it got.
    private static func list(_ contract: String, count: String, item: String, from: UInt64, cap: UInt64, read: Read, each: (String) -> Void) -> UInt64 {
        guard let n = try? EVMABI.uint64(read(contract, EVMABI.call(count))) else { return from }
        var i = from
        while i < min(n, cap) {
            guard let a = try? EVMABI.address(read(contract, EVMABI.call(item, [EVMABI.word(uint: i)]))) else { break }
            each(a)
            i += 1
        }
        return i
    }

    /// Symbol, name and decimals; nil when `decimals` is missing or absurd.
    static func info(_ address: String, read: Read) -> TokenInfo? {
        guard let d = try? EVMABI.uint64(read(address, EVMABI.call(Sel.decimals))), d <= 77 else { return nil }
        let symbol = (try? EVMABI.string(read(address, EVMABI.call(Sel.symbol)))).flatMap { $0.isEmpty ? nil : $0 } ?? "???"
        let name = (try? EVMABI.string(read(address, EVMABI.call(Sel.name)))) ?? ""
        return TokenInfo(address: address.lowercased(), symbol: String(symbol.prefix(16)), name: String(name.prefix(48)), decimals: Int(d))
    }
}

enum TokenScanError: Error { case badAnswer }

/// The few Solidity ABI pieces the scan needs.
enum EVMABI {
    static func call(_ selector: String, _ words: [String] = []) -> String { "0x" + selector + words.joined() }

    static func word(address a: String) -> String {
        let h = a.lowercased().hasPrefix("0x") ? String(a.dropFirst(2)) : a.lowercased()
        return String(repeating: "0", count: max(0, 64 - h.count)) + h.lowercased()
    }

    static func word(uint v: UInt64) -> String {
        let h = String(v, radix: 16)
        return String(repeating: "0", count: 64 - h.count) + h
    }

    static func words(_ data: String) throws -> [String] {
        let h = data.hasPrefix("0x") ? String(data.dropFirst(2)) : data
        guard h.count % 64 == 0, h.allSatisfy(\.isHexDigit) else { throw TokenScanError.badAnswer }
        let c = Array(h)
        return stride(from: 0, to: c.count, by: 64).map { String(c[$0..<($0 + 64)]) }
    }

    /// Word `i` as a decimal string (exact, any size).
    static func uint(_ data: String, at i: Int = 0) throws -> String {
        let w = try words(data)
        guard i < w.count else { throw TokenScanError.badAnswer }
        return WeiMath.decimal("0x" + w[i])
    }

    static func uint64(_ data: String, at i: Int = 0) throws -> UInt64 {
        let w = try words(data)
        guard i < w.count, w[i].prefix(48).allSatisfy({ $0 == "0" }), let v = UInt64(w[i].suffix(16), radix: 16) else { throw TokenScanError.badAnswer }
        return v
    }

    static func address(_ data: String, at i: Int = 0) throws -> String {
        let w = try words(data)
        guard i < w.count else { throw TokenScanError.badAnswer }
        return "0x" + w[i].suffix(40).lowercased()
    }

    /// A dynamic `string` return value.
    static func string(_ data: String) throws -> String {
        let w = try words(data)
        let off = try uint64(data, at: 0)
        guard off % 32 == 0, Int(off / 32) < w.count else { throw TokenScanError.badAnswer }
        let at = Int(off / 32)
        let n = Int(try uint64(data, at: at))
        guard n <= 4096 else { throw TokenScanError.badAnswer }
        let hex = Array(w[(at + 1)...].joined().prefix(n * 2))
        guard hex.count == n * 2 else { throw TokenScanError.badAnswer }
        let bytes = stride(from: 0, to: hex.count, by: 2).map { UInt8(String(hex[$0..<($0 + 2)]), radix: 16) ?? 0 }
        return String(decoding: bytes, as: UTF8.self).trimmingCharacters(in: .controlCharacters)
    }
}

/// Base units <-> display text for any number of decimals (exact).
enum TokenUnits {
    /// At most 6 fraction digits, trailing zeros dropped.
    static func format(_ raw: String, decimals: Int) -> String {
        guard decimals > 0, raw != "0" else { return raw }
        let padded = String(repeating: "0", count: max(0, decimals + 1 - raw.count)) + raw
        let whole = padded.dropLast(decimals).drop(while: { $0 == "0" })
        var frac = String(padded.suffix(decimals).prefix(6))
        while frac.hasSuffix("0") { frac.removeLast() }
        let w = whole.isEmpty ? "0" : String(whole)
        if frac.isEmpty, w == "0" { return "<0.000001" }
        return frac.isEmpty ? w : "\(w).\(frac)"
    }
}
