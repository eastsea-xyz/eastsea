import Foundation

/// Records launch health before the wallet starts optional work. A pending
/// launch in the previous process is an early death; two in a row make the
/// third launch safe. Recovery stays latched until a normal run is healthy or
/// the running binary's known version/build changes.
struct LaunchTracker {
    enum Mode: String, Codable, Equatable {
        case normal
        case safe
    }

    static let crashThreshold = 2
    static let healthyInterval: TimeInterval = 60

    private(set) var mode: Mode = .normal
    private(set) var consecutiveCrashes = 0
    private(set) var safeModeRequired = false
    private(set) var startedAt: Date?
    private(set) var healthyAt: Date?
    private(set) var cleanShutdownAt: Date?
    private(set) var runningVersion = ""
    private(set) var runningBuild = ""

    private let recordURL: URL?
    private let now: () -> Date
    /// Loaded records describe a previous process, never this instance.
    private var hasBegunLaunch = false

    /// The wallet's own Application Support directory. Resolving this path
    /// must never migrate, inspect, or create the node's data directory.
    static var defaultRecordURL: URL? {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first?
            .appendingPathComponent("EastSeaWallet", isDirectory: true)
            .appendingPathComponent("launch-state.json")
    }

    /// Missing, corrupt, or foreign records start fresh. State writes are
    /// best effort so an unavailable support directory cannot crash startup.
    init(recordURL: URL? = LaunchTracker.defaultRecordURL, now: @escaping () -> Date = { Date() }) {
        self.recordURL = recordURL
        self.now = now
        guard let recordURL,
              let data = try? Data(contentsOf: recordURL),
              let record = try? JSONDecoder().decode(Record.self, from: data),
              record.schemaVersion == 1 else { return }
        mode = record.mode
        consecutiveCrashes = max(0, record.consecutiveCrashes)
        safeModeRequired = record.safeModeRequired || mode == .safe || consecutiveCrashes >= Self.crashThreshold
        startedAt = record.startedAt
        healthyAt = record.healthyAt
        cleanShutdownAt = record.cleanShutdownAt
        runningVersion = record.runningVersion
        runningBuild = record.runningBuild
    }

    /// Call once, before any other startup work. Repeated notifications in
    /// this process neither count a death nor overwrite its launch marker.
    @discardableResult
    mutating func beginLaunch(runningVersion version: String, runningBuild build: String) -> Mode {
        guard !hasBegunLaunch else { return mode }
        let version = version.trimmingCharacters(in: .whitespacesAndNewlines)
        let build = build.trimmingCharacters(in: .whitespacesAndNewlines)
        let versionChanged = !runningVersion.isEmpty && !version.isEmpty && runningVersion != version
        let buildChanged = !runningBuild.isEmpty && !build.isEmpty && runningBuild != build
        if versionChanged || buildChanged {
            // A completed update is a fact about the new process, not an
            // installer callback that might precede an aborted install.
            consecutiveCrashes = 0
            safeModeRequired = false
        } else if startedAt != nil && healthyAt == nil && cleanShutdownAt == nil {
            if consecutiveCrashes < Int.max { consecutiveCrashes += 1 }
        }
        safeModeRequired = safeModeRequired || consecutiveCrashes >= Self.crashThreshold
        mode = safeModeRequired ? .safe : .normal
        self.runningVersion = version
        self.runningBuild = build
        startedAt = now()
        healthyAt = nil
        cleanShutdownAt = nil
        hasBegunLaunch = true
        save()
        return mode
    }

    /// The caller supplies elapsed awake time from its monotonic clock;
    /// persisted wall-clock dates are only diagnostic timestamps. True only
    /// for the first healthy notification in this run.
    @discardableResult
    mutating func markHealthy(elapsed: TimeInterval) -> Bool {
        guard hasBegunLaunch, healthyAt == nil, cleanShutdownAt == nil,
              elapsed.isFinite, elapsed >= Self.healthyInterval else { return false }
        healthyAt = now()
        if mode == .normal {
            consecutiveCrashes = 0
            safeModeRequired = false
        }
        save()
        return true
    }

    /// A deliberate quit is not a death. It breaks an ordinary crash streak,
    /// but a clean quit during recovery cannot prove a normal run healthy.
    mutating func cleanShutdown() {
        guard hasBegunLaunch, cleanShutdownAt == nil else { return }
        cleanShutdownAt = now()
        if !safeModeRequired { consecutiveCrashes = 0 }
        save()
    }

    /// Try optional work again in this process. The recovery latch survives
    /// both a failed retry and a deliberate quit before its health deadline.
    /// Repeated button actions must not restart the health deadline.
    mutating func retryNormal() {
        guard hasBegunLaunch, mode == .safe, cleanShutdownAt == nil else { return }
        mode = .normal
        startedAt = now()
        healthyAt = nil
        save()
    }

    private struct Record: Codable {
        let schemaVersion: Int
        let mode: Mode
        let consecutiveCrashes: Int
        let safeModeRequired: Bool
        let startedAt: Date?
        let healthyAt: Date?
        let cleanShutdownAt: Date?
        let runningVersion: String
        let runningBuild: String
    }

    private func save() {
        guard let recordURL else { return }
        let record = Record(schemaVersion: 1, mode: mode, consecutiveCrashes: consecutiveCrashes,
                            safeModeRequired: safeModeRequired, startedAt: startedAt,
                            healthyAt: healthyAt, cleanShutdownAt: cleanShutdownAt,
                            runningVersion: runningVersion, runningBuild: runningBuild)
        guard let data = try? JSONEncoder().encode(record) else { return }
        try? FileManager.default.createDirectory(at: recordURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? data.write(to: recordURL, options: .atomic)
    }
}
