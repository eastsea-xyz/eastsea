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
    /// stub `Helpers/eastsea-node-daemon.sh`, which waits for the marker and
    /// runs the node as the marker's user — the plist is static in the signed
    /// bundle, so per-user choices travel in the marker instead.
    static let plistName = "com.pipln.eastsea.node.plist"

    /// One of these, in the user's words (Settings).
    enum Status: Equatable {
        case off
        /// Registered, but the user has not allowed it in System Settings ▸
        /// Login Items yet — the daemon does not run until they do.
        case needsApproval
        case approved
        case failed(String)
    }

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

    /// Re-read the approval state (called on Settings appearance and after
    /// register/unregister — the user approves outside the app, so the state
    /// changes while we are not looking).
    func refreshStatus() {
        guard enabled else {
            status = .off
            return
        }
        switch service.status {
        case .enabled: status = .approved
        case .requiresApproval, .notRegistered: status = .needsApproval
        case .notFound: status = .failed("Daemon support missing from this build")
        default: status = .off
        }
    }

    /// The system-settings pane where the user approves the daemon.
    func openApprovalPane() {
        try? SMAppService.openSystemSettingsLoginItems()
    }

    private func applyEnabledChange() {
        if enabled {
            do {
                if service.status == .notRegistered { try service.register() }
            } catch {
                status = .failed("Daemon: \(error.localizedDescription)")
                return
            }
        } else {
            try? service.unregister()
            Marker.remove()
            stopDaemonNode()
        }
        refreshStatus()
        syncMarker()
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
        guard enabled, nodeEnabled, !wrongLocation else {
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
                                                       storageFlag: StorageSetting.flag(shards: storageShards ?? StorageSetting.defaultShards)),
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

    /// Stop the daemon's node (the user turned the node off in the app).
    /// Only the wrapper's own pid file, and only a process that still is the
    /// node/wrapper binary — never an arbitrary recycled pid.
    func stopDaemonNode() {
        guard let pid = Marker.pid(), Self.processIsOurs(pid) else { return }
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
        let dict: [String: Any] = [
            "user": NSUserName(),
            "bundle": Bundle.main.bundlePath,
            "data": NodeController.dataDir.path,
            "binary": binary.path,
            "argv": argv,
            "prove": proveAddress ?? "",
        ]
        try? FileManager.default.createDirectory(at: NodeController.dataDir, withIntermediateDirectories: true)
        if let data = try? PropertyListSerialization.data(fromPropertyList: dict, format: .xml, options: 0) {
            try? data.write(to: url, options: .atomic)
        }
    }

    static func remove() {
        try? FileManager.default.removeItem(at: url)
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
