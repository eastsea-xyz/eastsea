import AppKit
import Security
import ServiceManagement

/// Runs the bridge's one job, in order, showing each step (BridgeView):
/// 1. stop the old node cleanly (SIGTERM, then wait for its run.lock),
/// 2. find EastSea — an installed genuine one, or the newest on EastSea's feed,
/// 3. download it and check Sparkle's EdDSA signature,
/// 4. check the app inside: Developer ID, Pipln's team, EastSea's bundle id,
///    notarized (Gatekeeper),
/// 5. install it in /Applications (or ~/Applications), re-checked in place,
/// 6. unregister Aether's login item,
/// 7. open EastSea, which moves the data itself, verified.
/// Nothing here reads, moves or deletes wallet or node data.
@MainActor
final class Mover: ObservableObject {
    enum StepState: Equatable { case waiting, working, done, failed }
    struct Step: Identifiable, Equatable {
        let id: Int
        let title: String
        var state: StepState = .waiting
    }

    @Published private(set) var steps: [Step]
    @Published private(set) var problem: String?
    @Published private(set) var finished = false
    @Published private(set) var busy = false
    @Published private(set) var eastSeaURL: URL?

    private let fm = FileManager.default
    private let ko = Locale.preferredLanguages.first?.hasPrefix("ko") ?? false

    init() {
        let ko = self.ko
        steps = (ko ? ["Aether 노드 종료", "EastSea 찾기", "내려받기와 서명 확인", "앱 서명·공증 확인",
                       "응용 프로그램 폴더에 설치", "Aether 로그인 항목 해제", "EastSea 열기"]
                    : ["Stop Aether's node", "Find EastSea", "Download and check its signature",
                       "Check the app's signature and notarization", "Install in Applications",
                       "Remove Aether's login item", "Open EastSea"])
            .enumerated().map { Step(id: $0.offset, title: $0.element) }
        if let done = MovedRecord.load(), fm.fileExists(atPath: done.path) {
            eastSeaURL = done
            finished = true
            steps = steps.map { var s = $0; s.state = .done; return s }
        }
    }

    // MARK: running

    func run() {
        guard !busy else { return }
        busy = true
        problem = nil
        finished = false
        steps = steps.map { var s = $0; s.state = .waiting; return s }
        Task {
            defer { busy = false }
            do {
                try await step(0) { try await self.stopOldNode() }
                let installed = try await step(1) { self.installedEastSea() }
                var app = installed
                if app == nil {
                    let item = try await step(1) { try await self.newestEastSea() }
                    let dmg = try await step(2) { try await self.download(item) }
                    let mounted = try await step(3) { try await self.mountAndCheck(dmg: dmg, item: item) }
                    defer { mounted.detach() }
                    app = try await step(4) { try await self.install(mounted.app, version: item.version) }
                } else {
                    mark(2, .done); mark(3, .done); mark(4, .done)
                }
                guard let app else { throw BridgeError.text(ko ? "EastSea를 찾지 못했습니다." : "EastSea was not found.") }
                try await step(5) { self.unregisterLoginItem() }
                try await step(6) { try await self.launch(app) }
                eastSeaURL = app
                MovedRecord.save(app)
                finished = true
            } catch {
                problem = (error as? BridgeError)?.message ?? error.localizedDescription
            }
        }
    }

    @discardableResult
    private func step<T>(_ i: Int, _ work: () async throws -> T) async throws -> T {
        mark(i, .working)
        do {
            let value = try await work()
            mark(i, .done)
            return value
        } catch {
            mark(i, .failed)
            throw error
        }
    }

    private func mark(_ i: Int, _ state: StepState) {
        guard steps.indices.contains(i) else { return }
        steps[i].state = state
    }

    // MARK: 1. the old node

    private var supportURL: URL { fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0] }
    private var oldLock: URL { supportURL.appendingPathComponent("Aether/node/run.lock") }

    /// Whether some process holds the old node's run.lock. Probed and
    /// released at once — holding it would make EastSea's migration defer.
    /// A lock file that does not exist means no old node ever ran here.
    private func oldNodeRunning() -> Bool {
        let fd = open(oldLock.path, O_RDWR)
        guard fd >= 0 else { return false }
        defer { close(fd) }
        if flock(fd, LOCK_EX | LOCK_NB) == 0 {
            flock(fd, LOCK_UN)
            return false
        }
        return true
    }

    private func stopOldNode() async throws {
        guard oldNodeRunning() else { return }
        for attempt in 0..<90 {
            if attempt % 30 == 0 {
                let ps = await run("/bin/ps", ["-axww", "-o", "pid=,uid=,args="]).output
                for pid in BridgePlan.oldNodePIDs(psOutput: ps, uid: getuid()) { kill(pid, SIGTERM) }
            }
            try await Task.sleep(nanoseconds: 1_000_000_000)
            if !oldNodeRunning() { return }
        }
        throw BridgeError.text(ko
            ? "Aether 노드가 아직 실행 중입니다. 터미널에서 실행한 aether 가 있으면 끄고 다시 시도해 주세요."
            : "Aether's node is still running. If you started `aether` in a terminal, stop it, then try again.")
    }

    // MARK: 2. finding EastSea

    /// A genuine EastSea that is new enough, already installed anywhere
    /// LaunchServices knows (except a mounted image or the Trash).
    private func installedEastSea() -> URL? {
        NSWorkspace.shared.urlsForApplications(withBundleIdentifier: BridgePlan.eastSeaBundleID)
            .filter { !$0.path.hasPrefix("/Volumes/") && !$0.path.contains("/.Trash/") }
            .first { url in
                BridgePlan.installedIsEnough(version: version(of: url), valid: signatureProblem(url) == nil)
            }
    }

    private func newestEastSea() async throws -> BridgePlan.Item {
        let (data, response) = try await URLSession.shared.data(from: BridgePlan.feedURL)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw BridgeError.text(ko ? "EastSea 업데이트 정보를 받지 못했습니다." : "Could not read EastSea's update feed.")
        }
        let os = ProcessInfo.processInfo.operatingSystemVersion
        let system = "\(os.majorVersion).\(os.minorVersion).\(os.patchVersion)"
        guard let item = BridgePlan.choose(BridgePlan.parseAppcast(data), systemVersion: system) else {
            throw BridgeError.text(ko ? "이 Mac에 맞는 EastSea 릴리스가 피드에 없습니다." : "EastSea's feed lists no release for this Mac.")
        }
        return item
    }

    // MARK: 3. downloading

    private var cacheDir: URL {
        fm.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent("com.pipln.aether.bridge", isDirectory: true)
    }

    private func download(_ item: BridgePlan.Item) async throws -> URL {
        guard let url = item.url else { throw BridgeError.text("no download URL") }
        try fm.createDirectory(at: cacheDir, withIntermediateDirectories: true)
        let (temp, response) = try await URLSession.shared.download(from: url)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw BridgeError.text(ko ? "EastSea를 내려받지 못했습니다." : "The EastSea download failed.")
        }
        let dmg = cacheDir.appendingPathComponent("EastSea-\(item.version).dmg")
        try? fm.removeItem(at: dmg)
        try fm.moveItem(at: temp, to: dmg)
        let size = (try? fm.attributesOfItem(atPath: dmg.path)[.size] as? Int64) ?? -1
        guard size == item.length else {
            throw BridgeError.text(ko ? "내려받은 파일 크기가 맞지 않습니다." : "The download has the wrong size.")
        }
        guard let key = Bundle.main.object(forInfoDictionaryKey: "SUPublicEDKey") as? String,
              BridgePlan.verifyEdDSA(file: dmg, signatureBase64: item.edSignature, publicKeyBase64: key) else {
            try? fm.removeItem(at: dmg)
            throw BridgeError.text(ko ? "내려받은 파일의 서명이 맞지 않아 지웠습니다." : "The download's signature did not verify; it was deleted.")
        }
        return dmg
    }

    // MARK: 4–5. checking and installing

    struct Mounted {
        let point: URL
        let app: URL
        func detach() {
            let path = point.path
            Task.detached {
                _ = await Mover.runDetached("/usr/bin/hdiutil", ["detach", "-quiet", path])
                try? FileManager.default.removeItem(atPath: path)
            }
        }
    }

    /// Mount the verified image read-only and check the app inside it.
    private func mountAndCheck(dmg: URL, item: BridgePlan.Item) async throws -> Mounted {
        let point = cacheDir.appendingPathComponent("mnt-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: point, withIntermediateDirectories: true)
        let attach = await run("/usr/bin/hdiutil", ["attach", "-nobrowse", "-readonly", "-noautoopen",
                                                    "-mountpoint", point.path, dmg.path])
        guard attach.status == 0 else {
            try? fm.removeItem(at: point)
            throw BridgeError.text("hdiutil attach: \(attach.output)")
        }
        let mounted = Mounted(point: point, app: point.appendingPathComponent(BridgePlan.eastSeaAppName))
        do {
            try verify(mounted.app, expectedVersion: item.version)
            let gate = await run("/usr/sbin/spctl", ["--assess", "--type", "execute", "--verbose=2", mounted.app.path])
            guard BridgePlan.gatekeeperAccepts(output: gate.output, exitCode: gate.status) else {
                throw BridgeError.text((ko ? "Apple 공증 확인에 실패했습니다: " : "Apple's notarization check failed: ") + gate.output)
            }
        } catch {
            mounted.detach()
            throw error
        }
        return mounted
    }

    private func verify(_ app: URL, expectedVersion: String?) throws {
        let bundle = Bundle(url: app)
        guard bundle?.bundleIdentifier == BridgePlan.eastSeaBundleID else {
            throw BridgeError.text(ko ? "내려받은 앱이 EastSea가 아닙니다." : "The downloaded app is not EastSea.")
        }
        if let expectedVersion, version(of: app) != expectedVersion {
            throw BridgeError.text(ko ? "내려받은 앱의 버전이 피드와 다릅니다." : "The app's version differs from the feed's.")
        }
        if let why = signatureProblem(app) {
            throw BridgeError.text((ko ? "EastSea 서명 확인 실패: " : "EastSea's signature did not check out: ") + why)
        }
    }

    private func install(_ source: URL, version: String) async throws -> URL {
        let dir = BridgePlan.installDirectory(applicationsWritable: fm.isWritableFile(atPath: "/Applications"),
                                              home: fm.homeDirectoryForCurrentUser)
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        let dest = dir.appendingPathComponent(BridgePlan.eastSeaAppName)
        let existing = fm.fileExists(atPath: dest.path) ? dest : nil
        switch BridgePlan.existingDecision(installedVersion: existing.flatMap(version(of:)),
                                           installedValid: existing.map { signatureProblem($0) == nil } ?? false,
                                           candidateVersion: version) {
        case .keep: return dest
        case .replace, .none: break
        }
        let staging = dir.appendingPathComponent(".EastSea.app.installing-\(UUID().uuidString)")
        let copy = await run("/usr/bin/ditto", [source.path, staging.path])
        guard copy.status == 0 else {
            try? fm.removeItem(at: staging)
            throw BridgeError.text("ditto: \(copy.output)")
        }
        do {
            try verify(staging, expectedVersion: version)
        } catch {
            try? fm.removeItem(at: staging)
            throw error
        }
        if let existing { try fm.trashItem(at: existing, resultingItemURL: nil) }
        guard rename(staging.path, dest.path) == 0 else {
            try? fm.removeItem(at: staging)
            throw BridgeError.text(String(cString: strerror(errno)))
        }
        return dest
    }

    /// nil when `app` is genuine EastSea: valid, strict, every nested piece
    /// of code signed, and signed by Pipln's Developer ID.
    private func signatureProblem(_ app: URL) -> String? {
        var code: SecStaticCode?
        guard SecStaticCodeCreateWithPath(app as CFURL, [], &code) == errSecSuccess, let code else { return "unreadable" }
        var requirement: SecRequirement?
        guard SecRequirementCreateWithString(BridgePlan.requirement as CFString, [], &requirement) == errSecSuccess else {
            return "bad requirement"
        }
        var error: Unmanaged<CFError>?
        let flags = SecCSFlags(rawValue: kSecCSCheckAllArchitectures | kSecCSStrictValidate | kSecCSCheckNestedCode)
        guard SecStaticCodeCheckValidityWithErrors(code, flags, requirement, &error) == errSecSuccess else {
            return error.map { ($0.takeRetainedValue() as Error).localizedDescription } ?? "invalid"
        }
        return nil
    }

    private func version(of app: URL) -> String? {
        Bundle(url: app)?.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
    }

    // MARK: 6–7. stepping aside

    /// Aether's own login item (`SMAppService.mainApp`, this bundle id): the
    /// only way it can be removed is from an app with this bundle id.
    private func unregisterLoginItem() {
        let service = SMAppService.mainApp
        if service.status == .enabled || service.status == .requiresApproval {
            try? service.unregister()
        }
    }

    private func launch(_ app: URL) async throws {
        let config = NSWorkspace.OpenConfiguration()
        config.activates = true
        _ = try await NSWorkspace.shared.openApplication(at: app, configuration: config)
    }

    func openEastSea() {
        guard let eastSeaURL else { return }
        NSWorkspace.shared.openApplication(at: eastSeaURL, configuration: NSWorkspace.OpenConfiguration())
    }

    // MARK: processes

    private func run(_ tool: String, _ args: [String]) async -> (status: Int32, output: String) {
        await Mover.runDetached(tool, args)
    }

    nonisolated static func runDetached(_ tool: String, _ args: [String]) async -> (status: Int32, output: String) {
        await withCheckedContinuation { done in
            DispatchQueue.global().async {
                let p = Process(), pipe = Pipe()
                p.executableURL = URL(fileURLWithPath: tool)
                p.arguments = args
                p.standardOutput = pipe
                p.standardError = pipe
                do { try p.run() } catch {
                    done.resume(returning: (-1, error.localizedDescription))
                    return
                }
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                p.waitUntilExit()
                done.resume(returning: (p.terminationStatus, String(decoding: data, as: UTF8.self)))
            }
        }
    }
}

enum BridgeError: Error {
    case text(String)
    var message: String { if case .text(let s) = self { return s }; return "" }
}

/// Where the bridge remembers that it finished (its own file, not the old
/// app's preferences domain — EastSea copies that one during its migration).
enum MovedRecord {
    private static var url: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("AetherBridge/moved-to-eastsea.txt")
    }

    static func load() -> URL? {
        guard let path = try? String(contentsOf: url, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines),
              !path.isEmpty else { return nil }
        return URL(fileURLWithPath: path)
    }

    static func save(_ app: URL) {
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? app.path.write(to: url, atomically: true, encoding: .utf8)
    }
}
