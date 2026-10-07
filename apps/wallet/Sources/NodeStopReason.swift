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
        case .switchedOff, .onBattery, .migrating, .restarting, .movingStorage: return false
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
    static func wait(_ seconds: Int, ko: Bool) -> String {
        if seconds >= 90 {
            let m = (seconds + 59) / 60
            return ko ? "\(m)분" : "\(m) min"
        }
        return ko ? "\(max(seconds, 1))초" : "\(max(seconds, 1)) s"
    }

    func copy(ko: Bool) -> NodeStopCopy {
        switch self {
        case .switchedOff:
            return NodeStopCopy(title: ko ? "꺼져 있음" : "Off",
                                detail: ko ? "켜면 이 Mac이 블록을 직접 확인해요." : "Turn it on and this Mac checks every block itself.",
                                resume: "", action: .turnOn, actionLabel: ko ? "켜기" : "Turn On")
        case .onBattery:
            return NodeStopCopy(title: ko ? "배터리 사용 중 · 쉬는 중" : "On battery · resting",
                                detail: ko ? "전원 어댑터에서만 돌도록 설정되어 있어요." : "It is set to run only on the power adapter.",
                                resume: ko ? "전원을 연결하면 30초 안에 저절로 다시 시작해요." : "Plug in and it restarts by itself within 30 s.",
                                action: .runOnBattery, actionLabel: ko ? "배터리에서도 실행" : "Run on Battery Too")
        case .wrongLocation:
            return NodeStopCopy(title: ko ? "응용 프로그램 폴더 밖에서 실행 중" : "Not in Applications",
                                detail: ko ? "디스크 이미지나 다운로드 폴더에서는 노드를 돌릴 수 없어요. 응용 프로그램 폴더로 옮긴 뒤 다시 열어 주세요."
                                    : "The node cannot run from a disk image or the Downloads folder. Move the app to Applications and open it again.",
                                resume: "", action: .showInFinder, actionLabel: ko ? "Finder에서 보기" : "Show in Finder")
        case .noHelper:
            return NodeStopCopy(title: ko ? "노드 프로그램이 없음" : "Node missing",
                                detail: ko ? "이 앱 안에 노드 프로그램이 빠져 있어요. 앱을 업데이트하거나 다시 설치해 주세요."
                                    : "This copy of the app is missing its node. Update or reinstall the app.",
                                resume: "", action: .checkForUpdates, actionLabel: ko ? "업데이트 확인" : "Check for Updates")
        case .migrating:
            return NodeStopCopy(title: ko ? "이전 데이터 옮기는 중" : "Moving your old data",
                                detail: ko ? "Aether의 지갑과 노드 데이터를 옮기고 있어요." : "Your Aether wallet and node data are moving over.",
                                resume: ko ? "끝나면 저절로 시작해요." : "The node starts by itself when it is done.",
                                action: nil, actionLabel: nil)
        case .migrationBlocked(let why):
            return NodeStopCopy(title: ko ? "이전 데이터 이동이 끝나지 않음" : "Old data not moved yet",
                                detail: why,
                                resume: ko ? "30초마다 다시 확인해요." : "Checked again every 30 s.",
                                action: .retryNow, actionLabel: ko ? "지금 다시 시도" : "Try Now")
        case .otherNodeRunning:
            return NodeStopCopy(title: ko ? "다른 프로그램이 노드 데이터를 사용 중" : "Another program has the node data",
                                detail: ko ? "다른 노드 프로그램(이전 Aether 앱 등)이 같은 데이터를 쓰고 있어요. 그 프로그램을 종료해 주세요."
                                    : "Another node program (such as the old Aether app) is using the same data. Quit it.",
                                resume: ko ? "그 프로그램이 끝나면 30초 안에 저절로 시작해요." : "The node starts by itself within 30 s of it quitting.",
                                action: .retryNow, actionLabel: ko ? "지금 다시 시도" : "Try Now")
        case .diskFull(let free, let resume, let volume):
            let on = volume.map { ko ? "‘\($0)’ " : "“\($0)”: " } ?? ""
            return NodeStopCopy(title: ko ? "저장 공간 부족 · 노드 쉬는 중" : "Storage low · node resting",
                                detail: ko ? "\(on)저장 공간 \(Self.gb(free)) 남음. 약 \(Self.need(free: free, resume: resume))만 더 비워 주세요."
                                    : "\(on)\(Self.gb(free)) free. Free about \(Self.need(free: free, resume: resume)) more.",
                                resume: ko ? "\(Self.gb(resume))가 되면 저절로 다시 시작해요." : "It restarts by itself at \(Self.gb(resume)).",
                                action: .openStorage, actionLabel: ko ? "저장 공간 관리" : "Manage Storage")
        case .diskMissing(let volume):
            return NodeStopCopy(title: ko ? "‘\(volume)’ 디스크가 연결되지 않음" : "Disk “\(volume)” not connected",
                                detail: ko ? "블록 데이터가 이 디스크에 있어요. 내장 디스크로 몰래 다시 받지 않아요."
                                    : "The block data lives on this disk. Nothing is re-downloaded to the internal disk behind your back.",
                                resume: ko ? "디스크를 연결하면 저절로 다시 시작해요." : "Connect it and the node restarts by itself.",
                                action: .chooseDisk, actionLabel: ko ? "다른 위치 선택" : "Choose Another Location")
        case .diskNoAccess(let volume):
            return NodeStopCopy(title: ko ? "‘\(volume)’에 접근할 수 없음" : "No access to “\(volume)”",
                                detail: ko ? "시스템 설정 › 개인정보 보호 및 보안 › 파일 및 폴더에서 \(Brand.projectKo)의 ‘이동식 볼륨’을 켜 주세요."
                                    : "Turn on “Removable Volumes” for \(Brand.project) in System Settings › Privacy & Security › Files and Folders.",
                                resume: ko ? "켜면 30초 안에 저절로 시작해요." : "The node starts by itself within 30 s.",
                                action: .openPrivacySettings, actionLabel: ko ? "시스템 설정 열기" : "Open System Settings")
        case .restarting(let s):
            return NodeStopCopy(title: ko ? "노드 다시 시작하는 중" : "Restarting the node",
                                detail: ko ? "노드가 멈춰서 다시 시작해요." : "The node stopped, so it is restarting.",
                                resume: ko ? "\(Self.wait(s, ko: ko)) 뒤에 시작해요." : "Starting in \(Self.wait(s, ko: ko)).",
                                action: nil, actionLabel: nil)
        case .crashLoop(let failure, let s):
            return NodeStopCopy(title: ko ? "노드가 계속 멈춤" : "The node keeps stopping",
                                detail: Self.crashDetail(failure, ko: ko),
                                resume: ko ? "\(Self.wait(s, ko: ko)) 뒤 저절로 다시 시도해요." : "It tries again by itself in \(Self.wait(s, ko: ko)).",
                                action: .retryNow, actionLabel: ko ? "지금 다시 시도" : "Try Now")
        case .needsAttention(let failure):
            return NodeStopCopy(title: ko ? "노드 데이터에 문제가 있음" : "The node's data needs attention",
                                detail: failure.sentence,
                                resume: ko ? "고친 뒤 ‘다시 시도’를 눌러 주세요." : "After fixing it, press Try Again.",
                                action: .retryNow, actionLabel: ko ? "다시 시도" : "Try Again")
        case .upgradeNeeded:
            return NodeStopCopy(title: ko ? "업데이트 필요" : "Update needed",
                                detail: ko ? "네트워크 규칙이 바뀌어 이 버전의 노드로는 따라갈 수 없어요." : "The network's rules changed and this version's node cannot follow them.",
                                resume: ko ? "업데이트가 설치되면 저절로 시작해요." : "It starts by itself once the update is installed.",
                                action: .checkForUpdates, actionLabel: ko ? "업데이트 확인" : "Check for Updates")
        case .identityLost:
            return NodeStopCopy(title: ko ? "노드 키를 읽을 수 없음" : "Node key unreadable",
                                detail: NodeWatchdog.Failure.identityLost.sentence,
                                resume: "", action: .copyDiagnostics, actionLabel: ko ? "진단 정보 복사" : "Copy Diagnostics")
        case .launchFailed(let why):
            return NodeStopCopy(title: ko ? "노드를 시작하지 못함" : "The node could not start",
                                detail: (ko ? "macOS가 노드 실행을 거부했어요: " : "macOS refused to launch the node: ") + why,
                                resume: ko ? "30초마다 다시 시도해요." : "Tried again every 30 s.",
                                action: .retryNow, actionLabel: ko ? "지금 다시 시도" : "Try Now")
        case .movingStorage(let p):
            return NodeStopCopy(title: ko ? "블록 데이터 옮기는 중 · \(p)%" : "Moving block data · \(p)%",
                                detail: ko ? "블록 데이터를 새 위치로 복사하고 확인하는 중이에요. 지갑은 그동안에도 써요."
                                    : "The block data is being copied and checked at its new place. The wallet keeps working.",
                                resume: ko ? "다 옮기면 저절로 시작해요." : "The node starts by itself when it is done.",
                                action: nil, actionLabel: nil)
        }
    }

    /// The crash loop's cause in one plain clause (the watchdog's own
    /// sentence for the specific kinds).
    private static func crashDetail(_ f: NodeWatchdog.Failure, ko: Bool) -> String {
        switch f {
        case .memory: return f.sentence
        case .network: return ko ? "네트워크에 닿지 못해 노드가 멈췄어요. 인터넷 연결을 확인해 주세요." : "The node could not reach the network. Check the internet connection."
        default: return ko ? "노드가 시작 직후 여러 번 멈췄어요. 잔액은 다른 노드로 계속 확인해요." : "The node stopped several times right after starting. Your balance is still checked through other nodes."
        }
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
            case .chosen(let v, let m, let w): storage = "\(v)(mounted=\(m),writable=\(w))"
            }
            s += " | enabled=\(f.enabled) proc=\(f.processRunning) attached=\(f.attached) lockOther=\(f.lockHeldByOther)"
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
