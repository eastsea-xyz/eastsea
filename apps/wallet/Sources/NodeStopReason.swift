import Foundation

/// Why the node on this Mac is not running while its switch is on — one
/// typed reason, never a bare "paused" (the founder's 0.7.0 report: "왜
/// 멈췄는지 왜 설명을 안해줌?"). Every reason carries what happened, what to
/// do, when it resumes by itself, and at most one button. The sidebar line,
/// the Node page, the menu-bar panel and the health banner all read this one
/// value, so the four places can never disagree.
///
/// Pure (Foundation only): `NodeController` gathers the facts, `NodeResume`
/// decides, and Tests/node-stop covers every rule and every sentence.
enum NodeStopReason: Equatable {
    /// The switch is off (not an incident: the person chose it).
    case switchedOff
    /// "Only while on the power adapter" and the Mac is on battery.
    case onBattery
    /// Running from a DMG, a translocated copy or a read-only volume.
    case wrongLocation
    /// This build carries no node binary.
    case noHelper
    /// The Aether → EastSea data move is copying right now.
    case migrating
    /// The data move has not finished; its gate's own sentence.
    case migrationBlocked(String)
    /// Another program holds this node's data directory (`run.lock`).
    case otherNodeRunning
    /// The node's data volume is below the resume threshold.
    case diskFull(freeBytes: UInt64, resumeBytes: UInt64, volume: String?)
    /// The chosen block-data disk is not connected.
    case diskMissing(volume: String)
    /// The chosen block-data disk is connected but this app may not use it
    /// (macOS privacy: removable volumes), or it went read-only.
    case diskNoAccess(volume: String)
    /// The watchdog is waiting out a restart backoff.
    case restarting(inSeconds: Int)
    /// The node died over and over; it is tried again by itself later.
    case crashLoop(NodeWatchdog.Failure, retryInSeconds: Int)
    /// A failure no automatic retry can fix (damaged data, a handoff that
    /// cannot be recovered, storage that cannot be opened).
    case needsAttention(NodeWatchdog.Failure)
    /// The chain runs rules this app's node does not have.
    case upgradeNeeded
    /// This Mac's node key cannot be read.
    case identityLost
    /// The node is alive and retrying an unavailable Mac-identity read.
    case waitingForMacConfirmation
    /// A successful identity read proved these keys are bound to another Mac.
    case keyElsewhere
    /// The process could not even be launched (an OS error).
    case launchFailed(String)
    /// The block data is moving to another disk (0…100).
    case movingStorage(percent: Int)

    /// A stable code for the status log and diagnostics (never shown).
    var code: String {
        switch self {
        case .switchedOff: return "switched_off"
        case .onBattery: return "on_battery"
        case .wrongLocation: return "wrong_location"
        case .noHelper: return "no_helper"
        case .migrating: return "migrating"
        case .migrationBlocked: return "migration_blocked"
        case .otherNodeRunning: return "data_dir_locked"
        case .diskFull: return "disk_full"
        case .diskMissing: return "disk_missing"
        case .diskNoAccess: return "disk_no_access"
        case .restarting: return "restarting"
        case .crashLoop: return "crash_loop"
        case .needsAttention: return "needs_attention"
        case .upgradeNeeded: return "upgrade_needed"
        case .identityLost: return "identity_lost"
        case .waitingForMacConfirmation: return "waiting_for_mac_confirmation"
        case .keyElsewhere: return "key_elsewhere"
        case .launchFailed: return "launch_failed"
        case .movingStorage: return "moving_storage"
        }
    }

    /// Whether this is something to tell the person about (a banner, a
    /// warning colour). The switch being off, waiting for power, a data move
    /// or a restart a few seconds away are expected states, said plainly but
    /// not raised as incidents.
    var isIncident: Bool {
        switch self {
        case .switchedOff, .onBattery, .migrating, .restarting, .movingStorage, .waitingForMacConfirmation: return false
        default: return true
        }
    }
}

/// The one button a stop reason carries.
enum NodeStopAction: Equatable {
    case turnOn
    case runOnBattery
    case showInFinder
    case checkForUpdates
    case retryNow
    case openStorage
    case chooseDisk
    case openPrivacySettings
    case copyDiagnostics
    case rebindKeys
}

/// What a reason says, in the app's language.
struct NodeStopCopy: Equatable {
    /// Short: the sidebar and the menu (two lines at most, never truncated).
    let title: String
    /// What happened and what to do.
    let detail: String
    /// When it resumes by itself (empty when it needs the person).
    let resume: String
    let action: NodeStopAction?
    /// The button's label (nil when there is no button).
    let actionLabel: String?

    /// The detail and the resume sentence as one paragraph.
    var paragraph: String { resume.isEmpty ? detail : detail + " " + resume }
}

extension NodeStopReason {
    /// Gigabytes as the node counts them (GiB, `resources.rs` GB), one decimal.
    static func gb(_ bytes: UInt64) -> String {
        String(format: "%.1f GB", Double(bytes) / 1_073_741_824)
    }

    /// How much more to free to reach `resume`: one decimal below 1 GB,
    /// whole gigabytes (rounded up) above.
    static func need(free: UInt64, resume: UInt64) -> String {
        let gib = Double(resume > free ? resume - free : 0) / 1_073_741_824
        if gib < 1 { return String(format: "%.1f GB", max(gib, 0.1)) }
        return "\(Int(gib.rounded(.up))) GB"
    }

    /// Seconds as "N초"/"N s", minutes above 90 s.
    static func wait(_ seconds: Int, locale: Locale = .current, bundle: Bundle = .main) -> String {
        if seconds >= 90 {
            let m = (seconds + 59) / 60
            return String(localized: "\(String(m)) min", bundle: bundle, locale: locale)
        }
        return String(localized: "\(String(max(seconds, 1))) s", bundle: bundle, locale: locale)
    }

    func copy(locale: Locale = .current, bundle: Bundle = .main) -> NodeStopCopy {
        switch self {
        case .switchedOff:
            return NodeStopCopy(title: String(localized: "Off — node status", defaultValue: "Off", bundle: bundle, locale: locale),
                                detail: String(localized: "Turn it on and this Mac checks every block itself.", bundle: bundle, locale: locale),
                                resume: "", action: .turnOn, actionLabel: String(localized: "Turn On", bundle: bundle, locale: locale))
        case .onBattery:
            return NodeStopCopy(title: String(localized: "On battery · resting", bundle: bundle, locale: locale),
                                detail: String(localized: "It is set to run only on the power adapter.", bundle: bundle, locale: locale),
                                resume: String(localized: "Plug in and it restarts by itself within 30 s.", bundle: bundle, locale: locale),
                                action: .runOnBattery, actionLabel: String(localized: "Run on Battery Too", bundle: bundle, locale: locale))
        case .wrongLocation:
            return NodeStopCopy(title: String(localized: "Not in Applications", bundle: bundle, locale: locale),
                                detail: String(localized: "The node cannot run from a disk image or the Downloads folder. Move the app to Applications and open it again.", bundle: bundle, locale: locale),
                                resume: "", action: .showInFinder, actionLabel: String(localized: "Show in Finder", bundle: bundle, locale: locale))
        case .noHelper:
            return NodeStopCopy(title: String(localized: "Node missing", bundle: bundle, locale: locale),
                                detail: String(localized: "This copy of the app is missing its node. Update or reinstall the app.", bundle: bundle, locale: locale),
                                resume: "", action: .checkForUpdates, actionLabel: String(localized: "Check for Updates", bundle: bundle, locale: locale))
        case .migrating:
            return NodeStopCopy(title: String(localized: "Moving your old data", bundle: bundle, locale: locale),
                                detail: String(localized: "Your Aether wallet and node data are moving over.", bundle: bundle, locale: locale),
                                resume: String(localized: "The node starts by itself when it is done.", bundle: bundle, locale: locale),
                                action: nil, actionLabel: nil)
        case .migrationBlocked(let why):
            return NodeStopCopy(title: String(localized: "Old data not moved yet", bundle: bundle, locale: locale),
                                detail: why,
                                resume: String(localized: "Checked again every 30 s.", bundle: bundle, locale: locale),
                                action: .retryNow, actionLabel: String(localized: "Try Now", bundle: bundle, locale: locale))
        case .otherNodeRunning:
            return NodeStopCopy(title: String(localized: "Another program has the node data", bundle: bundle, locale: locale),
                                detail: String(localized: "Another node program (such as the old Aether app) is using the same data. Quit it.", bundle: bundle, locale: locale),
                                resume: String(localized: "The node starts by itself within 30 s of it quitting.", bundle: bundle, locale: locale),
                                action: .retryNow, actionLabel: String(localized: "Try Now", bundle: bundle, locale: locale))
        case .diskFull(let free, let resume, let volume):
            let on = volume.map { String(localized: "“\($0)”: ", bundle: bundle, locale: locale) } ?? ""
            return NodeStopCopy(title: String(localized: "Storage low · node resting", bundle: bundle, locale: locale),
                                detail: String(localized: "\(on)\(Self.gb(free)) free. Free about \(Self.need(free: free, resume: resume)) more.", bundle: bundle, locale: locale),
                                resume: String(localized: "It restarts by itself at \(Self.gb(resume)).", bundle: bundle, locale: locale),
                                action: .openStorage, actionLabel: String(localized: "Manage Storage", bundle: bundle, locale: locale))
        case .diskMissing(let volume):
            return NodeStopCopy(title: String(localized: "Disk “\(volume)” not connected", bundle: bundle, locale: locale),
                                detail: String(localized: "The block data lives on this disk. Nothing is re-downloaded to the internal disk behind your back.", bundle: bundle, locale: locale),
                                resume: String(localized: "Connect it and the node restarts by itself.", bundle: bundle, locale: locale),
                                action: .chooseDisk, actionLabel: String(localized: "Choose Another Location", bundle: bundle, locale: locale))
        case .diskNoAccess(let volume):
            return NodeStopCopy(title: String(localized: "No access to “\(volume)”", bundle: bundle, locale: locale),
                                detail: String(localized: "Turn on “Removable Volumes” for EastSea in System Settings › Privacy & Security › Files and Folders.", bundle: bundle, locale: locale),
                                resume: String(localized: "The node starts by itself within 30 s.", bundle: bundle, locale: locale),
                                action: .openPrivacySettings, actionLabel: String(localized: "Open System Settings", bundle: bundle, locale: locale))
        case .restarting(let s):
            return NodeStopCopy(title: String(localized: "Restarting the node", bundle: bundle, locale: locale),
                                detail: String(localized: "The node stopped, so it is restarting.", bundle: bundle, locale: locale),
                                resume: String(localized: "Starting in \(Self.wait(s, locale: locale, bundle: bundle)).", bundle: bundle, locale: locale),
                                action: nil, actionLabel: nil)
        case .crashLoop(let failure, let s):
            return NodeStopCopy(title: String(localized: "The node keeps stopping", bundle: bundle, locale: locale),
                                detail: Self.crashDetail(failure, locale: locale, bundle: bundle),
                                resume: String(localized: "It tries again by itself in \(Self.wait(s, locale: locale, bundle: bundle)).", bundle: bundle, locale: locale),
                                action: .retryNow, actionLabel: String(localized: "Try Now", bundle: bundle, locale: locale))
        case .needsAttention(let failure):
            return NodeStopCopy(title: String(localized: "The node's data needs attention", bundle: bundle, locale: locale),
                                detail: failure.sentence(locale: locale, bundle: bundle),
                                resume: String(localized: "After fixing it, press Try Again.", bundle: bundle, locale: locale),
                                action: .retryNow, actionLabel: String(localized: "Try Again", bundle: bundle, locale: locale))
        case .upgradeNeeded:
            return NodeStopCopy(title: String(localized: "Update needed", bundle: bundle, locale: locale),
                                detail: String(localized: "The network's rules changed and this version's node cannot follow them.", bundle: bundle, locale: locale),
                                resume: String(localized: "It starts by itself once the update is installed.", bundle: bundle, locale: locale),
                                action: .checkForUpdates, actionLabel: String(localized: "Check for Updates", bundle: bundle, locale: locale))
        case .identityLost:
            return NodeStopCopy(title: String(localized: "Node key unreadable", bundle: bundle, locale: locale),
                                detail: NodeWatchdog.Failure.identityLost.sentence(locale: locale, bundle: bundle),
                                resume: "", action: .copyDiagnostics, actionLabel: String(localized: "Copy Diagnostics", bundle: bundle, locale: locale))
        case .waitingForMacConfirmation:
            return NodeStopCopy(title: String(localized: "Waiting to confirm this Mac", bundle: bundle, locale: locale),
                                detail: String(localized: "This Mac's node keys cannot be confirmed right now. The node keeps running and waits before signing.", bundle: bundle, locale: locale),
                                resume: String(localized: "It checks again by itself. No action is needed.", bundle: bundle, locale: locale),
                                action: nil, actionLabel: nil)
        case .keyElsewhere:
            return NodeStopCopy(title: String(localized: "Node keys from another Mac", bundle: bundle, locale: locale),
                                detail: String(localized: "This node's keys came from another Mac. Restore them on the original Mac, or rebind them here if you intentionally moved this node.", bundle: bundle, locale: locale),
                                resume: "", action: .rebindKeys, actionLabel: String(localized: "Rebind Node Keys…", bundle: bundle, locale: locale))
        case .launchFailed:
            return NodeStopCopy(title: String(localized: "The node could not start", bundle: bundle, locale: locale),
                                detail: String(localized: "macOS refused to launch the node.", bundle: bundle, locale: locale),
                                resume: String(localized: "Tried again every 30 s.", bundle: bundle, locale: locale),
                                action: .retryNow, actionLabel: String(localized: "Try Now", bundle: bundle, locale: locale))
        case .movingStorage(let p):
            return NodeStopCopy(title: String(localized: "Moving block data · \(String(p))%", bundle: bundle, locale: locale),
                                detail: String(localized: "The block data is being copied and checked at its new place. The wallet keeps working.", bundle: bundle, locale: locale),
                                resume: String(localized: "The node starts after the block data move.", defaultValue: "The node starts by itself when it is done.", bundle: bundle, locale: locale),
                                action: nil, actionLabel: nil)
        }
    }

    /// The crash loop's cause in one plain clause (the watchdog's own
    /// sentence for the specific kinds).
    private static func crashDetail(_ f: NodeWatchdog.Failure, locale: Locale = .current, bundle: Bundle = .main) -> String {
        switch f {
        case .memory: return f.sentence(locale: locale, bundle: bundle)
        case .network: return String(localized: "The node could not reach the network. Check the internet connection.", bundle: bundle, locale: locale)
        default: return String(localized: "The node stopped several times right after starting. Your balance is still checked through other nodes.", bundle: bundle, locale: locale)
        }
    }
}

/// The node reports an unavailable read while staying alive. These markers
/// are state transitions, not exit reasons; unrelated output retains the
/// previous state until a successful retry confirms this Mac.
enum NodeMacConfirmation {
    static func waiting(in log: String, previously: Bool = false) -> Bool {
        var waiting = previously
        for line in log.split(separator: "\n") {
            if line.contains("waiting to confirm this Mac") { waiting = true }
            if line.contains("Mac key binding confirmed") { waiting = false }
        }
        return waiting
    }
}

/// Written by the node only after a successful hardware read proves a
/// mismatch. It survives daemon/app exits and is removed only by owner rebind.
enum NodeBindingRefusal {
    static let fileName = "key-binding-refused"

    static func exists(in dataDirectory: URL) -> Bool {
        FileManager.default.fileExists(atPath: dataDirectory.appendingPathComponent(fileName).path)
    }
}

/// Rebinding is an owner action for a proven mismatch. A successful owner
/// authentication creates the only approval the terminal runner accepts.
/// The approval is local: its message cannot be submitted as a transaction.
enum NodeKeyRebind {
    struct Approval: Sendable {
        let validatorAddress: String
        let dataDirectory: String
        let confirmationLine: String
        fileprivate init(validatorAddress: String, dataDirectory: String, confirmationLine: String) {
            self.validatorAddress = validatorAddress
            self.dataDirectory = dataDirectory
            self.confirmationLine = confirmationLine
        }
    }

    enum Refusal: LocalizedError {
        case invalidValidatorAddress
        case confirmationDidNotMatch
        case ownerKeyUnavailable
        case nodeRunning

        var errorDescription: String? { sentence() }

        func sentence(locale: Locale = .current, bundle: Bundle = .main) -> String {
            switch self {
            case .invalidValidatorAddress:
                return String(localized: "The validator address cannot be read. Restore the keys and try again.", bundle: bundle, locale: locale)
            case .confirmationDidNotMatch:
                return String(localized: "The typed validator address did not match. The keys were not changed.", bundle: bundle, locale: locale)
            case .ownerKeyUnavailable:
                return String(localized: "Owner authentication is unavailable. Try again when the wallet key is ready.", bundle: bundle, locale: locale)
            case .nodeRunning:
                return String(localized: "The node is running. Wait for it to stop completely and try again.", bundle: bundle, locale: locale)
            }
        }
    }

    static func canOffer(reason: NodeStopReason?, processRunning: Bool, attached: Bool, lockHeld: Bool) -> Bool {
        reason == .keyElsewhere && !processRunning && !attached && !lockHeld
    }

    static func warning(locale: Locale = .current, bundle: Bundle = .main) -> String {
        String(localized: "Only do this if you moved this Mac's node on purpose; running the same keys on two Macs gets the validator slashed.", bundle: bundle, locale: locale)
    }

    /// The validator address is its 32-byte Ed25519 public key, not the
    /// wallet account. Read only the public entry; never open private keys.
    static func validatorAddress(in data: Data) throws -> String {
        struct PublicEntry: Decodable { let key: String }
        guard data.count <= 4_096,
              let entry = try? JSONDecoder().decode(PublicEntry.self, from: data),
              let address = normalized(entry.key) else { throw Refusal.invalidValidatorAddress }
        return address
    }

    static func normalized(_ address: String) -> String? {
        var text = address.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if text.hasPrefix("0x") { text.removeFirst(2) }
        guard text.utf8.count == 64,
              text.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else { return nil }
        return text
    }

    static func authorize(validatorAddress: String, typedAddress: String, dataDirectory: String,
                          authenticate: (Data) throws -> Void) throws -> Approval {
        guard let address = normalized(validatorAddress) else { throw Refusal.invalidValidatorAddress }
        guard normalized(typedAddress) == address else { throw Refusal.confirmationDidNotMatch }
        let message = Data(("Aether local node key rebind approval v1\n"
                            + "validator: \(address)\ndata: \(dataDirectory)\nchallenge: \(UUID().uuidString)\n").utf8)
        try authenticate(message)
        return Approval(validatorAddress: address, dataDirectory: dataDirectory,
                        confirmationLine: typedAddress.trimmingCharacters(in: .whitespacesAndNewlines) + "\n")
    }
}

/// Where the block data lives, as the start gate sees it.
enum NodeStorageState: Equatable {
    /// Application Support on the internal disk.
    case standard
    /// A disk the person chose: its volume name, whether it is mounted (the
    /// folder exists), and whether this app may write there.
    case chosen(volume: String, mounted: Bool, writable: Bool)
}

/// Everything the start gate reads, gathered by `NodeController` every power
/// tick (30 s) and on every event that can change it.
struct NodeResumeFacts: Equatable {
    var enabled = true
    var wrongLocation = false
    var hasBinary = true
    var migrating = false
    var migrationGate: String?
    var movingStoragePercent: Int?
    /// Our own child process is alive.
    var processRunning = false
    /// The alive node has paused signing while it retries Mac verification.
    var confirmingMac = false
    /// Attached to a node someone else started (the unattended daemon's).
    var attached = false
    /// `run.lock` is held by a process that is not our child: an exclusive
    /// non-blocking `flock` probe failed.
    var lockHeldByOther = false
    /// Our own start just exited 7 (run.lock held) and the holder did not
    /// answer: something that is not a node we can attach to has the data.
    var lockRefused = false
    /// A watchdog restart is scheduled and its timer is still valid.
    var restartInSeconds: Int?
    var onlyOnPower = true
    var onBattery = false
    var isValidator = false
    /// The watchdog's terminal decision, and how long ago it was made.
    var blocked: NodeWatchdog.Failure?
    /// A proven mismatch persisted by the node, including exits the wallet
    /// did not observe because it was attached to the unattended daemon.
    var keyBindingRefused = false
    var blockedForSeconds: Int = 0
    /// Free space on the volume that holds the block data, and its name
    /// (nil: the internal disk).
    var freeBytes: UInt64?
    var volumeName: String?
    var storage: NodeStorageState = .standard
    /// `launchFailed`'s OS error from the last attempt, if that is all.
    var launchError: String?
}

/// The start gate's verdict.
enum NodeResumeDecision: Equatable {
    /// A node runs (ours or the one we are attached to): nothing to do.
    case keepRunning
    /// Start our node now. `detach` first when we were attached to a node
    /// that is gone.
    case start(detach: Bool)
    /// Do not start; this is why (shown everywhere, logged).
    case wait(NodeStopReason)
}

/// The start gate (docs/design/24-self-healing.md): every path that leaves
/// the switch on without a running node ends here, every 30 s, so a state
/// the app forgot to undo can no longer park the node forever.
enum NodeResume {
    /// The node's own write floor and resume level (`resources.rs`:
    /// `min_free_disk` 5 GB, `DISK_RESUME` +2 GB), as the app shows them.
    static let floorBytes: UInt64 = 5 * 1_073_741_824
    static let resumeBytes: UInt64 = 7 * 1_073_741_824
    /// A crash loop that a restart might fix (memory, network, unknown) is
    /// tried again after the watchdog's own crash window: ten quiet minutes.
    static let autoRetryAfter = 600

    static func decide(_ f: NodeResumeFacts) -> NodeResumeDecision {
        guard f.enabled else { return .wait(.switchedOff) }
        // This must precede attached takeover, retry timers and crash-loop
        // recovery. A terminal refusal does not expire when the app relaunches.
        if f.keyBindingRefused || f.blocked == .keyElsewhere { return .wait(.keyElsewhere) }
        if f.wrongLocation { return .wait(.wrongLocation) }
        // Attached to a node someone else started: it is alive exactly while
        // it holds run.lock. Once nobody does, it is gone — take the data
        // directory back now (the founder's MacBook, 2026-10-07: the app
        // attached to the previous app's node, which then exited with its
        // parent, and nothing ever started a node again).
        if f.attached { return f.lockHeldByOther ? .keepRunning : .start(detach: true) }
        if f.processRunning { return .keepRunning }
        guard f.hasBinary else { return .wait(.noHelper) }
        if let p = f.movingStoragePercent { return .wait(.movingStorage(percent: p)) }
        if f.migrating { return .wait(.migrating) }
        if let why = f.migrationGate { return .wait(.migrationBlocked(why)) }
        if case .chosen(let volume, let mounted, let writable) = f.storage {
            if !mounted { return .wait(.diskMissing(volume: volume)) }
            if !writable { return .wait(.diskNoAccess(volume: volume)) }
        }
        if f.onlyOnPower && f.onBattery && !f.isValidator { return .wait(.onBattery) }
        if let s = f.restartInSeconds { return .wait(.restarting(inSeconds: s)) }
        if let failure = f.blocked {
            switch failure {
            case .diskFull:
                // The node's own resume level, not a bigger app-only one:
                // the two used to disagree (10 GB here, 7 GB in the node).
                let free = f.freeBytes ?? 0
                return free >= resumeBytes ? .start(detach: false)
                    : .wait(.diskFull(freeBytes: free, resumeBytes: resumeBytes, volume: f.volumeName))
            case .upgradeNeeded: return .wait(.upgradeNeeded)
            case .identityLost: return .wait(.identityLost)
            case .keyElsewhere: return .wait(.keyElsewhere)
            case .database, .handoff, .storage: return .wait(.needsAttention(failure))
            case .alreadyRunning:
                return f.lockHeldByOther ? .wait(.otherNodeRunning) : .start(detach: false)
            case .memory, .network, .other:
                // Used to block until the app was relaunched by hand.
                let left = autoRetryAfter - f.blockedForSeconds
                return left <= 0 ? .start(detach: false) : .wait(.crashLoop(failure, retryInSeconds: left))
            }
        }
        // A held lock our last start already bounced off, with a holder that
        // does not answer: say who has it instead of spawning node after node.
        if f.lockHeldByOther && f.lockRefused { return .wait(.otherNodeRunning) }
        // A held lock with nothing blocked: start anyway — the node exits 7
        // at once and the app attaches to the holder if it answers.
        return .start(detach: false)
    }
}

/// `node-status.log` in the node's data folder: one line per change of the
/// node's state, append-only, capped — so a Mac can be diagnosed remotely
/// from a file (the founder's MacBook could not be, on 2026-10-07).
enum NodeStatusLog {
    static let fileName = "node-status.log"
    /// The file never grows past this; the oldest half is dropped.
    static let cap = 128 * 1024

    /// One line: UTC time, the reason's code, its English sentence, and the
    /// facts that decided it.
    static func line(at date: Date, event: String, detail: String, facts: NodeResumeFacts?) -> String {
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        var s = "\(iso.string(from: date)) \(event) \(detail.replacingOccurrences(of: "\n", with: " "))"
        if let f = facts {
            let free = f.freeBytes.map { NodeStopReason.gb($0) } ?? "?"
            let storage: String
            switch f.storage {
            case .standard: storage = "internal"
            case .chosen(let v, let m, let w): storage = "\(v)(mounted=\(String(m)),writable=\(w))"
            }
            s += " | enabled=\(f.enabled) proc=\(f.processRunning) confirmingMac=\(f.confirmingMac) attached=\(f.attached) lockOther=\(f.lockHeldByOther)"
                + " keyBindingRefused=\(f.keyBindingRefused)"
                + " battery=\(f.onBattery) onlyOnPower=\(f.onlyOnPower) blocked=\(f.blocked.map { "\($0)" } ?? "-")"
                + " blockedFor=\(f.blockedForSeconds)s restartIn=\(f.restartInSeconds.map(String.init) ?? "-")"
                + " free=\(free) storage=\(storage) migrating=\(f.migrating) gate=\(f.migrationGate == nil ? "open" : "shut")"
        }
        return s + "\n"
    }

    /// `existing` plus `line`, trimmed to `cap` by dropping whole lines from
    /// the front.
    static func appending(_ existing: Data, line: String, cap: Int = cap) -> Data {
        var data = existing
        data.append(Data(line.utf8))
        guard data.count > cap else { return data }
        let keepFrom = data.count - cap / 2
        let tail = data[keepFrom...]
        if let nl = tail.firstIndex(of: UInt8(ascii: "\n")) {
            return Data(data[(nl + 1)...])
        }
        return Data(tail)
    }

    /// Append to `<dir>/node-status.log` (best effort: a log must never stop
    /// the node). Never creates `dir`: an EastSea/node folder made before the
    /// Aether data move would turn the move into the slow verified copy.
    static func append(_ line: String, in dir: URL, fileName: String = NodeStatusLog.fileName) {
        var isDir: ObjCBool = false
        guard FileManager.default.fileExists(atPath: dir.path, isDirectory: &isDir), isDir.boolValue else { return }
        let url = dir.appendingPathComponent(fileName)
        let existing = (try? Data(contentsOf: url)) ?? Data()
        try? appending(existing, line: line).write(to: url, options: .atomic)
    }
}
