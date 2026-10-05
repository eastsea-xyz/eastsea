// The watchdog's decisions (docs/design/24-self-healing.md layer 2), fed the
// events NodeController sees. Pure logic, no app:
//   swiftc -o ./tmp/watchdog-check apps/wallet/Sources/Brand.swift apps/wallet/Sources/Clock.swift apps/wallet/Sources/NodeWatchdog.swift apps/wallet/Tests/watchdog/main.swift && ./tmp/watchdog-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }
check(Brand.project == "EastSea" && Brand.projectKo == "동해", "project names are localized")
check(Brand.coinName == "Doubloon" && Brand.coinTicker == "DBLN", "coin name is the current brand")
check(["EastSea", "동해"].contains { NodeWatchdog.Failure.alreadyRunning.sentence.contains($0) },
      "another running app is named by the current brand")

// Red team #6: every event below is a MonotonicInstant — the type a wall-clock
// `Date` cannot be passed as (this file compiling is that guarantee). The
// arithmetic itself can never run backward either:
check(MonotonicInstant(10).elapsed(since: MonotonicInstant(4)) == 6, "elapsed is the difference")
check(MonotonicInstant(4).elapsed(since: MonotonicInstant(4)) == 0, "no time passes between equal instants")
check(MonotonicInstant(4).elapsed(since: MonotonicInstant(10)) == -6, "elapsed(since:) may be negative only if the caller passes a later instant first")
let readA = UptimeClock().now
let readB = UptimeClock().now
check(readB.elapsed(since: readA) >= 0, "the production clock's readings never move backward")

let t0 = MonotonicInstant(0)

// A single death restarts after a second of backoff, and the backoff doubles
// per death inside the window.
var w = NodeWatchdog()
w.started(t0)
check(w.exited(t0.advanced(by: 60), code: 1) == .restart(after: 1), "first death: restart after 1 s")
check(w.exited(t0.advanced(by: 120), code: 1) == .restart(after: 2), "second death: 2 s")
check(w.exited(t0.advanced(by: 180), code: 1) == .restart(after: 4), "third death: 4 s (third restart)")

// The fourth death inside ten minutes stops the loop with the failure's sentence.
let stopped = w.exited(t0.advanced(by: 240), code: 4, log: "storage failed: Io(\"No space left on device\")\nthe store database did not recover; exiting")
check(stopped == .stop(.diskFull), "fourth death inside 10 min: stop with the disk-full sentence")
var full = NodeWatchdog()
full.started(t0)
check(full.exited(t0.advanced(by: 3), code: 4, log: "No space left on device") == .stop(.diskFull), "a persistent full disk stops after the first exhausted recovery")
check(!NodeWatchdog.storageRecovered(freeBytes: nil), "unknown free space never resumes a storage crash loop")
check(!NodeWatchdog.storageRecovered(freeBytes: NodeWatchdog.diskResumeBytes - 1), "one byte below the resume threshold stays stopped")
check(NodeWatchdog.storageRecovered(freeBytes: NodeWatchdog.diskResumeBytes), "a restored 10 GB reserve permits a restart")
if case let .stop(f) = stopped {
    check(f.sentence.contains("10"), "the sentence says what to do (free space)")
}

// Deaths older than ten minutes do not count toward the stop.
var old = NodeWatchdog()
old.started(t0)
_ = old.exited(t0.advanced(by: 10), code: 1)
_ = old.exited(t0.advanced(by: 20), code: 1)
_ = old.exited(t0.advanced(by: 30), code: 1)
check(old.exited(t0.advanced(by: 700), code: 1) == .restart(after: 1), "a death after the window starts over")

// A stalled node: the network moves, ours does not, for over a minute → one
// restart; a node that still does not move after it does not restart again
// (no restart loop of its own), and progress clears the freeze.
var s = NodeWatchdog()
s.started(t0)
for secs in stride(from: 2.0, through: 62, by: 2) {
    check(s.polled(t0.advanced(by: secs), local: 100, network: 400) == .none, "frozen \(secs) s: wait")
}
check(s.polled(t0.advanced(by: 66), local: 100, network: 400) == .restart(after: 0), "frozen past 60 s while the network moves: restart")
s.restarting()
check(s.polled(t0.advanced(by: 68), local: 100, network: 400) == .none, "still frozen right after the restart: wait, do not loop")
check(s.polled(t0.advanced(by: 200), local: 101, network: 400) == .none, "progress clears the freeze")
check(s.polled(t0.advanced(by: 300), local: 101, network: 400) == .none, "a fresh freeze needs its own 60 s")

// Red team #6: a wall-clock jump can neither fabricate nor hide the stall.
// The scenario the old `Date()`-fed watchdog failed: 58 s frozen, then the
// Mac's clock steps forward two hours (a manual change, an NTP step, DST) —
// under the wall clock the freeze looked 2 h old *at once*. The monotonic
// polls the watchdog actually receives still say 58 s, so nothing restarts a
// healthy validator; only genuine awake time reaching 60 s does. And the
// mirror case — the clock steps back two hours mid-freeze — cannot hide the
// seconds that truly passed.
var jump = NodeWatchdog()
jump.started(t0)
for secs in stride(from: 2.0, through: 58, by: 2) {
    check(jump.polled(t0.advanced(by: secs), local: 100, network: 400) == .none, "frozen \(secs) s: wait")
}
check(jump.polled(t0.advanced(by: 59), local: 100, network: 400) == .none, "a +2 h wall-clock jump after 59 monotonic seconds still waits")
check(jump.polled(t0.advanced(by: 65), local: 100, network: 400) == .restart(after: 0), "61 genuine monotonic seconds of freeze restart even if the wall clock stepped back")
// The same for the crash window and the quick-exit streak: only monotonic
// distance between starts and exits counts, never the wall clock's reading.
var jumpCrash = NodeWatchdog()
jumpCrash.started(t0)
_ = jumpCrash.exited(t0.advanced(by: 30), code: 1)
_ = jumpCrash.exited(t0.advanced(by: 60), code: 1)
_ = jumpCrash.exited(t0.advanced(by: 90), code: 1)
check(jumpCrash.exited(t0.advanced(by: 120), code: 1) == .stop(.other), "the crash window measures monotonic time, not the wall clock")

// A network that is itself paused is not the node's stall.
var paused = NodeWatchdog()
paused.started(t0)
for secs in stride(from: 2.0, through: 120, by: 2) {
    check(paused.polled(t0.advanced(by: secs), local: 100, network: 101) == .none, "network frozen too: never a node restart")
}

// A node that died almost immediately three times in a row: run the previous
// binary (an update that cannot start), instead of restarting this one again.
var up = NodeWatchdog()
up.started(t0)
check(up.exited(t0.advanced(by: 5), code: 1) == .restart(after: 1), "quick death 1: restart")
up.started(t0.advanced(by: 20))
check(up.exited(t0.advanced(by: 23), code: 1) == .restart(after: 2), "quick death 2: restart")
up.started(t0.advanced(by: 50))
check(up.exited(t0.advanced(by: 80), code: 1) == .rollback, "third quick death in a row: roll the update back")
var damaged = NodeWatchdog()
for i in 1...3 {
    let start = t0.advanced(by: Double(i) * 20)
    damaged.started(start)
    check(damaged.exited(start.advanced(by: 3), code: 4, log: "corrupt database") == .restart(after: pow(2, Double(i - 1))), "database damage \(i): no binary rollback")
}
damaged.started(t0.advanced(by: 80))
check(damaged.exited(t0.advanced(by: 83), code: 4, log: "corrupt database") == .stop(.database), "fourth damaged-database exit stops instead of rolling back")

// A long-lived run in between breaks the streak.
var mixed = NodeWatchdog()
mixed.started(t0)
_ = mixed.exited(t0.advanced(by: 5), code: 1)
mixed.started(t0.advanced(by: 20))
_ = mixed.exited(t0.advanced(by: 90), code: 1)  // ran 70 s: not quick
mixed.started(t0.advanced(by: 100))
check(mixed.exited(t0.advanced(by: 105), code: 1) == .restart(after: 4), "the quick-death streak broke")

// Classification: what the sentence should say.
check(NodeWatchdog.classify(code: 4, signaled: false, log: "Io: No space left on device") == .diskFull, "ENOSPC → disk full")
check(NodeWatchdog.classify(code: 4, signaled: false, log: "enospc") == .diskFull, "lowercased ENOSPC")
check(NodeWatchdog.classify(code: 4, signaled: false, log: "the state database does not verify: moved it aside") == .database, "verification failure → database")
check(NodeWatchdog.classify(code: 4, signaled: false, log: "corrupt database") == .database, "corruption text → database")
check(NodeWatchdog.classify(code: 4, signaled: false, log: "completed handoff cannot be installed") == .handoff, "incomplete handoff is diagnosed separately")
var handoff = NodeWatchdog()
handoff.started(t0)
check(handoff.exited(t0.advanced(by: 3), code: 4, log: "completed handoff cannot be installed") == .stop(.handoff), "a broken completed handoff never restarts a signer")
check(NodeWatchdog.classify(code: 4, signaled: false, log: "quiet") == .storage, "unknown storage failure is not guessed to be a full disk")
var failedStorage = NodeWatchdog()
failedStorage.started(t0)
check(failedStorage.exited(t0.advanced(by: 3), code: 4, log: "permission denied") == .stop(.storage), "a persistent storage condition does not restart every 30 seconds")
check(NodeWatchdog.classify(code: 1, signaled: false, log: "memory allocation of 4 GiB failed") == .other, "an unclear log is neither disk nor database")
check(NodeWatchdog.classify(code: 1, signaled: false, log: "out of memory") == .memory, "OOM text → memory")
check(NodeWatchdog.classify(code: 0, signaled: true, log: "") == .memory, "a signal death (kill) → memory")
check(NodeWatchdog.classify(code: 1, signaled: false, log: "connection refused") == .network, "network text")
// The sentences stay plain: no log paths, no jargon, whatever the language.
for f in [NodeWatchdog.Failure.diskFull, .database, .handoff, .storage, .memory, .network, .other, .upgradeNeeded, .identityLost, .alreadyRunning] {
    check(!f.sentence.contains("/") && !f.sentence.contains("redb"), "\(f): no paths or jargon")
    check(!f.sentence.isEmpty, "\(f): says something")
}

// The node's own "do not restart me" exit codes (red team #1/#12): the first
// death stops the app's restarts with the matching sentence — even a quick
// one, and never the rollback path.
var gated = NodeWatchdog()
gated.started(t0)
check(gated.exited(t0.advanced(by: 3), code: 7) == .stop(.alreadyRunning), "a locked data dir stops at once")
check(gated.exited(t0.advanced(by: 4), code: 6) == .stop(.identityLost), "a lost key stops at once")
check(gated.exited(t0.advanced(by: 5), code: 3) == .stop(.upgradeNeeded), "an upgrade exit stops at once")
check(gated.exited(t0.advanced(by: 6), code: 5) == .stop(.upgradeNeeded), "a missing verifier stops at once")
var keyloss = NodeWatchdog()
keyloss.started(t0)
for i in 1...3 {
    keyloss.started(t0.advanced(by: Double(i) * 20))
    check(keyloss.exited(t0.advanced(by: Double(i) * 20 + 3), code: 6) == .stop(.identityLost), "identity death \(i): never rollback, never restart")
}

// Red team #2: work at a frozen height is progress. A node downloading a
// snapshot (its `activity` counter rising every poll) is never a stall; the
// moment the work stops, the usual 60 s applies.
var busy = NodeWatchdog()
busy.started(t0)
for secs in stride(from: 2.0, through: 300, by: 2) {
    check(busy.polled(t0.advanced(by: secs), local: 0, network: 2_100, activity: UInt64(secs / 2), voting: false) == .none, "busy at height 0 for \(secs) s: not a stall")
}
for secs in stride(from: 302.0, through: 356, by: 2) {
    check(busy.polled(t0.advanced(by: secs), local: 0, network: 2_100, activity: 150) == .none, "work stopped, \(secs - 300) s: wait")
}
check(busy.polled(t0.advanced(by: 364), local: 0, network: 2_100, activity: 150) == .restart(after: 0), "work stopped and 60 s passed: restart")

// Red team #2's quorum check: a voting node is given twice the patience — its
// restart costs the network a signature.
var voting = NodeWatchdog()
voting.started(t0)
for secs in stride(from: 2.0, through: 118, by: 2) {
    check(voting.polled(t0.advanced(by: secs), local: 100, network: 400, activity: 5, voting: true) == .none, "voting and frozen \(secs) s: wait")
}
check(voting.polled(t0.advanced(by: 124), local: 100, network: 400, activity: 5, voting: true) == .none, "voting: no restart without quorum evidence")
check(voting.polled(t0.advanced(by: 126), local: 100, network: 400, activity: 5, voting: true, quorumSafe: true) == .restart(after: 0), "voting: restart only after quorum safety is established")

// Red team #9: a sleep or wake invalidates what the watchdog was timing. A
// freeze counted before the sleep must not fire on the strength of it.
var woke = NodeWatchdog()
woke.started(t0)
for secs in stride(from: 2.0, through: 58, by: 2) {
    _ = woke.polled(t0.advanced(by: secs), local: 100, network: 400)
}
woke.invalidate()
check(woke.polled(t0.advanced(by: 64), local: 100, network: 400) == .none, "the pre-sleep freeze (60 s old by now) is gone: the clock starts over")
for secs in stride(from: 66.0, through: 122, by: 2) {
    check(woke.polled(t0.advanced(by: secs), local: 100, network: 400) == .none, "freshly frozen \(secs - 64) s after the wake: wait")
}
check(woke.polled(t0.advanced(by: 126), local: 100, network: 400) == .restart(after: 0), "and 60 s of genuinely frozen polls still restart")

// Red team #3: roll back only when the previous binary speaks the protocol the
// chain has scheduled. An unknown on either side is a refusal, not a guess.
check(!NodeWatchdog.rollbackAllowed(prevProtocol: nil, chainScheduled: 3), "unknown previous binary: refuse")
check(!NodeWatchdog.rollbackAllowed(prevProtocol: 2, chainScheduled: nil), "unknown chain schedule: refuse")
check(!NodeWatchdog.rollbackAllowed(prevProtocol: 2, chainScheduled: 3), "the old binary cannot run the chain: refuse")
check(NodeWatchdog.rollbackAllowed(prevProtocol: 3, chainScheduled: 3), "the old binary speaks the scheduled protocol")
check(NodeWatchdog.rollbackAllowed(prevProtocol: 4, chainScheduled: 3), "an even older-newer binary is fine")

// Red team #14/#17: a dead status RPC releases the local route immediately;
// a live but stale node releases it after five verified lag observations.
var route = NodeWatchdog()
check(route.useLocalNode(local: 100, network: 100, responsive: true, currentlyLocal: false, at: t0.advanced(by: 2)) == .remote, "one caught-up poll is insufficient")
check(route.useLocalNode(local: 100, network: 100, responsive: true, currentlyLocal: false, at: t0.advanced(by: 4)) == .remote, "two caught-up polls are insufficient")
check(route.useLocalNode(local: 100, network: 100, responsive: true, currentlyLocal: false, at: t0.advanced(by: 6)) == .local, "three verified caught-up polls select local")
for i in 1...4 {
    check(route.useLocalNode(local: 100, network: 200, responsive: true, currentlyLocal: true, at: t0.advanced(by: 6 + Double(i) * 2)) == .local, "brief verified lag does not flap")
}
check(route.useLocalNode(local: 100, network: 200, responsive: true, currentlyLocal: true, at: t0.advanced(by: 16)) == .remote, "five lag polls yield local")
check(route.useLocalNode(local: 100, network: 200, responsive: false, currentlyLocal: true, at: t0.advanced(by: 18)) == .remote, "an unresponsive RPC yields local immediately")
check(route.useLocalNode(local: nil, network: 200, responsive: false, currentlyLocal: true, at: t0.advanced(by: 20)) == .remote, "a dead RPC without height still yields local")

// Red team #9: after wake, a prior local selection is stale — the controller
// releases the route on the wake, and every streak (verified or pending)
// starts from zero. A remote view that answers again is re-earned the
// verified way; a missing one takes three fresh healthy polls like below.
var wakeRoute = NodeWatchdog()
for i in 1...3 { _ = wakeRoute.useLocalNode(local: 100, network: 100, responsive: true, currentlyLocal: false, at: t0.advanced(by: Double(i) * 2)) }
wakeRoute.invalidate()
check(wakeRoute.useLocalNode(local: 100, network: 100, responsive: true, currentlyLocal: false, at: t0.advanced(by: 200)) == .remote, "the first post-wake certificate is only the first readiness poll")
check(wakeRoute.useLocalNode(local: 100, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 202)) == .remote, "the first healthy poll without a remote view is also only the first of three")

// The incident of 2026-10-05: after a reboot every validator read times out
// for minutes, so `authenticatedRemoteHeight()` cannot answer, while the
// bundled node on this Mac answers in 20 ms — verified and caught up. The
// wallet used to camp on "Connecting" for the whole outage (a nil remote view
// read as "behind"); now the healthy local node serves reads after three
// healthy polls, marked network-check-pending.
var incident = NodeWatchdog()
check(incident.useLocalNode(local: 2_000, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 2)) == .remote, "no remote view: one healthy poll is not enough")
check(incident.useLocalNode(local: 2_000, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 4)) == .remote, "no remote view: two healthy polls are not enough")
check(incident.useLocalNode(local: 2_000, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 6)) == .localPending, "the third healthy poll selects this Mac's node, marked network-check-pending")
for secs in stride(from: 8.0, through: 58, by: 2) {
    check(incident.useLocalNode(local: 2_000 + UInt64(secs / 2), network: nil, responsive: true, currentlyLocal: true, at: t0.advanced(by: secs)) == .localPending, "pending on a node that keeps moving (\(Int(secs)) s): stay local")
}
// The local node goes quiet — a partition looks exactly like this, and with
// no cross-check nothing can vouch for it: pending may not camp on a stalled
// node. Sixty frozen seconds step back to the validators.
for secs in stride(from: 60.0, through: 118, by: 2) {
    check(incident.useLocalNode(local: 2_029, network: nil, responsive: true, currentlyLocal: true, at: t0.advanced(by: secs)) == .localPending, "frozen \(Int(secs - 60)) s with no cross-check: the last seconds of patience")
}
check(incident.useLocalNode(local: 2_029, network: nil, responsive: true, currentlyLocal: true, at: t0.advanced(by: 120)) == .remote, "frozen 60 s while pending: step back to remote")
check(incident.useLocalNode(local: 2_029, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 122)) == .remote, "after the step-back the streak starts over (poll 1)")
check(incident.useLocalNode(local: 2_029, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 124)) == .remote, "poll 2 of the fresh streak")
check(incident.useLocalNode(local: 2_029, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 126)) == .localPending, "poll 3: local again — the node may have recovered")
// Red team #17, preserved: the moment the remote view can answer again, it is
// authoritative again — "unavailable" never becomes "ahead". A pending local
// selection the certificates call behind yields to the same five
// observations as ever, and the pending mark clears on the very first answer.
check(incident.useLocalNode(local: 2_029, network: 2_129, responsive: true, currentlyLocal: true, at: t0.advanced(by: 128)) == .local, "the remote view returns calling this node behind: the pending mark clears at once")
for i in 1...3 {
    check(incident.useLocalNode(local: 2_029, network: 2_129, responsive: true, currentlyLocal: true, at: t0.advanced(by: 128 + Double(i) * 2)) == .local, "verified lag \(i + 1)/5: brief lag does not flap")
}
check(incident.useLocalNode(local: 2_029, network: 2_129, responsive: true, currentlyLocal: true, at: t0.advanced(by: 136)) == .remote, "five verified lag observations yield local")
check(incident.useLocalNode(local: 2_029, network: 2_029, responsive: true, currentlyLocal: false, at: t0.advanced(by: 138)) == .remote, "a view that returns healthy re-earns local the verified way (poll 1)")
check(incident.useLocalNode(local: 2_029, network: 2_029, responsive: true, currentlyLocal: false, at: t0.advanced(by: 140)) == .remote, "verified poll 2")
check(incident.useLocalNode(local: 2_029, network: 2_029, responsive: true, currentlyLocal: false, at: t0.advanced(by: 142)) == .local, "verified poll 3: local, and no longer pending")

// A node still at height 0 is starting, not a read route — healthy polls at
// genesis never select local, with or without a remote view.
var genesis = NodeWatchdog()
for i in 1...3 {
    check(genesis.useLocalNode(local: 0, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: Double(i) * 2)) == .remote, "height 0, poll \(i): a starting node is not a read route")
}

// A local node that stops answering (or reports no height) never holds the
// route, and a broken streak resets: local is re-earned after three fresh
// healthy polls, not on the first one back.
var unresp = NodeWatchdog()
_ = unresp.useLocalNode(local: 500, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 2))
_ = unresp.useLocalNode(local: 500, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 4))
check(unresp.useLocalNode(local: 500, network: nil, responsive: false, currentlyLocal: false, at: t0.advanced(by: 6)) == .remote, "an unresponsive node is never a read route")
check(unresp.useLocalNode(local: 500, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 8)) == .remote, "the streak restarted with the outage (poll 1)")
check(unresp.useLocalNode(local: 500, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 10)) == .remote, "poll 2 after the outage")
check(unresp.useLocalNode(local: 500, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 12)) == .localPending, "poll 3: three fresh healthy polls, local again")
var heightless = NodeWatchdog()
_ = heightless.useLocalNode(local: 500, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 2))
check(heightless.useLocalNode(local: nil, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 4)) == .remote, "a status RPC that answers without a height is not healthy")
check(heightless.useLocalNode(local: 500, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 6)) == .remote, "the heightless poll reset the streak (poll 1)")
check(heightless.useLocalNode(local: 500, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 8)) == .remote, "poll 2")
check(heightless.useLocalNode(local: 500, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 10)) == .localPending, "poll 3: local again")

// Red team #9 in the pending regime: a sleep invalidates the pending timing
// too. A freeze counted before the sleep must not fire on the strength of
// it — only 60 genuinely awake, frozen seconds do.
var pendingWake = NodeWatchdog()
_ = pendingWake.useLocalNode(local: 700, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 2))
_ = pendingWake.useLocalNode(local: 700, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 4))
check(pendingWake.useLocalNode(local: 700, network: nil, responsive: true, currentlyLocal: false, at: t0.advanced(by: 6)) == .localPending, "three healthy polls: local, pending")
for secs in stride(from: 8.0, through: 60, by: 2) {
    check(pendingWake.useLocalNode(local: 700, network: nil, responsive: true, currentlyLocal: true, at: t0.advanced(by: secs)) == .localPending, "frozen \(Int(secs - 4)) s before the sleep: still inside the 60 s")
}
pendingWake.invalidate()
check(pendingWake.useLocalNode(local: 700, network: nil, responsive: true, currentlyLocal: true, at: t0.advanced(by: 400)) == .localPending, "the wake forgets the 56-s-old freeze: the clock starts over")
for secs in stride(from: 402.0, through: 458, by: 2) {
    check(pendingWake.useLocalNode(local: 700, network: nil, responsive: true, currentlyLocal: true, at: t0.advanced(by: secs)) == .localPending, "freshly frozen \(Int(secs - 400)) s after the wake: wait")
}
check(pendingWake.useLocalNode(local: 700, network: nil, responsive: true, currentlyLocal: true, at: t0.advanced(by: 462)) == .remote, "and 60 genuinely frozen seconds still step back to remote")

print("watchdog: all checks passed")
