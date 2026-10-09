import CryptoKit
import Foundation
import Network
import Sparkle

// Only the production default refers to the FFI bridge. This standalone
// fixture supplies inert symbols and injects its independently verified proof.
enum WalletError: Error { case Network(message: String), Verification(message: String) }
func configuredChainId() -> UInt64 { 9_001 }
func verifiedRelease(contract: String, codeHash: String, index: UInt64, validators: UInt32) throws -> ReleaseProof {
    fatalError("fixture must use its injected verifier")
}
func check(_ value: @autoclosure () -> Bool, _ message: String) {
    if !value() { fatalError(message) }
}
func hex(_ data: Data) -> String { data.map { String(format: "%02x", $0) }.joined() }
func json(_ value: Any) throws -> Data { try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]) }

let keys = (0..<3).map { _ in P256.Signing.PrivateKey() }
let trust = ReleaseTrust(chainId: 9_001, logAddress: "0x0000000000000000000000000000000000007705",
    codeHash: "0x" + String(repeating: "11", count: 32), builderKeys: keys.map { hex($0.publicKey.x963Representation) }, legacy: false)
let archive = Data("real Sparkle handoff fixture archive".utf8)
let archiveHash = ReleaseApproval.digest(archive)
let sparkleKey = Curve25519.Signing.PrivateKey()
let sparkleSignatureData = try sparkleKey.signature(for: archive)
let sparkleSignature = sparkleSignatureData.base64EncodedString()
let height: UInt64 = 100 + ReleaseApproval.waitSeconds
let timestamp: UInt64 = 1_000_000 + ReleaseApproval.waitSeconds
let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent("tmp/chain-release-sparkle-\(UUID().uuidString)")
try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: root) }

func fixture(signers: Int = 2, stateHeight: UInt64 = height, forged: Bool = false,
             version: String = "0.7.4", index: UInt64 = 0) throws -> (ChainReleaseAnnouncement, ReleaseProof) {
    let manifest = try json(["artifacts": [["name": "EastSea.dmg", "sha256": archiveHash, "size": archive.count]],
        "build": "74", "chain_id": 9_001, "log_address": trust.logAddress,
        "platform": "macos-arm64-dmg", "version": version, "emergency": false,
        "sparkle_ed_signature": sparkleSignature] as [String: Any])
    let signatures = try json(try keys.prefix(signers).map { key in
        ["public_key": hex(key.publicKey.x963Representation),
         "signature": hex(try key.signature(for: manifest).rawRepresentation)]
    })
    let proof = ReleaseProof(manifestSha256: ReleaseApproval.digest(manifest), archiveSha256: archiveHash,
        signaturesSha256: ReleaseApproval.digest(signatures), publishedBlock: 100, publishedAt: 1_000_000,
        emergency: false, stateHeight: stateHeight, certifiedTimestampMs: timestamp * 1_000,
        certifiedBlock: stateHeight + 1)
    let announcement = try json(["index": index, "chain_id": 9_001, "log_address": trust.logAddress,
        "version": version, "build": "74", "manifest_hash": forged ? String(repeating: "0", count: 64) : proof.manifestSha256,
        "archive_sha256": archiveHash, "signatures_hash": proof.signaturesSha256,
        "manifest": String(decoding: manifest, as: UTF8.self), "signatures": String(decoding: signatures, as: UTF8.self),
        "approved_at_height": 100, "install_after_height": height,
        "restart_slot": ["slot_blocks": 600, "cycle_slots": 4]] as [String: Any])
    guard let parsed = ChainReleaseAnnouncement.parse(json: String(decoding: announcement, as: UTF8.self)) else {
        fatalError("fixture announcement must parse restart metadata")
    }
    return (parsed, proof)
}

func prepared(_ gate: ReleaseUpdateGate, _ announcement: ChainReleaseAnnouncement) async -> (PreparedChainRelease?, PendingRelease?, String?) {
    await withCheckedContinuation { continuation in
        gate.prepare(announcement: announcement, validators: 4) { item, pending, issue in
            continuation.resume(returning: (item, pending, issue))
        }
    }
}

func splitHeaderRequest(_ url: URL) async throws -> Data {
    try await withCheckedThrowingContinuation { continuation in
        let queue = DispatchQueue(label: "chain-release.fixture.split-header")
        let connection = NWConnection(host: "127.0.0.1", port: NWEndpoint.Port(rawValue: UInt16(url.port!))!, using: .tcp)
        var response = Data()
        var finished = false
        func finish(_ result: Result<Data, Error>) {
            guard !finished else { return }
            finished = true
            connection.stateUpdateHandler = nil
            connection.cancel()
            continuation.resume(with: result)
        }
        func receive() {
            connection.receive(minimumIncompleteLength: 1, maximumLength: 16_384) { part, _, complete, error in
                if let part { response.append(part) }
                if complete || error != nil {
                    if !response.isEmpty { finish(.success(response)) }
                    else { finish(.failure(error ?? URLError(.badServerResponse))) }
                } else { receive() }
            }
        }
        connection.stateUpdateHandler = { state in
            switch state {
            case .ready:
                let first = Data("GET \(url.path) HTTP/1.1\r\nHost: 127.0.0.1\r\n".utf8)
                connection.send(content: first, completion: .contentProcessed { error in
                    if let error { finish(.failure(error)); return }
                    queue.asyncAfter(deadline: .now() + 0.1) {
                        connection.send(content: Data("\r\n".utf8), completion: .contentProcessed { error in
                            if let error { finish(.failure(error)) }
                            else { receive() }
                        })
                    }
                })
            case .failed(let error): finish(.failure(error))
            default: break
            }
        }
        connection.start(queue: queue)
    }
}

let approved = try fixture()
var currentProof = approved.1
var fetches = 0
var activeChainId: UInt64 = 9_001
var switchChainDuringVerify = false
var sourceOffline = false
var proofOffline = false
let gate = ReleaseUpdateGate(trust: trust, chainId: { activeChainId }, verify: { _, _, _ in
    if proofOffline { throw WalletError.Network(message: "fixture is offline") }
    if switchChainDuringVerify { activeChainId = 9_002 }
    return currentProof
}, cacheDirectory: root) { _, destination in
    fetches += 1
    if sourceOffline { throw URLError(.notConnectedToInternet) }
    try archive.write(to: destination)
}
let result = await prepared(gate, approved.0)
guard let item = result.0 else { fatalError("approved release failed: \(result.2 ?? "missing item")") }
check(result.2 == nil && result.1?.version == "0.7.4" && result.1?.installAfterHeight == height,
    "approved release is silent and its pending height comes from the proof")
check(fetches == 1 && item.appcastURL.host == "127.0.0.1", "release event fetches once and binds loopback")
check(item.item.versionString == "74" && item.item.displayVersionString == "0.7.4", "real SUAppcastItem built from manifest")
check(gate.mayProceed(item.item), "real Sparkle handoff passes only verified item")

let session = URLSession(configuration: .ephemeral)
let (feed, feedResponse) = try await session.data(from: item.appcastURL)
check((feedResponse as? HTTPURLResponse)?.statusCode == 200, "real loopback feed is HTTP-compatible")
check(String(decoding: feed, as: UTF8.self).contains("sparkle:shortVersionString=\"0.7.4\""), "served feed matches signed manifest")
let splitResponse = try await splitHeaderRequest(item.appcastURL)
check(String(decoding: splitResponse, as: UTF8.self).contains("HTTP/1.1 200 OK")
    && splitResponse.suffix(feed.count) == feed, "fragmented header is accumulated before serving exact feed bytes")
guard let artifactURL = item.item.fileURL as URL? else { fatalError("verified item must have its enclosure URL") }
let (downloaded, downloadResponse) = try await session.data(from: artifactURL)
check((downloadResponse as? HTTPURLResponse)?.statusCode == 200 && downloaded == archive,
    "Sparkle's exact enclosure serves already verified bytes")
check(sparkleKey.publicKey.isValidSignature(sparkleSignatureData, for: downloaded),
    "fixture's pinned EdDSA key verifies actual served archive")
let guessed = item.appcastURL.deletingLastPathComponent().appendingPathComponent("unknown/EastSea.dmg")
let (_, guessedResponse) = try await session.data(from: guessed)
check((guessedResponse as? HTTPURLResponse)?.statusCode == 404, "unknown routes cannot read files")

let remote = SUAppcastItem(dictionary: ["enclosure": ["url": "https://malicious.example/EastSea.dmg",
    "sparkle:version": "74", "sparkle:shortVersionString": "0.7.4",
    "sparkle:edSignature": sparkleSignature]])!
check(!gate.mayProceed(remote), "same signed metadata cannot authorize poisoned remote second download")

var installs = 0
let install = { installs += 1 }
let issue = await gate.validateForInstall(item: item.item, validators: 4)
if issue == nil { install() }
check(issue == nil && installs == 1, "approved entry calls immediate install closure without prompt")
currentProof.stateHeight = height - 1
currentProof.certifiedBlock = height
let earlyIssue = await gate.validateForInstall(item: item.item, validators: 4)
check(earlyIssue == ChainReleaseFailure.beforeSlot.sentence && installs == 1,
    "fresh independent certificate checked immediately before install")
currentProof = approved.1
switchChainDuringVerify = true
let switchedIssue = await gate.validateForInstall(item: item.item, validators: 4)
check(switchedIssue == ChainReleaseFailure.forgedEntry.sentence && installs == 1,
    "network switch during asynchronous proof recheck refuses before install")
switchChainDuringVerify = false
activeChainId = 9_001

let cache = root.appendingPathComponent(archiveHash).appendingPathComponent("EastSea.dmg")
try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: cache.path)
try Data("poisoned after preflight".utf8).write(to: cache)
check(!gate.mayProceed(item.item), "cache mutation cannot pass the download callback")
let mismatchIssue = await gate.validateForInstall(item: item.item, validators: 4)
check(mismatchIssue == ChainReleaseFailure.hashMismatch.sentence, "late hash mismatch refuses with a sentence")

let faults: [(String, (ChainReleaseAnnouncement, ReleaseProof), ChainReleaseFailure, Bool)] = [
    ("forged", try fixture(forged: true), .forgedEntry, false),
    ("unapproved", try fixture(signers: 1), .insufficientApprovals, false),
    ("early", try fixture(stateHeight: height - 1), .beforeSlot, false),
    ("hash", approved, .hashMismatch, true),
]
for (name, source, failure, poisoned) in faults {
    var reachedFetch = false
    let faultGate = ReleaseUpdateGate(trust: trust, chainId: { 9_001 }, verify: { _, _, _ in source.1 },
        cacheDirectory: root.appendingPathComponent(name)) { _, destination in
        reachedFetch = true
        try (poisoned ? Data("wrong hash".utf8) : archive).write(to: destination)
    }
    let faultResult = await prepared(faultGate, source.0)
    check(faultResult.0 == nil && faultResult.2 == failure.sentence, "\(name) has one exact refusal sentence")
    check(reachedFetch == poisoned, "\(name): no artifact network call before approval and slot")
}

// Superseding an entry closes its old origin and changes the bound Sparkle key.
try archive.write(to: cache)
let approvalBeforeSuperseding = await gate.validateForInstall(item: item.item, validators: 4)
check(approvalBeforeSuperseding == nil, "old release was approved before asynchronous preparation")
let next = try fixture(version: "0.7.5", index: 1)
currentProof = next.1
proofOffline = true
let unavailableProof = await prepared(gate, next.0)
check(unavailableProof.0 == nil && unavailableProof.2 == ChainReleaseFailure.unavailable.sentence,
    "a transient proof transport failure remains retryable without installing")
proofOffline = false
try FileManager.default.removeItem(at: cache)
sourceOffline = true
let failedReplacement = await prepared(gate, next.0)
check(failedReplacement.0 == nil && failedReplacement.2 == ChainReleaseFailure.unavailable.sentence
    && !gate.hasPreparedApproval(for: item.item),
    "failed supersession revokes the old feed and remains eligible for a transport retry")
sourceOffline = false
let replacement = await prepared(gate, next.0)
guard let replacementItem = replacement.0 else { fatalError("new approved entry must prepare") }
check(!gate.mayProceed(item.item) && gate.mayProceed(replacementItem.item), "superseded item loses permission")
var stoppedWriters = 0
if gate.hasPreparedApproval(for: item.item) { stoppedWriters += 1 }
check(!gate.hasPreparedApproval(for: item.item) && gate.hasPreparedApproval(for: replacementItem.item)
    && stoppedWriters == 0 && installs == 1,
    "superseding after proof validation refuses old item before quiescing writers")
var oldRequest = URLRequest(url: item.appcastURL)
oldRequest.timeoutInterval = 2
let oldOrigin = try? await session.data(for: oldRequest)
check(oldOrigin == nil, "superseded loopback origin is closed")
session.invalidateAndCancel()
print("chain-release-sparkle: four faults refused; real item/feed/archive and one prompt-free install closure passed")
