// 블록 데이터 위치 and 전체 기록 보관 (BlockDataLocation.swift): which disks
// may hold the block data, the node flags each choice makes, and the archive
// requirements on the archive node's real numbers.
//   scripts/test-swift-pure.sh   (run block-data)
import Foundation
#if os(macOS)
import AppKit

// Legacy cleanup preferences stay in memory; fixtures never write cfprefsd.
final class MoveMemoryDefaults: UserDefaults {
    private let lock = NSLock()
    private var values: [String: Any] = [:]
    init() { super.init(suiteName: nil)! }
    override func object(forKey key: String) -> Any? { lock.lock(); defer { lock.unlock() }; return values[key] }
    override func set(_ value: Any?, forKey key: String) {
        lock.lock(); defer { lock.unlock() }
        if let value { values[key] = value } else { values.removeValue(forKey: key) }
    }
    override func set(_ value: Bool, forKey key: String) { set(value as Any?, forKey: key) }
    override func removeObject(forKey key: String) { set(nil as Any?, forKey: key) }
}

// Exercise the real mover against fixtures, without starting a node or app.
@MainActor final class NodeController {
    nonisolated static let storageMoveLockTimeout: TimeInterval = 0.05
    nonisolated static let storageMoveDefaults = MoveMemoryDefaults()
    nonisolated static let dataDir = URL(fileURLWithPath: ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"]!)
        .appendingPathComponent("block-data-internal-\(UUID().uuidString)")
    var selectionLockChecks: [Bool] = []
    var chainDataPath = "" {
        didSet {
            if stops > 0, chainDataPath != oldValue { selectionLockChecks.append(Self.lockHeld(in: Self.dataDir)) }
        }
    }
    var storageMoveOffersDiskUtility = false
    var updateInProgress = false
    var storageMovePercent: Int?
    var storageMoveError: String?
    var attached = false
    var unattended: MoveDaemonStub? = MoveDaemonStub()
    var archive = false
    var mountObservers: [NSObjectProtocol] = []
    var stops = 0
    var markerAtStops: [Bool] = []
    func stop(keepSwitch: Bool) {
        markerAtStops.append(unattended?.markerEnabled ?? false)
        stops += 1
    }
    func logEvent(_ event: String, _ message: String) {}
    func applyPower() {}
    nonisolated static func lockHeld(in dir: URL) -> Bool {
        let fd = open(dir.appendingPathComponent("run.lock").path, O_RDONLY)
        guard fd >= 0 else { return false }
        defer { close(fd) }
        if flock(fd, LOCK_EX | LOCK_NB) == 0 { flock(fd, LOCK_UN); return false }
        return errno == EWOULDBLOCK
    }
}
@MainActor final class MoveDaemonStub {
    var markerEnabled = true
    var suspensionSucceeds = true
    var suspensions = 0
    var pauseCalls = 0
    var resumeCalls = 0
    var markerAtDaemonStops: [Bool] = []
    @discardableResult func suspendRespawn() -> Bool {
        pauseCalls += 1
        suspensions += 1
        if suspensionSucceeds { markerEnabled = false }
        return suspensionSucceeds
    }
    func resumeRespawn() {
        resumeCalls += 1
        suspensions = max(0, suspensions - 1)
        syncMarker()
    }
    func stopDaemonNode() { markerAtDaemonStops.append(markerEnabled) }
    func syncMarker() { markerEnabled = suspensions == 0 }
}
enum HealthCheck { static let korean = false }
#endif
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }
let GiB: UInt64 = 1_073_741_824

func vol(_ format: String, local: Bool = true, ro: Bool = false, free: UInt64 = 500 * GiB) -> BlockDataLocation.Volume {
    BlockDataLocation.Volume(name: "SSD", format: format, isLocal: local, isReadOnly: ro, isInternal: false, freeBytes: free)
}

// MARK: which disks

check(BlockDataLocation.validate(vol("apfs"), dataBytes: 20 * GiB) == nil, "APFS is fine")
check(BlockDataLocation.validate(vol("hfs"), dataBytes: 20 * GiB) == nil, "Mac OS Extended is fine")
check(BlockDataLocation.validate(vol("exfat"), dataBytes: 1) == .unsupportedFormat("exfat"), "exFAT is refused")
check(BlockDataLocation.validate(vol("msdos"), dataBytes: 1) == .unsupportedFormat("msdos"), "FAT is refused")
check(BlockDataLocation.validate(vol("smbfs"), dataBytes: 1) == .networkShare, "an SMB share is refused")
check(BlockDataLocation.validate(vol("apfs", local: false), dataBytes: 1) == .networkShare, "any non-local volume is refused")
check(BlockDataLocation.validate(vol("apfs", ro: true), dataBytes: 1) == .readOnly, "read-only is refused")
check(BlockDataLocation.validate(vol("apfs", free: 26 * GiB), dataBytes: 20 * GiB) == .notEnoughSpace(freeBytes: 26 * GiB, neededBytes: 27 * GiB),
      "the data plus the node's 7 GB resume margin must fit")
check(BlockDataLocation.validate(vol("apfs", free: 27 * GiB), dataBytes: 20 * GiB) == nil, "exactly enough is enough")
check(BlockDataLocation.sentence(.unsupportedFormat("exfat"), ko: true)
      == "이 디스크는 블록 저장에 안전하지 않은 형식(exFAT)이에요. 디스크 유틸리티에서 APFS로 지우면 쓸 수 있어요 (디스크 안의 파일은 지워져요).",
      "exFAT: why, and how to fix it in Disk Utility")
check(BlockDataLocation.sentence(.unsupportedFormat("exfat"), ko: false)
      == "This disk's format (exFAT) is not safe for block data. Erase it as APFS in Disk Utility to use it (this deletes the files on the disk).",
      "exFAT in English")
check(BlockDataLocation.Problem.unsupportedFormat("msdos").fixInDiskUtility && !BlockDataLocation.Problem.readOnly.fixInDiskUtility,
      "only a format problem offers Disk Utility")
check(BlockDataLocation.sentence(.notEnoughSpace(freeBytes: 26 * GiB, neededBytes: 27 * GiB), ko: false) == "Not enough space: 26.0 GB free, 27.0 GB needed.",
      "exact numbers")
for p in [BlockDataLocation.Problem.networkShare, .unsupportedFormat("ntfs"), .readOnly, .notEnoughSpace(freeBytes: 1, neededBytes: 2), .inUse] {
    for ko in [true, false] { check(!BlockDataLocation.sentence(p, ko: ko).isEmpty, "\(p) has words") }
}

// MARK: the folder and the flags

let picked = URL(fileURLWithPath: "/Volumes/외장 SSD/Node")
check(BlockDataLocation.chainDir(picked: picked).path == "/Volumes/외장 SSD/Node/EastSea Block Data", "a folder is made inside the pick")
check(BlockDataLocation.chainDir(picked: BlockDataLocation.chainDir(picked: picked)) == BlockDataLocation.chainDir(picked: picked),
      "picking the data folder itself uses it as is")
check(BlockDataLocation.volumeName(ofPath: "/Volumes/외장 SSD/Node/EastSea Block Data") == "외장 SSD", "volume name from the path")
check(BlockDataLocation.volumeName(ofPath: "/Users/me/x") == nil, "not on /Volumes")
check(BlockDataLocation.flags(chainDataPath: "", archive: false) == [], "default: no flags, today's argv exactly")
check(BlockDataLocation.flags(chainDataPath: "/Volumes/X/EastSea Block Data", archive: false) == ["--chain-data", "/Volumes/X/EastSea Block Data"],
      "a chosen disk passes --chain-data")
check(BlockDataLocation.flags(chainDataPath: "", archive: true) == ["--archive"], "archive passes --archive")
check(BlockDataLocation.flags(chainDataPath: "/v", archive: true) == ["--chain-data", "/v", "--archive"], "both")
let argv = UnattendedDecision.nodeArgv(dataDir: "/d", rpcPort: 1, p2pPort: 2, networkPath: nil, proverFlags: [],
                                       storageFlag: "--max-shards=8", locationFlags: BlockDataLocation.flags(chainDataPath: "/v", archive: true))
check(argv == ["run", "--data", "/d", "--rpc-port", "1", "--port", "2", "--max-shards=8", "--chain-data", "/v", "--archive"],
      "the app's node and the daemon's take the same argv, location last: \(argv)")
check(argv[2] == "/d", "keys stay in --data (the internal disk)")
check(BlockDataLocation.movedDirs == ["follow", "archive"], "only the follower's and the archive's state move")
check(BlockDataLocation.keepInternal.contains("wallet-node.key"), "the follower's endpoint key stays on the internal disk")

#if os(macOS)
// R01: a real nested destination must be refused before a write or shutdown.
@MainActor func runMoveFixtureTests() async throws {
let moveFixture = NodeController.dataDir.deletingLastPathComponent()
    .appendingPathComponent("block-data-move-\(UUID().uuidString)")
try FileManager.default.createDirectory(at: moveFixture, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: moveFixture); try? FileManager.default.removeItem(at: NodeController.dataDir) }
// R07/R03 alias followup. These fixtures use the existing sync/copy APIs.
// Select key or directories to capture each independent baseline failure.
let aliasCase = ProcessInfo.processInfo.environment["AETHER_STORAGE_ALIAS_CASE"] ?? "all"
check(["all", "key", "directories"].contains(aliasCase), "R07 valid alias fixture selector")
let aliasFixture = moveFixture.appendingPathComponent("r07-name-alias-\(UUID().uuidString)")
let aliasProbe = aliasFixture.appendingPathComponent("case-probe")
try FileManager.default.createDirectory(at: aliasProbe, withIntermediateDirectories: true)
let aliasKey = aliasProbe.appendingPathComponent("Wallet-Node.Key")
try Data("case-alias-probe".utf8).write(to: aliasKey)
let lookupKey = aliasProbe.appendingPathComponent("wallet-node.key")
let caseInsensitiveAliases = FileManager.default.fileExists(atPath: lookupKey.path)
    && BlockDataMove.identity(aliasKey, directory: false) == BlockDataMove.identity(lookupKey, directory: false)
func hasEndpointAlias(_ root: URL) -> Bool {
    guard let entries = FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil) else { return false }
    return entries.compactMap { $0 as? URL }.contains { $0.lastPathComponent.lowercased() == "wallet-node.key" }
}
if caseInsensitiveAliases {
    if aliasCase == "all" || aliasCase == "key" {
        let source = aliasFixture.appendingPathComponent("direct-source")
        let target = aliasFixture.appendingPathComponent("direct-external")
        try FileManager.default.createDirectory(at: source.appendingPathComponent("nested"), withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: target, withIntermediateDirectories: true)
        let state = source.appendingPathComponent("state.db")
        let key = source.appendingPathComponent("Wallet-Node.Key")
        let nestedKey = source.appendingPathComponent("nested/WALLET-NODE.KEY")
        try Data("case-alias-chain-state".utf8).write(to: state)
        try Data("private-alias-endpoint".utf8).write(to: key)
        try Data("nested-alias-endpoint".utf8).write(to: nestedKey)
        var exposureChecks: [Bool] = []
        let meter = DataMigration.ProgressMeter(reportEvery: 1) { _ in exposureChecks.append(hasEndpointAlias(target)) }
        let copied = DataMigration.syncTreeVerified(source, target, meter: meter, excluding: BlockDataLocation.keepInternal)
        check(copied && !hasEndpointAlias(target) && !exposureChecks.isEmpty && exposureChecks.allSatisfy { !$0 },
              "R07 alias endpoint variants never enter external verified copy")
        check((try? Data(contentsOf: target.appendingPathComponent("state.db"))) == Data("case-alias-chain-state".utf8),
              "R07 alias exclusion still copies actual chain data")
        check((try? Data(contentsOf: key)) == Data("private-alias-endpoint".utf8)
              && (try? Data(contentsOf: nestedKey)) == Data("nested-alias-endpoint".utf8),
              "R07 alias exclusion preserves exact source identity bytes")
    }
    if aliasCase == "all" || aliasCase == "directories" {
        let source = aliasFixture.appendingPathComponent("directory-source")
        let target = aliasFixture.appendingPathComponent("directory-external")
        let internalRoot = aliasFixture.appendingPathComponent("directory-owner")
        try FileManager.default.createDirectory(at: source.appendingPathComponent("Follow"), withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: internalRoot, withIntermediateDirectories: true)
        let state = source.appendingPathComponent("Follow/state.db")
        let key = source.appendingPathComponent("Follow/Wallet-Node.Key")
        try Data("real-follow-alias-history".utf8).write(to: state)
        try Data("internal-follow-alias-key".utf8).write(to: key)
        let sourceID = BlockDataMove.identity(source)!
        let bytes = try BlockDataMove.copy(source: source, target: target, sourceID: sourceID,
                                          internalRoot: internalRoot, preservingInternalKeys: false,
                                          meter: DataMigration.ProgressMeter(reportEvery: 1) { _ in })
        check(bytes == UInt64(Data("real-follow-alias-history".utf8).count)
              && (try? Data(contentsOf: target.appendingPathComponent("follow/state.db"))) == Data("real-follow-alias-history".utf8),
              "R03 alias Follow directory must copy actual history before publication")
        check(!hasEndpointAlias(target) && (try? Data(contentsOf: key)) == Data("internal-follow-alias-key".utf8),
              "R07 alias Follow copy preserves the internal endpoint and excludes it externally")
    }
} else {
    print("SKIP R07/R03 filename aliases: fixture filesystem is case-sensitive")
}

if caseInsensitiveAliases && (aliasCase == "all" || aliasCase == "key") {
    // Copy back beside an existing differently-cased internal endpoint key.
    let source = aliasFixture.appendingPathComponent("return-source")
    let target = aliasFixture.appendingPathComponent("return-internal")
    let internalRoot = aliasFixture.appendingPathComponent("return-owner")
    try FileManager.default.createDirectory(at: source.appendingPathComponent("follow"), withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: target.appendingPathComponent("Follow"), withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: internalRoot, withIntermediateDirectories: true)
    let sourceKey = source.appendingPathComponent("follow/Wallet-Node.Key")
    let targetKey = target.appendingPathComponent("Follow/Wallet-Node.Key")
    try Data("source-alias-identity".utf8).write(to: sourceKey)
    try Data("original-internal-alias-identity".utf8).write(to: targetKey)
    let sourceState = source.appendingPathComponent("follow/state.db")
    try Data("return-alias-history".utf8).write(to: sourceState)
    check(BlockDataLocation.destinationAvailable(target, preservingInternalKeys: true),
          "R07 internal alias-key-only destination remains available")
    _ = try BlockDataMove.copy(source: source, target: target, sourceID: BlockDataMove.identity(source)!,
                              internalRoot: internalRoot, preservingInternalKeys: true,
                              meter: DataMigration.ProgressMeter(reportEvery: 1) { _ in })
    check((try? Data(contentsOf: targetKey)) == Data("original-internal-alias-identity".utf8)
          && (try? Data(contentsOf: sourceKey)) == Data("source-alias-identity".utf8),
          "R07 return copy never replaces or removes either alias identity")
    check((try? Data(contentsOf: target.appendingPathComponent("follow/state.db"))) == Data("return-alias-history".utf8),
          "R07 return copy publishes data beside the original alias endpoint")
    let foreign = source.appendingPathComponent("follow/not-in-copy-manifest.db")
    try Data("foreign-new-source-history".utf8).write(to: foreign)
    BlockDataMove.cleanup(confirmedTarget: target, internalRoot: internalRoot)
    check((try? Data(contentsOf: sourceKey)) == Data("source-alias-identity".utf8)
          && (try? Data(contentsOf: targetKey)) == Data("original-internal-alias-identity".utf8)
          && (try? Data(contentsOf: foreign)) == Data("foreign-new-source-history".utf8),
          "R07 cleanup retains alias identities and unmanifested source data")
    // A record from the pre-fix writer cannot authorize alias-key cleanup.
    let recordURL = internalRoot.appendingPathComponent(BlockDataMove.recordName)
    let prior = try JSONDecoder().decode(BlockDataMove.Record.self, from: Data(contentsOf: recordURL))
    var unsafeFiles = prior.files
    unsafeFiles["follow/Wallet-Node.Key"] = BlockDataMove.File(identity: BlockDataMove.identity(sourceKey, directory: false)!,
                                                          hash: DataMigration.streamSHA256(sourceKey)!)
    var unsafe = BlockDataMove.Record(source: prior.source, target: prior.target, staging: prior.staging,
                                     stagingID: prior.stagingID, sourceID: prior.sourceID, targetID: prior.targetID,
                                     directories: prior.directories, files: unsafeFiles)
    unsafe.committed = true
    try JSONEncoder().encode(unsafe).write(to: recordURL, options: .atomic)
    var rejectedUnsafeRecord = false
    do { _ = try BlockDataMove.authoritativeRoot(in: internalRoot) } catch { rejectedUnsafeRecord = true }
    check(rejectedUnsafeRecord, "R07 old alias-key manifests are refused before replay or cleanup hashing")
    BlockDataMove.cleanup(confirmedTarget: target, internalRoot: internalRoot)
    check((try? Data(contentsOf: sourceKey)) == Data("source-alias-identity".utf8)
          && (try? Data(contentsOf: targetKey)) == Data("original-internal-alias-identity".utf8),
          "R07 invalid legacy alias manifest removes no key data")
}
if caseInsensitiveAliases && (aliasCase == "all" || aliasCase == "directories") {
    for name in ["Follow", "Archive"] {
        let source = aliasFixture.appendingPathComponent("broken-\(name)-source")
        let target = aliasFixture.appendingPathComponent("broken-\(name)-target")
        let internalRoot = aliasFixture.appendingPathComponent("broken-\(name)-owner")
        try FileManager.default.createDirectory(at: source, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: internalRoot, withIntermediateDirectories: true)
        let missing = aliasFixture.appendingPathComponent("not-present-\(name)")
        let link = source.appendingPathComponent(name)
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: missing)
        var rejected = false
        do {
            _ = try BlockDataMove.copy(source: source, target: target, sourceID: BlockDataMove.identity(source)!,
                                      internalRoot: internalRoot, preservingInternalKeys: false,
                                      meter: DataMigration.ProgressMeter(reportEvery: 1) { _ in })
        } catch { rejected = true }
        check(rejected && !FileManager.default.fileExists(atPath: target.path)
              && !FileManager.default.fileExists(atPath: internalRoot.appendingPathComponent(BlockDataMove.recordName).path),
              "R03 broken \(name) aliases cannot become successful empty copies")
        check((try? FileManager.default.destinationOfSymbolicLink(atPath: link.path)) == missing.path,
              "R03 refusing an alias namespace preserves its original link")
    }
    let source = aliasFixture.appendingPathComponent("file-follow-source")
    let target = aliasFixture.appendingPathComponent("file-follow-target")
    let internalRoot = aliasFixture.appendingPathComponent("file-follow-owner")
    try FileManager.default.createDirectory(at: source, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: internalRoot, withIntermediateDirectories: true)
    let occupant = source.appendingPathComponent("Follow")
    try Data("unrelated-follow-occupant".utf8).write(to: occupant)
    var rejected = false
    do {
        _ = try BlockDataMove.copy(source: source, target: target, sourceID: BlockDataMove.identity(source)!,
                                  internalRoot: internalRoot, preservingInternalKeys: false,
                                  meter: DataMigration.ProgressMeter(reportEvery: 1) { _ in })
    } catch { rejected = true }
    check(rejected && !FileManager.default.fileExists(atPath: target.path)
          && (try? Data(contentsOf: occupant)) == Data("unrelated-follow-occupant".utf8),
          "R03 a non-directory namespace is preserved and refused")
}

let nestedSource = moveFixture.appendingPathComponent("source")
try FileManager.default.createDirectory(at: nestedSource.appendingPathComponent("follow"), withIntermediateDirectories: true)
let mover = NodeController()
mover.chainDataPath = nestedSource.path
check(mover.problem(with: nestedSource.appendingPathComponent("follow")) != nil,
      "R01 nested destination is rejected before copy and cleanup")
let alias = moveFixture.appendingPathComponent("source-alias")
try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: nestedSource)
check(mover.problem(with: alias.appendingPathComponent("follow")) != nil, "R01 symlink aliases cannot bypass ancestry")
mover.moveBlockData(to: nestedSource.appendingPathComponent("follow/new"))
check(mover.storageMovePercent == nil && mover.stops == 0, "R01 direct move rejects nesting before stopping")
check(!BlockDataLocation.disjoint(nestedSource, nestedSource.deletingLastPathComponent()), "R01 ancestor is rejected")
check(BlockDataLocation.disjoint(nestedSource, moveFixture.appendingPathComponent("source-sibling")), "R01 siblings remain allowed")
let authoritative = nestedSource.appendingPathComponent("follow/new")
try FileManager.default.createDirectory(at: authoritative, withIntermediateDirectories: true)
let sentinel = authoritative.appendingPathComponent("state.db")
try Data("authoritative".utf8).write(to: sentinel)
mover.chainDataPath = authoritative.path
NodeController.storageMoveDefaults.set(nestedSource.path, forKey: NodeController.cleanupKey)
mover.finishBlockDataMove()
try await Task.sleep(nanoseconds: 100_000_000)
check(FileManager.default.fileExists(atPath: sentinel.path), "R01 cleanup rechecks ancestry")

func waitForMove(_ node: NodeController) async throws {
    let end = Date().addingTimeInterval(10)
    while node.storageMovePercent != nil && Date() < end { try await Task.sleep(nanoseconds: 10_000_000) }
    check(node.storageMovePercent == nil, "fixture move completes within deadline")
}
// R02: force a failed copy into somebody else's preexisting chain tree.
let badSource = moveFixture.appendingPathComponent("bad-source")
let occupied = moveFixture.appendingPathComponent("occupied")
try FileManager.default.createDirectory(at: badSource.appendingPathComponent("follow"), withIntermediateDirectories: true)
try FileManager.default.createDirectory(at: occupied.appendingPathComponent("follow"), withIntermediateDirectories: true)
try FileManager.default.createSymbolicLink(atPath: badSource.appendingPathComponent("follow/unreadable").path,
                                         withDestinationPath: moveFixture.appendingPathComponent("missing-file").path)
let unrelated = occupied.appendingPathComponent("follow/unrelated.db")
try Data("unrelated-history".utf8).write(to: unrelated)
let failedMover = NodeController()
failedMover.chainDataPath = badSource.path
failedMover.moveBlockData(to: occupied)
try await waitForMove(failedMover)
check((try? Data(contentsOf: unrelated)) == Data("unrelated-history".utf8), "R02 failed move preserves preexisting destination data")
check(failedMover.chainDataPath == badSource.path, "R02 rejected move keeps source authoritative")
let updatingMover = NodeController()
updatingMover.chainDataPath = badSource.path
updatingMover.updateInProgress = true
updatingMover.moveBlockData(to: moveFixture.appendingPathComponent("during-update"))
check(updatingMover.stops == 0 && updatingMover.storageMovePercent == nil,
      "R11 update preparation excludes a new storage move")
// R03: a disconnected source cannot turn into a successful empty move back.
try FileManager.default.createDirectory(at: NodeController.dataDir, withIntermediateDirectories: true)
let oldProbe = NodeController.dataDir.appendingPathComponent(".eastsea-write-check")
try Data("existing-file".utf8).write(to: oldProbe)
check(NodeController.canWrite(in: NodeController.dataDir) && (try? Data(contentsOf: oldProbe)) == Data("existing-file".utf8),
      "R03 disk writability probes preserve preexisting files")
let missingRoot = moveFixture.appendingPathComponent("offline-disk/data")
let missingMover = NodeController()
missingMover.chainDataPath = missingRoot.path
missingMover.moveBlockData(to: nil)
try await waitForMove(missingMover)
check(missingMover.chainDataPath == missingRoot.path && missingMover.storageMoveError != nil,
      "R03 absent source cannot commit an empty move or authorize cleanup")
let verifiedSource = moveFixture.appendingPathComponent("verified-source")
let verifiedTarget = moveFixture.appendingPathComponent("verified-target")
try FileManager.default.createDirectory(at: verifiedSource.appendingPathComponent("follow"), withIntermediateDirectories: true)
let copiedFile = verifiedSource.appendingPathComponent("follow/state.db")
try Data("verified-data".utf8).write(to: copiedFile)
try FileManager.default.createDirectory(at: verifiedSource.appendingPathComponent("archive"), withIntermediateDirectories: true)
let archiveFile = verifiedSource.appendingPathComponent("archive/history.db")
try Data("archive-history".utf8).write(to: archiveFile)
let goodMover = NodeController()
goodMover.chainDataPath = verifiedSource.path
goodMover.moveBlockData(to: verifiedTarget)
try await waitForMove(goodMover)
check(goodMover.chainDataPath == verifiedTarget.path, "R03 real verified copy commits")
check(try BlockDataMove.authoritativeRoot(in: NodeController.dataDir)?.path == verifiedTarget.path,
      "R03 committed location survives lost preference writes")
// Replay the verified publication with the preference still at the source.
let recordURL = NodeController.dataDir.appendingPathComponent(BlockDataMove.recordName)
var interrupted = try JSONDecoder().decode(BlockDataMove.Record.self, from: Data(contentsOf: recordURL))
interrupted.committed = false
try JSONEncoder().encode(interrupted).write(to: recordURL, options: .atomic)
let replacedStage = verifiedTarget.appendingPathComponent(interrupted.staging)
try FileManager.default.createDirectory(at: replacedStage, withIntermediateDirectories: true)
let foreignStageFile = replacedStage.appendingPathComponent("foreign.db")
try Data("not-created-by-move".utf8).write(to: foreignStageFile)
goodMover.chainDataPath = verifiedSource.path
goodMover.moveBlockData(to: verifiedTarget)
try await waitForMove(goodMover)
check(goodMover.chainDataPath == verifiedSource.path && (try? Data(contentsOf: foreignStageFile)) == Data("not-created-by-move".utf8),
      "R03 replaced staging directories never become rollback cargo")
try FileManager.default.removeItem(at: replacedStage)
goodMover.moveBlockData(to: verifiedTarget)
try await waitForMove(goodMover)
check(goodMover.chainDataPath == verifiedTarget.path, "R03 interrupted publication resumes without re-merging foreign data")
let oldTarget = moveFixture.appendingPathComponent("temporarily-away")
try FileManager.default.moveItem(at: verifiedTarget, to: oldTarget)
try FileManager.default.createDirectory(at: verifiedTarget, withIntermediateDirectories: true)
check(!BlockDataMove.selectionAvailable(verifiedTarget, internalRoot: NodeController.dataDir),
      "R03 a replacement root at the same path cannot start a fresh chain")
try FileManager.default.removeItem(at: verifiedTarget)
try FileManager.default.moveItem(at: oldTarget, to: verifiedTarget)
check(BlockDataMove.selectionAvailable(verifiedTarget, internalRoot: NodeController.dataDir), "R03 the original root resumes")
try FileManager.default.removeItem(at: verifiedTarget.appendingPathComponent("archive/history.db"))
let lateFile = verifiedSource.appendingPathComponent("follow/new-after-copy.db")
try Data("new-history".utf8).write(to: lateFile)
goodMover.finishBlockDataMove()
try await Task.sleep(nanoseconds: 150_000_000)
check(!FileManager.default.fileExists(atPath: copiedFile.path), "R03 verified source file can be cleaned")
check((try? Data(contentsOf: lateFile)) == Data("new-history".utf8), "R03 unmanifested source data survives cleanup")
check((try? Data(contentsOf: archiveFile)) == Data("archive-history".utf8), "R03 missing destination history retains its only source copy")
goodMover.finishBlockDataMove()
try await Task.sleep(nanoseconds: 50_000_000)
check(FileManager.default.fileExists(atPath: lateFile.path), "R03 cleanup replay is idempotent")
// A stale unpublished copy must not permanently exclude future safe moves.
let staleSource = moveFixture.appendingPathComponent("stale-source")
let staleTarget = moveFixture.appendingPathComponent("stale-target")
try FileManager.default.createDirectory(at: staleSource.appendingPathComponent("follow"), withIntermediateDirectories: true)
let staleDB = staleSource.appendingPathComponent("follow/state.db")
try Data("before-crash".utf8).write(to: staleDB)
let staleMover = NodeController()
staleMover.chainDataPath = staleSource.path
staleMover.moveBlockData(to: staleTarget)
try await waitForMove(staleMover)
var staleRecord = try JSONDecoder().decode(BlockDataMove.Record.self, from: Data(contentsOf: recordURL))
staleRecord.committed = false
try JSONEncoder().encode(staleRecord).write(to: recordURL, options: .atomic)
try Data("advanced-source".utf8).write(to: staleDB)
staleMover.chainDataPath = staleSource.path
let freshTarget = moveFixture.appendingPathComponent("fresh-target")
staleMover.moveBlockData(to: freshTarget)
try await waitForMove(staleMover)
check(staleMover.chainDataPath == freshTarget.path && (try? Data(contentsOf: freshTarget.appendingPathComponent("follow/state.db"))) == Data("advanced-source".utf8),
      "R03 an advanced source can safely abandon stale publication and move afresh")
check((try? Data(contentsOf: staleTarget.appendingPathComponent("follow/state.db"))) == Data("before-crash".utf8),
      "R03 abandoning a stale move retains its earlier copied cargo")

// R06. The actual mover pauses daemon respawn before either node stops and
// retains ownership during copy, selection publication and commit.
try? FileManager.default.removeItem(at: recordURL)
let fencedSource = moveFixture.appendingPathComponent("r06-source")
let fencedTarget = moveFixture.appendingPathComponent("r06-target")
try FileManager.default.createDirectory(at: fencedSource.appendingPathComponent("follow"), withIntermediateDirectories: true)
try Data(repeating: 0x31, count: 256 * 1024).write(to: fencedSource.appendingPathComponent("follow/state.db"))
let fencedMover = NodeController()
fencedMover.attached = true
fencedMover.chainDataPath = fencedSource.path
fencedMover.moveBlockData(to: fencedTarget)
check(fencedMover.markerAtStops == [false]
      && fencedMover.unattended?.markerAtDaemonStops == [false],
      "R06 daemon respawn is suspended before stopping either node")
try await waitForMove(fencedMover)
check(fencedMover.chainDataPath == fencedTarget.path, "R06 fenced copy commits its destination")
check(fencedMover.selectionLockChecks == [true],
      "R06 exclusive run.lock is retained through copy publication and selection commit")
check(!NodeController.lockHeld(in: NodeController.dataDir)
      && fencedMover.unattended?.resumeCalls == 1
      && fencedMover.unattended?.markerEnabled == true,
      "R06 ownership is released and daemon respawn resumes after commit")

// A live writer that does not exit must time out before any copying or
// publication; resuming the marker preserves the original source choice.
try? FileManager.default.removeItem(at: recordURL)
let timeoutSource = moveFixture.appendingPathComponent("r06-timeout-source")
let timeoutTarget = moveFixture.appendingPathComponent("r06-timeout-target")
try FileManager.default.createDirectory(at: timeoutSource.appendingPathComponent("follow"), withIntermediateDirectories: true)
let timeoutDB = timeoutSource.appendingPathComponent("follow/state.db")
try Data("writer-owned".utf8).write(to: timeoutDB)
let busyFD = open(NodeController.dataDir.appendingPathComponent("run.lock").path, O_RDWR | O_CREAT, 0o600)
check(busyFD >= 0 && flock(busyFD, LOCK_EX | LOCK_NB) == 0, "R06 fixture owns the live writer lock")
let timeoutMover = NodeController()
timeoutMover.attached = true
timeoutMover.chainDataPath = timeoutSource.path
timeoutMover.moveBlockData(to: timeoutTarget)
try await waitForMove(timeoutMover)
check(timeoutMover.chainDataPath == timeoutSource.path && timeoutMover.storageMoveError != nil,
      "R06 lock timeout never switches source selection")
check(!FileManager.default.fileExists(atPath: timeoutTarget.path)
      && !FileManager.default.fileExists(atPath: recordURL.path)
      && (try? Data(contentsOf: timeoutDB)) == Data("writer-owned".utf8),
      "R06 lock timeout creates no copy or cleanup authorization")
check(NodeController.lockHeld(in: NodeController.dataDir)
      && timeoutMover.unattended?.resumeCalls == 1
      && timeoutMover.unattended?.markerEnabled == true,
      "R06 timeout keeps the writer lock and resumes the original daemon choice")
close(busyFD)

// A copy failure still balances suspension and releases only our descriptor.
let failureSource = moveFixture.appendingPathComponent("r06-failure-source")
let failureTarget = moveFixture.appendingPathComponent("r06-failure-target")
try FileManager.default.createDirectory(at: failureSource.appendingPathComponent("follow"), withIntermediateDirectories: true)
try FileManager.default.createSymbolicLink(at: failureSource.appendingPathComponent("follow/unreadable"),
                                         withDestinationURL: moveFixture.appendingPathComponent("r06-missing-file"))
let copyFailureMover = NodeController()
copyFailureMover.chainDataPath = failureSource.path
copyFailureMover.moveBlockData(to: failureTarget)
try await waitForMove(copyFailureMover)
check(copyFailureMover.chainDataPath == failureSource.path && copyFailureMover.storageMoveError != nil
      && copyFailureMover.unattended?.resumeCalls == 1
      && !NodeController.lockHeld(in: NodeController.dataDir),
      "R06 failed copy preserves source and resumes daemon after releasing ownership")

// If the marker cannot be durably removed, stop nothing and copy nothing.
let pauseFailureMover = NodeController()
pauseFailureMover.chainDataPath = timeoutSource.path
pauseFailureMover.unattended?.suspensionSucceeds = false
pauseFailureMover.moveBlockData(to: moveFixture.appendingPathComponent("r06-pause-failure-target"))
try await waitForMove(pauseFailureMover)
check(pauseFailureMover.stops == 0 && pauseFailureMover.storageMoveError != nil
      && pauseFailureMover.chainDataPath == timeoutSource.path
      && pauseFailureMover.unattended?.resumeCalls == 1,
      "R06 failed marker suspension leaves the running source untouched")


// The descriptor used by the R11 installer must close on exec.
let execFD = await BlockDataMove.holdRunLock(in: NodeController.dataDir, timeout: 0.05)
check(execFD != nil, "R06 shared lock helper returns owned descriptor")
if let fd = execFD {
    check((fcntl(fd, F_GETFD) & FD_CLOEXEC) != 0, "R06 installer children cannot inherit the move lock")
    close(fd)
}


// R07. A round trip back into the default key-only follow directory must
// preserve the exact internal endpoint key, never regenerate its identity.
try? FileManager.default.removeItem(at: recordURL)
let internalFollow = NodeController.dataDir.appendingPathComponent("follow")
try FileManager.default.createDirectory(at: internalFollow, withIntermediateDirectories: true)
let internalKey = internalFollow.appendingPathComponent("wallet-node.key")
let internalState = internalFollow.appendingPathComponent("state.db")
let keyBytes = Data("internal-endpoint-key".utf8)
try keyBytes.write(to: internalKey)
try Data("round-trip-state".utf8).write(to: internalState)
let roundTripTarget = moveFixture.appendingPathComponent("r07-round-trip-target")
let roundTripMover = NodeController()
roundTripMover.moveBlockData(to: roundTripTarget)
try await waitForMove(roundTripMover)
check(roundTripMover.chainDataPath == roundTripTarget.path
      && (try? Data(contentsOf: internalKey)) == keyBytes,
      "R07 outward move leaves the endpoint identity on the internal disk")
roundTripMover.finishBlockDataMove()
let cleanupEnd = Date().addingTimeInterval(5)
while FileManager.default.fileExists(atPath: internalState.path) && Date() < cleanupEnd {
    try await Task.sleep(nanoseconds: 10_000_000)
}
check(!FileManager.default.fileExists(atPath: internalState.path),
      "R07 fixture cleanup leaves the default follow directory key-only")
roundTripMover.moveBlockData(to: nil)
try await waitForMove(roundTripMover)
check(roundTripMover.chainDataPath.isEmpty && (try? Data(contentsOf: internalKey)) == keyBytes,
      "R07 round trip preserves the preexisting internal endpoint key")
check((try? Data(contentsOf: internalState)) == Data("round-trip-state".utf8),
      "R07 round trip returns block data beside the original endpoint key")

// Inspect private staging on synchronous progress, then force source
// verification to fail. A key must never be copied even temporarily.
try? FileManager.default.removeItem(at: recordURL)
let keySource = moveFixture.appendingPathComponent("r07-key-source")
let keyTarget = moveFixture.appendingPathComponent("r07-key-target")
let keyFollow = keySource.appendingPathComponent("follow")
try FileManager.default.createDirectory(at: keyFollow.appendingPathComponent("nested"), withIntermediateDirectories: true)
let keyDB = keyFollow.appendingPathComponent("state.db")
try Data("verified-before-fault".utf8).write(to: keyDB)
try Data("private-endpoint-key".utf8).write(to: keyFollow.appendingPathComponent("wallet-node.key"))
try Data("nested-private-key".utf8).write(to: keyFollow.appendingPathComponent("nested/wallet-node.key"))
func containsEndpointKey(_ root: URL) -> Bool {
    guard let entries = FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil) else { return false }
    return entries.compactMap { $0 as? URL }.contains { $0.lastPathComponent == "wallet-node.key" }
}
var progressHasNoKeys: [Bool] = []
var faultInjected = false
let keyMeter = DataMigration.ProgressMeter(reportEvery: 1) { fraction in
    progressHasNoKeys.append(!containsEndpointKey(keyTarget))
    if fraction > 0, !faultInjected {
        faultInjected = true
        try? Data("changed-after-snapshot".utf8).write(to: keyDB, options: .atomic)
    }
}
let keySourceID = BlockDataMove.identity(keySource)!
var copyFailed = false
do {
    _ = try BlockDataMove.copy(source: keySource, target: keyTarget, sourceID: keySourceID,
                              internalRoot: NodeController.dataDir, preservingInternalKeys: false, meter: keyMeter)
} catch { copyFailed = true }
check(faultInjected && copyFailed, "R07 fixture fails after staging at the verification boundary")
check(!progressHasNoKeys.isEmpty && progressHasNoKeys.allSatisfy { $0 } && !containsEndpointKey(keyTarget),
      "R07 progress and failed staging never expose an endpoint key on the external destination")
check((try? Data(contentsOf: keyFollow.appendingPathComponent("wallet-node.key"))) == Data("private-endpoint-key".utf8),
      "R07 failed copy retains its source endpoint key")

}
#endif

// MARK: archive requirements, on measured numbers

check(ArchiveRequirements.bytesPerBlock > 0 && ArchiveRequirements.bytesPerDay > 0, "measured on the archive node, not zero")
let req = ArchiveRequirements(height: 500_000)
check(req.sizeNowBytes == UInt64(500_000 * ArchiveRequirements.bytesPerBlock), "size scales with the height")
check(req.recommendedFreeBytes == req.sizeNowBytes + req.perYearBytes + 7 * GiB, "now + a year + the resume margin")
for ko in [true, false] {
    let lines = req.lines(ko: ko)
    check(lines.count == 4, "size, disk, first sync, no reward")
    check(lines[0].contains(NodeStopReason.gb(req.sizeNowBytes)) && lines[0].contains(NodeStopReason.gb(req.perMonthBytes)), "real numbers: \(lines[0])")
    check(lines[1].contains(NodeStopReason.gb(req.recommendedFreeBytes)), "the recommended disk size")
    let all = lines.joined(separator: " ")
    for promise in ["earn", "벌", "수익", "APY", "%"] { check(!all.contains(promise), "no reward promise (\(promise)) ko=\(ko)") }
}
check(req.lines(ko: true)[3].contains("보상은 없어요"), "says plainly there is no reward")
check(req.lines(ko: true)[2].contains("몇 시간에서 며칠"), "hours to days")
#if os(macOS)
Task { @MainActor in
    do { try await runMoveFixtureTests(); print("OK block-data"); exit(0) }
    catch { print("FAIL fixture:", error); exit(1) }
}
dispatchMain()
#else
print("OK block-data")
#endif
