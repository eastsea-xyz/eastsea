import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

// The runner sets TMPDIR to the lane's ./tmp. Never fall back to system /tmp
// or write an actual wallet launch marker from a pure test.
guard let tempPath = ProcessInfo.processInfo.environment["TMPDIR"], tempPath.hasPrefix("/") else {
    print("FAIL launch-tracker requires an absolute TMPDIR inside the lane's ./tmp")
    exit(1)
}
let tempRoot = URL(fileURLWithPath: tempPath, isDirectory: true).standardizedFileURL
let laneTemp = URL(fileURLWithPath: FileManager.default.currentDirectoryPath, isDirectory: true)
    .appendingPathComponent("tmp", isDirectory: true).standardizedFileURL
check(tempRoot.path == laneTemp.path || tempRoot.path.hasPrefix(laneTemp.path + "/"), "TMPDIR belongs to this lane")
let directory = tempRoot.appendingPathComponent("launch-tracker-\(UUID().uuidString)", isDirectory: true)
try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: directory) }
let t0 = Date(timeIntervalSince1970: 1_790_000_000)
func freshRecord() -> URL { directory.appendingPathComponent("\(UUID().uuidString).json") }

final class TestClock {
    var now = t0
    func advance(_ seconds: TimeInterval) { now = now.addingTimeInterval(seconds) }
}

final class Rig {
    let recordURL: URL
    let clock: TestClock
    var tracker: LaunchTracker

    init(recordURL: URL = freshRecord()) {
        self.recordURL = recordURL
        clock = TestClock()
        tracker = LaunchTracker(recordURL: recordURL, now: { [clock] in clock.now })
    }

    @discardableResult
    func begin(version: String = "0.7.3", build: String = "703") -> LaunchTracker.Mode {
        tracker.beginLaunch(runningVersion: version, runningBuild: build)
    }

    @discardableResult
    func relaunch(version: String = "0.7.3", build: String = "703") -> LaunchTracker.Mode {
        clock.advance(10)
        tracker = LaunchTracker(recordURL: recordURL, now: { [clock] in clock.now })
        return begin(version: version, build: build)
    }

    func enterSafeMode() {
        check(begin() == .normal, "first launch is normal")
        check(relaunch() == .normal, "one early death still allows normal startup")
        check(relaunch() == .safe, "two early deaths select safe startup")
    }
}

check(LaunchTracker.crashThreshold == 2, "two deaths trigger recovery")
check(LaunchTracker.healthyInterval == 60, "health requires a minute")
let expectedPath = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first?
    .appendingPathComponent("EastSeaWallet/launch-state.json").path
check(LaunchTracker.defaultRecordURL?.path == expectedPath, "launch state belongs to wallet Application Support")

let first = Rig()
check(first.tracker.mode == .normal && first.tracker.consecutiveCrashes == 0 && !first.tracker.safeModeRequired,
      "a missing record starts fresh")
check(!FileManager.default.fileExists(atPath: first.recordURL.path), "initialization does not write a launch")
check(!first.tracker.markHealthy(elapsed: 60), "health before launch is ignored")
first.tracker.cleanShutdown()
first.tracker.retryNormal()
check(first.tracker.startedAt == nil && first.tracker.cleanShutdownAt == nil, "events before launch are harmless")
check(first.begin() == .normal, "a first start is normal")
check(first.tracker.startedAt == t0 && first.tracker.healthyAt == nil && first.tracker.cleanShutdownAt == nil,
      "start is recorded before other work")
let firstData = try Data(contentsOf: first.recordURL)
check(firstData.count < 4096, "launch record stays tiny")
let firstJSON = try JSONSerialization.jsonObject(with: firstData) as! [String: Any]
check(firstJSON["startedAt"] != nil && firstJSON["runningVersion"] as? String == "0.7.3",
      "the launch marker and binary identity are persisted")
first.clock.advance(5)
check(first.begin(version: "0.7.4", build: "704") == .normal, "duplicate launch notifications are harmless")
check(first.tracker.startedAt == t0 && first.tracker.runningVersion == "0.7.3" && first.tracker.consecutiveCrashes == 0,
      "duplicate launch callbacks cannot fabricate a death or update")
check(first.relaunch() == .normal && first.tracker.consecutiveCrashes == 1, "the second launch counts one death")
check(first.relaunch() == .safe && first.tracker.consecutiveCrashes == 2 && first.tracker.safeModeRequired,
      "the third launch reaches the exact threshold")
check(first.relaunch() == .safe && first.tracker.consecutiveCrashes == 3, "a safe-mode early death also remains recorded")

let clean = Rig()
clean.begin()
clean.relaunch()
clean.clock.advance(1)
clean.tracker.cleanShutdown()
let quitAt = clean.tracker.cleanShutdownAt
clean.clock.advance(10)
clean.tracker.cleanShutdown()
check(clean.tracker.cleanShutdownAt == quitAt, "duplicate quit notifications keep the first quit timestamp")
check(!clean.tracker.markHealthy(elapsed: 60), "a quit run cannot later become healthy")
check(clean.relaunch() == .normal && clean.tracker.consecutiveCrashes == 0, "clean shutdown breaks an ordinary crash streak")
check(clean.relaunch() == .normal && clean.tracker.consecutiveCrashes == 1, "a later death starts a new streak")

let health = Rig()
health.begin()
health.relaunch()
for elapsed in [-1.0, -.infinity, .infinity, .nan, 59.999] {
    check(!health.tracker.markHealthy(elapsed: elapsed), "invalid or sub-minute elapsed time cannot establish health")
}
health.clock.advance(-100_000)
check(!health.tracker.markHealthy(elapsed: 59.999), "wall-clock changes do not establish health")
check(health.tracker.markHealthy(elapsed: 60), "exactly 60 monotonic seconds establishes health")
let healthyAt = health.tracker.healthyAt
check(healthyAt == health.clock.now && health.tracker.consecutiveCrashes == 0, "normal health resets the streak and records time")
health.clock.advance(30)
check(!health.tracker.markHealthy(elapsed: 90) && health.tracker.healthyAt == healthyAt,
      "duplicate health callbacks are idempotent")
check(health.relaunch() == .normal && health.tracker.consecutiveCrashes == 0, "a later unclean exit after health is not an early death")

let safeHealthy = Rig()
safeHealthy.enterSafeMode()
check(safeHealthy.tracker.markHealthy(elapsed: 60), "safe startup can record health")
check(safeHealthy.tracker.mode == .safe && safeHealthy.tracker.safeModeRequired,
      "healthy safe startup does not release recovery")
safeHealthy.tracker.cleanShutdown()
check(safeHealthy.relaunch() == .safe && safeHealthy.tracker.consecutiveCrashes == 2,
      "healthy and clean safe-mode runs keep recovery latched without adding a death")
safeHealthy.tracker.cleanShutdown()
check(safeHealthy.relaunch() == .safe, "repeated clean safe runs cannot exit recovery")

let failedRetry = Rig()
failedRetry.enterSafeMode()
failedRetry.tracker.markHealthy(elapsed: 60)
failedRetry.clock.advance(61)
failedRetry.tracker.retryNormal()
let retryStart = failedRetry.tracker.startedAt
check(failedRetry.tracker.mode == .normal && failedRetry.tracker.safeModeRequired && failedRetry.tracker.consecutiveCrashes == 2,
      "normal retry preserves the recovery latch and streak")
check(retryStart == failedRetry.clock.now && failedRetry.tracker.healthyAt == nil, "retry starts its own health interval")
failedRetry.clock.advance(1)
failedRetry.tracker.retryNormal()
check(failedRetry.tracker.startedAt == retryStart, "duplicate retries cannot postpone health")
check(!failedRetry.tracker.markHealthy(elapsed: 59.999), "a retry also needs a full minute")
check(failedRetry.relaunch() == .safe && failedRetry.tracker.consecutiveCrashes == 3, "a failed retry returns to recovery")

let quitRetry = Rig()
quitRetry.enterSafeMode()
quitRetry.tracker.retryNormal()
quitRetry.tracker.cleanShutdown()
check(quitRetry.tracker.safeModeRequired && quitRetry.tracker.consecutiveCrashes == 2, "a short clean retry does not clear recovery")
check(quitRetry.relaunch() == .safe && quitRetry.tracker.consecutiveCrashes == 2,
      "a short clean retry returns safe without counting a death")

let successfulRetry = Rig()
successfulRetry.enterSafeMode()
successfulRetry.tracker.retryNormal()
check(successfulRetry.tracker.markHealthy(elapsed: 60), "a normal retry can establish health")
check(successfulRetry.tracker.mode == .normal && !successfulRetry.tracker.safeModeRequired && successfulRetry.tracker.consecutiveCrashes == 0,
      "one healthy normal run releases recovery")
check(successfulRetry.relaunch() == .normal && !successfulRetry.tracker.safeModeRequired, "normal health persists across relaunch")

let update = Rig()
update.enterSafeMode()
check(update.relaunch(version: "0.7.4", build: "704") == .normal, "an installed version exits recovery")
check(!update.tracker.safeModeRequired && update.tracker.consecutiveCrashes == 0, "an update does not count the old pending launch")
check(update.relaunch(version: "0.7.4", build: "704") == .normal && update.tracker.consecutiveCrashes == 1,
      "the new binary must earn its own health")
check(update.relaunch(version: "0.7.4", build: "704") == .safe, "a broken update gets the same protection")

let buildUpdate = Rig()
buildUpdate.enterSafeMode()
check(buildUpdate.relaunch(version: "0.7.3", build: "704") == .normal && buildUpdate.tracker.consecutiveCrashes == 0,
      "a changed build of the same version is an actual update")
let abortedUpdate = Rig()
abortedUpdate.enterSafeMode()
check(abortedUpdate.relaunch() == .safe && abortedUpdate.tracker.consecutiveCrashes == 3,
      "the same binary after an aborted install does not exit recovery")

let missingIdentity = Rig()
missingIdentity.enterSafeMode()
check(missingIdentity.relaunch(version: "", build: "") == .safe, "an absent binary identity cannot prove an update")
check(missingIdentity.relaunch(version: "0.7.4", build: "704") == .safe,
      "identity appearing after an unknown run cannot fabricate an update")
let missingVersion = Rig()
missingVersion.enterSafeMode()
check(missingVersion.relaunch(version: "", build: "703") == .safe, "a missing version with unchanged build stays safe")
let missingBuild = Rig()
missingBuild.enterSafeMode()
check(missingBuild.relaunch(version: "0.7.3", build: "") == .safe, "a missing build with unchanged version stays safe")
let knownVersionChange = Rig()
knownVersionChange.begin(version: "0.7.3", build: "")
knownVersionChange.relaunch(version: "0.7.3", build: "")
knownVersionChange.relaunch(version: "0.7.3", build: "")
check(knownVersionChange.relaunch(version: "0.7.4", build: "") == .normal,
      "a changed known version proves an update even when builds are unavailable")

func fixture(_ changes: [String: Any]) throws -> URL {
    var json = firstJSON
    for (key, value) in changes { json[key] = value }
    let url = freshRecord()
    try JSONSerialization.data(withJSONObject: json).write(to: url)
    return url
}

let corruptURL = freshRecord()
try Data("{broken".utf8).write(to: corruptURL)
let corrupt = Rig(recordURL: corruptURL)
check(corrupt.begin() == .normal && corrupt.tracker.consecutiveCrashes == 0, "malformed JSON recovers gracefully")
check((try? JSONSerialization.jsonObject(with: Data(contentsOf: corruptURL))) != nil, "a fresh start replaces malformed JSON")
let wrongType = Rig(recordURL: try fixture(["consecutiveCrashes": "many"]))
check(wrongType.begin() == .normal && wrongType.tracker.consecutiveCrashes == 0, "incorrect JSON field types recover gracefully")
let foreignSchema = Rig(recordURL: try fixture(["schemaVersion": 999, "safeModeRequired": true]))
check(foreignSchema.begin() == .normal, "foreign schema records start fresh")
let negative = Rig(recordURL: try fixture(["consecutiveCrashes": -10]))
check(negative.begin() == .normal && negative.tracker.consecutiveCrashes == 1, "negative persisted counts normalize before counting")
let saturated = Rig(recordURL: try fixture(["consecutiveCrashes": Int.max]))
check(saturated.begin() == .safe && saturated.tracker.consecutiveCrashes == Int.max, "maximum persisted count cannot overflow")
check(saturated.relaunch() == .safe && saturated.tracker.consecutiveCrashes == Int.max, "the crash count remains saturated across restarts")
let recoveredLatch = Rig(recordURL: try fixture(["mode": "safe", "safeModeRequired": false]))
check(recoveredLatch.begin() == .safe, "a stored safe mode repairs an inconsistent recovery latch")

var memoryOnly = LaunchTracker(recordURL: nil, now: { t0 })
check(memoryOnly.beginLaunch(runningVersion: "0.7.3", runningBuild: "703") == .normal, "nil storage supports pure in-memory use")
check(memoryOnly.markHealthy(elapsed: 60), "nil storage cannot prevent health")
memoryOnly.cleanShutdown()
check(memoryOnly.cleanShutdownAt == t0, "nil storage cannot prevent clean shutdown")

print("OK launch-tracker")
