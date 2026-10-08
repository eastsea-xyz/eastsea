import Foundation

let root = ProcessInfo.processInfo.environment["AETHER_PUSH_SOURCE_ROOT"] ?? FileManager.default.currentDirectoryPath
func source(_ path: String) -> String { (try? String(contentsOfFile: root + "/" + path, encoding: .utf8)) ?? "" }
let wallet = source("apps/wallet/Sources/WalletModel.swift")
let app = source("apps/wallet/Sources/AetherWalletApp.swift")
let node = source("apps/wallet/Sources/NodeController.swift")
let push = source("apps/wallet/Sources/WalletPush.swift")
var failures = 0
func check(_ condition: Bool, _ name: String) {
    print("\(condition ? "OK" : "FAIL") \(name)")
    if !condition { failures += 1 }
}
check(wallet.contains("WalletPushClient"), "wallet owns a local node subscription")
check(!wallet.contains("timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true)"), "wallet has no recurring balance discovery timer")
check(wallet.contains("shouldReadTransaction") && wallet.contains("push.revision"), "receipt following wakes on push or backed-off fallback")
check(app.contains("automaticallyChecksForUpdates = false"), "Sparkle recurring discovery is disabled")
check(app.contains("model.onReleaseNotice ="), "release push goes through the existing updater")
check(app.contains("pendingReleaseItem") && app.contains("advanceReleaseWindow()"), "head push revisits a known release when its approval window opens")
check(node.contains("func walletReleaseNotice()") && !node.contains("        refreshUpgrade()\n        refreshDisk()"), "upgrade discovery leaves the node safety polling loop")
check(!wallet.contains("($0.state == .pending && Date().timeIntervalSince($0.date) > Self.trackLimit)"), "saved pending receipts reconcile without an eleven-minute delay")
check(wallet.contains("if notice.transactionsChanged { reconcileUnresolved(force: true) }"), "transaction hints bypass the regular receipt throttle")
check(wallet.contains("reconciliation.finish(retry:") && wallet.contains("drainUnresolvedReconciliation()"), "dirty receipts survive in-flight and partial asynchronous batches")
check(wallet.contains("reconciliation.headOpportunity("), "hashes beyond the subscription cap get fair head-driven reconciliation")
check(wallet.contains("push.onPulse =") && wallet.contains("self?.reevaluateCachedProgress()"), "a ping-healthy stalled chain still advances cached pause detection")
check(wallet.contains("if readBalance { self.refreshTokens(force: true) }"), "healthy head-only frames do not rediscover token balances")
check(wallet.contains("if force { tokenRefreshWork.invalidate([\"tokens\"]) }") && wallet.contains("if self.tokenRefreshWork.hasReadyWork { self.refreshTokens() }"), "token dirtiness received during a scan is retained and drained")
let acknowledgement = push.components(separatedBy: "mutating func connected()").dropFirst().first?.components(separatedBy: "mutating func").first ?? ""
let subscriptionAcknowledgement = push.components(separatedBy: "case .subscribed(let id):").dropFirst().first?.components(separatedBy: "case .notice(let notice):").first ?? ""
check(!acknowledgement.contains("failures = 0") && !subscriptionAcknowledgement.contains("connectDeadline = nil") && push.contains("policy.healthy()"), "acknowledgement alone does not reset flapping-stream backoff")
check(app.contains("if tracker.retryDue(), tracker.itemKey != nil"), "the update timer retries known items without bare feed discovery")
exit(failures == 0 ? 0 : 1)
