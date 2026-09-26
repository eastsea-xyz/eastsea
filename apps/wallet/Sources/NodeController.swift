#if os(macOS)
import Foundation
import IOKit.ps
import ServiceManagement
import SwiftUI

/// The node inside the app (Transmission-style on/off). On: the bundled `aether`
/// follows the chain, verifying and re-executing every block on this Mac, and
/// the wallet talks to it. Off, or when the app quits: it stops, and the wallet
/// goes back to asking validators directly (still verifying everything).
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
    @AppStorage("nodeEnabled") var enabled = false {
        didSet { enabled ? startIfAllowed() : stop() }
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
        } else if process == nil {
            start()
        }
    }

    static let port: UInt16 = 18_545
    private var process: Process?
    private var poll: Timer?

    static var dataDir: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/Aether/node", isDirectory: true)
    }

    private var binary: URL? {
        let helper = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/aether")
        return FileManager.default.isExecutableFile(atPath: helper.path) ? helper : nil
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
        var args = ["follow", "--data", Self.dataDir.path, "--rpc-port", String(Self.port), "--exit-with-parent"]
        if let network = Bundle.main.url(forResource: "network", withExtension: "json") {
            args += ["--network", network.path]
        }
        let p = Process()
        p.executableURL = binary
        p.arguments = args
        let log = Self.dataDir.appendingPathComponent("node.log")
        FileManager.default.createFile(atPath: log.path, contents: nil)
        if let h = try? FileHandle(forWritingTo: log) {
            p.standardOutput = h
            p.standardError = h
        }
        p.terminationHandler = { [weak self] proc in
            Task { @MainActor in self?.exited(status: proc.terminationStatus) }
        }
        do {
            try p.run()
        } catch {
            state = .failed(error.localizedDescription)
            return
        }
        process = p
        state = .starting
        poll = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.check() }
        }
    }

    func stop(keepSwitch: Bool = false) {
        if !keepSwitch {
            powerTimer?.invalidate()
            powerTimer = nil
        }
        poll?.invalidate()
        poll = nil
        switched = false
        useLocalNode(port: nil)
        if let p = process, p.isRunning { p.terminate() }
        process = nil
        state = .off
    }

    private func exited(status: Int32) {
        guard process != nil else { return }  // stopped on purpose
        process = nil
        poll?.invalidate()
        switched = false
        useLocalNode(port: nil)
        state = .failed("The node stopped (exit \(status)); see \(Self.dataDir.appendingPathComponent("node.log").path)")
    }

    /// Switch the wallet to the local node once it has caught up with the network.
    private var switched = false

    private func check() {
        let port = Self.port, switched = self.switched
        Task.detached {
            let local = localNodeHeight(port: port)
            let network = switched ? nil : (try? chainStatus())?.height
            await MainActor.run {
                guard self.process != nil, let local else { return }
                self.height = local
                if self.switched || local + 2 >= (network ?? 0) {
                    useLocalNode(port: port)
                    self.switched = true
                    self.state = .running
                } else {
                    self.state = .starting  // catching up; the wallet keeps asking validators meanwhile
                }
            }
        }
    }
}
#endif
