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
    /// Prove blocks on this Mac's GPU for rewards (protocol 2); paid to `proveAddress`.
    @AppStorage("proveBlocks") var prove = false {
        didSet { restartIfRunning() }
    }
    @AppStorage("proveAddress") var proveAddress = ""
    /// What the prover did last (from the node's `aether_proverStatus`).
    @Published private(set) var prover: ProverStatus?

    struct ProverStatus: Decodable, Equatable, Sendable {
        let running: Bool
        let proving: UInt64?
        let last_height: UInt64?
        let last_txs: Int?
        let last_seconds: Double?
        let proofs: UInt64?
        let error: String?
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

    private func restartIfRunning() {
        guard process != nil else { return }
        stop(keepSwitch: true)
        start()
    }

    private func refreshProver() {
        guard prove else {
            prover = nil
            return
        }
        let port = Self.port
        Task.detached {
            let v = await LocalRPC.call(port: port, method: "aether_proverStatus", params: [])
            let status = v.flatMap { try? JSONSerialization.data(withJSONObject: $0) }.flatMap { try? JSONDecoder().decode(ProverStatus.self, from: $0) }
            await MainActor.run { self.prover = status }
        }
    }

    /// Rewards this Mac's proofs earned, as CSV (for tax records).
    func rewardsCSV() async -> String? {
        guard !proveAddress.isEmpty,
              let list = await LocalRPC.call(port: Self.port, method: "aether_rewards", params: [proveAddress]) as? [[String: Any]] else { return nil }
        var csv = "proven_block,amount_aeth,paid_in_block,time_utc\n"
        let iso = ISO8601DateFormatter()
        for r in list {
            let amount = Wei.exact(LocalRPC.decimal(r["amount"]))
            let ms = (r["timestamp_ms"] as? NSNumber)?.doubleValue ?? 0
            csv += "\(r["proven"] ?? ""),\(amount),\(r["height"] ?? ""),\(iso.string(from: Date(timeIntervalSince1970: ms / 1000)))\n"
        }
        return csv
    }

    private func check() {
        refreshVoting()
        refreshProver()
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
