import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

func rejects(_ message: String, _ body: () throws -> Void) {
    do { try body(); check(false, message) } catch { }
}

func rejectsAsync(_ message: String, _ body: () async throws -> Void) async {
    do { try await body(); check(false, message) } catch { }
}

func parse(_ files: [AppBundleFile]) throws -> AppBundleIndex {
    try AppBundleIndex(data: AppBundleIndex.canonicalData(for: files))
}

let assets = [
    "app.js": Data("document.title = 'Bundle loaded';".utf8),
    "asset.svg": Data("<svg xmlns='http://www.w3.org/2000/svg'><script>window.inlineViolation=true</script></svg>".utf8),
    "index.html": Data("<!doctype html><html><head><script src='app.js' defer></script><link rel='stylesheet' href='style.css'></head><body>Verified app</body></html>".utf8),
    "style.css": Data("body { color: navy; }".utf8),
]
let entries = assets.keys.sorted().map { AppBundleFile(path: $0, sha256: AppBundleHash.hex(assets[$0]!), size: assets[$0]!.count) }
let indexData = AppBundleIndex.canonicalData(for: entries)
let bundleHash = AppBundleHash.hex(indexData)
let index = try AppBundleIndex(data: indexData)
check(index.files == entries, "canonical index parses")
check(AppBundleHash.normalized("0x" + bundleHash.uppercased()) == bundleHash, "on-chain hash normalizes")
check(AppBundleHash.normalized("0x123") == nil, "short hash refused")

actor FetchLog {
    var paths: [String] = []
    func record(_ path: String) { paths.append(path) }
}
let log = FetchLog()
let bundle = try await AppBundleLoader.load(bundleHash: "0x" + bundleHash) { path in
    await log.record(path)
    guard let data = path == "bundle.json" ? indexData : assets[path] else { throw AppContentError.missingFile(path) }
    return AppBundleFileResponse(bundleHash: bundleHash, path: path, sha256: AppBundleHash.hex(data), size: data.count, data: data)
}
check(bundle.isVerified && bundle.files == assets && bundle.entryPoint == "index.html", "every asset is returned verified")
check(await log.paths == ["bundle.json", "app.js", "asset.svg", "index.html", "style.css"], "all assets are checked before load returns")
let appKey = String(repeating: "a", count: 52)
check(try bundle.pageURL(appKey: appKey).path == "/index.html", "HTML entry can load")
rejects("active SVG cannot become a privileged document") { _ = try bundle.pageURL(appKey: appKey, path: "/asset.svg") }
check(bundle.documentPath(for: URL(string: "eastsea-app://\(appKey)/asset.svg")!, appKey: appKey) == nil, "SVG navigation is refused")
rejects("invalid query escapes cannot reach a checked Foundation setter") { _ = try bundle.pageURL(appKey: appKey, query: "bad=%ZZ") }

await rejectsAsync("index hash mismatch rejected") {
    _ = try await AppBundleLoader.load(bundleHash: String(repeating: "0", count: 64)) { path in
        AppBundleFileResponse(bundleHash: String(repeating: "0", count: 64), path: path,
                              sha256: AppBundleHash.hex(indexData), size: indexData.count, data: indexData)
    }
}
await rejectsAsync("last asset mismatch rejected even after index.html verified") {
    _ = try await AppBundleLoader.load(bundleHash: bundleHash) { path in
        var data = path == "bundle.json" ? indexData : assets[path]!
        if path == "style.css" { data[0] = 88 }
        return AppBundleFileResponse(bundleHash: bundleHash, path: path, sha256: AppBundleHash.hex(data), size: data.count, data: data)
    }
}
await rejectsAsync("response metadata cannot substitute a different requested path") {
    _ = try await AppBundleLoader.load(bundleHash: bundleHash) { _ in
        AppBundleFileResponse(bundleHash: bundleHash, path: "other.json", sha256: bundleHash, size: indexData.count, data: indexData)
    }
}
await rejectsAsync("missing assets do not become a partial bundle") {
    _ = try await AppBundleLoader.load(bundleHash: bundleHash) { path in
        guard path == "bundle.json" else { throw AppContentError.missingFile(path) }
        return AppBundleFileResponse(bundleHash: bundleHash, path: path, sha256: bundleHash, size: indexData.count, data: indexData)
    }
}
await rejectsAsync("offline fetch cannot yield a bundle") {
    _ = try await AppBundleLoader.load(bundleHash: bundleHash) { _ in throw AppContentError.nodeUnavailable }
}

let zeroHash = String(repeating: "0", count: 64)
for path in ["../index.html", "/index.html", "js/../app.js", "js/./app.js", "a//x.js", "a/", "", "a\\x.js", "a/%2e%2e/x.js", "é.js", String(repeating: "a", count: 198) + ".js", "bundle.json", "BUNDLE.JSON"] {
    rejects("unsafe index path \(path)") { _ = try parse([AppBundleFile(path: path, sha256: zeroHash, size: 0)]) }
}
rejects("casefold duplicates rejected") {
    _ = try parse([AppBundleFile(path: "App.js", sha256: zeroHash, size: 0), AppBundleFile(path: "app.js", sha256: zeroHash, size: 0)])
}
rejects("byte order rejected") { _ = try parse(Array(entries.reversed())) }
rejects("unlisted file type rejected") { _ = try parse([AppBundleFile(path: "binary.exe", sha256: zeroHash, size: 0)]) }
rejects("uppercase file digest is noncanonical") {
    _ = try parse([AppBundleFile(path: "index.html", sha256: bundleHash.uppercased(), size: 0)])
}
rejects("negative size rejected") { _ = try parse([AppBundleFile(path: "index.html", sha256: zeroHash, size: -1)]) }
rejects("per-file limit respected") {
    _ = try parse([AppBundleFile(path: "index.html", sha256: zeroHash, size: AppBundleIndex.maximumFileBytes + 1)])
}
rejects("20 MiB cap counts archive overhead") {
    _ = try parse([AppBundleFile(path: "app.js", sha256: zeroHash, size: AppBundleIndex.maximumFileBytes),
                   AppBundleFile(path: "index.html", sha256: zeroHash, size: AppBundleIndex.maximumFileBytes)])
}
rejects("file-count cap respected") {
    _ = try parse((0...AppBundleIndex.maximumFiles).map {
        AppBundleFile(path: String(format: "%04d.js", $0), sha256: zeroHash, size: 0)
    })
}
for malformed in [
    Data(" ".utf8) + indexData,
    Data([0xef, 0xbb, 0xbf]) + indexData,
    Data(String(data: indexData, encoding: .utf8)!.replacingOccurrences(of: "{\"format\":", with: "{\"extra\":1,\"format\":").utf8),
    Data(String(data: indexData, encoding: .utf8)!.replacingOccurrences(of: "\"format\":\"eastsea-bundle/1\",", with: "\"format\":\"eastsea-bundle/1\",\"format\":\"eastsea-bundle/1\",").utf8),
    Data(String(data: indexData, encoding: .utf8)!.replacingOccurrences(of: "eastsea-bundle/1", with: "eastsea-bundle/2").utf8),
    Data(String(data: indexData, encoding: .utf8)!.replacingOccurrences(of: "\"size\":\(entries[0].size)", with: "\"size\":\(entries[0].size).0").utf8),
    Data("{\"files\":[],\"format\":\"eastsea-bundle/1\"}".utf8),
] {
    rejects("noncanonical bytes rejected") { _ = try AppBundleIndex(data: malformed) }
}

check(AppBundlePath.isAppKey(appKey), "52-character base32 authority")
check(!AppBundlePath.isAppKey(String(repeating: "a", count: 51) + "b"), "noncanonical final base32 bits refused")
func requestPath(_ url: String) -> String? {
    guard let url = URL(string: url) else { return nil }
    return AppBundlePath.requestPath(url, appKey: appKey, entryPoint: "index.html")
}
check(requestPath("eastsea-app://\(appKey)/") == "index.html", "root uses declared entry")
check(requestPath("eastsea-app://\(appKey)/app.js?v=2") == "app.js", "asset query is harmless")
for url in [
    "eastsea-app://\(appKey)/../index.html", "eastsea-app://\(appKey)/a/./app.js",
    "eastsea-app://\(appKey)/a//app.js", "eastsea-app://\(appKey)/%2e%2e/index.html",
    "eastsea-app://\(appKey)/a%2fb.js", "eastsea-app://\(appKey)/a%5cb.js",
    "eastsea-app://\(appKey):8080/index.html", "eastsea-app://user@\(appKey)/index.html",
    "eastsea-app://\(appKey)./index.html", "eastsea-app://b\(appKey.dropFirst())/index.html",
    "https://\(appKey)/index.html",
] { check(requestPath(url) == nil, "unsafe scheme request rejected: \(url)") }
check(AppBundlePath.mimeType(for: "app.mjs") == "text/javascript; charset=utf-8", "module mime")
check(AppBundlePath.mimeType(for: "app.wasm") == "application/wasm", "wasm mime without eval permission")
let rendered = try AppBundlePath.renderData(assets["index.html"]!, path: "index.html")
let renderedHTML = String(data: rendered, encoding: .utf8)!
check(renderedHTML.hasPrefix("<!DOCTYPE html><meta http-equiv=\"Content-Security-Policy\""), "CSP applies before publisher markup")
check(rendered.suffix(assets["index.html"]!.count) == assets["index.html"]!, "original verified HTML preserved")
check(AppBundlePath.contentSecurityPolicy.contains("connect-src 'none'") &&
      AppBundlePath.contentSecurityPolicy.contains("script-src 'self'") &&
      !AppBundlePath.contentSecurityPolicy.contains("unsafe-eval") &&
      !AppBundlePath.contentSecurityPolicy.contains("127.0.0.1"), "app CSP blocks node access and remote code")
rejects("non-UTF8 HTML refused") { _ = try AppBundlePath.renderData(Data([0xff]), path: "index.html") }

_ = try AppBundleRPCClient(endpoint: URL(string: "http://127.0.0.1:18545/")!)
for endpoint in ["https://127.0.0.1:18545/", "http://localhost:18545/", "http://example.com/", "http://127.0.0.1:18545/private", "http://user@127.0.0.1/", "http://127.0.0.1/?x=1"] {
    rejects("RPC endpoint is literal loopback only") { _ = try AppBundleRPCClient(endpoint: URL(string: endpoint)!) }
}

let fm = FileManager.default
let tempRoot = URL(fileURLWithPath: ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"] ??
    URL(fileURLWithPath: fm.currentDirectoryPath).appendingPathComponent("tmp").path)
    .appendingPathComponent("app-content-\(UUID().uuidString)", isDirectory: true)
try fm.createDirectory(at: tempRoot, withIntermediateDirectories: true)
defer { try? fm.removeItem(at: tempRoot) }
let folder = tempRoot.appendingPathComponent("app", isDirectory: true)
try fm.createDirectory(at: folder, withIntermediateDirectories: true)
for (path, bytes) in assets { try bytes.write(to: folder.appendingPathComponent(path)) }
rejects("developer gate precedes all folder access") { _ = try LocalAppFolder.load(from: folder, developerModeEnabled: false) }
let local = try LocalAppFolder.load(from: folder, developerModeEnabled: true)
check(!local.isVerified && local.files == assets, "developer snapshot stays visibly unverified")
try Data("changed".utf8).write(to: folder.appendingPathComponent("index.html"))
check(local.files["index.html"] == assets["index.html"], "folder changes cannot alter an opened snapshot")
let outside = tempRoot.appendingPathComponent("outside.html")
try Data("outside".utf8).write(to: outside)
let link = folder.appendingPathComponent("escape.html")
try fm.createSymbolicLink(at: link, withDestinationURL: outside)
rejects("local symlink file escape rejected") { _ = try LocalAppFolder.load(from: folder, developerModeEnabled: true) }
try fm.removeItem(at: link)
let directoryLink = folder.appendingPathComponent("outside", isDirectory: true)
try fm.createSymbolicLink(at: directoryLink, withDestinationURL: tempRoot)
rejects("local symlink directory escape rejected") { _ = try LocalAppFolder.load(from: folder, developerModeEnabled: true) }
try fm.removeItem(at: directoryLink)
let rootLink = tempRoot.appendingPathComponent("root-link", isDirectory: true)
try fm.createSymbolicLink(at: rootLink, withDestinationURL: folder)
rejects("symlink root folder rejected") { _ = try LocalAppFolder.load(from: rootLink, developerModeEnabled: true) }

print("app-content OK")
