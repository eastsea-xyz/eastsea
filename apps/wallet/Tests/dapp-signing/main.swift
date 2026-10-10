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

/// Hold completion to exercise refusal/dismissal while a real request lifecycle
/// is waiting for broadcast, without a node or Secure Enclave in the fixture.
final class DelayedDappSubmit {
    private var completion: ((Result<Any?, ProviderError>) -> Void)?
    private(set) var started = false

    func start(completion: @escaping (Result<Any?, ProviderError>) -> Void) {
        started = true
        self.completion = completion
    }

    func complete(_ result: Result<Any?, ProviderError>) {
        let callback = completion
        completion = nil
        callback?(result)
    }
}

let rejected = ProviderError(code: ProviderErrorCode.denied, message: "Refused")
do {
    var replies: [Result<Any?, ProviderError>] = []
    let request = DappApprovalLifecycle { replies.append($0) }
    let transport = DelayedDappSubmit()
    check(request.cancel(rejected), "refusal can cancel preparation before signing/submission")
    if request.beginSubmission(stillApproved: true) { transport.start { request.finish($0) } }
    check(!transport.started, "a request refused during preparation never broadcasts")
    check(replies.count == 1, "pre-submission refusal replies once")
    if let first = replies.first, case .failure(let error) = first { check(error.code == 4001, "pre-submission refusal is 4001") }
    else { check(false, "pre-submission refusal has no success value") }
}
do {
    var replies: [Result<Any?, ProviderError>] = []
    let request = DappApprovalLifecycle { replies.append($0) }
    let transport = DelayedDappSubmit()
    let consentStillCurrentAfterSigning = false
    if request.beginSubmission(stillApproved: consentStillCurrentAfterSigning) { transport.start { request.finish($0) } }
    check(!transport.started && request.canCancel, "context invalidated while signing cannot enter broadcast")
    check(request.cancel(rejected) && replies.count == 1, "stale signed request remains cancellable before broadcast")
}
for succeed in [true, false] {
    var originalReplies: [Result<Any?, ProviderError>] = []
    var nextPageReplies: [Result<Any?, ProviderError>] = []
    let original = DappApprovalLifecycle { originalReplies.append($0) }
    let nextPage = DappApprovalLifecycle { nextPageReplies.append($0) }
    let transport = DelayedDappSubmit()
    check(original.beginSubmission(stillApproved: true), "reviewed signed request enters submission synchronously")
    transport.start { original.finish($0) }
    check(original.isSubmitting && !original.canCancel, "delayed broadcast has an irreversible submission phase")
    check(!original.cancel(rejected), "Refuse/dismiss cannot claim unsent rejection during delayed broadcast")
    check(originalReplies.isEmpty, "broadcast cancellation leaves original reply pending for the real outcome")
    check(nextPage.canCancel && nextPageReplies.isEmpty, "new page retains independent consent and reply")
    let result: Result<Any?, ProviderError> = succeed
        ? .success("0xactual-broadcast-hash")
        : .failure(ProviderError(code: ProviderErrorCode.internalError, message: "Broadcast failed"))
    transport.complete(result)
    check(originalReplies.count == 1 && nextPageReplies.isEmpty, "completion reaches only the original request after page change")
    if succeed, let first = originalReplies.first, case .success(let hash) = first {
        check(hash as? String == "0xactual-broadcast-hash", "delayed success returns actual transaction hash")
    } else if !succeed, let first = originalReplies.first, case .failure(let error) = first {
        check(error.code == ProviderErrorCode.internalError && error.code != 4001, "delayed failure returns broadcast error rather than refusal")
    } else { check(false, "delayed broadcast returns expected outcome") }
    check(!original.finish(result) && !original.cancel(rejected) && originalReplies.count == 1,
          "late completion and dismissal cannot reply twice")
}
print("dapp-signing: \(failures) failures")
exit(failures == 0 ? 0 : 1)
