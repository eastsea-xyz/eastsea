// The node's stop reasons and start gate (NodeStopReason.swift): every path
// that leaves the switch on with no node running has one typed reason with
// plain copy and exact numbers, and the 30 s gate never parks the node
// forever on a state it could recover from by itself.
//   scripts/test-swift-pure.sh   (run node-stop)
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }
let GiB: UInt64 = 1_073_741_824

// MARK: the founder's MacBook (2026-10-07)

// The new app launched while the previous one's node still held run.lock: its
// own start exited 7 and it attached to that node. The old node then exited
// with its parent. While attached, the gate used to answer "keep running"
// whatever happened to that node — the attach-miss counter behind the 2 s
// poll was the only way out. The lock probe is the ground truth: once nobody
// holds run.lock, the attached node is gone, so start our own.
var f = NodeResumeFacts()
f.attached = true
f.lockHeldByOther = false
check(NodeResume.decide(f) == .start(detach: true), "attached to a node that no longer holds run.lock: detach and start")
f.lockHeldByOther = true
check(NodeResume.decide(f) == .keepRunning, "attached to a live node (it holds run.lock): keep it")

// A crash loop that a restart may fix (memory, network, unknown) used to
// block every automatic start until the app was relaunched by hand. It is
// tried again after the ten-minute crash window, and says when.
f = NodeResumeFacts()
f.blocked = .other
f.blockedForSeconds = 60
check(NodeResume.decide(f) == .wait(.crashLoop(.other, retryInSeconds: 540)), "a crash loop waits out ten minutes and says how long is left")
f.blockedForSeconds = 600
check(NodeResume.decide(f) == .start(detach: false), "after ten minutes a crash loop is tried again by itself")
f.blocked = .network; f.blockedForSeconds = 601
check(NodeResume.decide(f) == .start(detach: false), "network crash loops retry too")

// "Another app runs this node" resumes when that app lets go of run.lock.
f = NodeResumeFacts()
f.blocked = .alreadyRunning
f.lockHeldByOther = true
check(NodeResume.decide(f) == .wait(.otherNodeRunning), "lock still held: wait, and say who to quit")
f.lockHeldByOther = false
check(NodeResume.decide(f) == .start(detach: false), "lock released: start by itself")

// MARK: the disk floor, on the node's own numbers (5 GB floor, +2 GB resume)

f = NodeResumeFacts()
f.blocked = .diskFull
f.freeBytes = UInt64(6.1 * Double(GiB))
check(NodeResume.decide(f) == .wait(.diskFull(freeBytes: f.freeBytes!, resumeBytes: 7 * GiB, volume: nil)),
      "below the resume level: wait with the exact numbers")
f.freeBytes = 7 * GiB
check(NodeResume.decide(f) == .start(detach: false), "at the node's own resume level (7 GB): start")
let disk = NodeStopReason.diskFull(freeBytes: UInt64(6.1 * Double(GiB)), resumeBytes: 7 * GiB, volume: nil).copy(ko: true)
check(disk.detail == "저장 공간 6.1 GB 남음. 약 0.9 GB만 더 비워 주세요.", "ko disk detail: \(disk.detail)")
check(disk.resume == "7.0 GB가 되면 저절로 다시 시작해요.", "ko disk resume: \(disk.resume)")
check(disk.action == .openStorage, "the disk reason's button opens storage")
check(NodeStopReason.need(free: 3 * GiB, resume: 7 * GiB) == "4 GB", "whole gigabytes above 1 GB")
check(NodeStopReason.need(free: UInt64(6.95 * Double(GiB)), resume: 7 * GiB) == "0.1 GB", "never says 0.0")
let ext = NodeStopReason.diskFull(freeBytes: 2 * GiB, resumeBytes: 7 * GiB, volume: "외장 SSD").copy(ko: true)
check(ext.detail.hasPrefix("‘외장 SSD’ 저장 공간 2.0 GB 남음"), "the chosen volume is named: \(ext.detail)")

// MARK: the chosen block-data disk

f = NodeResumeFacts()
f.storage = .chosen(volume: "Archive", mounted: false, writable: false)
check(NodeResume.decide(f) == .wait(.diskMissing(volume: "Archive")), "an unplugged disk waits — never falls back to the internal disk")
f.storage = .chosen(volume: "Archive", mounted: true, writable: false)
check(NodeResume.decide(f) == .wait(.diskNoAccess(volume: "Archive")), "no permission: say where to allow it")
f.storage = .chosen(volume: "Archive", mounted: true, writable: true)
check(NodeResume.decide(f) == .start(detach: false), "disk back: start")
let missing = NodeStopReason.diskMissing(volume: "Archive").copy(ko: false)
check(missing.title == "Disk “Archive” not connected" && missing.action == .chooseDisk, "missing-disk copy")
check(NodeStopReason.diskMissing(volume: "외장").copy(ko: true).title == "‘외장’ 디스크가 연결되지 않음", "ko missing-disk title")

// MARK: every other path, in order

f = NodeResumeFacts(); f.enabled = false
check(NodeResume.decide(f) == .wait(.switchedOff), "switch off")
f = NodeResumeFacts(); f.wrongLocation = true
check(NodeResume.decide(f) == .wait(.wrongLocation), "wrong location")
f = NodeResumeFacts(); f.hasBinary = false
check(NodeResume.decide(f) == .wait(.noHelper), "no helper")
f = NodeResumeFacts(); f.migrating = true
check(NodeResume.decide(f) == .wait(.migrating), "migration copying")
f = NodeResumeFacts(); f.migrationGate = "x"
check(NodeResume.decide(f) == .wait(.migrationBlocked("x")), "migration gate")
f = NodeResumeFacts(); f.movingStoragePercent = 42
check(NodeResume.decide(f) == .wait(.movingStorage(percent: 42)), "storage move")
f = NodeResumeFacts(); f.onBattery = true
check(NodeResume.decide(f) == .wait(.onBattery), "battery")
f.isValidator = true
check(NodeResume.decide(f) == .start(detach: false), "a validator keeps voting on battery")
f = NodeResumeFacts(); f.onBattery = true; f.onlyOnPower = false
check(NodeResume.decide(f) == .start(detach: false), "battery allowed")
f = NodeResumeFacts(); f.restartInSeconds = 4
check(NodeResume.decide(f) == .wait(.restarting(inSeconds: 4)), "a pending restart keeps its backoff")
f = NodeResumeFacts(); f.blocked = .upgradeNeeded
check(NodeResume.decide(f) == .wait(.upgradeNeeded), "upgrade")
f = NodeResumeFacts(); f.blocked = .identityLost
check(NodeResume.decide(f) == .wait(.identityLost), "identity")
f = NodeResumeFacts(); f.blocked = .keyElsewhere; f.blockedForSeconds = 86_400
check(NodeResume.decide(f) == .wait(.keyElsewhere), "hardware binding needs attention even after the crash retry window")
check(NodeStopReason.keyElsewhere.copy(ko: true).detail.contains("이 노드의 키가 다른 Mac에서 옮겨 왔어요"), "copied-key reason explains the stop in Korean")
check(NodeStopReason.keyElsewhere.copy(ko: false).detail.contains("This node's keys came from another Mac"), "copied-key reason explains the stop in English")
check(NodeStopReason.keyElsewhere.copy(ko: false).detail.contains("Restore") && NodeStopReason.keyElsewhere.copy(ko: false).detail.contains("rebind"), "confirmed mismatch offers restore or explicit rebind, preserving the identity")
check(NodeStopReason.keyElsewhere.copy(ko: true).detail.contains("복원") && NodeStopReason.keyElsewhere.copy(ko: true).detail.contains("다시 연결"), "confirmed mismatch offers restore or explicit rebind in Korean")

// An unavailable read is a live node waiting, never a proven mismatch or a
// request to change identities. Its log markers recover without a restart.
for ko in [false, true] {
    let waiting = NodeStopReason.waitingForMacConfirmation.copy(ko: ko)
    check(!NodeStopReason.waitingForMacConfirmation.isIncident, "Mac confirmation is a calm waiting state")
    check(waiting.action == nil && waiting.actionLabel == nil, "an unavailable read has no owner recovery action")
    check(!waiting.detail.contains("another Mac") && !waiting.detail.contains("다른 Mac"), "unknown verification does not assert another Mac")
    check(waiting.resume.contains(ko ? "저절로" : "by itself"), "Mac confirmation says it retries by itself")
}
check(NodeStopReason.waitingForMacConfirmation.copy(ko: false).title == "Waiting to confirm this Mac", "unavailable-read English title")
check(NodeStopReason.waitingForMacConfirmation.copy(ko: true).title == "이 Mac 확인 대기", "unavailable-read Korean title")
check(NodeMacConfirmation.waiting(in: "waiting to confirm this Mac: IOKit lookup unavailable"), "an unavailable hardware read enters the waiting state")
check(NodeMacConfirmation.waiting(in: "still working", previously: true), "unrelated logs preserve a pending confirmation")
check(!NodeMacConfirmation.waiting(in: "waiting to confirm this Mac\nMac key binding confirmed"), "a successful retry clears the waiting state")
check(NodeMacConfirmation.waiting(in: "Mac key binding confirmed\nwaiting to confirm this Mac"), "the latest binding transition wins")
f = NodeResumeFacts(); f.processRunning = true; f.confirmingMac = true
check(NodeResume.decide(f) == .keepRunning, "an unavailable read keeps the node running")
f.processRunning = false; f.attached = true; f.lockHeldByOther = true
check(NodeResume.decide(f) == .keepRunning, "an attached node waiting to confirm this Mac is kept alive")

// The unattended wrapper hides the child's terminal exit from an attached
// wallet, and a new wallet process has no watchdog memory. The node's persisted
// proven-mismatch marker must block both paths before they can start a child.
f = NodeResumeFacts(); f.attached = true; f.blocked = .keyElsewhere; f.lockHeldByOther = false
check(NodeResume.decide(f) == .wait(.keyElsewhere), "a terminal watchdog refusal prevents attached takeover after run.lock is released")
guard let markerTempRoot = ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"] else {
    check(false, "AETHER_AGENT_TEST_TMP is required for the persisted-refusal fixture")
    exit(1)
}
let markerDirectory = URL(fileURLWithPath: markerTempRoot)
    .appendingPathComponent("key-binding-refusal-\(UUID().uuidString)", isDirectory: true)
do {
    try FileManager.default.createDirectory(at: markerDirectory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: markerDirectory) }
    check(!NodeBindingRefusal.exists(in: markerDirectory), "an unavailable read has no persisted mismatch marker")
    try Data("proven mismatch\n".utf8).write(to: markerDirectory.appendingPathComponent(NodeBindingRefusal.fileName))
    f = NodeResumeFacts(); f.keyBindingRefused = NodeBindingRefusal.exists(in: markerDirectory)
    check(f.blocked == nil && NodeResume.decide(f) == .wait(.keyElsewhere), "an app relaunch with no watchdog memory refuses an automatic start")
    f.attached = true; f.lockHeldByOther = false
    check(NodeResume.decide(f) == .wait(.keyElsewhere), "an attached daemon's persisted exit 15 prevents takeover after its lock releases")
    f.lockHeldByOther = true
    check(NodeResume.decide(f) == .wait(.keyElsewhere), "a terminal refusal prevents another start while the dying daemon still holds run.lock")
    f = NodeResumeFacts(); f.keyBindingRefused = true; f.restartInSeconds = 1
    check(NodeResume.decide(f) == .wait(.keyElsewhere), "persisted refusal wins over an already scheduled watchdog restart")
    f.restartInSeconds = nil; f.blocked = .memory; f.blockedForSeconds = 86_400
    check(NodeResume.decide(f) == .wait(.keyElsewhere), "persisted refusal never expires through crash-loop recovery")
    // Only successful owner rebind clears this file; the wallet merely reads it.
    try FileManager.default.removeItem(at: markerDirectory.appendingPathComponent(NodeBindingRefusal.fileName))
    f = NodeResumeFacts(); f.keyBindingRefused = NodeBindingRefusal.exists(in: markerDirectory)
    check(NodeResume.decide(f) == .start(detach: false), "successful owner rebind permits a new start after clearing persisted refusal")
    f.processRunning = true; f.confirmingMac = true
    check(NodeResume.decide(f) == .keepRunning, "unavailable verification without a mismatch marker keeps the node alive")
} catch { check(false, "persisted-refusal fixture: \(error)") }

// Rebinding is limited to a confirmed stop screen, and the CLI approval does
// not exist unless the typed identity matches and owner authentication succeeds.
check(NodeStopReason.keyElsewhere.copy(ko: false).action == .rebindKeys, "the confirmed mismatch exposes owner rebind")
check(NodeStopReason.keyElsewhere.copy(ko: false).actionLabel == "Rebind Node Keys…", "confirmed mismatch names the owner recovery action")
check(NodeKeyRebind.canOffer(reason: .keyElsewhere, processRunning: false, attached: false, lockHeld: false), "a stopped mismatched node can request owner recovery")
for reason in [NodeStopReason.waitingForMacConfirmation, .identityLost, .switchedOff, .needsAttention(.storage)] {
    check(!NodeKeyRebind.canOffer(reason: reason, processRunning: false, attached: false, lockHeld: false), "\(reason.code) cannot offer rebind")
}
check(!NodeKeyRebind.canOffer(reason: nil, processRunning: false, attached: false, lockHeld: false), "healthy/off-screen paths cannot offer rebind")
check(!NodeKeyRebind.canOffer(reason: .keyElsewhere, processRunning: true, attached: false, lockHeld: false), "an owned running process refuses rebind")
check(!NodeKeyRebind.canOffer(reason: .keyElsewhere, processRunning: false, attached: true, lockHeld: false), "an attached running node refuses rebind")
check(!NodeKeyRebind.canOffer(reason: .keyElsewhere, processRunning: false, attached: false, lockHeld: true), "another process holding run.lock refuses rebind")
check(NodeKeyRebind.warning(ko: false) == "Only do this if you moved this Mac's node on purpose; running the same keys on two Macs gets the validator slashed.", "the owner sees the full slashing warning")
check(NodeKeyRebind.warning(ko: true).contains("의도적으로") && NodeKeyRebind.warning(ko: true).contains("슬래싱"), "Korean warning explains intentional moves and slashing")
let validator = String(repeating: "ab", count: 32)
let publicEntry = Data(("{\"key\":\"" + validator + "\",\"node\":\"public-node\"}").utf8)
check((try? NodeKeyRebind.validatorAddress(in: publicEntry)) == validator, "rebind reads the validator identity from the public entry")
check((try? NodeKeyRebind.validatorAddress(in: Data("{\"key\":\"wallet-account\"}".utf8))) == nil, "wallet-account-shaped addresses cannot authorize validator rebind")
check(NodeKeyRebind.normalized("0x" + validator.uppercased()) == validator, "hex case and optional 0x describe the same validator bytes")
check(NodeKeyRebind.normalized(validator + "\nother-input") == nil, "a confirmation cannot inject another terminal line")
var ownerCalls = 0
do {
    _ = try NodeKeyRebind.authorize(validatorAddress: validator, typedAddress: String(repeating: "cd", count: 32), dataDirectory: "fixture/node") { _ in ownerCalls += 1 }
    check(false, "a different validator cannot obtain an approval")
} catch NodeKeyRebind.Refusal.confirmationDidNotMatch {} catch { check(false, "wrong-address refusal is typed: \(error)") }
check(ownerCalls == 0, "a mistyped address refuses before owner authentication")
enum OwnerCancelled: Error { case refused }
var approvalAfterCancellation: NodeKeyRebind.Approval?
do {
    approvalAfterCancellation = try NodeKeyRebind.authorize(validatorAddress: validator, typedAddress: validator, dataDirectory: "fixture/node") { _ in
        ownerCalls += 1
        throw OwnerCancelled.refused
    }
    check(false, "cancelled owner authentication cannot approve a rebind")
} catch OwnerCancelled.refused {} catch { check(false, "owner-auth failure is preserved: \(error)") }
check(ownerCalls == 1 && approvalAfterCancellation == nil, "owner authentication failure creates no CLI approval")
var localApprovalMessage = Data()
let approved = try! NodeKeyRebind.authorize(validatorAddress: validator, typedAddress: "0x" + validator.uppercased(), dataDirectory: "fixture/node") { message in
    ownerCalls += 1
    localApprovalMessage = message
}
check(ownerCalls == 2 && approved.validatorAddress == validator, "the correct identity and successful owner authentication approve rebind")
check(approved.dataDirectory == "fixture/node", "owner approval stays bound to its node data directory")
check(approved.confirmationLine == "0x" + validator.uppercased() + "\n", "the terminal receives the address the owner actually typed")
check(String(decoding: localApprovalMessage, as: UTF8.self).hasPrefix("Aether local node key rebind approval v1\nvalidator: " + validator + "\ndata: fixture/node\nchallenge: "), "owner approval is domain-separated and bound to this identity and directory")
for bad in [NodeWatchdog.Failure.database, .handoff, .storage] {
    f = NodeResumeFacts(); f.blocked = bad; f.blockedForSeconds = 100_000
    check(NodeResume.decide(f) == .wait(.needsAttention(bad)), "\(bad) needs a person, even after hours")
}
f = NodeResumeFacts(); f.processRunning = true
check(NodeResume.decide(f) == .keepRunning, "our node runs")
f = NodeResumeFacts(); f.lockHeldByOther = true
check(NodeResume.decide(f) == .start(detach: false), "a held lock with nothing blocked: start (exit 7 then attaches)")
f.lockRefused = true
check(NodeResume.decide(f) == .wait(.otherNodeRunning), "a holder that does not answer: wait and say so, no spawn loop")
f.lockHeldByOther = false
check(NodeResume.decide(f) == .start(detach: false), "the holder let go: start")
f = NodeResumeFacts(); f.launchError = "Permission denied"
check(NodeResume.decide(f) == .start(detach: false), "a launch error is retried every tick")

// MARK: copy: every reason, both languages, says something and never truncates to nothing

let all: [NodeStopReason] = [.switchedOff, .onBattery, .wrongLocation, .noHelper, .migrating, .migrationBlocked("m"),
                             .otherNodeRunning, .diskFull(freeBytes: GiB, resumeBytes: 7 * GiB, volume: nil),
                             .diskMissing(volume: "v"), .diskNoAccess(volume: "v"), .restarting(inSeconds: 3),
                             .crashLoop(.other, retryInSeconds: 300), .needsAttention(.database), .upgradeNeeded,
                             .identityLost, .waitingForMacConfirmation, .keyElsewhere, .launchFailed("e"), .movingStorage(percent: 5)]
var codes = Set<String>()
for r in all {
    codes.insert(r.code)
    for ko in [true, false] {
        let c = r.copy(ko: ko)
        check(!c.title.isEmpty && !c.detail.isEmpty, "\(r.code) has a title and a detail (ko=\(ko))")
        check(c.title.count <= 40, "\(r.code) title fits the sidebar: \(c.title)")
        check((c.action == nil) == (c.actionLabel == nil), "\(r.code) button has a label")
        check(!c.paragraph.lowercased().contains("rpc") && !c.paragraph.contains("exit "), "\(r.code) has no jargon")
        if r.isIncident && r != .identityLost && r != .keyElsewhere && r != .wrongLocation && r != .noHelper {
            check(!c.resume.isEmpty, "\(r.code) says when it resumes (ko=\(ko))")
        }
    }
}
check(codes.count == all.count, "codes are unique")
check(NodeStopReason.crashLoop(.other, retryInSeconds: 540).copy(ko: true).resume == "9분 뒤 저절로 다시 시도해요.", "minutes")
check(NodeStopReason.restarting(inSeconds: 4).copy(ko: false).resume == "Starting in 4 s.", "seconds")
check(!NodeStopReason.onBattery.isIncident && NodeStopReason.otherNodeRunning.isIncident, "incident flags")

// MARK: node-status.log

let at = Date(timeIntervalSince1970: 1_791_000_000)
var facts = NodeResumeFacts(); facts.attached = true; facts.freeBytes = 6 * GiB
let line = NodeStatusLog.line(at: at, event: "wait disk_full", detail: "Storage low", facts: facts)
check(line.hasSuffix("\n") && line.contains("disk_full") && line.contains("attached=true") && line.contains("free=6.0 GB"), "log line: \(line)")
check(line.hasPrefix("2026-"), "UTC ISO time first")
var log = Data()
for i in 0..<5000 { log = NodeStatusLog.appending(log, line: "line \(i) xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n", cap: 4096) }
check(log.count <= 4096, "capped")
let text = String(decoding: log, as: UTF8.self)
check(text.hasSuffix("line 4999 xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n"), "the newest line is kept")
check(text.hasPrefix("line "), "trimmed at a line boundary")
let dir = URL(fileURLWithPath: ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"] ?? NSTemporaryDirectory())
    .appendingPathComponent("node-stop-\(getpid())")
NodeStatusLog.append("x\n", in: dir)
check(!FileManager.default.fileExists(atPath: dir.path), "the log never creates the node folder (it would derail the Aether data move)")
try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
NodeStatusLog.append("a\n", in: dir)
NodeStatusLog.append("b\n", in: dir)
check((try? String(contentsOf: dir.appendingPathComponent("node-status.log"), encoding: .utf8)) == "a\nb\n", "append-only on disk")
try? FileManager.default.removeItem(at: dir)
print("OK node-stop")
