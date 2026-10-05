// Checks the `window.eastsea.verify` surface's pure half (VerifyBridge.swift):
// what counts as a well-formed ask, the shape every answer resolves with, and
// which origins may ask at all — without a view or a node. The WebKit half
// (message routing, the FFI calls) lives in BrowserController.
//   swiftc -o ./tmp/browser-verify-check apps/wallet/Sources/Brand.swift apps/wallet/Sources/BrowserOriginPolicy.swift apps/wallet/Sources/BrowserPolicy.swift apps/wallet/Sources/VerifyBridge.swift apps/wallet/Tests/browser-verify/main.swift && ./tmp/browser-verify-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

extension Result {
    var isSuccess: Bool { if case .success = self { return true }; return false }
}

let ADDR = "0x00000000000000000000000000000000000000aa"
let TX = "0x" + String(repeating: "ab", count: 32)

// ---- parsing an ask ----

// The three questions a page may ask, each with its one argument.
check(VerifyBridge.parse(["what": "block", "param": 6]) == .success(.block(height: 6)), "block height")
check(VerifyBridge.parse(["what": "block", "param": 0]) == .success(.block(height: 0)), "height zero is a height")
check(VerifyBridge.parse(["what": "account", "param": ADDR]) == .success(.account(address: ADDR)), "account address")
check(VerifyBridge.parse(["what": "receipt", "param": TX]) == .success(.receipt(txHash: TX)), "receipt tx hash")
// A checksummed address is as good as a lowercase one.
check(VerifyBridge.parse(["what": "account", "param": "0xABCDEF0000000000000000000000000000000001"]).isSuccess, "uppercase hex address")

// Malformed asks are bridge errors (-32602), never verdicts: the page asked
// nonsense, nothing was checked.
func fails(_ d: [String: Any], _ code: Int = ProviderErrorCode.params) -> ProviderError? {
    if case .failure(let e) = VerifyBridge.parse(d) { return e.code == code ? e : nil }
    return nil
}
check(fails([:]) != nil, "no what")
check(fails(["what": "block"]) != nil, "no param")
check(fails(["what": "block", "param": "6"]) != nil, "height as a string")
check(fails(["what": "block", "param": 6.5]) != nil, "fractional height")
check(fails(["what": "block", "param": -6]) != nil, "negative height")
check(fails(["what": "block", "param": true]) != nil, "a bool is not a height (JS true bridges as NSNumber)")
check(fails(["what": "account", "param": "0x1234"]) != nil, "short address")
check(fails(["what": "account", "param": 42]) != nil, "account not a string")
check(fails(["what": "receipt", "param": "0xabc"]) != nil, "short tx hash")
check(fails(["what": "receipt", "param": ADDR]) != nil, "an address is not a tx hash")
check(fails(["what": "receipt", "param": "0x" + String(repeating: "g", count: 64)]) != nil, "not hex")
// An unknown question is a 4200 refusal, like the provider's unknown method.
check(fails(["what": "chain", "param": 1], ProviderErrorCode.unsupported) != nil, "unknown what is unsupported")

// ---- the answer shape ----

// A certified answer carries its height; a refused one does not pretend to one.
check(VerifyBridge.Verdict.certified(height: 6).asDictionary() as NSDictionary
      == ["verified": true, "height": 6, "reason": ""] as NSDictionary, "certified dictionary")
check(VerifyBridge.Verdict.refused("certificate: expired").asDictionary() as NSDictionary
      == ["verified": false, "reason": "certificate: expired"] as NSDictionary, "refused dictionary has no height")
// The one answer receipts get today: no block commits to a receipt yet.
check(VerifyBridge.Verdict.notCommitted == VerifyBridge.Verdict.refused("not committed"), "not committed is a refusal named for what it is")

// ---- which origins may ask ----

// The pages the app bundles always may — connected or not, they are this app.
check(VerifyBridge.allows(scheme: BrowserOriginPolicy.bundledScheme, connected: false), "bundled explorer")
check(VerifyBridge.allows(scheme: "EASTSEA-PAGE", connected: false), "scheme case does not matter")
// An external page: https and connected, or nothing.
check(VerifyBridge.allows(scheme: "https", connected: true), "connected https site")
check(!VerifyBridge.allows(scheme: "https", connected: false), "unconnected https site may not ask")
check(!VerifyBridge.allows(scheme: "http", connected: true), "no cleartext origins")
check(!VerifyBridge.allows(scheme: "eastsea-page-evil", connected: true), "a lookalike scheme is not the bundled scheme")

print("browser-verify OK")
