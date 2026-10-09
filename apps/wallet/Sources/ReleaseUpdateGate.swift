#if os(macOS)
import CryptoKit
import Foundation
import Network
import Sparkle

/// Pins come from the currently running signed bundle. A proposed update can
/// never replace these pins before it passes their approval policy.
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
    let installAfterHeight: UInt64
    let availableAt: Date?
    let emergency: Bool
}

struct PreparedChainRelease {
    let identity: String
    let item: SUAppcastItem
    let appcastURL: URL
}

/// Chain events choose the release, not a timed appcast check. The status entry
/// supplies discovery bytes only; the injected production verifier independently
/// checks the pinned runtime, storage proof and finality certificate through FFI.
final class ReleaseUpdateGate {
    typealias ProofVerifier = (ChainReleaseAnnouncement, ReleaseTrust, UInt32) throws -> ReleaseProof
    private struct Prepared {
        let value: PreparedChainRelease
        let release: VerifiedChainRelease
        let artifact: URL
        let artifactIdentity: ReleaseArtifact.FileIdentity
        let server: ReleaseFeedServer
    }

    private let lock = NSLock()
    private let trust: ReleaseTrust?
    private let chainId: () -> UInt64
    private let verify: ProofVerifier
    private let cacheDirectory: URL
    private let fetch: ReleaseArtifact.Fetch
    private var prepared: Prepared?
    private var requestedIdentity: String?
    private var preparation: Task<Void, Never>?

    init(trust: ReleaseTrust? = ReleaseTrust.bundled(),
         chainId: @escaping () -> UInt64 = configuredChainId,
         verify: @escaping ProofVerifier = ReleaseUpdateGate.verifiedProof,
         cacheDirectory: URL? = nil,
         fetch: @escaping ReleaseArtifact.Fetch = ReleaseArtifact.download) {
        self.trust = trust
        self.chainId = chainId
        self.verify = verify
        self.cacheDirectory = cacheDirectory ?? FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("com.pipln.eastsea/ReleaseArtifacts", isDirectory: true)
        self.fetch = fetch
    }

    deinit {
        preparation?.cancel()
        prepared?.server.shutdown()
    }

    static func itemKey(_ item: SUAppcastItem, signature: String) -> String {
        "\(item.versionString)|\((item.displayVersionString as String?) ?? "")|\(signature)|\((item.fileURL as URL?)?.absoluteString ?? "")"
    }

    static func trackerKey(_ item: SUAppcastItem) -> String {
        itemKey(item, signature: sparkleSignature(item) ?? "")
    }

    /// Cheap final barrier after asynchronous shutdown preparation. A newer
    /// announcement revokes the old item even if its earlier proof was valid.
    func hasPreparedApproval(for item: SUAppcastItem) -> Bool {
        guard let trust, chainId() == trust.chainId else { return false }
        if trust.legacy { return UpdateChannel.pollsForDiscovery(trust: trust) }
        return lock.withLock { prepared.map { Self.trackerKey(item) == Self.trackerKey($0.value.item) } ?? false }
    }

    func mayProceed(_ item: SUAppcastItem) -> Bool {
        guard let trust, chainId() == trust.chainId else { return false }
        if trust.legacy { return UpdateChannel.pollsForDiscovery(trust: trust) }
        guard let state = lock.withLock({ prepared }),
              Self.trackerKey(item) == Self.trackerKey(state.value.item) else { return false }
        return ReleaseArtifact.identity(state.artifact) == state.artifactIdentity && hasPreparedApproval(for: item)
    }

    /// Compatibility for the existing Sparkle delegate. A remote appcast can
    /// never create approval on a release-log network. Only prepare can do so.
    func inspect(_ item: SUAppcastItem, validators: UInt32,
                 finished: @escaping (PendingRelease?, String?, Bool) -> Void) {
        guard let trust else { finished(nil, ReleaseTrust.missingPin, false); return }
        if trust.legacy { return }
        guard mayProceed(item), let state = lock.withLock({ prepared }) else {
            finished(nil, ChainReleaseFailure.forgedEntry.sentence, false)
            return
        }
        finished(Self.pending(state.release.announcement, proof: state.release.proof), nil, true)
    }

    func prepare(announcement: ChainReleaseAnnouncement, validators: UInt32,
                 finished: @escaping (PreparedChainRelease?, PendingRelease?, String?) -> Void) {
        guard let trust else { finished(nil, nil, ReleaseTrust.missingPin); return }
        guard !trust.legacy, chainId() == trust.chainId, validators > 0,
              announcement.chainId == trust.chainId,
              announcement.logAddress.lowercased() == trust.logAddress.lowercased() else {
            finished(nil, nil, ChainReleaseFailure.forgedEntry.sentence)
            return
        }
        let identity = announcement.identity
        let old = lock.withLock { () -> Prepared? in
            let old = prepared
            if requestedIdentity != identity {
                preparation?.cancel()
                prepared = nil
                requestedIdentity = identity
            }
            return old?.value.identity != identity ? old : nil
        }
        old?.server.shutdown()
        let task = Task.detached { [weak self] in
            guard let self else { return }
            var proof: ReleaseProof?
            var server: ReleaseFeedServer?
            do {
                let freshProof = try self.verify(announcement, trust, validators)
                proof = freshProof
                let release = try ChainReleasePolicy.verify(announcement: announcement, trust: trust, proof: freshProof)
                let artifact = try await ReleaseArtifact.acquire(release, cacheDirectory: self.cacheDirectory, fetch: self.fetch)
                try Task.checkCancellation()
                guard let artifactIdentity = ReleaseArtifact.identity(artifact) else { throw ChainReleaseFailure.hashMismatch }
                let origin = try ReleaseFeedServer(release: release, artifact: artifact)
                server = origin
                let feedURL = try await origin.start()
                guard let artifactURL = origin.artifactURL,
                      let size = (try artifact.resourceValues(forKeys: [.fileSizeKey])).fileSize,
                      let item = Self.makeItem(release: release, artifactURL: artifactURL, length: UInt64(size)) else {
                    throw ChainReleaseFailure.unavailable
                }
                let value = PreparedChainRelease(identity: identity, item: item, appcastURL: feedURL)
                let accepted = self.lock.withLock { () -> Bool in
                    guard self.requestedIdentity == identity, !Task.isCancelled,
                          self.chainId() == trust.chainId else { return false }
                    self.prepared?.server.shutdown()
                    self.prepared = Prepared(value: value, release: release, artifact: artifact,
                        artifactIdentity: artifactIdentity, server: origin)
                    return true
                }
                guard accepted else { throw CancellationError() }
                DispatchQueue.main.async { finished(value, Self.pending(announcement, proof: freshProof), nil) }
            } catch is CancellationError {
                server?.shutdown()
                DispatchQueue.main.async { finished(nil, nil, nil) }
            } catch {
                server?.shutdown()
                let failure = Self.failure(error)
                let pending = failure == .beforeSlot ? proof.map { Self.pending(announcement, proof: $0) } : nil
                DispatchQueue.main.async { finished(nil, pending, failure.sentence) }
            }
        }
        lock.withLock { preparation = task }
    }

    /// Run immediately before the existing storage/vote/quiesce/rollback
    /// shutdown path. A fresh independent certificate must still cover the
    /// barrier; rehashing the cached bytes prevents a poisoned second download.
    func validateForInstall(item: SUAppcastItem, validators: UInt32) async -> String? {
        guard let trust else { return ReleaseTrust.missingPin }
        guard chainId() == trust.chainId else { return ChainReleaseFailure.forgedEntry.sentence }
        if trust.legacy { return UpdateChannel.pollsForDiscovery(trust: trust) ? nil : ChainReleaseFailure.forgedEntry.sentence }
        guard validators > 0, let state = lock.withLock({ prepared }),
              Self.trackerKey(item) == Self.trackerKey(state.value.item) else {
            return ChainReleaseFailure.forgedEntry.sentence
        }
        let result = await Task.detached { [verify] () -> ChainReleaseFailure? in
            do {
                let proof = try verify(state.release.announcement, trust, validators)
                _ = try ChainReleasePolicy.verify(announcement: state.release.announcement, trust: trust, proof: proof)
                guard ReleaseArtifact.valid(state.artifact, release: state.release) else { return .hashMismatch }
                return nil
            } catch { return Self.failure(error) }
        }.value
        guard chainId() == trust.chainId,
              lock.withLock({ prepared?.value.identity == state.value.identity }) else {
            return ChainReleaseFailure.forgedEntry.sentence
        }
        return result?.sentence
    }

    private static func failure(_ error: Error) -> ChainReleaseFailure {
        if let failure = error as? ChainReleaseFailure { return failure }
        if let walletError = error as? WalletError, case .Network = walletError { return .unavailable }
        if error is URLError { return .unavailable }
        return .forgedEntry
    }

    private static func verifiedProof(_ announcement: ChainReleaseAnnouncement, _ trust: ReleaseTrust,
                                      _ validators: UInt32) throws -> ReleaseProof {
        let entry = try verifiedRelease(contract: trust.logAddress, codeHash: trust.codeHash,
                                        index: announcement.index, validators: validators)
        return ReleaseProof(manifestSha256: entry.manifestSha256, archiveSha256: entry.archiveSha256,
            signaturesSha256: entry.signaturesSha256, publishedBlock: entry.publishedBlock,
            publishedAt: entry.publishedAt, emergency: entry.emergency, stateHeight: entry.stateHeight,
            certifiedTimestampMs: entry.certifiedTimestampMs, certifiedBlock: entry.certifiedBlock)
    }

    private static func pending(_ announcement: ChainReleaseAnnouncement, proof: ReleaseProof) -> PendingRelease {
        let until = proof.publishedAt.addingReportingOverflow(ReleaseApproval.waitSeconds)
        let manifest = try? JSONDecoder().decode(ReleaseManifest.self, from: announcement.manifestData)
        let height = manifest.flatMap { try? ChainReleasePolicy.installAfterHeight(proof: proof, manifest: $0) }
        return PendingRelease(version: announcement.version, build: announcement.build,
            fingerprint: proof.archiveSha256, publishedBlock: proof.publishedBlock,
            installAfterHeight: height ?? UInt64.max,
            availableAt: until.overflow ? nil : Date(timeIntervalSince1970: TimeInterval(until.partialValue)),
            emergency: proof.emergency)
    }

    private static func makeItem(release: VerifiedChainRelease, artifactURL: URL, length: UInt64) -> SUAppcastItem? {
        var fields: [String: Any] = ["title": String(localized: "EastSea \(release.manifest.version)"),
            "enclosure": ["url": artifactURL.absoluteString, "length": String(length),
                "type": "application/octet-stream", "sparkle:version": release.manifest.build,
                "sparkle:shortVersionString": release.manifest.version,
                "sparkle:edSignature": release.manifest.sparkleEdSignature]]
        if let channel = release.manifest.channel { fields["sparkle:channel"] = channel }
        return SUAppcastItem(dictionary: fields)
    }

    private static func sparkleSignature(_ item: SUAppcastItem) -> String? {
        func find(_ value: Any, depth: Int) -> String? {
            guard depth < 4, let fields = value as? [AnyHashable: Any] else { return nil }
            if let direct = fields["sparkle:edSignature"] as? String { return direct }
            for child in fields.values { if let found = find(child, depth: depth + 1) { return found } }
            return nil
        }
        return find(item.propertiesDictionary, depth: 0)
    }
}

/// Sparkle's public downloader rejects file URLs. Serve the manifest-derived
/// feed and the verified cache entry through one random, loopback-only origin.
/// Requests never become filesystem paths; only two exact routes exist.
private final class ReleaseFeedServer: @unchecked Sendable {
    private let queue = DispatchQueue(label: "com.pipln.eastsea.release-feed")
    private let listener: NWListener
    private let release: VerifiedChainRelease
    private let artifact: URL
    private let token = UUID().uuidString
    private var continuation: CheckedContinuation<URL, Error>?
    private var connections: [UUID: NWConnection] = [:]
    private var pendingHeaders = Set<UUID>()
    private var feed = Data()
    private(set) var artifactURL: URL?

    init(release: VerifiedChainRelease, artifact: URL) throws {
        self.release = release
        self.artifact = artifact
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
        self.listener = try NWListener(using: parameters)
    }

    deinit { listener.cancel() }

    func start() async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            listener.stateUpdateHandler = { [weak self] state in
                guard let self else { return }
                switch state {
                case .ready:
                    do {
                        guard let port = self.listener.port,
                              let origin = URL(string: "http://127.0.0.1:\(port.rawValue)/\(self.token)/"),
                              let length = try self.artifact.resourceValues(forKeys: [.fileSizeKey]).fileSize else {
                            throw ChainReleaseFailure.unavailable
                        }
                        let archive = origin.appendingPathComponent(self.release.artifact.sha256.lowercased())
                            .appendingPathComponent("EastSea.dmg")
                        self.artifactURL = archive
                        self.feed = try ChainReleasePolicy.appcastXML(release: self.release, artifactURL: archive, length: UInt64(length))
                        self.continuation?.resume(returning: origin.appendingPathComponent("appcast.xml"))
                        self.continuation = nil
                    } catch {
                        self.continuation?.resume(throwing: error)
                        self.continuation = nil
                        self.listener.cancel()
                    }
                case .failed(let error):
                    self.continuation?.resume(throwing: error)
                    self.continuation = nil
                case .cancelled:
                    self.continuation?.resume(throwing: CancellationError())
                    self.continuation = nil
                default: break
                }
            }
            listener.newConnectionHandler = { [weak self] connection in self?.accept(connection) }
            listener.start(queue: queue)
        }
    }

    func shutdown() {
        queue.async { [self] in
            listener.cancel()
            for connection in connections.values { connection.cancel() }
            connections.removeAll()
            pendingHeaders.removeAll()
        }
    }

    private func accept(_ connection: NWConnection) {
        guard connections.count < 8 else { connection.cancel(); return }
        let id = UUID()
        connections[id] = connection
        pendingHeaders.insert(id)
        connection.stateUpdateHandler = { [weak self] state in
            switch state {
            case .failed, .cancelled:
                self?.connections.removeValue(forKey: id)
                self?.pendingHeaders.remove(id)
            default: break
            }
        }
        connection.start(queue: queue)
        queue.asyncAfter(deadline: .now() + 10) { [weak self, weak connection] in
            guard let self, self.pendingHeaders.contains(id), let connection else { return }
            connection.cancel()
        }
        receiveHeader(connection, id: id, buffer: Data())
    }

    private func receiveHeader(_ connection: NWConnection, id: UUID, buffer: Data) {
        guard buffer.count < 8_192 else { connection.cancel(); return }
        connection.receive(minimumIncompleteLength: 1, maximumLength: 8_192 - buffer.count) { [weak self] data, _, complete, error in
            guard let self, error == nil, let data, !data.isEmpty else { connection.cancel(); return }
            var accumulated = buffer
            accumulated.append(data)
            guard let delimiter = accumulated.range(of: Data("\r\n\r\n".utf8)) else {
                if complete { connection.cancel() }
                else { self.receiveHeader(connection, id: id, buffer: accumulated) }
                return
            }
            self.pendingHeaders.remove(id)
            guard delimiter.upperBound == accumulated.endIndex,
                  let header = String(data: accumulated, encoding: .utf8),
                  let line = header.components(separatedBy: "\r\n").first else { connection.cancel(); return }
            let request = line.split(separator: " ")
            guard request.count == 3, request[0] == "GET" || request[0] == "HEAD",
                  request[2] == "HTTP/1.1" || request[2] == "HTTP/1.0" else { connection.cancel(); return }
            let path = String(request[1].split(separator: "?", maxSplits: 1, omittingEmptySubsequences: false)[0])
            guard path.hasPrefix("/") else { connection.cancel(); return }
            let head = request[0] == "HEAD"
            if path == "/\(self.token)/appcast.xml" {
                self.send(connection, bytes: self.feed, contentType: "application/xml", head: head)
            } else if path == self.artifactURL?.path {
                self.sendArtifact(connection, head: head)
            } else {
                self.send(connection, bytes: Data(), contentType: "text/plain", head: true, status: "404 Not Found")
            }
        }
    }

    private func send(_ connection: NWConnection, bytes: Data, contentType: String, head: Bool,
                      status: String = "200 OK") {
        var response = Data("HTTP/1.1 \(status)\r\nContent-Type: \(contentType)\r\nContent-Length: \(bytes.count)\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n".utf8)
        if !head { response.append(bytes) }
        connection.send(content: response, completion: .contentProcessed { _ in connection.cancel() })
    }

    private func sendArtifact(_ connection: NWConnection, head: Bool) {
        guard ReleaseArtifact.valid(artifact, release: release),
              let length = try? artifact.resourceValues(forKeys: [.fileSizeKey]).fileSize,
              let handle = try? FileHandle(forReadingFrom: artifact) else { connection.cancel(); return }
        let header = Data("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: \(length)\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n".utf8)
        connection.send(content: header, completion: .contentProcessed { [weak self] error in
            guard error == nil, !head, let self else { try? handle.close(); connection.cancel(); return }
            self.sendChunk(connection, handle: handle)
        })
    }

    private func sendChunk(_ connection: NWConnection, handle: FileHandle) {
        guard let part = try? handle.read(upToCount: 256 * 1_024), !part.isEmpty else {
            try? handle.close()
            connection.cancel()
            return
        }
        connection.send(content: part, completion: .contentProcessed { [weak self] error in
            guard error == nil, let self else { try? handle.close(); connection.cancel(); return }
            self.sendChunk(connection, handle: handle)
        })
    }
}
#endif
