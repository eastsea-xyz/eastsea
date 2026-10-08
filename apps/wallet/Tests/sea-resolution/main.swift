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
var corruptOwner = false
var asked: [(String, String)] = []
func read(_ to: String, _ data: String) throws -> String {
    asked.append((to, data))
    switch String(data.prefix(10)) {
    case "0x864ce5e1": return "0x" + node
    case "0x7dd56411": return corruptOwner ? "0x" + String(repeating: "f", count: 64) : "0x" + String(repeating: "0", count: 24) + owner.dropFirst(2)
    case "0xb25be181": return "0x" + String(repeating: "0", count: 24) + addr.dropFirst(2)
    case "0x9dfcc616": return "0x" + word(expired ? 100 : 200)
    case "0xffdbd0e3": return text(bound ? appID : "")
    case "0x470d9498": return "0x" + word(1) + hash + hash + word(listed ? 1 : 0)
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
print("OK sea-resolution (\(checked) checks)")
