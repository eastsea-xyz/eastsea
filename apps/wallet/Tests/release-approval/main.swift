import CryptoKit
import Foundation

func hex(_ data: Data) -> String { data.map { String(format: "%02x", $0) }.joined() }

let keys = (0..<3).map { _ in P256.Signing.PrivateKey() }
let publicKeys = keys.map { hex($0.publicKey.x963Representation) }
let archive = Data("approved archive".utf8)
let archiveHash = ReleaseApproval.digest(archive)
let manifest: [String: Any] = [
    "artifacts": [["name": "EastSea.dmg", "sha256": archiveHash]],
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

// Checklist B6: what the shipped network.json pins.
func network(_ fields: [String: Any]) -> Data {
    (try? JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys])) ?? Data()
}
let codeHash = "0x4417ad7040420fe3547cdc3fdcd0fa0a690ba2f65e98af851a5e9fa5589db1ec"
let releaseLog = "0x0000000000000000000000000000000000007705"
func pin(_ change: (inout [String: Any]) -> Void = { _ in }) -> [String: Any] {
    var value: [String: Any] = ["log": releaseLog, "code_hash": codeHash, "builder_keys": publicKeys,
                                "threshold": 2, "emergency_threshold": 3]
    change(&value)
    return value
}
let shipped = try Data(contentsOf: URL(fileURLWithPath: "apps/wallet/Resources/network.json"))
assert(ReleaseTrust.parse(shipped)?.legacy == true, "the shipped 7780 file keeps the legacy Sparkle path")
assert(ReleaseTrust.parse(network(["chain_id": 7_777]))?.legacy == true, "7777 too")
assert(ReleaseTrust.parse(network(["chain_id": 9_001])) == nil,
       "a new chain without a pin has no compiled-in fallback: updates stay off")
let pinned = ReleaseTrust.parse(network(["chain_id": 9_001, "release": pin()]))
assert(pinned == ReleaseTrust(chainId: 9_001, logAddress: releaseLog, codeHash: codeHash,
                              builderKeys: publicKeys, legacy: false), "a valid pin is trusted as written")
assert(ReleaseTrust.parse(network(["chain_id": 7_780, "release": pin()]))?.legacy == false,
       "a pin, where present, is enforced even on 7780")
let flat = network(["chain_id": 9_001, "release_log": releaseLog, "release_log_code_hash": codeHash,
                        "builder_keys": publicKeys])
assert(ReleaseTrust.parse(flat) == nil, "only the release object pins (no flat-field fallback)")
let broken: [(String, [String: Any])] = [
    ("two keys", pin { $0["builder_keys"] = Array(publicKeys.prefix(2)) }),
    ("a key twice", pin { $0["builder_keys"] = [publicKeys[0], publicKeys[1], publicKeys[0].uppercased()] }),
    ("off-curve key", pin { $0["builder_keys"] = [publicKeys[0], publicKeys[1], "04" + String(repeating: "11", count: 64)] }),
    ("1-of-3", pin { $0["threshold"] = 1 }),
    ("2-of-3 emergency", pin { $0["emergency_threshold"] = 2 }),
    ("zero code hash", pin { $0["code_hash"] = "0x" + String(repeating: "0", count: 64) }),
    ("short code hash", pin { $0["code_hash"] = "0x1234" }),
    ("bare log", pin { $0["log"] = String(releaseLog.dropFirst(2)) }),
    ("no keys", pin { $0.removeValue(forKey: "builder_keys") }),
]
for (why, value) in broken {
    assert(ReleaseTrust.parse(network(["chain_id": 9_001, "release": value])) == nil, "refuses \(why)")
}
assert(ReleaseTrust.parse(network(["chain_id": 7_780, "release": "yes"])) == nil,
       "a malformed pin never falls back to the legacy path")
assert(!ReleaseTrust.missingPin.isEmpty)
print("release trust pin: \(6 + broken.count + 1) cases passed")
