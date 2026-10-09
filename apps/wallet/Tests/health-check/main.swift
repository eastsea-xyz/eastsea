// Layer 1 of the health signal (docs/design/32-health-signal.md §4.2): every
// table row L1–L9, input → sentence, one alert per incident, one "resolved",
// clock jumps ignored. Pure logic, no app:
//   swiftc -o ./tmp/health-check apps/wallet/Sources/Brand.swift apps/wallet/Sources/Clock.swift apps/wallet/Sources/NodeWatchdog.swift apps/wallet/Sources/HealthCheck.swift apps/wallet/Tests/health-check/main.swift && ./tmp/health-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }
let enLocale = walletTestLocale("en"), enBundle = walletTestBundle("en")
let koLocale = walletTestLocale("ko"), koBundle = walletTestBundle("ko")

typealias O = HealthCheck.Observation
let t0 = MonotonicInstant(1_000)
func at(_ s: TimeInterval) -> MonotonicInstant { t0.advanced(by: s) }

/// Feeds `o` every 2 s (the node poll) from `from` through `to`, collecting events.
func run(_ h: inout HealthCheck, _ o: O, from: TimeInterval, to: TimeInterval, step: TimeInterval = 2) -> [(TimeInterval, HealthCheck.Event)] {
    var out: [(TimeInterval, HealthCheck.Event)] = []
    for s in stride(from: from, through: to, by: step) {
        for e in h.observe(o, at: at(s)) { out.append((s, e)) }
    }
    return out
}
func raises(_ events: [(TimeInterval, HealthCheck.Event)], _ issue: HealthCheck.Issue) -> Int {
    events.filter { $0.1 == .raised(issue) }.count
}
func resolves(_ events: [(TimeInterval, HealthCheck.Event)], _ issue: HealthCheck.Issue) -> Int {
    events.filter { $0.1 == .resolved(issue) }.count
}

// A healthy wallet says nothing.
var calm = HealthCheck()
check(run(&calm, O(), from: 0, to: 600).isEmpty, "a healthy wallet raises nothing")
check(calm.alert(locale: koLocale, bundle: koBundle) == nil && calm.healthyBadgeAllowed, "no banner, the healthy badge may show")

// Every row exists, in the design's numbering.
check(Set(HealthCheck.Issue.allCases.map(\.row)) == Set((1...10).map { "L\($0)" }), "each of L1–L10 has an issue")

// MARK: L1 — proofs rejected / program mismatch

var proving = O()
proving.nodeRunning = true
proving.nodeAnsweredSinceStart = true
proving.proving = true
var mismatch = proving
mismatch.programMismatch = true
mismatch.proofsFailing = true
mismatch.proverPausedForProgram = true
var l1 = HealthCheck()
var ev = run(&l1, mismatch, from: 0, to: 60)
check(raises(ev, .programMismatch) == 1 && ev.first?.0 == 0, "L1: a program mismatch is raised at once, once")
check(raises(ev, .proofsRejected) == 0, "L1: the mismatch is one incident, not also a rejection")
check(l1.alert(locale: koLocale, bundle: koBundle)?.sentence == "이 Mac은 지금 블록 증명을 쉬고 있어요. 네트워크가 이 버전의 증명을 아직 확인하지 못해서예요. 잃는 건 없어요.",
      "L1 without an update: proving rests, plainly, nothing lost")
check(l1.alert(locale: koLocale, bundle: koBundle)?.action == .checkForUpdates, "L1: [업데이트 확인]")
mismatch.updateAvailable = true
_ = l1.observe(mismatch, at: at(62))
check(l1.alert(locale: koLocale, bundle: koBundle)?.sentence == "보상 증명이 거절되고 있어요. 앱을 업데이트하면 해결돼요.", "L1 with an update: update and it is fixed")
check(l1.alert(locale: enLocale, bundle: enBundle)?.sentence == "Your reward proofs are being rejected. Updating the app fixes it.", "L1 in English")
ev = run(&l1, proving, from: 64, to: 120)
check(resolves(ev, .programMismatch) == 1, "L1: the match returning resolves the incident once")
check(ev.first { $0.1 == .resolved(.programMismatch) }!.0 >= 64 + HealthCheck.resolveAfter, "…after the condition stayed gone for resolveAfter")
check(HealthCheck.resolvedSentence(.programMismatch, locale: koLocale, bundle: koBundle).hasPrefix("해결됐어요"), "L1 resolution says 해결됐어요")

var rejected = proving
rejected.proofsFailing = true
var l1b = HealthCheck()
ev = run(&l1b, rejected, from: 0, to: 30)
check(raises(ev, .proofsRejected) == 1, "L1: acceptance below the floor is a rejection")
check(l1b.sentence(.proofsRejected, locale: koLocale, bundle: koBundle).contains("거절"), "L1 rejection sentence")

// L1's cross-check: six hours proving, no new reward, while layer 0 sees other
// proofs paid. Measured on the monotonic clock, restarted by any new reward.
var silent = proving
silent.lastReward = "0x10"
silent.networkProofsRewarded = true
var l1c = HealthCheck()
ev = run(&l1c, silent, from: 0, to: 6 * 3600 - 60, step: 60)
check(raises(ev, .proofsRejected) == 0, "L1 cross-check: under six hours of silence, wait")
ev = run(&l1c, silent, from: 6 * 3600, to: 6 * 3600 + 60, step: 60)
check(raises(ev, .proofsRejected) == 1, "L1 cross-check: six hours without a reward while others are paid")
var unknownNetwork = silent
unknownNetwork.networkProofsRewarded = nil
var l1d = HealthCheck()
check(raises(run(&l1d, unknownNetwork, from: 0, to: 7 * 3600, step: 60), .proofsRejected) == 0,
      "L1 cross-check stays off when the app cannot see the network's rewards")
var l1e = HealthCheck()
var paid = silent
_ = run(&l1e, paid, from: 0, to: 5 * 3600, step: 60)
paid.lastReward = "0x20"
check(raises(run(&l1e, paid, from: 5 * 3600 + 60, to: 10 * 3600, step: 60), .proofsRejected) == 0,
      "L1 cross-check: a new reward restarts the six hours")

// MARK: L2 — stuck on "connecting"

var offlineWallet = O()
offlineWallet.walletConnected = false
offlineWallet.internetReachable = true
var l2 = HealthCheck()
ev = run(&l2, offlineWallet, from: 0, to: 88)
check(ev.isEmpty, "L2: nothing before 90 s")
ev = run(&l2, offlineWallet, from: 90, to: 178)
check(ev.map(\.1) == [.rediscover] && ev[0].0 == 90, "L2: at 90 s, one automatic re-discovery and no banner yet")
ev = run(&l2, offlineWallet, from: 180, to: 400)
check(raises(ev, .connectionStuck) == 1 && ev.first?.0 == 180, "L2: at 3 min, the sentence")
check(!ev.contains { $0.1 == .rediscover }, "L2: the automatic re-discovery happens once per incident")
check(l2.alert(locale: koLocale, bundle: koBundle)?.sentence == "네트워크를 찾지 못하고 있어요. 다시 시도해 볼게요. 잔액은 안전해요.", "L2 sentence says the balance is safe")
check(l2.alert(locale: koLocale, bundle: koBundle)?.action == .retryConnection, "L2: [다시 시도]")
check(l2.retry() == [.rediscover], "L2: [다시 시도] looks again at once")
var online = offlineWallet
online.walletConnected = true
ev = run(&l2, online, from: 402, to: 460)
check(resolves(ev, .connectionStuck) == 1, "L2: connecting resolves it, once")
check(HealthCheck.resolvedSentence(.connectionStuck, locale: koLocale, bundle: koBundle) == "해결됐어요. 네트워크에 다시 연결됐어요.", "L2 resolution")

// No internet at all: no pointless re-discovery, and the sentence says it
// reconnects by itself.
var noInternet = offlineWallet
noInternet.internetReachable = false
var l2b = HealthCheck()
ev = run(&l2b, noInternet, from: 0, to: 300)
check(!ev.contains { $0.1 == .rediscover }, "L2: no re-discovery without internet")
check(l2b.alert(locale: koLocale, bundle: koBundle)?.sentence == "인터넷에 연결되지 않았어요. 연결되면 저절로 이어져요.", "L2: the no-internet sentence")
check(l2b.alert(locale: koLocale, bundle: koBundle)?.action == nil, "L2: nothing to press without internet")

// W3: right after a reboot the network attaches late — the internet comes up
// at 40 s and the validators answer at 150 s. One re-discovery at 90 s, no
// banner, no notification, nothing to resolve.
var l2c = HealthCheck()
ev = run(&l2c, noInternet, from: 0, to: 38)
ev += run(&l2c, offlineWallet, from: 40, to: 148)
ev += run(&l2c, online, from: 150, to: 400)
check(ev.map(\.1) == [.rediscover], "reboot with a late network: one re-discovery at 90 s and nothing else")

// MARK: L3 / L4 — disk

var almost = O()
almost.nodeRunning = true
almost.diskAlmostFull = true
var l3 = HealthCheck()
ev = run(&l3, almost, from: 0, to: 30)
check(raises(ev, .diskAlmostFull) == 1, "L3: the 3 GB warning")
check(l3.alert(locale: koLocale, bundle: koBundle)?.sentence == "저장 공간이 곧 부족해요. 3 GB쯤 비워 주세요.", "L3 warning sentence")
check(l3.alert(locale: koLocale, bundle: koBundle)?.action == .openStorage, "L3: [저장 공간 관리 열기]")
check(l3.healthyBadgeAllowed, "L3's warning alone does not hide the badge")

var halfDead = almost
halfDead.diskPaused = true
halfDead.nodeResponsive = true  // the RPC still answers: half dead
ev = run(&l3, halfDead, from: 32, to: 80)
check(raises(ev, .diskPaused) == 1, "L3/L4: the pause is raised once")
check(l3.alert(locale: koLocale, bundle: koBundle)?.issue == .diskPaused, "L4 outranks the warning in the banner")
check(l3.alert(locale: koLocale, bundle: koBundle)?.sentence == "저장 공간이 부족해 네트워크 참여를 잠시 멈췄어요. 남은 공간이 7 GB가 되면 저절로 다시 시작해요.", "L3 pause sentence (the node's 7 GB resume level)")
check(!l3.healthyBadgeAllowed, "W2: never a healthy badge while L4 holds")
check(!HealthCheck.healthyBadgeAllowed(halfDead), "W2: disk_low → no Verified/정상 badge, from the observation itself")
var voter = halfDead
voter.voting = true
_ = l3.observe(voter, at: at(82))
check(l3.alert(locale: koLocale, bundle: koBundle)?.sentence.hasSuffix("투표 노드라서 다른 검증자들이 기다리고 있어요.") == true, "L4 on a voting node adds the waiting validators")
check(l3.alert(locale: enLocale, bundle: enBundle)?.sentence.contains("voting node") == true, "L4 voting sentence in English")
check(resolves(ev, .diskAlmostFull) == 0, "the warning growing into the pause is no 'resolved'")
// Freed some space: above the floor, still short — the incident steps back
// to the warning without a word; freed enough: one resolution in total.
ev = run(&l3, almost, from: 84, to: 140)
check(ev.isEmpty && l3.alert(locale: koLocale, bundle: koBundle)?.issue == .diskAlmostFull, "pause → warning: quietly back to the warning banner")
check(l3.healthyBadgeAllowed, "the badge may return once the pause is over")
ev = run(&l3, O(), from: 142, to: 200)
check(ev.map(\.1) == [.resolved(.diskAlmostFull)], "space freed: exactly one resolution for the whole disk incident")

// ENOSPC: the watchdog stopped on a full disk — the same row, same badge rule.
var enospc = O()
enospc.stopped = .diskFull
var l4 = HealthCheck()
ev = run(&l4, enospc, from: 0, to: 10)
check(raises(ev, .diskPaused) == 1 && raises(ev, .crashLoop) == 0, "L4: a full-disk stop is the disk row, not L7")
check(!l4.healthyBadgeAllowed, "L4: no healthy badge after ENOSPC")
check(HealthCheck.Issue.diskPaused.failureKind == "disk_floor_pause", "L4 counts as disk_floor_pause")

// MARK: L5 — follower stuck

var running = O()
running.nodeRunning = true
running.nodeAnsweredSinceStart = true
var l5 = HealthCheck()
_ = l5.observe(running, at: at(0))
l5.stallRestarted(at: at(10))
ev = run(&l5, running, from: 12, to: 100)
check(raises(ev, .followerStuck) == 1, "L5: a stall restart is raised once")
check(l5.alert(locale: koLocale, bundle: koBundle)?.sentence == "이 Mac이 네트워크를 따라가지 못해 다시 시작했어요.", "L5 first sentence")
l5.stallRestarted(at: at(300))
ev = run(&l5, running, from: 302, to: 400)
check(raises(ev, .followerStuck) == 0, "L5: the second restart is the same incident — no new notification")
check(l5.alert(locale: koLocale, bundle: koBundle)?.sentence == "계속 멈춰요. 앱을 업데이트하거나 Mac을 재시동해 주세요. 그동안 잔액은 다른 노드로 확인해요.", "L5 repeated sentence")
ev = run(&l5, running, from: 402, to: 1_000)
check(resolves(ev, .followerStuck) == 1, "L5: ten quiet minutes resolve it")

// MARK: L6 — local RPC silent

var silentRpc = running
silentRpc.nodeResponsive = false
var l6 = HealthCheck()
ev = run(&l6, silentRpc, from: 0, to: 28)
check(ev.isEmpty, "L6: under 30 s of silence, wait")
ev = run(&l6, silentRpc, from: 30, to: 200)
check(ev.map(\.1) == [.raised(.nodeUnresponsive), .restartNode], "L6: at 30 s one sentence and one restart")
check(l6.alert(locale: koLocale, bundle: koBundle)?.sentence == "이 Mac의 노드가 응답하지 않아 다시 시작할게요.", "L6 sentence")
var starting = silentRpc
starting.nodeAnsweredSinceStart = false
var l6b = HealthCheck()
check(run(&l6b, starting, from: 0, to: 300).isEmpty, "L6: a node still starting up is not an unanswering one")

// MARK: L7 — crash loop

var crashed = O()
crashed.stopped = .memory
var l7 = HealthCheck()
ev = run(&l7, crashed, from: 0, to: 20)
check(raises(ev, .crashLoop) == 1, "L7: the watchdog's stop is raised once")
check(l7.alert(locale: enLocale, bundle: enBundle)?.sentence == NodeWatchdog.Failure.memory.sentence(locale: enLocale, bundle: enBundle), "L7 reuses layer 4's sentence")
check(l7.alert(locale: enLocale, bundle: enBundle)?.action == .copyDiagnostics, "L7: [진단 정보 복사]")

// MARK: L8 — update required

var outdated = O()
outdated.upgradeRequired = true
var l8 = HealthCheck()
ev = run(&l8, outdated, from: 0, to: 20)
check(raises(ev, .upgradeRequired) == 1, "L8: raised once")
check(l8.alert(locale: koLocale, bundle: koBundle)?.sentence == "네트워크 규칙이 바뀌어 업데이트가 필요해요. 업데이트 전까지 잔액은 다른 노드로 확인해요.", "L8 sentence")
var exit3 = O()
exit3.stopped = .upgradeNeeded
var l8b = HealthCheck()
ev = run(&l8b, exit3, from: 0, to: 20)
check(raises(ev, .upgradeRequired) == 1 && raises(ev, .crashLoop) == 0, "L8: exit code 3 is the update row, not L7")

// MARK: L9 — rolled back after an update

var rolled = O()
rolled.rolledBack = true
var l9 = HealthCheck()
ev = run(&l9, rolled, from: 0, to: 20)
check(raises(ev, .updateRolledBack) == 1, "L9: raised once")
check(l9.alert(locale: koLocale, bundle: koBundle)?.sentence == "새 버전에서 노드가 잘 돌지 않아 이전 버전으로 돌아갔어요. 고친 버전이 오면 저절로 받아요.", "L9 sentence")

// Every row has an English sentence too, and no jargon in either language.
var all = O()
all.proving = true; all.programMismatch = true; all.walletConnected = false; all.diskPaused = true
all.nodeRunning = true; all.upgradeRequired = true; all.rolledBack = true
var words = HealthCheck()
_ = words.observe(all, at: at(0))
for issue in HealthCheck.Issue.allCases {
    for language in ["en", "ko", "ja"] {
        let s = words.sentence(issue, locale: walletTestLocale(language), bundle: walletTestBundle(language))
        check(!s.isEmpty, "\(issue) has a sentence (language: \(language))")
        for jargon in ["guest", "ENOSPC", "RPC", "enospc"] {
            check(!s.contains(jargon), "\(issue) avoids \(jargon) (language: \(language))")
        }
        check(!HealthCheck.resolvedSentence(issue, locale: walletTestLocale(language), bundle: walletTestBundle(language)).isEmpty, "\(issue) has a resolution sentence")
    }
}
check(words.sentence(.connectionStuck, locale: enLocale, bundle: enBundle).contains("balance is safe"), "English L2 says the balance is safe")

// L1, the 0.7.0 case (docs .claude/team/prover-070-mismatch.md C): the node
// cannot even confirm the validators' program (program_unknown) and pauses —
// no mismatch flag, no failing proofs. That pause is an incident too, with
// the same plain sentence; program ids never appear outside developer mode.
var unknownProgram = proving
unknownProgram.proverPausedForProgram = true
var l1u = HealthCheck()
ev = run(&l1u, unknownProgram, from: 0, to: 30)
check(raises(ev, .programMismatch) == 1, "L1: a pause for an unconfirmed program raises the alert")
check(l1u.alert(locale: enLocale, bundle: enBundle)?.sentence == "This Mac is resting from proving blocks for now: the network cannot check this version's proofs yet. Nothing is lost.",
      "L1 unknown in English: \(l1u.alert(locale: enLocale, bundle: enBundle)?.sentence ?? "nil")")

// MARK: L10 — the node is not running, and why (NodeStopReason)

// The founder's 0.7.0 report: the node sat "paused" for hours and nothing
// said why. Any incident-grade stop reason is raised once, with the reason's
// own sentence (what happened, what to do, when it resumes) and its button.
var stoppedNode = O()
stoppedNode.nodeStop = .otherNodeRunning
var l10 = HealthCheck()
ev = run(&l10, stoppedNode, from: 0, to: 60)
check(raises(ev, .nodeStopped) == 1, "L10: a stopped node is raised once")
check(l10.alert(locale: koLocale, bundle: koBundle)?.sentence == NodeStopReason.otherNodeRunning.copy(locale: koLocale, bundle: koBundle).paragraph, "L10 speaks the reason's own words")
check(l10.alert(locale: koLocale, bundle: koBundle)?.action == .fixNode, "L10's button is the reason's")
ev = run(&l10, O(), from: 62, to: 120)
check(resolves(ev, .nodeStopped) == 1, "L10 resolves when the node runs again")
var resting = O()
resting.nodeStop = .onBattery
var l10b = HealthCheck()
check(raises(run(&l10b, resting, from: 0, to: 60), .nodeStopped) == 0, "waiting for power is said, not raised")
var gone = O()
gone.nodeStop = .diskMissing(volume: "Archive")
var l10c = HealthCheck()
_ = run(&l10c, gone, from: 0, to: 10)
check(l10c.alert(locale: koLocale, bundle: koBundle)?.sentence.contains("‘Archive’") == false && l10c.alert(locale: koLocale, bundle: koBundle)?.sentence.contains("연결하면 저절로") == true,
      "a missing disk says it resumes when connected: \(l10c.alert(locale: koLocale, bundle: koBundle)?.sentence ?? "nil")")
// The crash loop and the disk pause speak the reason's exact words too.
var looping = O()
looping.stopped = .other
looping.nodeStop = .crashLoop(.other, retryInSeconds: 540)
var l7s = HealthCheck()
_ = run(&l7s, looping, from: 0, to: 10)
check(l7s.alert(locale: koLocale, bundle: koBundle)?.issue == .crashLoop && l7s.alert(locale: koLocale, bundle: koBundle)?.sentence.contains("9분 뒤") == true,
      "L7 says when it retries: \(l7s.alert(locale: koLocale, bundle: koBundle)?.sentence ?? "nil")")
var low = O()
low.diskPaused = true
low.nodeStop = .diskFull(freeBytes: 6 * 1_073_741_824, resumeBytes: 7 * 1_073_741_824, volume: nil)
var l4s = HealthCheck()
_ = run(&l4s, low, from: 0, to: 10)
check(l4s.alert(locale: koLocale, bundle: koBundle)?.sentence.contains("6.0 GB 남음") == true, "L4 gives the exact numbers: \(l4s.alert(locale: koLocale, bundle: koBundle)?.sentence ?? "nil")")

// MARK: one alert per incident

// A flapping condition (disk paused for 4 s, clear for 4 s, …) is one
// incident: the resolution needs `resolveAfter` of quiet.
var flap = HealthCheck()
var flapEvents: [(TimeInterval, HealthCheck.Event)] = []
for i in 0..<60 {
    let o = i % 4 < 2 ? halfDead : O()
    for e in flap.observe(o, at: at(Double(i) * 2)) { flapEvents.append((Double(i) * 2, e)) }
}
check(raises(flapEvents, .diskPaused) == 1 && resolves(flapEvents, .diskPaused) == 0, "flapping: one alert, no resolution spam")
// After a real resolution a new incident is a new alert.
flapEvents = run(&flap, O(), from: 120, to: 200)
check(resolves(flapEvents, .diskPaused) == 1, "steady quiet resolves it")
flapEvents = run(&flap, halfDead, from: 202, to: 220)
check(raises(flapEvents, .diskPaused) == 1, "a new incident after the resolution is raised again")

// MARK: clock jumps

// The type is the first guarantee: observe(_:at:) takes a MonotonicInstant,
// which a wall-clock Date cannot be passed as. Beyond that, time the app did
// not observe never counts as "held": a gap longer than maxGap (the app was
// suspended) restarts every pending hold, and a sleep/wake does the same.
var jump = HealthCheck()
ev = run(&jump, offlineWallet, from: 0, to: 60)
ev += run(&jump, offlineWallet, from: 60 + 3_600, to: 60 + 3_600 + 60)
check(ev.isEmpty, "a one-hour gap between observations neither re-discovers nor alerts at once")
ev = run(&jump, offlineWallet, from: 3_722, to: 3_660 + 180)
check(raises(ev, .connectionStuck) == 1, "…the hold restarts and the alert comes after 3 observed minutes")
var wake = HealthCheck()
_ = run(&wake, offlineWallet, from: 0, to: 170)
wake.invalidate()
ev = run(&wake, offlineWallet, from: 172, to: 200)
check(ev.isEmpty, "a sleep/wake restarts the 3-minute hold (no alert 10 s after waking)")
var stayed = HealthCheck()
_ = run(&stayed, halfDead, from: 0, to: 10)
stayed.invalidate()
check(run(&stayed, halfDead, from: 12, to: 60).isEmpty, "a raised incident survives a wake without a second alert")
// A suspension in the middle of the L1 six-hour cross-check does not count.
var l1jump = HealthCheck()
ev = run(&l1jump, silent, from: 0, to: 3 * 3600, step: 60)
ev += run(&l1jump, silent, from: 3 * 3600 + 4 * 3600, to: 3 * 3600 + 4 * 3600 + 600, step: 60)
check(raises(ev, .proofsRejected) == 0, "an unobserved gap does not complete the six-hour silence")

// MARK: diagnostics counts

var counts = HealthCheck()
_ = run(&counts, mismatch, from: 0, to: 10)
_ = run(&counts, proving, from: 12, to: 60)
_ = run(&counts, mismatch, from: 62, to: 70)
_ = run(&counts, silentRpc, from: 72, to: 110)
check(counts.failureCounts()["prover_program_mismatch"] == 2, "two mismatch incidents counted")
check(counts.failureCounts()["rpc_unreachable"] == 1, "the silent node counted as rpc_unreachable")
check(counts.failureCounts()["disk_floor_pause"] == nil, "nothing counted that did not happen")

// MARK: L1 — the stalled prover (2026-10-06 incident)

// The node answered `sidecar: Broken pipe (os error 32)` for two hours while
// the lag grew to 6,657 blocks and rewards stopped. The node's own error,
// with the chain moving ahead of it, is a stall the person must hear about —
// after ten minutes, not two hours, and only while the lag actually grows.
var stalled = proving
stalled.proverError = true
stalled.proverLag = 120
var l1stall = HealthCheck()
_ = l1stall.observe(stalled, at: at(0))
stalled.proverLag = 6_657
ev = run(&l1stall, stalled, from: 2, to: 602)
check(raises(ev, .proverStalled) == 1, "L1: a proving error with the lag growing is a stall, raised once")
check(ev.first { $0.1 == .raised(.proverStalled) }!.0 >= HealthCheck.proverStallAfter, "…after ten minutes of it")
check(l1stall.alert(locale: koLocale, bundle: koBundle)?.sentence == "보상 증명이 멈춰 있어요. 이 Mac이 저절로 다시 시도하고 있어요. 오래 계속되면 앱을 다시 열어 주세요.",
      "the stalled sentence says it plainly")
check(l1stall.alert(locale: enLocale, bundle: enBundle)?.sentence == "Reward proofs have stopped. This Mac is retrying by itself; if it keeps up, please reopen the app.",
      "the English sentence too")
check(l1stall.action(.proverStalled) == .copyDiagnostics, "L1 stall: [진단 복사]")
check(l1stall.failureCounts()["prover_stalled"] == 1, "the stall counts as its own failure kind")
// The node recovered (its restart cleared the error): resolved once.
stalled.proverError = false
ev = run(&l1stall, stalled, from: 604, to: 640)
check(resolves(ev, .proverStalled) == 1, "L1: the error clearing resolves the stall")
check(HealthCheck.resolvedSentence(.proverStalled, locale: koLocale, bundle: koBundle).hasPrefix("해결됐어요"), "L1 stall resolution says 해결됐어요")
// An error without a growing lag is not a stall (a quiet chain, or a retry
// about to clear it)…
var frozen = HealthCheck()
var quiet = stalled
quiet.proverError = true
quiet.proverLag = 300
check(run(&frozen, quiet, from: 0, to: 700).filter { $0.1 == .raised(.proverStalled) }.isEmpty,
      "an error with a frozen lag is not a stall")
// …and a proof that landed (the lag fell) starts the count over.
var recovered = HealthCheck()
_ = recovered.observe(quiet, at: at(0))
_ = recovered.observe(quiet, at: at(2))
quiet.proverLag = 100
_ = recovered.observe(quiet, at: at(4))
quiet.proverLag = 200
ev = run(&recovered, quiet, from: 6, to: 700)
check(raises(ev, .proverStalled) == 1, "…but the stall counts again from the proof that landed")

// MARK: diagnostics only while the displayed incident still holds

// The banner intentionally waits for 20 quiet seconds. Neither a secondary
// copy button nor an old primary copy action may linger through that wait.
var diagnosticIncident = HealthCheck()
check(diagnosticIncident.alert() == nil, "idle: no main-UI diagnostics")
_ = diagnosticIncident.observe(crashed, at: at(0))
check(diagnosticIncident.alert()?.diagnosticsVisible == true
      && diagnosticIncident.alert()?.action == .copyDiagnostics
      && diagnosticIncident.alert()?.showsSeparateDiagnostics == false,
      "unresolved crash: one primary diagnostics action")
check(diagnosticIncident.observe(O(), at: at(1)).isEmpty, "first healthy observation starts the quiet period")
check(diagnosticIncident.alert() != nil && diagnosticIncident.alert()?.diagnosticsVisible == false
      && diagnosticIncident.alert()?.action == nil && diagnosticIncident.alert()?.showsSeparateDiagnostics == false,
      "first healthy observation: diagnostics disappear while the banner remains")
check(diagnosticIncident.observe(O(), at: at(20)).isEmpty && diagnosticIncident.alert() != nil,
      "19 quiet seconds: the banner remains, without diagnostics")
check(diagnosticIncident.observe(O(), at: at(21)) == [.resolved(.crashLoop)] && diagnosticIncident.alert() == nil,
      "20 quiet seconds: the existing resolution boundary removes the banner")

var recurringDiagnostics = HealthCheck()
_ = recurringDiagnostics.observe(crashed, at: at(0))
_ = recurringDiagnostics.observe(O(), at: at(1))
check(recurringDiagnostics.observe(crashed, at: at(10)).isEmpty
      && recurringDiagnostics.alert()?.diagnosticsVisible == true
      && recurringDiagnostics.alert()?.action == .copyDiagnostics,
      "a recurring fault restores diagnostics without a second incident notification")
_ = recurringDiagnostics.observe(O(), at: at(11))
check(recurringDiagnostics.observe(O(), at: at(30)).isEmpty && recurringDiagnostics.alert() != nil,
      "recurrence restarts the quiet period")
check(recurringDiagnostics.observe(O(), at: at(31)) == [.resolved(.crashLoop)] && recurringDiagnostics.alert() == nil,
      "the recurring incident resolves after 20 fresh quiet seconds")

var userStopped = O()
userStopped.nodeStop = .switchedOff
var stoppedDiagnostics = HealthCheck()
_ = stoppedDiagnostics.observe(crashed, at: at(0))
_ = stoppedDiagnostics.observe(userStopped, at: at(1))
check(stoppedDiagnostics.alert()?.diagnosticsVisible == false && stoppedDiagnostics.alert()?.action == nil,
      "the user switching off the node hides an old crash's diagnostics immediately")
var expectedStop = HealthCheck()
check(run(&expectedStop, userStopped, from: 0, to: 30).isEmpty && expectedStop.alert() == nil,
      "an ordinary off node is not an incident")

var missingIdentity = O()
missingIdentity.nodeStop = .identityLost
var identityDiagnostics = HealthCheck()
_ = identityDiagnostics.observe(missingIdentity, at: at(0))
check(identityDiagnostics.alert()?.action == .fixNode
      && identityDiagnostics.alert()?.primaryActionCopiesDiagnostics == true
      && identityDiagnostics.alert()?.diagnosticsVisible == true
      && identityDiagnostics.alert()?.showsSeparateDiagnostics == false,
      "the node reason's primary diagnostics action has no duplicate secondary button")
_ = identityDiagnostics.observe(O(), at: at(1))
check(identityDiagnostics.alert()?.diagnosticsVisible == false
      && identityDiagnostics.alert()?.showsSeparateDiagnostics == false && identityDiagnostics.alert()?.action == nil,
      "a repaired identity leaves no diagnostics or stale node action beside the held banner")

var retainedFixDiagnostics = HealthCheck()
var identityCrash = missingIdentity
identityCrash.stopped = .identityLost
_ = retainedFixDiagnostics.observe(identityCrash, at: at(0))
check(retainedFixDiagnostics.alert()?.action == .fixNode
      && retainedFixDiagnostics.alert()?.primaryActionCopiesDiagnostics == true,
      "a crash can use the node reason's primary diagnostics action")
_ = retainedFixDiagnostics.observe(missingIdentity, at: at(1))
check(retainedFixDiagnostics.alert()?.issue == .crashLoop
      && retainedFixDiagnostics.alert()?.diagnosticsVisible == false
      && retainedFixDiagnostics.alert()?.action == nil,
      "an old crash's .fixNode diagnostics action is hidden as soon as that crash condition clears")

var storageDiagnostics = HealthCheck()
_ = storageDiagnostics.observe(halfDead, at: at(0))
check(storageDiagnostics.alert()?.action == .openStorage
      && storageDiagnostics.alert()?.showsSeparateDiagnostics == true,
      "an unresolved storage incident retains diagnostics beside its recovery action")
_ = storageDiagnostics.observe(O(), at: at(1))
check(storageDiagnostics.alert()?.showsSeparateDiagnostics == false,
      "a cleared storage incident hides the secondary diagnostic action immediately")

var proverDiagnostics = HealthCheck()
var proverFault = proving
proverFault.proverError = true
proverFault.proverLag = 1
_ = proverDiagnostics.observe(proverFault, at: at(0))
proverFault.proverLag = 2
_ = run(&proverDiagnostics, proverFault, from: 2, to: 602)
check(proverDiagnostics.alert()?.diagnosticsVisible == true && proverDiagnostics.alert()?.action == .copyDiagnostics,
      "a stalled prover retains its primary diagnostics action")
proverFault.proverError = false
_ = proverDiagnostics.observe(proverFault, at: at(604))
check(proverDiagnostics.alert() != nil && proverDiagnostics.alert()?.diagnosticsVisible == false
      && proverDiagnostics.alert()?.action == nil,
      "a recovered prover's old primary diagnostics action disappears before the banner")

print("health-check: all checks passed")
