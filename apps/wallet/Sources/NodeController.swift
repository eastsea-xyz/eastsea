#if os(macOS)
import AppKit
import DeviceCheck
import Foundation
import IOKit.ps
import IOKit.pwr_mgt
import ServiceManagement
import SwiftUI
import Combine
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
    nonisolated static let storageMoveLockTimeout: TimeInterval = 60
    nonisolated static var storageMoveDefaults: UserDefaults { .standard }
    enum State: Equatable {
        case off
        case starting
        case running
        /// On, but waiting for the power adapter (see `onlyOnPower`).
        case waitingForPower
        case failed(String)
    }

    @Published private(set) var state: State = .off {
        didSet { if state != oldValue { refreshStopReason() } }
    }
    /// Why the node is not running while its switch is on — the one reason
    /// the sidebar, the Node page, the menu and the health banner all show
    /// (nil while it runs normally). See `NodeStopReason`.
    @Published private(set) var stopReason: NodeStopReason?
    /// An unavailable hardware read pauses signatures inside the live node.
    /// It is allowed to retry without a watchdog restart.
    @Published private(set) var confirmingMac = false
    @Published private(set) var keyRebindInProgress = false
    @Published private(set) var keyRebindError: String?
    /// Wired to the wallet's existing owner key; there is no unauthenticated
    /// fallback when that key is locked or the owner cancels Touch ID.
    var authorizeKeyRebind: ((String, String, String) async throws -> NodeKeyRebind.Approval)?
    /// The last stop, as `node-status.log` recorded it (diagnostics).
    var lastStopLine: String? { UserDefaults.standard.string(forKey: "nodeLastStop") }
    /// The code last written to `node-status.log` (one line per change).
    private var lastLoggedCode: String?
    /// When the watchdog's terminal decision was made (the gate retries the
    /// recoverable ones after `NodeResume.autoRetryAfter`).
    private var blockedAt: MonotonicInstant?
    /// The OS error of the last launch attempt, until one succeeds.
    private var launchError: String?
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
                unblock()
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
    @AppStorage("proveAddress") private var savedProveAddress = ""
    /// Compatibility for existing proving controls: starting the node from
    /// another selected wallet cannot silently change its explicit payout.
    /// New account UI changes it through AccountStore.setPayoutAccount.
    var proveAddress: String {
        get { AccountStore.wallet().payoutAddress.isEmpty ? savedProveAddress : AccountStore.wallet().payoutAddress }
        set { savedProveAddress = AccountStore.wallet().payoutAddress.isEmpty ? newValue : AccountStore.wallet().payoutAddress }
    }
    private var payoutSubscription: AnyCancellable?
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
    /// 블록 데이터 위치 (`BlockDataLocation`): the node's `--chain-data`
    /// folder on a disk the person picked; empty = the default (the node's
    /// data folder, internal disk). Changed only by `moveBlockData`, which
    /// copies and verifies first.
    @AppStorage("nodeChainDataPath") var chainDataPath = ""
    /// 전체 기록 보관 (아카이브): run the follower as an archive (replays from
    /// genesis, never jumps, keeps the full history). Applies on restart.
    @AppStorage("nodeArchive") var archive = false {
        didSet { if archive != oldValue { restartIfRunning(); unattended?.syncMarker() } }
    }
    /// The block data is moving (0…100), or nil.
    @Published var storageMovePercent: Int?
    /// The last move's failure, in the person's words, until the next try.
    @Published var storageMoveError: String?
    /// That failure is a disk format Disk Utility can fix (exFAT, FAT).
    @Published var storageMoveOffersDiskUtility = false
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
        let stale: Bool?
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
        /// That reward predates this version's proving (crates/node rpc.rs):
        /// never shown as the latest.
        let last_reward_stale: Bool?
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
                logEvent("login", "failed: \(error.localizedDescription)")
                state = .failed(String(localized: "Could not change Open at Login. Please try again."))
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
        do {
            if let root = try BlockDataMove.authoritativeRoot(in: Self.dataDir) {
                chainDataPath = root.path == BlockDataLocation.resolvedRoot(Self.dataDir).path ? "" : root.path
            }
        } catch {
            // An unreadable transaction record must never select an older
            // preference and start writing a second chain store.
            storageMoveError = String(localized: "The block-data move record could not be read. Keep both copies and retry after reconnecting the disk.")
            storageMovePercent = 0
        }
        payoutSubscription = AccountStore.wallet().payoutAddressPublisher.removeDuplicates().dropFirst().sink { [weak self] address in
            guard let self, !address.isEmpty else { return }
            let changed = self.savedProveAddress.lowercased() != address.lowercased()
            self.savedProveAddress = address
            if changed && self.prove { self.restartIfRunning() }
        }
    }
    /// The update owns the node's startup gate and run.lock until this
    /// process terminates. Abort restores the marker and normal start gate.
    private(set) var updateInProgress = false
    private var updateRunLock: Int32?
    private var updateOwnsRespawnSuspension = false
    private var updatePreparationGeneration: UInt64 = 0
    @Published private(set) var runningReleaseVerified = false
    private var releaseVerifiedPID: Int32?
    private var verifiedStatusBinding: NodeReleaseIdentity.Binding?
    private var verifiedStatusRequestedAt: MonotonicInstant?

    /// A cached signature observation never authenticates a replacement PID.
    var updateReleaseVerified: Bool {
        guard runningReleaseVerified, let requestedAt = verifiedStatusRequestedAt,
              clock.now.elapsed(since: requestedAt) >= 0, clock.now.elapsed(since: requestedAt) <= 15,
              let binding = verifiedStatusBinding,
              let pid = process?.processIdentifier ?? unattended?.runningNodePID else { return false }
        return pid == releaseVerifiedPID && pid == binding.rootPID
    }

    func prepareForUpdate() async -> Bool {
        guard !updateInProgress else { return updateRunLock != nil }
        guard storageMovePercent == nil else { return false }
        updatePreparationGeneration &+= 1
        let generation = updatePreparationGeneration
        updateInProgress = true
        let dir = Self.dataDir
        guard DataMigration.mayStartNode() == nil else { abortUpdatePreparation(); return false }
        do {
            // Only the app's internal metadata directory. Never create a
            // selected external chain-data directory merely to install.
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true,
                                                    attributes: [.posixPermissions: 0o700])
        } catch { abortUpdatePreparation(); return false }
        guard let unattended else { abortUpdatePreparation(); return false }
        // suspendRespawn increments its reference count even when its
        // filesystem removal fails; balance exactly this attempt on abort.
        updateOwnsRespawnSuspension = true
        guard unattended.suspendRespawn() else {
            abortUpdatePreparation()
            return false
        }
        // A daemon can be present before we attach. Never stop an unknown
        // parent and mistake its released lock for dead writer children.
        let ownPID = process?.processIdentifier
        let daemonPID = unattended.runningNodePID
        if let ownPID, let daemonPID, ownPID != daemonPID {
            abortUpdatePreparation()
            return false
        }
        let rootPID = ownPID ?? daemonPID
        var reservedFD: Int32?
        defer { if let reservedFD { close(reservedFD) } }
        let releaseVerified: Bool
        let runtimeAbsent: Bool
        let attestedBinding: NodeReleaseIdentity.Binding?
        if let rootPID, let expected = Self.helperBinaryURL {
            let sample = await LocalRPC.callVerified(rootPID: rootPID, port: Self.port, expected: expected,
                                                     method: "aether_status", params: [])
            releaseVerified = sample.map { NodeReleaseIdentity.hasWriterLease(status: $0.value) } ?? false
            runtimeAbsent = false
            attestedBinding = sample?.binding
        } else {
            releaseVerified = false
            attestedBinding = nil
            // Do not wait for an unrecognized holder to disappear: its old
            // children may still write. Claim an already-free lock at once.
            if await LocalRPC.endpointIsAbsent(port: Self.port) {
                reservedFD = await BlockDataMove.holdRunLock(in: dir, timeout: 0)
            }
            runtimeAbsent = reservedFD != nil
        }
        guard updatePreparationGeneration == generation, updateInProgress, !Task.isCancelled,
              process?.processIdentifier == ownPID, unattended.runningNodePID == daemonPID,
              UnattendedDecision.mayStopForUpdate(ownProcess: ownPID != nil, attached: attached,
                  daemonPresent: daemonPID != nil, releaseVerified: releaseVerified,
                  unclaimedRuntimeAbsent: runtimeAbsent) else {
            if updatePreparationGeneration == generation { abortUpdatePreparation() }
            return false
        }
        if rootPID != nil {
            // Returning to the main actor can outlive the attested instance.
            // A reused PID or replaced same-release listener must attest anew.
            guard let attestedBinding, let expected = Self.helperBinaryURL,
                  NodeReleaseIdentity.matches(binding: attestedBinding, port: Self.port, expected: expected) else {
                abortUpdatePreparation()
                return false
            }
        }
        stop(keepSwitch: true)
        if let daemonPID { unattended.stopDaemonNode(expectedPID: daemonPID) }
        let heldFD: Int32?
        if let reservedFD { heldFD = reservedFD }
        else { heldFD = await BlockDataMove.holdRunLock(in: dir, timeout: 60) }
        guard let fd = heldFD else {
            if updatePreparationGeneration == generation { abortUpdatePreparation() }
            return false
        }
        reservedFD = nil
        guard updatePreparationGeneration == generation, updateInProgress else { close(fd); return false }
        guard !Task.isCancelled, fcntl(fd, F_SETFD, FD_CLOEXEC) == 0 else {
            close(fd)
            abortUpdatePreparation()
            return false
        }
        updateRunLock = fd
        return true
    }

    func abortUpdatePreparation() {
        guard updateInProgress else { return }
        updatePreparationGeneration &+= 1
        if let fd = updateRunLock { close(fd) }
        updateRunLock = nil
        updateInProgress = false
        if updateOwnsRespawnSuspension {
            updateOwnsRespawnSuspension = false
            unattended?.resumeRespawn()
        }
        if enabled { applyPower() }
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
        #if WALLET_SCREENS
        // The screens renderer (scripts/wallet-screens.sh) never touches the real data.
        return 
        #endif
        powerTimer?.invalidate()
        powerTimer = Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.applyPower() }
        }
        applyPower()
    }

    /// The start gate, every 30 s and on every event that can change it
    /// (power source, wake, a finished data move, a disk mounting): gather
    /// the facts, let `NodeResume` decide, carry it out, and publish the one
    /// reason when the node does not run. Nothing else may leave the switch
    /// on with no node and no reason (the founder's 0.7.0 report).
    func applyPower() {
        guard !updateInProgress else { return }
        if storageMovePercent != nil {
            refreshStopReason()
            return
        }
        // The daemon's approval changes outside the app (System Settings):
        // re-read it on every tick, so its sentence appears and goes by itself.
        unattended?.refreshStatus()
        guard enabled else { refreshStopReason(); return }
        if refusePersistedBindingMismatch() { return }
        if !attached, process != nil, onlyOnPower, Self.onBattery, !isValidator {
            // Keep voting until the announced handoff lands (isValidator).
            stop(keepSwitch: true)
            state = .waitingForPower
            return
        }
        let facts = resumeFacts()
        switch NodeResume.decide(facts) {
        case .keepRunning:
            if attached, state != .running { state = .running }
        case .start(let detach):
            if detach {
                logEvent("detach", "the node this app attached to no longer holds run.lock; starting our own")
                detachFromGoneNode()
            }
            if automaticRestartBlocked {
                logEvent("retry", "automatic retry after \(facts.blockedForSeconds)s (\(stoppedFailure.map { "\($0)" } ?? "-"))")
                unblock()
            }
            start()
        case .wait(let reason):
            switch reason {
            case .onBattery: state = .waitingForPower
            case .restarting: break   // the watchdog's own line stays
            default:
                let title = reason.copy().title
                if state != .failed(title) { state = .failed(title) }
            }
        }
        refreshStopReason(facts)
    }

    /// Clear the watchdog's terminal decision (the switch turned on, a retry
    /// is due, or the person pressed the button).
    private func unblock() {
        if refusePersistedBindingMismatch() { return }
        automaticRestartBlocked = false
        blockedAt = nil
        stoppedFailure = nil
    }

    /// The watchdog stopped restarting: remember why and since when, so the
    /// gate can say it and retry the recoverable kinds later.
    private func block(_ failure: NodeWatchdog.Failure) {
        automaticRestartBlocked = true
        blockedAt = clock.now
        stoppedFailure = failure
    }

    /// The daemon maps terminal exit 15 to a successful outer exit so launchd
    /// does not retry it. The persisted marker is how an attached or relaunched
    /// wallet observes that same terminal decision without spawning again.
    @discardableResult
    private func refusePersistedBindingMismatch() -> Bool {
        guard NodeBindingRefusal.exists(in: Self.dataDir) else { return false }
        restartTimer?.invalidate()
        restartTimer = nil
        if attached, !Self.lockHeld(in: Self.dataDir) {
            logEvent("detach", "the attached node released run.lock after a key-binding refusal; owner recovery is required")
            detachFromGoneNode()
        }
        confirmingMac = false
        if stoppedFailure != .keyElsewhere || !automaticRestartBlocked { block(.keyElsewhere) }
        let title = NodeStopReason.keyElsewhere.copy().title
        if state != .failed(title) { state = .failed(title) }
        applyDuty()
        refreshStopReason()
        return true
    }

    /// The reason's one button.
    func perform(_ action: NodeStopAction) {
        switch action {
        case .turnOn: enabled = true
        case .runOnBattery: onlyOnPower = false
        case .showInFinder: InstallLocation.revealInFinder()
        case .checkForUpdates: onUpgradeNeeded?()
        case .retryNow:
            unblock()
            restartTimer?.invalidate()
            restartTimer = nil
            applyPower()
        case .openStorage:
            if let url = URL(string: "x-apple.systempreferences:com.apple.settings.Storage") { NSWorkspace.shared.open(url) }
        case .chooseDisk: chooseDiskRequested = true
        case .openPrivacySettings:
            if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders") {
                NSWorkspace.shared.open(url)
            }
        case .copyDiagnostics:
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(statusLogTail(), forType: .string)
        case .rebindKeys:
            rebindNodeKeys()
        }
    }

    /// Owner recovery exists only for a stopped node with a proven mismatch.
    /// The CLI takes run.lock itself too, closing the race after this UI check.
    private func rebindNodeKeys() {
        guard !keyRebindInProgress,
              NodeKeyRebind.canOffer(reason: stopReason, processRunning: process != nil,
                                     attached: attached, lockHeld: Self.lockHeld(in: Self.dataDir)) else { return }
        let dir = Self.dataDir
        keyRebindError = nil
        do {
            guard let binary, let authenticate = authorizeKeyRebind else { throw NodeKeyRebind.Refusal.ownerKeyUnavailable }
            let address = try NodeKeyRebind.validatorAddress(in: Data(contentsOf: dir.appendingPathComponent("validator.pub.json")))
            let alert = NSAlert()
            alert.alertStyle = .warning
            alert.messageText = String(localized: "Rebind these node keys to this Mac?")
            alert.informativeText = NodeKeyRebind.warning() + "\n\n"
                + String(localized: "Type this validator address to confirm:\n") + address
            alert.addButton(withTitle: String(localized: "Authenticate Owner and Rebind"))
            alert.addButton(withTitle: String(localized: "Cancel"))
            let field = NSTextField(string: "")
            field.frame = NSRect(x: 0, y: 0, width: 540, height: 24)
            field.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
            field.placeholderString = String(localized: "Type the validator address")
            alert.accessoryView = field
            alert.window.initialFirstResponder = field
            guard alert.runModal() == .alertFirstButtonReturn else { return }
            let typed = field.stringValue
            guard NodeKeyRebind.normalized(typed) == address else { throw NodeKeyRebind.Refusal.confirmationDidNotMatch }
            keyRebindInProgress = true
            Task {
                defer { keyRebindInProgress = false }
                do {
                    let approval = try await authenticate(address, typed, dir.path)
                    guard NodeKeyRebind.canOffer(reason: stopReason, processRunning: process != nil,
                                                 attached: attached, lockHeld: Self.lockHeld(in: dir)) else {
                        throw NodeKeyRebind.Refusal.nodeRunning
                    }
                    let result = try await Task.detached {
                        try NodeKeyRebindCommand.run(binary: binary, dataDirectory: dir, approval: approval)
                    }.value
                    logEvent("key_rebind", result)
                    candidate = nil
                    identityNoticePosted = false
                    nextCandidateRetry = clock.now
                    unblock()
                    applyPower()
                } catch {
                    keyRebindError = error.localizedDescription
                    logEvent("key_rebind_refused", error.localizedDescription)
                }
            }
        } catch {
            keyRebindError = error.localizedDescription
            logEvent("key_rebind_refused", error.localizedDescription)
        }
    }
    /// The Node page opens the block-data location picker when a stop reason's
    /// button asks for it.
    @Published var chooseDiskRequested = false

    /// What the start gate reads, right now.
    private func resumeFacts() -> NodeResumeFacts {
        var f = NodeResumeFacts()
        f.enabled = enabled
        f.wrongLocation = wrongLocation
        f.hasBinary = binary != nil
        f.migrating = DataMigration.Runner.shared.isRunning
        f.migrationGate = f.migrating ? nil : DataMigration.mayStartNode()
        f.movingStoragePercent = storageMovePercent
        f.processRunning = process != nil
        f.confirmingMac = confirmingMac
        f.attached = attached
        f.lockHeldByOther = process == nil && Self.lockHeld(in: Self.dataDir)
        if !f.lockHeldByOther { lockRefused = false }
        f.lockRefused = lockRefused
        if let t = restartTimer, t.isValid {
            f.restartInSeconds = max(1, Int(t.fireDate.timeIntervalSinceNow.rounded(.up)))
        }
        f.onlyOnPower = onlyOnPower
        f.onBattery = Self.onBattery
        f.isValidator = isValidator
        f.blocked = automaticRestartBlocked ? (stoppedFailure ?? .other) : nil
        f.keyBindingRefused = NodeBindingRefusal.exists(in: Self.dataDir)
        f.blockedForSeconds = blockedAt.map { Int(clock.now.elapsed(since: $0)) } ?? 0
        f.storage = blockDataStorageState()
        if case .chosen(let volume, let mounted, _) = f.storage {
            f.volumeName = volume
            if mounted { f.freeBytes = StorageSetting.freeBytes(atPath: chainDataPath).map { UInt64(max(0, $0)) } }
        } else {
            f.freeBytes = StorageSetting.freeBytes(atPath: Self.dataDir.path).map { UInt64(max(0, $0)) }
        }
        f.launchError = launchError
        return f
    }

    /// Recompute `stopReason` (cheap: syscalls only) and log a change.
    private func refreshStopReason(_ given: NodeResumeFacts? = nil) {
        let reason: NodeStopReason?
        var facts = given
        if !enabled {
            reason = .switchedOff
        } else if NodeBindingRefusal.exists(in: Self.dataDir) {
            reason = .keyElsewhere
        } else if process != nil || attached {
            if confirmingMac {
                reason = .waitingForMacConfirmation
            } else if diskPaused {
                // Running, but below the node's write floor: the node waits
                // for space by itself — say how much, on which disk.
                let f = facts ?? resumeFacts()
                facts = f
                reason = .diskFull(freeBytes: f.freeBytes ?? 0, resumeBytes: NodeResume.resumeBytes, volume: f.volumeName)
            } else {
                reason = nil
            }
        } else {
            let f = facts ?? resumeFacts()
            facts = f
            switch NodeResume.decide(f) {
            case .wait(let r): reason = r
            case .keepRunning: reason = nil
            case .start: reason = launchError.map { .launchFailed($0) }
            }
        }
        if stopReason != reason { stopReason = reason }
        record(reason, facts: facts)
    }

    /// One line in `node-status.log` per change of reason (not per countdown).
    private func record(_ reason: NodeStopReason?, facts: NodeResumeFacts?) {
        let code = reason?.code ?? "running"
        guard code != lastLoggedCode else { return }
        let first = lastLoggedCode == nil
        lastLoggedCode = code
        if first && reason == .switchedOff { return }   // a switched-off app at launch is not news
        let en = reason?.copy(locale: Locale(identifier: "en"), bundle: AppLanguage.bundle(for: "en"))
        let detail = en.map { "\($0.title). \($0.paragraph)" } ?? "the node runs"
        let line = NodeStatusLog.line(at: Date(), event: reason == nil ? "running" : "stopped \(code)", detail: detail, facts: facts)
        NodeStatusLog.append(line, in: Self.dataDir)
        if reason != nil { UserDefaults.standard.set(line.trimmingCharacters(in: .newlines), forKey: "nodeLastStop") }
    }

    /// A process event (an exit, a lock exit, an attach) for `node-status.log`.
    func logEvent(_ event: String, _ detail: String) {
        NodeStatusLog.append(NodeStatusLog.line(at: Date(), event: event, detail: detail, facts: nil), in: Self.dataDir)
    }

    /// The last lines of `node-status.log`, for "copy diagnostics".
    func statusLogTail(lines: Int = 40) -> String {
        let text = (try? String(contentsOf: Self.dataDir.appendingPathComponent(NodeStatusLog.fileName), encoding: .utf8)) ?? ""
        return text.split(separator: "\n").suffix(lines).joined(separator: "\n")
    }

    /// Whether some other process holds `<dir>/run.lock` (an exclusive
    /// non-blocking flock probe, released at once). Never creates the file.
    nonisolated static func lockHeld(in dir: URL) -> Bool {
        let fd = open(dir.appendingPathComponent("run.lock").path, O_RDONLY)
        guard fd >= 0 else { return false }
        defer { close(fd) }
        if flock(fd, LOCK_EX | LOCK_NB) == 0 {
            flock(fd, LOCK_UN)
            return false
        }
        return errno == EWOULDBLOCK
    }

    /// The node we were attached to is gone: forget it before starting ours.
    private func detachFromGoneNode() {
        invalidateUpdateMembership()
        stopCandidateRead()
        attached = false
        confirmingMac = false
        attachMisses = 0
        poll?.invalidate()
        poll = nil
        switched = false
        networkCheckPending = false
        if !UserDefaults.standard.bool(forKey: "useDevelopmentNetwork") { useLocalNode(port: nil) }
    }

    static let port: UInt16 = 18_545
    /// crates/node EXIT_CHAIN_DATA_MISSING: the chosen block-data folder is gone.
    static let chainDataMissingExit: Int32 = 13
    /// crates/node EXIT_KEYS_ON_CHAIN_DATA: keys found in the block-data folder.
    static let keysOnChainDataExit: Int32 = 14
    /// Our last start bounced off a run.lock whose holder does not answer.
    private var lockRefused = false

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
        #if WALLET_SCREENS
        // The screens renderer: a folder that does not exist, so nothing it
        // logs or reads ever lands in the real node folder.
        return FileManager.default.temporaryDirectory.appendingPathComponent("wallet-screens-no-node", isDirectory: true)
        #endif
        DataMigration.ensure()
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/EastSea/node", isDirectory: true)
    }

    /// The note the Connected status appends while `networkCheckPending`:
    /// one phrase, ko/en, localized like `NodeWatchdog.Failure.sentence`.
    var pendingRouteNote: String {
        return String(localized: "· via this Mac's node; network check pending")
    }

    private var binary: URL? {
        let helpers = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers")
        let name = usePreviousBinary ? "NodeRollback.bundle/Contents/MacOS/aether.prev" : "aether"
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
        #if WALLET_SCREENS
        // The screens renderer lives in DerivedData, which is no place to run
        // from; the sample is a Mac that runs EastSea from Applications.
        wrongLocation = UserDefaults.standard.string(forKey: "previewWrongLocation") == "1"
        if wrongLocation { state = .failed(InstallLocation.moveSentence) }
        candidate = Candidate(validatorKey: "0xpreview", nodeId: "preview", beaconer: "0xpreview")
        voting = VotingNodeStatus(registered: true, streak: 5, lastEpoch: 7_675, epoch: 7_676, voting: false, candidates: 3)
        history = HistoryKept(bytes: 12_884_901_888, shards: 12, windowDays: 7, passPercent: 98)
        if prove {
            prover = ProverStatus(running: true, stale: false, proving: 184_211, last_height: 184_209, last_txs: 3, last_seconds: 41.6,
                                  proofs: 57, proofs_failing: false, acceptance_rate_percent: 100, program_unknown: false,
                                  program_mismatch: false, network_program: nil, error: nil, paused: nil,
                                  memory_bytes: 5_690_000_000, memory_cap: 6_442_450_944, lag: 2,
                                  last_reward: "0x6f05b59d3b20000", last_reward_stale: false)
        }
        #endif
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
    /// When the app last sent the node its availability wake-up (SIGUSR1).
    private var lastWakeSignal: Date?
    /// Disk mount/unmount observers (block data on a chosen disk).
    var mountObservers: [NSObjectProtocol] = []

    /// The node signs and gossips this local request with its registered voting
    /// key. A signal wakes its one-second loop before macOS suspends the process.
    private func announceAvailability(leaving: Bool) {
        let file = Self.dataDir.appendingPathComponent("availability-state")
        try? Data((leaving ? "leaving" : "back").utf8).write(to: file, options: .atomic)
        if case .running = state, candidate != nil, let process, process.isRunning {
            // `aether run` forwards it to its child from this release on; an
            // older supervisor had no handler and died of it, silently
            // (2026-10-07T04:39Z). Logged so such a death is never a mystery.
            lastWakeSignal = Date()
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
        #if WALLET_SCREENS
        // The screens renderer (scripts/wallet-screens.sh) never touches the real data.
        return
        #endif
        // The gate's timer runs whether or not the switch reads on at launch:
        // a switch that turns on behind our back (the old app's preferences
        // copied at the end of the data move — the founder's MacBook) is
        // picked up within 30 s instead of never.
        startIfAllowed()
        watchVolumes()
    }

    /// The data move from Aether finished (M1): a start the gate refused
    /// while it ran is retried now rather than at the next 30 s power tick.
    func migrationFinished() {
        logEvent("migration", "the data move finished; the switch reads \(enabled ? "on" : "off")")
        guard process == nil else { return }
        applyPower()
    }

    func start() {
        guard !updateInProgress, storageMovePercent == nil else { return }
        runningReleaseVerified = false
        #if WALLET_SCREENS
        // The screens renderer (scripts/wallet-screens.sh) never touches the real data.
        return 
        #endif
        if refusePersistedBindingMismatch() { return }
        guard !wrongLocation else {
            // Red team #10: from a DMG/Downloads/read-only place the node's
            // data would point into a bundle that disappears. One sentence;
            // the wallet keeps working through other nodes.
            state = .failed(InstallLocation.moveSentence)
            return
        }
        guard process == nil else { return }
        guard let binary else {
            state = .failed(String(localized: "This build of the app does not include the node."))
            return
        }
        invalidateUpdateMembership()
        // Audit 5, A5-7: while the old Aether node data (identity, threshold
        // share, chain) waits unmigrated, starting fresh here would strand
        // this Mac's validator identity. The migration itself already ran or
        // is running (Self.dataDir) — this catches its running, deferred or
        // failed state; `migrationFinished` retries once it is done.
        // Retry a deferred move first (the old app may have quit since): a
        // no-op once settled, a cheap lock probe while the old app runs.
        DataMigration.ensure()
        if let why = DataMigration.mayStartNode() {
            state = .failed(why)
            return
        }
        do {
            try FileManager.default.createDirectory(at: Self.dataDir, withIntermediateDirectories: true)
        } catch {
            launchError = error.localizedDescription
            logEvent("launch", "failed: \(error.localizedDescription)")
            state = .failed(NodeStopReason.launchFailed(error.localizedDescription).copy().detail)
            return
        }
        announceAvailability(leaving: Self.onBattery)
        nextCandidateRetry = clock.now
        // The same argv the daemon would run (UnattendedDecision.nodeArgv is
        // the single source), plus the app-child-only --exit-with-parent.
        var args = UnattendedDecision.nodeArgv(
            dataDir: Self.dataDir.path,
            rpcPort: Self.port,
            p2pPort: Self.p2pPort,
            networkPath: Bundle.main.url(forResource: "network", withExtension: "json")?.path,
            proverFlags: ProverFlags.build(memory: proverMemory, cores: proverCores, battery: proverOnBattery,
                                           activeProcessors: ProcessInfo.processInfo.activeProcessorCount),
            storageFlag: StorageSetting.flag(shards: storageShards),
            locationFlags: BlockDataLocation.flags(chainDataPath: chainDataPath, archive: archive))
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
            launchError = error.localizedDescription
            logEvent("launch", "failed: \(error.localizedDescription)")
            state = .failed(NodeStopReason.launchFailed(error.localizedDescription).copy().detail)
            return
        }
        launchError = nil
        lockRefused = false
        process = p
        confirmingMac = false
        // Design 36 N3: the keys never ride a Time Machine backup onto
        // another Mac (sticky exclusion; idempotent and cheap).
        let keyDir = Self.dataDir
        Task.detached { KeySafety.excludeKeysFromBackup(in: keyDir) }
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
        if !updateInProgress { invalidateUpdateMembership() }
        runningReleaseVerified = false
        stopCandidateRead()
        if attached {
            // Detach only: the daemon's node is the point of the unattended
            // restart — quitting the app must not stop it (docs/design/29).
            // The node switch being turned off stops it (`nodeSwitchedOff`).
            attached = false
            confirmingMac = false
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
        confirmingMac = false
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

    /// The latest transition wins; once both markers leave the bounded tail,
    /// retain the state until another transition. Re-reading the tail also
    /// handles a marker that was only partly written at the previous poll.
    private func refreshMacConfirmation() {
        let waiting = NodeMacConfirmation.waiting(in: nodeLogTail(65_536), previously: confirmingMac)
        if confirmingMac != waiting {
            confirmingMac = waiting
            watchdog.invalidate()
            refreshStopReason()
        }
    }

    private func exited(_ proc: Process) {
        // Stopped on purpose, or an older process (after a restart) finishing late.
        guard let current = process, current === proc else { return }
        invalidateUpdateMembership()
        runningReleaseVerified = false
        stopCandidateRead()
        let status = proc.terminationStatus
        let signaled = proc.terminationReason == .uncaughtSignal
        let afterWake = signaled && status == SIGUSR1 && (lastWakeSignal.map { Date().timeIntervalSince($0) < 5 } ?? false)
        logEvent("exit", "status=\(status) signaled=\(signaled)" + (afterWake ? " (died of the app's SIGUSR1 wake-up: an older node without the handler)" : ""))
        if status == 3 || status == 5 {  // UPGRADE REQUIRED / no proof verifier (see `watch_upgrades`, `install_verifier`)
            upgradeRequired = true
            onUpgradeNeeded?()
        }
        process = nil
        confirmingMac = false
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
            let daemonPID = unattended?.runningNodePID
            let expected = Self.helperBinaryURL
            let requestedAt = clock.now
            Task.detached {
                let sample: LocalRPC.VerifiedReply?
                if let pid = daemonPID, let expected {
                    sample = await LocalRPC.callVerified(rootPID: pid, port: port, expected: expected,
                                                         method: "aether_status", params: [])
                } else { sample = nil }
                let alive = sample != nil
                let releaseMatches = sample.map { NodeReleaseIdentity.hasWriterLease(status: $0.value) } ?? false
                await MainActor.run {
                    guard self.enabled, self.process == nil, !self.automaticRestartBlocked,
                          !self.updateInProgress, self.unattended?.runningNodePID == daemonPID else { return }
                    if UnattendedDecision.afterLockExit(rpcAlive: alive, releaseMatches: releaseMatches) == .attach,
                       let daemonPID {
                        self.logEvent("attach", "run.lock is held and the holder answers: attached to it")
                        self.verifiedStatusBinding = sample?.binding
                        self.verifiedStatusRequestedAt = requestedAt
                        self.attachToRunningNode(releaseVerified: releaseMatches, pid: daemonPID)
                    } else {
                        // The holder is dying, or is not a node at all: the
                        // gate retries in 2 s and says who holds the lock if
                        // it still does — never a tight spawn loop.
                        self.logEvent("lock", "run.lock is held and the holder does not answer")
                        self.lockRefused = true
                        Timer.scheduledTimer(withTimeInterval: 2, repeats: false) { [weak self] _ in
                            Task { @MainActor in self?.applyPower() }
                        }
                    }
                }
            }
            return
        }
        if status == Self.keysOnChainDataExit {
            // crates/node EXIT_KEYS_ON_CHAIN_DATA: the block-data folder holds
            // node keys (design 36 N2). The mover refuses such a folder, so
            // this means keys were copied there by hand: a person decides.
            logEvent("storage", "the node refused: node keys found in the block-data folder \(chainDataPath)")
            block(.storage)
            state = .failed(NodeWatchdog.Failure.storage.sentence)
            return
        }
        if status == Self.chainDataMissingExit {
            // The chosen block-data disk went away under the node (exit 13,
            // crates/node EXIT_CHAIN_DATA_MISSING): not a crash. The gate
            // says "디스크가 연결되지 않음" and starts it again when it returns.
            applyPower()
            return
        }
        switch watchdog.exited(clock.now, code: status, signaled: proc.terminationReason == .uncaughtSignal, log: nodeLogTail()) {
        case .restart(let after):
            // Restart with backoff (docs/design/24-self-healing.md layer 2):
            // the wallet is on remote nodes already, so a few seconds cost
            // nothing but a crash loop.
            state = .failed(String(localized: "The node stopped. Restarting it…"))
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
            block(failure)
            state = .failed(failure.sentence)
        case .rollback:
            // The updated binary cannot start: back to the previous one —
            // once, and only if that binary can still run the chain (red team
            // #3): an old binary on a chain it cannot read stops the node for
            // good, which is worse than the crash loop this was meant to fix.
            // Otherwise voting stops and the update is asked for again.
            let prev = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/NodeRollback.bundle/Contents/MacOS/aether.prev")
            if !usePreviousBinary, FileManager.default.isExecutableFile(atPath: prev.path) {
                let scheduled = scheduledProtocol
                Task.detached {
                    let prevProtocol = Self.protocolOf(prev)
                    await MainActor.run {
                        guard self.enabled, self.process == nil else { return }
                        if NodeWatchdog.rollbackAllowed(prevProtocol: prevProtocol, chainScheduled: scheduled) {
                            self.usePreviousBinary = true
                            self.state = .failed(String(localized: "The node did not start after the update, so the previous version is running again."))
                            self.watchdog.restarting()
                            self.start()
                        } else {
                            self.block(.upgradeNeeded)
                            self.state = .failed(NodeWatchdog.Failure.upgradeNeeded.sentence)
                            self.upgradeAsked = true
                            self.onUpgradeNeeded?()
                        }
                    }
                }
            } else {
                block(.other)
                state = .failed(NodeWatchdog.Failure.other.sentence)
            }
        case .none:
            state = .failed(String(localized: "The node stopped. Copy Diagnostics on Home shows why."))
        }
    }

    private var candidateProcess: Process?

    private func stopCandidateRead() {
        if let p = candidateProcess, p.isRunning { p.terminate() }
        candidateProcess = nil
    }

    /// Read (or create) this Mac's voting-node keys with the bundled helper.
    /// Hardware verification can wait indefinitely; it must not hold the
    /// main actor or delay launching the node that reports the waiting state.
    private func loadCandidate(_ binary: URL) {
        guard candidate == nil, candidateProcess == nil, !confirmingMac, answeredSinceStart,
              DataMigration.mayStartNode() == nil else { return }
        // RPC only starts after the node's identity setup. Waiting for it
        // avoids racing another candidate-info against fresh key creation.
        let p = Process(), out = Pipe()
        p.executableURL = binary
        p.arguments = ["candidate-info", "--data", Self.dataDir.path]
        p.standardOutput = out
        guard (try? p.run()) != nil else { return }
        candidateProcess = p
        Task.detached {
            p.waitUntilExit()
            let code = p.terminationStatus
            let entry: Candidate?
            if code == 0,
               let v = try? JSONSerialization.jsonObject(with: out.fileHandleForReading.readDataToEndOfFile()) as? [String: Any],
               let key = v["validator_key"] as? String, let node = v["node_id"] as? String,
               let beaconer = v["beaconer"] as? String {
                entry = Candidate(validatorKey: key, nodeId: node, beaconer: beaconer)
            } else {
                entry = nil
            }
            await MainActor.run {
                guard self.candidateProcess === p else { return }
                self.candidateProcess = nil
                if code == 6 {
                    // Missing/unreadable keys never mint a replacement identity.
                    if !self.identityNoticePosted {
                        self.identityNoticePosted = true
                        LocalNotice.post(title: Brand.name, body: NodeWatchdog.Failure.identityLost.sentence)
                    }
                    return
                }
                self.identityNoticePosted = false
                if let entry { self.candidate = entry }
            }
        }
    }

    /// The voting key's signature asking to be registered under `account` (the wallet, as operator).
    func ownership(account: String, chainId: UInt64) -> String? {
        guard let binary else { return nil }
        // `candidate-info` mints keys in an empty data directory: never while
        // an old identity waits unmigrated (release-070 review, B4).
        guard DataMigration.mayStartNode() == nil else { return nil }
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

    /// Running permission needs a fresh leased status and a membership
    /// read from the same signed listener. An unclaimed endpoint is unknown.
    var updateMembership: Bool? {
        if updateInProgress { return updateMembershipSnapshot.value(at: clock.now.seconds) }
        guard process != nil || attached else {
            guard !enabled, !lockRefused, unattended?.runningNodePID == nil,
                  let at = unclaimedProbeRequestedAt,
                  clock.now.elapsed(since: at) >= 0, clock.now.elapsed(since: at) <= 15,
                  unclaimedEndpointAbsent, Self.updateLockIsClear(in: Self.dataDir) else { return nil }
            return false
        }
        guard updateReleaseVerified else { return nil }
        return updateMembershipSnapshot.value(at: clock.now.seconds)
    }

    /// Off means positively no endpoint and no lock holder. File-open or
    /// lock-probe errors are unknown, rather than a fabricated unseated state.
    private nonisolated static func updateLockIsClear(in dir: URL) -> Bool {
        let fd = open(dir.appendingPathComponent("run.lock").path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
        if fd < 0 { return errno == ENOENT }
        defer { close(fd) }
        guard flock(fd, LOCK_EX | LOCK_NB) == 0 else { return false }
        flock(fd, LOCK_UN)
        return true
    }

    var onUpdateMomentChanged: (() -> Void)?
    private var updateMembershipSnapshot = UpdateWindow.MembershipSnapshot()
    private var lastUpdateMembershipCheck = MonotonicInstant.distantPast
    private var updateMembershipTask: Task<Void, Never>?
    private var unclaimedProbeRequestedAt: MonotonicInstant?
    private var unclaimedEndpointAbsent = false

    private func invalidateUpdateMembership() {
        updateMembershipTask?.cancel()
        updateMembershipTask = nil
        updateMembershipSnapshot.invalidate()
        lastUpdateMembershipCheck = .distantPast
        unclaimedProbeRequestedAt = nil
        unclaimedEndpointAbsent = false
    }

    /// Called while a held update waits, including when the node is off and
    /// has no poll timer. Completion wakes the updater before freshness expires.
    func refreshUpdateMembership() {
        guard !updateInProgress, updateMembershipTask == nil,
              clock.now.elapsed(since: lastUpdateMembershipCheck) > 10 else { return }
        let generation = updateMembershipSnapshot.generation
        let requestedAt = clock.now
        let port = Self.port
        if process == nil, !attached {
            guard !enabled, !lockRefused, unattended?.runningNodePID == nil,
                  Self.updateLockIsClear(in: Self.dataDir) else { return }
            lastUpdateMembershipCheck = requestedAt
            updateMembershipTask = Task { [weak self] in
                let absent = await LocalRPC.endpointIsAbsent(port: port)
                guard let self, self.updateMembershipSnapshot.generation == generation else { return }
                self.updateMembershipTask = nil
                guard !self.updateInProgress, self.process == nil, !self.attached else { return }
                self.unclaimedProbeRequestedAt = requestedAt
                self.unclaimedEndpointAbsent = absent
                let clear = absent && !self.enabled && self.unattended?.runningNodePID == nil
                    && Self.updateLockIsClear(in: Self.dataDir)
                self.updateMembershipSnapshot.observe(clear ? false : nil,
                    requestedAt: requestedAt.seconds, generation: generation)
                self.onUpdateMomentChanged?()
            }
            return
        }
        guard updateReleaseVerified, let leasedBinding = verifiedStatusBinding,
              let key = candidate?.validatorKey, let expected = Self.helperBinaryURL else { return }
        lastUpdateMembershipCheck = requestedAt
        updateMembershipTask = Task { [weak self] in
            let sample = await LocalRPC.callVerified(rootPID: leasedBinding.rootPID, port: port, expected: expected,
                                                     method: "aether_network", params: [])
            guard let self, self.updateMembershipSnapshot.generation == generation else { return }
            self.updateMembershipTask = nil
            guard !self.updateInProgress else { return }
            guard self.updateReleaseVerified, self.verifiedStatusBinding == leasedBinding,
                  sample?.binding == leasedBinding, self.candidate?.validatorKey == key else {
                self.updateMembershipSnapshot.observe(nil, requestedAt: requestedAt.seconds, generation: generation)
                self.onUpdateMomentChanged?()
                return
            }
            let membership = UpdateWindow.votingMembership(network: sample?.value, validatorKey: key)
            self.updateMembershipSnapshot.observe(membership, requestedAt: requestedAt.seconds, generation: generation)
            self.onUpdateMomentChanged?()
        }
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
        guard !updateInProgress, storageMovePercent == nil else { return }
        if attached {
            // A stall in the node we are attached to: take it over. The
            // watchdog's layer-2 rule applies no matter who started the node
            // (docs/design/24): stop the daemon's node, then start our own.
            unattended?.stopDaemonNode()
            watchdog.restarting()
            let timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] t in
                Task { @MainActor in
                    guard let self, self.enabled, self.process == nil else { t.invalidate(); self?.restartTimer = nil; return }
                    // A fired takeover must not stay behind as a "pending
                    // restart": the gate would wait on it forever.
                    self.restartTimer = nil
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
    private func attachToRunningNode(releaseVerified: Bool, pid: Int32) {
        invalidateUpdateMembership()
        guard releaseVerified else { return }
        releaseVerifiedPID = pid
        runningReleaseVerified = true
        if refusePersistedBindingMismatch() { return }
        attached = true
        attachMisses = 0
        switched = false
        state = .running
        nextCandidateRetry = clock.now
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
        if refusePersistedBindingMismatch() { return }
        refreshMacConfirmation()
        // The node owns verification retries. Restarting it while a read is
        // unavailable would reset its backoff and erase the calm waiting state.
        if confirmingMac {
            refreshStopReason()
            return
        }
        if candidate == nil, answeredSinceStart, clock.now >= nextCandidateRetry {
            nextCandidateRetry = clock.now.advanced(by: 60)
            if let binary { loadCandidate(binary) }
        }
        refreshUpdateMembership()
        refreshVoting()
        applyDuty()
        refreshProver()
        refreshHistoryKept()
        refreshUpgrade()
        refreshDisk()
        guard !checkInFlight else { return }
        checkInFlight = true
        let port = Self.port, switched = self.switched
        let monitoredPID = process?.processIdentifier ?? unattended?.runningNodePID
        let requestedAt = clock.now
        let expected = Self.helperBinaryURL
        Task.detached {
            // One reading of the local node covers all three feeds: its
            // height, its stage-wise activity counter (red team #2), and —
            // cached for the rollback decision (red team #3) — the newest
            // protocol the chain has scheduled.
            let sample: LocalRPC.VerifiedReply?
            if let pid = monitoredPID, let expected {
                sample = await LocalRPC.callVerified(rootPID: pid, port: port, expected: expected,
                                                     method: "aether_status", params: [])
            } else { sample = nil }
            let releaseMatches = sample.map { NodeReleaseIdentity.hasWriterLease(status: $0.value) } ?? false
            let status = releaseMatches ? sample?.value as? [String: Any] : nil
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
                guard (self.process?.processIdentifier ?? self.unattended?.runningNodePID) == monitoredPID else { return }
                self.runningReleaseVerified = status != nil && releaseMatches
                if !releaseMatches { self.invalidateUpdateMembership() }
                self.releaseVerifiedPID = self.runningReleaseVerified ? monitoredPID : nil
                self.verifiedStatusBinding = self.runningReleaseVerified ? sample?.binding : nil
                self.verifiedStatusRequestedAt = self.runningReleaseVerified ? requestedAt : nil
                if self.refusePersistedBindingMismatch() { return }
                guard !self.confirmingMac else { return }
                if self.attached, status == nil {
                    // The attached (daemon-started) node stopped answering:
                    // after a short grace (it may be restarting under the
                    // daemon), take the data directory back and run our own.
                    self.attachMisses += 1
                    if self.attachMisses >= 5, self.enabled, self.process == nil {
                        // The start gate checks the refusal marker and the
                        // run.lock holder again before taking over this data.
                        self.applyPower()
                    }
                    return
                }
                if self.attached { self.attachMisses = 0 }
                let answered = status != nil
                if self.rpcAnswering != answered { self.rpcAnswering = answered }
                if answered, !self.answeredSinceStart {
                    self.answeredSinceStart = true
                    // The node answered from its (new) block-data place:
                    // only now may the old copy of a move go.
                    if self.process != nil { self.finishBlockDataMove() }
                }
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
                MainActor.assumeIsolated {
                    self?.invalidateUpdateMembership()
                    self?.announceAvailability(leaving: true)
                }
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
                MainActor.assumeIsolated {
                    self?.invalidateUpdateMembership()
                    self?.announceAvailability(leaving: Self.onBattery)
                }
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
        if keepsAwake { return String(localized: "This Mac signs blocks now: \(Brand.name) keeps it from sleeping (the display can still sleep).") }
        return onlyOnPower
            ? String(localized: "While this Mac signs blocks on power, \(Brand.name) keeps it from sleeping.")
            : String(localized: "While this Mac signs blocks, \(Brand.name) keeps it from sleeping, on battery too.")
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
            LocalNotice.post(title: String(localized: "The network is paused"),
                             body: String(localized: "No new block for a minute. Keep this Mac awake and online: it is one of the Macs that sign blocks."))
        } else if since == nil, pauseNotified {
            pauseNotified = false
            LocalNotice.post(title: String(localized: "The network is running again"), body: String(localized: "New blocks are coming in again."))
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
    struct VerifiedReply {
        let value: Any
        let binding: NodeReleaseIdentity.Binding
    }

    static func callVerified(rootPID: Int32, port: UInt16, expected: URL,
                             method: String, params: [Any]) async -> VerifiedReply? {
        guard let sample = await NodeReleaseIdentity.readVerified(rootPID: rootPID, port: port, expected: expected,
            operation: { await call(port: port, method: method, params: params) }) else { return nil }
        return VerifiedReply(value: sample.value, binding: sample.binding)
    }

    /// Positive endpoint absence, rather than an unavailable/malformed RPC.
    /// Only a refused fresh TCP connect can confirm no local listener.
    static func endpointIsAbsent(port: UInt16) async -> Bool {
        await Task.detached {
            let fd = Darwin.socket(AF_INET, SOCK_STREAM, 0)
            guard fd >= 0 else { return false }
            defer { close(fd) }
            let flags = fcntl(fd, F_GETFL)
            guard flags >= 0, fcntl(fd, F_SETFL, flags | O_NONBLOCK) == 0,
                  fcntl(fd, F_SETFD, FD_CLOEXEC) == 0 else { return false }
            var address = sockaddr_in()
            address.sin_len = UInt8(MemoryLayout<sockaddr_in>.stride)
            address.sin_family = sa_family_t(AF_INET)
            address.sin_port = port.bigEndian
            address.sin_addr.s_addr = UInt32(0x7f00_0001).bigEndian
            let result = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.stride))
                }
            }
            if result == 0 { return false }
            if errno == ECONNREFUSED { return true }
            guard errno == EINPROGRESS || errno == EALREADY else { return false }
            var pending = pollfd(fd: fd, events: Int16(POLLOUT), revents: 0)
            guard poll(&pending, 1, 500) > 0 else { return false }
            var error: Int32 = 0
            var size = socklen_t(MemoryLayout<Int32>.stride)
            guard getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &size) == 0 else { return false }
            return error == ECONNREFUSED
        }.value
    }

    static func call(port: UInt16, method: String, params: [Any]) async -> Any? {
        guard let url = URL(string: "http://127.0.0.1:\(port)/") else { return nil }
        var req = URLRequest(url: url, timeoutInterval: 5)
        req.httpMethod = "POST"
        req.cachePolicy = .reloadIgnoringLocalCacheData
        req.setValue("close", forHTTPHeaderField: "Connection")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try? JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "id": 1, "method": method, "params": params])
        // A dedicated ephemeral session has no inherited keep-alive pool.
        let configuration = URLSessionConfiguration.ephemeral
        configuration.urlCache = nil
        configuration.connectionProxyDictionary = [:]
        configuration.urlCredentialStorage = nil
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        guard let (data, response) = try? await session.data(for: req),
              let http = response as? HTTPURLResponse, http.statusCode == 200,
              http.url?.scheme == "http", http.url?.host == "127.0.0.1", http.url?.port == Int(port),
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
