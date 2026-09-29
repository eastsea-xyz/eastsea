#if os(macOS)
import CryptoKit
import Foundation

struct ReleaseManifest: Decodable {
    struct Artifact: Decodable {
        let name: String
        let sha256: String
    }
    let artifacts: [Artifact]
    let build: String
    let chainId: UInt64
    let emergency: Bool
    let logAddress: String
    let platform: String
    let sparkleEdSignature: String
    let version: String

    enum CodingKeys: String, CodingKey {
        case artifacts, build, emergency, platform, version
        case chainId = "chain_id", logAddress = "log_address"
        case sparkleEdSignature = "sparkle_ed_signature"
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
    let stateHeight: UInt64
    let certifiedTimestampMs: UInt64
}

enum ReleaseDecision: Equatable {
    case ready
    case pending(until: Date)
    case rejected
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

    static func decide(manifestData: Data, signaturesData: Data, archiveSha256: String,
                       version: String, build: String, sparkleSignature: String,
                       chainId: UInt64, logAddress: String, pinnedKeys: [String],
                       proof: ReleaseProof) -> ReleaseDecision {
        guard manifestData.count <= 16_384, signaturesData.count <= 4_096,
              let manifest = try? JSONDecoder().decode(ReleaseManifest.self, from: manifestData),
              let signatures = try? JSONDecoder().decode([ReleaseBuilderSignature].self, from: signaturesData),
              manifest.chainId == chainId, manifest.logAddress.lowercased() == logAddress.lowercased(),
              manifest.platform == "macos-arm64-dmg", manifest.version == version,
              manifest.build == build, manifest.sparkleEdSignature == sparkleSignature,
              manifest.emergency == proof.emergency,
              digest(manifestData) == proof.manifestSha256.lowercased(),
              digest(signaturesData) == proof.signaturesSha256.lowercased(),
              archiveSha256.lowercased() == proof.archiveSha256.lowercased(),
              manifest.artifacts.count >= 1 && manifest.artifacts.count <= 16,
              Set(manifest.artifacts.map(\.name)).count == manifest.artifacts.count,
              manifest.artifacts.first(where: { $0.name == "Aether.dmg" })?.sha256.lowercased() == archiveSha256.lowercased(),
              pinnedKeys.count == 3, Set(pinnedKeys.map { $0.lowercased() }).count == 3,
              proof.publishedBlock > 0, proof.publishedBlock <= proof.stateHeight,
              proof.publishedAt > 0, proof.certifiedTimestampMs / 1000 >= proof.publishedAt else {
            return .rejected
        }
        let allowed = Set(pinnedKeys.map { $0.lowercased() })
        var valid = Set<String>()
        for signed in signatures {
            let keyHex = signed.publicKey.lowercased()
            guard allowed.contains(keyHex), !valid.contains(keyHex),
                  let keyData = bytes(keyHex), keyData.count == 65,
                  let signatureData = bytes(signed.signature), signatureData.count == 64,
                  let key = try? P256.Signing.PublicKey(x963Representation: keyData),
                  let signature = try? P256.Signing.ECDSASignature(rawRepresentation: signatureData),
                  key.isValidSignature(signature, for: manifestData) else { return .rejected }
            valid.insert(keyHex)
        }
        guard valid.count >= (manifest.emergency ? 3 : 2) else { return .rejected }
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
