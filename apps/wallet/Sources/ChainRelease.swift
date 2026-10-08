#if os(macOS)
import Foundation

/// Discovery data from aether_status.release. Even its approval counters and
/// heights are untrusted until bound to a separately verified ReleaseLog proof.
struct ChainReleaseAnnouncement: Decodable {
    let index: UInt64
    let chainId: UInt64
    let logAddress: String
    let version: String
    let build: String
    let manifestHash: String
    let archiveSha256: String
    let signaturesHash: String
    let manifest: String
    let signatures: String
    let approvedAtHeight: UInt64
    let installAfterHeight: UInt64
    let restartSlotHeight: UInt64?

    enum CodingKeys: String, CodingKey {
        case index, version, build, manifest, signatures
        case chainId = "chain_id", logAddress = "log_address"
        case manifestHash = "manifest_hash", archiveSha256 = "archive_sha256"
        case signaturesHash = "signatures_hash", approvedAtHeight = "approved_at_height"
        case installAfterHeight = "install_after_height", restartSlotHeight = "restart_slot_height"
    }

    var manifestData: Data { Data(manifest.utf8) }
    var signaturesData: Data { Data(signatures.utf8) }
    var identity: String {
        "\(chainId)|\(logAddress.lowercased())|\(index)|\(manifestHash.lowercased())|\(archiveSha256.lowercased())|\(signaturesHash.lowercased())|\(installAfterHeight)|\(restartSlotHeight ?? 0)"
    }

    static func parse(json: String) -> ChainReleaseAnnouncement? {
        guard json.utf8.count <= 128 * 1_024,
              let value = try? JSONDecoder().decode(Self.self, from: Data(json.utf8)),
              value.manifestData.count <= 16_384, value.signaturesData.count <= 4_096,
              value.version.utf8.count <= 96, value.build.utf8.count <= 32,
              value.logAddress.utf8.count == 42,
              [value.manifestHash, value.archiveSha256, value.signaturesHash].allSatisfy(validHash)
        else { return nil }
        return value
    }

    private static func validHash(_ value: String) -> Bool {
        value.utf8.count == 64 && ReleaseApproval.bytes(value)?.count == 32
    }
}

enum ChainReleaseFailure: Error, Equatable {
    case forgedEntry, insufficientApprovals, hashMismatch, beforeSlot, unavailable

    var sentence: String {
        switch self {
        case .forgedEntry:
            return String(localized: "This update does not match the release approved by the network.")
        case .insufficientApprovals:
            return String(localized: "This update does not have the required builder approvals.")
        case .hashMismatch:
            return String(localized: "The downloaded update does not match the approved file.")
        case .beforeSlot:
            return String(localized: "This update cannot install before its approved restart slot.")
        case .unavailable:
            return String(localized: "The approved update could not be downloaded yet.")
        }
    }
}

struct VerifiedChainRelease {
    let announcement: ChainReleaseAnnouncement
    let manifest: ReleaseManifest
    let artifact: ReleaseManifest.Artifact
    let artifactSources: [URL]
    let proof: ReleaseProof
    let installAfterHeight: UInt64
}

enum ChainReleasePolicy {
    static let maximumArchiveSize: UInt64 = 2 * 1_024 * 1_024 * 1_024

    static func verify(announcement: ChainReleaseAnnouncement, trust: ReleaseTrust,
                       proof: ReleaseProof) throws -> VerifiedChainRelease {
        let next = proof.stateHeight.addingReportingOverflow(1)
        guard !trust.legacy, announcement.chainId == trust.chainId,
              announcement.logAddress.lowercased() == trust.logAddress.lowercased(),
              proof.publishedBlock > 0, proof.publishedBlock <= proof.stateHeight,
              !next.overflow, proof.certifiedBlock == next.partialValue,
              proof.publishedAt > 0, proof.certifiedTimestampMs / 1_000 >= proof.publishedAt,
              announcement.approvedAtHeight == proof.publishedBlock,
              announcement.manifestHash.lowercased() == proof.manifestSha256.lowercased(),
              announcement.archiveSha256.lowercased() == proof.archiveSha256.lowercased(),
              announcement.signaturesHash.lowercased() == proof.signaturesSha256.lowercased(),
              ReleaseApproval.digest(announcement.manifestData) == proof.manifestSha256.lowercased(),
              ReleaseApproval.digest(announcement.signaturesData) == proof.signaturesSha256.lowercased(),
              let manifest = try? JSONDecoder().decode(ReleaseManifest.self, from: announcement.manifestData),
              manifest.chainId == trust.chainId, manifest.logAddress.lowercased() == trust.logAddress.lowercased(),
              manifest.emergency == proof.emergency, manifest.platform == "macos-arm64-dmg",
              manifest.version == announcement.version, manifest.build == announcement.build,
              validVersion(manifest.version), !manifest.build.isEmpty, manifest.build.utf8.count <= 32,
              manifest.build.allSatisfy({ $0.isASCII && $0.isNumber }),
              Data(base64Encoded: manifest.sparkleEdSignature)?.count == 64,
              !manifest.artifacts.isEmpty, manifest.artifacts.count <= 16,
              Set(manifest.artifacts.map(\.name)).count == manifest.artifacts.count,
              manifest.artifacts.allSatisfy({ $0.name.utf8.count <= 128 && !$0.name.isEmpty
                  && $0.sha256.count == 64 && ReleaseApproval.bytes($0.sha256)?.count == 32 }),
              let artifact = manifest.artifacts.first(where: { $0.name == "EastSea.dmg" }),
              artifact.sha256.lowercased() == proof.archiveSha256.lowercased(),
              artifact.size == nil || (artifact.size! > 0 && artifact.size! <= maximumArchiveSize),
              manifest.channel == nil || manifest.channel == UpdateChannel.canary else {
            throw ChainReleaseFailure.forgedEntry
        }
        let sources = try artifactSources(artifact, version: manifest.version)
        guard let count = ReleaseApproval.approvedSigners(manifestData: announcement.manifestData,
            signaturesData: announcement.signaturesData, pinnedKeys: trust.builderKeys),
              count >= (manifest.emergency ? ReleaseTrust.emergencyThreshold : ReleaseTrust.threshold) else {
            throw ChainReleaseFailure.insufficientApprovals
        }
        // An unauthenticated status response cannot shorten either wait.
        // New chain slots use one-second blocks, including emergency releases.
        let minimumHeight = proof.publishedBlock.addingReportingOverflow(ReleaseApproval.waitSeconds)
        let minimumTime = proof.publishedAt.addingReportingOverflow(ReleaseApproval.waitSeconds)
        guard !minimumHeight.overflow, !minimumTime.overflow else { throw ChainReleaseFailure.forgedEntry }
        let barrier = max(max(minimumHeight.partialValue, announcement.installAfterHeight),
            max(announcement.restartSlotHeight ?? 0, max(manifest.installAfterHeight ?? 0, manifest.restartSlotHeight ?? 0)))
        guard proof.stateHeight >= barrier, proof.certifiedBlock > barrier,
              proof.certifiedTimestampMs / 1_000 >= minimumTime.partialValue else {
            throw ChainReleaseFailure.beforeSlot
        }
        return VerifiedChainRelease(announcement: announcement, manifest: manifest, artifact: artifact,
            artifactSources: sources, proof: proof, installAfterHeight: barrier)
    }

    static func artifactSources(_ artifact: ReleaseManifest.Artifact, version: String) throws -> [URL] {
        guard (artifact.webseeds?.count ?? 0) <= 8, (artifact.peers?.count ?? 0) <= 8 else {
            throw ChainReleaseFailure.forgedEntry
        }
        let primary = artifact.url ?? "https://github.com/eastsea-xyz/eastsea/releases/download/app-v\(version)/EastSea-\(version).dmg"
        let candidates = [primary] + (artifact.webseeds ?? []) + (artifact.peers ?? [])
        var seen = Set<String>()
        return try candidates.compactMap { value in
            guard value.utf8.count <= 2_048, let parts = URLComponents(string: value),
                  let scheme = parts.scheme?.lowercased(), scheme == "https" || scheme == "http",
                  let host = parts.host, !host.isEmpty, parts.user == nil, parts.password == nil,
                  parts.fragment == nil, let url = parts.url else { throw ChainReleaseFailure.forgedEntry }
            return seen.insert(url.absoluteString).inserted ? url : nil
        }
    }

    static func appcastXML(release: VerifiedChainRelease, artifactURL: URL, length: UInt64) throws -> Data {
        guard artifactURL.host == "127.0.0.1", artifactURL.scheme == "http",
              length > 0, length <= maximumArchiveSize else { throw ChainReleaseFailure.forgedEntry }
        let manifest = release.manifest
        let channel = manifest.channel.map { "<sparkle:channel>\(escape($0))</sparkle:channel>" } ?? ""
        return Data("""
        <?xml version="1.0" encoding="utf-8"?>
        <rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel>
        <title>EastSea</title><item><title>EastSea \(escape(manifest.version))</title>\(channel)
        <enclosure url="\(escape(artifactURL.absoluteString))" sparkle:version="\(escape(manifest.build))" sparkle:shortVersionString="\(escape(manifest.version))" sparkle:edSignature="\(escape(manifest.sparkleEdSignature))" length="\(length)" type="application/octet-stream"/>
        </item></channel></rss>
        """.utf8)
    }

    private static func validVersion(_ value: String) -> Bool {
        value.utf8.count <= 96 && value.range(of: "^[0-9]+\\.[0-9]+\\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\\+[0-9A-Za-z.-]+)?$", options: .regularExpression) != nil
    }

    private static func escape(_ value: String) -> String {
        value.replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "\"", with: "&quot;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
            .replacingOccurrences(of: "'", with: "&apos;")
    }
}
#endif
