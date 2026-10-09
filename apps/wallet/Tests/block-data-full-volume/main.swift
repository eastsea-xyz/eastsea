// Real ENOSPC coverage. The image, mount and destination are confined to
// the repository tmp directory; this executable never starts an app or node.
import Foundation
#if os(macOS)
import Darwin

struct FixtureFailure: Error, CustomStringConvertible {
    let description: String
}

func require(_ condition: @autoclosure () throws -> Bool, _ message: String) throws {
    if try !condition() { throw FixtureFailure(description: message) }
}

struct FileSnapshot: Equatable {
    let device: UInt64
    let inode: UInt64
    let bytes: Int64
    let blocks: Int64
    let modifiedSeconds: Int64
    let modifiedNanoseconds: Int64

    init(_ url: URL) throws {
        var value = stat()
        try require(lstat(url.path, &value) == 0, "stat \(url.path): errno \(errno)")
        device = UInt64(UInt32(bitPattern: value.st_dev))
        inode = UInt64(value.st_ino)
        bytes = Int64(value.st_size)
        blocks = Int64(value.st_blocks)
        modifiedSeconds = Int64(value.st_mtimespec.tv_sec)
        modifiedNanoseconds = Int64(value.st_mtimespec.tv_nsec)
    }
}

func mounted(at point: URL) -> Bool {
    var value = statfs()
    guard statfs(point.path, &value) == 0 else { return false }
    let mount = withUnsafeBytes(of: value.f_mntonname) {
        String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self)
    }
    return mount == point.path
}

@discardableResult
func runHelper(_ executable: String, _ arguments: [String], in fixture: URL, tmpRoot: URL) throws -> Data {
    let name = "\(URL(fileURLWithPath: executable).lastPathComponent)-\(UUID().uuidString)"
    let outputURL = fixture.appendingPathComponent("\(name).out")
    let errorURL = fixture.appendingPathComponent("\(name).err")
    let fm = FileManager.default
    try require(fm.createFile(atPath: outputURL.path, contents: nil)
                && fm.createFile(atPath: errorURL.path, contents: nil), "create hdiutil capture files")
    let output = try FileHandle(forWritingTo: outputURL)
    let errors = try FileHandle(forWritingTo: errorURL)
    defer { try? output.close(); try? errors.close() }
    let process = Process()
    process.executableURL = URL(fileURLWithPath: executable)
    process.arguments = arguments
    process.currentDirectoryURL = fixture
    var environment = ProcessInfo.processInfo.environment
    environment["TMPDIR"] = tmpRoot.path
    process.environment = environment
    process.standardInput = FileHandle.nullDevice
    process.standardOutput = output
    process.standardError = errors
    try process.run()
    let deadline = DispatchWorkItem { if process.isRunning { process.terminate() } }
    DispatchQueue.global().asyncAfter(deadline: .now() + 45, execute: deadline)
    process.waitUntilExit()
    deadline.cancel()
    if process.terminationStatus != 0 {
        let detail = ((try? String(contentsOf: errorURL, encoding: .utf8)) ?? "")
            + ((try? String(contentsOf: outputURL, encoding: .utf8)) ?? "")
        throw FixtureFailure(description: "\(executable) \(arguments.first ?? "") failed (\(process.terminationStatus)): \(detail)")
    }
    return try Data(contentsOf: outputURL)
}

@discardableResult
func hdiutil(_ arguments: [String], in fixture: URL, tmpRoot: URL) throws -> Data {
    try runHelper("/usr/bin/hdiutil", arguments, in: fixture, tmpRoot: tmpRoot)
}

func fillToNoSpace(_ url: URL) throws -> UInt64 {
    let fd = open(url.path, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0o600)
    try require(fd >= 0, "create fixture filler: errno \(errno)")
    defer { close(fd) }
    let chunk = Data(repeating: 0xa7, count: 1 << 20)
    var quantum = chunk.count
    var written: UInt64 = 0
    var sawNoSpace = false
    while true {
        let count = chunk.withUnsafeBytes { write(fd, $0.baseAddress, quantum) }
        if count > 0 {
            written += UInt64(count)
            try require(written <= 128 << 20, "filler escaped the bounded 64 MiB image")
            continue
        }
        if count < 0, errno == EINTR { continue }
        try require(count < 0 && errno == ENOSPC, "filler must fail with real ENOSPC, got errno \(errno)")
        sawNoSpace = true
        // Exhaust the final allocation block even when a larger write fails
        // or succeeds partially. A one-byte ENOSPC is the final boundary.
        if quantum > 1 { quantum = max(1, quantum / 2); continue }
        break
    }
    try require(sawNoSpace && fsync(fd) == 0, "flush fully allocated filler")
    var space = statfs()
    try require(fstatfs(fd, &space) == 0 && space.f_bavail == 0,
                "source must have exactly zero available blocks after ENOSPC (\(space.f_bavail))")
    return written
}

func exerciseFullVolume() async throws {
    let fm = FileManager.default
    let tmpRoot = URL(fileURLWithPath: fm.currentDirectoryPath, isDirectory: true)
        .appendingPathComponent("tmp", isDirectory: true).standardizedFileURL
    try fm.createDirectory(at: tmpRoot, withIntermediateDirectories: true)
    if let configured = ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"] {
        try require(configured.hasPrefix("/")
                    && URL(fileURLWithPath: configured).resolvingSymlinksInPath().path
                        == tmpRoot.resolvingSymlinksInPath().path,
                    "AETHER_AGENT_TEST_TMP must be this repository's absolute tmp directory")
    }
    let fixture = tmpRoot.appendingPathComponent("block-data-full-volume-\(UUID().uuidString)", isDirectory: true)
    let point = fixture.appendingPathComponent("mount", isDirectory: true)
    let image = fixture.appendingPathComponent("source.dmg")
    let target = fixture.appendingPathComponent("destination", isDirectory: true)
    try fm.createDirectory(at: point, withIntermediateDirectories: true)
    var moveFD: Int32?
    var attachedDevice: String?
    var completed = false
    defer {
        if let fd = moveFD { close(fd) }
        if let device = attachedDevice {
            do { try hdiutil(["detach", device], in: fixture, tmpRoot: tmpRoot) }
            catch {
                fputs("fixture detach retry: \(error)\n", stderr)
                do { try hdiutil(["detach", "-force", device], in: fixture, tmpRoot: tmpRoot) }
                catch { fputs("FAIL: fixture mount remains at \(point.path): \(error)\n", stderr) }
            }
        }
        if completed && !mounted(at: point) { try? fm.removeItem(at: fixture) }
        else { fputs("fixture artifacts: \(fixture.path)\n", stderr) }
    }
    try hdiutil(["create", "-size", "64m", "-fs", "HFS+", "-volname", "MoveFullFixture",
                 "-type", "UDIF", image.path], in: fixture, tmpRoot: tmpRoot)
    // DiskImages refuses its framework mount path on an external workspace.
    // Attach only this image, then mount its own HFS slice directly; this
    // needs no privileged helper and keeps the mount inside repository tmp.
    let attachment = try hdiutil(["attach", "-nomount", "-nobrowse", "-plist", image.path],
                                 in: fixture, tmpRoot: tmpRoot)
    guard let plist = try PropertyListSerialization.propertyList(from: attachment, options: [], format: nil) as? [String: Any],
          let entities = plist["system-entities"] as? [[String: Any]] else {
        throw FixtureFailure(description: "hdiutil must identify the fixture image's own attached devices")
    }
    let devices = entities.compactMap { $0["dev-entry"] as? String }
    let roots = devices.filter { $0.range(of: "^/dev/disk[0-9]+$", options: .regularExpression) != nil }
    try require(roots.count == 1, "the fixture image must attach exactly one whole device")
    attachedDevice = roots[0]
    let slices = entities.filter { ($0["content-hint"] as? String) == "Apple_HFS" }
        .compactMap { $0["dev-entry"] as? String }
    try require(slices.count == 1 && slices[0].hasPrefix(roots[0] + "s")
                && slices[0].range(of: "^/dev/disk[0-9]+s[0-9]+$", options: .regularExpression) != nil,
                "mount only the HFS slice belonging to the task-owned image")
    try runHelper("/sbin/mount_hfs", ["-o", "nobrowse,noowners", slices[0], point.path],
                  in: fixture, tmpRoot: tmpRoot)
    try require(mounted(at: point), "the source image must mount at the repository fixture path")
    let source = point.appendingPathComponent("node", isDirectory: true)
    let follow = source.appendingPathComponent("follow", isDirectory: true)
    try fm.createDirectory(at: follow, withIntermediateDirectories: true)
    let state = follow.appendingPathComponent("state.db")
    let endpoint = follow.appendingPathComponent("wallet-node.key")
    let validator = source.appendingPathComponent("validator.key")
    let lock = source.appendingPathComponent("run.lock")
    let history = Data(repeating: 0x93, count: 8 << 20)
    let endpointBytes = Data("fixture-endpoint-private-key".utf8)
    let validatorBytes = Data("fixture-validator-private-key".utf8)
    let diagnosticBytes: [String: Data] = [
        "node.log": Data(repeating: 0x6e, count: 1 << 20),
        "node-status.log": Data(repeating: 0x73, count: 64 << 10),
    ]
    try history.write(to: state)
    try endpointBytes.write(to: endpoint)
    try validatorBytes.write(to: validator)
    try Data("existing-run-lock-inode".utf8).write(to: lock)
    for (name, bytes) in diagnosticBytes { try bytes.write(to: source.appendingPathComponent(name)) }
    let initialState = try FileSnapshot(state)
    let initialEndpoint = try FileSnapshot(endpoint)
    let initialValidator = try FileSnapshot(validator)
    let initialLock = try FileSnapshot(lock)
    let statusLog = source.appendingPathComponent(NodeStatusLog.fileName)
    let initialStatusLog = try FileSnapshot(statusLog)
    let filled = try fillToNoSpace(point.appendingPathComponent("filler.bin"))
    NodeStatusLog.append("full-volume fixture: a diagnostic must not prevent moving\n", in: source)
    try require(try FileSnapshot(statusLog) == initialStatusLog
                && Data(contentsOf: statusLog) == diagnosticBytes[NodeStatusLog.fileName],
                "status logging must skip the full volume without changing diagnostic bytes or allocation")
    guard let sourceID = BlockDataMove.identity(source) else {
        throw FixtureFailure(description: "production identity must recognize the genuine nested fixture mount")
    }
    moveFD = await BlockDataMove.holdRunLock(in: source, timeout: 0)
    guard let fd = moveFD else { throw FixtureFailure(description: "existing run.lock must be acquired with zero free bytes") }
    try require(fcntl(fd, F_GETFL) & O_ACCMODE == O_RDONLY, "existing lock must use a read-only descriptor")
    try require(try FileSnapshot(lock) == initialLock, "lock acquisition must preserve the existing inode and bytes")
    var progress: [Double] = []
    let meter = DataMigration.ProgressMeter(reportEvery: 1 << 20) { progress.append($0) }
    meter.expect(Int64(history.count) * 2)
    let copied = try BlockDataMove.copy(source: source, target: target, sourceID: sourceID,
                                        internalRoot: source, preservingInternalKeys: false, meter: meter)
    try require(copied == UInt64(history.count), "full-volume copy publishes every block-data byte")
    try require(progress.count >= 2 && progress.last == 1, "copy and verification must both report progress through completion")
    try require(zip(progress, progress.dropFirst()).allSatisfy { pair in pair.0 <= pair.1 }, "full-volume callbacks must be monotonic")
    try require(try Data(contentsOf: target.appendingPathComponent("follow/state.db")) == history,
                "destination bytes must match the full source")
    try require(try FileSnapshot(state) == initialState && Data(contentsOf: state) == history,
                "source state must remain untouched until explicit destination confirmation")
    try require(try FileSnapshot(endpoint) == initialEndpoint && Data(contentsOf: endpoint) == endpointBytes,
                "the endpoint key must stay unchanged on the source")
    try require(try FileSnapshot(validator) == initialValidator && Data(contentsOf: validator) == validatorBytes,
                "the validator key must stay unchanged on the source")
    try require(!fm.fileExists(atPath: target.appendingPathComponent("follow/wallet-node.key").path)
                && !fm.fileExists(atPath: target.appendingPathComponent("validator.key").path), "keys must never move")
    // This reads only the production durable journal, with no preference or
    // controller memory available to rescue an incomplete commit.
    try require(try BlockDataMove.authoritativeRoot(in: source)?.path == target.path,
                "the committed target must survive lost preference writes")
    let record = try JSONDecoder().decode(BlockDataMove.Record.self,
                                         from: Data(contentsOf: source.appendingPathComponent(BlockDataMove.recordName)))
    try require(record.committed && !record.cleanupDone, "the journal commits without authorizing premature deletion")
    let names = try fm.contentsOfDirectory(atPath: target.path)
    var preservedLogs = 0
    for (name, bytes) in diagnosticBytes {
        let backups = names.filter { $0.hasPrefix(".\(name).before-storage-move-") }
        let original = try Data(contentsOf: source.appendingPathComponent(name))
        if original.isEmpty {
            try require(backups.count == 1, "reclaimed \(name) must have exactly one destination backup")
            try require(try Data(contentsOf: target.appendingPathComponent(backups[0])) == bytes,
                        "backed-up \(name) must preserve every diagnostic byte")
            preservedLogs += 1
        } else {
            try require(original == bytes && backups.isEmpty, "unreclaimed diagnostics must be untouched")
        }
    }
    try require(preservedLogs > 0, "real journal ENOSPC must exercise verified diagnostic preservation")
    close(fd)
    moveFD = nil
    BlockDataMove.cleanup(confirmedTarget: fixture, internalRoot: source)
    try require(fm.fileExists(atPath: state.path), "a different target must not authorize cleanup")
    BlockDataMove.cleanup(confirmedTarget: target, internalRoot: source)
    try require(!fm.fileExists(atPath: state.path), "confirmed destination cleanup removes the verified old state")
    try require(try Data(contentsOf: endpoint) == endpointBytes && Data(contentsOf: validator) == validatorBytes,
                "confirmation cleanup must retain both private keys")
    try require(try Data(contentsOf: target.appendingPathComponent("follow/state.db")) == history,
                "confirmation cleanup must retain destination block data")
    try hdiutil(["detach", roots[0]], in: fixture, tmpRoot: tmpRoot)
    attachedDevice = nil
    try require(!mounted(at: point), "the fixture must detach its source image")
    completed = true
    print("PASS block-data-full-volume: real ENOSPC, zero available blocks, \(filled) filler bytes, \(progress.count) monotonic callbacks, \(preservedLogs) preserved diagnostic logs")
}

do { try await exerciseFullVolume() }
catch { fputs("FAIL block-data-full-volume: \(error)\n", stderr); exit(1) }
#else
fputs("FAIL block-data-full-volume: macOS hdiutil is required\n", stderr)
exit(1)
#endif
