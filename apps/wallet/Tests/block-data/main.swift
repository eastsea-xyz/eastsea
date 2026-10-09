// 블록 데이터 위치 and 전체 기록 보관 (BlockDataLocation.swift): which disks
// may hold the block data, the node flags each choice makes, and the archive
// requirements on the archive node's real numbers.
//   scripts/test-swift-pure.sh   (run block-data)
import Foundation
#if os(macOS)
import AppKit

// Legacy cleanup preferences stay in memory; fixtures never write cfprefsd.
final class MoveMemoryDefaults: UserDefaults, @unchecked Sendable {
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

final class MoveProcessStub {
    let processIdentifier: Int32
    init(_ pid: Int32) { processIdentifier = pid }
}

// Exercise the real mover against fixtures, without starting a node or app.
@MainActor final class NodeController {
    nonisolated static let storageMoveLockTimeout: TimeInterval = 0.05
    nonisolated static let port: UInt16 = 1
    nonisolated static let helperBinaryURL: URL? = URL(fileURLWithPath: "/fixture/aether")
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
    var storageMovePreparing = false
    var storageMoveSync: BlockDataMove.Progress?
    var storageMoveCleanup: Task<Void, Never>?
    var storageMoveError: String?
    var attached = false
    var process: MoveProcessStub?
    var storageLeaseVerified = true
    var storageBindingMatches = true
    var storageEndpointAbsent = true
    var storagePreflightCalls = 0
    var onStorageStatus: (() -> Void)?
    var onStorageAbsence: (() -> Void)?
    var unattended: MoveDaemonStub? = MoveDaemonStub()
    var archive = false
    var mountObservers: [NSObjectProtocol] = []
    var stops = 0
    var markerAtStops: [Bool] = []
    init() { LocalRPC.node = self }
    func stop(keepSwitch: Bool) {
        markerAtStops.append(unattended?.markerEnabled ?? false)
        stops += 1
        process = nil
        attached = false
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

// Only RPC and signature observations are doubled. The fixture compiles the
// production ownership method, including its checks across each suspension.
@MainActor enum LocalRPC {
    static weak var node: NodeController?
    struct VerifiedReply { let value: Any; let binding: Int32 }
    static func callVerified(rootPID: Int32, port: UInt16, expected: URL,
                             method: String, params: [Any]) async -> VerifiedReply? {
        guard let node else { return nil }
        node.storagePreflightCalls += 1
        check(port == NodeController.port && expected == NodeController.helperBinaryURL
              && method == "aether_status" && params.isEmpty, "R06 fresh status preflight uses the current helper")
        let reply = VerifiedReply(value: node.storageLeaseVerified, binding: rootPID)
        await Task.yield()
        node.onStorageStatus?()
        return reply
    }
    static func endpointIsAbsent(port: UInt16) async -> Bool {
        guard let node else { return false }
        node.storagePreflightCalls += 1
        let absent = node.storageEndpointAbsent
        await Task.yield()
        node.onStorageAbsence?()
        return absent
    }
}
@MainActor enum NodeReleaseIdentity {
    static func hasWriterLease(status: Any?) -> Bool { status as? Bool == true }
    static func matches(binding: Int32, port: UInt16, expected: URL) -> Bool {
        LocalRPC.node?.storageBindingMatches == true
    }
}
@MainActor final class MoveDaemonStub {
    var markerEnabled = true
    var runningNodePID: Int32?
    var onStop: (() -> Void)?
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
    func stopDaemonNode(expectedPID: Int32? = nil) {
        if let expectedPID, expectedPID != runningNodePID { return }
        markerAtDaemonStops.append(markerEnabled)
        onStop?()
        runningNodePID = nil
    }
    func syncMarker() { markerEnabled = suspensions == 0 }
}
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
check(BlockDataLocation.sentence(.unsupportedFormat("exfat"), locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))
      == "이 디스크는 블록 저장에 안전하지 않은 형식(exFAT)이에요. 디스크 유틸리티에서 APFS로 지우면 쓸 수 있어요 (디스크 안의 파일은 지워져요).",
      "exFAT: why, and how to fix it in Disk Utility")
check(BlockDataLocation.sentence(.unsupportedFormat("exfat"), locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
      == "This disk's format (exFAT) is not safe for block data. Erase it as APFS in Disk Utility to use it (this deletes the files on the disk).",
      "exFAT in English")
check(BlockDataLocation.Problem.unsupportedFormat("msdos").fixInDiskUtility && !BlockDataLocation.Problem.readOnly.fixInDiskUtility,
      "only a format problem offers Disk Utility")
check(BlockDataLocation.sentence(.notEnoughSpace(freeBytes: 26 * GiB, neededBytes: 27 * GiB), locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == "Not enough space: 26.0 GB free, 27.0 GB needed.",
      "exact numbers")
for p in [BlockDataLocation.Problem.networkShare, .unsupportedFormat("ntfs"), .readOnly, .notEnoughSpace(freeBytes: 1, neededBytes: 2), .inUse] {
    for language in ["en", "ko", "ja", "zh-Hans", "zh-Hant"] { check(!BlockDataLocation.sentence(p, locale: walletTestLocale(language), bundle: walletTestBundle(language)).isEmpty, "\(p) has words in \(language)") }
}

for language in ["en", "ko", "ja", "zh-Hans", "zh-Hant"] {
    let reason = NodeStopReason.startingStorage(height: 32, target: 100)
    let text = reason.copy(locale: walletTestLocale(language), bundle: walletTestBundle(language)).title
    check(text.contains("32") && text.contains("100") && !text.contains("%"),
          "height sync replaces copy percentages in \(language): \(text)")
    check(!reason.isIncident, "a fresh sync is an expected state")
    let ordinary = BlockDataLocation.confirmation(archive: false, locale: walletTestLocale(language), bundle: walletTestBundle(language))
    let archive = BlockDataLocation.confirmation(archive: true, locale: walletTestLocale(language), bundle: walletTestBundle(language))
    check(!ordinary.isEmpty && archive.hasPrefix(ordinary) && archive.count > ordinary.count,
          "archive confirmation adds its history rebuilding explanation in \(language)")
}
check(BlockDataLocation.validateFresh(vol("apfs", free: BlockDataLocation.freshFootprintBytes)) == nil,
      "the fresh footprint fits without the old database's allocation")
check(BlockDataLocation.validateFresh(vol("apfs", free: BlockDataLocation.freshFootprintBytes - 1))
      == .notEnoughSpace(freeBytes: BlockDataLocation.freshFootprintBytes - 1, neededBytes: BlockDataLocation.freshFootprintBytes),
      "move back to default also requires the fresh footprint")
for language in ["en", "ko", "ja", "zh-Hans", "zh-Hant"] {
    let refusal = BlockDataLocation.sentence(.notEnoughSpace(freeBytes: 3 * GiB, neededBytes: BlockDataLocation.freshFootprintBytes),
                                            returningToDefault: true, locale: walletTestLocale(language), bundle: walletTestBundle(language))
    check(refusal.contains("3.0 GB") && refusal.contains("8.0 GB"), "return refusal gives the fresh footprint in \(language)")
}

// The former language branches resolve from the same catalog in Japanese.
let japaneseProblems: [(BlockDataLocation.Problem, String)] = [
    (.networkShare, "ネットワークの共有フォルダにはブロックデータを保存できません。このMacに直接接続したディスクを選んでください。"),
    (.readOnly, "このディスクは読み取り専用です。書き込めるディスクを選んでください。"),
    (.notEnoughSpace(freeBytes: 26 * GiB, neededBytes: 27 * GiB), "空き容量が足りません。空きは26.0 GB、必要な容量は27.0 GBです。"),
    (.inUse, "ブロックデータはすでにこの場所にあります。"),
]
for (problem, expected) in japaneseProblems {
    check(BlockDataLocation.sentence(problem, locale: walletTestLocale("ja"), bundle: walletTestBundle("ja")) == expected,
          "Japanese block-data sentence: \(problem)")
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
@MainActor func runMoveFixtureTests() async throws {
let fm = FileManager.default
let moveFixture = NodeController.dataDir.deletingLastPathComponent()
    .appendingPathComponent("block-data-move-\(UUID().uuidString)")
try fm.createDirectory(at: moveFixture, withIntermediateDirectories: true)
try fm.createDirectory(at: NodeController.dataDir, withIntermediateDirectories: true)
defer { try? fm.removeItem(at: moveFixture); try? fm.removeItem(at: NodeController.dataDir) }
let recordURL = NodeController.dataDir.appendingPathComponent(BlockDataMove.recordName)

func waitForMove(_ node: NodeController) async throws {
    let deadline = ProcessInfo.processInfo.systemUptime + 30
    while node.storageMovePreparing {
        guard ProcessInfo.processInfo.systemUptime < deadline else {
            throw NSError(domain: "BlockDataFixture", code: 1, userInfo: [NSLocalizedDescriptionKey:
                "timed out after 30s waiting for fresh-start preparation (source=\(node.chainDataPath))"])
        }
        try await Task.sleep(nanoseconds: 10_000_000)
    }
}
func resetJournal() { try? fm.removeItem(at: recordURL) }
func prepare(_ tag: String) throws -> (URL, URL) {
    resetJournal()
    let source = moveFixture.appendingPathComponent("\(tag)-source")
    let target = moveFixture.appendingPathComponent("\(tag)-target")
    try fm.createDirectory(at: source.appendingPathComponent("follow"), withIntermediateDirectories: true)
    try Data("old block data".utf8).write(to: source.appendingPathComponent("follow/state.redb"))
    try BlockDataMove.prepare(source: source, target: target, sourceID: BlockDataMove.identity(source)!,
                              internalRoot: NodeController.dataDir, preservingInternalKeys: false)
    return (source, target)
}

// This is the mutation target: a later height/certificate with no response
// must never authorize deleting the source. The guard-removal test uses this
// unchanged fixture against a production-source mutant under tmp.
let (guardSource, guardTarget) = try prepare("delete-only-after-answer")
let guardDB = guardSource.appendingPathComponent("follow/state.redb")
check((try? fm.contentsOfDirectory(atPath: guardTarget.path)) == [],
      "a location change creates an empty destination, with no copied database")
check(try BlockDataMove.authoritativeRoot(in: NodeController.dataDir)?.path == guardTarget.path,
      "the fresh destination is durable before any startup response")
check(!BlockDataMove.cleanup(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir)
      && fm.fileExists(atPath: guardDB.path), "startup alone never deletes old data")
check(try !BlockDataMove.observe(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir,
                                answered: true, height: 10, certifiedHeight: 10), "first answer is only a baseline")
_ = try BlockDataMove.observe(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir,
                             answered: false, height: 11, certifiedHeight: 11)
_ = BlockDataMove.cleanup(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir)
check(fm.fileExists(atPath: guardDB.path), "delete-only-after-answer: no answer must retain old data")
_ = try BlockDataMove.observe(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir,
                             answered: true, height: 10, certifiedHeight: 10)
check(!BlockDataMove.cleanup(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir),
      "a repeated answer at the baseline has not followed a block")
_ = try BlockDataMove.observe(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir,
                             answered: true, height: 11, certifiedHeight: nil)
check(!BlockDataMove.cleanup(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir),
      "an increased snapshot height without a local certificate cannot delete old data")
_ = try BlockDataMove.observe(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir,
                             answered: true, height: 11, certifiedHeight: 12)
check(!BlockDataMove.cleanup(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir),
      "a certificate for a different height cannot confirm the new node")
check(try BlockDataMove.observe(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir,
                               answered: true, height: 11, certifiedHeight: 11),
      "the new node answered and then followed a certified block")
try fm.createDirectory(at: guardTarget.appendingPathComponent("follow"), withIntermediateDirectories: true)
let newDB = guardTarget.appendingPathComponent("follow/state.redb")
try Data("fresh synced block data".utf8).write(to: newDB)
check(BlockDataMove.cleanup(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir)
      && !fm.fileExists(atPath: guardDB.path), "only now does the old block data go")
check((try? Data(contentsOf: newDB)) == Data("fresh synced block data".utf8),
      "cleanup never touches the new synced state")
check(BlockDataMove.cleanup(confirmedTarget: guardTarget, internalRoot: NodeController.dataDir),
      "cleanup is idempotent and its completion is durable")

// A restarted wallet has only the durable journal, not an in-memory flag.
let (_, restoredTarget) = try prepare("restore")
check(try BlockDataMove.pending(in: NodeController.dataDir), "a saved move restores its sync state")
_ = try BlockDataMove.observe(confirmedTarget: restoredTarget, internalRoot: NodeController.dataDir,
                             answered: true, height: 20, certifiedHeight: nil)
check(try BlockDataMove.observe(confirmedTarget: restoredTarget, internalRoot: NodeController.dataDir,
                               answered: true, height: 21, certifiedHeight: 21),
      "a later process can resume confirmation from the durable first response")

// Eject during sync: no cleanup, no fallback onto a replacement directory.
// Reconnecting the same disk resumes the existing node gate.
let (ejectSource, ejectTarget) = try prepare("eject")
let away = moveFixture.appendingPathComponent("disk-away")
try fm.moveItem(at: ejectTarget, to: away)
check(!BlockDataMove.selectionAvailable(ejectTarget, internalRoot: NodeController.dataDir),
      "ejected selected disk cannot start on the internal fallback path")
check(!BlockDataMove.cleanup(confirmedTarget: ejectTarget, internalRoot: NodeController.dataDir)
      && fm.fileExists(atPath: ejectSource.appendingPathComponent("follow/state.redb").path),
      "eject during sync retains old data")
try fm.createDirectory(at: ejectTarget, withIntermediateDirectories: true)
check(!BlockDataMove.selectionAvailable(ejectTarget, internalRoot: NodeController.dataDir),
      "a different disk/directory at the selected path is refused")
try fm.removeItem(at: ejectTarget)
try fm.moveItem(at: away, to: ejectTarget)
check(BlockDataMove.selectionAvailable(ejectTarget, internalRoot: NodeController.dataDir),
      "the same destination resumes when reconnected")
var resume = NodeResumeFacts()
resume.storage = .chosen(volume: "Fixture", mounted: false, writable: false)
resume.onlyOnPower = false
check(NodeResume.decide(resume) == .wait(.diskMissing(volume: "Fixture")), "eject during sync stops the node")
resume.storage = .chosen(volume: "Fixture", mounted: true, writable: true)
check(NodeResume.decide(resume) == .start(detach: false), "mount resumes the node rather than moving it back")

// Replacement source namespaces cannot turn unrelated data into cleanup cargo.
let (replacedSource, replacedTarget) = try prepare("source-replaced")
let originalFollow = moveFixture.appendingPathComponent("original-follow")
try fm.moveItem(at: replacedSource.appendingPathComponent("follow"), to: originalFollow)
try fm.createDirectory(at: replacedSource.appendingPathComponent("follow"), withIntermediateDirectories: true)
let unrelated = replacedSource.appendingPathComponent("follow/unrelated.db")
try Data("unrelated replacement".utf8).write(to: unrelated)
_ = try BlockDataMove.observe(confirmedTarget: replacedTarget, internalRoot: NodeController.dataDir,
                             answered: true, height: 40, certifiedHeight: nil)
_ = try BlockDataMove.observe(confirmedTarget: replacedTarget, internalRoot: NodeController.dataDir,
                             answered: true, height: 41, certifiedHeight: 41)
check(!BlockDataMove.cleanup(confirmedTarget: replacedTarget, internalRoot: NodeController.dataDir)
      && (try? Data(contentsOf: unrelated)) == Data("unrelated replacement".utf8),
      "only a pinned old block-data namespace can be deleted")

// Validation retains nesting, occupancy, source availability and update gates.
resetJournal()
let nestedSource = moveFixture.appendingPathComponent("nested-source")
try fm.createDirectory(at: nestedSource.appendingPathComponent("follow"), withIntermediateDirectories: true)
let mover = NodeController()
mover.chainDataPath = nestedSource.path
mover.moveBlockData(to: nestedSource.appendingPathComponent("follow/new"))
check(!mover.storageMovePreparing && mover.stops == 0, "nested destination is refused before shutdown")
let alias = moveFixture.appendingPathComponent("source-alias")
try fm.createSymbolicLink(at: alias, withDestinationURL: nestedSource)
check(mover.problem(with: alias.appendingPathComponent("follow")) != nil, "symlink aliases cannot bypass ancestry")
let occupied = moveFixture.appendingPathComponent("occupied")
try fm.createDirectory(at: occupied.appendingPathComponent("follow"), withIntermediateDirectories: true)
try Data("unrelated history".utf8).write(to: occupied.appendingPathComponent("follow/state.redb"))
mover.moveBlockData(to: occupied)
check(mover.stops == 0 && mover.chainDataPath == nestedSource.path, "existing destination history is never reused or overwritten")
let missingMover = NodeController()
missingMover.chainDataPath = moveFixture.appendingPathComponent("missing-source").path
missingMover.moveBlockData(to: nil)
check(!missingMover.storageMovePreparing && missingMover.storageMoveError != nil, "an absent source remains protected")
let updatingMover = NodeController()
updatingMover.chainDataPath = nestedSource.path
updatingMover.updateInProgress = true
updatingMover.moveBlockData(to: moveFixture.appendingPathComponent("during-update"))
check(updatingMover.stops == 0 && !updatingMover.storageMovePreparing, "update preparation excludes a new storage change")

// R06 legacy writer: the parent's stop releases its real lock while an
// unleased child remains a writer. Attestation must precede even that stop.
try? FileManager.default.removeItem(at: recordURL)
let legacySource = moveFixture.appendingPathComponent("r06-legacy-source")
let legacyTarget = moveFixture.appendingPathComponent("r06-legacy-target")
try FileManager.default.createDirectory(at: legacySource.appendingPathComponent("follow"), withIntermediateDirectories: true)
let legacyDB = legacySource.appendingPathComponent("follow/state.db")
try Data("legacy-writer-data".utf8).write(to: legacyDB)
var legacyParentFD: Int32? = open(NodeController.dataDir.appendingPathComponent("run.lock").path, O_RDWR | O_CREAT, 0o600)
check(legacyParentFD != nil && legacyParentFD! >= 0 && flock(legacyParentFD!, LOCK_EX | LOCK_NB) == 0,
      "R06 legacy fixture holds the parent's lock")
let legacyMover = NodeController()
legacyMover.attached = true
legacyMover.chainDataPath = legacySource.path
legacyMover.storageLeaseVerified = false
legacyMover.unattended?.runningNodePID = 54
legacyMover.unattended?.onStop = {
    if let fd = legacyParentFD { close(fd); legacyParentFD = nil }
    // A valid snapshot may still verify even though this child survives.
    try? Data("legacy-child-survives-parent".utf8).write(to: legacyDB)
}
legacyMover.moveBlockData(to: legacyTarget)
try await waitForMove(legacyMover)
check(legacyMover.stops == 0 && legacyMover.unattended?.markerAtDaemonStops.isEmpty == true,
      "R06 unleased daemon is never stopped before live storage preflight")
check(legacyMover.chainDataPath == legacySource.path && legacyMover.storageMoveError != nil
      && !FileManager.default.fileExists(atPath: legacyTarget.path)
      && !FileManager.default.fileExists(atPath: recordURL.path),
      "R06 legacy writer cannot publish a fresh location or cleanup authorization")
if let fd = legacyParentFD { close(fd); legacyParentFD = nil }

// Previous/rollback own process and a replaced attested listener also defer.
for (tag, lease, binding) in [("previous", false, true), ("listener-replaced", true, false)] {
    let candidateSource = moveFixture.appendingPathComponent("r06-\(tag)-source")
    let candidateTarget = moveFixture.appendingPathComponent("r06-\(tag)-target")
    try FileManager.default.createDirectory(at: candidateSource.appendingPathComponent("follow"), withIntermediateDirectories: true)
    try Data("owned-history".utf8).write(to: candidateSource.appendingPathComponent("follow/state.db"))
    let candidateMover = NodeController()
    candidateMover.process = MoveProcessStub(55)
    candidateMover.storageLeaseVerified = lease
    candidateMover.storageBindingMatches = binding
    candidateMover.chainDataPath = candidateSource.path
    candidateMover.moveBlockData(to: candidateTarget)
    try await waitForMove(candidateMover)
    check(candidateMover.stops == 0 && candidateMover.chainDataPath == candidateSource.path
          && candidateMover.storageMoveError != nil && !FileManager.default.fileExists(atPath: candidateTarget.path),
          "R06 \(tag) writer defers without stopping or preparing")
}
let unknownMover = NodeController()
unknownMover.chainDataPath = legacySource.path
unknownMover.storageEndpointAbsent = false
unknownMover.moveBlockData(to: moveFixture.appendingPathComponent("r06-unknown-target"))
try await waitForMove(unknownMover)
check(unknownMover.stops == 0 && unknownMover.chainDataPath == legacySource.path && unknownMover.storageMoveError != nil,
      "R06 unclaimed live runtime defers before any stop or preparation")

// No claimed PID and a refused endpoint must still defer on an unknown
// held lock; waiting for its parent to exit would not prove child quiescence.
let unknownHolderFD = open(NodeController.dataDir.appendingPathComponent("run.lock").path, O_RDWR | O_CREAT, 0o600)
check(unknownHolderFD >= 0 && flock(unknownHolderFD, LOCK_EX | LOCK_NB) == 0,
      "R06 unknown holder fixture owns the lock without claiming a PID")
let unknownHolder = NodeController()
unknownHolder.chainDataPath = legacySource.path
unknownHolder.storageEndpointAbsent = true
let unknownHolderTarget = moveFixture.appendingPathComponent("r06-unknown-held-lock-target")
unknownHolder.moveBlockData(to: unknownHolderTarget)
try await waitForMove(unknownHolder)
check(unknownHolder.stops == 0 && unknownHolder.chainDataPath == legacySource.path
      && unknownHolder.storageMoveError != nil && !FileManager.default.fileExists(atPath: unknownHolderTarget.path)
      && NodeController.lockHeld(in: NodeController.dataDir),
      "R06 unknown holder is never stopped or awaited before fresh-start preparation")
close(unknownHolderFD)

// Exercise the production preflight across its RPC suspension, rather than
// trusting a fixture implementation of the ownership decision.
let statusChanges: [(String, (NodeController) -> Void)] = [
    ("own PID", { $0.process = MoveProcessStub(56) }),
    ("daemon PID", { $0.unattended?.runningNodePID = 56 }),
    ("attachment", { $0.attached = true }),
    ("update gate", { $0.updateInProgress = true }),
    ("move gate", { $0.storageMovePreparing = false }),
    ("listener binding", { $0.storageBindingMatches = false }),
]
for (tag, change) in statusChanges {
    let node = NodeController()
    node.process = MoveProcessStub(55)
    node.storageMovePreparing = true
    node.onStorageStatus = { change(node) }
    let ownership = await node.acquireStorageMoveOwnership()
    if let ownership { close(ownership) }
    check(ownership == nil && node.stops == 0 && node.storagePreflightCalls == 1,
          "R06 changed \(tag) after status cannot authorize source shutdown")
    node.onStorageStatus = nil
}
let absenceChanges: [(String, (NodeController) -> Void)] = [
    ("own PID", { $0.process = MoveProcessStub(57) }),
    ("daemon PID", { $0.unattended?.runningNodePID = 57 }),
    ("attachment", { $0.attached = true }),
    ("update gate", { $0.updateInProgress = true }),
    ("move gate", { $0.storageMovePreparing = false }),
]
for (tag, change) in absenceChanges {
    let node = NodeController()
    node.storageMovePreparing = true
    node.onStorageAbsence = { change(node) }
    let ownership = await node.acquireStorageMoveOwnership()
    if let ownership { close(ownership) }
    check(ownership == nil && node.stops == 0 && node.storagePreflightCalls == 1,
          "R06 changed \(tag) after endpoint absence cannot authorize ownership")
    node.onStorageAbsence = nil
}
let conflictingParents = NodeController()
conflictingParents.storageMovePreparing = true
conflictingParents.process = MoveProcessStub(58)
conflictingParents.unattended?.runningNodePID = 59
check(await conflictingParents.acquireStorageMoveOwnership() == nil
      && conflictingParents.stops == 0 && conflictingParents.storagePreflightCalls == 0,
      "R06 conflicting parents are refused before RPC or shutdown")
let unclaimedAttachment = NodeController()
unclaimedAttachment.storageMovePreparing = true
unclaimedAttachment.attached = true
check(await unclaimedAttachment.acquireStorageMoveOwnership() == nil
      && unclaimedAttachment.stops == 0 && unclaimedAttachment.storagePreflightCalls == 0,
      "R06 an unclaimed attachment cannot be treated as endpoint absence")

let cancelledPreflight = NodeController()
cancelledPreflight.storageMovePreparing = true
cancelledPreflight.process = MoveProcessStub(60)
var ownershipTask: Task<Int32?, Never>?
cancelledPreflight.onStorageStatus = { ownershipTask?.cancel() }
ownershipTask = Task { await cancelledPreflight.acquireStorageMoveOwnership() }
let cancelledOwnership = await ownershipTask!.value
if let cancelledOwnership { close(cancelledOwnership) }
check(cancelledOwnership == nil && cancelledPreflight.stops == 0
      && cancelledPreflight.storagePreflightCalls == 1,
      "R06 cancellation after the status read cannot authorize shutdown")
cancelledPreflight.onStorageStatus = nil

// The lock becomes available after shutdown, but the update gate changes
// during that wait. The production post-acquisition guard must close its fd.
let changingGate = NodeController()
changingGate.storageMovePreparing = true
changingGate.unattended?.runningNodePID = 61
let changingGateFD = open(NodeController.dataDir.appendingPathComponent("run.lock").path, O_RDWR | O_CREAT, 0o600)
check(changingGateFD >= 0 && flock(changingGateFD, LOCK_EX | LOCK_NB) == 0,
      "R06 changing gate fixture owns the lock before shutdown")
changingGate.unattended?.onStop = {
    Task { @MainActor in
        changingGate.updateInProgress = true
        close(changingGateFD)
    }
}
let rejectedOwnership = await changingGate.acquireStorageMoveOwnership()
if let rejectedOwnership { close(rejectedOwnership) }
check(rejectedOwnership == nil && changingGate.stops == 1
      && !NodeController.lockHeld(in: NodeController.dataDir),
      "R06 a changed gate after lock acquisition releases the owned descriptor")
changingGate.unattended?.onStop = nil

// R06. The actual mover pauses daemon respawn before either node stops and
// retains ownership during preparation, selection publication and commit.
try? FileManager.default.removeItem(at: recordURL)
let fencedSource = moveFixture.appendingPathComponent("r06-source")
let fencedTarget = moveFixture.appendingPathComponent("r06-target")
try FileManager.default.createDirectory(at: fencedSource.appendingPathComponent("follow"), withIntermediateDirectories: true)
try Data(repeating: 0x31, count: 256 * 1024).write(to: fencedSource.appendingPathComponent("follow/state.db"))
let fencedMover = NodeController()
fencedMover.attached = true
fencedMover.unattended?.runningNodePID = 42
fencedMover.chainDataPath = fencedSource.path
fencedMover.moveBlockData(to: fencedTarget)
try await waitForMove(fencedMover)
check(fencedMover.markerAtStops == [false]
      && fencedMover.unattended?.markerAtDaemonStops == [false],
      "R06 daemon respawn is suspended before stopping either node")
check(fencedMover.chainDataPath == fencedTarget.path, "R06 fenced fresh start commits its destination")
check(fencedMover.selectionLockChecks == [true],
      "R06 exclusive run.lock is retained through journal publication and selection commit")
check(!NodeController.lockHeld(in: NodeController.dataDir)
      && fencedMover.unattended?.resumeCalls == 1
      && fencedMover.unattended?.markerEnabled == true,
      "R06 ownership is released and daemon respawn resumes after commit")

// A live writer that does not exit must time out before any preparation or
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
timeoutMover.unattended?.runningNodePID = 43
timeoutMover.chainDataPath = timeoutSource.path
timeoutMover.moveBlockData(to: timeoutTarget)
try await waitForMove(timeoutMover)
check(timeoutMover.chainDataPath == timeoutSource.path && timeoutMover.storageMoveError != nil,
      "R06 lock timeout never switches source selection")
check(!FileManager.default.fileExists(atPath: timeoutTarget.path)
      && !FileManager.default.fileExists(atPath: recordURL.path)
      && (try? Data(contentsOf: timeoutDB)) == Data("writer-owned".utf8),
      "R06 lock timeout creates no fresh location or cleanup authorization")
check(NodeController.lockHeld(in: NodeController.dataDir)
      && timeoutMover.unattended?.resumeCalls == 1
      && timeoutMover.unattended?.markerEnabled == true,
      "R06 timeout keeps the writer lock and resumes the original daemon choice")
close(busyFD)

// If the marker cannot be durably removed, stop nothing and prepare nothing.
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



// A full default round trip must never export or replace keys or settings.
resetJournal()
let internalFollow = NodeController.dataDir.appendingPathComponent("follow")
try fm.createDirectory(at: internalFollow.appendingPathComponent("nested"), withIntermediateDirectories: true)
let internalState = internalFollow.appendingPathComponent("state.redb")
try Data("old internal state".utf8).write(to: internalState)
let protectedPaths = [
    "validator.key", "node-account.key", "node.identity", "key-binding.json", "network.json", "settings.json",
    "follow/wallet-node.key", "follow/nested/Wallet-Node.Key", "follow/nested/key-binding.json",
]
for path in protectedPaths { try Data("protected \(path)".utf8).write(to: NodeController.dataDir.appendingPathComponent(path)) }
let rootSentinel = NodeController.dataDir.appendingPathComponent("unrelated.txt")
try Data("outside block data".utf8).write(to: rootSentinel)
let link = internalFollow.appendingPathComponent("outside-link")
try fm.createSymbolicLink(at: link, withDestinationURL: rootSentinel)
let roundTripTarget = moveFixture.appendingPathComponent("round-trip-target")
let roundTripMover = NodeController()
roundTripMover.moveBlockData(to: roundTripTarget)
try await waitForMove(roundTripMover)
check(roundTripMover.storageMoveError == nil && roundTripMover.chainDataPath == roundTripTarget.path
      && roundTripMover.storageMoveSync != nil, "the real mover starts fresh and keeps a sync status")
check((try? fm.contentsOfDirectory(atPath: roundTripTarget.path)) == [],
      "real move performs no block-data or key copy")
roundTripMover.finishBlockDataMove(answered: true, height: 50, certifiedHeight: nil, networkHeight: 100)
await roundTripMover.storageMoveCleanup?.value
check(fm.fileExists(atPath: internalState.path), "the real controller retains old data after just an answer")
roundTripMover.finishBlockDataMove(answered: true, height: 51, certifiedHeight: 51, networkHeight: 100)
await roundTripMover.storageMoveCleanup?.value
check(!fm.fileExists(atPath: internalState.path) && roundTripMover.storageMoveSync != nil,
      "following a certified block frees old space while the remaining sync stays normal")
for path in protectedPaths {
    check((try? Data(contentsOf: NodeController.dataDir.appendingPathComponent(path))) == Data("protected \(path)".utf8),
          "the exact key/identity/settings bytes stay internal: \(path)")
    check(!fm.fileExists(atPath: roundTripTarget.appendingPathComponent(path).path), "no protected file is exported: \(path)")
}
check((try? Data(contentsOf: rootSentinel)) == Data("outside block data".utf8)
      && (try? fm.destinationOfSymbolicLink(atPath: link.path)) == rootSentinel.path,
      "cleanup preserves unrelated files and never follows links")
roundTripMover.finishBlockDataMove(answered: true, height: 100, certifiedHeight: 100, networkHeight: 100)
await roundTripMover.storageMoveCleanup?.value
check(roundTripMover.storageMoveSync == nil, "the sync status clears after catching up")
let completedOutward = try JSONDecoder().decode(BlockDataMove.Record.self, from: Data(contentsOf: recordURL))
check(completedOutward.committed && completedOutward.cleanupDone && completedOutward.target == roundTripTarget.path,
      "R07 reverse movement waits for the committed outward cleanup record")
// A return accepts retained nested keys in place, and refuses links.
check(!BlockDataLocation.destinationAvailable(NodeController.dataDir, preservingInternalKeys: true),
      "a retained link is not an empty return destination")
try fm.removeItem(at: link)
check(BlockDataLocation.destinationAvailable(NodeController.dataDir, preservingInternalKeys: true),
      "nested internal keys remain in place when returning to default")
let externalFollow = roundTripTarget.appendingPathComponent("follow")
try fm.createDirectory(at: externalFollow, withIntermediateDirectories: true)
let externalState = externalFollow.appendingPathComponent("state.redb")
try Data("external synced state".utf8).write(to: externalState)
roundTripMover.moveBlockData(to: nil)
try await waitForMove(roundTripMover)
check(roundTripMover.storageMoveError == nil && roundTripMover.chainDataPath.isEmpty
      && !fm.fileExists(atPath: internalState.path), "move back to default starts empty without copying external data")
check((try? Data(contentsOf: internalFollow.appendingPathComponent("wallet-node.key"))) == Data("protected follow/wallet-node.key".utf8),
      "the internal endpoint key survives the round trip")
roundTripMover.finishBlockDataMove(answered: true, height: 100, certifiedHeight: nil, networkHeight: 101)
await roundTripMover.storageMoveCleanup?.value
check(fm.fileExists(atPath: externalState.path), "return move also waits for an answered-and-followed block")
roundTripMover.finishBlockDataMove(answered: true, height: 101, certifiedHeight: 101, networkHeight: 101)
await roundTripMover.storageMoveCleanup?.value
check(!fm.fileExists(atPath: externalState.path), "return confirmation deletes only external block data")

// A legacy copy journal remains authoritative, but cannot replay or delete.
resetJournal()
let (legacySource2, legacyTarget2) = try prepare("legacy-record")
let raw = try Data(contentsOf: recordURL)
var legacy = try JSONSerialization.jsonObject(with: raw) as! [String: Any]
legacy.removeValue(forKey: "version")
legacy["directories"] = ["follow"]
legacy["files"] = ["follow/state.redb": ["hash": "unused"]]
try JSONSerialization.data(withJSONObject: legacy).write(to: recordURL, options: .atomic)
check(try BlockDataMove.authoritativeRoot(in: NodeController.dataDir)?.path == legacyTarget2.path,
      "legacy selection is respected without reading or verifying old data")
check(!BlockDataMove.cleanup(confirmedTarget: legacyTarget2, internalRoot: NodeController.dataDir)
      && fm.fileExists(atPath: legacySource2.appendingPathComponent("follow/state.redb").path),
      "legacy records authorize no fresh-start cleanup")
}
#endif

// MARK: archive requirements, on measured numbers

check(ArchiveRequirements.bytesPerBlock > 0 && ArchiveRequirements.bytesPerDay > 0, "measured on the archive node, not zero")
let req = ArchiveRequirements(height: 500_000)
check(req.sizeNowBytes == UInt64(500_000 * ArchiveRequirements.bytesPerBlock), "size scales with the height")
check(req.recommendedFreeBytes == req.sizeNowBytes + req.perYearBytes + 7 * GiB, "now + a year + the resume margin")
for language in ["en", "ko", "ja", "zh-Hans", "zh-Hant"] {
    let lines = req.lines(locale: walletTestLocale(language), bundle: walletTestBundle(language))
    check(lines.count == 4, "size, disk, first sync, no reward")
    check(lines[0].contains(NodeStopReason.gb(req.sizeNowBytes)) && lines[0].contains(NodeStopReason.gb(req.perMonthBytes)), "real numbers: \(lines[0])")
    check(lines[1].contains(NodeStopReason.gb(req.recommendedFreeBytes)), "the recommended disk size")
    let all = lines.joined(separator: " ")
    for promise in ["earn", "벌", "수익", "APY", "%"] { check(!all.contains(promise), "no reward promise (\(promise)) language=\(language)") }
}
check(req.lines(locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))[3].contains("보상은 없어요"), "says plainly there is no reward")
check(req.lines(locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))[2].contains("몇 시간에서 며칠"), "hours to days")
#if os(macOS)
Task { @MainActor in
    do { try await runMoveFixtureTests(); print("OK block-data"); exit(0) }
    catch { print("FAIL fixture:", error); exit(1) }
}
dispatchMain()
#else
print("OK block-data")
#endif
