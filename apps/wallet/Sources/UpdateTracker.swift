import Foundation

/// The app-update state machine (docs/design/24-self-healing.md row 11, red
/// team #11): a Sparkle update that fails halfway used to leave no trace and
/// no retry. This makes discover → download → verify → install → health-check
/// a persisted state with retries by cause. Pure logic — `AppDelegate` feeds
/// Sparkle's delegate callbacks in, and the Network page shows `sentence`.
/// No Sparkle import here: the file builds for iOS too. Every transition is a
/// plain unit test (Tests/update-state).
///
/// Causes and policy:
/// - a network/download failure retries with backoff (1 min doubling, 6 h
///   cap), with no limit on attempts but never a tight loop;
/// - a signature or approval-gate refusal does not retry the same item — it
///   waits for a new appcast item (if Sparkle proceeds with that item anyway,
///   the gate pass supersedes the refusal);
/// - an install failure retries at most three times with backoff, then stops
///   and says to download the app manually;
/// - a health failure (relaunched on the target version, the node never got
///   healthy in the window) is recorded for the user message; the node's own
///   watchdog and binary rollback stay the *only* recovery mechanisms.
///
/// Everything time-based reads the injected clock. A relaunch (the app killed
/// at any state) resumes from the record: whether an install landed is a fact
/// about the running binary, decided by comparing versions. A corrupt or
/// missing record is idle, never a crash.
struct UpdateTracker {
    /// Why an update attempt ended, and which retry policy applies.
    enum Cause: String, Equatable {
        /// The feed or the download could not be fetched: retry with backoff.
        case network
        /// The signature or the on-chain approval gate refused this item: do
        /// not retry it; wait for a new appcast item.
        case gate
        /// Sparkle could not install what it verified: at most three tries.
        case install
        /// The app relaunched on the target version but its node did not get
        /// healthy in the window. Recorded only — the node's watchdog heals.
        case health
    }

    /// idle → found → downloading → verified → installing → awaitingHealth →
    /// healthy | failed. `failed` carries the cause, the attempt count and
    /// when (or whether) a retry is scheduled.
    enum State: Equatable {
        case idle
        case found(version: String, build: String)
        case downloading(version: String, build: String)
        case verified(version: String, build: String)
        case installing(version: String, build: String)
        case awaitingHealth(version: String, build: String, startedAt: Date)
        case healthy(version: String, build: String)
        case failed(cause: Cause, attempts: Int, nextRetryAt: Date?)

        /// The version an `awaitingHealth` state is health-checking, if any.
        var awaitingHealthVersion: String? {
            if case .awaitingHealth(let version, _, _) = self { return version }
            return nil
        }

        var failedCause: Cause? {
            if case .failed(let cause, _, _) = self { return cause }
            return nil
        }

        var failedAttempts: Int? {
            if case .failed(_, let attempts, _) = self { return attempts }
            return nil
        }
    }

    // Retry policy (docs/design/24-self-healing.md row 11).
    static let networkFirstBackoff: TimeInterval = 60
    static let networkMaxBackoff: TimeInterval = 6 * 3600
    static let installFirstBackoff: TimeInterval = 60
    static let installMaxBackoff: TimeInterval = 3600
    static let maxInstallAttempts = 3
    /// How long a relaunched update has to show a living node before the
    /// health failure is recorded. Catching up counts as living: a node that
    /// is downloading a snapshot is working, not broken.
    static let healthWindow: TimeInterval = 600
    /// No time this tracker persists can legitimately sit further in the
    /// future than its largest backoff. A `nextRetryAt` or health-window
    /// start beyond that (red team #6: the record was written while the
    /// Mac's clock was wrong, then the clock was corrected) is a clock
    /// artifact, not a schedule — clamp it to now, so a retry or a health
    /// check can never be postponed for weeks.
    static let maxFutureTolerance: TimeInterval = networkMaxBackoff

    static func networkBackoff(afterAttempts attempts: Int) -> TimeInterval {
        min(networkFirstBackoff * pow(2, Double(attempts - 1)), networkMaxBackoff)
    }

    static func installBackoff(afterAttempts attempts: Int) -> TimeInterval {
        min(installFirstBackoff * pow(2, Double(attempts - 1)), installMaxBackoff)
    }

    private(set) var state: State = .idle
    /// The item this cycle is about (version·build·signature·URL, as the
    /// release gate identifies it), persisted with the state.
    private(set) var itemKey: String?
    /// The item that must not be retried: a gate refusal, or an install that
    /// already failed three times.
    private var blockedKey: String?
    private var currentVersion = ""
    private var currentBuild = ""
    /// Attempts of the currently failing cause: what the backoff doubles on.
    private var attempts = 0
    private var lastCause: Cause?
    private let recordURL: URL?
    private let now: () -> Date

    /// `~/Library/Application Support/EastSea/update-state.json`, beside the
    /// node's own data.
    static var defaultRecordURL: URL? {
        DataMigration.ensure()
        return FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first?
            .appendingPathComponent("EastSea/update-state.json")
    }

    /// Reads the persisted record. Corrupt, missing or foreign: idle, and the
    /// next event rewrites it. Never throws, never crashes the caller.
    init(recordURL: URL? = UpdateTracker.defaultRecordURL, now: @escaping () -> Date = { Date() }) {
        self.recordURL = recordURL
        self.now = now
        guard let recordURL,
              let data = try? Data(contentsOf: recordURL),
              let record = try? JSONDecoder().decode(Record.self, from: data) else { return }
        itemKey = record.itemKey
        blockedKey = record.blockedKey
        currentVersion = record.version ?? ""
        currentBuild = record.build ?? ""
        attempts = record.attempts
        lastCause = record.lastCause.flatMap(Cause.init(rawValue:))
        // Red team #6: persisted times are wall-clock by necessity (they must
        // survive a restart), so they are made robust instead — anything the
        // app could never have scheduled that far ahead is clamped to now.
        let present = now()
        let recordStartedAt = Self.clampedToPresent(record.startedAt, now: present)
        let recordNextRetryAt = Self.clampedToPresent(record.nextRetryAt, now: present)
        switch record.phase {
        case "found": state = .found(version: currentVersion, build: currentBuild)
        case "downloading": state = .downloading(version: currentVersion, build: currentBuild)
        case "verified": state = .verified(version: currentVersion, build: currentBuild)
        case "installing": state = .installing(version: currentVersion, build: currentBuild)
        case "awaitingHealth":
            guard let startedAt = recordStartedAt else { return }
            state = .awaitingHealth(version: currentVersion, build: currentBuild, startedAt: startedAt)
        case "healthy": state = .healthy(version: currentVersion, build: currentBuild)
        case "failed":
            guard let cause = lastCause else { return }
            state = .failed(cause: cause, attempts: record.attempts, nextRetryAt: recordNextRetryAt)
        default: return
        }
    }

    /// A persisted time far in the future is a clock artifact (red team #6):
    /// clamp it to now rather than trust it as a schedule. A clamped retry
    /// is due at once — the persisted attempt count keeps the backoff
    /// growing, so this can never become a tight loop.
    private static func clampedToPresent(_ date: Date?, now: Date) -> Date? {
        guard let date, date.timeIntervalSince(now) > maxFutureTolerance else { return date }
        return now
    }

    // MARK: events (fed by AppDelegate from Sparkle's delegate callbacks)

    /// Sparkle offered an item. False when this exact item must not be
    /// restarted (refused, or an install that already failed three times):
    /// the app then waits for a new appcast item. A retryable failure of the
    /// same item keeps its attempt count, so the backoff keeps growing.
    @discardableResult
    mutating func found(key: String, version: String, build: String) -> Bool {
        let known = key == itemKey
        itemKey = key
        currentVersion = version
        currentBuild = build
        if key == blockedKey, case .failed = state {
            save()
            return false
        }
        if known, case .failed = state {
            // A retryable failure of the same item: a fresh attempt at the
            // same cycle, with the attempt count kept — the backoff keeps
            // growing across retries.
            state = .found(version: version, build: build)
            save()
            return true
        }
        if known {
            switch state {
            case .found, .downloading, .verified, .installing, .awaitingHealth:
                save()
                return true  // mid-cycle on the same item: nothing to restart
            case .idle, .healthy, .failed:
                break  // a leftover record (failures are handled above)
            }
        }
        // A new item: a fresh cycle with a fresh attempt count.
        blockedKey = nil
        attempts = 0
        lastCause = nil
        state = .found(version: version, build: build)
        save()
        return true
    }

    /// The approval gate let Sparkle proceed: it downloads and verifies now.
    /// A download that starts is stronger than a stale refusal of the same
    /// item, so this also unblocks it.
    mutating func downloading() {
        switch state {
        case .awaitingHealth, .healthy:
            return
        case .idle:
            guard itemKey != nil else { return }
        case .found, .downloading, .verified, .installing, .failed:
            break
        }
        blockedKey = nil
        state = .downloading(version: currentVersion, build: currentBuild)
        save()
    }

    /// Sparkle verified the downloaded archive's signature.
    mutating func verified() {
        switch state {
        case .downloading, .found, .verified:
            state = .verified(version: currentVersion, build: currentBuild)
            save()
        case .idle, .failed, .installing, .awaitingHealth, .healthy:
            break
        }
    }

    /// Sparkle begins installing (the app is about to relaunch).
    mutating func installing() {
        switch state {
        case .verified, .downloading, .found:
            state = .installing(version: currentVersion, build: currentBuild)
            save()
        case .idle, .failed, .installing, .awaitingHealth, .healthy:
            break
        }
    }

    /// The app started again. Whether the update landed is a fact about the
    /// running binary, not about what Sparkle last said: compare the two.
    mutating func relaunched(runningVersion: String, runningBuild: String) {
        switch state {
        case .installing(let version, let build):
            if version == runningVersion, build == runningBuild {
                state = .awaitingHealth(version: version, build: build, startedAt: now())
            } else {
                fail(.install)  // killed mid-install and the old app came back
            }
        case .awaitingHealth(let version, let build, let startedAt):
            if version == runningVersion, build == runningBuild {
                // The window keeps running from the *first* relaunch on the
                // target version, not from every launch after it.
                if now().timeIntervalSince(startedAt) >= Self.healthWindow { fail(.health) }
            } else {
                fail(.health)  // back on another version: the cycle's outcome is void
            }
        case .downloading, .verified:
            fail(.network)  // killed before the install: the download never completed
        case .idle, .found, .healthy, .failed:
            break  // waiting for approval is not a failure; neither is being done
        }
        save()
    }

    /// The release-approval gate said no for the current item (its signature
    /// or its on-chain approval failed): do not retry this item. Stale
    /// answers about a cycle that already moved past the gate are ignored.
    mutating func refused() {
        switch state {
        case .idle, .found, .failed:
            refuseItem()
        case .verified, .downloading, .installing, .awaitingHealth, .healthy:
            break
        }
    }

    /// Sparkle gave up (`didAbortWithError`). Which bucket it belongs to is a
    /// fact about the phase it died in: a non-network abort while downloading
    /// means the verification refused the bytes.
    mutating func aborted(networkError: Bool) {
        switch state {
        case .downloading:
            if networkError { fail(.network) } else { refuseItem() }
        case .found, .idle, .failed:
            fail(.network)  // the check itself failed: retry with backoff
        case .verified, .installing:
            fail(.install)
        case .awaitingHealth, .healthy:
            break
        }
    }

    /// The caller confirmed a running, responsive node on the installed
    /// release. A starting node or unverified daemon does not pass health.
    mutating func nodeRunning(running: Bool = false, releaseVerified: Bool = false) {
        guard running && releaseVerified else { return }
        if case .awaitingHealth(let version, let build, _) = state {
            attempts = 0
            lastCause = nil
            state = .healthy(version: version, build: build)
            save()
        }
    }

    /// The app's periodic tick (30 s): ends the health window when it passed.
    mutating func tick() {
        if case .awaitingHealth(_, _, let startedAt) = state {
            // A startedAt in the future means the wall clock stepped back
            // after the record was written (red team #6): measure the window
            // from now instead of waiting for the clock to catch up with a
            // wrong value — the check must eventually fire, never early.
            let windowStart = min(startedAt, now())
            if now().timeIntervalSince(windowStart) >= Self.healthWindow {
                fail(.health)
            }
        }
    }

    /// Whether a scheduled retry (network or install) is due now — the app
    /// then asks Sparkle to check again. Refusals, health failures and a
    /// given-up install schedule nothing. A retry scheduled *further* out
    /// than the largest possible backoff cannot be one we wrote — only a
    /// clock artifact (red team #6) — and is due now; exactly the cap is a
    /// legal schedule (the 6 h network backoff) and still waits.
    func retryDue() -> Bool {
        if case .failed(_, _, let next?) = state {
            return now() >= next || next.timeIntervalSince(now()) > Self.maxFutureTolerance
        }
        return false
    }

    /// One honest sentence for the Network page: what happened, what happens
    /// next. Nil while there is nothing to say.
    var sentence: String? {
        switch state {
        case .idle, .found, .downloading, .verified, .installing, .healthy:
            return nil
        case .awaitingHealth(let version, _, _):
            return String(localized: "Updated to \(version). \(Brand.name) is checking that this Mac's node is healthy…")
        case .failed(let cause, let attempts, let nextRetryAt):
            switch cause {
            case .network:
                return String(localized: "The update could not be downloaded. \(Brand.name) will try again by itself.")
            case .gate:
                return String(localized: "This update is not approved by the network. \(Brand.name) left it alone and waits for the next approved release.")
            case .install:
                return nextRetryAt == nil
                    ? String(localized: "The update could not be installed after \(Self.maxInstallAttempts) tries. Download \(Brand.name) again from its website and replace this app.")
                    : String(localized: "The update could not be installed (try \(attempts) of \(Self.maxInstallAttempts)). \(Brand.name) will try again by itself.")
            case .health:
                return String(localized: "The update finished, but this Mac's node did not get healthy in time. \(Brand.name) recorded it; the node keeps healing itself as before.")
            }
        }
    }

    /// The record's location (the tests resume a second tracker from it).
    var recordURLForTesting: URL? { recordURL }

    // MARK: internals

    private mutating func refuseItem() {
        blockedKey = itemKey
        attempts = 1
        lastCause = .gate
        state = .failed(cause: .gate, attempts: 1, nextRetryAt: nil)
        save()
    }

    /// Attempts are counted per cause: a download failure does not eat
    /// install tries, and the same cause repeating keeps growing its backoff.
    private mutating func fail(_ cause: Cause) {
        attempts = lastCause == cause ? attempts + 1 : 1
        lastCause = cause
        switch cause {
        case .network:
            state = .failed(cause: .network, attempts: attempts,
                nextRetryAt: now().addingTimeInterval(Self.networkBackoff(afterAttempts: attempts)))
        case .install:
            if attempts >= Self.maxInstallAttempts {
                blockedKey = itemKey  // this item is done; a new one gets fresh tries
                state = .failed(cause: .install, attempts: attempts, nextRetryAt: nil)
            } else {
                state = .failed(cause: .install, attempts: attempts,
                    nextRetryAt: now().addingTimeInterval(Self.installBackoff(afterAttempts: attempts)))
            }
        case .gate:
            refuseItem()
        case .health:
            state = .failed(cause: .health, attempts: attempts, nextRetryAt: nil)
        }
        save()
    }

    private func record() -> Record {
        var r = Record()
        r.version = currentVersion
        r.build = currentBuild
        r.itemKey = itemKey
        r.blockedKey = blockedKey
        r.lastCause = lastCause?.rawValue
        r.attempts = attempts
        switch state {
        case .idle:
            break
        case .found:
            r.phase = "found"
        case .downloading:
            r.phase = "downloading"
        case .verified:
            r.phase = "verified"
        case .installing:
            r.phase = "installing"
        case .awaitingHealth(_, _, let startedAt):
            r.phase = "awaitingHealth"
            r.startedAt = startedAt
        case .healthy:
            r.phase = "healthy"
        case .failed(let cause, _, let nextRetryAt):
            r.phase = "failed"
            r.lastCause = cause.rawValue
            r.nextRetryAt = nextRetryAt
        }
        return r
    }

    /// Written atomically; any failure is silent — the state machine must
    /// never take the app down with it.
    private mutating func save() {
        guard let recordURL,
              let data = try? JSONEncoder().encode(record()) else { return }
        try? FileManager.default.createDirectory(at: recordURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? data.write(to: recordURL, options: .atomic)
    }

    /// The persisted record: a small flat JSON file. Missing required fields
    /// fail decoding, which reads as idle (a corrupt file never runs).
    private struct Record: Codable {
        var v = 1
        var phase = "idle"
        var version: String?
        var build: String?
        var itemKey: String?
        var blockedKey: String?
        var startedAt: Date?
        var lastCause: String?
        var attempts = 0
        var nextRetryAt: Date?
    }
}
