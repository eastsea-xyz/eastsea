// The watchdog's decisions (docs/design/24-self-healing.md layer 2), fed the
// events NodeController sees. Pure logic, no app:
//   swiftc -o /tmp/watchdog-check apps/wallet/Sources/NodeWatchdog.swift apps/wallet/Tests/watchdog/main.swift && /tmp/watchdog-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

let t0 = Date(timeIntervalSince1970: 1_790_000_000)

// A single death restarts after a second of backoff, and the backoff doubles
// per death inside the window.
var w = NodeWatchdog()
w.started(t0)
check(w.exited(t0.addingTimeInterval(60), code: 4, log: "Io: No space left on device") == .restart(after: 1), "first death: restart after 1 s")
check(w.exited(t0.addingTimeInterval(120), code: 4) == .restart(after: 2), "second death: 2 s")
check(w.exited(t0.addingTimeInterval(180), code: 4) == .restart(after: 4), "third death: 4 s (third restart)")

// The fourth death inside ten minutes stops the loop with the failure's sentence.
let stopped = w.exited(t0.addingTimeInterval(240), code: 4, log: "storage failed: Io(\"No space left on device\")\nthe store database did not recover; exiting")
check(stopped == .stop(.diskFull), "fourth death inside 10 min: stop with the disk-full sentence")
if case let .stop(f) = stopped {
    check(f.sentence.contains("10"), "the sentence says what to do (free space)")
}

// Deaths older than ten minutes do not count toward the stop.
var old = NodeWatchdog()
old.started(t0)
_ = old.exited(t0.addingTimeInterval(10), code: 1)
_ = old.exited(t0.addingTimeInterval(20), code: 1)
_ = old.exited(t0.addingTimeInterval(30), code: 1)
check(old.exited(t0.addingTimeInterval(700), code: 1) == .restart(after: 1), "a death after the window starts over")

// A stalled node: the network moves, ours does not, for over a minute → one
// restart; a node that still does not move after it does not restart again
// (no restart loop of its own), and progress clears the freeze.
var s = NodeWatchdog()
s.started(t0)
for secs in stride(from: 2.0, through: 62, by: 2) {
    check(s.polled(t0.addingTimeInterval(secs), local: 100, network: 400) == .none, "frozen \(secs) s: wait")
}
check(s.polled(t0.addingTimeInterval(66), local: 100, network: 400) == .restart(after: 0), "frozen past 60 s while the network moves: restart")
s.restarting()
check(s.polled(t0.addingTimeInterval(68), local: 100, network: 400) == .none, "still frozen right after the restart: wait, do not loop")
check(s.polled(t0.addingTimeInterval(200), local: 101, network: 400) == .none, "progress clears the freeze")
check(s.polled(t0.addingTimeInterval(300), local: 101, network: 400) == .none, "a fresh freeze needs its own 60 s")

// A network that is itself paused is not the node's stall.
var paused = NodeWatchdog()
paused.started(t0)
for secs in stride(from: 2.0, through: 120, by: 2) {
    check(paused.polled(t0.addingTimeInterval(secs), local: 100, network: 101) == .none, "network frozen too: never a node restart")
}

// A node that died almost immediately three times in a row: run the previous
// binary (an update that cannot start), instead of restarting this one again.
var up = NodeWatchdog()
up.started(t0)
check(up.exited(t0.addingTimeInterval(5), code: 1) == .restart(after: 1), "quick death 1: restart")
up.started(t0.addingTimeInterval(20))
check(up.exited(t0.addingTimeInterval(23), code: 1) == .restart(after: 2), "quick death 2: restart")
up.started(t0.addingTimeInterval(50))
check(up.exited(t0.addingTimeInterval(80), code: 1) == .rollback, "third quick death in a row: roll the update back")

// A long-lived run in between breaks the streak.
var mixed = NodeWatchdog()
mixed.started(t0)
_ = mixed.exited(t0.addingTimeInterval(5), code: 1)
mixed.started(t0.addingTimeInterval(20))
_ = mixed.exited(t0.addingTimeInterval(90), code: 1)  // ran 70 s: not quick
mixed.started(t0.addingTimeInterval(100))
check(mixed.exited(t0.addingTimeInterval(105), code: 1) == .restart(after: 4), "the quick-death streak broke")

// Classification: what the sentence should say.
check(NodeWatchdog.classify(code: 4, signaled: false, log: "Io: No space left on device") == .diskFull, "ENOSPC → disk full")
check(NodeWatchdog.classify(code: 4, signaled: false, log: "enospc") == .diskFull, "lowercased ENOSPC")
check(NodeWatchdog.classify(code: 4, signaled: false, log: "the state database does not verify: moved it aside") == .database, "verification failure → database")
check(NodeWatchdog.classify(code: 4, signaled: false, log: "corrupt database") == .database, "corruption text → database")
check(NodeWatchdog.classify(code: 4, signaled: false, log: "quiet") == .diskFull, "storage exit without detail: assume the disk (the incident's shape)")
check(NodeWatchdog.classify(code: 1, signaled: false, log: "memory allocation of 4 GiB failed") == .other, "an unclear log is neither disk nor database")
check(NodeWatchdog.classify(code: 1, signaled: false, log: "out of memory") == .memory, "OOM text → memory")
check(NodeWatchdog.classify(code: 0, signaled: true, log: "") == .memory, "a signal death (kill) → memory")
check(NodeWatchdog.classify(code: 1, signaled: false, log: "connection refused") == .network, "network text")
// The sentences stay plain: no log paths, no jargon, whatever the language.
for f in [NodeWatchdog.Failure.diskFull, .database, .memory, .network, .other] {
    check(!f.sentence.contains("/") && !f.sentence.contains("redb"), "\(f): no paths or jargon")
    check(!f.sentence.isEmpty, "\(f): says something")
}

print("watchdog: all checks passed")
