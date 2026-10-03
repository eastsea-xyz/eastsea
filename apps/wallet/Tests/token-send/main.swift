// Checks the token-send helpers without an app or a node (mirrors the
// extension's test/send.test.mjs):
//   swiftc -o ./tmp/token-send-check apps/wallet/Sources/Brand.swift apps/wallet/Sources/TokenSend.swift apps/wallet/Sources/TokenAssets.swift apps/wallet/Sources/EarningsModel.swift apps/wallet/Tests/token-send/main.swift && ./tmp/token-send-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

// Amount parsing: exact, any decimals, spaces trimmed, junk rejected.
check(TokenAmount.parse("1.5", decimals: 18) == "1500000000000000000", "1.5 @18")
check(TokenAmount.parse("  1.5 \n", decimals: 18) == "1500000000000000000", "spaces trimmed")
check(TokenAmount.parse("1.000000000000000001", decimals: 18) == "1000000000000000001", "18 fraction digits exact")
check(TokenAmount.parse("0.000000000000000001", decimals: 18) == "1", "smallest unit")
check(TokenAmount.parse("1234567", decimals: 0) == "1234567", "0 decimals integer")
check(TokenAmount.parse("1234567.0", decimals: 0) == nil, "fraction rejected at 0 decimals")
check(TokenAmount.parse("1.000000000000000001", decimals: 17) == nil, "too many decimals")
check(TokenAmount.parse("1.0000000000000000010", decimals: 18) == nil, "19 digits")
check(TokenAmount.parse("0", decimals: 18) == "0", "zero")
check(TokenAmount.parse("007", decimals: 2) == "700", "leading zeros")
check(TokenAmount.parse(".5", decimals: 1) == "5", "bare fraction")
check(TokenAmount.parse("1.", decimals: 2) == "100", "trailing dot")
check(TokenAmount.parse("", decimals: 18) == nil, "empty")
check(TokenAmount.parse(".", decimals: 18) == nil, "dot only")
check(TokenAmount.parse("abc", decimals: 18) == nil, "junk")
check(TokenAmount.parse("1.2.3", decimals: 18) == nil, "two dots")
check(TokenAmount.parse("-1", decimals: 18) == nil, "negative")
check(TokenAmount.parse("1,5", decimals: 18) == nil, "comma")
check(TokenAmount.parse("１", decimals: 18) == nil, "non-ascii digit")

// Exact display for the Max button (all digits, unlike TokenUnits.format's 6).
check(TokenAmount.exact("1000000000000000001", decimals: 18) == "1.000000000000000001", "exact 18")
check(TokenAmount.exact("1234567", decimals: 0) == "1234567", "exact 0 decimals")
check(TokenAmount.exact("1500000", decimals: 6) == "1.500000", "exact keeps zeros")
check(TokenAmount.exact("0", decimals: 18) == "0.000000000000000000", "exact zero")

// Balance comparisons on decimal strings.
check(WeiMath.compare("5", "5") == 0, "equal")
check(WeiMath.compare("1000000000000000001", "1000000000000000000") > 0, "18-digit compare")
check(WeiMath.compare("999", "1000") < 0, "length first")

// transfer(address,uint256) calldata: selector + two padded words.
let rcpt = "0x00000000000000000000000000000000000000aA"
check(ERC20.transferCalldata(to: rcpt, amount: "25000000000000000000")
      == "0xa9059cbb" + String(repeating: "0", count: 24) + "00000000000000000000000000000000000000aa"
      + String(repeating: "0", count: 47) + "15af1d78b58c40000", "transfer calldata")
check(ERC20.transferCalldata(to: rcpt, amount: "0")?.count == 10 + 128, "zero amount word")
check(ERC20.transferCalldata(to: rcpt, amount: "1" + String(repeating: "0", count: 77))?.count == 10 + 128, "10^77 still fits uint256")
check(ERC20.transferCalldata(to: rcpt, amount: "2" + String(repeating: "0", count: 77)) == nil, "2*10^77 overflows uint256")
check(ERC20.transferCalldata(to: "0x1234", amount: "1") == nil, "bad address still encodes lowercased") // the FFI validates before sending
check(EVMABI.word(uintDecimal: String(UInt64.max)) == String(repeating: "0", count: 48) + "ffffffffffffffff", "uint64 max")
check(EVMABI.word(uintDecimal: "1" + String(repeating: "0", count: 78)) == nil, "2^256 rejected")

// Address safety: poisoning (first 4 and last 4 hex chars) and first sends.
let hist: Set<String> = ["0x1234567890abcdef1234567890abcdef12345678", "0xaabbccddeeff00112233445566778899aabbccdd"]
var risk = SendSafety.addressRisk("0x1234abcdef777777777777777777777777775678", sentTo: hist)
check(risk.poisoningMatch == "0x1234567890abcdef1234567890abcdef12345678", "poisoning prefix4+suffix4")
check(risk.firstSend, "poisoned address is also a first send")
risk = SendSafety.addressRisk("0x1234567890ABCDEF1234567890abcdef12345678", sentTo: hist)
check(risk.poisoningMatch == nil && !risk.firstSend, "the same address is not a first send")
risk = SendSafety.addressRisk("0x1234567890abcdef1234567890abcdef12345679", sentTo: hist)
check(risk.poisoningMatch == nil && risk.firstSend, "only suffix differs: not poisoning, but new")
risk = SendSafety.addressRisk("0xabc4567890abcdef1234567890abcdef12345678", sentTo: hist)
check(risk.poisoningMatch == nil, "middle differs but prefix/suffix differ: no match")
risk = SendSafety.addressRisk("0x1234", sentTo: hist)
check(risk.poisoningMatch == nil && risk.firstSend, "not an address")
check(SendSafety.isValidAddress("0x1234567890AbCdEf1234567890aBcDeF12345678"), "mixed case is an address")
check(!SendSafety.isValidAddress("1234567890abcdef1234567890abcdef12345678"), "needs 0x")
check(!SendSafety.isValidAddress("0x1234567890abcdef1234567890abcdef1234567g"), "hex only")

// Look-alike symbols (NFKD-folded, edit distance 1, the official ticker family),
// derived from the brand constants so a rename cannot leave these testing a dead name.
let official: [(symbol: String, name: String)] = [(Brand.coinTicker, Brand.coinName), ("USDT", "Tether Dollar")]
let tick = Brand.coinTicker
check(SendSafety.looksLikeOfficial(symbol: tick, name: "x", official: official), "equal symbol")
check(SendSafety.looksLikeOfficial(symbol: String(tick.dropLast()), name: "", official: official), "one edit away")
check(SendSafety.looksLikeOfficial(symbol: tick + "R", name: "", official: official), "one insert away")
check(SendSafety.looksLikeOfficial(symbol: "W" + tick, name: "", official: official), "contains the ticker")
check(SendSafety.looksLikeOfficial(symbol: String(tick.unicodeScalars.map { Character(Unicode.Scalar($0.value + 0xFEE0)!) }), name: "", official: official), "fullwidth letters fold to the ticker")
check(SendSafety.looksLikeOfficial(symbol: "NEB", name: tick + "s", official: official), "name resembles")
check(SendSafety.looksLikeOfficial(symbol: "NEB", name: Brand.coinName, official: official), "same name as the coin")
check(!SendSafety.looksLikeOfficial(symbol: "NEB", name: "Nebula", official: official), "unrelated")
check(!SendSafety.looksLikeOfficial(symbol: "NEBD", name: "Nebula", official: official), "one edit but short official only")
check(SendSafety.looksLikeOfficial(symbol: "usdt", name: "", official: official), "case-insensitive")
check(SendSafety.normalized("Æth​er ") == "aether", "zero-width and spaces stripped")

// Revert reasons from node error strings.
let reason = SendSafety.revertReason("execution reverted: 0x08c379a0"
    + String(repeating: "0", count: 62) + "20" + String(repeating: "0", count: 63) + "e"
    + "546f6f206c6974746c6520676173" + String(repeating: "0", count: 36))
check(reason == "\"Too little gas\"", "Error(string) decoded: \(reason)")
check(SendSafety.revertReason("execution reverted: 0x") == "execution reverted: 0x", "no reason kept")
check(SendSafety.revertReason("node did not answer") == "node did not answer", "passthrough")

// Labels: never a symbol alone.
check(TokenLabel.short("0x8a9b000000000000000000000000000000000f41c") == "0x8a9b…f41c", "short address")
check(TokenLabel.row(TokenInfo(address: "0x8a9b000000000000000000000000000000000f41c", symbol: "NEB", name: "Nebula", decimals: 18, origin: nil)) == "NEB · 0x8a9b…f41c", "row label")

// The display policy: own actions and official tokens are main-listed, the
// rest wait in the collapsed Unverified section; user choices win.
let cat = TokenDisplayPolicy(touched: ["0xtouched"], official: ["0xofficial"], hidden: ["0xhidden"], shown: ["0xshown"])
check(cat.isMain("0xtouched") && cat.isMain("0xofficial") && cat.isMain("0xshown"), "main-listed")
check(!cat.isMain("0xhidden") && !cat.isMain("0xrandom"), "not main-listed")
check(cat.isMain("0xTOUCHED"), "case-insensitive")
let holdings = [
    TokenHolding(token: TokenInfo(address: "0xofficial", symbol: "A", name: "", decimals: 18, origin: "seed"), balance: "1"),
    TokenHolding(token: TokenInfo(address: "0xrandom", symbol: "B", name: "", decimals: 18, origin: nil), balance: "2"),
    TokenHolding(token: TokenInfo(address: "0xtouched", symbol: "C", name: "", decimals: 18, origin: "launchpad"), balance: "3"),
]
let (main, unverified) = cat.split(holdings)
check(main.map(\.token.symbol) == ["A", "C"], "main \(main.map(\.token.symbol))")
check(unverified.map(\.token.symbol) == ["B"], "unverified")

// Dry-run: nothing answers on port 1, so plain AETH falls back to the FFI-style
// call and token sends come back honestly unchecked.
let semaphore = DispatchSemaphore(value: 0)
Task {
    check(await SendDryRun.check(from: "0xabc", to: "0xdef", valueWei: "0", data: "0x", port: 1,
                                 plainCall: { to, data in data == "0x" && to == "0xdef" ? "0x" : "" }) == .ok,
          "plain transfer fallback ok")
    let refused = await SendDryRun.check(from: "0xabc", to: "0xdef", valueWei: "0", data: "0x", port: 1,
        plainCall: { _, _ in throw DryRunError(reason: "execution reverted: 0x08c379a0" + String(repeating: "0", count: 120)) })
    if case .reverted(let m) = refused { check(m.hasPrefix("execution reverted"), "revert message kept: \(m)") }
    else { check(false, "expected .reverted, got \(refused)") }
    check(await SendDryRun.check(from: "0xabc", to: "0xdef", valueWei: "0", data: "0xa9059cbb", port: 1) == .unchecked,
          "no node, no fallback for tokens")
    semaphore.signal()
}
semaphore.wait()
print("OK")
