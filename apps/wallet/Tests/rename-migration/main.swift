// rename-migration: the move/copy rules of the Aether → EastSea data move
// (docs/design/25-rename.md phase 3), on throwaway directories.
import Foundation

var failures = 0
func expect(_ ok: Bool, _ what: String) {
    print(ok ? "ok   \(what)" : "FAIL \(what)")
    if !ok { failures += 1 }
}

func tmpDir(_ parts: String...) -> URL {
    let url = FileManager.default.temporaryDirectory
        .appendingPathComponent("rename-migration-\(UUID().uuidString)", isDirectory: true)
        .appending(path: parts.joined(separator: "/"))
    try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}
/// A path under a fresh temp root that is NOT created yet (the migration
/// must make the target itself).
func absentPath(_ parts: String...) -> URL {
    tmpDir().appending(path: parts.joined(separator: "/"))
}

// 1. moveOrCopyTree moves the tree when the target is missing.
do {
    let old = tmpDir("Aether", "node"), new = absentPath("EastSea", "node")
    try "chain".write(to: old.appendingPathComponent("data.db"), atomically: true, encoding: .utf8)
    DataMigration.moveOrCopyTree(old, new)
    expect(FileManager.default.fileExists(atPath: new.appendingPathComponent("data.db").path), "move brings the files")
    expect(!FileManager.default.fileExists(atPath: old.path), "same-volume move vacates the old tree")
}

// 2. An existing target is never touched, and the old tree stays.
do {
    let old = tmpDir("Aether", "node"), new = tmpDir("EastSea", "node")
    try "old".write(to: old.appendingPathComponent("a"), atomically: true, encoding: .utf8)
    try "new".write(to: new.appendingPathComponent("b"), atomically: true, encoding: .utf8)
    DataMigration.moveOrCopyTree(old, new)
    expect(FileManager.default.fileExists(atPath: old.appendingPathComponent("a").path), "existing target keeps the old tree")
    expect(try String(contentsOf: new.appendingPathComponent("b"), encoding: .utf8) == "new", "existing target keeps its own files")
}

// 3. A missing source is a no-op (fresh install).
do {
    let new = tmpDir("EastSea", "node")
    DataMigration.moveOrCopyTree(tmpDir("Aether-absent", "node"), new)
    expect(!FileManager.default.fileExists(atPath: new.appendingPathComponent("anything").path), "missing source is a no-op")
}

// 4. copyIfMissing copies once and never overwrites or deletes.
do {
    let old = tmpDir("AetherWallet"), new = tmpDir("EastSeaWallet")
    try "handle".write(to: old.appendingPathComponent("enclave-key.dat"), atomically: true, encoding: .utf8)
    DataMigration.copyIfMissing(old.appendingPathComponent("enclave-key.dat"), new.appendingPathComponent("enclave-key.dat"))
    DataMigration.copyIfMissing(old.appendingPathComponent("enclave-key.dat"), new.appendingPathComponent("enclave-key.dat"))
    expect(try String(contentsOf: new.appendingPathComponent("enclave-key.dat"), encoding: .utf8) == "handle", "key handle copied")
    expect(FileManager.default.fileExists(atPath: old.appendingPathComponent("enclave-key.dat").path), "old key handle kept")
}

exit(Int32(failures))
