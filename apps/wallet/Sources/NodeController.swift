#if os(macOS)
import AppKit
import DeviceCheck
import Foundation
import IOKit.ps
import IOKit.pwr_mgt
import ServiceManagement
import SwiftUI
import UserNotifications

/// The node inside the app (Transmission-style on/off). On: the bundled `aether`
/// (`aether run`) follows the chain, verifying and re-executing every block on
/// this Mac, and the wallet talks to it. Once the owner registers the Mac as a
/// voting node, it also proves it is alive every epoch, and when the network
/// picks it, it becomes a validator by itself (and steps back the same way).
/// Off, or when the app quits: it stops, and the wallet goes back to asking
/// validators directly (still verifying everything).
@MainActor
final class NodeController: ObservableObject {
    enum State: Equatable {
        case off
        case starting
        case running
        /// On, but waiting for the power adapter (see `onlyOnPower`).
        case waitingForPower
        case failed(String)
    }

    @Published private(set) var state: State = .off
    @Published private(set) var height: UInt64 = 0
    /// The wallet is reading through this Mac's node without the remote
    /// cross-check (the view `authenticatedRemoteHeight()` reads could not
    /// answer — the incident of 2026-10-05): the route is provisional, the
    /// UI says so, and the watchdog releases it if the node stalls.
    @Published private(set) var networkCheckPending = false
    /// This Mac's voting-node identity (keys live in the node's data folder).
    @Published private(set) var candidate: Candidate?
    /// The registry's view of this Mac (refreshed while the node runs).
    @Published private(set) var voting: VotingNodeStatus?

    struct Candidate: Equatable, Sendable {
        let validatorKey: String
        let nodeId: String
        let beaconer: String
    }
    @AppStorage("nodeEnabled") var enabled = false {
        didSet {
            if enabled {
                automaticRestartBlocked = false
                startIfAllowed()
            } else {
                stop()
                // The node switch off must stop the daemon's node too, and
                // keep it stopped across the next restart: the marker goes
                // away (docs/design/29).
                unattended?.nodeSwitchedOff()
            }
        }
    }
    /// The unattended-restart half of the feature (docs/design/29): set by
    /// the app at launch. The node tells it when it runs (marker in sync) and
    /// hands it the daemon's node to stop when the user turns the node off.
    var unattended: UnattendedDaemon?
    /// Attached to the daemon-started node (unattended restart): it answers on
    /// the node's RPC port and holds `run.lock`, and this app did not start
    /// it. One node per data dir — while attached, `process` is nil and the
    /// app never starts a second one.
    @Published private(set) var attached = false
    /// Prove blocks on this Mac's GPU for rewards (protocol 2); paid to `proveAddress`.
    @AppStorage("proveBlocks") var prove = false {
        didSet { restartIfRunning() }
    }
    @AppStorage("proveAddress") var proveAddress = ""
    /// Settings ▸ 리소스 (docs/ops/resource-limits.md): the prover's memory cap
    /// ("auto" = RAM의 25%, GB, "off"), CPU share ("half"/"all"), and whether it
    /// may run on battery. Passed to the node as flags on (re)start.
    @AppStorage("proverMemory") var proverMemory = "auto" {
        didSet { restartIfRunning() }
    }
    @AppStorage("proverCores") var proverCores = "half" {
        didSet { restartIfRunning() }
    }
    @AppStorage("proverOnBattery") var proverOnBattery = false {
        didSet { restartIfRunning() }
    }
    /// What the prover did last (from the node's `aether_proverStatus`).
    @Published private(set) var prover: ProverStatus?
    /// 설정 ▸ 역사 보관 (docs/design/15-node-rewards.md "C. 보관"): how much
    /// of the network's past this Mac keeps, in the user's words. The choice
    /// maps to the node's `--max-shards` (`StorageSetting`) and applies on
    /// the node's next start; the daemon's marker follows the change at once,
    /// so an unattended restart runs with the same budget. Nothing is lost by
    /// lowering it — the node drops only the shards beyond its assignment.
    @AppStorage("historyStorage") var historyStorage = StorageSetting.defaultChoice {
        didSet { pushStorageSetting() }
    }
    /// What this Mac keeps right now and how it has been checking out
    /// (`aether_shardStats`): the honest line the 역사 보관 setting stands on.
    @Published private(set) var history: HistoryKept?

    /// The shard budget both nodes (this app's child and the daemon's) run
    /// with: the stored choice resolved against the registry (a registered
    /// Mac has no "off") and the data volume's free space. One resolver feeds
    /// `start` and the daemon's marker, so the two argvs cannot drift.
    var storageShards: Int {
        StorageSetting.resolve(choice: historyStorage,
                               registered: voting?.registered == true,
                               freeBytes: StorageSetting.freeBytes(atPath: Self.dataDir.path))
    }

    /// What this Mac keeps of the network's past, for one line in Settings.
    /// `passPercent` is this Mac's own row in `aether_shardStats` when it has
    /// been checked, else the pass rate this Mac observed across the network's
    /// holders — phase 1 keeps challenge results only where they were asked.
    struct HistoryKept: Equatable, Sendable {
        let bytes: UInt64
        let shards: Int
        let windowDays: Int
        let passPercent: Int?
    }

    private struct ShardStatsJSON: Decodable {
        let cap: Int?
        let window_days: Int?
        let held: [Held]?
        let held_bytes: UInt64?
        let me: String?
        let candidates: [Candidate]?
        struct Held: Decodable { let era: UInt64; let shard: UInt32 }
        struct Candidate: Decodable { let node: String; let checked: Int; let ok: Int; let failed: Int }
    }

    /// Hand the resolved budget to the daemon (its marker is written without
    /// the node's registry view) and rewrite the marker right away: a reboot
    /// before the node's next start must not resurrect the old budget.
    private func pushStorageSetting() {
        unattended?.storageShards = storageShards
        unattended?.syncMarker()
    }
    /// The node's data volume is below its free-space floor (`aether_status`):
    /// no new era files or shards, proving paused — shown as "디스크 공간 부족".
    @Published private(set) var diskLow = false
    /// The two halves of `disk_low` (`aether_status` ▸ resources), for the
    /// health checks (docs/design/32-health-signal.md L3/L4): within 3 GB of
    /// the floor, and below it — writes held, the node's part paused. A node
    /// too old to tell them apart reports `disk_low` only, read as paused:
    /// the healthy badge must never show on a half-dead node by mistake.
    @Published private(set) var diskAlmostFull = false
    @Published private(set) var diskPaused = false
    /// Why the watchdog stopped restarting (layer 4), until the next start.
    @Published private(set) var stoppedFailure: NodeWatchdog.Failure?
    /// Stall restarts so far: the health check counts them (L5).
    @Published private(set) var stallRestarts = 0
    /// The node's RPC answered the latest poll, and has answered at least once
    /// since the node started (L6 tells a silent node from a starting one).
    @Published private(set) var rpcAnswering = true
    @Published private(set) var answeredSinceStart = false
    /// The chain needs a newer node than this app carries (exit 3/5, or a
    /// scheduled protocol above the node's own): L8.
    @Published private(set) var upgradeRequired = false
    /// The protocol this app's node runs, as it last said (diagnostics).
    private(set) var nodeProtocol: UInt64?
    /// Called when the chain schedules a protocol this app's node does not run
    /// (or the node stopped for it): look for the signed update right away.
    var onUpgradeNeeded: (() -> Void)?
    private var upgradeAsked = false

    struct ProverStatus: Decodable, Equatable, Sendable {
        let running: Bool
        let proving: UInt64?
        let last_height: UInt64?
        let last_txs: Int?
        let last_seconds: Double?
        let proofs: UInt64?
        let proofs_failing: Bool?
        let acceptance_rate_percent: UInt8?
        let program_unknown: Bool?
        let program_mismatch: Bool?
        /// The validators' guest program ID (the diagnostics' first 8 digits).
        let network_program: String?
        let error: String?
        /// Why proving is paused right now ("memory", "pressure", "battery", "disk", "program").
        let paused: String?
        /// The sidecar's physical footprint at the last sample, and its cap.
        let memory_bytes: UInt64?
        let memory_cap: UInt64?
        /// Blocks between the chain head and the last block proven here.
        let lag: UInt64?
        /// The last reward received (wei, hex).
        let last_reward: String?
    }

    /// Run the node only while the Mac is on its power adapter (laptops).
    @AppStorage("nodeOnlyOnPower") var onlyOnPower = true {
        didSet { if enabled { applyPower() } }
    }
    /// Open EastSea at login (the node then resumes if it was on).
    var startAtLogin: Bool {
        get { SMAppService.mainApp.status == .enabled }
        set {
            if wrongLocation, newValue {
                // Red team #10: never point a login item at a place that
                // disappears (a DMG, a translocated copy). The toggle reads
                // back off, and the move sentence says why.
                return
            }
            do {
                if newValue { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
            } catch {
                state = .failed("Login item: \(error.localizedDescription)")
            }
            objectWillChange.send()
        }
    }
    /// Durations — the stall clock, the crash window, retry throttles — run
    /// on this never-jumping clock (red team #6). Wall-clock `Date` stays
    /// only where a person reads it.
    private let clock: Clock
    /// Red team #10: the app is running from a place it cannot live in — a
    /// mounted DMG, a translocated Downloads copy, a read-only volume. From
    /// there the node never starts and no login item is registered; the
    /// wallet keeps working through other nodes and one sentence says what
    /// to do. The wallet's read-only views are never blocked for this.
    @Published private(set) var wrongLocation: Bool

    init(clock: Clock = UptimeClock()) {
        self.clock = clock
        wrongLocation = !InstallLocation.currentIsRunnable
        if wrongLocation { state = .failed(InstallLocation.moveSentence) }
    }
    private var powerTimer: Timer?
    /// Held while this Mac is a validator (see `applyDuty`).
    let sleepGuard = SleepGuard(reason: "\(Brand.project): this Mac signs blocks for the network (voting node)")
    /// A "network is paused" notice was posted and its "running again" is due.
    var pauseNotified = false

    static var onBattery: Bool {
        guard let info = IOPSCopyPowerSourcesInfo()?.takeRetainedValue(),
              let type = IOPSGetProvidingPowerSourceType(info)?.takeUnretainedValue() as String? else { return false }
        return type == kIOPSBatteryPowerValue
    }

    private func startIfAllowed() {
        powerTimer?.invalidate()
        powerTimer = Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.applyPower() }
        }
        applyPower()
    }

    /// Start or pause for the power source; called every 30 s while the switch is on.
    private func applyPower() {
        guard enabled else { return }
        if attached {
            // Unattended beats the adapter rule (docs/design/29): the daemon's
            // node keeps a voting Mac alive through restarts, on battery too.
            state = .running
            return
        }
        if automaticRestartBlocked && watchdog.lastFailure == .diskFull,
           let attrs = try? FileManager.default.attributesOfFileSystem(forPath: Self.dataDir.path),
           let free = attrs[.systemFreeSize] as? NSNumber,
           NodeWatchdog.storageRecovered(freeBytes: free.uint64Value) {
            automaticRestartBlocked = false
        }
        if onlyOnPower && Self.onBattery {
            // Keep voting until the announced handoff lands. Stopping a seated
            // Mac before the old quorum signs can stall the whole committee.
            if isValidator { return }
            if process != nil { stop(keepSwitch: true) }
            state = .waitingForPower
        } else if process == nil, restartTimer == nil, !automaticRestartBlocked {
            // A watchdog restart already scheduled keeps its backoff.
            start()
        }
    }

    static let port: UInt16 = 18_545

    func refreshWalletRoute() {
        guard !UserDefaults.standard.bool(forKey: "useDevelopmentNetwork") else { return }
        useLocalNode(port: switched ? Self.port : nil)
    }
    /// The health check's L6 (docs/design/32-health-signal.md): the process
    /// lives but its RPC has been silent for 30 s — restart it, once per
    /// incident; the watchdog's backoff governs everything after.
    func restartUnresponsive() {
        restartIfRunning()
    }

    /// The newest protocol the chain has scheduled, as last heard (diagnostics).
    var newestScheduledProtocol: UInt64? { scheduledProtocol }

    /// Validator-to-validator port, used only while this Mac is voting.
    static let p2pPort: UInt16 = 19_101
    private(set) var process: Process?
    private var poll: Timer?

    static var dataDir: URL {
        DataMigration.ensure()
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/EastSea/node", isDirectory: true)
    }

    /// The note the Connected status appends while `networkCheckPending`:
    /// one phrase, ko/en, localized like `NodeWatchdog.Failure.sentence`.
    var pendingRouteNote: String {
        let ko = Locale.preferredLanguages.first?.hasPrefix("ko") ?? false
        return ko ? "· 이 Mac의 노드 사용 (네트워크 확인 대기)" : "· via this Mac's node; network check pending"
    }

    private var binary: URL? {
        let helpers = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers")
        let name = usePreviousBinary ? "aether.prev" : "aether"
        let helper = helpers.appendingPathComponent(name)
        return FileManager.default.isExecutableFile(atPath: helper.path) ? helper : nil
    }

    /// The bundled node binary for anything outside this controller that
    /// needs the very binary the app runs (the unattended marker). The
    /// rollback choice (`aether.prev`) stays private to `binary`.
    static var helperBinaryURL: URL? {
        let helper = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/aether")
        return FileManager.default.isExecutableFile(atPath: helper.path) ? helper : nil
    }

    #if DEBUG
    /// Design preview: looks like a running node, runs nothing.
    func loadPreview() {
        state = .running
        height = 184_210
    }
    #endif

    /// What the app does when its node stops, stalls, or dies over and over
    /// (docs/design/24-self-healing.md layer 2).
    private var watchdog = NodeWatchdog()
    /// A watchdog-ordered restart is pending (its backoff is running).
    private var restartTimer: Timer?
    /// Consecutive polls the attached (daemon-started) node did not answer
    /// (docs/design/29): a few are a restart under the daemon, five take the
    /// data directory back.
    private var attachMisses = 0
    /// A terminal watchdog decision must also gate the periodic power timer.
    private var automaticRestartBlocked = false
    /// The last update's binary kept beside the current one: rolled back to
    /// when the new one cannot start (docs/design/24-self-healing.md layer 2).
    @Published private(set) var usePreviousBinary = false
    /// The "this Mac's node key cannot be read" notice went out (once per
    /// bout; red team #5 — a person must restore the key).
    private var identityNoticePosted = false
    /// Monotonic (red team #6): a wall-clock jump must not postpone the
    /// key-loss re-check for hours or fire it instantly.
    private var nextCandidateRetry = MonotonicInstant.distantPast
    /// Sleep/wake observers (red team #9): the watchdog's timing is stale the
    /// moment the Mac sleeps. Added once, kept for the app's lifetime.
    private var wakeObservers: [NSObjectProtocol] = []
    private var powerSourceSource: CFRunLoopSource?

    /// The node signs and gossips this local request with its registered voting
    /// key. A signal wakes its one-second loop before macOS suspends the process.
    private func announceAvailability(leaving: Bool) {
        let file = Self.dataDir.appendingPathComponent("availability-state")
        try? Data((leaving ? "leaving" : "back").utf8).write(to: file, options: .atomic)
        if case .running = state, candidate != nil, let process, process.isRunning {
            Darwin.kill(process.processIdentifier, SIGUSR1)
        }
    }

    /// The newest protocol the chain has scheduled, as this app last heard it
    /// from its own node (`aether_status.newest_scheduled`), kept across
    /// restarts: the rollback decision needs it exactly when the node can no
    /// longer answer (red team #3).
    private var scheduledProtocol: UInt64? {
        get { UserDefaults.standard.string(forKey: "nodeScheduledProtocol").flatMap(UInt64.init) }
        set { UserDefaults.standard.set(newValue.map(String.init), forKey: "nodeScheduledProtocol") }
    }

    /// The protocol a binary implements (`aether protocol` — no chain, no
    /// data): asked of the previous binary before returning to it (red team
    /// #3). Synchronous; call off the main actor.
    nonisolated static func protocolOf(_ binary: URL) -> UInt64? {
        let p = Process(), out = Pipe()
        p.executableURL = binary
        p.arguments = ["protocol"]
        p.standardOutput = out
        guard (try? p.run()) != nil else { return nil }
        p.waitUntilExit()
        guard p.terminationStatus == 0,
              let line = String(data: out.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)?
                  .trimmingCharacters(in: .whitespacesAndNewlines) else { return nil }
        return UInt64(line)
    }

    /// Resume the user's choice at launch.
    func restore() {
        if enabled { startIfAllowed() }
    }

    func start() {
        guard !wrongLocation else {
            // Red team #10: from a DMG/Downloads/read-only place the node's
            // data would point into a bundle that disappears. One sentence;
            // the wallet keeps working through other nodes.
            state = .failed(InstallLocation.moveSentence)
            return
        }
        guard process == nil else { return }
        guard let binary else {
            state = .failed("This build does not include the node")
            return
        }
        // Audit 5, A5-7: while the old Aether node data (identity, threshold
        // share, chain) waits unmigrated, starting fresh here would strand
        // this Mac's validator identity. The migration itself already ran
        // (Self.dataDir) — this catches its deferred/failed outcome.
        if let why = DataMigration.mayStartNode() {
            state = .failed(why)
            return
        }
        do {
            try FileManager.default.createDirectory(at: Self.dataDir, withIntermediateDirectories: true)
        } catch {
            state = .failed("\(error.localizedDescription)")
            return
        }
        announceAvailability(leaving: Self.onBattery)
        loadCandidate(binary)
        nextCandidateRetry = clock.now.advanced(by: 60)
        // The same argv the daemon would run (UnattendedDecision.nodeArgv is
        // the single source), plus the app-child-only --exit-with-parent.
        var args = UnattendedDecision.nodeArgv(
            dataDir: Self.dataDir.path,
            rpcPort: Self.port,
            p2pPort: Self.p2pPort,
            networkPath: Bundle.main.url(forResource: "network", withExtension: "json")?.path,
            proverFlags: ProverFlags.build(memory: proverMemory, cores: proverCores, battery: proverOnBattery,
                                           activeProcessors: ProcessInfo.processInfo.activeProcessorCount),
            storageFlag: StorageSetting.flag(shards: storageShards))
        args += ["--exit-with-parent"]
        unattended?.nodeSwitchedOn()
        let p = Process()
        p.executableURL = binary
        p.arguments = args
        if prove, !proveAddress.isEmpty {
            // The node proves blocks with the bundled aether-prover and pays this address.
            var env = ProcessInfo.processInfo.environment
            env["AETHER_PROVE"] = proveAddress
            p.environment = env
        }
        let log = Self.dataDir.appendingPathComponent("node.log")
        FileManager.default.createFile(atPath: log.path, contents: nil)
        if let h = try? FileHandle(forWritingTo: log) {
            p.standardOutput = h
            p.standardError = h
        }
        p.terminationHandler = { [weak self] proc in
            Task { @MainActor in self?.exited(proc) }
        }
        do {
            try p.run()
        } catch {
            state = .failed(error.localizedDescription)
            return
        }
        process = p
        watchdog.started(clock.now)
        stoppedFailure = nil
        answeredSinceStart = false
        rpcAnswering = true
        state = .starting
        watchSleep()
        poll = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.check() }
        }
        refreshDeviceToken()
        tokenTimer?.invalidate()
        tokenTimer = Timer.scheduledTimer(withTimeInterval: 3600, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refreshDeviceToken() }
        }
    }

    private var tokenTimer: Timer?

    /// Daily re-attestation (docs/design/15-node-rewards.md): once a day the node
    /// proves with a fresh Apple DeviceCheck token that it still runs on a real,
    /// registered Mac. Only the app can make one, so it leaves one every hour
    /// where the node looks for it (owner-readable only).
    private func refreshDeviceToken() {
        guard DCDevice.current.isSupported else { return }
        let file = Self.dataDir.appendingPathComponent("devicecheck-token")
        Task.detached {
            guard let token = try? await DCDevice.current.generateToken() else { return }
            let fm = FileManager.default
            let tmp = file.appendingPathExtension("tmp")
            guard fm.createFile(atPath: tmp.path, contents: Data(token.base64EncodedString().utf8), attributes: [.posixPermissions: 0o600]) else { return }
            if fm.fileExists(atPath: file.path) {
                _ = try? fm.replaceItemAt(file, withItemAt: tmp)
            } else {
                try? fm.moveItem(at: tmp, to: file)
            }
        }
    }

    func stop(keepSwitch: Bool = false) {
        if attached {
            // Detach only: the daemon's node is the point of the unattended
            // restart — quitting the app must not stop it (docs/design/29).
            // The node switch being turned off stops it (`nodeSwitchedOff`).
            attached = false
            poll?.invalidate()
            tokenTimer?.invalidate()
            restartTimer?.invalidate()
            poll = nil
            restartTimer = nil
            switched = false
            watchdog.invalidate()
            if !UserDefaults.standard.bool(forKey: "useDevelopmentNetwork") { useLocalNode(port: nil) }
            state = .off
            applyDuty()
            return
        }
        if !keepSwitch {
            powerTimer?.invalidate()
            powerTimer = nil
            // Switched off on purpose: the last crash loop is no longer an
            // incident to show (the health banner reads this).
            stoppedFailure = nil
        }
        poll?.invalidate()
        tokenTimer?.invalidate()
        restartTimer?.invalidate()
        poll = nil
        restartTimer = nil
        switched = false
        networkCheckPending = false
        watchdog.invalidate()
        if !UserDefaults.standard.bool(forKey: "useDevelopmentNetwork") { useLocalNode(port: nil) }
        if let p = process, p.isRunning { p.terminate() }
        process = nil
        state = .off
        applyDuty()
    }

    /// The tail of the node's log: what the watchdog reads to tell a full disk
    /// from a damaged database when the node exits with the storage code.
    private func nodeLogTail(_ bytes: Int = 8_192) -> String {
        guard let h = try? FileHandle(forReadingFrom: Self.dataDir.appendingPathComponent("node.log")) else { return "" }
        defer { try? h.close() }
        let size = (try? h.seekToEnd()) ?? 0
        try? h.seek(toOffset: max(0, size - UInt64(bytes)))
        return String(data: h.readDataToEndOfFile(), encoding: .utf8) ?? ""
    }

    private func exited(_ proc: Process) {
        // Stopped on purpose, or an older process (after a restart) finishing late.
        guard let current = process, current === proc else { return }
        let status = proc.terminationStatus
        if status == 3 || status == 5 {  // UPGRADE REQUIRED / no proof verifier (see `watch_upgrades`, `install_verifier`)
            upgradeRequired = true
            onUpgradeNeeded?()
        }
        process = nil
        poll?.invalidate()
        switched = false
        networkCheckPending = false
        if !UserDefaults.standard.bool(forKey: "useDevelopmentNetwork") { useLocalNode(port: nil) }  // the wallet reads other nodes from this moment on
        applyDuty()
        if status == UnattendedDecision.lockExitCode {
            // One data directory, one node (`run.lock`, red team #12): the
            // daemon's node of the unattended restart already runs it
            // (docs/design/29). Attach to it when it answers on RPC; if
            // nothing does, the holder is dying — start our own again.
            let port = Self.port
            Task.detached {
                let alive = await Self.nodeAnswersRpc(port: port)
                await MainActor.run {
                    guard self.enabled, self.process == nil, !self.automaticRestartBlocked else { return }
                    if UnattendedDecision.afterLockExit(rpcAlive: alive) == .attach {
                        self.attachToRunningNode()
                    } else {
                        self.start()
                    }
                }
            }
            return
        }
        switch watchdog.exited(clock.now, code: status, signaled: proc.terminationReason == .uncaughtSignal, log: nodeLogTail()) {
        case .restart(let after):
            // Restart with backoff (docs/design/24-self-healing.md layer 2):
            // the wallet is on remote nodes already, so a few seconds cost
            // nothing but a crash loop.
            state = .failed("The node stopped (exit \(status)); restarting it")
            watchdog.restarting()
            restartTimer = Timer.scheduledTimer(withTimeInterval: max(after, 0.05), repeats: false) { [weak self] _ in
                Task { @MainActor in
                    guard let self, self.process == nil, self.restartTimer != nil else { return }
                    self.restartTimer = nil
                    self.start()
                }
            }
        case .stop(let failure):
            // Too many deaths: stop restarting, one plain sentence (layer 4).
            automaticRestartBlocked = true
            stoppedFailure = failure
            state = .failed(failure.sentence)
        case .rollback:
            // The updated binary cannot start: back to the previous one —
            // once, and only if that binary can still run the chain (red team
            // #3): an old binary on a chain it cannot read stops the node for
            // good, which is worse than the crash loop this was meant to fix.
            // Otherwise voting stops and the update is asked for again.
            let prev = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/aether.prev")
            if !usePreviousBinary, FileManager.default.isExecutableFile(atPath: prev.path) {
                let scheduled = scheduledProtocol
                Task.detached {
                    let prevProtocol = Self.protocolOf(prev)
                    await MainActor.run {
                        guard self.enabled, self.process == nil else { return }
                        if NodeWatchdog.rollbackAllowed(prevProtocol: prevProtocol, chainScheduled: scheduled) {
                            self.usePreviousBinary = true
                            self.state = .failed("업데이트 뒤 노드가 시작되지 않아 이전 버전으로 되돌렸습니다")
                            self.watchdog.restarting()
                            self.start()
                        } else {
                            self.automaticRestartBlocked = true
                            self.stoppedFailure = .upgradeNeeded
                            self.state = .failed(NodeWatchdog.Failure.upgradeNeeded.sentence)
                            self.upgradeAsked = true
                            self.onUpgradeNeeded?()
                        }
                    }
                }
            } else {
                automaticRestartBlocked = true
                stoppedFailure = .other
                state = .failed(NodeWatchdog.Failure.other.sentence)
            }
        case .none:
            state = .failed("The node stopped (exit \(status)); see \(Self.dataDir.appendingPathComponent("node.log").path)")
        }
    }

    /// Read (or create) this Mac's voting-node keys with the bundled helper.
    private func loadCandidate(_ binary: URL) {
        guard candidate == nil else { return }
        let p = Process(), out = Pipe()
        p.executableURL = binary
        p.arguments = ["candidate-info", "--data", Self.dataDir.path]
        p.standardOutput = out
        guard (try? p.run()) != nil else { return }
        p.waitUntilExit()
        if p.terminationStatus == 6 {
            // The node key cannot be read (red team #5): the helper refuses to
            // mint a replacement identity, and so does the app — a person
            // restores the key from a backup. The node (keyless) still runs
            // and the wallet still works; this says why it does not vote.
            if !identityNoticePosted {
                identityNoticePosted = true
                LocalNotice.post(title: "\(Brand.project)", body: NodeWatchdog.Failure.identityLost.sentence)
            }
            return
        }
        identityNoticePosted = false
        guard p.terminationStatus == 0,
              let v = try? JSONSerialization.jsonObject(with: out.fileHandleForReading.readDataToEndOfFile()) as? [String: Any],
              let key = v["validator_key"] as? String, let node = v["node_id"] as? String, let beaconer = v["beaconer"] as? String else { return }
        candidate = Candidate(validatorKey: key, nodeId: node, beaconer: beaconer)
    }

    /// The voting key's signature asking to be registered under `account` (the wallet, as operator).
    func ownership(account: String, chainId: UInt64) -> String? {
        guard let binary else { return nil }
        let p = Process(), out = Pipe()
        p.executableURL = binary
        p.arguments = ["candidate-info", "--data", Self.dataDir.path, "--operator", account, "--chain-id", String(chainId)]
        p.standardOutput = out
        guard (try? p.run()) != nil else { return nil }
        p.waitUntilExit()
        guard p.terminationStatus == 0,
              let v = try? JSONSerialization.jsonObject(with: out.fileHandleForReading.readDataToEndOfFile()) as? [String: Any] else { return nil }
        return v["ownership"] as? String
    }

    private var lastVotingCheck = MonotonicInstant.distantPast

    private func refreshVoting() {
        guard let key = candidate?.validatorKey, clock.now.elapsed(since: lastVotingCheck) > 10 else { return }
        lastVotingCheck = clock.now
        Task.detached {
            let status = try? votingNodeStatus(validatorKey: key)
            await MainActor.run {
                // Published only when it changed: the poll runs every 2 s.
                if let status, self.voting != status { self.voting = status }
                // The unattended-restart default follows the registry: a Mac
                // in or entering the voting set keeps running through
                // restarts unless the user chose otherwise (docs/design/29).
                if let status {
                    self.unattended?.applyDefault(registered: status.registered)
                    self.unattended?.storageShards = self.storageShards
                }
                self.applyDuty()
            }
        }
    }

    /// Switch the wallet to the local node once it has caught up with the network.
    private var switched = false
    /// One `check()` read at a time: a slow remote check must not stack
    /// detached tasks behind the 2 s poll (the incident's pile-up) — the
    /// first tick after one lands re-reads everything fresh.
    private var checkInFlight = false

    private func restartIfRunning() {
        if attached {
            // A stall in the node we are attached to: take it over. The
            // watchdog's layer-2 rule applies no matter who started the node
            // (docs/design/24): stop the daemon's node, then start our own.
            unattended?.stopDaemonNode()
            watchdog.restarting()
            let timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] t in
                Task { @MainActor in
                    guard let self, self.enabled, self.process == nil else { t.invalidate(); return }
                    if self.attached { self.attached = false }
                    self.start()
                    t.invalidate()
                }
            }
            restartTimer = timer
            return
        }
        guard process != nil else { return }
        stop(keepSwitch: true)
        watchdog.restarting()
        start()
    }

    /// Attach to the daemon-started node (unattended restart,
    /// docs/design/29): it holds `run.lock` and answers on the node's RPC
    /// port. The app monitors it exactly like its own — height, voting duty,
    /// DeviceCheck tokens, stall detection — without ever starting a second
    /// node on the same data directory.
    private func attachToRunningNode() {
        attached = true
        attachMisses = 0
        switched = false
        state = .running
        if candidate == nil, let binary {
            loadCandidate(binary)
            nextCandidateRetry = clock.now.advanced(by: 60)
        }
        watchdog.started(clock.now)
        answeredSinceStart = false
        rpcAnswering = true
        watchSleep()
        refreshDeviceToken()
        tokenTimer?.invalidate()
        tokenTimer = Timer.scheduledTimer(withTimeInterval: 3600, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refreshDeviceToken() }
        }
        poll?.invalidate()
        poll = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.check() }
        }
        applyDuty()
    }

    /// Whether a node answers on `port` (the daemon's node when the app opens
    /// after an unattended restart). Synchronous for callers off the main
    /// actor.
    nonisolated static func nodeAnswersRpc(port: UInt16) async -> Bool {
        await LocalRPC.call(port: port, method: "aether_status", params: []) != nil
    }

    private func refreshProver() {
        guard prove else {
            if prover != nil { prover = nil }
            return
        }
        let port = Self.port
        Task.detached {
            let v = await LocalRPC.call(port: port, method: "aether_proverStatus", params: [])
            let status = v.flatMap { try? JSONSerialization.data(withJSONObject: $0) }.flatMap { try? JSONDecoder().decode(ProverStatus.self, from: $0) }
            await MainActor.run { if self.prover != status { self.prover = status } }
        }
    }

    /// Last `aether_shardStats` read (throttled: the call lists the shard
    /// directory, and the Settings line it feeds does not change by the second).
    private var lastShardStatsCheck = MonotonicInstant.distantPast

    private func refreshHistoryKept() {
        guard clock.now.elapsed(since: lastShardStatsCheck) > 30 else { return }
        lastShardStatsCheck = clock.now
        let port = Self.port
        Task.detached {
            // The me row is this Mac as others have checked it; before anyone
            // has, the honest fallback is the pass rate this Mac observed
            // about the network's holders (that is all phase 1 records).
            let v = await LocalRPC.call(port: port, method: "aether_shardStats", params: []) as? [String: Any]
            let kept = v.flatMap { try? JSONSerialization.data(withJSONObject: $0) }
                .flatMap { try? JSONDecoder().decode(ShardStatsJSON.self, from: $0) }
                .map { s in
                    let rows = s.candidates ?? []
                    let mine = s.me.flatMap { me in rows.first { $0.node.lowercased() == me.lowercased() && $0.checked > 0 } }
                    let checked = mine?.checked ?? rows.reduce(0) { $0 + $1.checked }
                    let ok = mine?.ok ?? rows.reduce(0) { $0 + $1.ok }
                    return HistoryKept(bytes: s.held_bytes ?? 0,
                                       shards: s.held?.count ?? 0,
                                       windowDays: s.window_days ?? 7,
                                       passPercent: checked > 0 ? Int((Double(ok) / Double(checked) * 100).rounded()) : nil)
                }
            await MainActor.run { if self.history != kept { self.history = kept } }
        }
    }

    /// Rewards this Mac earned, as the same EarningsCSV document the Earnings
    /// screen exports — the menu bar has no screen state of its own, so it
    /// reads the rows fresh from the node instead (same file, same columns).
    func rewardsCSV() async -> String? {
        guard !proveAddress.isEmpty else { return nil }
        let (rows, _) = await Self.allRewards(port: Self.port, address: proveAddress)
        guard !rows.isEmpty else { return nil }
        return EarningsCSV.document(rows.compactMap(RewardEntry.init(json:)))
    }

    /// Every reward row the node holds for `address`, paged through with
    /// cursors — a single flat read is capped by the node's default limit and
    /// would quietly drop the tail of a long history (the founder's 2,583
    /// rewards did not fit one page). `total` is the count the node reports
    /// for the address, so the caller can show "N of M" instead of a silent
    /// cap. An older node without the paged method falls back to the flat
    /// list.
    nonisolated static func allRewards(port: UInt16, address: String) async -> ([[String: Any]], Int?) {
        var rows: [[String: Any]] = []
        var total: Int?
        var cursor: String?
        while true {
            guard let page = await LocalRPC.call(port: port, method: "aether_rewardsPage",
                                                 params: [address, cursor ?? NSNull(), 10_000]) as? [String: Any],
                  let batch = page["rewards"] as? [[String: Any]] else {
                if cursor == nil,
                   let flat = await LocalRPC.call(port: port, method: "aether_rewards", params: [address, 10_000]) as? [[String: Any]] {
                    return (flat, flat.count)
                }
                return (rows, total)
            }
            rows.append(contentsOf: batch)
            total = (page["total"] as? NSNumber)?.intValue ?? total
            guard let next = page["next_cursor"] as? String, !next.isEmpty, rows.count < 100_000 else { break }
            cursor = next
        }
        return (rows, total)
    }

    private func refreshUpgrade() {
        guard !upgradeAsked else { return }
        let port = Self.port
        Task.detached {
            guard let s = await LocalRPC.call(port: port, method: "aether_status", params: []) as? [String: Any],
                  let newest = (s["newest_scheduled"] as? NSNumber)?.intValue,
                  let mine = (s["node_protocol"] as? NSNumber)?.intValue, newest > mine else { return }
            await MainActor.run {
                self.upgradeAsked = true
                self.upgradeRequired = true
                self.onUpgradeNeeded?()
            }
        }
    }

    /// The node's data volume dropped below its free-space floor (`aether_status`
    /// ▸ resources): era seals and shard writes hold, proving pauses, and the
    /// Settings page says 디스크 공간 부족 until 2 GB above the floor again.
    private func refreshDisk() {
        guard state == .running || state == .starting else {
            if diskLow { diskLow = false }
            if diskAlmostFull { diskAlmostFull = false }
            if diskPaused { diskPaused = false }
            return
        }
        let port = Self.port
        Task.detached {
            let status = await LocalRPC.call(port: port, method: "aether_status", params: []) as? [String: Any]
            let resources = status?["resources"] as? [String: Any]
            let low = (resources?["disk_low"] as? Bool) ?? false
            let paused = (resources?["disk_paused"] as? Bool) ?? low
            let almost = (resources?["disk_almost_full"] as? Bool) ?? false
            await MainActor.run {
                if self.diskLow != low { self.diskLow = low }
                if self.diskPaused != paused { self.diskPaused = paused }
                if self.diskAlmostFull != almost { self.diskAlmostFull = almost }
            }
        }
    }

    private func check() {
        if candidate == nil, clock.now >= nextCandidateRetry {
            nextCandidateRetry = clock.now.advanced(by: 60)
            if let binary { loadCandidate(binary) }
        }
        refreshVoting()
        applyDuty()
        refreshProver()
        refreshHistoryKept()
        refreshUpgrade()
        refreshDisk()
        guard !checkInFlight else { return }
        checkInFlight = true
        let port = Self.port, switched = self.switched
        Task.detached {
            // One reading of the local node covers all three feeds: its
            // height, its stage-wise activity counter (red team #2), and —
            // cached for the rollback decision (red team #3) — the newest
            // protocol the chain has scheduled.
            let status = await LocalRPC.call(port: port, method: "aether_status", params: []) as? [String: Any]
            let statusHeight = (status?["height"] as? NSNumber)?.uint64Value
            let local = statusHeight ?? localNodeHeight(port: port)
            // The network's height stays in the picture after the switch too
            // (red team #17): the wallet's own multi-source verified view,
            // whether or not it is reading through this node.
            let network = try? authenticatedRemoteHeight()
            let activity = (status?["activity"] as? NSNumber)?.uint64Value
            await MainActor.run {
                self.checkInFlight = false
                guard self.process != nil || self.attached else { return }
                if self.attached, status == nil {
                    // The attached (daemon-started) node stopped answering:
                    // after a short grace (it may be restarting under the
                    // daemon), take the data directory back and run our own.
                    self.attachMisses += 1
                    if self.attachMisses >= 5, self.enabled, self.process == nil {
                        self.attached = false
                        self.attachMisses = 0
                        self.start()
                    }
                    return
                }
                if self.attached { self.attachMisses = 0 }
                let answered = status != nil
                if self.rpcAnswering != answered { self.rpcAnswering = answered }
                if answered, !self.answeredSinceStart { self.answeredSinceStart = true }
                if let mine = (status?["node_protocol"] as? NSNumber)?.uint64Value { self.nodeProtocol = mine }
                let route = self.watchdog.useLocalNode(
                    local: statusHeight, network: network,
                    responsive: status != nil, currentlyLocal: self.switched,
                    at: self.clock.now)
                if self.networkCheckPending != route.networkPending {
                    self.networkCheckPending = route.networkPending
                }
                let useLocal = route.useLocal
                if self.switched != useLocal {
                    self.switched = useLocal
                    if !UserDefaults.standard.bool(forKey: "useDevelopmentNetwork") { useLocalNode(port: useLocal ? port : nil) }
                    self.state = useLocal ? .running : .starting
                }
                guard let local else {
                    self.state = .starting
                    return
                }
                if self.height != local { self.height = local }
                if let scheduled = (status?["newest_scheduled"] as? NSNumber)?.uint64Value {
                    self.scheduledProtocol = scheduled
                }
                if !self.switched && self.state != .starting {
                    self.state = .starting  // catching up; the wallet keeps asking validators meanwhile
                }
                // A stall (the network moves, ours has not for a minute) —
                // unless the node's own work counter is moving (a snapshot
                // download, a store recovery, a backlog replay), and with
                // twice the patience while this Mac is in the voting set (its
                // restart costs the network a signature). The incident of
                // 2026-09-29 looked exactly like this, and "끊김" told the
                // user nothing.
                if case .restart = self.watchdog.polled(self.clock.now, local: local, network: network, activity: activity, voting: self.isValidator) {
                    self.stallRestarts += 1
                    self.restartIfRunning()
                }
            }
        }
    }

    /// Sleep/wake (red team #9): everything the watchdog was timing before a
    /// sleep is stale the moment the Mac sleeps — the freeze it was counting,
    /// the heights it was comparing. Invalidate on both ends of a sleep, and
    /// take a fresh reading right on the wake.
    private func watchSleep() {
        guard wakeObservers.isEmpty else { return }
        let center = NSWorkspace.shared.notificationCenter
        if let source = IOPSNotificationCreateRunLoopSource({ context in
            guard let context else { return }
            let controller = Unmanaged<NodeController>.fromOpaque(context).takeUnretainedValue()
            Task { @MainActor in
                controller.announceAvailability(leaving: NodeController.onBattery)
                controller.applyPower()
            }
        }, Unmanaged.passUnretained(self).toOpaque())?.takeRetainedValue() {
            powerSourceSource = source
            CFRunLoopAddSource(CFRunLoopGetMain(), source, .defaultMode)
        }
        wakeObservers = [
            center.addObserver(forName: NSWorkspace.willSleepNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.announceAvailability(leaving: true) }
                Task { @MainActor in
                    guard let self, self.process != nil else { return }
                    self.watchdog.invalidate()
                    self.switched = false
                    self.networkCheckPending = false
                    if !UserDefaults.standard.bool(forKey: "useDevelopmentNetwork") { useLocalNode(port: nil) }
                    self.state = .starting
                }
            },
            center.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.announceAvailability(leaving: Self.onBattery) }
                Task { @MainActor in
                    guard let self, self.process != nil else { return }
                    self.watchdog.invalidate()
                    self.switched = false
                    self.networkCheckPending = false
                    if !UserDefaults.standard.bool(forKey: "useDevelopmentNetwork") { useLocalNode(port: nil) }
                    self.state = .starting
                    self.check()
                }
            },
            center.addObserver(forName: NSWorkspace.willPowerOffNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.announceAvailability(leaving: true) }
            },
        ]
    }
}
// MARK: validator duty (docs/design/13-roadmap.md F, P0)

extension NodeController {
    /// This Mac's node runs and the network has it in the voting set. The
    /// daemon-started node counts the same (docs/design/29): attached is
    /// running.
    var isValidator: Bool { (process != nil || attached) && voting?.voting == true }

    /// Keep the Mac awake while it signs blocks: a sleeping member is a missing
    /// vote, and a third of them asleep pauses the network. On battery with
    /// "Only while on the power adapter" on, the node is off anyway.
    var keepsAwake: Bool { isValidator && !(onlyOnPower && Self.onBattery) }

    /// One line for Settings: what Aether does about sleep.
    var awakeNote: String {
        if keepsAwake { return "This Mac signs blocks now: \(Brand.project) keeps it from sleeping (the display can still sleep)." }
        return onlyOnPower
            ? "While this Mac signs blocks on power, \(Brand.project) keeps it from sleeping."
            : "While this Mac signs blocks, \(Brand.project) keeps it from sleeping, on battery too."
    }

    /// Take or release the no-idle-sleep assertion to match `keepsAwake`.
    func applyDuty() {
        let on = keepsAwake
        if on != sleepGuard.held {
            sleepGuard.set(on)
            objectWillChange.send()
        }
    }

    /// The network made no block for a minute (`WalletModel.chainPausedSince`):
    /// a validator hears it once when it pauses and once when it resumes.
    func networkPaused(since: Date?) {
        if since != nil, isValidator, !pauseNotified {
            pauseNotified = true
            LocalNotice.post(title: "The network is paused",
                             body: "No block has been finalized for a minute. Keep this Mac awake and online: it is one of the Macs that sign blocks.")
        } else if since == nil, pauseNotified {
            pauseNotified = false
            LocalNotice.post(title: "The network is running again", body: "Blocks are being finalized again.")
        }
    }
}

/// An IOKit assertion that stops idle system sleep (not display sleep) while held.
final class SleepGuard {
    private var id: IOPMAssertionID = 0
    private(set) var held = false
    let reason: String

    init(reason: String) { self.reason = reason }

    func set(_ on: Bool) {
        if on, !held {
            held = IOPMAssertionCreateWithName(kIOPMAssertionTypePreventUserIdleSystemSleep as CFString,
                                               IOPMAssertionLevel(kIOPMAssertionLevelOn), reason as CFString, &id) == kIOReturnSuccess
        } else if !on, held {
            IOPMAssertionRelease(id)
            held = false
        }
    }

    deinit { set(false) }
}

/// Local notifications (asks for permission the first time one is posted).
enum LocalNotice {
    static func post(title: String, body: String) {
        let center = UNUserNotificationCenter.current()
        center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
            guard granted else { return }
            let content = UNMutableNotificationContent()
            content.title = title
            content.body = body
            content.sound = .default
            center.add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
        }
    }
}

/// JSON-RPC to the local node (loopback only).
enum LocalRPC {
    static func call(port: UInt16, method: String, params: [Any]) async -> Any? {
        guard let url = URL(string: "http://127.0.0.1:\(port)/") else { return nil }
        var req = URLRequest(url: url, timeoutInterval: 5)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try? JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "id": 1, "method": method, "params": params])
        guard let (data, _) = try? await URLSession.shared.data(for: req),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return obj["result"]
    }

    /// A U256 from JSON ("0x…" hex or a number) as a decimal string of wei.
    static func decimal(_ v: Any?) -> String {
        if let n = v as? NSNumber { return n.stringValue }
        guard var hex = v as? String else { return "0" }
        if hex.hasPrefix("0x") { hex.removeFirst(2) }
        var digits: [UInt8] = [0]  // little-endian base 10
        for c in hex {
            guard let d = c.hexDigitValue else { return "0" }
            var carry = d
            for i in digits.indices {
                let x = Int(digits[i]) * 16 + carry
                digits[i] = UInt8(x % 10)
                carry = x / 10
            }
            while carry > 0 {
                digits.append(UInt8(carry % 10))
                carry /= 10
            }
        }
        let s = digits.reversed().map(String.init).joined().drop(while: { $0 == "0" })
        return s.isEmpty ? "0" : String(s)
    }
}
#endif
