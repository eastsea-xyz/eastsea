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
        didSet { enabled ? startIfAllowed() : stop() }
    }
    /// Prove blocks on this Mac's GPU for rewards (protocol 2); paid to `proveAddress`.
    @AppStorage("proveBlocks") var prove = false {
        didSet { restartIfRunning() }
    }
    @AppStorage("proveAddress") var proveAddress = ""
    /// What the prover did last (from the node's `aether_proverStatus`).
    @Published private(set) var prover: ProverStatus?
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
        let error: String?
        /// Blocks between the chain head and the last block proven here.
        let lag: UInt64?
        /// The last reward received (wei, hex).
        let last_reward: String?
    }

    /// Run the node only while the Mac is on its power adapter (laptops).
    @AppStorage("nodeOnlyOnPower") var onlyOnPower = true {
        didSet { if enabled { applyPower() } }
    }
    /// Open Aether at login (the node then resumes if it was on).
    var startAtLogin: Bool {
        get { SMAppService.mainApp.status == .enabled }
        set {
            do {
                if newValue { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
            } catch {
                state = .failed("Login item: \(error.localizedDescription)")
            }
            objectWillChange.send()
        }
    }
    private var powerTimer: Timer?
    /// Held while this Mac is a validator (see `applyDuty`).
    let sleepGuard = SleepGuard(reason: "Aether: this Mac signs blocks for the network (voting node)")
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
        if onlyOnPower && Self.onBattery {
            if process != nil { stop(keepSwitch: true) }
            state = .waitingForPower
        } else if process == nil, restartTimer == nil {
            // A watchdog restart already scheduled keeps its backoff.
            start()
        }
    }

    static let port: UInt16 = 18_545
    /// Validator-to-validator port, used only while this Mac is voting.
    static let p2pPort: UInt16 = 19_101
    private(set) var process: Process?
    private var poll: Timer?

    static var dataDir: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/Aether/node", isDirectory: true)
    }

    private var binary: URL? {
        let helpers = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers")
        let name = usePreviousBinary ? "aether.prev" : "aether"
        let helper = helpers.appendingPathComponent(name)
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
    /// The last update's binary kept beside the current one: rolled back to
    /// when the new one cannot start (docs/design/24-self-healing.md layer 2).
    private var usePreviousBinary = false
    /// Polls in a row the switched-to local node was behind the network
    /// (red team #17): a few, so a second of lag at the tip does not flip it.
    private var behindPolls = 0
    /// The "this Mac's node key cannot be read" notice went out (once per
    /// bout; red team #5 — a person must restore the key).
    private var identityNoticePosted = false
    /// Sleep/wake observers (red team #9): the watchdog's timing is stale the
    /// moment the Mac sleeps. Added once, kept for the app's lifetime.
    private var wakeObservers: [NSObjectProtocol] = []

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
        guard process == nil else { return }
        guard let binary else {
            state = .failed("This build does not include the node")
            return
        }
        do {
            try FileManager.default.createDirectory(at: Self.dataDir, withIntermediateDirectories: true)
        } catch {
            state = .failed("\(error.localizedDescription)")
            return
        }
        loadCandidate(binary)
        var args = ["run", "--data", Self.dataDir.path, "--rpc-port", String(Self.port), "--port", String(Self.p2pPort), "--exit-with-parent"]
        if let network = Bundle.main.url(forResource: "network", withExtension: "json") {
            args += ["--network", network.path]
        }
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
        watchdog.started(Date())
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
        if !keepSwitch {
            powerTimer?.invalidate()
            powerTimer = nil
        }
        poll?.invalidate()
        tokenTimer?.invalidate()
        restartTimer?.invalidate()
        poll = nil
        restartTimer = nil
        switched = false
        behindPolls = 0
        useLocalNode(port: nil)
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
        if status == 3 || status == 5 { onUpgradeNeeded?() }  // UPGRADE REQUIRED / no proof verifier (see `watch_upgrades`, `install_verifier`)
        process = nil
        poll?.invalidate()
        switched = false
        useLocalNode(port: nil)  // the wallet reads other nodes from this moment on
        applyDuty()
        switch watchdog.exited(Date(), code: status, signaled: proc.terminationReason == .uncaughtSignal, log: nodeLogTail()) {
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
                            self.state = .failed(NodeWatchdog.Failure.upgradeNeeded.sentence)
                            self.upgradeAsked = true
                            self.onUpgradeNeeded?()
                        }
                    }
                }
            } else {
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
                LocalNotice.post(title: "Aether", body: NodeWatchdog.Failure.identityLost.sentence)
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

    private var lastVotingCheck = Date.distantPast

    private func refreshVoting() {
        guard let key = candidate?.validatorKey, Date().timeIntervalSince(lastVotingCheck) > 10 else { return }
        lastVotingCheck = Date()
        Task.detached {
            let status = try? votingNodeStatus(validatorKey: key)
            await MainActor.run {
                // Published only when it changed: the poll runs every 2 s.
                if let status, self.voting != status { self.voting = status }
                self.applyDuty()
            }
        }
    }

    /// Switch the wallet to the local node once it has caught up with the network.
    private var switched = false

    private func restartIfRunning() {
        guard process != nil else { return }
        stop(keepSwitch: true)
        watchdog.restarting()
        start()
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

    /// Rewards this Mac's proofs earned, as CSV (for tax records). The records
    /// the node keeps name no transaction hash — a reward is paid by the block
    /// that first used the proof, not by a transaction — so the block stands in
    /// for it (docs/research/node-reward-tax-2026.md).
    func rewardsCSV() async -> String? {
        guard !proveAddress.isEmpty,
              let list = await LocalRPC.call(port: Self.port, method: "aether_rewards", params: [proveAddress, 10_000]) as? [[String: Any]] else { return nil }
        var csv = "time_utc,kind,proven_block,paid_in_block,amount_aeth\n"
        let iso = ISO8601DateFormatter()
        for r in list {
            let amount = Wei.exact(LocalRPC.decimal(r["amount"]))
            let ms = (r["timestamp_ms"] as? NSNumber)?.doubleValue ?? 0
            csv += "\(iso.string(from: Date(timeIntervalSince1970: ms / 1000))),\(r["kind"] ?? ""),\(r["proven"] ?? ""),\(r["height"] ?? ""),\(amount)\n"
        }
        return csv
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
                self.onUpgradeNeeded?()
            }
        }
    }

    private func check() {
        refreshVoting()
        applyDuty()
        refreshProver()
        refreshUpgrade()
        let port = Self.port
        Task.detached {
            // One reading of the local node covers all three feeds: its
            // height, its stage-wise activity counter (red team #2), and —
            // cached for the rollback decision (red team #3) — the newest
            // protocol the chain has scheduled.
            let status = await LocalRPC.call(port: port, method: "aether_status", params: []) as? [String: Any]
            let local = (status?["height"] as? NSNumber)?.uint64Value ?? localNodeHeight(port: port)
            // The network's height stays in the picture after the switch too
            // (red team #17): the wallet's own multi-source verified view,
            // whether or not it is reading through this node.
            let network = (try? chainStatus())?.height
            let activity = (status?["activity"] as? NSNumber)?.uint64Value
            await MainActor.run {
                guard self.process != nil, let local else { return }
                if self.height != local { self.height = local }
                if let scheduled = (status?["newest_scheduled"] as? NSNumber)?.uint64Value {
                    self.scheduledProtocol = scheduled
                }
                if self.switched {
                    // Red team #14/#17: a local node that falls behind gives
                    // the wallet's traffic back to the network's nodes — after
                    // a few polls in a row, so a second of lag at the tip (or
                    // one slow answer) does not flip it.
                    if let network, local + 2 < network {
                        self.behindPolls += 1
                        if self.behindPolls >= 5 {
                            self.behindPolls = 0
                            self.switched = false
                            useLocalNode(port: nil)
                            self.state = .starting
                        }
                    } else {
                        self.behindPolls = 0
                    }
                } else if local + 2 >= (network ?? 0) {
                    useLocalNode(port: port)
                    self.switched = true
                    self.behindPolls = 0
                    if self.state != .running { self.state = .running }
                } else if self.state != .starting {
                    self.state = .starting  // catching up; the wallet keeps asking validators meanwhile
                }
                // A stall (the network moves, ours has not for a minute) —
                // unless the node's own work counter is moving (a snapshot
                // download, a store recovery, a backlog replay), and with
                // twice the patience while this Mac is in the voting set (its
                // restart costs the network a signature). The incident of
                // 2026-09-29 looked exactly like this, and "끊김" told the
                // user nothing.
                if case .restart = self.watchdog.polled(Date(), local: local, network: network, activity: activity, voting: self.isValidator) {
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
        wakeObservers = [
            center.addObserver(forName: NSWorkspace.willSleepNotification, object: nil, queue: .main) { [weak self] _ in
                Task { @MainActor in self?.watchdog.invalidate() }
            },
            center.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: .main) { [weak self] _ in
                Task { @MainActor in
                    self?.watchdog.invalidate()
                    self?.check()
                }
            },
        ]
    }
}
// MARK: validator duty (docs/design/13-roadmap.md F, P0)

extension NodeController {
    /// This Mac's node runs and the network has it in the voting set.
    var isValidator: Bool { process != nil && voting?.voting == true }

    /// Keep the Mac awake while it signs blocks: a sleeping member is a missing
    /// vote, and a third of them asleep pauses the network. On battery with
    /// "Only while on the power adapter" on, the node is off anyway.
    var keepsAwake: Bool { isValidator && !(onlyOnPower && Self.onBattery) }

    /// One line for Settings: what Aether does about sleep.
    var awakeNote: String {
        if keepsAwake { return "This Mac signs blocks now: Aether keeps it from sleeping (the display can still sleep)." }
        return onlyOnPower
            ? "While this Mac signs blocks on power, Aether keeps it from sleeping."
            : "While this Mac signs blocks, Aether keeps it from sleeping, on battery too."
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
