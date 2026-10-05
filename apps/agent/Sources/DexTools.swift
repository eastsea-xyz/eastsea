import Foundation

/// The DEX tools (read-only; see Dex.swift).
extension Dex {
    static let specs: [Tools.Spec] = {
        let t = Tools.coinTicker
        return [
        Tools.Spec(name: "dex_pools", description: "EastSea DEX pools: pair address, the two tokens, reserves, and the price of token0 in token1. Read-only.",
                   schema: Tools.object([:]), readOnly: true) { _ in try pools() },
        Tools.Spec(name: "dex_token_info", description: "A DEX token's symbol, name, decimals, total supply, and the agent account's balance of it. token: 0x address or symbol (e.g. \"NEB\"); \"\(t)\" (legacy \"AETH\") is the native coin. Read-only.",
                   schema: Tools.object(["token": Tools.prop("string", "0x address or symbol; \(t) = native"),
                                         "holder": Tools.prop("string", "optional 0x address to show the balance of instead of the agent's")],
                                        required: ["token"]), readOnly: true) { a in try tokenInfo(a) },
        Tools.Spec(name: "dex_quote", description: "Estimate a swap on the EastSea DEX without making it: best route (direct, or one hop through \(t) or a main DEX token), expected output, price impact, and the minimum received at the slippage (default 0.5%). Amounts are decimal token units. Agents cannot swap yet.",
                   schema: Tools.object(["from": Tools.prop("string", "token to sell: 0x address or symbol; \(t) = native"),
                                         "to": Tools.prop("string", "token to buy: 0x address or symbol; \(t) = native"),
                                         "amount": Tools.prop("string", "amount of `from`, decimal, e.g. \"1.5\""),
                                         "slippage_percent": Tools.prop("string", "tolerance for minimum_received, default 0.5")],
                                        required: ["from", "to", "amount"]), readOnly: true) { a in try quote(a) },
    ]
    }()

    static func pools() throws -> [String: Any] {
        let r = Reader(try deployment())
        let n = try ABI.uint(read(r.dep.pairFactory, ABI.call(Sel.allPairsLength))).limbs[0]
        var list: [[String: Any]] = []
        for i in 0..<min(n, UInt64(maxPools)) {
            let pair = try ABI.address(read(r.dep.pairFactory, ABI.call(Sel.allPairs, [U256(i).hexWord])))
            let t0 = try r.token(ABI.address(read(pair, ABI.call(Sel.token0))))
            let t1 = try r.token(ABI.address(read(pair, ABI.call(Sel.token1))))
            let res = try read(pair, ABI.call(Sel.getReserves))
            let (r0, r1) = (try ABI.uint(res, at: 0), try ABI.uint(res, at: 1))
            let (x0, x1) = (real(r0, t0.decimals), real(r1, t1.decimals))
            list.append(["pair": pair, "token0": describe(t0, chainId: r.dep.chainId), "token1": describe(t1, chainId: r.dep.chainId),
                         "reserve0": r0.units(t0.decimals), "reserve1": r1.units(t1.decimals),
                         "price_token1_per_token0": x0 > 0 ? number(x1 / x0) : "n/a",
                         "price_token0_per_token1": x1 > 0 ? number(x0 / x1) : "n/a"])
        }
        var out: [String: Any] = ["network": r.dep.network, "chain_id": r.dep.chainId, "count": n, "pools": list,
                                  "note": "\(Coin.ticker(r.dep.chainId)) in a pool is WAETH (wrapped 1:1). " + readNote]
        if n > UInt64(maxPools) { out["truncated_to"] = maxPools }
        return out
    }

    static func tokenInfo(_ a: Tools.Args) throws -> [String: Any] {
        guard let input = a["token"] as? String, !input.isEmpty else { throw AgentError.input("token is required") }
        let r = Reader(try deployment())
        let t = try r.resolve(input)
        var out: [String: Any] = ["symbol": t.symbol, "name": t.name, "decimals": t.decimals, "native": t.native,
                                  "address": t.native ? "native" : t.address, "chain_id": r.dep.chainId]
        if t.native {
            out["total_supply"] = NSNull()
            out["dex_address"] = r.dep.waeth
            out["note"] = "Native coin; pools hold it as WAETH (1:1). The balance is verified on this Mac."
        } else {
            out["total_supply"] = try ABI.uint(read(t.address, ABI.call(Sel.totalSupply))).units(t.decimals)
            out["note"] = readNote
        }
        let holder: String
        if let h = a["holder"] as? String, !h.isEmpty {
            guard ABI.isAddress(h) else { throw AgentError.input("\(h) is not a 0x address") }
            holder = h
        } else if let id = try? Tools.identity() {
            holder = id.account
        } else {
            out["balance"] = NSNull()
            out["balance_note"] = "the agent wallet is not set up (a human runs `aether-agent init`); pass holder to check an address"
            return out
        }
        out["holder"] = holder
        if t.native {
            out["balance"] = Wei(decimal: try verifiedAccount(address: holder, validators: Tools.validatorCount).balanceWei)?.aeth ?? "?"
        } else {
            out["balance"] = try ABI.uint(read(t.address, ABI.call(Sel.balanceOf, [ABI.word(address: holder)]))).units(t.decimals)
        }
        return out
    }

    struct Route {
        let path: [String]
        let out: U256
        let idealOut: Double  // at the pools' current prices, after the 0.3% fee per hop, with no price movement
    }

    static func quote(_ a: Tools.Args) throws -> [String: Any] {
        guard let f = a["from"] as? String, let t = a["to"] as? String, let amt = a["amount"] as? String else {
            throw AgentError.input("from, to and amount are required")
        }
        let slippage = (a["slippage_percent"] as? Double) ?? Double(a["slippage_percent"] as? String ?? "") ?? defaultSlippagePercent
        guard (0...50).contains(slippage) else { throw AgentError.input("slippage_percent must be between 0 and 50") }
        let r = Reader(try deployment())
        let from = try r.resolve(f), to = try r.resolve(t)
        guard !same(from.address, to.address) else { throw AgentError.input("from and to are the same token (\(Coin.ticker(r.dep.chainId)) and WAETH are 1:1)") }
        guard let amountIn = U256(units: amt, decimals: from.decimals), !amountIn.isZero else {
            throw AgentError.input("amount \"\(amt)\" is not a positive \(from.symbol) amount (at most \(from.decimals) decimals)")
        }
        // Direct, or one hop through the native coin (WAETH) or a seed token of the deployment.
        let hubs = [r.dep.waeth] + r.dep.seed.values.sorted()
        let paths = [[from.address, to.address]] + hubs
            .filter { !same($0, from.address) && !same($0, to.address) }
            .map { [from.address, $0, to.address] }
        let routes = paths.compactMap { try? route($0, amountIn: amountIn, reader: r) }.sorted { $1.out < $0.out }
        guard let best = routes.first, !best.out.isZero else {
            throw AgentError.io("no pool route from \(from.symbol) to \(to.symbol) (direct, or one hop through \(Coin.ticker(r.dep.chainId)) or a main DEX token)")
        }
        let bps = UInt64((slippage * 100).rounded())
        let minOut = (best.out.multiplied(by: 10_000 - bps) ?? best.out).divided(by: 10_000).quotient
        let outReal = real(best.out, to.decimals), inReal = real(amountIn, from.decimals)
        let impact = max(0, 1 - outReal / best.idealOut) * 100
        let hops = best.path.count - 1
        let symbols = try best.path.enumerated().map { i, p -> String in
            i == 0 ? from.symbol : i == hops ? to.symbol : try r.token(p).symbol
        }
        return ["from": describe(from, chainId: r.dep.chainId), "to": describe(to, chainId: r.dep.chainId), "amount_in": amountIn.units(from.decimals),
                "expected_out": best.out.units(to.decimals), "minimum_received": minOut.units(to.decimals),
                "slippage_percent": number(slippage), "route": symbols, "route_addresses": best.path,
                "price_impact_percent": number((impact * 10_000).rounded() / 10_000),
                "lp_fee_percent": number(((1 - pow(0.997, Double(hops))) * 100 * 10_000).rounded() / 10_000),
                "execution_price": "\(number(outReal / inReal)) \(to.symbol) per \(from.symbol)",
                "routes_checked": try routes.map { ["route": $0.path.count == 2 ? "direct" : "via " + (try r.token($0.path[1]).symbol),
                                                    "expected_out": $0.out.units(to.decimals)] },
                "chain_id": r.dep.chainId,
                "note": "Estimate from the pools' current reserves (Router.getAmountsOut); the real result can differ. No transaction was made, and agents cannot swap yet."]
    }

    /// Output of `path` from the Router, plus the no-impact output from the reserves.
    static func route(_ path: [String], amountIn: U256, reader r: Reader) throws -> Route {
        var ideal = real(amountIn, try decimals(path[0], r))
        for i in 0..<(path.count - 1) {
            guard let pair = try r.pair(path[i], path[i + 1]) else { throw AgentError.io("no pool") }
            let (rIn, rOut) = try r.reserves(pair, tokenIn: path[i])
            let xIn = real(rIn, try decimals(path[i], r)), xOut = real(rOut, try decimals(path[i + 1], r))
            guard xIn > 0, xOut > 0 else { throw AgentError.io("empty pool") }
            ideal = ideal * (xOut / xIn) * 0.997
        }
        let amounts = try ABI.uintArray(read(r.dep.router, ABI.amountsOut(Sel.getAmountsOut, amount: amountIn, path: path)))
        guard let out = amounts.last, amounts.count == path.count else { throw AgentError.io("bad getAmountsOut answer") }
        return Route(path: path, out: out, idealOut: ideal)
    }

    private static func decimals(_ address: String, _ r: Reader) throws -> Int { try r.token(address).decimals }
}
