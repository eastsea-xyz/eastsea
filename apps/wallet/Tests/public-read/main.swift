// The public-read preference without an app or a real node directory.
// The runner supplies a repository-owned ./tmp directory for every fixture.
import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

guard let temporaryPath = ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"] else {
    fatalError("AETHER_AGENT_TEST_TMP must point to the workspace's ./tmp directory")
}
let root = URL(fileURLWithPath: temporaryPath, isDirectory: true)
    .appendingPathComponent("public-read-settings-\(UUID().uuidString)", isDirectory: true)
defer { try? FileManager.default.removeItem(at: root) }

let fresh = PublicReadSettings.read(in: root)
check(fresh.settings.enabled, "a node without a saved preference helps browsers by default")
check(fresh.settings.dailyBytes == 268_435_456, "the initial daily cap is 256 MiB")
check(!fresh.needsRepair, "a missing preference is a valid default")
check(!FileManager.default.fileExists(atPath: root.path), "reading defaults does not create a node directory")

let off = PublicReadSettings(enabled: false, dailyBytes: 64 * PublicReadSettings.bytesPerMiB)
try off.write(in: root)
check(PublicReadSettings.read(in: root).settings == off, "turning sharing off persists immediately")
let file = root.appendingPathComponent(PublicReadSettings.fileName)
let saved = try JSONSerialization.jsonObject(with: Data(contentsOf: file)) as! [String: Any]
check(Set(saved.keys) == ["enabled", "daily_bytes"], "the saved schema is the node's enabled/daily_bytes contract")
check(saved["enabled"] as? Bool == false, "the disabled preference reaches the node")
check((saved["daily_bytes"] as? NSNumber)?.uint64Value == 67_108_864, "the MiB choice reaches the node in bytes")

let on = PublicReadSettings(enabled: true, dailyBytes: 512 * PublicReadSettings.bytesPerMiB)
try on.write(in: root)
check(PublicReadSettings.read(in: root).settings == on, "a cap change replaces the previous preference")
check(try FileManager.default.contentsOfDirectory(atPath: root.path) == [PublicReadSettings.fileName],
      "atomic replacement leaves only the applied preference in the node directory")

for cap in [UInt64(0), 1, UInt64.max] {
    let config = PublicReadSettings(enabled: true, dailyBytes: cap)
    try config.write(in: root)
    let read = PublicReadSettings.read(in: root)
    check(read.settings == config && !read.needsRepair, "valid byte caps round-trip exactly, including zero (\(cap))")
}

for malformed in [
    "not json", "{}", "[]", "null",
    "{\"enabled\":true}", "{\"daily_bytes\":268435456}",
    "{\"enabled\":\"true\",\"daily_bytes\":268435456}",
    "{\"enabled\":true,\"daily_bytes\":-1}",
    "{\"enabled\":true,\"daily_bytes\":0.5}",
    "{\"enabled\":true,\"daily_bytes\":\"268435456\"}",
    "{\"enabled\":true,\"daily_bytes\":18446744073709551616}",
    "{\"enabled\":true,\"daily_bytes\":268435456,\"unknown\":true}",
    "{\"enabled\":true,\"daily_bytes\":268435456,\"bytes_per_second\":1048576}"
] {
    try Data(malformed.utf8).write(to: file, options: .atomic)
    let read = PublicReadSettings.read(in: root)
    check(!read.settings.enabled && read.needsRepair, "malformed saved settings fail closed: \(malformed)")
}

// A path occupied by a directory is unavailable, rather than a fresh install.
try FileManager.default.removeItem(at: file)
try FileManager.default.createDirectory(at: file, withIntermediateDirectories: false)
let unreadable = PublicReadSettings.read(in: root)
check(!unreadable.settings.enabled && unreadable.needsRepair, "unreadable saved settings fail closed")
do {
    try on.write(in: root)
    check(false, "a failed save reports an error instead of claiming the setting applied")
} catch {}
check(!PublicReadSettings.read(in: root).settings.enabled, "a failed save cannot enable sharing")

try FileManager.default.removeItem(at: file)
try off.write(in: root)
check(PublicReadSettings.read(in: root).settings == off && !PublicReadSettings.read(in: root).needsRepair,
      "saving a valid preference repairs a malformed one")

print("ok")
