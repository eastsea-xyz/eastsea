import Foundation
import CryptoKit

var checked = 0
func check(_ ok: Bool, _ message: String) {
    checked += 1
    if !ok { fatalError(message) }
}
let owner = "0x" + String(repeating: "1", count: 40)
let addr = "0x" + String(repeating: "2", count: 40)
let names = "0x" + String(repeating: "3", count: 40)
let apps = "0x" + String(repeating: "4", count: 40)
let appID = "0x" + String(repeating: "5", count: 64)
let node = String(repeating: "6", count: 64)
let hash = String(repeating: "7", count: 64)
let code = "0x6000"
let codeHash = SHA256.hash(data: Data([0x60, 0])).map { String(format: "%02x", $0) }.joined()
let pins = SeaRegistrySources(names: .init(address: names, codeSHA256: codeHash),
                              apps: .init(address: apps, codeSHA256: codeHash))
func word(_ value: UInt64) -> String {
    let s = String(value, radix: 16)
    return String(repeating: "0", count: 64 - s.count) + s
}
func text(_ value: String) -> String {
    let h = value.utf8.map { String(format: "%02x", $0) }.joined()
    return "0x" + word(32) + word(UInt64(value.utf8.count)) + h
        + String(repeating: "0", count: (64 - h.count % 64) % 64)
}
var expired = false
var listed = true
var bound = true
var registered = true
var corruptOwner = false
var releaseOverride: String?
var asked: [(String, String)] = []
func read(_ to: String, _ data: String) throws -> String {
    asked.append((to, data))
    switch String(data.prefix(10)) {
    case "0x864ce5e1": return "0x" + node
    case "0x7dd56411":
        let value = registered ? owner : "0x" + String(repeating: "0", count: 40)
        return corruptOwner ? "0x" + String(repeating: "f", count: 64) : "0x" + String(repeating: "0", count: 24) + value.dropFirst(2)
    case "0xb25be181": return "0x" + String(repeating: "0", count: 24) + addr.dropFirst(2)
    case "0x9dfcc616": return "0x" + word(expired ? 100 : 200)
    case "0xffdbd0e3": return text(bound ? appID : "")
    case "0x470d9498": return releaseOverride ?? "0x" + word(1) + hash + hash + word(listed ? 1 : 0)
    default: fatalError("unexpected contract read")
    }
}
let link: SeaURL.NameLink
if case .name(let n) = try SeaURL.parse("sea://any.harbor.sea/docs?q=1") { link = n }
else { fatalError("expected name") }
let result = try await SeaNameResolver.resolve(link, now: 150, sources: pins, code: { _ in code }, read: read)
check(result.owner == owner && result.address == addr, "name record is read before its app")
check(result.app.appID == appID && result.app.sequence == 1, "app binding resolves currentRelease")
check(result.link.canonicalURL == "sea://any.harbor.sea/docs?q=1", "canonical location survives resolution")
check(asked.last?.0 == apps, "app lookup uses pinned registry")
check(asked[0].1.contains("616e792e686172626f722e736561"), "new registry receives full hostname")

func fails(_ expected: SeaNameResolver.Failure, _ action: () async throws -> Void) async {
    do { try await action(); fatalError("expected \(expected)") }
    catch let e as SeaNameResolver.Failure { check(e == expected, "wrong failure: \(e)") }
    catch { fatalError("unexpected error: \(error)") }
}
func parseFails(_ expected: SeaURL.ParseError, _ action: () async throws -> Void) async {
    do { try await action(); fatalError("expected \(expected)") }
    catch let error as SeaURL.ParseError { check(error == expected, "wrong parse failure: \(error)") }
    catch { fatalError("unexpected error: \(error)") }
}
expired = true
await fails(.expired) { _ = try await SeaNameResolver.resolve(link, now: 100, sources: pins, code: { _ in code }, read: read) }
expired = false
listed = false
await fails(.unlistedApp) { _ = try await SeaNameResolver.resolve(link, now: 150, sources: pins, code: { _ in code }, read: read) }
listed = true
bound = false
await fails(.noApp) { _ = try await SeaNameResolver.resolve(link, now: 150, sources: pins, code: { _ in code }, read: read) }
bound = true
corruptOwner = true
await fails(.badAnswer) { _ = try await SeaNameResolver.resolve(link, now: 150, sources: pins, code: { _ in code }, read: read) }
corruptOwner = false
asked.removeAll()
await fails(.registryUnavailable) { _ = try await SeaNameResolver.resolve(link, now: 150, sources: nil, code: { _ in code }, read: read) }
check(asked.isEmpty, "missing deployment does not guess addresses")
await fails(.wrongCode) { _ = try await SeaNameResolver.resolve(link, now: 150, sources: pins, code: { _ in "0x6001" }, read: read) }
check(asked.isEmpty, "runtime code pin gates all registry calls")

let oldLink: SeaURL.NameLink
if case .name(let n) = try SeaURL.parse("sea://harbor.aeth", chainID: 7780) { oldLink = n }
else { fatalError("expected legacy alias") }
let old = try await SeaNameResolver.resolve(oldLink, now: 150, sources: pins, chainID: 7780, code: { _ in code }, read: read)
check(old.link.name == "harbor.sea", "legacy name is displayed canonically")
check(asked[0].1.contains("686172626f72") && !asked[0].1.contains("2e736561"), "7780 uses the old bare-label ABI")
asked.removeAll()
do {
    _ = try await SeaNameResolver.resolve(oldLink, now: 150, sources: pins, chainID: 1, code: { _ in code }, read: read)
    fatalError("legacy alias must not cross chains")
} catch let error as SeaURL.ParseError {
    check(error == .legacyNameUnsupported && asked.isEmpty, "legacy provenance is checked before registry access")
}
let payload = Data("{\"chains\":{}}".utf8)
check(SeaRegistrySources.parse(payload, chainID: 7780) == nil, "absent registry config stays absent")
let rootLink: SeaURL.NameLink
if case .name(let n) = try SeaURL.parse("sea://harbor") { rootLink = n }
else { fatalError("expected root") }
let grace = try await SeaNameResolver.resolve(rootLink, now: 201, sources: pins, code: { _ in code }, read: read)
check(grace.link.name == "harbor.sea", "root resolution preserves its existing 30-day grace")
await fails(.expired) {
    _ = try await SeaNameResolver.resolve(rootLink, now: 200 + 30 * 24 * 60 * 60,
                                         sources: pins, code: { _ in code }, read: read)
}

// Search resolves address records even when no app registry or binding exists.
let nameOnlyPins = SeaRegistrySources(names: pins.names, apps: .init(address: "invalid", codeSHA256: "invalid"))
var lookupCodeAddresses: [String] = []
func lookupCode(_ to: String) async throws -> String {
    lookupCodeAddresses.append(to)
    return code
}
bound = false
asked.removeAll()
let nameRecord = try await SeaNameResolver.lookup(link, now: 150, sources: nameOnlyPins, code: lookupCode, read: read)
check(nameRecord == .init(link: link, owner: owner, address: addr), "name lookup preserves the canonical link, owner and address")
check(lookupCodeAddresses == [names], "name lookup verifies only the names code pin")
check(asked.count == 4 && asked.allSatisfy { $0.0 == names }, "address-only names do not read an app binding or registry")
bound = true
asked.removeAll()
lookupCodeAddresses.removeAll()
await fails(.registryUnavailable) {
    _ = try await SeaNameResolver.lookup(link, now: 150, sources: nil, code: lookupCode, read: read)
}
let invalidNamePin = SeaRegistrySources(names: .init(address: "0x" + String(repeating: "0", count: 40), codeSHA256: codeHash), apps: pins.apps)
await fails(.registryUnavailable) {
    _ = try await SeaNameResolver.lookup(link, now: 150, sources: invalidNamePin, code: lookupCode, read: read)
}
check(asked.isEmpty && lookupCodeAddresses.isEmpty, "missing or invalid name pins cause no contract or code reads")
await fails(.wrongCode) {
    _ = try await SeaNameResolver.lookup(link, now: 150, sources: nameOnlyPins, code: { _ in "0x6001" }, read: read)
}
check(asked.isEmpty, "name-only lookup rejects a code mismatch before contract reads")
registered = false
await fails(.unregistered) {
    _ = try await SeaNameResolver.lookup(link, now: 150, sources: nameOnlyPins, code: lookupCode, read: read)
}
registered = true
expired = true
await fails(.expired) {
    _ = try await SeaNameResolver.lookup(link, now: 100, sources: nameOnlyPins, code: lookupCode, read: read)
}
expired = false
let nameGrace = try await SeaNameResolver.lookup(rootLink, now: 201, sources: nameOnlyPins, code: lookupCode, read: read)
check(nameGrace.link == grace.link, "name-only root lookup preserves the existing grace window")
await fails(.expired) {
    _ = try await SeaNameResolver.lookup(rootLink, now: 200 + 30 * 24 * 60 * 60,
                                        sources: nameOnlyPins, code: lookupCode, read: read)
}
asked.removeAll()
lookupCodeAddresses.removeAll()
let inconsistentLink = SeaURL.NameLink(name: link.name, canonicalURL: "sea://other.sea/", path: link.path,
                                       query: link.query, isLegacy: link.isLegacy, registryName: link.registryName)
await parseFails(.invalidName) {
    _ = try await SeaNameResolver.lookup(inconsistentLink, now: 150, sources: nameOnlyPins, code: lookupCode, read: read)
}
let invalidLink = SeaURL.NameLink(name: "Harbor.sea", canonicalURL: "sea://Harbor.sea/", path: "/",
                                  query: nil, isLegacy: false, registryName: "Harbor.sea")
await parseFails(.invalidName) {
    _ = try await SeaNameResolver.lookup(invalidLink, now: 150, sources: nameOnlyPins, code: lookupCode, read: read)
}
await parseFails(.legacyNameUnsupported) {
    _ = try await SeaNameResolver.lookup(oldLink, now: 150, sources: nameOnlyPins, chainID: 1, code: lookupCode, read: read)
}
check(asked.isEmpty && lookupCodeAddresses.isEmpty, "canonical grammar and legacy provenance are checked before any registry access")
let oldName = try await SeaNameResolver.lookup(oldLink, now: 150, sources: nameOnlyPins, chainID: 7780, code: lookupCode, read: read)
check(oldName.link == oldLink && asked[0].1.contains("686172626f72") && !asked[0].1.contains("2e736561"), "legacy name-only lookup retains the old bare-label ABI")

// A registry app may be opened directly by ID without any registered name.
let appOnlyPins = SeaRegistrySources(names: .init(address: "invalid", codeSHA256: "invalid"), apps: pins.apps)
asked.removeAll()
lookupCodeAddresses.removeAll()
registered = false
expired = true
bound = false
let directApp = try await SeaNameResolver.resolveApp(appID: appID, sources: appOnlyPins, code: lookupCode, read: read)
check(directApp == result.app, "direct app resolution reads the same active release as name resolution")
check(lookupCodeAddresses == [apps], "direct app resolution verifies only the app-registry code pin")
check(asked.count == 1 && asked[0].0 == apps && asked[0].1 == "0x470d9498" + appID.dropFirst(2),
      "direct app resolution uses the currentRelease ABI without a name record")
registered = true
expired = false
bound = true
listed = false
await fails(.unlistedApp) {
    _ = try await SeaNameResolver.resolveApp(appID: appID, sources: appOnlyPins, code: lookupCode, read: read)
}
listed = true
for malformed in [
    "0x" + word(0) + hash + hash + word(1),
    "0x" + word(UInt64(UInt32.max) + 1) + hash + hash + word(1),
    "0x" + String(repeating: "f", count: 64) + hash + hash + word(1),
    "0x" + word(1) + word(0) + hash + word(1),
    "0x" + word(1) + hash + word(0) + word(1),
    "0x" + word(1) + hash + hash + word(2),
    "0x" + word(1) + hash + hash
] {
    releaseOverride = malformed
    await fails(.badAnswer) {
        _ = try await SeaNameResolver.resolveApp(appID: appID, sources: appOnlyPins, code: lookupCode, read: read)
    }
}
releaseOverride = "0x" + word(UInt64(UInt32.max)) + hash + hash + word(1)
let lastSequence = try await SeaNameResolver.resolveApp(appID: appID, sources: appOnlyPins, code: lookupCode, read: read)
check(lastSequence.sequence == UInt32.max, "the highest valid release sequence is accepted")
releaseOverride = nil
asked.removeAll()
lookupCodeAddresses.removeAll()
for invalidID in ["0x" + String(repeating: "0", count: 64), "0x55", "0x" + String(repeating: "G", count: 64)] {
    await fails(.noApp) {
        _ = try await SeaNameResolver.resolveApp(appID: invalidID, sources: appOnlyPins, code: lookupCode, read: read)
    }
}
await fails(.registryUnavailable) {
    _ = try await SeaNameResolver.resolveApp(appID: appID, sources: nil, code: lookupCode, read: read)
}
await fails(.registryUnavailable) {
    _ = try await SeaNameResolver.resolveApp(appID: appID, sources: nameOnlyPins, code: lookupCode, read: read)
}
check(asked.isEmpty && lookupCodeAddresses.isEmpty, "invalid app IDs or unavailable app pins cause no registry reads")
let wrongAppCodePins = SeaRegistrySources(names: appOnlyPins.names,
                                         apps: .init(address: apps, codeSHA256: String(repeating: "0", count: 64)))
await fails(.wrongCode) {
    _ = try await SeaNameResolver.resolveApp(appID: appID, sources: wrongAppCodePins, code: lookupCode, read: read)
}
check(asked.isEmpty && lookupCodeAddresses == [apps], "a mismatched app code pin blocks currentRelease reads")

var snapshots = 0
var continuallyChanging = false
var wrongChain = false
var rpcCodeAddresses: [String] = []
func rpc(_ method: String, _ params: [Any]) async throws -> Any {
    switch method {
    case "aether_status":
        snapshots += 1
        let height = continuallyChanging ? snapshots : (snapshots == 1 ? 1 : 2)
        return ["chain_id": wrongChain ? 7780 : 1, "height": height, "state_root": hash,
                "hash": hash, "timestamp_ms": 150_000] as [String: Any]
    case "eth_getCode":
        rpcCodeAddresses.append(params[0] as! String)
        return code
    case "eth_call":
        let call = params[0] as! [String: String]
        return try read(call["to"]!, call["data"]!)
    default: fatalError("unexpected RPC method")
    }
}
let coherent = try await SeaRegistryReader.resolve(link, chainID: 1, port: 18545, sources: pins, readRPC: rpc)
check(coherent.app == result.app && snapshots == 4, "a moving snapshot retries the complete lookup")
snapshots = 0
continuallyChanging = true
await fails(.unstable) {
    _ = try await SeaRegistryReader.resolve(link, chainID: 1, port: 18545, sources: pins, readRPC: rpc)
}
check(snapshots == 6, "snapshot retries are bounded")
snapshots = 0
wrongChain = true
await fails(.unstable) {
    _ = try await SeaRegistryReader.resolve(link, chainID: 1, port: 18545, sources: pins, readRPC: rpc)
}
check(snapshots == 1, "a foreign node is rejected before contract reads")

snapshots = 0
continuallyChanging = false
wrongChain = false
bound = false
asked.removeAll()
rpcCodeAddresses.removeAll()
let coherentName = try await SeaRegistryReader.lookup(link, chainID: 1, port: 18545, sources: nameOnlyPins, readRPC: rpc)
check(coherentName == nameRecord && snapshots == 4, "a moving snapshot retries the complete name-only lookup")
check(rpcCodeAddresses == [names, names] && asked.count == 8 && asked.allSatisfy { $0.0 == names }, "snapshot retries never require an app record")
bound = true
snapshots = 0
registered = false
await fails(.unregistered) {
    _ = try await SeaRegistryReader.lookup(link, chainID: 1, port: 18545, sources: nameOnlyPins, readRPC: rpc)
}
check(snapshots == 4, "lookup errors are retried when their snapshot changed")
registered = true
snapshots = 0
continuallyChanging = true
await fails(.unstable) {
    _ = try await SeaRegistryReader.lookup(link, chainID: 1, port: 18545, sources: nameOnlyPins, readRPC: rpc)
}
check(snapshots == 6, "name-only snapshot retries are bounded to three attempts")
snapshots = 0
wrongChain = true
asked.removeAll()
rpcCodeAddresses.removeAll()
await fails(.unstable) {
    _ = try await SeaRegistryReader.lookup(link, chainID: 1, port: 18545, sources: nameOnlyPins, readRPC: rpc)
}
check(snapshots == 1 && asked.isEmpty && rpcCodeAddresses.isEmpty, "name-only lookup rejects a foreign node before contract reads")
snapshots = 0
await fails(.registryUnavailable) {
    _ = try await SeaRegistryReader.lookup(link, chainID: 1, port: 18545, sources: nil, readRPC: rpc)
}
check(snapshots == 0, "name-only lookup does not contact a node without registry sources")
let cancelledLookup = Task {
    withUnsafeCurrentTask { $0?.cancel() }
    return try await SeaRegistryReader.lookup(link, chainID: 1, port: 18545, sources: nameOnlyPins, readRPC: rpc)
}
do {
    _ = try await cancelledLookup.value
    fatalError("expected cancellation")
} catch is CancellationError {
    check(snapshots == 0 && asked.isEmpty && rpcCodeAddresses.isEmpty, "a cancelled lookup performs no node requests")
}
wrongChain = false
continuallyChanging = false
let cancelledDuringLookup = Task {
    try await SeaRegistryReader.lookup(link, chainID: 1, port: 18545, sources: nameOnlyPins, readRPC: { method, params in
        let value = try await rpc(method, params)
        if method == "eth_getCode" { withUnsafeCurrentTask { $0?.cancel() } }
        return value
    })
}
do {
    _ = try await cancelledDuringLookup.value
    fatalError("expected cancellation during lookup")
} catch is CancellationError {
    check(snapshots == 1 && asked.isEmpty && rpcCodeAddresses == [names], "cancellation during lookup stops further reads and snapshot retries")
}

snapshots = 0
asked.removeAll()
rpcCodeAddresses.removeAll()
let coherentApp = try await SeaRegistryReader.resolveApp(appID: appID, chainID: 1, port: 18545, sources: appOnlyPins, readRPC: rpc)
check(coherentApp == directApp && snapshots == 4, "direct app resolution retries a changed finalized snapshot")
check(rpcCodeAddresses == [apps, apps] && asked.count == 2 && asked.allSatisfy { $0.0 == apps },
      "direct app snapshot retries access only the pinned app registry")
snapshots = 0
listed = false
await fails(.unlistedApp) {
    _ = try await SeaRegistryReader.resolveApp(appID: appID, chainID: 1, port: 18545, sources: appOnlyPins, readRPC: rpc)
}
check(snapshots == 4, "an unlisted direct app is reported only from a stable snapshot")
listed = true
snapshots = 0
continuallyChanging = true
await fails(.unstable) {
    _ = try await SeaRegistryReader.resolveApp(appID: appID, chainID: 1, port: 18545, sources: appOnlyPins, readRPC: rpc)
}
check(snapshots == 6, "direct app resolution is bounded to three snapshot attempts")
snapshots = 0
wrongChain = true
asked.removeAll()
rpcCodeAddresses.removeAll()
await fails(.unstable) {
    _ = try await SeaRegistryReader.resolveApp(appID: appID, chainID: 1, port: 18545, sources: appOnlyPins, readRPC: rpc)
}
check(snapshots == 1 && asked.isEmpty && rpcCodeAddresses.isEmpty, "direct app resolution rejects the wrong chain before registry reads")
snapshots = 0
await fails(.registryUnavailable) {
    _ = try await SeaRegistryReader.resolveApp(appID: appID, chainID: 1, port: 18545, sources: nil, readRPC: rpc)
}
check(snapshots == 0, "direct app resolution makes no node request without registry sources")
let cancelledApp = Task {
    withUnsafeCurrentTask { $0?.cancel() }
    return try await SeaRegistryReader.resolveApp(appID: appID, chainID: 1, port: 18545, sources: appOnlyPins, readRPC: rpc)
}
do {
    _ = try await cancelledApp.value
    fatalError("expected direct app cancellation")
} catch is CancellationError {
    check(snapshots == 0 && asked.isEmpty && rpcCodeAddresses.isEmpty, "a cancelled direct app resolution makes no node request")
}
wrongChain = false
continuallyChanging = false
let cancelledDuringApp = Task {
    try await SeaRegistryReader.resolveApp(appID: appID, chainID: 1, port: 18545, sources: appOnlyPins, readRPC: { method, params in
        let value = try await rpc(method, params)
        if method == "eth_getCode" { withUnsafeCurrentTask { $0?.cancel() } }
        return value
    })
}
do {
    _ = try await cancelledDuringApp.value
    fatalError("expected cancellation during direct app resolution")
} catch is CancellationError {
    check(snapshots == 1 && asked.isEmpty && rpcCodeAddresses == [apps], "direct app cancellation stops contract reads and further snapshot requests")
}
print("OK sea-resolution (\(checked) checks)")
