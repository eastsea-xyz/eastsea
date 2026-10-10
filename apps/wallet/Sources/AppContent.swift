import Foundation
import CryptoKit
import Darwin

enum AppContentError: Error, LocalizedError, Equatable {
    case invalidHash
    case invalidIndex
    case invalidPath(String)
    case unsupportedFile(String)
    case tooLarge
    case hashMismatch(String)
    case missingFile(String)
    case invalidResponse
    case nodeUnavailable
    case developerModeRequired
    case unsafeLocalFile(String)
    case unverifiedContent

    var errorDescription: String? {
        switch self {
        case .invalidHash, .invalidIndex:
            return String(localized: "This app's bundle index is invalid. The app was not opened.")
        case .invalidPath, .unsupportedFile, .unsafeLocalFile:
            return String(localized: "This app contains an unsupported file or an unsafe path. The app was not opened.")
        case .tooLarge:
            return String(localized: "This app exceeds the bundle size limit. The app was not opened.")
        case .hashMismatch:
            return String(localized: "The received files differ from the registered app. The app was not opened.")
        case .missingFile:
            return String(localized: "A required app file is missing. The app was not opened.")
        case .invalidResponse:
            return String(localized: "The node returned an invalid app bundle. The app was not opened.")
        case .nodeUnavailable:
            return String(localized: "The app could not be fetched from the node. Check that this Mac's node is running and connected.")
        case .developerModeRequired:
            return String(localized: "Turn on developer mode to open a local app folder.")
        case .unverifiedContent:
            return String(localized: "This app has not been verified. The app was not opened.")
        }
    }
}

enum AppBundleHash {
    static func hex(_ data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    static func normalized(_ value: String) -> String? {
        let hex = value.hasPrefix("0x") ? String(value.dropFirst(2)) : value
        guard hex.utf8.count == 64, hex.utf8.allSatisfy({
            (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0)
        }) else { return nil }
        return hex.lowercased()
    }
}

/// The same strict path language is used for indexes, folder snapshots and
/// WebKit requests. No normalization can turn an unsafe path into an asset.
enum AppBundlePath {
    static let scheme = "eastsea-app"
    static let contentSecurityPolicy = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; media-src 'self' blob:; worker-src 'self'; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'none'"

    static func isSafe(_ path: String) -> Bool {
        let bytes = path.utf8
        guard (1...200).contains(bytes.count), bytes.allSatisfy({
            (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0)
                || $0 == 46 || $0 == 95 || $0 == 45 || $0 == 47
        }) else { return false }
        return path.split(separator: "/", omittingEmptySubsequences: false)
            .allSatisfy { !$0.isEmpty && $0 != "." && $0 != ".." }
    }

    static func mimeType(for path: String) -> String? {
        guard let leaf = path.split(separator: "/").last,
              let dot = leaf.lastIndex(of: ".") else { return nil }
        switch leaf[leaf.index(after: dot)...].lowercased() {
        case "html": return "text/html; charset=utf-8"
        case "js", "mjs": return "text/javascript; charset=utf-8"
        case "css": return "text/css; charset=utf-8"
        case "json": return "application/json; charset=utf-8"
        case "svg": return "image/svg+xml"
        case "png": return "image/png"
        case "jpg": return "image/jpeg"
        case "webp": return "image/webp"
        case "ico": return "image/x-icon"
        case "woff2": return "font/woff2"
        case "txt": return "text/plain; charset=utf-8"
        case "wasm": return "application/wasm"
        default: return nil
        }
    }

    static func isAppKey(_ key: String) -> Bool {
        // base32(32 bytes), without padding: its final four bits are zero.
        key.utf8.count == 52 && key.utf8.allSatisfy {
            (97...122).contains($0) || (50...55).contains($0)
        } && (key.last == "a" || key.last == "q")
    }

    static func requestPath(_ url: URL, appKey: String, entryPoint: String) -> String? {
        guard isAppKey(appKey), let parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              parts.scheme == scheme, parts.host == appKey,
              parts.user == nil, parts.password == nil, parts.port == nil,
              !parts.percentEncodedPath.contains("%") else { return nil }
        // Check the literal authority too; Foundation can decode escaped hosts.
        let prefix = "\(scheme)://\(appKey)"
        guard url.absoluteString.hasPrefix(prefix),
              url.absoluteString.dropFirst(prefix.count).first.map({ "/?#".contains($0) }) ?? true else { return nil }
        let path = parts.percentEncodedPath
        if path.isEmpty || path == "/" { return isSafe(entryPoint) ? entryPoint : nil }
        guard path.hasPrefix("/") else { return nil }
        let relative = String(path.dropFirst())
        return isSafe(relative) ? relative : nil
    }

    /// A custom WK scheme does not consistently enforce HTTP response CSP.
    /// Install the same policy before any publisher markup is parsed. Hashes
    /// are checked against the original bytes, before this transformation.
    static func renderData(_ data: Data, path: String) throws -> Data {
        guard mimeType(for: path) == "text/html; charset=utf-8" else { return data }
        guard String(data: data, encoding: .utf8) != nil else { throw AppContentError.invalidResponse }
        let prefix = "<!DOCTYPE html><meta http-equiv=\"Content-Security-Policy\" content=\"\(contentSecurityPolicy)\">"
        var rendered = Data(prefix.utf8)
        rendered.append(data)
        return rendered
    }
}

struct AppBundleFile: Decodable, Equatable, Sendable {
    let path: String
    let sha256: String
    let size: Int
}

struct AppBundleIndex: Sendable {
    static let maximumFiles = 2_000
    static let maximumFileBytes = 10 * 1_024 * 1_024
    static let maximumBundleBytes = 20_000_000
    static let maximumIndexBytes = 1_024 * 1_024

    let files: [AppBundleFile]
    let data: Data

    init(data: Data) throws {
        guard !data.isEmpty, data.count <= Self.maximumIndexBytes,
              !data.starts(with: [0xef, 0xbb, 0xbf]) else { throw AppContentError.invalidIndex }
        struct WireIndex: Decodable { let format: String; let files: [AppBundleFile] }
        guard let wire = try? JSONDecoder().decode(WireIndex.self, from: data),
              wire.format == "eastsea-bundle/1" else { throw AppContentError.invalidIndex }
        try Self.validate(wire.files)
        // Comparing exact bytes also rejects unknown/duplicate keys, whitespace,
        // escape spellings, alternate number spellings and noncanonical order.
        guard data == Self.canonicalData(for: wire.files) else { throw AppContentError.invalidIndex }
        guard Self.archiveSize(files: wire.files, indexBytes: data.count) <= Self.maximumBundleBytes else {
            throw AppContentError.tooLarge
        }
        files = wire.files
        self.data = data
    }

    static func canonicalData(for files: [AppBundleFile]) -> Data {
        let entries = files.map { "{\"path\":\"\($0.path)\",\"sha256\":\"\($0.sha256)\",\"size\":\($0.size)}" }
        return Data("{\"format\":\"eastsea-bundle/1\",\"files\":[\(entries.joined(separator: ","))]}".utf8)
    }

    private static func validate(_ files: [AppBundleFile]) throws {
        guard !files.isEmpty, files.count <= maximumFiles else { throw AppContentError.invalidIndex }
        var folded = Set<String>()
        var previous: String?
        var total = 0
        for file in files {
            guard AppBundlePath.isSafe(file.path), file.path.lowercased() != "bundle.json" else {
                throw AppContentError.invalidPath(file.path)
            }
            guard AppBundlePath.mimeType(for: file.path) != nil else { throw AppContentError.unsupportedFile(file.path) }
            guard AppBundleHash.normalized(file.sha256) == file.sha256 else { throw AppContentError.invalidIndex }
            guard file.size >= 0, file.size <= maximumFileBytes,
                  total <= maximumBundleBytes - file.size else { throw AppContentError.tooLarge }
            guard folded.insert(file.path.lowercased()).inserted,
                  previous.map({ $0.utf8.lexicographicallyPrecedes(file.path.utf8) }) ?? true else {
                throw AppContentError.invalidIndex
            }
            total += file.size
            previous = file.path
        }
    }

    private static func archiveSize(files: [AppBundleFile], indexBytes: Int) -> Int {
        func padded(_ size: Int) -> Int { ((size + 511) / 512) * 512 }
        return 512 + padded(indexBytes) + files.reduce(0) { $0 + 512 + padded($1.size) } + 1_024
    }
}

/// Immutable bytes returned only after the entire index and every asset pass.
/// A developer snapshot has no on-chain trust and retains that distinction.
struct AppBundle: Sendable {
    let bundleHash: String
    let index: AppBundleIndex
    let files: [String: Data]
    let entryPoint: String
    let isVerified: Bool

    fileprivate init(bundleHash: String, index: AppBundleIndex, files: [String: Data],
                     entryPoint: String, isVerified: Bool) throws {
        guard AppBundlePath.isSafe(entryPoint),
              AppBundlePath.mimeType(for: entryPoint) == "text/html; charset=utf-8",
              files[entryPoint] != nil else { throw AppContentError.missingFile(entryPoint) }
        self.bundleHash = bundleHash
        self.index = index
        self.files = files
        self.entryPoint = entryPoint
        self.isVerified = isVerified
    }

    func documentPath(for url: URL, appKey: String) -> String? {
        guard let path = AppBundlePath.requestPath(url, appKey: appKey, entryPoint: entryPoint),
              files[path] != nil, AppBundlePath.mimeType(for: path) == "text/html; charset=utf-8" else { return nil }
        return path
    }

    /// Only HTML receives the CSP meta fallback. SVG/JS stay assets and can
    /// never become a privileged top-level document.
    func pageURL(appKey: String, path: String = "/", query: String? = nil) throws -> URL {
        let relative = path == "/" || path.isEmpty ? entryPoint : String(path.dropFirst())
        guard (path.isEmpty || path.hasPrefix("/")), AppBundlePath.isAppKey(appKey),
              AppBundlePath.isSafe(relative), files[relative] != nil,
              AppBundlePath.mimeType(for: relative) == "text/html; charset=utf-8" else { throw AppContentError.invalidPath(path) }
        var target = URLComponents()
        target.scheme = AppBundlePath.scheme
        target.host = appKey
        target.path = "/\(relative)"
        // Validate percent escapes before using Foundation's checked setter.
        if let query {
            guard let checked = URLComponents(string: "https://example.invalid/?" + query),
                  checked.fragment == nil, checked.percentEncodedQuery == query else { throw AppContentError.invalidResponse }
        }
        target.percentEncodedQuery = query
        guard let url = target.url, documentPath(for: url, appKey: appKey) != nil else { throw AppContentError.invalidResponse }
        return url
    }
}

struct AppBundleFileResponse: Sendable {
    let bundleHash: String
    let path: String
    let sha256: String
    let size: Int
    let data: Data
}

enum AppBundleLoader {
    typealias Fetch = @Sendable (String) async throws -> AppBundleFileResponse

    static func load(bundleHash: String, entryPoint: String = "index.html", fetch: Fetch) async throws -> AppBundle {
        guard let expected = AppBundleHash.normalized(bundleHash) else { throw AppContentError.invalidHash }
        try Task.checkCancellation()
        let response = try await fetch("bundle.json")
        try verify(response, expectedBundle: expected, path: "bundle.json", maximumBytes: AppBundleIndex.maximumIndexBytes)
        guard response.sha256 == expected else { throw AppContentError.hashMismatch("bundle.json") }
        let index = try AppBundleIndex(data: response.data)
        var files: [String: Data] = [:]
        for file in index.files {
            try Task.checkCancellation()
            let asset = try await fetch(file.path)
            try verify(asset, expectedBundle: expected, path: file.path, maximumBytes: file.size)
            guard asset.size == file.size, asset.sha256 == file.sha256 else {
                throw AppContentError.hashMismatch(file.path)
            }
            files[file.path] = asset.data
        }
        try Task.checkCancellation()
        return try AppBundle(bundleHash: expected, index: index, files: files, entryPoint: entryPoint, isVerified: true)
    }

    private static func verify(_ response: AppBundleFileResponse, expectedBundle: String,
                               path: String, maximumBytes: Int) throws {
        guard AppBundleHash.normalized(response.bundleHash) == expectedBundle,
              response.path == path, response.size >= 0,
              response.data.count <= maximumBytes else { throw AppContentError.invalidResponse }
        guard response.size == response.data.count,
              response.sha256 == AppBundleHash.hex(response.data) else { throw AppContentError.hashMismatch(path) }
    }
}

struct AppBundleRPCClient: Sendable {
    let endpoint: URL

    init(endpoint: URL) throws {
        guard let parts = URLComponents(url: endpoint, resolvingAgainstBaseURL: false),
              parts.scheme == "http", ["127.0.0.1", "::1", "[::1]"].contains(parts.host ?? ""),
              parts.user == nil, parts.password == nil, parts.query == nil, parts.fragment == nil,
              parts.path.isEmpty || parts.path == "/",
              parts.port.map({ (1...65_535).contains($0) }) ?? true else { throw AppContentError.invalidResponse }
        self.endpoint = endpoint
    }

    func load(bundleHash: String, entryPoint: String = "index.html") async throws -> AppBundle {
        try await AppBundleLoader.load(bundleHash: bundleHash, entryPoint: entryPoint) { path in
            try await fetch(bundleHash: bundleHash, path: path)
        }
    }

    func fetch(bundleHash: String, path: String) async throws -> AppBundleFileResponse {
        guard let expected = AppBundleHash.normalized(bundleHash),
              path == "bundle.json" || AppBundlePath.isSafe(path) else { throw AppContentError.invalidPath(path) }
        var request = URLRequest(url: endpoint)
        request.httpMethod = "POST"
        request.timeoutInterval = 30
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let payload: [String: Any] = [
            "jsonrpc": "2.0", "id": 1, "method": "aether_appBundle", "params": [expected, path],
        ]
        request.httpBody = try JSONSerialization.data(withJSONObject: payload)
        let maximumFileBytes = path == "bundle.json" ? AppBundleIndex.maximumIndexBytes : AppBundleIndex.maximumFileBytes
        let maximumResponseBytes = ((maximumFileBytes + 2) / 3) * 4 + 4_096
        let configuration = URLSessionConfiguration.ephemeral
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.timeoutIntervalForResource = 30
        let session = URLSession(configuration: configuration, delegate: AppBundleRPCNoRedirect(), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        do {
            let (bytes, response) = try await session.bytes(for: request)
            guard (response as? HTTPURLResponse)?.statusCode == 200,
                  response.expectedContentLength <= Int64(maximumResponseBytes) else { throw AppContentError.invalidResponse }
            var data = Data()
            if response.expectedContentLength > 0 { data.reserveCapacity(Int(response.expectedContentLength)) }
            for try await byte in bytes {
                guard data.count < maximumResponseBytes else { throw AppContentError.tooLarge }
                data.append(byte)
            }
            struct Envelope: Decodable {
                struct Result: Decodable {
                    let bundleHash: String; let path: String; let sha256: String; let size: Int; let data: Data
                }
                struct RPCError: Decodable { let code: Int; let message: String }
                let jsonrpc: String
                let id: Int
                let result: Result?
                let error: RPCError?
            }
            guard let reply = try? JSONDecoder().decode(Envelope.self, from: data), reply.jsonrpc == "2.0",
                  reply.id == 1 else { throw AppContentError.invalidResponse }
            if reply.error != nil { throw AppContentError.nodeUnavailable }
            guard let result = reply.result else { throw AppContentError.invalidResponse }
            return AppBundleFileResponse(bundleHash: result.bundleHash, path: result.path, sha256: result.sha256,
                                         size: result.size, data: result.data)
        } catch let error as AppContentError {
            throw error
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            if Task.isCancelled { throw CancellationError() }
            throw AppContentError.nodeUnavailable
        }
    }
}

private final class AppBundleRPCNoRedirect: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

enum LocalAppFolder {
    static func load(from directory: URL, developerModeEnabled: Bool, entryPoint: String = "index.html") throws -> AppBundle {
        guard developerModeEnabled else { throw AppContentError.developerModeRequired }
        guard directory.isFileURL else { throw AppContentError.unsafeLocalFile("") }
        let root = directory.standardizedFileURL
        let rootDescriptor = open(root.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
        guard rootDescriptor >= 0 else { throw AppContentError.unsafeLocalFile("") }
        defer { close(rootDescriptor) }
        var enumerationError = false
        guard let enumerator = FileManager.default.enumerator(at: root,
            includingPropertiesForKeys: [.isRegularFileKey, .isDirectoryKey, .isSymbolicLinkKey], options: [],
            errorHandler: { _, _ in enumerationError = true; return false }) else {
            throw AppContentError.unsafeLocalFile("")
        }
        var files: [String: Data] = [:]
        var total = 0
        let prefix = root.path.hasSuffix("/") ? root.path : root.path + "/"
        for case let file as URL in enumerator {
            guard file.path.hasPrefix(prefix) else { throw AppContentError.unsafeLocalFile("") }
            let path = String(file.path.dropFirst(prefix.count))
            let values = try file.resourceValues(forKeys: [.isRegularFileKey, .isDirectoryKey, .isSymbolicLinkKey])
            guard values.isSymbolicLink != true, AppBundlePath.isSafe(path) else { throw AppContentError.unsafeLocalFile(path) }
            if values.isDirectory == true { continue }
            guard values.isRegularFile == true, path.lowercased() != "bundle.json" else { throw AppContentError.unsafeLocalFile(path) }
            guard AppBundlePath.mimeType(for: path) != nil else { throw AppContentError.unsupportedFile(path) }
            guard files.count < AppBundleIndex.maximumFiles else { throw AppContentError.tooLarge }
            let bytes = try read(path: path, rootDescriptor: rootDescriptor)
            guard total <= AppBundleIndex.maximumBundleBytes - bytes.count else { throw AppContentError.tooLarge }
            total += bytes.count
            files[path] = bytes
        }
        guard !enumerationError else { throw AppContentError.unsafeLocalFile("") }
        let entries = files.keys.sorted { $0.utf8.lexicographicallyPrecedes($1.utf8) }.map {
            AppBundleFile(path: $0, sha256: AppBundleHash.hex(files[$0]!), size: files[$0]!.count)
        }
        let index = try AppBundleIndex(data: AppBundleIndex.canonicalData(for: entries))
        return try AppBundle(bundleHash: AppBundleHash.hex(index.data), index: index, files: files,
                             entryPoint: entryPoint, isVerified: false)
    }

    /// Resolve every component against directory descriptors with O_NOFOLLOW.
    /// A concurrent rename or symlink replacement cannot redirect reads outside
    /// the chosen root. Nonblocking open also avoids hanging on a replaced FIFO.
    private static func read(path: String, rootDescriptor: Int32) throws -> Data {
        let parts = path.split(separator: "/").map(String.init)
        var directoryDescriptor = dup(rootDescriptor)
        guard directoryDescriptor >= 0 else { throw AppContentError.unsafeLocalFile(path) }
        defer { close(directoryDescriptor) }
        for part in parts.dropLast() {
            let next = openat(directoryDescriptor, part, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
            guard next >= 0 else { throw AppContentError.unsafeLocalFile(path) }
            close(directoryDescriptor)
            directoryDescriptor = next
        }
        let descriptor = openat(directoryDescriptor, parts.last!, O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
        guard descriptor >= 0 else { throw AppContentError.unsafeLocalFile(path) }
        defer { close(descriptor) }
        var info = stat()
        guard fstat(descriptor, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG else { throw AppContentError.unsafeLocalFile(path) }
        guard info.st_size >= 0, info.st_size <= AppBundleIndex.maximumFileBytes else { throw AppContentError.tooLarge }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 65_536)
        while true {
            let count = buffer.withUnsafeMutableBytes { Darwin.read(descriptor, $0.baseAddress, $0.count) }
            if count == 0 { return data }
            if count < 0, errno == EINTR { continue }
            guard count > 0 else { throw AppContentError.unsafeLocalFile(path) }
            guard data.count <= AppBundleIndex.maximumFileBytes - count else { throw AppContentError.tooLarge }
            data.append(contentsOf: buffer.prefix(count))
        }
    }
}
