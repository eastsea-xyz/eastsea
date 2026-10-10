#if os(macOS)
import Darwin
import Foundation
import ServiceManagement
import SwiftUI

/// The "keep this Mac's node running after restarts" half of the app
/// (docs/design/29-unattended-restart.md). Registers the bundled LaunchDaemon
/// (`SMAppService.daemon`, approved once by the user in System Settings ▸
/// Login Items), keeps the opt-in marker `unattended.plist` in the node's data
/// directory in step with the app's settings, and stops the daemon's node when
/// the user turns the node off. Pure decisions live in `UnattendedDecision`
/// (Tests/unattended); this file only talks to the system.
@MainActor
final class UnattendedDaemon: ObservableObject {
    /// The daemon plist shipped in the app bundle
    /// (`Contents/Library/LaunchDaemons`). Its `BundleProgram` is the root
    /// stub `Contents/Resources/eastsea-node-daemon.sh`, which waits for the marker and
    /// runs the node as the marker's user — the plist is static in the signed
    /// bundle, so per-user choices travel in the marker instead.
    static let plistName = "com.pipln.eastsea.node.plist"

    /// The same pure state drives Settings and node-status.log.
    typealias Status = UnattendedDecision.Status

    @Published private(set) var status: Status = .off
    /// The honest power facts (`pmset -g`, `fdesetup status`). The app reads,
    /// never writes, these.
    @Published private(set) var power = UnattendedDecision.PowerFacts()

    /// The opt-in switch. Default ON for Macs in or entering the voting set
    /// (`UnattendedDecision.defaultEnabled`); once the user moves it, theirs.
    @AppStorage("nodeUnattended") var enabled = false {
        didSet {
            objectWillChange.send()
            if !applyingDefault { userChose = true }
            applyEnabledChange()
        }
    }
    @AppStorage("unattendedUserChose") private var userChose = false
    /// A default-on change is not the user's hand (see `applyDefault`).
    private var applyingDefault = false
    /// Storage moves and quiet updates may overlap; neither may restore
    /// respawn while the other still owns its suspension.
    private var respawnSuspensions = 0

    @discardableResult
    func suspendRespawn() -> Bool {
        #if WALLET_SCREENS
        return true
        #endif
        respawnSuspensions += 1
        return Marker.remove()
    }

    func resumeRespawn() {
        guard respawnSuspensions > 0 else { return }
        respawnSuspensions -= 1
        syncMarker()
    }

    /// Set by the app: the marker must not point a daemon at a node while the
    /// user keeps the node switch off, or from a place the app cannot live in.
    var nodeEnabled = false
    var wrongLocation = false
    /// The history-storage shard budget the daemon's node runs with (설정 ▸
    /// 역사 보관), resolved by the node controller — the same value its own
    /// child starts with — and pushed whenever the setting or the registry
    /// view changes. `nil` until then: the node's own default holds.
    var storageShards: Int?

    private var service: SMAppService { SMAppService.daemon(plistName: Self.plistName) }
    private var registrationFailure: String?

    /// Status is a registration lookup, not a bundle/notarization check.
    /// Validate the two packaged inputs separately before interpreting it.
    private var bundledServiceAvailable: Bool {
        let root = Bundle.main.bundleURL
        let plist = root.appendingPathComponent("Contents/Library/LaunchDaemons/\(Self.plistName)")
        guard let data = try? Data(contentsOf: plist),
              let values = (try? PropertyListSerialization.propertyList(from: data, format: nil)) as? [String: Any],
              values["Label"] as? String == "com.pipln.eastsea.node",
              values["BundleProgram"] as? String == "Contents/Resources/eastsea-node-daemon.sh" else { return false }
        return FileManager.default.isExecutableFile(atPath: root.appendingPathComponent("Contents/Resources/eastsea-node-daemon.sh").path)
    }

    private func registrationStatus(_ value: SMAppService.Status) -> UnattendedDecision.ServiceStatus {
        switch value {
        case .notRegistered: return .notRegistered
        case .enabled: return .enabled
        case .requiresApproval: return .requiresApproval
        case .notFound: return .notFound
        @unknown default: return .unknown
        }
    }

    /// AppStorage restores the choice without invoking its didSet. Reconcile
    /// a saved opt-in once at startup, including a service never seen by macOS.
    func restore() {
        guard !LaunchRecovery.shared.isSafeMode else { return }
        if enabled { applyEnabledChange() } else { refreshStatus() }
    }

    /// Re-read the approval state (called on Settings appearance and after
    /// register/unregister — the user approves outside the app, so the state
    /// changes while we are not looking).
    func refreshStatus() {
        let before = status
        status = UnattendedDecision.status(enabled: enabled, bundledService: bundledServiceAvailable,
                                          service: registrationStatus(service.status),
                                          registrationFailure: registrationFailure)
        if status == .off || status == .approved || status == .needsApproval { registrationFailure = nil }
        // Every change of the daemon's state goes to node-status.log: the app
        // node keeps running whatever this says (it never hands over to a
        // daemon that is not answering — the app only attaches to a node
        // that already holds run.lock and answers), and a remote diagnosis
        // must be able to tell the two apart.
        if status != before {
            NodeStatusLog.append(NodeStatusLog.line(at: Date(), event: "unattended", detail: "\(before) -> \(status)", facts: nil),
                                 in: NodeController.dataDir)
        }
        // Approval can arrive outside the app after a registration error
        // removed the marker. Reconcile it even if our own node stayed up.
        syncMarker()
    }

    /// The block data lives on a disk the daemon cannot open.
    var blockDataOnExternalDisk: Bool {
        // A full internal disk may delay cfprefsd. The move's durable
        // record, also used by app startup, wins over a stale preference.
        guard let path = try? selectedChainDataPath() else { return true }
        return path.hasPrefix("/Volumes/")
    }

    private func selectedChainDataPath() throws -> String {
        if let root = try BlockDataMove.authoritativeRoot(in: NodeController.dataDir) {
            guard BlockDataMove.selectionAvailable(root, internalRoot: NodeController.dataDir) else { throw BlockDataMove.Failure.unavailable }
            return root.path == BlockDataLocation.resolvedRoot(NodeController.dataDir).path ? "" : root.path
        }
        return UserDefaults.standard.string(forKey: "nodeChainDataPath") ?? ""
    }

    /// The one sentence (and the Login Items button) while the daemon waits
    /// for the person's approval; nil otherwise. The node keeps running in
    /// the app meanwhile.
    var approvalSentence: String? {
        if enabled, blockDataOnExternalDisk {
            return String(localized: "The block data is on an external disk, so after a restart the node starts when the app opens.")
        }
        guard enabled, status == .needsApproval else { return nil }
        return String(localized: "To keep the node running while the Mac is locked or after a restart, turn on “Allow in the Background”. Until then it runs inside the app.")
    }

    /// The system-settings pane where the user approves the daemon.
    func openApprovalPane() {
        try? SMAppService.openSystemSettingsLoginItems()
    }

    private func applyEnabledChange() {
        guard !LaunchRecovery.shared.isSafeMode else { return }
        #if WALLET_SCREENS
        // The screens renderer (scripts/wallet-screens.sh) never touches the real data.
        return 
        #endif
        registrationFailure = nil
        if enabled {
            let service = self.service
            do {
                if UnattendedDecision.shouldRegister(enabled: enabled, bundledService: bundledServiceAvailable,
                                                      service: registrationStatus(service.status)), !wrongLocation {
                    try service.register()
                }
            } catch {
                // Withheld consent is an expected approval step. An actual
                // registration error stays visible until a retry or approval.
                if service.status != .requiresApproval && service.status != .enabled {
                    NodeStatusLog.append(NodeStatusLog.line(at: Date(), event: "unattended_registration_failed",
                                                           detail: error.localizedDescription, facts: nil),
                                         in: NodeController.dataDir)
                    registrationFailure = String(localized: "Could not keep the node running after restarts.")
                }
            }
        } else {
            try? service.unregister()
            Marker.remove()
            stopDaemonNode()
        }
        refreshStatus()
    }

    /// The default-on rule (a Mac in or entering the voting set keeps running
    /// through restarts by default). Applied only while the user has not
    /// moved the switch; a default change is not a user choice, so it must
    /// not freeze the switch (`userChose` stays unset).
    func applyDefault(registered: Bool) {
        guard !userChose, !applyingDefault, wrongLocation == false else { return }
        let wanted = UnattendedDecision.defaultEnabled(registered: registered)
        if wanted != enabled {
            applyingDefault = true
            enabled = wanted
            applyingDefault = false
        }
    }

    /// Keep the marker in step with the app's settings. The marker is the
    /// single source of what the daemon runs: the same argv the app's own node
    /// takes (minus `--exit-with-parent`), so a restart changes nothing.
    func syncMarker() {
        // An aborted update must not start background work from recovery.
        // The saved preference is applied again on normal startup/retry.
        guard !LaunchRecovery.shared.isSafeMode else { return }
        #if WALLET_SCREENS
        return
        #endif
        // The marker makes the root daemon run the node — past the app's own
        // start gate — so it obeys the same gate (release-070 review, B4).
        guard respawnSuspensions == 0, enabled, status.allowsMarker, nodeEnabled, !wrongLocation, DataMigration.mayStartNode() == nil else {
            Marker.remove()
            return
        }
        // A launchd daemon cannot reach /Volumes (TCC, exit 78 on the
        // testnet Macs): with the block data on a chosen external disk the
        // node runs inside the app only, and Settings says so.
        guard let chainDataPath = try? selectedChainDataPath(), !chainDataPath.hasPrefix("/Volumes/") else {
            Marker.remove()
            return
        }
        Marker.write(binary: NodeController.helperBinaryURL,
                     argv: UnattendedDecision.nodeArgv(
                        dataDir: NodeController.dataDir.path,
                        rpcPort: NodeController.port,
                        p2pPort: NodeController.p2pPort,
                        networkPath: Bundle.main.url(forResource: "network", withExtension: "json")?.path,
                        proverFlags: ProverFlags.build(memory: UserDefaults.standard.string(forKey: "proverMemory") ?? "auto",
                                                       cores: UserDefaults.standard.string(forKey: "proverCores") ?? "half",
                                                       battery: UserDefaults.standard.bool(forKey: "proverOnBattery"),
                                                       activeProcessors: ProcessInfo.processInfo.activeProcessorCount),
                                                       storageFlag: StorageSetting.flag(shards: storageShards ?? StorageSetting.defaultShards),
                        locationFlags: BlockDataLocation.flags(
                            chainDataPath: chainDataPath,
                            archive: UserDefaults.standard.bool(forKey: "nodeArchive")),
                        presenceFlags: PresenceCountry.preference().flags),
                     proveAddress: UserDefaults.standard.string(forKey: "proveAddress"))
    }

    /// The node switch turned on (the node is running or about to): the
    /// marker follows the current settings so the daemon would run exactly
    /// what the app runs. (`wrongLocation` stays as the app keeps it.)
    func nodeSwitchedOn() {
        nodeEnabled = true
        syncMarker()
    }

    /// The node switch turned off: the daemon must not bring the node back
    /// after the next restart either — the marker goes away, and a daemon
    /// node that is running stops (docs/design/29).
    func nodeSwitchedOff() {
        #if WALLET_SCREENS
        return
        #endif
        nodeEnabled = false
        syncMarker()
        stopDaemonNode()
    }

    /// Read the power facts the honest sentences stand on. Off the main actor:
    /// two subprocess calls, once per Settings appearance.
    func refreshPower() {
        Task.detached {
            let autorestart = Self.readPowerFact(binary: "/usr/bin/pmset", args: ["-g"]).flatMap(UnattendedDecision.autorestart(from:))
            let fileVault = Self.readPowerFact(binary: "/usr/bin/fdesetup", args: ["status"]).flatMap(UnattendedDecision.fileVault(from:))
            await MainActor.run {
                self.power = UnattendedDecision.PowerFacts(fileVault: fileVault, autorestart: autorestart)
            }
        }
    }

    private nonisolated static func readPowerFact(binary: String, args: [String]) -> String? {
        let p = Process(), out = Pipe()
        p.executableURL = URL(fileURLWithPath: binary)
        p.arguments = args
        p.standardOutput = out
        p.standardError = Pipe()
        guard (try? p.run()) != nil else { return nil }
        p.waitUntilExit()
        guard p.terminationStatus == 0 else { return nil }
        return String(data: out.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)
    }

    /// The wrapper's recorded PID, accepted only while its executable is
    /// still ours. Release verification is an additional check by the caller.
    var runningNodePID: Int32? {
        guard let pid = Marker.pid(), Self.processIsOurs(pid) else { return nil }
        return pid
    }

    /// Stop the daemon's node (the user turned the node off in the app).
    /// Only the wrapper's own pid file, and only a process that still is the
    /// node/wrapper binary — never an arbitrary recycled pid.
    func stopDaemonNode(expectedPID: Int32? = nil) {
        #if WALLET_SCREENS
        return
        #endif
        guard let pid = Marker.pid(), expectedPID == nil || expectedPID == pid,
              Self.processIsOurs(pid) else { return }
        kill(pid, SIGTERM)
    }

    /// `proc_pidpath` must name our wrapper or the node binary it exec'd.
    private static func processIsOurs(_ pid: pid_t) -> Bool {
        var path = [CChar](repeating: 0, count: 1024)
        guard proc_pidpath(pid, &path, 1024) > 0 else { return false }
        let p = String(cString: path)
        return p.hasSuffix("/eastsea-node-daemon.sh") || p.hasSuffix("/eastsea-node-wrapper.sh") || p.hasSuffix("/Helpers/aether")
    }
}

/// The opt-in marker `unattended.plist` in the node's data directory. Its
/// presence (and contents) is what the root stub wakes up to: the user, the
/// app bundle, the node binary, the exact argv, and the proving address.
@MainActor
private enum Marker {
    static var url: URL { NodeController.dataDir.appendingPathComponent("unattended.plist") }

    static func write(binary: URL?, argv: [String], proveAddress: String?) {
        guard let binary else { return }
        var dict: [String: Any] = [
            "user": NSUserName(),
            "bundle": Bundle.main.bundlePath,
            "data": NodeController.dataDir.path,
            "binary": binary.path,
            "argv": argv,
            "prove": proveAddress ?? "",
        ]
        // The default M49 region survives independently of country consent.
        // New wrappers discard legacy country arguments unless this marker
        // records an explicit answer and the exact authorized preference.
        dict.merge(PresenceCountry.preference().markerFields) { _, choice in choice }
        try? FileManager.default.createDirectory(at: NodeController.dataDir, withIntermediateDirectories: true)
        if let data = try? PropertyListSerialization.data(fromPropertyList: dict, format: .xml, options: 0) {
            try? data.write(to: url, options: .atomic)
        }
    }

    @discardableResult
    static func remove() -> Bool {
        // Persist the absence before releasing the source writer. A reboot
        // must not resurrect a marker naming the pre-move chain-data path.
        guard unlink(url.path) == 0 || errno == ENOENT else { return false }
        let fd = open(url.deletingLastPathComponent().path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
        guard fd >= 0 else { return false }
        defer { close(fd) }
        return fsync(fd) == 0
    }

    /// The pid the daemon's wrapper recorded (`unattended.pid` in the data
    /// directory) — the wrapper before it execs the node, the node after.
    static func pid() -> pid_t? {
        guard let text = try? String(contentsOf: NodeController.dataDir.appendingPathComponent("unattended.pid"), encoding: .utf8),
              let pid = pid_t(text.trimmingCharacters(in: .whitespacesAndNewlines)), pid > 1 else { return nil }
        return pid
    }
}
#endif
