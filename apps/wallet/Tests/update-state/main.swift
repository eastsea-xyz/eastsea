// The update state machine (docs/design/24-self-healing.md row 11, red team
// #11): every transition, the retry schedule per cause, resume after a kill in
// each state, a corrupt record, and a clock that goes backwards. Pure logic,
// no Sparkle, no app:
//   scripts/test-swift-pure.sh   (run update-state)
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

let t0 = Date(timeIntervalSince1970: 1_790_000_000)
let dir = FileManager.default.temporaryDirectory.appendingPathComponent("update-state-\(UUID().uuidString)", isDirectory: true)
try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
func freshRecord() -> URL { dir.appendingPathComponent("\(UUID().uuidString).json") }

/// A movable clock shared with a tracker over its own record (a class, so the
/// tracker's injected `now` sees every move).
final class Clock { var now: Date; init(_ d: Date) { now = d } }
final class Rig {
    let clock: Clock
    var tracker: UpdateTracker
    var now: Date { clock.now }
    init(start: Date = t0) {
        clock = Clock(start)
        tracker = UpdateTracker(recordURL: freshRecord(), now: { [clock] in clock.now })
    }
    init(resuming record: URL, from start: Date) {
        clock = Clock(start)
        tracker = UpdateTracker(recordURL: record, now: { [clock] in clock.now })
    }
    func advance(_ secs: TimeInterval) { clock.now = clock.now.addingTimeInterval(secs) }
}

// A named regression can also run alone to verify its failure before the fix.
func regression(_ name: String, _ body: () throws -> Void) rethrows {
    let selected = CommandLine.arguments.dropFirst().first
    guard selected == nil || selected == name else { return }
    try body()
    print("update-state: \(name) passed")
    if selected == name { exit(0) }
}

try regression("no-update") {
    let current = Rig()
    let noUpdate = NSError(domain: "SUSparkleErrorDomain", code: 1001) // SUNoUpdateError
    for _ in 0..<50 { current.tracker.aborted(error: noUpdate) }
    check(current.tracker.state == .idle, "50 no-update checks leave the tracker idle")
    let record = try JSONSerialization.jsonObject(with: Data(contentsOf: current.tracker.recordURLForTesting!)) as! [String: Any]
    check(record["attempts"] as? Int == 0, "50 no-update checks persist zero attempts")
    check(record["lastCause"] == nil && record["nextRetryAt"] == nil, "no-update clears the failure and retry")
    check(!current.tracker.retryDue() && current.tracker.sentence == nil, "up to date has no retry or failure notice")
    check(Rig(resuming: current.tracker.recordURLForTesting!, from: t0).tracker.state == .idle,
          "no-update remains idle after relaunch")

    let recovering = Rig()
    recovering.tracker.aborted(error: NSError(domain: NSURLErrorDomain, code: NSURLErrorTimedOut))
    recovering.tracker.aborted(error: noUpdate)
    check(recovering.tracker.state == .idle, "a successful current feed clears an earlier URL failure")
    recovering.tracker.aborted(error: NSError(domain: NSURLErrorDomain, code: NSURLErrorTimedOut))
    check(recovering.tracker.state.failedAttempts == 1, "the next URL failure starts at attempt one")

    for code in [4007, 4008] { // SUInstallationCanceledError, SUInstallationAuthorizeLaterError
        let cancelled = Rig()
        cancelled.tracker.found(key: "k1", version: "1.2", build: "34")
        cancelled.tracker.downloading(); cancelled.tracker.verified(); cancelled.tracker.installing()
        cancelled.tracker.aborted(error: NSError(domain: "SUSparkleErrorDomain", code: code))
        check(cancelled.tracker.state == .idle, "user cancellation/deferral \(code) returns to idle")
        let record = try JSONSerialization.jsonObject(with: Data(contentsOf: cancelled.tracker.recordURLForTesting!)) as! [String: Any]
        check(record["attempts"] as? Int == 0, "user cancellation/deferral \(code) clears attempts")
    }

    let refused = Rig()
    refused.tracker.found(key: "k1", version: "1.2", build: "34")
    refused.tracker.refused()
    refused.tracker.aborted(error: noUpdate)
    check(refused.tracker.state == .idle, "no-update clears the notice for a refused item")
    check(!refused.tracker.found(key: "k1", version: "1.2", build: "34"), "no-update cannot unblock a refused item")
    let refusedResume = Rig(resuming: refused.tracker.recordURLForTesting!, from: t0)
    check(!refusedResume.tracker.found(key: "k1", version: "1.2", build: "34"), "the refusal survives idle and relaunch")
    check(refusedResume.tracker.found(key: "k2", version: "1.3", build: "40"), "a different item is still allowed")

    let exhausted = Rig()
    for _ in 0..<3 {
        exhausted.tracker.found(key: "k1", version: "1.2", build: "34")
        exhausted.tracker.downloading(); exhausted.tracker.verified(); exhausted.tracker.installing()
        exhausted.tracker.aborted(networkError: false)
    }
    exhausted.tracker.aborted(error: noUpdate)
    check(!exhausted.tracker.found(key: "k1", version: "1.2", build: "34"), "no-update cannot restart an exhausted install")

    let health = Rig()
    health.tracker.found(key: "k1", version: "1.2", build: "34")
    health.tracker.downloading(); health.tracker.verified(); health.tracker.installing()
    health.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
    let awaiting = health.tracker.state
    health.tracker.aborted(error: noUpdate)
    check(health.tracker.state == awaiting, "a current feed cannot erase an ongoing health check")
    health.advance(UpdateTracker.healthWindow)
    health.tracker.tick()
    let failedHealth = health.tracker.state
    health.tracker.aborted(error: noUpdate)
    check(health.tracker.state == failedHealth, "a current feed cannot erase a recorded health failure")
}

regression("network-cap") {
    let feed = Rig()
    let timeout = NSError(domain: NSURLErrorDomain, code: NSURLErrorTimedOut)
    for attempt in 1...50 {
        feed.tracker.aborted(error: timeout)
        guard case .failed(let cause, let count, let next?) = feed.tracker.state else {
            check(false, "a real URL failure counts as network attempt \(attempt)")
            return
        }
        check(cause == .network && count == attempt, "a real URL failure counts as network attempt \(attempt)")
        check(next.timeIntervalSince(feed.now) == min(60 * pow(2, Double(attempt - 1)), 3600),
              "URL failure \(attempt) backs off exponentially within one hour")
    }
    check(!feed.tracker.retryDue(), "a fresh URL failure waits for backoff")
    feed.advance(3599)
    check(!feed.tracker.retryDue(), "the capped backoff waits until its deadline")
    feed.advance(1)
    check(feed.tracker.retryDue(), "the capped backoff is due at one hour")

    let wrapped = Rig()
    wrapped.tracker.aborted(error: NSError(domain: "SUSparkleErrorDomain", code: 2001,
        userInfo: [NSUnderlyingErrorKey: timeout]))
    check(wrapped.tracker.state == .failed(cause: .network, attempts: 1, nextRetryAt: t0.addingTimeInterval(60)),
          "a Sparkle error wrapping a URL failure still backs off")
    let parse = Rig()
    parse.tracker.aborted(error: NSError(domain: "SUSparkleErrorDomain", code: 1000))
    check(parse.tracker.state == .idle, "a feed parse error creates no network retry or gate refusal")
    let installing = Rig()
    installing.tracker.found(key: "k1", version: "1.2", build: "34")
    installing.tracker.downloading(); installing.tracker.verified(); installing.tracker.installing()
    installing.tracker.aborted(networkError: false)
    let installFailure = installing.tracker.state
    installing.tracker.aborted(error: NSError(domain: "SUSparkleErrorDomain", code: 1000))
    check(installing.tracker.state == installFailure, "a feed parse error preserves the previous install outcome")
    let foreign = Rig()
    foreign.tracker.aborted(error: NSError(domain: NSURLErrorDomain, code: 1001))
    check(foreign.tracker.state.failedCause == .network, "a no-update code in another domain is not benign")
}

try regression("found-after-backoff") {
    // A 0.7.1 record can carry a six-hour feed backoff with no item at all.
    let record = freshRecord()
    try JSONSerialization.data(withJSONObject: ["v": 1, "phase": "failed", "lastCause": "network", "attempts": 30,
        "nextRetryAt": t0.addingTimeInterval(6 * 3600).timeIntervalSinceReferenceDate]).write(to: record)
    let legacy = Rig(resuming: record, from: t0)
    check(legacy.tracker.retryDue(), "an obsolete six-hour backoff cannot delay a fresh feed check")
    check(legacy.tracker.found(key: "k2", version: "1.3", build: "40"), "a newer item after the old backoff proceeds")
    check(legacy.tracker.state == .found(version: "1.3", build: "40"), "the newer item replaces the failed feed state")
    check(!legacy.tracker.retryDue(), "finding the newer item clears the scheduled retry")
    legacy.tracker.downloading()
    check(legacy.tracker.state == .downloading(version: "1.3", build: "40"), "the gate can proceed with the newer item")

    let pending = Rig()
    pending.tracker.aborted(error: NSError(domain: NSURLErrorDomain, code: NSURLErrorTimedOut))
    check(!pending.tracker.retryDue(), "the current backoff has not expired")
    check(pending.tracker.found(key: "k2", version: "1.3", build: "40"), "a successful newer feed overrides backoff immediately")
    pending.tracker.downloading()
    check(pending.tracker.state == .downloading(version: "1.3", build: "40"), "no pending retry delays an available update")
}

// R11: an unverified running-node observation cannot complete update health.
let unidentified = Rig()
_ = unidentified.tracker.found(key: "R11", version: "1.2", build: "34")
unidentified.tracker.downloading()
unidentified.tracker.verified()
unidentified.tracker.installing()
unidentified.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
unidentified.tracker.nodeRunning()
check(unidentified.tracker.state.awaitingHealthVersion == "1.2",
      "R11 unverified or starting node cannot mark an update healthy")

unidentified.tracker.nodeRunning(running: false, releaseVerified: true)
check(unidentified.tracker.state.awaitingHealthVersion == "1.2", "R11 starting verified helper still waits for health")
unidentified.tracker.nodeRunning(running: true, releaseVerified: false)
check(unidentified.tracker.state.awaitingHealthVersion == "1.2", "R11 running old helper still waits for health")

// A missing record means idle — never a crash, never a stale cycle.
let missing = Rig()
check(missing.tracker.state == .idle, "no record: idle")
check(missing.tracker.sentence == nil, "no record: nothing to say")

// The happy path, one event per Sparkle callback: found → downloading →
// verified → installing → (relaunch on the target) → awaitingHealth → healthy.
let happy = Rig()
check(happy.tracker.found(key: "k1", version: "1.2", build: "34") == true, "a new item starts a cycle")
check(happy.tracker.state == UpdateTracker.State.found(version: "1.2", build: "34"), "found carries version and build")
happy.tracker.downloading()
check(happy.tracker.state == .downloading(version: "1.2", build: "34"), "gate passed: downloading")
happy.tracker.verified()
check(happy.tracker.state == .verified(version: "1.2", build: "34"), "download verified")
happy.tracker.installing()
check(happy.tracker.state == .installing(version: "1.2", build: "34"), "install begins")
// Killed right there; the next launch runs the target version.
let resumed = Rig(resuming: happy.tracker.recordURLForTesting!, from: t0.addingTimeInterval(120))
resumed.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
check(resumed.tracker.state == .awaitingHealth(version: "1.2", build: "34", startedAt: t0.addingTimeInterval(120)),
      "kill during install + relaunch on target: awaiting health, clocked from the relaunch")
check(resumed.tracker.sentence == "Updated to 1.2. \(Brand.project) is checking that this Mac's node is healthy…",
      "awaitingHealth: one honest sentence naming the version")
resumed.tracker.nodeRunning(running: true, releaseVerified: true)
check(resumed.tracker.state == .healthy(version: "1.2", build: "34"), "the node came up: healthy")
check(resumed.tracker.sentence == nil, "healthy: nothing to say")
resumed.tracker.tick()
check(resumed.tracker.state == .healthy(version: "1.2", build: "34"), "ticks do not disturb healthy")

// The health window: ten minutes, then the failure is recorded (the node's own
// watchdog and rollback stay the only mechanisms — this only records).
let health = Rig()
health.tracker.found(key: "k1", version: "1.2", build: "34")
health.tracker.downloading(); health.tracker.verified(); health.tracker.installing()
health.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
health.advance(599)
health.tracker.tick()
check(health.tracker.state.awaitingHealthVersion == "1.2", "one second inside the window: still waiting")
health.advance(1)
health.tracker.tick()
check(health.tracker.state == .failed(cause: .health, attempts: 1, nextRetryAt: nil),
      "past the window without a healthy node: failed(health), no scheduled retry")
check(health.tracker.sentence!.contains("node"), "the health sentence says what stayed wrong")
check(!health.tracker.retryDue(), "a health failure schedules no retry of its own")

// Relaunch inside the health window keeps the original startedAt (the window
// does not restart on every launch).
let window = Rig()
window.tracker.found(key: "k1", version: "1.2", build: "34")
window.tracker.downloading(); window.tracker.verified(); window.tracker.installing()
window.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
window.advance(300)
let window2 = Rig(resuming: window.tracker.recordURLForTesting!, from: t0.addingTimeInterval(300))
window2.advance(301)
window2.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
window2.tracker.tick()
check(window2.tracker.state == .failed(cause: .health, attempts: 1, nextRetryAt: nil),
      "the health window is measured from the first relaunch, not the second")

// Relaunched on a different version while awaiting health: the cycle's outcome
// is void; record it, do not pretend it went fine.
let rolled = Rig()
rolled.tracker.found(key: "k1", version: "1.2", build: "34")
rolled.tracker.downloading(); rolled.tracker.verified(); rolled.tracker.installing()
rolled.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
let rolled2 = Rig(resuming: rolled.tracker.recordURLForTesting!, from: t0.addingTimeInterval(60))
rolled2.tracker.relaunched(runningVersion: "1.1", runningBuild: "33")
check(rolled2.tracker.state == .failed(cause: .health, attempts: 1, nextRetryAt: nil),
      "back on an older version while awaiting health: recorded as a health failure")

// Network/download failures: unlimited attempts, backoff 1 min doubling to a
// 6 h cap, never a tight loop.
let net = Rig()
net.tracker.found(key: "k1", version: "1.2", build: "34")
net.tracker.downloading()
net.tracker.aborted(networkError: true)
check(net.tracker.state == .failed(cause: .network, attempts: 1, nextRetryAt: t0.addingTimeInterval(60)),
      "first download failure: retry in a minute")
for (n, expected) in [(2, 120.0), (3, 240.0), (4, 480.0), (5, 960.0), (6, 1920.0), (7, 3840.0), (8, 7680.0), (9, 15360.0), (10, 21600.0), (11, 21600.0)] {
    net.tracker.found(key: "k1", version: "1.2", build: "34")  // Sparkle offers the same item again
    net.tracker.downloading()
    net.tracker.aborted(networkError: true)
    check(net.tracker.state == .failed(cause: .network, attempts: n, nextRetryAt: t0.addingTimeInterval(expected)),
          "network attempt \(n): backoff \(expected) s")
}
check(!net.tracker.retryDue(), "before the retry time: not due")
net.advance(21600)
check(net.tracker.retryDue(), "at the retry time: due")

// A *new* item after download failures starts a fresh count.
net.tracker.found(key: "k2", version: "1.3", build: "40")
net.tracker.downloading()
net.tracker.aborted(networkError: true)
check(net.tracker.state == .failed(cause: .network, attempts: 1, nextRetryAt: net.now.addingTimeInterval(60)),
      "a new item resets the attempt count")

// The approval gate: a refusal is final for that item — no retry, one plain
// sentence; the next appcast item starts clean. If Sparkle later proceeds with
// the same item anyway (the approval landed), that fact supersedes the refusal.
let gate = Rig()
gate.tracker.found(key: "k1", version: "1.2", build: "34")
gate.tracker.refused()
check(gate.tracker.state == .failed(cause: .gate, attempts: 1, nextRetryAt: nil), "refusal: failed(gate), nothing scheduled")
check(gate.tracker.sentence == "This update is not approved by the network. \(Brand.project) left it alone and waits for the next approved release.",
      "refusal: one plain sentence")
check(!gate.tracker.retryDue(), "a refusal schedules no retry")
check(gate.tracker.found(key: "k1", version: "1.2", build: "34") == false, "the same item is not restarted")
check(gate.tracker.state == .failed(cause: .gate, attempts: 1, nextRetryAt: nil), "the refusal state stands")
gate.tracker.downloading()  // the gate passed after all
check(gate.tracker.state == .downloading(version: "1.2", build: "34"), "a later gate pass supersedes the refusal")
check(gate.tracker.found(key: "k2", version: "1.3", build: "40") == true, "a new appcast item starts clean")

let gate2 = Rig()
gate2.tracker.found(key: "k1", version: "1.2", build: "34")
gate2.tracker.refused()
check(gate2.tracker.found(key: "k2", version: "1.3", build: "40") == true, "after a refusal, the next item is a new cycle")
check(gate2.tracker.state == UpdateTracker.State.found(version: "1.3", build: "40"), "the new cycle is on the new item")

// A non-network abort while downloading is a verification refusal: same policy
// as the gate.
let sig = Rig()
sig.tracker.found(key: "k1", version: "1.2", build: "34")
sig.tracker.downloading()
sig.tracker.aborted(networkError: false)
check(sig.tracker.state == .failed(cause: .gate, attempts: 1, nextRetryAt: nil), "signature failure while downloading: gate policy")

// Install failures: at most three attempts with backoff, then stop and say to
// download manually; a new item is still allowed.
func installAttempt(_ rig: Rig, onTarget: Bool) {
    rig.tracker.found(key: "k1", version: "1.2", build: "34")
    rig.tracker.downloading(); rig.tracker.verified(); rig.tracker.installing()
    rig.advance(30)
    rig.tracker.relaunched(runningVersion: onTarget ? "1.2" : "1.1", runningBuild: onTarget ? "34" : "30")
}
let inst = Rig()
installAttempt(inst, onTarget: false)
check(inst.tracker.state == .failed(cause: .install, attempts: 1, nextRetryAt: inst.now.addingTimeInterval(60)),
      "install attempt 1: retry in a minute")
check(!inst.tracker.retryDue(), "not yet due")
installAttempt(inst, onTarget: false)
check(inst.tracker.state.failedCause == .install && inst.tracker.state.failedAttempts == 2, "install attempt 2 counted")
installAttempt(inst, onTarget: false)
check(inst.tracker.state == .failed(cause: .install, attempts: 3, nextRetryAt: nil),
      "install attempt 3: no more retries, the item is blocked")
check(inst.tracker.sentence == "The update could not be installed after 3 tries. Download \(Brand.project) again from its website and replace this app.",
      "given up: the sentence says to download manually")
check(inst.tracker.found(key: "k1", version: "1.2", build: "34") == false, "the given-up item is not restarted")
check(inst.tracker.found(key: "k9", version: "1.4", build: "50") == true, "a later item gets its own three tries")

// An abort during the install phase is the same bucket.
let instAbort = Rig()
instAbort.tracker.found(key: "k1", version: "1.2", build: "34")
instAbort.tracker.downloading(); instAbort.tracker.verified(); instAbort.tracker.installing()
instAbort.tracker.aborted(networkError: true)
check(instAbort.tracker.state == .failed(cause: .install, attempts: 1, nextRetryAt: t0.addingTimeInterval(60)),
      "an abort while installing counts as an install failure, not a network one")

// The attempt count is per cause: a download failure does not eat install tries.
let mixed = Rig()
mixed.tracker.found(key: "k1", version: "1.2", build: "34")
mixed.tracker.downloading()
mixed.tracker.aborted(networkError: true)
mixed.tracker.found(key: "k1", version: "1.2", build: "34")
mixed.tracker.downloading()
mixed.tracker.aborted(networkError: true)
mixed.tracker.found(key: "k1", version: "1.2", build: "34")
mixed.tracker.downloading(); mixed.tracker.verified(); mixed.tracker.installing()
mixed.tracker.aborted(networkError: false)
check(mixed.tracker.state == .failed(cause: .install, attempts: 1, nextRetryAt: mixed.now.addingTimeInterval(60)),
      "two network failures leave the install count untouched")

// Resume after a kill in each state: a second tracker over the same record.
func killAndResume(_ rig: Rig) -> Rig {
    let next = Rig(resuming: rig.tracker.recordURLForTesting!, from: rig.now.addingTimeInterval(600))
    next.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
    return next
}
let killIdle = Rig()
check(killAndResume(killIdle).tracker.state == .idle, "kill at idle: still idle")

let killFound = Rig()
killFound.tracker.found(key: "k1", version: "1.2", build: "34")
check(killAndResume(killFound).tracker.state == UpdateTracker.State.found(version: "1.2", build: "34"),
      "kill while waiting for approval: unchanged (approval is not a failure)")

let killDownload = Rig()
killDownload.tracker.found(key: "k1", version: "1.2", build: "34")
killDownload.tracker.downloading()
let killDownloadNext = killAndResume(killDownload)
check(killDownloadNext.tracker.state == .failed(cause: .network, attempts: 1, nextRetryAt: killDownloadNext.now.addingTimeInterval(60)),
      "kill during download: a recorded download failure with backoff")

let killVerified = Rig()
killVerified.tracker.found(key: "k1", version: "1.2", build: "34")
killVerified.tracker.downloading(); killVerified.tracker.verified()
let killVerifiedNext = killAndResume(killVerified)
check(killVerifiedNext.tracker.state == .failed(cause: .network, attempts: 1, nextRetryAt: killVerifiedNext.now.addingTimeInterval(60)),
      "kill after verify, before install: a recorded failure with backoff")

let killInstallOld = Rig()
killInstallOld.tracker.found(key: "k1", version: "1.2", build: "34")
killInstallOld.tracker.downloading(); killInstallOld.tracker.verified(); killInstallOld.tracker.installing()
let killInstallOldNext = Rig(resuming: killInstallOld.tracker.recordURLForTesting!, from: t0.addingTimeInterval(600))
killInstallOldNext.tracker.relaunched(runningVersion: "1.1", runningBuild: "30")
check(killInstallOldNext.tracker.state == .failed(cause: .install, attempts: 1, nextRetryAt: killInstallOldNext.now.addingTimeInterval(60)),
      "kill during install, relaunch on the old version: an install failure")

let killHealth = Rig()
killHealth.tracker.found(key: "k1", version: "1.2", build: "34")
killHealth.tracker.downloading(); killHealth.tracker.verified(); killHealth.tracker.installing()
killHealth.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
let killHealthNext = Rig(resuming: killHealth.tracker.recordURLForTesting!, from: t0.addingTimeInterval(60))
killHealthNext.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
check(killHealthNext.tracker.state.awaitingHealthVersion == "1.2", "kill while awaiting health: still awaiting, same window")

let killHealthy = Rig()
killHealthy.tracker.found(key: "k1", version: "1.2", build: "34")
killHealthy.tracker.downloading(); killHealthy.tracker.verified(); killHealthy.tracker.installing()
killHealthy.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
killHealthy.tracker.nodeRunning(running: true, releaseVerified: true)
check(killAndResume(killHealthy).tracker.state == .healthy(version: "1.2", build: "34"), "kill at healthy: still healthy")

// A corrupt record is idle, never a crash — and the next event rewrites it.
let corruptURL = freshRecord()
try Data("{ not json".utf8).write(to: corruptURL)
let corrupt = Rig(resuming: corruptURL, from: t0)
check(corrupt.tracker.state == .idle, "a corrupt record reads as idle")
check(corrupt.tracker.found(key: "k1", version: "1.2", build: "34") == true, "a corrupt record does not block the next cycle")
let corruptNext = Rig(resuming: corruptURL, from: t0)
check(corruptNext.tracker.state == UpdateTracker.State.found(version: "1.2", build: "34"), "the rewritten record survives a relaunch")

// An empty record is idle too.
let emptyURL = freshRecord()
try Data().write(to: emptyURL)
let empty = Rig(resuming: emptyURL, from: t0)
check(empty.tracker.state == .idle, "an empty record reads as idle")

// A clock that goes backwards: no timeout fires early, no retry fires early,
// and the backoff math never depends on wall-clock distance.
let back = Rig()
back.tracker.found(key: "k1", version: "1.2", build: "34")
back.tracker.downloading(); back.tracker.verified(); back.tracker.installing()
back.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
back.advance(-3600)  // the clock jumped back an hour
back.tracker.tick()
check(back.tracker.state.awaitingHealthVersion == "1.2", "a negative elapsed window never times out")
let back2 = Rig(resuming: back.tracker.recordURLForTesting!, from: t0.addingTimeInterval(-10_000))
back2.tracker.tick()
check(back2.tracker.state.awaitingHealthVersion == "1.2", "still no timeout while the clock is behind")
back2.advance(10_000 + 601)
back2.tracker.tick()
check(back2.tracker.state.failedCause == .health, "once the clock is honestly past the window, it fires")

let backNet = Rig()
backNet.tracker.found(key: "k1", version: "1.2", build: "34")
backNet.tracker.downloading()
backNet.advance(1000)
backNet.tracker.aborted(networkError: true)
check(backNet.tracker.state == .failed(cause: .network, attempts: 1, nextRetryAt: backNet.now.addingTimeInterval(60)),
      "backoff is scheduled from the (already moved) now, not from t0")
backNet.advance(-5000)
check(!backNet.tracker.retryDue(), "clock behind the failure: no retry")
backNet.tracker.found(key: "k1", version: "1.2", build: "34")
backNet.tracker.downloading()
backNet.tracker.aborted(networkError: true)
check(backNet.tracker.state == .failed(cause: .network, attempts: 2, nextRetryAt: backNet.now.addingTimeInterval(120)),
      "attempt 2 while the clock was behind: backoff doubles by attempt count alone")
backNet.advance(120)
check(backNet.tracker.retryDue(), "and becomes due 120 s after that failure")

// Stale gate answers about an older item never poison a cycle in flight.
let stale = Rig()
stale.tracker.found(key: "k1", version: "1.2", build: "34")
stale.tracker.downloading()
stale.tracker.refused()
check(stale.tracker.state == .downloading(version: "1.2", build: "34"), "a refusal arriving after the gate passed is ignored")

// Found for the same item mid-cycle does not restart anything.
let mid = Rig()
mid.tracker.found(key: "k1", version: "1.2", build: "34")
mid.tracker.downloading()
check(mid.tracker.found(key: "k1", version: "1.2", build: "34") == true, "same item mid-cycle: accepted, no restart")
check(mid.tracker.state == .downloading(version: "1.2", build: "34"), "still downloading the same item")

// Red team #6: a record written while the Mac's clock was wrong. The failure
// was scheduled +30 days out (the clock was 30 days fast); relaunched with an
// honest clock, that "schedule" is clamped to now — the retry is due at once,
// and the persisted attempt count keeps the backoff honest afterwards.
let fastClock = Rig()
fastClock.tracker.found(key: "k1", version: "1.2", build: "34")
fastClock.tracker.downloading()
fastClock.advance(30 * 86_400)
fastClock.tracker.aborted(networkError: true)
let honest = Rig(resuming: fastClock.tracker.recordURLForTesting!, from: t0)
check(honest.tracker.state == .failed(cause: .network, attempts: 1, nextRetryAt: t0),
      "a retry scheduled beyond the largest backoff is clamped to now")
check(honest.tracker.retryDue(), "a clamped retry is due at once")
honest.tracker.found(key: "k1", version: "1.2", build: "34")
honest.tracker.downloading()
honest.tracker.aborted(networkError: true)
check(honest.tracker.state == .failed(cause: .network, attempts: 2, nextRetryAt: t0.addingTimeInterval(120)),
      "the attempt count survived the clamp: the backoff keeps doubling")

// The same clamp for a health window started while the clock was wrong: it
// runs its honest ten minutes from now, not thirty days from now.
let fastHealth = Rig()
fastHealth.tracker.found(key: "k1", version: "1.2", build: "34")
fastHealth.tracker.downloading(); fastHealth.tracker.verified(); fastHealth.tracker.installing()
fastHealth.advance(30 * 86_400)
fastHealth.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
let honestHealth = Rig(resuming: fastHealth.tracker.recordURLForTesting!, from: t0)
check(honestHealth.tracker.state == .awaitingHealth(version: "1.2", build: "34", startedAt: t0),
      "a health window started beyond the largest backoff is clamped to now")
honestHealth.tracker.tick()
check(honestHealth.tracker.state.awaitingHealthVersion == "1.2", "the clamped window does not fire at once")
honestHealth.advance(599)
honestHealth.tracker.tick()
check(honestHealth.tracker.state.awaitingHealthVersion == "1.2", "one second inside the clamped window: still waiting")
honestHealth.advance(1)
honestHealth.tracker.tick()
check(honestHealth.tracker.state.failedCause == .health, "the clamped window ends on time")

// Mid-session: the clock steps back 10 h after a failure was scheduled +60 s
// out — the "retry" now sits 10 h in the future, which no backoff of ours
// could have written. It is due now rather than postponed to a wrong schedule.
let stepBack = Rig()
stepBack.tracker.found(key: "k1", version: "1.2", build: "34")
stepBack.tracker.downloading()
stepBack.tracker.aborted(networkError: true)
check(!stepBack.tracker.retryDue(), "freshly scheduled +60 s: not due yet")
stepBack.advance(-10 * 3600)
check(stepBack.tracker.retryDue(), "a retry 10 h in the future (only a clock artifact) is due now")

// And a startedAt pushed into the future by a backward step never fires the
// window early, but the window still ends once the clock is honestly past it.
let stepBackHealth = Rig()
stepBackHealth.tracker.found(key: "k1", version: "1.2", build: "34")
stepBackHealth.tracker.downloading(); stepBackHealth.tracker.verified(); stepBackHealth.tracker.installing()
stepBackHealth.tracker.relaunched(runningVersion: "1.2", runningBuild: "34")
stepBackHealth.advance(-7200)
stepBackHealth.tracker.tick()
check(stepBackHealth.tracker.state.awaitingHealthVersion == "1.2", "a startedAt pushed into the future by a backward step never fires early")
stepBackHealth.advance(7200 + 600)
stepBackHealth.tracker.tick()
check(stepBackHealth.tracker.state.failedCause == .health, "and the window still ends once honestly past")

print("update-state: all checks passed")
