// When a downloaded, verified update installs (docs/design/34-silent-updates.md
// §3.2, W1). Pure logic:
//   swiftc -o ./tmp/update-window apps/wallet/Sources/UpdateWindow.swift apps/wallet/Tests/update-window/main.swift && ./tmp/update-window
import Foundation
var failures = 0
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); failures += 1 } else { print("ok  ", m) } }

// R09: exercise the actual AppDelegate input, not only the pure Boolean gate.
let delegateURL = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    .appendingPathComponent("Sources/AetherWalletApp.swift")
let delegateSource = try String(contentsOf: delegateURL, encoding: .utf8)
check(delegateSource.contains("storageMoving: node.storageMovePreparing"),
      "R09 active storage mover is wired into the update gate")

typealias W = UpdateWindow
let slotRoster: [String: Any] = ["validators": [["key": String(repeating: "a", count: 64)]]]
let ourKey = String(repeating: "b", count: 64)
let assignedSlot: [String: Any] = ["committee_size": 4, "seat_index": 2, "allowed": false]
check(W.reconciledMembership(network: slotRoster, validatorKey: ourKey, restartSlot: assignedSlot) == true,
      "a new slot assignment cannot inherit an earlier absent roster observation")
check(W.reconciledMembership(network: slotRoster, validatorKey: ourKey,
    restartSlot: ["committee_size": 4, "seat_index": NSNull()]) == false,
      "both complete observations must confirm an unseated Mac")
check(W.reconciledMembership(network: slotRoster, validatorKey: ourKey, restartSlot: nil) == nil,
      "missing slot membership cannot authorize a writer stop")
var lateLease = W.MembershipSnapshot()
lateLease.observe(true, requestedAt: 100, generation: lateLease.generation)
var writerStops = 0
if W.decide(W.Moment(seated: lateLease.value(at: 116), inOwnSlot: true)) == .installNow { writerStops += 1 }
check(writerStops == 0, "slot lease expiring during verification stops no writer")
// R10: absent membership has no permission to restart without a chain slot.
check(W.decide(W.Moment()) != .installNow,
      "R10 unknown membership must wait without a restart slot")
check(delegateSource.contains("seated: node.updateMembership"),
      "R10 AppDelegate preserves unknown update membership")

let idle = W.Moment(seated: false)

check(W.decide(idle) == .installNow, "a non-validator with nothing open installs right away")

var m = idle
m.sendSheetOpen = true
check(W.decide(m) == .wait(.sendSheet), "an open send sheet holds the install")

m = idle
m.signing = true
check(W.decide(m) == .wait(.signing), "a Touch ID prompt or a signing in flight holds the install")

m = idle
m.migrating = true
check(W.decide(m) == .wait(.migration), "a running data migration holds the install")

m = idle
m.storageMoving = true
check(W.decide(m) == .wait(.storageMove), "a running block-data move holds the install")

m = idle
m.seated = true
m.inOwnSlot = true
check(W.decide(m) == .installNow, "a seated validator in its chain-assigned slot installs")

m.inOwnSlot = false
check(W.decide(m) == .wait(.outOfSlot), "a seated validator outside its slot waits")

m.inOwnSlot = nil
check(W.decide(m) == .wait(.seatedNoSlot), "a seated validator with no slot from the chain waits (installs when it leaves the committee, or at quit)")

m = idle
m.seated = true
m.inOwnSlot = true
m.sendSheetOpen = true
check(W.decide(m) == .wait(.sendSheet), "the slot never overrides an open send sheet")

m = idle
m.inOwnSlot = false
check(W.decide(m) == .installNow, "slot data is ignored for a Mac that is not seated")

check(W.Reason.seatedNoSlot.logLine.contains("waiting for safe moment"), "the waiting line says so")

// R10: unknown membership and absent slots never grant permission.
m = W.Moment()
check(W.decide(m) == .wait(.membershipUnknown), "R10 unknown membership waits")
m.inOwnSlot = true
check(W.decide(m) == .wait(.membershipUnknown), "R10 an unverified slot does not resolve unknown membership")
m.storageMoving = true
check(W.decide(m) == .wait(.storageMove), "R10 storage safety still takes priority")

let mine = String(repeating: "a", count: 64)
let other = String(repeating: "b", count: 64)
let member: [String: Any] = ["validators": [["key": mine]]]
let follower: [String: Any] = ["validators": [["key": other]]]
check(W.votingMembership(network: member, validatorKey: mine) == true, "R10 confirmed seated membership")
check(W.votingMembership(network: follower, validatorKey: mine) == false, "R10 complete voting set confirms unseated")
check(W.votingMembership(network: nil, validatorKey: mine) == nil, "R10 failed initial network read remains unknown")
check(W.votingMembership(network: NSNull(), validatorKey: mine) == nil, "R10 forwarded null remains unknown")
check(W.votingMembership(network: [:], validatorKey: mine) == nil, "R10 missing validator list remains unknown")
check(W.votingMembership(network: ["validators": []], validatorKey: mine) == nil, "R10 empty voting set remains unknown")
check(W.votingMembership(network: ["validators": [["key": other], ["bad": mine]]], validatorKey: mine) == nil,
      "R10 malformed member row cannot confirm absence")
check(W.votingMembership(network: ["validators": [["key": other], ["key": "bad"]]], validatorKey: mine) == nil,
      "R10 malformed member key cannot confirm absence")
check(W.votingMembership(network: ["validators": [["key": other], ["key": other]]], validatorKey: mine) == nil,
      "R10 duplicate member keys cannot confirm absence")
check(W.votingMembership(network: member, validatorKey: "bad") == nil, "R10 unknown candidate identity waits")
check(W.votingMembership(network: member, validatorKey: "0x" + mine.uppercased()) == true,
      "R10 canonical public-key spelling identifies the same member")

// Recovery uses the attested shard identity; a validator key is not a node ID.
let myNode = String(repeating: "c", count: 64)
let otherNode = String(repeating: "d", count: 64)
let nodeMember: [String: Any] = ["validators": [["key": mine, "node": myNode], ["key": other, "node": otherNode]]]
let nodeFollower: [String: Any] = ["validators": [["key": myNode, "node": otherNode]]]
check(W.votingMembership(network: nodeMember, nodeID: myNode) == true, "attested seated node membership is confirmed")
check(W.votingMembership(network: nodeFollower, nodeID: myNode) == false, "a complete node set confirms a follower")
check(W.votingMembership(network: nodeMember, nodeID: mine) == false, "validator keys do not identify member nodes")
check(W.votingMembership(network: nodeFollower, validatorKey: myNode) == true, "the key overload keeps its separate identity semantics")
check(W.votingMembership(network: nodeMember, validatorKey: mine) == true, "node fields preserve existing key membership")
check(W.votingMembership(network: nodeMember, nodeID: "0X" + myNode.uppercased()) == true,
      "node identity accepts canonical prefix and case variants")
check(W.votingMembership(network: ["validators": [["node": "0x" + myNode.uppercased()]]], nodeID: myNode) == true,
      "member node prefix and case variants normalize before comparison")
check(W.votingMembership(network: nil, nodeID: myNode) == nil, "failed node membership read remains unknown")
check(W.votingMembership(network: NSNull(), nodeID: myNode) == nil, "null node membership remains unknown")
check(W.votingMembership(network: [:], nodeID: myNode) == nil, "missing node member list remains unknown")
check(W.votingMembership(network: ["validators": []], nodeID: myNode) == nil, "empty node member list remains unknown")
check(W.votingMembership(network: ["validators": "invalid"], nodeID: myNode) == nil, "a non-array node member list remains unknown")
check(W.votingMembership(network: ["validators": [["node": otherNode], ["key": mine]]], nodeID: myNode) == nil,
      "a missing member node ID cannot confirm absence")
check(W.votingMembership(network: ["validators": [["node": otherNode], ["node": ""]]], nodeID: myNode) == nil,
      "an empty member node ID cannot confirm absence")
check(W.votingMembership(network: ["validators": [["node": myNode], ["node": "bad"]]], nodeID: myNode) == nil,
      "a malformed later member invalidates even a seated match")
check(W.votingMembership(network: ["validators": [["node": otherNode], ["node": 7]]], nodeID: myNode) == nil,
      "a non-string member node ID cannot confirm absence")
check(W.votingMembership(network: ["validators": [["node": otherNode], ["node": otherNode]]], nodeID: myNode) == nil,
      "duplicate member node IDs cannot confirm absence")
check(W.votingMembership(network: ["validators": [["node": myNode], ["node": "0X" + myNode.uppercased()]]], nodeID: myNode) == nil,
      "duplicate canonical node identities invalidate even a seated match")
for invalidNode in ["", "bad", String(repeating: "c", count: 63), String(repeating: "c", count: 65), String(repeating: "g", count: 64)] {
    check(W.votingMembership(network: nodeMember, nodeID: invalidNode) == nil, "a malformed own node identity remains unknown")
}

var membership = W.MembershipSnapshot()
check(membership.value(at: 100) == nil, "R10 initial membership snapshot is unknown")
let initialGeneration = membership.generation
membership.observe(false, requestedAt: 100, generation: initialGeneration)
m = W.Moment(seated: membership.value(at: 101))
check(W.decide(m) == .installNow, "R10 freshly confirmed unseated status permits installation")
m.seated = membership.value(at: 100 + W.MembershipSnapshot.maxAge + 0.01)
check(W.decide(m) == .wait(.membershipUnknown), "R10 stale unseated status waits")
check(membership.value(at: 99) == nil, "R10 future-dated membership cannot grant permission")
membership.observe(nil, requestedAt: 102, generation: initialGeneration)
check(membership.value(at: 103) == nil, "R10 failed refresh invalidates earlier unseated status")
membership.observe(false, requestedAt: 104, generation: initialGeneration)
membership.invalidate()
check(!membership.observe(false, requestedAt: 105, generation: initialGeneration),
      "R10 delayed result from an earlier node generation is rejected")
check(membership.value(at: 106) == nil, "R10 restart or wake requires new membership")
membership.observe(true, requestedAt: 107, generation: membership.generation)
m = W.Moment(seated: membership.value(at: 108))
check(W.decide(m) == .wait(.seatedNoSlot), "R10 fresh seated membership waits without a chain slot")

if failures > 0 { print("\(failures) failed"); exit(1) }
print("all passed")
