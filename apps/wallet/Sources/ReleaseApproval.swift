#if os(macOS)
import CryptoKit
import Foundation

struct ReleaseManifest: Decodable {
    struct Artifact: Decodable {
        let name: String
        let sha256: String
        let size: UInt64?
        let url: String?
        let webseeds: [String]?
        let peers: [String]?
    }
    let artifacts: [Artifact]
    let build: String
    let chainId: UInt64
    let emergency: Bool
    let logAddress: String
    let platform: String
    let sparkleEdSignature: String
    let version: String
    let installAfterHeight: UInt64?
    let restartSlotHeight: UInt64?
    let channel: String?

    enum CodingKeys: String, CodingKey {
        case artifacts, build, emergency, platform, version, channel
        case chainId = "chain_id", logAddress = "log_address"
        case sparkleEdSignature = "sparkle_ed_signature"
        case installAfterHeight = "install_after_height"
        case restartSlotHeight = "restart_slot_height"
    }
}

struct ReleaseBuilderSignature: Decodable {
    let publicKey: String
    let signature: String
    enum CodingKeys: String, CodingKey {
        case publicKey = "public_key", signature
    }
}

struct ReleaseProof {
    let manifestSha256: String
    let archiveSha256: String
    let signaturesSha256: String
    let publishedBlock: UInt64
    let publishedAt: UInt64
    let emergency: Bool
    var stateHeight: UInt64
    let certifiedTimestampMs: UInt64
    var certifiedBlock: UInt64

    init(manifestSha256: String, archiveSha256: String, signaturesSha256: String,
         publishedBlock: UInt64, publishedAt: UInt64, emergency: Bool, stateHeight: UInt64,
         certifiedTimestampMs: UInt64, certifiedBlock: UInt64? = nil) {
        self.manifestSha256 = manifestSha256
        self.archiveSha256 = archiveSha256
        self.signaturesSha256 = signaturesSha256
        self.publishedBlock = publishedBlock
        self.publishedAt = publishedAt
        self.emergency = emergency
        self.stateHeight = stateHeight
        self.certifiedTimestampMs = certifiedTimestampMs
        let following = stateHeight.addingReportingOverflow(1)
        self.certifiedBlock = certifiedBlock ?? (following.overflow ? 0 : following.partialValue)
    }
}

enum ReleaseDecision: Equatable {
    case ready
    case pending(until: Date)
    case rejected
}

/// What the app's own network.json says the updater trusts (docs/design/19,
/// checklist B6). A file with a `release` pin trusts only that ReleaseLog
/// address, its runtime code hash and those three builder keys under the
/// 2/3 (emergency 3/3) rule. Only the legacy testnets 7777/7780 — whose file
/// carries no pin — keep the plain Sparkle path. Anything else, including a
/// new chain with a missing or malformed pin, yields nil: there is no
/// compiled-in fallback, so updates stay off (fail closed).
struct ReleaseTrust: Equatable {
    let chainId: UInt64
    let logAddress: String
    let codeHash: String
    let builderKeys: [String]
    /// 7777/7780 without a pin: Sparkle EdDSA only (docs/design/19 "구형 네트워크").
    let legacy: Bool

    /// Shown when the bundled file pins nothing usable.
    static var missingPin: String { String(localized: "Updates are off: this copy of the app cannot check which updates the network approved.") }
    static let threshold = 2
    static let emergencyThreshold = 3

    static func parse(_ data: Data) -> ReleaseTrust? {
        guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let chainId = json["chain_id"] as? UInt64 else { return nil }
        guard let release = json["release"] else {
            if chainId == 7_777 || chainId == 7_780 {
                return ReleaseTrust(chainId: chainId, logAddress: "", codeHash: "", builderKeys: [], legacy: true)
            }
            return nil
        }
        func hex(_ value: Any?, bytes count: Int) -> String? {
            guard let text = value as? String, text.hasPrefix("0x"),
                  ReleaseApproval.bytes(String(text.dropFirst(2)))?.count == count else { return nil }
            return text
        }
        guard let pin = release as? [String: Any],
              let log = hex(pin["log"], bytes: 20),
              let codeHash = hex(pin["code_hash"], bytes: 32),
              ReleaseApproval.bytes(String(codeHash.dropFirst(2)))?.contains(where: { $0 != 0 }) == true,
              let keys = pin["builder_keys"] as? [String], keys.count == 3,
              Set(keys.map { $0.lowercased() }).count == 3,
              keys.allSatisfy({ key in
                  guard key.lowercased().hasPrefix("04"), let raw = ReleaseApproval.bytes(key), raw.count == 65 else { return false }
                  return (try? P256.Signing.PublicKey(x963Representation: raw)) != nil
              }),
              (pin["threshold"] as? Int) == threshold,
              (pin["emergency_threshold"] as? Int) == emergencyThreshold else { return nil }
        return ReleaseTrust(chainId: chainId, logAddress: log, codeHash: codeHash, builderKeys: keys, legacy: false)
    }
}

/// Pure installation policy. The caller must hash the actual archive bytes it
/// downloaded and bind Sparkle's appcast signature to the signed manifest.
enum ReleaseApproval {
    static let waitSeconds: UInt64 = 72 * 60 * 60

    static func digest(_ data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    static func bytes(_ string: String) -> Data? {
        guard string.count.isMultiple(of: 2) else { return nil }
        var value = Data()
        var i = string.startIndex
        while i < string.endIndex {
            let end = string.index(i, offsetBy: 2)
            guard let byte = UInt8(string[i..<end], radix: 16) else { return nil }
            value.append(byte)
            i = end
        }
        return value
    }

    /// Count only distinct pinned keys whose P-256 signature verifies over
    /// these exact manifest bytes. RPC approval counters are never consulted.
    static func approvedSigners(manifestData: Data, signaturesData: Data, pinnedKeys: [String]) -> Int? {
        guard manifestData.count <= 16_384, signaturesData.count <= 4_096,
              pinnedKeys.count == 3, Set(pinnedKeys.map { $0.lowercased() }).count == 3,
              let signatures = try? JSONDecoder().decode([ReleaseBuilderSignature].self, from: signaturesData),
              signatures.count <= 3 else { return nil }
        let allowed = Set(pinnedKeys.map { $0.lowercased() })
        var valid = Set<String>()
        for signed in signatures {
            let keyHex = signed.publicKey.lowercased()
            guard allowed.contains(keyHex), !valid.contains(keyHex),
                  let keyData = bytes(keyHex), keyData.count == 65,
                  let signatureData = bytes(signed.signature), signatureData.count == 64,
                  let key = try? P256.Signing.PublicKey(x963Representation: keyData),
                  let signature = try? P256.Signing.ECDSASignature(rawRepresentation: signatureData),
                  key.isValidSignature(signature, for: manifestData) else { return nil }
            valid.insert(keyHex)
        }
        return valid.count
    }

    static func decide(manifestData: Data, signaturesData: Data, archiveSha256: String,
                       version: String, build: String, sparkleSignature: String,
                       chainId: UInt64, logAddress: String, pinnedKeys: [String],
                       proof: ReleaseProof) -> ReleaseDecision {
        guard manifestData.count <= 16_384, signaturesData.count <= 4_096,
              let manifest = try? JSONDecoder().decode(ReleaseManifest.self, from: manifestData),
              manifest.chainId == chainId, manifest.logAddress.lowercased() == logAddress.lowercased(),
              manifest.platform == "macos-arm64-dmg", manifest.version == version,
              manifest.build == build, manifest.sparkleEdSignature == sparkleSignature,
              manifest.emergency == proof.emergency,
              digest(manifestData) == proof.manifestSha256.lowercased(),
              digest(signaturesData) == proof.signaturesSha256.lowercased(),
              archiveSha256.lowercased() == proof.archiveSha256.lowercased(),
              manifest.artifacts.count >= 1 && manifest.artifacts.count <= 16,
              Set(manifest.artifacts.map(\.name)).count == manifest.artifacts.count,
              manifest.artifacts.first(where: { $0.name == "EastSea.dmg" })?.sha256.lowercased() == archiveSha256.lowercased(),
              pinnedKeys.count == 3, Set(pinnedKeys.map { $0.lowercased() }).count == 3,
              proof.publishedBlock > 0, proof.publishedBlock <= proof.stateHeight,
              proof.publishedAt > 0, proof.certifiedTimestampMs / 1000 >= proof.publishedAt else {
            return .rejected
        }
        guard let valid = approvedSigners(manifestData: manifestData, signaturesData: signaturesData,
                                         pinnedKeys: pinnedKeys),
              valid >= (manifest.emergency ? 3 : 2) else { return .rejected }
        if manifest.emergency { return .ready }
        let readyAt = proof.publishedAt.addingReportingOverflow(waitSeconds)
        guard !readyAt.overflow else { return .rejected }
        if proof.certifiedTimestampMs / 1000 < readyAt.partialValue {
            return .pending(until: Date(timeIntervalSince1970: TimeInterval(readyAt.partialValue)))
        }
        return .ready
    }
}
#endif
