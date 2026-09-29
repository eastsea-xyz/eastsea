import CryptoKit
import Foundation

func hex(_ data: Data) -> String { data.map { String(format: "%02x", $0) }.joined() }

let keys = (0..<3).map { _ in P256.Signing.PrivateKey() }
let publicKeys = keys.map { hex($0.publicKey.x963Representation) }
let archive = Data("approved archive".utf8)
let archiveHash = ReleaseApproval.digest(archive)
let manifest: [String: Any] = [
    "artifacts": [["name": "Aether.dmg", "sha256": archiveHash]],
    "build": "42", "chain_id": 9_001, "emergency": false,
    "log_address": "0x0000000000000000000000000000000000007704",
    "platform": "macos-arm64-dmg", "sparkle_ed_signature": "sparkle-test-signature",
    "version": "1.2.3",
]
let manifestData = try JSONSerialization.data(withJSONObject: manifest, options: [.sortedKeys, .fragmentsAllowed])

func signatures(_ n: Int, for data: Data = manifestData) throws -> Data {
    let entries = try keys.prefix(n).map { key in
        ["public_key": hex(key.publicKey.x963Representation),
         "signature": hex(try key.signature(for: data).rawRepresentation)]
    }
    return try JSONSerialization.data(withJSONObject: entries, options: [.sortedKeys, .fragmentsAllowed])
}

func proof(_ signaturesData: Data, emergency: Bool = false, timestamp: UInt64 = 1_000_000 + 72 * 3_600) -> ReleaseProof {
    ReleaseProof(manifestSha256: ReleaseApproval.digest(manifestData), archiveSha256: archiveHash,
        signaturesSha256: ReleaseApproval.digest(signaturesData), publishedBlock: 100,
        publishedAt: 1_000_000, emergency: emergency, stateHeight: 200,
        certifiedTimestampMs: timestamp * 1_000)
}

func decision(_ data: Data, _ signaturesData: Data, _ entry: ReleaseProof,
              archiveSha: String = archiveHash) -> ReleaseDecision {
    ReleaseApproval.decide(manifestData: data, signaturesData: signaturesData,
        archiveSha256: archiveSha, version: "1.2.3", build: "42",
        sparkleSignature: "sparkle-test-signature", chainId: 9_001,
        logAddress: "0x0000000000000000000000000000000000007704",
        pinnedKeys: publicKeys, proof: entry)
}

let two = try signatures(2)
assert(decision(manifestData, two, proof(two)) == .ready, "approved hash must install")
assert(decision(manifestData, two, proof(two), archiveSha: String(repeating: "0", count: 64)) == .rejected,
       "unapproved archive must be refused")
assert(decision(manifestData, two, proof(two, timestamp: 1_000_000 + 71 * 3_600)) != .ready,
       "ordinary release must wait 72 hours")
assert(decision(manifestData, two, proof(two, emergency: true)) == .rejected,
       "emergency requires all builders")
let three = try signatures(3)
assert(decision(manifestData, three, proof(three, emergency: true, timestamp: 1_000_001)) == .rejected,
       "emergency flag must also be signed in the manifest")

var emergencyManifest = manifest
emergencyManifest["emergency"] = true
let emergencyData = try JSONSerialization.data(withJSONObject: emergencyManifest, options: [.sortedKeys, .fragmentsAllowed])
let emergencySigs = try signatures(3, for: emergencyData)
let emergencyProof = ReleaseProof(manifestSha256: ReleaseApproval.digest(emergencyData),
    archiveSha256: archiveHash, signaturesSha256: ReleaseApproval.digest(emergencySigs),
    publishedBlock: 100, publishedAt: 1_000_000, emergency: true, stateHeight: 101,
    certifiedTimestampMs: 1_000_001_000)
assert(decision(emergencyData, emergencySigs, emergencyProof) == .ready,
       "3/3 emergency release can install immediately")
print("release approval policy: 5 cases passed")
