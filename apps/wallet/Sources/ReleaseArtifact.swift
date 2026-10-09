#if os(macOS)
import CryptoKit
import Foundation

/// A source supplies bytes; only the verified manifest decides their identity.
/// The cache root is explicit so fixtures never read or write the wallet's data.
enum ReleaseArtifact {
    typealias Fetch = (URL, URL) async throws -> Void

    struct FileIdentity: Equatable {
        let device: UInt64
        let inode: UInt64
        let size: UInt64
        let modified: Date
    }

    /// A cheap change check for Sparkle's synchronous callback. Full hashing
    /// stays in asynchronous preparation and immediately before installation.
    static func identity(_ file: URL) -> FileIdentity? {
        guard let info = try? FileManager.default.attributesOfItem(atPath: file.path),
              info[.type] as? FileAttributeType == .typeRegular,
              let device = info[.systemNumber] as? NSNumber,
              let inode = info[.systemFileNumber] as? NSNumber,
              let size = info[.size] as? NSNumber,
              let modified = info[.modificationDate] as? Date else { return nil }
        return FileIdentity(device: device.uint64Value, inode: inode.uint64Value,
            size: size.uint64Value, modified: modified)
    }

    static func acquire(_ release: VerifiedChainRelease, cacheDirectory: URL,
                        fetch: Fetch = download) async throws -> URL {
        let manager = FileManager.default
        guard cacheDirectory.isFileURL else { throw ChainReleaseFailure.unavailable }
        let root = cacheDirectory.standardizedFileURL
        try createPrivateDirectory(root)
        let hash = release.artifact.sha256.lowercased()
        guard hash.count == 64, ReleaseApproval.bytes(hash)?.count == 32 else {
            throw ChainReleaseFailure.forgedEntry
        }
        let directory = root.appendingPathComponent(hash, isDirectory: true)
        try createPrivateDirectory(directory)
        let cached = directory.appendingPathComponent("EastSea.dmg")
        if valid(cached, release: release) { return cached }
        if manager.fileExists(atPath: cached.path) { try manager.removeItem(at: cached) }
        var mismatched = false
        for source in release.artifactSources {
            try Task.checkCancellation()
            let partial = directory.appendingPathComponent(UUID().uuidString + ".part")
            defer { try? manager.removeItem(at: partial) }
            do {
                try await fetch(source, partial)
                guard valid(partial, release: release) else {
                    mismatched = true
                    continue
                }
                // Sparkle is served this immutable cache entry, never a second
                // request to the origin which could now return different bytes.
                try manager.setAttributes([.posixPermissions: 0o400], ofItemAtPath: partial.path)
                try manager.moveItem(at: partial, to: cached)
                return cached
            } catch is CancellationError { throw CancellationError() }
            catch let failure as ChainReleaseFailure {
                if failure == .hashMismatch { mismatched = true }
            } catch { continue }
        }
        throw mismatched ? ChainReleaseFailure.hashMismatch : ChainReleaseFailure.unavailable
    }

    static func valid(_ file: URL, release: VerifiedChainRelease) -> Bool {
        guard let info = try? file.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey]),
              info.isRegularFile == true, info.isSymbolicLink != true,
              let size = info.fileSize, size > 0, UInt64(size) <= ChainReleasePolicy.maximumArchiveSize,
              release.artifact.size == nil || release.artifact.size == UInt64(size),
              let actual = try? digest(file: file) else { return false }
        return actual == release.artifact.sha256.lowercased()
    }

    static func digest(file: URL) throws -> String {
        let handle = try FileHandle(forReadingFrom: file)
        defer { try? handle.close() }
        var hasher = SHA256()
        var size: UInt64 = 0
        while try autoreleasepool(invoking: { () throws -> Bool in
            guard let part = try handle.read(upToCount: 1_024 * 1_024), !part.isEmpty else { return false }
            size += UInt64(part.count)
            guard size <= ChainReleasePolicy.maximumArchiveSize else { throw ChainReleaseFailure.hashMismatch }
            hasher.update(data: part)
            return true
        }) {} // Release bridged NSData within each iteration, including errors.
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }

    private static func createPrivateDirectory(_ directory: URL) throws {
        let manager = FileManager.default
        if manager.fileExists(atPath: directory.path) {
            let info = try directory.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
            guard info.isDirectory == true, info.isSymbolicLink != true else { throw ChainReleaseFailure.unavailable }
        } else {
            try manager.createDirectory(at: directory, withIntermediateDirectories: true,
                                        attributes: [.posixPermissions: 0o700])
        }
    }

    /// URLSession's downloadTask writes first into a system temporary directory.
    /// Stream with a data task instead, so even partial artifacts stay inside
    /// the explicit cache root and oversized sources are cancelled early.
    static func download(source: URL, destination: URL) async throws {
        let writer = ArtifactDownload(destination: destination)
        try await writer.run(source)
    }
}

private final class ArtifactDownload: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    private let destination: URL
    private var handle: FileHandle?
    private var continuation: CheckedContinuation<Void, Error>?
    private var session: URLSession?
    private var failure: Error?
    private var count: UInt64 = 0

    init(destination: URL) { self.destination = destination }

    func run(_ source: URL) async throws {
        guard FileManager.default.createFile(atPath: destination.path, contents: nil,
                                              attributes: [.posixPermissions: 0o600]) else {
            throw ChainReleaseFailure.unavailable
        }
        handle = try FileHandle(forWritingTo: destination)
        try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            let configuration = URLSessionConfiguration.ephemeral
            configuration.timeoutIntervalForRequest = 30
            configuration.timeoutIntervalForResource = 600
            let queue = OperationQueue()
            queue.maxConcurrentOperationCount = 1
            let session = URLSession(configuration: configuration, delegate: self, delegateQueue: queue)
            self.session = session
            var request = URLRequest(url: source)
            request.setValue("identity", forHTTPHeaderField: "Accept-Encoding")
            session.dataTask(with: request).resume()
        }
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        guard (response as? HTTPURLResponse)?.statusCode == 200,
              response.expectedContentLength <= Int64(ChainReleasePolicy.maximumArchiveSize) else {
            failure = ChainReleaseFailure.unavailable
            completionHandler(.cancel)
            return
        }
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        count += UInt64(data.count)
        do {
            guard count <= ChainReleasePolicy.maximumArchiveSize else { throw ChainReleaseFailure.hashMismatch }
            try handle?.write(contentsOf: data)
        } catch {
            failure = error
            dataTask.cancel()
        }
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        do { try handle?.close() } catch { if failure == nil { failure = error } }
        handle = nil
        let result = failure ?? error
        if let result { continuation?.resume(throwing: result) }
        else { continuation?.resume() }
        continuation = nil
        self.session = nil
        session.finishTasksAndInvalidate()
    }
}
#endif
