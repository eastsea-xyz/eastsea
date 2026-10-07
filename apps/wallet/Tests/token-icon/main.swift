// Checks which icon a token gets (the token-icon security rules, adopted
// 2026-10-04): official tokens — the shipped trust list and the native coin —
// get their bundled art, and NOTHING else ever does; every other token gets a
// deterministic generated glyph that no network answer influences. No URL, no
// token-supplied metadata, no look-alike promotion:
//   swiftc -o ./tmp/token-icon-check apps/wallet/Sources/Brand.swift apps/wallet/Sources/TokenSend.swift apps/wallet/Sources/TokenAssets.swift apps/wallet/Sources/EarningsModel.swift apps/wallet/Sources/TokenGuard.swift apps/wallet/Sources/TokenIconSpec.swift apps/wallet/Tests/token-icon/main.swift && ./tmp/token-icon-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

let chain: UInt64 = 7780
let waeth = "0xa2521982a17474cb2f8741c85de653b5282d72b0"   // on the shipped list
let neb = "0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416"     // on the shipped list
let usdx = "0x00000000000000000000000000000000000000c1"    // not on any list
let looky = "0x00000000000000000000000000000000000000d4"   // not on any list

// The official list exactly as WalletModel.officialSymbols builds it.
let official: [(symbol: String, name: String)] =
    [(Brand.coinTicker(chainId: chain), Brand.coinName(chainId: chain))] +
    KnownTokens.tokens[chain]!.values.sorted { $0.symbol < $1.symbol }.map { ($0.symbol, $0.name) }

func spec(_ address: String?, _ symbol: String, chain: UInt64 = chain) -> TokenIconSpec {
    TokenIconSpec.of(chainId: chain, address: address, symbol: symbol)
}

// ---- 1. the native coin ----
check(spec(nil, Brand.coinTicker).kind == .nativeCoin(ticker: "DBLN"), "no address is the native coin")
check(spec("", Brand.coinTicker).kind == .nativeCoin(ticker: "DBLN"), "an empty address is the native coin")
check(spec(nil, "Whatever A Node Claims").kind == .nativeCoin(ticker: "DBLN"), "the coin ignores any symbol claim")

// ---- 2. official tokens are recognized by address only ----
check(spec(waeth, "WAETH").kind == .official(symbol: "WAETH"), "the WAETH address gets WAETH art")
check(spec(waeth.uppercased(), "WAETH").kind == .official(symbol: "WAETH"), "uppercase address still official")
check(spec("0xA2521982A17474Cb2F8741C85De653B5282d72B0", "WAETH").kind == .official(symbol: "WAETH"),
      "mixed-case address still official")
check(spec(neb, "NEB").kind == .official(symbol: "NEB"), "the NEB address gets NEB art")
// The art follows the shipped list's symbol, never the node's claim: a scanner
// that read a mangled symbol for a listed address must not mangle the icon.
check(spec(waeth, "waeth!").kind == .official(symbol: "WAETH"), "the list's symbol decides, not the claim")

// ---- 3. everything else is a generated, unverified glyph ----
let unknown = spec(usdx, "USDX")
if case .official = unknown.kind { check(false, "an unknown address is never official") } else {}
if case .nativeCoin = unknown.kind { check(false, "an unknown address is not the native coin") } else {}
check(!unknown.verified, "an unknown address is unverified")
if case .generated(let letter, let seed) = unknown.kind {
    check(letter == "U", "the glyph letter is the symbol's first letter, uppercased")
    check(seed == TokenIconSpec.seed(of: usdx), "the glyph seed is the address hash")
} else { check(false, "an unknown address gets a generated glyph") }
check(spec(usdx, "aBc").kind == .generated(letter: "A", seed: TokenIconSpec.seed(of: usdx)),
      "a mixed-case symbol yields an uppercased single letter")
check(spec(usdx, "").kind == .generated(letter: "?", seed: TokenIconSpec.seed(of: usdx)),
      "an empty symbol yields \"?\"")
check(spec(waeth, "WAETH", chain: 7777).verified == false, "another chain does not know the list")

// ---- 4. a look-alike symbol never gets official art ----
check(SendSafety.looksLikeOfficial(symbol: "VVAETH", name: "Test AETH Cash", official: official),
      "precondition: VVAETH does look like an official symbol")
let mimic = spec(looky, "VVAETH")
check(!mimic.verified, "a look-alike on an unknown address stays unverified")
if case .official = mimic.kind { check(false, "a look-alike must not get official art") } else {}
check(mimic.kind == .generated(letter: "V", seed: TokenIconSpec.seed(of: looky)),
      "a look-alike gets the generated glyph of its own address")

// ---- 5. determinism: same address, same glyph ----
check(spec(usdx, "USDX") == spec(usdx, "USDX"), "the same address and symbol give the same spec")
check(spec(usdx, "USDX") == spec(usdx.uppercased(), "USDX"), "address case does not change the spec")
check(TokenIconSpec.seed(of: usdx) == TokenIconSpec.seed(of: usdx.uppercased()), "the seed ignores address case")

// ---- 6. the colour comes from the address hash only ----
check(TokenIconSpec.seed(of: usdx) != TokenIconSpec.seed(of: looky), "two addresses, two seeds")
let hueA = TokenIconSpec.hue(seed: TokenIconSpec.seed(of: usdx))
let hueB = TokenIconSpec.hue(seed: TokenIconSpec.seed(of: looky))
check((0...1).contains(hueA) && (0...1).contains(hueB), "hues stay in 0...1")
check(hueA == TokenIconSpec.hue(seed: TokenIconSpec.seed(of: usdx)), "the same seed gives the same hue")
check(abs(hueA - hueB) > 0.02, "these two addresses give visibly different hues")
let rgbA = TokenIconSpec.rgb(seed: TokenIconSpec.seed(of: usdx))
check((0...1).contains(rgbA.0) && (0...1).contains(rgbA.1) && (0...1).contains(rgbA.2), "rgb channels stay in 0...1")
check(rgbA == TokenIconSpec.rgb(seed: TokenIconSpec.seed(of: usdx)), "the same seed gives the same colour")

// ---- 7. accessibility: verification is said out loud ----
check(spec(waeth, "WAETH").accessibilityLabel == "WAETH, verified", "official tokens say their name and verified")
check(spec(nil, "AETH").accessibilityLabel == "DBLN, verified", "the legacy testnet coin is labeled DBLN and verified, whatever symbol is claimed")
check(spec(nil, "AETH", chain: 7777).accessibilityLabel == "DBLN, verified", "a new-genesis coin says DBLN")
check(spec(usdx, "USDX").accessibilityLabel == "Unverified token", "unknown tokens say unverified")

print("token-icon OK")
