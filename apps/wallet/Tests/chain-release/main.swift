import CryptoKit
import Foundation

func check(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() { fatalError(message) }
}
func hex(_ data: Data) -> String { data.map { String(format: "%02x", $0) }.joined() }
func json(_ object: Any) throws -> Data {
    try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
}

let keys = (0..<3).map { _ in P256.Signing.PrivateKey() }
let publicKeys = keys.map { hex($0.publicKey.x963Representation) }
let trust = ReleaseTrust(chainId: 9_001,
    logAddress: "0x0000000000000000000000000000000000007705",
    codeHash: "0x" + String(repeating: "11", count: 32), builderKeys: publicKeys, legacy: false)
let bytes = Data("approved fixture archive".utf8)
let archiveHash = ReleaseApproval.digest(bytes)
let published: UInt64 = 100
let readyHeight = published + ReleaseApproval.waitSeconds
let publishedAt: UInt64 = 1_000_000

func fixture(signers: Int = 2, emergency: Bool = false, height: UInt64 = readyHeight,
             timestamp: UInt64 = publishedAt + ReleaseApproval.waitSeconds,
             artifactChanges: [String: Any] = [:], manifestChanges: [String: Any] = [:],
             announcementChanges: [String: Any] = [:]) throws -> (ChainReleaseAnnouncement, ReleaseProof) {
    var artifact: [String: Any] = ["name": "EastSea.dmg", "sha256": archiveHash,
        "size": bytes.count, "url": "https://github.com/eastsea-xyz/eastsea/releases/download/app-v0.7.4/EastSea-0.7.4.dmg",
        "webseeds": ["https://seed.example/\(archiveHash)/EastSea.dmg"],
        "peers": ["http://peer.example/artifacts/\(archiveHash)"]]
    artifact.merge(artifactChanges, uniquingKeysWith: { _, new in new })
    var manifest: [String: Any] = ["artifacts": [artifact], "build": "74", "chain_id": 9_001,
        "log_address": trust.logAddress, "platform": "macos-arm64-dmg", "version": "0.7.4",
        "sparkle_ed_signature": Data(repeating: 7, count: 64).base64EncodedString(), "emergency": emergency]
    manifest.merge(manifestChanges, uniquingKeysWith: { _, new in new })
    let manifestData = try json(manifest)
    let signaturesData = try json(try keys.prefix(signers).map { key in
        ["public_key": hex(key.publicKey.x963Representation),
         "signature": hex(try key.signature(for: manifestData).rawRepresentation)]
    })
    let proof = ReleaseProof(manifestSha256: ReleaseApproval.digest(manifestData), archiveSha256: archiveHash,
        signaturesSha256: ReleaseApproval.digest(signaturesData), publishedBlock: published,
        publishedAt: publishedAt, emergency: emergency, stateHeight: height,
        certifiedTimestampMs: timestamp * 1_000, certifiedBlock: height + 1)
    var announcement: [String: Any] = ["index": 0, "chain_id": 9_001, "log_address": trust.logAddress,
        "version": "0.7.4", "build": "74", "manifest_hash": proof.manifestSha256,
        "archive_sha256": archiveHash, "signatures_hash": proof.signaturesSha256,
        "manifest": String(decoding: manifestData, as: UTF8.self),
        "signatures": String(decoding: signaturesData, as: UTF8.self),
        "approved_at_height": published, "install_after_height": readyHeight,
        "published_at": publishedAt, "emergency": emergency,
        "state_height": height, "certified_timestamp_ms": timestamp * 1_000,
        "approvals": 3, "required_approvals": emergency ? 3 : 2]
    announcement.merge(announcementChanges, uniquingKeysWith: { _, new in new })
    guard let parsed = ChainReleaseAnnouncement.parse(json: String(decoding: try json(announcement), as: UTF8.self))
    else { fatalError("valid fixture must parse") }
    return (parsed, proof)
}

func policy(_ fixture: (ChainReleaseAnnouncement, ReleaseProof)) throws -> VerifiedChainRelease {
    try ChainReleasePolicy.verify(announcement: fixture.0, trust: trust, proof: fixture.1)
}
func refuses(_ expected: ChainReleaseFailure, _ message: String,
             operation: () throws -> Void) {
    do { try operation(); fatalError("\(message) was accepted") }
    catch let failure as ChainReleaseFailure {
        check(failure == expected, "\(message): wrong refusal \(failure)")
        check(!failure.sentence.isEmpty && !failure.sentence.contains("\n"), "one plain sentence per refusal")
    } catch { fatalError("\(message): unexpected error \(error)") }
}

let approvedFixture = try fixture()
let approved = try policy(approvedFixture)
check(approved.manifest.version == "0.7.4" && approved.manifest.build == "74", "item comes from signed manifest")
check(approved.installAfterHeight == readyHeight, "install barrier derives from certified publication height")
check(approved.artifactSources.count == 3, "GitHub, webseed and peer are fetch candidates")

let inflatedDiscovery = try fixture(announcementChanges: ["install_after_height": UInt64.max,
    "restart_slot_height": UInt64.max])
let independentlyReady = try policy(inflatedDiscovery)
check(independentlyReady.installAfterHeight == readyHeight,
    "untrusted discovery heights cannot postpone a certified signed release")

// A maximum-sized valid manifest can expand when nested as an escaped JSON
// string. Bound the raw payloads independently of their discovery envelope.
let padding = Data(String(repeating: "\t", count: 16_384 - approvedFixture.0.manifestData.count).utf8)
let largeManifest = padding + approvedFixture.0.manifestData
let largeSignatures = try json(try keys.prefix(2).map { key in
    ["public_key": hex(key.publicKey.x963Representation),
     "signature": hex(try key.signature(for: largeManifest).rawRepresentation)]
})
let largeProof = ReleaseProof(manifestSha256: ReleaseApproval.digest(largeManifest), archiveSha256: archiveHash,
    signaturesSha256: ReleaseApproval.digest(largeSignatures), publishedBlock: published,
    publishedAt: publishedAt, emergency: false, stateHeight: readyHeight,
    certifiedTimestampMs: (publishedAt + ReleaseApproval.waitSeconds) * 1_000, certifiedBlock: readyHeight + 1)
let largeEnvelope = try json(["index": 1, "chain_id": 9_001, "log_address": trust.logAddress,
    "version": "0.7.4", "build": "74", "manifest_hash": largeProof.manifestSha256,
    "archive_sha256": archiveHash, "signatures_hash": largeProof.signaturesSha256,
    "manifest": String(decoding: largeManifest, as: UTF8.self),
    "signatures": String(decoding: largeSignatures, as: UTF8.self),
    "approved_at_height": published, "install_after_height": readyHeight] as [String: Any])
check(largeEnvelope.count > 32_768 && largeManifest.count == 16_384,
    "valid escaped discovery envelope exceeds the old parser cap")
guard let largeAnnouncement = ChainReleaseAnnouncement.parse(json: String(decoding: largeEnvelope, as: UTF8.self)) else {
    fatalError("full-sized signed manifest must remain discoverable")
}
_ = try ChainReleasePolicy.verify(announcement: largeAnnouncement, trust: trust, proof: largeProof)

// U4: discovery data grants no authority; every fault has one distinct sentence.
let forged = try fixture(announcementChanges: ["manifest_hash": String(repeating: "0", count: 64)])
refuses(.forgedEntry, "forged entry") { _ = try policy(forged) }
let unapproved = try fixture(signers: 1)
refuses(.insufficientApprovals, "RPC approval count with only one actual signature") { _ = try policy(unapproved) }
let early = try fixture(height: readyHeight - 1)
refuses(.beforeSlot, "install before its slot") { _ = try policy(early) }
let earlyClock = try fixture(timestamp: publishedAt + ReleaseApproval.waitSeconds - 1)
refuses(.beforeSlot, "uncertified 72 hour wait") { _ = try policy(earlyClock) }
let emergencyEarly = try fixture(signers: 3, emergency: true, height: published + 1, timestamp: publishedAt + 1)
refuses(.beforeSlot, "emergency does not bypass chain wait") { _ = try policy(emergencyEarly) }
_ = try policy(fixture(signers: 3, emergency: true))

let forgedHeight = try fixture(height: published + 1, announcementChanges: ["install_after_height": published + 1])
refuses(.beforeSlot, "discovery cannot lower the signed proof's minimum height") { _ = try policy(forgedHeight) }
let extended = try fixture(height: readyHeight + 9,
    manifestChanges: ["install_after_height": readyHeight + 10],
    announcementChanges: ["install_after_height": readyHeight + 10])
refuses(.beforeSlot, "signed additional install barrier") { _ = try policy(extended) }
let signedSlot = try fixture(height: readyHeight + 9,
    manifestChanges: ["restart_slot_height": readyHeight + 10],
    announcementChanges: ["restart_slot_height": readyHeight])
refuses(.beforeSlot, "discovery cannot lower the restart slot signed in the manifest") { _ = try policy(signedSlot) }
let omittedSlot = try fixture(height: readyHeight + 9,
    manifestChanges: ["restart_slot_height": readyHeight + 10])
refuses(.beforeSlot, "discovery cannot omit the restart slot signed in the manifest") { _ = try policy(omittedSlot) }
let changedPublished = try fixture(announcementChanges: ["approved_at_height": published - 1])
refuses(.forgedEntry, "discovery cannot change certified publication") { _ = try policy(changedPublished) }
var uncertified = approvedFixture.1
uncertified.certifiedBlock = uncertified.stateHeight
refuses(.forgedEntry, "proof root must have its following certificate") {
    _ = try ChainReleasePolicy.verify(announcement: approvedFixture.0, trust: trust, proof: uncertified)
}
var forgedState = approvedFixture.1
forgedState.stateHeight = 0
refuses(.forgedEntry, "missing independently verified state") {
    _ = try ChainReleasePolicy.verify(announcement: approvedFixture.0, trust: trust, proof: forgedState)
}
var foreignManifest = try JSONSerialization.jsonObject(with: approvedFixture.0.manifestData) as! [String: Any]
foreignManifest["chain_id"] = 9_002
let foreign = try fixture(manifestChanges: foreignManifest)
refuses(.forgedEntry, "manifest bound to another chain") { _ = try policy(foreign) }

for source in ["file:///etc/passwd", "https://user:pass@example/secret", "ftp://example/update", "https://example/#fragment"] {
    refuses(.forgedEntry, "unbounded or unsupported manifest source") {
        _ = try policy(fixture(artifactChanges: ["url": source]))
    }
}
refuses(.forgedEntry, "artifact count and transport sources are bounded") {
    _ = try policy(fixture(artifactChanges: ["webseeds": Array(repeating: "https://seed.example/file", count: 9)]))
}
refuses(.forgedEntry, "no giant archive fetch") {
    _ = try policy(fixture(artifactChanges: ["size": UInt64.max]))
}

// Old signed manifests contain just name/hash. Their source is deterministic,
// and is requested only after a chain announcement has passed verification.
let oldArtifact = try fixture(artifactChanges: ["size": NSNull(), "url": NSNull(),
    "webseeds": NSNull(), "peers": NSNull()])
let old = try policy(oldArtifact)
check(old.artifactSources.map(\.absoluteString) == ["https://github.com/eastsea-xyz/eastsea/releases/download/app-v0.7.4/EastSea-0.7.4.dmg"],
    "compatibility source is the existing release URL")

let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent("tmp/chain-release-\(UUID().uuidString)")
try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: root) }

var visited: [URL] = []
let localArtifact = try await ReleaseArtifact.acquire(approved, cacheDirectory: root) { source, destination in
    visited.append(source)
    // The remote origin is poisoned, but the next independently hashed source works.
    try (visited.count == 1 ? Data("wrong bytes".utf8) : bytes).write(to: destination)
}
check(visited.count == 2, "hash mismatch falls back to webseed")
let fetchedHash = try ReleaseArtifact.digest(file: localArtifact)
check(fetchedHash == archiveHash, "actual archive bytes match manifest hash")
check(localArtifact.path.hasPrefix(root.path + "/"), "artifact confined to the designated task cache")

let reused = try await ReleaseArtifact.acquire(approved, cacheDirectory: root) { _, _ in
    fatalError("verified cache must avoid another untrusted download")
}
check(reused == localArtifact, "same hash reuses exact cached bytes")

try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: localArtifact.path)
try Data("poisoned cached bytes".utf8).write(to: localArtifact)
var repaired = 0
_ = try await ReleaseArtifact.acquire(approved, cacheDirectory: root) { _, destination in
    repaired += 1
    try bytes.write(to: destination)
}
check(repaired == 1, "cached bytes are rehashed before reuse")

var peerVisits: [URL] = []
let peerArtifact = try await ReleaseArtifact.acquire(approved, cacheDirectory: root.appendingPathComponent("peer-fallback")) { source, destination in
    peerVisits.append(source)
    guard source.host == "peer.example" else { throw URLError(.cannotConnectToHost) }
    try bytes.write(to: destination)
}
let peerHash = try ReleaseArtifact.digest(file: peerArtifact)
check(peerVisits.count == 3 && peerHash == archiveHash, "unavailable origin and seed fall back to independently hashed peer bytes")

let mismatchRoot = root.appendingPathComponent("all-poisoned")
do {
    _ = try await ReleaseArtifact.acquire(approved, cacheDirectory: mismatchRoot) { _, destination in
        try Data("hash mismatch".utf8).write(to: destination)
    }
    fatalError("hash mismatch was accepted")
} catch let failure as ChainReleaseFailure {
    check(failure == .hashMismatch && !failure.sentence.isEmpty, "hash mismatch refuses with one sentence")
}
let wrongSizeRoot = root.appendingPathComponent("wrong-size")
let wrongSize = try policy(fixture(artifactChanges: ["size": bytes.count + 1]))
do {
    _ = try await ReleaseArtifact.acquire(wrongSize, cacheDirectory: wrongSizeRoot) { _, destination in
        try bytes.write(to: destination)
    }
    fatalError("signed size mismatch was accepted")
} catch let failure as ChainReleaseFailure { check(failure == .hashMismatch, "size is bound with hash") }

let loopback = URL(string: "http://127.0.0.1:1234/private/\(archiveHash)/EastSea.dmg")!
let feed = try ChainReleasePolicy.appcastXML(release: approved, artifactURL: loopback, length: UInt64(bytes.count))
let xml = String(decoding: feed, as: UTF8.self)
check(xml.contains("sparkle:version=\"74\"") && xml.contains("sparkle:shortVersionString=\"0.7.4\""), "Sparkle item uses verified manifest")
check(xml.contains(loopback.absoluteString) && !xml.contains("seed.example"), "Sparkle only downloads verified cached bytes")
check(xml.contains("sparkle:edSignature=\"\(approved.manifest.sparkleEdSignature)\""), "Sparkle keeps pinned EdDSA check")
check(!xml.contains("sparkle:criticalUpdate"), "silent install does not relax existing safety gates")

check(ChainReleaseAnnouncement.parse(json: "{}") == nil, "malformed announcement fails closed")
check(ChainReleaseAnnouncement.parse(json: String(repeating: "x", count: 128 * 1_024 + 1)) == nil, "discovery envelope bounded")
check(!UpdateChannel.pollsForDiscovery(trust: trust), "valid release pin disables discovery polling")
check(!UpdateChannel.pollsForDiscovery(trust: nil), "malformed or missing new chain pins cannot start polling")
let legacy = ReleaseTrust(chainId: 7_780, logAddress: "", codeHash: "", builderKeys: [], legacy: true)
check(UpdateChannel.pollsForDiscovery(trust: legacy), "pinned legacy chain keeps compatibility polling")
print("chain release: approval faults, certified timing, content-addressed fetch and quiet Sparkle feed passed")
