#if os(macOS)
import CryptoKit
import Foundation
#if canImport(Sparkle)
import Sparkle

/// Pins are read from the network.json inside the *currently running* signed
/// app. Replacing that file in a proposed update cannot relax this gate. The
/// parsing and the no-fallback rule live in `ReleaseTrust.parse` (pure).
extension ReleaseTrust {
    static let current = load()

    static func bundled() -> ReleaseTrust? { current }

    private static func load() -> ReleaseTrust? {
        guard let url = Bundle.main.url(forResource: "network", withExtension: "json"),
              let data = try? Data(contentsOf: url) else { return nil }
        return parse(data)
    }
}

struct PendingRelease {
    let version: String
    let build: String
    let fingerprint: String
    let publishedBlock: UInt64
    let availableAt: Date?
    let emergency: Bool
}
#endif

/// Sparkle's shouldProceed callback is synchronous. A first check starts a
/// background preflight and refuses this cycle. Once it succeeds, the app
/// starts another check; Sparkle then verifies the same EdDSA signature over
/// the archive it actually downloads. Different bytes cannot reuse that
/// EdDSA signature, even if the distribution server changes the URL contents.
final class ReleaseUpdateGate {
    /// Kept independent of Sparkle so the archive's file reader is tested directly.
    static func archiveSHA256(_ url: URL) throws -> String {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var hasher = SHA256()
        while try autoreleasepool(invoking: { () throws -> Bool in
            guard let part = try handle.read(upToCount: 1024 * 1024), !part.isEmpty else { return false }
            hasher.update(data: part)
            return true
        }) {}  // Keep each bridged NSData inside this iteration's pool.
        return Data(hasher.finalize()).map { String(format: "%02x", $0) }.joined()
    }

#if canImport(Sparkle)
    private let lock = NSLock()
    private var approvedUntil: [String: TimeInterval] = [:]
    private var checking = Set<String>()

    static func itemKey(_ item: SUAppcastItem, signature: String) -> String {
        "\(item.versionString)|\((item.displayVersionString as String?) ?? "")|\(signature)|\((item.fileURL as URL?)?.absoluteString ?? "")"
    }

    /// The update tracker's identity for an appcast item: the same notion of
    /// "this exact item" the gate itself uses (read-only; no gate behaviour).
    static func trackerKey(_ item: SUAppcastItem) -> String {
        itemKey(item, signature: sparkleSignature(item) ?? "")
    }

    func mayProceed(_ item: SUAppcastItem) -> Bool {
        guard let trust = ReleaseTrust.bundled() else { return false }
        if trust.legacy { return true }
        guard configuredChainId() == trust.chainId else { return false }
        guard let signature = Self.sparkleSignature(item) else { return false }
        lock.lock()
        defer { lock.unlock() }
        return (approvedUntil[Self.itemKey(item, signature: signature)] ?? 0) > ProcessInfo.processInfo.systemUptime
    }

    func inspect(_ item: SUAppcastItem, validators: UInt32,
                 finished: @escaping (PendingRelease?, String?, Bool) -> Void) {
        guard let trust = ReleaseTrust.bundled() else {
            finished(nil, ReleaseTrust.missingPin, false)
            return
        }
        if trust.legacy { return }
        guard configuredChainId() == trust.chainId else {
            finished(nil, String(localized: "This update is not approved by the network yet."), false)
            return
        }
        guard let signature = Self.sparkleSignature(item) else {
            finished(nil, String(localized: "This update is not approved by the network yet."), false)
            return
        }
        let key = Self.itemKey(item, signature: signature)
        lock.lock()
        let fresh = (approvedUntil[key] ?? 0) <= ProcessInfo.processInfo.systemUptime && checking.insert(key).inserted
        lock.unlock()
        guard fresh else { return }
        Task.detached { [weak self] in
            do {
                let result = try await Self.preflight(item, signature: signature, trust: trust, validators: validators)
                self?.lock.withLock {
                    if result.2 { self?.approvedUntil[key] = ProcessInfo.processInfo.systemUptime + 300 }
                    self?.checking.remove(key)
                }
                DispatchQueue.main.async { finished(result.0, result.1, result.2) }
            } catch {
                _ = self?.lock.withLock { self?.checking.remove(key) }
                DispatchQueue.main.async { finished(nil, String(localized: "This update is not approved by the network yet."), false) }
            }
        }
    }

    private static func preflight(_ item: SUAppcastItem, signature: String,
                                  trust: ReleaseTrust, validators: UInt32) async throws -> (PendingRelease?, String?, Bool) {
        guard validators > 0, let url = item.fileURL as URL?,
              let version = item.displayVersionString as String?, !version.isEmpty else {
            throw GateError.unavailable
        }
        let stem = "EastSea-\(version)"
        let base = url.deletingLastPathComponent()
        func fetch(_ suffix: String) async throws -> Data {
            let request = URLRequest(url: base.appendingPathComponent(stem + suffix), timeoutInterval: 30)
            let (data, response) = try await URLSession.shared.data(for: request)
            guard (response as? HTTPURLResponse)?.statusCode == 200, data.count <= 64_000 else { throw GateError.unavailable }
            return data
        }
        let manifest = try await fetch("-manifest.json")
        let signatures = try await fetch("-builder-sigs.json")
        let indexData = try await fetch("-release-index.json")
        guard let indexObject = try JSONSerialization.jsonObject(with: indexData) as? [String: Any],
              let index = indexObject["index"] as? UInt64 else { throw GateError.unavailable }
        let entry = try verifiedRelease(contract: trust.logAddress, codeHash: trust.codeHash,
            index: index, validators: validators)
        let proof = ReleaseProof(manifestSha256: entry.manifestSha256,
            archiveSha256: entry.archiveSha256, signaturesSha256: entry.signaturesSha256,
            publishedBlock: entry.publishedBlock, publishedAt: entry.publishedAt,
            emergency: entry.emergency, stateHeight: entry.stateHeight,
            certifiedTimestampMs: entry.certifiedTimestampMs)
        func decide(_ archiveHash: String) -> ReleaseDecision {
            ReleaseApproval.decide(manifestData: manifest, signaturesData: signatures,
            archiveSha256: archiveHash, version: version, build: item.versionString,
            sparkleSignature: signature, chainId: trust.chainId, logAddress: trust.logAddress,
            pinnedKeys: trust.builderKeys, proof: proof)
        }
        let readyAt = entry.publishedAt.addingReportingOverflow(ReleaseApproval.waitSeconds)
        let pending = PendingRelease(version: version, build: item.versionString,
            fingerprint: entry.archiveSha256, publishedBlock: entry.publishedBlock,
            availableAt: entry.emergency || readyAt.overflow ? nil : Date(timeIntervalSince1970: TimeInterval(readyAt.partialValue)),
            emergency: entry.emergency)
        switch decide(entry.archiveSha256) {
        case .pending: return (pending, String(localized: "This update is not approved by the network yet."), false)
        case .rejected: return (nil, String(localized: "This update is not approved by the network yet."), false)
        case .ready: break
        }
        // The URL is untrusted. Hash the archive before Sparkle's own download;
        // Sparkle then verifies the same EdDSA signature on the bytes it uses.
        let request = URLRequest(url: url, timeoutInterval: 120)
        let (download, response) = try await URLSession.shared.download(for: request)
        defer { try? FileManager.default.removeItem(at: download) }
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw GateError.unavailable }
        let archiveHash = try archiveSHA256(download)
        guard decide(archiveHash) == .ready else { return (nil, String(localized: "This update is not approved by the network yet."), false) }
        return (pending, nil, true)
    }

    /// Sparkle retains the parsed enclosure attributes in this dictionary.
    /// Unknown Sparkle formats fail closed on release-log networks.
    private static func sparkleSignature(_ item: SUAppcastItem) -> String? {
        func find(_ value: Any, depth: Int) -> String? {
            guard depth < 4, let fields = value as? [AnyHashable: Any] else { return nil }
            if let direct = fields["sparkle:edSignature"] as? String { return direct }
            for child in fields.values {
                if let found = find(child, depth: depth + 1) { return found }
            }
            return nil
        }
        return find(item.propertiesDictionary, depth: 0)
    }

    private enum GateError: Error { case unavailable }
#endif
}
#endif
