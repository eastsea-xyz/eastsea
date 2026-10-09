import Foundation

var failures = 0
func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); failures += 1 }
}
func bytes(_ text: String) -> Data { Data(text.utf8) }
let addr = "0x" + String(repeating: "11", count: 20)
let hash = "0x" + String(repeating: "22", count: 32)
let filter = WalletPushFilter(address: addr, transactions: [hash, hash.uppercased()] + (0..<40).map { "0x" + String(format: "%064x", $0) })
check(filter.address == addr && filter.transactions.count == 32, "bounded, deduplicated wallet filters")
check(WalletPushFilter(address: "bad", transactions: ["bad", hash]).address == nil, "invalid account never becomes a subscription address")
check(WalletPushFilter(address: addr, transactions: ["bad", hash]).transactions == [hash], "invalid transaction hashes are excluded")
let request = WalletPushWire.request(id: 1, filter: filter, after: 123)
let json = try! JSONSerialization.jsonObject(with: request) as! [String: Any]
let params = json["params"] as! [Any]
let body = params[1] as! [String: Any]
check(json["method"] as? String == "aether_subscribe" && params[0] as? String == "wallet", "wallet subscription method and mode")
check(body["after"] as? UInt64 == 123 && (body["transactions"] as? [String])?.count == 32, "resume cursor and bounded transaction filters")
check(WalletPushWire.parse(bytes(#"{"jsonrpc":"2.0","id":1,"result":"0xa"}"#), subscription: nil) == .subscribed("0xa"), "subscription acknowledgement")
let frame = bytes(#"{"jsonrpc":"2.0","method":"aether_subscription","params":{"subscription":"0xa","result":{"kind":"wallet","height":124,"topics":["head","balance","tx_status","release"]}}}"#)
if case .notice(let notice) = WalletPushWire.parse(frame, subscription: "0xa") {
    check(notice.height == 124 && notice.balanceChanged && notice.transactionsChanged && notice.releaseChanged, "topic dirtiness is parsed as hints")
} else { check(false, "topic dirtiness is parsed as hints") }
check(WalletPushWire.parse(frame, subscription: "0xb") == nil, "frames for another subscription are ignored")
check(WalletPushWire.parse(bytes(#"{"method":"aether_subscription","params":{"subscription":"0xa","result":{"kind":"gap","height":124,"reason":"slow_reader"}}}"#), subscription: "0xa") == .gap, "gap ends the stream for bounded reconciliation")
check(WalletPushWire.parse(bytes(#"{"id":1,"error":{"code":-32601,"message":"unsupported"}}"#), subscription: nil) == .rejected, "old node enters fallback on unsupported subscription")
check(WalletPushWire.parse(bytes(#"{"method":"aether_subscription","params":{"subscription":"0xa","result":{"kind":"wallet","height":true,"topics":["balance"]}}}"#), subscription: "0xa") == nil, "boolean height is rejected")
check(WalletPushWire.parse(bytes(#"{"method":"aether_subscription","params":{"subscription":"0xa","result":{"kind":"wallet","height":-1,"topics":["balance"]}}}"#), subscription: "0xa") == nil, "negative height is rejected")
check(WalletPushWire.parse(Data(repeating: 32, count: WalletPushWire.maxFrameBytes + 1), subscription: "0xa") == .rejected, "oversized stream frames are rejected before decoding")

var policy = WalletPushPolicy()
policy.connecting(at: 0)
check(!policy.retryDue(at: 4), "initial connection does not poll before the stream fails")
check(policy.connected() && !policy.connected(), "exactly one catch-up per connection acknowledgement")
policy.healthy()
var healthyPolls = 0
for second in 1...60 { if policy.retryDue(at: Double(second)) { healthyPolls += 1 } }
check(healthyPolls == 0, "healthy subscription has zero discovery polls in sixty seconds")
policy.disconnected(at: 60, jitter: 0.5)
check(!policy.retryDue(at: 61) && policy.retryDue(at: 62), "drop permits fallback only after the first backoff")
policy.connecting(at: 62)
policy.disconnected(at: 62, jitter: 0.5)
check(!policy.retryDue(at: 65) && policy.retryDue(at: 66), "repeated failure doubles backoff")
for attempt in 0..<100 { policy.disconnected(at: Double(attempt), jitter: 1) }
check(policy.nextRetry.map { $0 - 99 <= WalletPushPolicy.maxBackoff } == true, "jitter never exceeds the backoff cap")
check(policy.connected(), "reconnect schedules a single catch-up")
check(!policy.retryDue(at: 1_000), "successful reconnect immediately disables fallback polling")
policy.healthy()
policy.disconnected(at: 1_000, jitter: 0.5)
check(policy.nextRetry == 1_002, "successful stream resets retry backoff")
check(policy.shouldReadTransaction(lastRevision: nil, revision: 1), "submission reads its initial receipt once")
check(!policy.shouldReadTransaction(lastRevision: 7, revision: 7), "quiet stream does not rediscover a tracked receipt")
check(policy.shouldReadTransaction(lastRevision: 7, revision: 8), "transaction hint or fallback wakes receipt reading")

var flapping = WalletPushPolicy()
flapping.connecting(at: 0)
check(flapping.connected(), "flapping stream still performs its first catch-up")
flapping.disconnected(at: 0, jitter: 0.5)
check(flapping.nextRetry == 2, "first acknowledgement-then-close waits two seconds")
flapping.connecting(at: 2)
check(flapping.connected() && !flapping.connected(), "each flapping acknowledgement catches up exactly once")
flapping.disconnected(at: 2, jitter: 0.5)
check(flapping.nextRetry == 6, "acknowledgement-then-close does not erase the next four-second backoff")
flapping.connecting(at: 6)
_ = flapping.connected()
flapping.disconnected(at: 6, jitter: 0.5)
check(flapping.nextRetry == 14, "repeated flapping continues to grow its backoff")
flapping.connecting(at: 14)
_ = flapping.connected()
flapping.healthy()
flapping.disconnected(at: 14, jitter: 0.5)
check(flapping.nextRetry == 16, "first valid wallet delivery resets the failure streak")

var outage = WalletPushPolicy()
outage.connecting(at: 0)
outage.disconnected(at: 0, jitter: 0.5)
var fallbackPolls = 0
for second in 1...60 {
    if outage.retryDue(at: Double(second)) {
        fallbackPolls += 1
        outage.connecting(at: Double(second))
        outage.disconnected(at: Double(second), jitter: 0.5)
    }
}
check(fallbackPolls == 4, "persistent outage backs off to four polls in the first minute")

let transactionHashes = (0..<40).map { "tx\($0)" }
var receipts = WalletPushReconciliation()
receipts.invalidate(transactionHashes)
let firstBatch = receipts.nextBatch()
check(firstBatch == Array(transactionHashes.prefix(8)), "receipt reconciliation begins with a bounded batch")
check(receipts.nextBatch().isEmpty, "receipt batches never overlap")
receipts.invalidate([transactionHashes[0]])
receipts.finish(retry: [])
var reconciled = firstBatch
while true {
    let batch = receipts.nextBatch()
    if batch.isEmpty { break }
    reconciled += batch
    receipts.finish(retry: [])
}
check(Set(reconciled) == Set(transactionHashes), "a single transaction hint drains every partial receipt batch")
check(reconciled.filter { $0 == transactionHashes[0] }.count == 2, "a hint arriving during an in-flight read is retained for another read")

var unavailableReceipt = WalletPushReconciliation()
unavailableReceipt.invalidate([hash])
check(unavailableReceipt.nextBatch() == [hash], "receipt read starts immediately on a dirty hint")
unavailableReceipt.finish(retry: [hash])
check(unavailableReceipt.nextBatch().isEmpty && unavailableReceipt.pendingCount == 1, "failed receipt read retains work without a busy retry loop")
unavailableReceipt.headOpportunity(available: [hash], subscribed: [hash])
check(unavailableReceipt.nextBatch() == [hash], "the next delivered head retries a retained failed read")
unavailableReceipt.finish(retry: [])

var overflowReceipts = WalletPushReconciliation()
var overflowVisited = Set<String>()
let overflowHashes = (0..<88).map { "overflow\($0)" }
for _ in 0..<7 {
    overflowReceipts.headOpportunity(available: overflowHashes, subscribed: Set(overflowHashes.prefix(32)))
    let batch = overflowReceipts.nextBatch()
    overflowVisited.formUnion(batch)
    overflowReceipts.finish(retry: [])
}
check(overflowVisited == Set(overflowHashes.dropFirst(32)), "heads cover every hash outside the 32-transaction subscription")
var quietReceipts = WalletPushReconciliation()
quietReceipts.headOpportunity(available: [hash], subscribed: [hash])
check(quietReceipts.nextBatch().isEmpty, "quiet subscribed transactions do not rediscover receipts on every head")
quietReceipts.invalidate([hash])
quietReceipts.retain(available: [])
check(quietReceipts.nextBatch().isEmpty && quietReceipts.pendingCount == 0, "settled or switched-account rows discard obsolete dirty work")
var boundedReceipts = WalletPushReconciliation()
boundedReceipts.invalidate((0..<1_000).map { "bounded\($0)" })
check(boundedReceipts.pendingCount == WalletPushReconciliation.maxPending, "dirty receipt memory is bounded to saved activity capacity")

var tokenReads = WalletPushReconciliation()
tokenReads.invalidate(["tokens"])
check(tokenReads.nextBatch() == ["tokens"], "token dirtiness starts one scan")
tokenReads.invalidate(["tokens"])
check(tokenReads.nextBatch().isEmpty, "an incoming transfer cannot overlap the current token scan")
tokenReads.finish(retry: [])
check(tokenReads.hasReadyWork && tokenReads.nextBatch() == ["tokens"], "a second transfer during a scan forces one retained follow-up scan")
tokenReads.finish(retry: [])
check(!tokenReads.hasReadyWork, "completed token dirtiness does not create a recurring scan")
tokenReads.invalidate(["tokens"])
_ = tokenReads.nextBatch()
tokenReads.finish(retry: ["tokens"])
check(!tokenReads.hasReadyWork && tokenReads.pendingCount == 1, "a failed token scan retains state without head-only retries")
tokenReads.invalidate(["tokens"])
check(tokenReads.hasReadyWork && tokenReads.nextBatch() == ["tokens"], "later token dirtiness retries a failed scan immediately")

let pausedNow = Date(timeIntervalSince1970: 1_000)
check(WalletPushProgress.pauseSince(now: pausedNow, changedAt: pausedNow.addingTimeInterval(-61), newest: pausedNow.addingTimeInterval(-61), after: 60) != nil,
      "cached chain age reaches paused even when a healthy ping supplies no heads")
check(WalletPushProgress.pauseSince(now: pausedNow, changedAt: pausedNow, newest: pausedNow, after: 60) == nil,
      "new chain progress clears the paused state without a discovery poll")
print("wallet-push polls/min: baseline=30 healthy=\(healthyPolls) first-minute-outage=\(fallbackPolls)")
if failures == 0 { print("wallet-push: all checks passed") }
exit(failures == 0 ? 0 : 1)
