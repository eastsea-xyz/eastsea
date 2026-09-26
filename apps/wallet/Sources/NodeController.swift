#if os(macOS)
import Foundation
import IOKit.ps
import ServiceManagement
import SwiftUI

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
    /// Validator-to-validator port, used only while this Mac is voting.
    static let p2pPort: UInt16 = 19_101
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
        loadCandidate(binary)
        var args = ["run", "--data", Self.dataDir.path, "--rpc-port", String(Self.port), "--port", String(Self.p2pPort), "--exit-with-parent"]
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

    /// Read (or create) this Mac's voting-node keys with the bundled helper.
    private func loadCandidate(_ binary: URL) {
        guard candidate == nil else { return }
        let p = Process(), out = Pipe()
        p.executableURL = binary
        p.arguments = ["candidate-info", "--data", Self.dataDir.path]
        p.standardOutput = out
        guard (try? p.run()) != nil else { return }
        p.waitUntilExit()
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
            await MainActor.run { if let status { self.voting = status } }
        }
    }

    /// Switch the wallet to the local node once it has caught up with the network.
    private var switched = false

    private func check() {
        refreshVoting()
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
