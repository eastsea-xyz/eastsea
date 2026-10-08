import Foundation

var failures = 0
func check(_ value: Bool, _ label: String) {
    if !value { failures += 1; print("FAIL", label) }
}
let me = "0x" + String(repeating: "11", count: 20)
let other = "0x" + String(repeating: "22", count: 20)
let token = "0x" + String(repeating: "33", count: 20)
func topic(_ address: String) -> String { "0x" + String(repeating: "0", count: 24) + address.dropFirst(2) }
func word(_ value: String) -> String { String(repeating: "0", count: 64 - value.count) + value }
func response(_ logs: [[String: Any]] = [], success: Bool = true) -> [String: Any] {
    ["success": success, "gasUsed": "0x12345", "output": "0x", "failureReason": success ? NSNull() : "Insufficient balance",
     "nativeChanges": [["address": me, "deltaWei": "-1000000000000000000"]], "logs": logs]
}
let transfer: [String: Any] = ["address": token, "topics": [DappSimulation.transferTopic, topic(me), topic(other)], "data": "0x" + word("ffffffffffffffffffffffffffffffff")]
let incoming: [String: Any] = ["address": token, "topics": [DappSimulation.transferTopic, topic(other), topic(me)], "data": "0x" + word("01")]
// Independent ABI event vector (cast keccak), not the implementation constant.
let approval: [String: Any] = ["address": token, "topics": ["0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925", topic(me), topic(other)], "data": "0x" + String(repeating: "f", count: 64)]
do {
    let preview = try DappSimulation.parse(response([transfer, incoming, approval]), account: me)
    check(preview.success && preview.gasUsed == 74565, "success and exact gas")
    check(preview.nativeDeltaWei == "-1000000000000000000", "native delta is exact")
    check(preview.changes.count == 1 && preview.changes[0].delta == "-340282366920938463463374607431768211454", "token deltas aggregate without floating point")
    check(preview.approvals.count == 1 && preview.approvals[0].unlimited, "unlimited approval is explicit")
    var measured = response([transfer])
    measured["tokenChanges"] = [["token": token, "delta": "-7"]]
    measured["measuredTokens"] = [token]
    measured["tokenCoverageComplete"] = true
    let actual = try DappSimulation.parse(measured, account: me)
    check(actual.balancesMeasured && actual.changes[0].delta == "-7", "measured balances override claimed Transfer events")
    measured["tokenChanges"] = [[String: Any]]()
    let zeroMeasured = try DappSimulation.parse(measured, account: me)
    check(zeroMeasured.changes.isEmpty, "measured zero balances override misleading logs")
    measured["measuredTokens"] = [String]()
    measured["tokenCoverageComplete"] = false
    let uncovered = try DappSimulation.parse(measured, account: me)
    check(!uncovered.balancesMeasured && !uncovered.changes.isEmpty, "unmeasured token movements remain visible with warning")
    let nftRevoke: [String: Any] = ["address": token, "topics": ["0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925", topic(me), topic("0x" + String(repeating: "0", count: 40)), "0x" + word("1")], "data": "0x"]
    check(try DappSimulation.parse(response([nftRevoke]), account: me).approvals.first?.amount == "0", "NFT zero-spender approval is a revocation")
    let failed = try DappSimulation.parse(response([transfer, approval], success: false), account: me)
    check(!failed.success && failed.changes.isEmpty && failed.approvals.isEmpty && failed.nativeDeltaWei == "0", "a reverted simulation grants and moves nothing")
    check(failed.failureReason == "Insufficient balance", "plain failure reason")
    check(!failed.canSign(extraConfirmation: false) && failed.canSign(extraConfirmation: true), "revert needs separate confirmation")
    let selfLog: [String: Any] = ["address": token, "topics": [DappSimulation.transferTopic, topic(me), topic(me)], "data": "0x" + word("1")]
    check(try DappSimulation.parse(response([selfLog]), account: me).changes.isEmpty, "self transfers net to zero")
    let unrelated: [String: Any] = ["address": token, "topics": [DappSimulation.approvalTopic, topic(other), topic(me)], "data": "0x" + word("f")]
    check(try DappSimulation.parse(response([unrelated]), account: me).approvals.isEmpty, "other owners do not grant our approvals")
} catch { check(false, "valid simulation parses: \(error)") }
for invalid in [[String: Any](), ["success": "true"], ["success": true, "gasUsed": "bad", "logs": [], "nativeChanges": []]] {
    do { _ = try DappSimulation.parse(invalid, account: me); check(false, "malformed response refused") } catch { }
}
let context = DappRequestContext(account: me, chainId: 7781, port: 18545, generation: 7)
check(context.matches(account: me.uppercased(), chainId: 7781, port: 18545, generation: 7), "same account comparison is case insensitive")
check(!context.matches(account: other, chainId: 7781, port: 18545, generation: 7), "switched account invalidates approval")
check(!context.matches(account: me, chainId: 7780, port: 18545, generation: 7), "wrong chain invalidates approval")
check(!context.matches(account: me, chainId: 7781, port: 18545, generation: 8), "switch away and back invalidates approval")
check(!context.matches(account: me, chainId: 7781, port: 18545, generation: 7, permissionGeneration: 1), "revoked and regranted site invalidates approval")
check(AccountMigration.validDestination(other, current: me), "fresh destination accepted")
check(!AccountMigration.validDestination(me, current: me) && !AccountMigration.validDestination("0x123", current: me), "self/invalid migration destination refused")
let typed = #"{"domain":{"name":"Market","chainId":"7781","verifyingContract":"0x2222222222222222222222222222222222222222"},"message":{"amount":"100000000000000000000","recipient":"0x2222222222222222222222222222222222222222","nested":{"active":true}},"primaryType":"Permit"}"#
do {
    let view = try TypedMessageFields.parse(typed)
    check(view.domain.contains { $0.path == "name" && $0.value == "Market" }, "typed domain is readable")
    check(view.message.contains { $0.path == "nested.active" && $0.value == "Yes" }, "nested boolean is words")
    check(view.message.contains { $0.path == "amount" && $0.value == "100000000000000000000" }, "typed integer is exact")
} catch { check(false, "typed fields parse") }
do {
    check(try TypedMessageRequest.parse([me, typed], context: context) == typed, "typed request matches connected account and chain")
} catch { check(false, "valid typed request") }
let invalidParams: [[Any]] = [[other, typed], [me, typed.replacingOccurrences(of: "7781", with: "7780")], [me, typed.replacingOccurrences(of: "Market", with: "\u{202e}Market")]]
for params in invalidParams {
    do { _ = try TypedMessageRequest.parse(params, context: context); check(false, "foreign account/chain/invisible name refused") } catch { }
}
print("dapp-signing: \(failures) failures")
exit(failures == 0 ? 0 : 1)
