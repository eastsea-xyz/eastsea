// Checks the token-safety guard (audits R2-2/R2-5, ported from the extension's
// lib/knownTokens.js + lib/sendIntent.js) without an app or a node (mirrors the
// extension's test/known-tokens.test.mjs and test/send-intent.test.mjs):
//   swiftc -o ./tmp/token-guard-check apps/wallet/Sources/Brand.swift apps/wallet/Sources/TokenSend.swift apps/wallet/Sources/TokenAssets.swift apps/wallet/Sources/EarningsModel.swift apps/wallet/Sources/TokenGuard.swift apps/wallet/Tests/token-guard/main.swift && ./tmp/token-guard-check
// The drift checks read the extension's sources and the bundled token-sources.json
// from the repo root (scripts/test-swift-pure.sh runs the binary there).
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }
func fails(with expected: TokenGuardError, _ m: String, _ body: () throws -> String) {
    do {
        _ = try body()
        print("FAIL", m, "(no error)"); exit(1)
    } catch let e as TokenGuardError {
        check(e == expected, "\(m) (wanted \(expected), got \(e))")
    } catch {
        print("FAIL", m, "(unexpected error \(error))"); exit(1)
    }
}
func read(_ path: String, _ what: String) -> String {
    guard let s = try? String(contentsOfFile: path, encoding: .utf8), !s.isEmpty else {
        print("FAIL cannot read \(what) at \(path)"); exit(1)
    }
    return s
}

let waeth = "0xa2521982a17474cb2f8741c85de653b5282d72b0"   // on the shipped list, 18 decimals
let usdx = "0x00000000000000000000000000000000000000c1"    // not on any list
let neb = "0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416"     // on the shipped list
let recipient = "0x00000000000000000000000000000000000000be"
let ten18 = String(repeating: "0", count: 18)
func info(_ address: String, _ symbol: String, _ name: String, _ decimals: Int) -> TokenInfo {
    TokenInfo(address: address, symbol: symbol, name: name, decimals: decimals, origin: nil)
}
let honest = info(usdx, "USDX", "Test Dollar", 6)

// ---- 1. the shipped table's shape (a typo must not quietly demote or promote) ----
for (chain, table) in KnownTokens.tokens {
    for (addr, t) in table {
        check(addr == addr.lowercased() && addr.hasPrefix("0x") && addr.count == 42
              && addr.dropFirst(2).allSatisfy(\.isHexDigit), "\(chain):\(addr) is a lowercase 0x address")
        check((1...16).contains(t.symbol.count), "\(chain):\(addr) symbol length")
        check((1...64).contains(t.name.count), "\(chain):\(addr) name length")
        check((0...77).contains(t.decimals), "\(chain):\(addr) decimals")
    }
}
check(KnownTokens.native[7780]?.decimals == 18, "the native entry is 18 decimals")
check(KnownTokens.native[7780]?.symbol == "AETH" && KnownTokens.native[7780]?.name == "Test AETH",
      "the legacy 7780 testnet's own coin record is unchanged (the extension's list matches it)")
check(Brand.coinTicker(chainId: 7780) == "DBLN" && Brand.coinName(chainId: 7780) == "Doubloon",
      "the brand labels the legacy testnet DBLN")
check(Brand.coinTicker(chainId: 0) == "DBLN" && Brand.coinName(chainId: 0) == "Doubloon",
      "a new-genesis chain is labeled DBLN/Doubloon")
// The record keeps the chain's own symbol (the extension's list matches it);
// what the wallet's screens call the coin is Brand's label, DBLN everywhere.
check(KnownTokens.native[7780]?.symbol == "AETH" && Brand.coinTicker(chainId: 7780) == "DBLN",
      "the native record stays the chain's own; the display label is DBLN")

// ---- 2. lookups ignore case and stay per chain ----
let waethEntry = KnownTokens.knownToken(chainId: 7780, address: waeth)
check(waethEntry?.decimals == 18 && waethEntry?.symbol == "WAETH", "WAETH is on the list")
check(KnownTokens.knownToken(chainId: 7780, address: waeth.uppercased()) == waethEntry, "uppercase still finds it")
check(KnownTokens.knownToken(chainId: 7780, address: "0xA2521982A17474Cb2F8741C85De653B5282d72B0") == waethEntry, "mixed case still finds it")
check(KnownTokens.knownToken(chainId: 7777, address: waeth) == nil, "another chain does not know it")
check(KnownTokens.knownToken(chainId: 7780, address: "0x00000000000000000000000000000000000000ff") == nil, "unknown address")
check(KnownTokens.knownToken(chainId: 7780, address: "") == nil, "empty address")
check(KnownTokens.knownToken(chainId: 7780, address: "0x1234") == nil, "short address")

// ---- 3. drift: the Swift table must equal the extension's knownTokens.js ----
// The extension's list is the source of truth; this parses the JS at runtime and
// fails when either side adds, removes or edits an entry without the other.
let js = read("apps/extension/src/lib/knownTokens.js", "the extension's knownTokens.js")
func jsMatches(_ pattern: String, in s: String) -> [NSTextCheckingResult] {
    (try? NSRegularExpression(pattern: pattern))?.matches(in: s, range: NSRange(s.startIndex..<s.endIndex, in: s)) ?? []
}
guard let knownAt = js.range(of: "export const KNOWN_TOKENS"),
      let fnAt = js.range(of: "export function", range: knownAt.upperBound..<js.endIndex) else {
    print("FAIL knownTokens.js does not have the expected KNOWN_TOKENS section"); exit(1)
}
let knownSection = String(js[knownAt.lowerBound..<fnAt.lowerBound])
let chains = jsMatches(#"(?m)^\s*(\d+): Object\.freeze\(\{\s*$"#, in: knownSection)
let entries = jsMatches(#"'(0x[0-9a-f]+)': Object\.freeze\(\{ symbol: '([^']*)', name: '([^']*)', decimals: (\d+) \}\)"#, in: knownSection)
check(!chains.isEmpty && !entries.isEmpty, "knownTokens.js parsed (chains: \(chains.count), entries: \(entries.count))")
var jsTable: [UInt64: [String: KnownToken]] = [:]
for e in entries {
    let owner = chains.last { $0.range.location < e.range.location }
    guard let owner, e.range.location > owner.range.location,
          let chain = UInt64((knownSection as NSString).substring(with: owner.range(at: 1))) else {
        print("FAIL knownTokens.js entry outside a chain block"); exit(1)
    }
    let ns = knownSection as NSString
    jsTable[chain, default: [:]][ns.substring(with: e.range(at: 1))] =
        KnownToken(symbol: ns.substring(with: e.range(at: 2)), name: ns.substring(with: e.range(at: 3)),
                   decimals: Int(ns.substring(with: e.range(at: 4)))!)
}
check(jsTable.count == KnownTokens.tokens.count, "same chain count as the extension (js \(jsTable.keys.sorted()), swift \(KnownTokens.tokens.keys.sorted())")
for (chain, table) in jsTable {
    let swift = KnownTokens.tokens[chain] ?? [:]
    check(table.count == swift.count, "chain \(chain): same entry count (js \(table.count), swift \(swift.count))")
    for (addr, t) in table { check(swift[addr] == t, "knownTokens.js entry \(chain) \(addr) \(t) is missing or different in the Swift table") }
    for (addr, t) in swift where table[addr] == nil { check(false, "Swift-only entry \(chain) \(addr) \(t) is not in knownTokens.js") }
}
// The native entry, with the extension's Brand resolved into Swift's.
let brandJs = read("apps/extension/src/lib/brand.js", "the extension's brand.js")
func brandValue(_ name: String) -> String {
    let ms = jsMatches("\(name): '([^']+)'", in: brandJs)
    guard let m = ms.first else { print("FAIL brand.js has no \(name)"); exit(1) }
    return (brandJs as NSString).substring(with: m.range(at: 1))
}
check(brandValue("coinTicker") == Brand.coinTicker, "the extension's coinTicker matches the app's Brand")
check(brandValue("coinName") == Brand.coinName, "the extension's coinName matches the app's Brand")
guard let nativeAt = js.range(of: "export const NATIVE_COINS"),
      let knownConstAt = js.range(of: "export const KNOWN_TOKENS", range: nativeAt.upperBound..<js.endIndex) else {
    print("FAIL knownTokens.js does not have the expected NATIVE_COINS section"); exit(1)
}
let nativeSection = String(js[nativeAt.lowerBound..<knownConstAt.lowerBound])
let nativeMatches = jsMatches(#"(\d+): Object\.freeze\(\{ symbol: '([^']+)', name: '([^']+)', decimals: (\d+) \}\)"#, in: nativeSection)
check(nativeMatches.count == KnownTokens.native.count, "same native chain count as the extension")
for m in nativeMatches {
    let ns = nativeSection as NSString
    let chain = UInt64(ns.substring(with: m.range(at: 1)))!
    let decimals = Int(ns.substring(with: m.range(at: 4)))!
    check(KnownTokens.native[chain] == KnownToken(symbol: ns.substring(with: m.range(at: 2)), name: ns.substring(with: m.range(at: 3)), decimals: decimals),
          "native entry for chain \(chain) matches the extension's")
    // The legacy 7780 testnet is the one exception: the wallet's screens say
    // DBLN (no old brand on screen), the chain record and the extension keep AETH.
    check(KnownTokens.native[chain]?.symbol == Brand.coinTicker(chainId: chain) || chain == Brand.legacyTestnetChainId,
          "the extension's native label for chain \(chain) follows the per-chain brand")
}

// ---- 4. every address the wallet itself ships as a source token is on the list ----
// token-sources.json is bundled with the app; a listing the UI treats as official
// must never fall back to unverified units.
guard let sourcesData = try? Data(contentsOf: URL(fileURLWithPath: "apps/wallet/Resources/token-sources.json")) else {
    print("FAIL cannot read apps/wallet/Resources/token-sources.json"); exit(1)
}
struct SourcesFile: Decodable {
    struct Chain: Decodable { var waeth: String?; var seed: [String]? }
    var chains: [String: Chain]
}
guard let sources = try? JSONDecoder().decode(SourcesFile.self, from: sourcesData) else {
    print("FAIL token-sources.json did not decode"); exit(1)
}
for (chain, s) in sources.chains {
    for address in [s.waeth].compactMap({ $0 }) + (s.seed ?? []) {
        check(KnownTokens.knownToken(chainId: UInt64(chain) ?? 0, address: address) != nil,
              "\(chain) \(address) is deployed but not in the Swift allowlist")
    }
}

// ---- 5. the denomination a display must use (extension tokenPin.denominationOf) ----
let lyingWaeth = info(waeth, "WAETH", "Wrapped AETH", 9)
var d = TokenDenomination.of(chainId: 7780, address: waeth, claimed: lyingWaeth)
check(d.decimals == 18 && d.symbol == "WAETH" && d.trusted && !d.unverifiedUnits && d.nodeDisagrees,
      "a lying node cannot move a listed token's units; it only reports a disagreement")
d = TokenDenomination.of(chainId: 7780, address: waeth, claimed: info(waeth, "WAETH", "Wrapped AETH", 18))
check(d.trusted && !d.nodeDisagrees, "an agreeing node is quiet")
d = TokenDenomination.of(chainId: 7780, address: usdx, claimed: honest)
check(d.decimals == 6 && d.symbol == "USDX" && !d.trusted && d.unverifiedUnits && !d.nodeDisagrees,
      "an unlisted token's units are unverified, but still displayable")
d = TokenDenomination.of(chainId: 7780, address: usdx, claimed: nil)
check(d.decimals == nil && !d.trusted && d.unconfirmed, "no claim at all is unconfirmed")
d = TokenDenomination.of(chainId: 7777, address: waeth, claimed: honest)
check(!d.trusted && d.unverifiedUnits, "a listed address on another chain is unverified")

// ---- 6. building the intent: parsed once, under the decimals shown ----
let goodToken = SendIntent.Token(address: usdx, decimals: 6, trusted: false, acknowledged: false)
var intent = try SendIntent.build(recipient: " 0x00000000000000000000000000000000000000BE \n", amountText: "2", token: goodToken)
check(intent.baseUnits == "2000000", "parsed under the shown decimals")
check(intent.recipient == "0x00000000000000000000000000000000000000BE", "recipient trimmed, case kept")
check(intent.token.address == usdx, "token lowercased")
intent = try SendIntent.build(recipient: recipient, amountText: "1.5", token: SendIntent.Token(address: usdx, decimals: 18, trusted: false, acknowledged: false))
check(intent.baseUnits == "1500000000000000000", "1.5 @18")
fails(with: .recipientNotAddress, "bad recipient") { try SendIntent.check(try SendIntent.build(recipient: "0xnope", amountText: "1", token: goodToken), current: honest, known: nil) }
do { _ = try SendIntent.build(recipient: "0xnope", amountText: "1", token: goodToken); check(false, "bad recipient rejected") } catch { check((error as? TokenGuardError) == .recipientNotAddress, "bad recipient rejected with the right error") }
do { _ = try SendIntent.build(recipient: recipient, amountText: "1", token: SendIntent.Token(address: "0xNOPE", decimals: 6, trusted: false, acknowledged: false)); check(false, "bad token rejected") } catch { check((error as? TokenGuardError) == .tokenNotAddress, "bad token rejected with the right error") }
for bad in [78, -1] {
    do { _ = try SendIntent.build(recipient: recipient, amountText: "1", token: SendIntent.Token(address: usdx, decimals: bad, trusted: false, acknowledged: false)); check(false, "decimals \(bad) rejected") } catch { check((error as? TokenGuardError) == .unusableDecimals, "decimals \(bad) rejected with the right error") }
}
for bad in ["abc", "1.2.3", "", "1.0000001"] {
    do { _ = try SendIntent.build(recipient: recipient, amountText: bad, token: goodToken); check(false, "amount '\(bad)' rejected") } catch { check((error as? TokenGuardError) == .unusableAmount, "amount '\(bad)' rejected with the right error") }
}

// ---- 7. R2-2: two identical malicious answers still cannot set the units silently ----
// (In the app the "pin" is the scanner's claimed metadata; both-node agreement is
// continuity, not trust — the claim only ever yields unverified units.)
let claimed9 = info(usdx, "USDX", "Test Dollar", 9)
d = TokenDenomination.of(chainId: 7780, address: usdx, claimed: claimed9)
check(d.unverifiedUnits && d.decimals == 9 && !d.trusted, "the malicious agreement is displayed but never trusted")
var ghost = try SendIntent.build(recipient: recipient, amountText: "1", token: SendIntent.Token(address: usdx, decimals: 9, trusted: false, acknowledged: false))
check(ghost.baseUnits == "1000000000", "1 × 10^9 units, shown and confirmed as units")
fails(with: .needsAcknowledgement, "no acknowledgement, no signature") { try SendIntent.check(ghost, current: claimed9, known: nil) }
var signed = try SendIntent.check(ghost.with(acknowledged: true), current: claimed9, known: nil)
check(signed == ERC20.transferCalldata(to: recipient, amount: "1000000000"), "signs exactly the confirmed 1,000,000,000 units")

// ---- 8. R2-2: a lying node cannot move a listed token's units ----
var waethIntent = try SendIntent.build(recipient: recipient, amountText: "1", token: SendIntent.Token(address: waeth, decimals: 18, trusted: true, acknowledged: false))
check(waethIntent.baseUnits == "1" + ten18, "1 @18 from the list")
check(waethIntent.token.trusted, "the intent carries trust")
signed = try SendIntent.check(waethIntent, current: lyingWaeth, known: waethEntry)   // trusted: the claim is ignored
check(signed == ERC20.transferCalldata(to: recipient, amount: "1" + ten18), "signs 10^18 without an acknowledgement flow")
signed = try SendIntent.check(waethIntent, current: nil, known: waethEntry)
check(signed == ERC20.transferCalldata(to: recipient, amount: "1" + ten18), "a listed token signs even without a current claim")
waethIntent = try SendIntent.build(recipient: recipient, amountText: "1", token: SendIntent.Token(address: waeth, decimals: 9, trusted: true, acknowledged: false))  // built under the node's lying decimals
fails(with: .listDecimals, "an intent carrying lying decimals is refused") { try SendIntent.check(waethIntent, current: lyingWaeth, known: waethEntry) }

// ---- 9. R2-5: a confirmation left open while the state moved is refused ----
var frozen = try SendIntent.build(recipient: recipient, amountText: "2", token: SendIntent.Token(address: usdx, decimals: 6, trusted: false, acknowledged: true))
fails(with: .staleIntent, "decimals moved under the open confirmation") { try SendIntent.check(frozen, current: claimed9, known: nil) }
fails(with: .unconfirmed, "the token vanished under the open confirmation") { try SendIntent.check(frozen, current: nil, known: nil) }
signed = try SendIntent.check(frozen, current: honest, known: nil)
check(signed == ERC20.transferCalldata(to: recipient, amount: "2000000"), "the current state still signs, with its own units")

// ---- 10. the executed side re-validates everything ----
frozen = try SendIntent.build(recipient: recipient, amountText: "1", token: goodToken)
let maxUint256 = "115792089237316195423570985008687907853269984665640564039457584007913129639935"
for bad in ["-1", "1.5", "0x10", "1e9", "", "007x", "1" + String(repeating: "0", count: 78), "115792089237316195423570985008687907853269984665640564039457584007913129639936"] {
    fails(with: .unusableBaseUnits, "baseUnits '\(bad.prefix(12))…' refused") {
        try SendIntent.check(SendIntent(token: frozen.token, recipient: frozen.recipient, baseUnits: bad), current: honest, known: nil)
    }
}
_ = try SendIntent.check(SendIntent(token: frozen.token, recipient: frozen.recipient, baseUnits: maxUint256).with(acknowledged: true), current: honest, known: nil)  // 2^256-1 fits
fails(with: .recipientNotAddress, "a tampered recipient is refused") {
    try SendIntent.check(SendIntent(token: frozen.token, recipient: "0xnope", baseUnits: frozen.baseUnits), current: honest, known: nil)
}

// ---- 11. the confirm screen's grouped unit count ----
check(SendIntent.grouped("0") == "0", "grouped zero")
check(SendIntent.grouped("999") == "999", "grouped under a thousand")
check(SendIntent.grouped("1000000000") == "1,000,000,000", "grouped a billion")
check(SendIntent.grouped("1234") == "1,234", "grouped a thousand and change")

print("OK")
