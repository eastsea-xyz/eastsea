// Checks the token scan against a fake chain (no app, no node):
//   swiftc -o /tmp/assets-check apps/wallet/Sources/EarningsModel.swift apps/wallet/Sources/TokenAssets.swift apps/wallet/Tests/assets/main.swift && /tmp/assets-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

func word(_ v: UInt64) -> String { EVMABI.word(uint: v) }
func addrWord(_ a: String) -> String { EVMABI.word(address: a) }
func str(_ s: String) -> String {
    let b = Array(s.utf8).map { String(format: "%02x", $0) }.joined()
    let padded = b + String(repeating: "0", count: (64 - b.count % 64) % 64)
    return "0x" + word(32) + word(UInt64(s.utf8.count)) + padded
}

let owner = "0x00000000000000000000000000000000000000aa"
let factory = "0x00000000000000000000000000000000000000f1", pairs = "0x00000000000000000000000000000000000000f2"
let launch = "0x00000000000000000000000000000000000000f3", waeth = "0x00000000000000000000000000000000000000e1"
let tA = "0x00000000000000000000000000000000000000a1", tB = "0x00000000000000000000000000000000000000b1"
let tC = "0x00000000000000000000000000000000000000c1", pair = "0x00000000000000000000000000000000000000d1"
let notToken = "0x00000000000000000000000000000000000000ee"
var calls = 0
var balances: [String: String] = [tA: "0x" + String(repeating: "0", count: 47) + "15af1d78b58c40000", tB: "0x" + word(0), tC: "0x" + word(1_500_000), waeth: "0x" + word(0)]
let meta: [String: (String, String, UInt64)] = [tA: ("NEB", "Nebula", 18), tB: ("ORB", "Orb", 18), tC: ("USDX", "Test Dollar", 6), waeth: ("WAETH", "Wrapped AETH", 18)]

func read(_ to: String, _ data: String) throws -> String {
    calls += 1
    let sel = String(data.dropFirst(2).prefix(8)), arg = String(data.dropFirst(10))
    switch (to, sel) {
    case (factory, TokenScanner.Sel.allTokensLength): return "0x" + word(2)
    case (factory, TokenScanner.Sel.allTokens): return "0x" + addrWord(arg.hasSuffix("0") ? tA : notToken)
    case (pairs, TokenScanner.Sel.allPairsLength): return "0x" + word(1)
    case (pairs, TokenScanner.Sel.allPairs): return "0x" + addrWord(pair)
    case (pair, TokenScanner.Sel.token0): return "0x" + addrWord(waeth)
    case (pair, TokenScanner.Sel.token1): return "0x" + addrWord(tB)
    case (launch, TokenScanner.Sel.tokenCount): return "0x" + word(1)
    case (launch, TokenScanner.Sel.tokens): return "0x" + addrWord(tC)
    default: break
    }
    guard let m = meta[to] else { throw TokenScanError.badAnswer }
    switch sel {
    case TokenScanner.Sel.decimals: return "0x" + word(m.2)
    case TokenScanner.Sel.symbol: return str(m.0)
    case TokenScanner.Sel.name: return str(m.1)
    case TokenScanner.Sel.balanceOf:
        check(arg == addrWord(owner), "balanceOf owner")
        return balances[to]!
    default: throw TokenScanError.badAnswer
    }
}

let src = TokenSources(network: "t", waeth: waeth, tokenFactory: factory, pairFactory: pairs, launchpad: launch, seed: [])
let (cat, held) = try TokenScanner.scan(owner: owner, sources: src, catalog: TokenCatalog(), read: read)
check(cat.tokens.count == 4, "catalog \(cat.tokens.keys.sorted())")
check(cat.rejected == [notToken], "rejected \(cat.rejected)")
check(cat.factoryRead == 2 && cat.pairsRead == 1 && cat.launchesRead == 1, "counts")
check(held.map(\.token.symbol) == ["NEB", "USDX"], "held \(held.map(\.token.symbol))")
check(held[0].balance == "25000000000000000000" && held[0].amount == "25", "NEB \(held[0].balance) \(held[0].amount)")
check(held[1].amount == "1.5", "USDX \(held[1].amount)")
check(held[1].token.name == "Test Dollar" && held[1].token.decimals == 6, "meta")
// Second scan: only balances (nothing new to enumerate or describe).
calls = 0
balances[tB] = "0x" + word(1)
let (cat2, held2) = try TokenScanner.scan(owner: owner, sources: src, catalog: cat, read: read)
check(cat2 == cat, "catalog unchanged")
check(calls == 3 + 4, "incremental calls \(calls)")
check(held2.map(\.token.symbol) == ["NEB", "ORB", "USDX"], "held2")
check(held2[1].amount == "<0.000001", "dust \(held2[1].amount)")
check(TokenUnits.format("0", decimals: 18) == "0", "zero")
check(TokenUnits.format("1234567", decimals: 0) == "1234567", "no decimals")
check(TokenSources.parse(Data(#"{"chains":{"7780":{"seed":["0x1"]}}}"#.utf8), chainId: 7780)?.seed == ["0x1"], "parse")
check(TokenSources.parse(Data(#"{"chains":{}}"#.utf8), chainId: 7780) == nil, "parse missing")
print("OK")
