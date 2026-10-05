import Foundation

/// Read-only EastSea DEX tools: pools, token facts, swap quotes. They read the
/// contracts with `eth_call` through the node the agent already uses and never
/// sign anything. Contract addresses come from the deployment files bundled at
/// build time (apps/agent/Resources/dex), keyed by chain id.
enum Dex {
    // Selectors (`forge inspect <Contract> methodIdentifiers` in aether-dex).
    enum Sel {
        static let allPairsLength = "574f2ba3", allPairs = "1e3dd18b", getPair = "e6a43905"
        static let token0 = "0dfe1681", token1 = "d21220a7", getReserves = "0902f1ac"
        static let symbol = "95d89b41", name = "06fdde03", decimals = "313ce567", totalSupply = "18160ddd", balanceOf = "70a08231"
        static let allTokensLength = "dbb80e42", allTokens = "634282af"
        static let getAmountsOut = "d06ca61f"
    }

    static let maxPools = 200
    static let maxFactoryTokens = 500
    static let defaultSlippagePercent = 0.5
    static let readNote = "Read from the node with eth_call (not light-client verified). Nothing was signed."

    struct Deployment {
        let chainId: UInt64
        let network: String
        let waeth: String
        let tokenFactory: String
        let pairFactory: String
        let router: String
        let seed: [String: String]  // symbol -> address
    }

    struct Token {
        let address: String  // WAETH's address for the native coin
        let symbol: String
        let name: String
        let decimals: Int
        let native: Bool
    }

    /// The DEX on the chain this agent is configured for, after checking the node agrees.
    static func deployment() throws -> Deployment {
        Tools.configure()
        let chain = configuredChainId()
        guard let json = DexDeployments.byChainId[chain], let d = json.data(using: .utf8),
              let v = try JSONSerialization.jsonObject(with: d) as? [String: Any],
              let c = v["contracts"] as? [String: String],
              let waeth = c["WAETH"], let tf = c["TokenFactory"], let pf = c["PairFactory"], let router = c["Router"] else {
            let known = DexDeployments.byChainId.keys.sorted().map(String.init).joined(separator: ", ")
            throw AgentError.io("no EastSea DEX is known on chain \(chain) (this agent knows chains: \(known))")
        }
        let node = try chainStatus().chainId
        guard node == chain else { throw AgentError.io("the node is on chain \(node), but this agent is configured for chain \(chain)") }
        let seed = ((v["seed"] as? [String: Any])?["tokens"] as? [String: String]) ?? [:]
        return Deployment(chainId: chain, network: v["network"] as? String ?? "chain \(chain)", waeth: waeth.lowercased(),
                          tokenFactory: tf, pairFactory: pf, router: router, seed: seed.mapValues { $0.lowercased() })
    }

    static func read(_ to: String, _ data: String) throws -> String { try ethCall(to: to, dataHex: data) }

    static func same(_ a: String, _ b: String) -> Bool { a.lowercased() == b.lowercased() }

    /// Token facts, cached per tool call.
    final class Reader {
        let dep: Deployment
        private var cache: [String: Token] = [:]

        init(_ dep: Deployment) { self.dep = dep }

        func token(_ address: String) throws -> Token {
            let a = address.lowercased()
            if let t = cache[a] { return t }
            let t: Token
            if a == dep.waeth {
                t = Token(address: a, symbol: "AETH", name: "Aether (wrapped)", decimals: 18, native: false)
            } else {
                let sym = (try? ABI.string(read(a, ABI.call(Sel.symbol)))) ?? "???"
                let name = (try? ABI.string(read(a, ABI.call(Sel.name)))) ?? ""
                let dec = try ABI.uint(read(a, ABI.call(Sel.decimals)))
                guard dec.limbs[0] <= 77, dec.limbs[1...].allSatisfy({ $0 == 0 }) else { throw AgentError.io("\(a) reports \(dec) decimals") }
                t = Token(address: a, symbol: sym, name: name, decimals: Int(dec.limbs[0]), native: false)
            }
            cache[a] = t
            return t
        }

        /// The native coin's ticker (legacy "AETH" still answers), "WAETH", a 0x
        /// address, or a symbol (deployment seed list first, then the token factory).
        func resolve(_ input: String) throws -> Token {
            let s = input.trimmingCharacters(in: .whitespaces)
            if s.uppercased() == Coin.ticker(dep.chainId) || s.uppercased() == "AETH" {
                return Token(address: dep.waeth, symbol: Coin.ticker(dep.chainId), name: Coin.name(dep.chainId), decimals: 18, native: true)
            }
            if s.uppercased() == "WAETH" { return try token(dep.waeth) }
            if ABI.isAddress(s) { return try token(s) }
            if let a = dep.seed.first(where: { $0.key.uppercased() == s.uppercased() })?.value { return try token(a) }
            let n = try ABI.uint(read(dep.tokenFactory, ABI.call(Sel.allTokensLength))).limbs[0]
            var hits: [Token] = []
            for i in 0..<min(n, UInt64(maxFactoryTokens)) {
                let a = try ABI.address(read(dep.tokenFactory, ABI.call(Sel.allTokens, [U256(i).hexWord])))
                if let t = try? token(a), t.symbol.uppercased() == s.uppercased() { hits.append(t) }
            }
            guard !hits.isEmpty else { throw AgentError.input("no token with the symbol \"\(s)\" on the DEX; pass its 0x address") }
            guard hits.count == 1 else {
                throw AgentError.input("\(hits.count) tokens use the symbol \"\(s)\" (\(hits.map(\.address).joined(separator: ", "))); pass the address")
            }
            return hits[0]
        }

        func pair(_ a: String, _ b: String) throws -> String? {
            let p = try ABI.address(read(dep.pairFactory, ABI.call(Sel.getPair, [ABI.word(address: a), ABI.word(address: b)])))
            return U256(hex: String(p.dropFirst(2)))?.isZero == false ? p : nil
        }

        /// Reserves of `pair` oriented as (reserve of `tokenIn`, reserve of the other).
        func reserves(_ pair: String, tokenIn: String) throws -> (U256, U256) {
            let t0 = try ABI.address(read(pair, ABI.call(Sel.token0)))
            let r = try read(pair, ABI.call(Sel.getReserves))
            let (r0, r1) = (try ABI.uint(r, at: 0), try ABI.uint(r, at: 1))
            return Dex.same(t0, tokenIn) ? (r0, r1) : (r1, r0)
        }
    }

    static func describe(_ t: Token, chainId: UInt64) -> [String: Any] {
        t.native ? ["symbol": Coin.ticker(chainId), "address": "native"] : ["symbol": t.symbol, "address": t.address]
    }

    /// Price as a plain decimal string (8 significant digits).
    static func number(_ x: Double) -> String {
        guard x.isFinite else { return "n/a" }
        if x == 0 { return "0" }
        let f = NumberFormatter()
        f.numberStyle = .decimal
        f.usesGroupingSeparator = false
        f.usesSignificantDigits = true
        f.maximumSignificantDigits = 8
        f.locale = Locale(identifier: "en_US_POSIX")
        return f.string(from: NSNumber(value: x)) ?? String(x)
    }

    static func real(_ v: U256, _ decimals: Int) -> Double { v.double / pow(10, Double(decimals)) }
}
