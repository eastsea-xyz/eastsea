// rename-migration: the Aether → EastSea data move (docs/design/25-rename.md
// phase 3; audit 5 A5-5 + A5-7), on throwaway directories. The contract under
// test: the done flag is set only after every precious item is copied AND
// verified; a failed run leaves the old data alone and retries on the next
// launch; no fresh wallet key or node start while an unmigrated old identity
// exists; the old node's run.lock defers the move (and is never copied); a
// symlinked destination is refused; a truncated copy is caught by content.
import Foundation

var failures = 0
func expect(_ ok: Bool, _ what: String) {
    print(ok ? "ok   \(what)" : "FAIL \(what)")
    if !ok { failures += 1 }
}

var suites: [String] = []
/// A fresh temp root standing in for Application Support, pre-populated with
/// an old Aether install: the node tree (identity, threshold, journal, DB,
/// run.lock), the wallet key handle and the update-state file.
func makeOldSupport() -> (root: URL, defaults: UserDefaults) {
    let root = FileManager.default.temporaryDirectory
        .appendingPathComponent("rename-migration-\(UUID().uuidString)", isDirectory: true)
    let node = root.appending(path: "Aether/node")
    try? FileManager.default.createDirectory(at: node, withIntermediateDirectories: true)
    try? "validator-key-bytes".write(to: node.appending(path: "validator.key"), atomically: true, encoding: .utf8)
    try? "node-account-key-bytes".write(to: node.appending(path: "node-account.key"), atomically: true, encoding: .utf8)
    try? "{\"round\":0,\"share\":\"secret\"}".write(to: node.appending(path: "threshold.json"), atomically: true, encoding: .utf8)
    try? "vote\nvote\n".write(to: node.appending(path: "votes.jsonl"), atomically: true, encoding: .utf8)
    try? String(repeating: "chain-data ", count: 500).write(to: node.appending(path: "data.db"), atomically: true, encoding: .utf8)
    FileManager.default.createFile(atPath: node.appending(path: "run.lock").path, contents: Data())
    try? FileManager.default.createDirectory(at: root.appending(path: "AetherWallet"), withIntermediateDirectories: true)
    try? "enclave-handle".write(to: root.appending(path: "AetherWallet/enclave-key.dat"), atomically: true, encoding: .utf8)
    try? "{}".write(to: root.appending(path: "Aether/update-state.json"), atomically: true, encoding: .utf8)
    let suite = "rename-migration-test-\(UUID().uuidString)"
    suites.append(suite)
    return (root, UserDefaults(suiteName: suite)!)
}
func doneFlag(_ d: UserDefaults) -> Bool { d.bool(forKey: "renameMigrationDone") }
func cleanup(_ root: URL, _ d: UserDefaults) {
    try? FileManager.default.removeItem(at: root)
    for s in suites { d.removePersistentDomain(forName: s) }
}

// 0. the hash the verification leans on (FIPS 180-4 vectors).
do {
    expect(DataMigration.sha256Hex(Data("abc".utf8)) == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
           "sha256(abc) matches the FIPS vector")
    expect(DataMigration.sha256Hex(Data()) == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
           "sha256(empty) matches the FIPS vector")
    // Same size, different content: a size-only check would pass, the hash must not.
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("rename-migration-\(UUID().uuidString)", isDirectory: true)
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    try? "aaaa".write(to: dir.appending(path: "a"), atomically: true, encoding: .utf8)
    try? "bbbb".write(to: dir.appending(path: "b"), atomically: true, encoding: .utf8)
    expect(!DataMigration.fileMatches(dir.appending(path: "a"), dir.appending(path: "b")),
           "fileMatches refuses two same-size files with different content")
    try? "aaaa".write(to: dir.appending(path: "c"), atomically: true, encoding: .utf8)
    expect(DataMigration.fileMatches(dir.appending(path: "a"), dir.appending(path: "c")), "fileMatches accepts an exact copy")
    // A truncated copy (audit A5-5's "half a file reads as done") is caught:
    try? "aaa".write(to: dir.appending(path: "d"), atomically: true, encoding: .utf8)
    expect(!DataMigration.fileMatches(dir.appending(path: "a"), dir.appending(path: "d")), "fileMatches catches a truncated copy")
    try? FileManager.default.removeItem(at: dir)
}

// 1. The happy path: everything moves, verifies, and only then is done set.
do {
    let (root, d) = makeOldSupport()
    let outcome = DataMigration.migrate(support: root, defaults: d)
    expect(outcome == .done, "a full old install migrates: \(outcome)")
    expect(doneFlag(d), "done is set after a verified migration")
    let fm = FileManager.default
    expect(!fm.fileExists(atPath: root.appending(path: "Aether/node").path), "the old node tree was moved away (same volume)")
    for f in ["validator.key", "node-account.key", "threshold.json", "votes.jsonl", "data.db"] {
        expect(fm.fileExists(atPath: root.appending(path: "EastSea/node/\(f)").path), "the new tree carries \(f)")
    }
    expect(try String(contentsOf: root.appending(path: "EastSea/node/threshold.json"), encoding: .utf8).contains("secret"),
           "the threshold share arrived intact")
    expect(try String(contentsOf: root.appending(path: "EastSeaWallet/enclave-key.dat"), encoding: .utf8) == "enclave-handle",
           "the key handle arrived intact")
    expect(fm.fileExists(atPath: root.appending(path: "EastSea/update-state.json").path), "update-state.json arrived")
    cleanup(root, d)
}

// 2. A failed run never sets done, and the next launch retries and succeeds.
do {
    let (root, d) = makeOldSupport()
    // An unopenable old data directory: even the lock cannot be taken, so
    // nothing may be touched.
    try? FileManager.default.setAttributes([.posixPermissions: 0],
                                           ofItemAtPath: root.appending(path: "Aether/node").path)
    let outcome = DataMigration.migrate(support: root, defaults: d)
    if case .failed = outcome { expect(true, "an unopenable old data directory fails the migration") }
    else { expect(false, "an unopenable old data directory must fail the migration: \(outcome)") }
    expect(!doneFlag(d), "a failed migration does NOT set done")
    expect(FileManager.default.fileExists(atPath: root.appending(path: "AetherWallet/enclave-key.dat").path),
           "the old key handle is left where it was")
    // Next launch (the audit's retry): permissions restored — and with them
    // the visibility to confirm nothing was touched — the move completes.
    try? FileManager.default.setAttributes([.posixPermissions: 0o755],
                                           ofItemAtPath: root.appending(path: "Aether/node").path)
    expect(FileManager.default.fileExists(atPath: root.appending(path: "Aether/node/data.db").path),
           "the old tree is left exactly where it was (nothing moved)")
    let retry = DataMigration.migrate(support: root, defaults: d)
    expect(retry == .done, "the retried migration completes: \(retry)")
    expect(doneFlag(d), "done is set only after the retry verified everything")
    expect(FileManager.default.fileExists(atPath: root.appending(path: "EastSea/node/validator.key").path),
           "the retried migration brought the identity across")
    cleanup(root, d)
}

// 3. Resume after a partial run: a truncated file from an earlier attempt is
//    detected and replaced, and run.lock is never copied.
do {
    let (root, d) = makeOldSupport()
    let newNode = root.appending(path: "EastSea/node")
    try? FileManager.default.createDirectory(at: newNode, withIntermediateDirectories: true)
    try? "chain-da".write(to: newNode.appending(path: "data.db"), atomically: true, encoding: .utf8) // half a file
    let outcome = DataMigration.migrate(support: root, defaults: d)
    expect(outcome == .done, "a partial earlier attempt is resumed, not trusted: \(outcome)")
    expect(doneFlag(d), "done is set after the resumed migration verified every file")
    expect(DataMigration.fileMatches(root.appending(path: "Aether/node/data.db"), newNode.appending(path: "data.db")),
           "the truncated file was replaced with a verified copy")
    expect(!FileManager.default.fileExists(atPath: newNode.appending(path: "run.lock").path),
           "run.lock is never copied (a copied lock file is not a lock)")
    expect(FileManager.default.fileExists(atPath: root.appending(path: "Aether/node/run.lock").path),
           "the copy fallback keeps the old tree, lock and all")
    expect(FileManager.default.fileExists(atPath: root.appending(path: "Aether/node/MIGRATED-TO-EASTSEA").path),
           "the left-behind old tree is marked migrated")
    cleanup(root, d)
}

// 4. The old node still running defers the move — nothing is touched.
do {
    let (root, d) = makeOldSupport()
    let lockPath = root.appending(path: "Aether/node/run.lock").path
    let fd = open(lockPath, O_RDWR)
    expect(fd >= 0 && flock(fd, LOCK_EX | LOCK_NB) == 0, "the test holds the old node's run.lock")
    let outcome = DataMigration.migrate(support: root, defaults: d)
    if case .deferred(let why) = outcome {
        expect(why.contains("Quit the old Aether app"), "the deferral tells the user what to do: \(why)")
    } else {
        expect(false, "a held run.lock must defer the migration: \(outcome)")
    }
    expect(!doneFlag(d), "a deferred migration does not set done")
    expect(!FileManager.default.fileExists(atPath: root.appending(path: "EastSea/node").path),
           "a deferred migration touches nothing")
    close(fd)
    let retry = DataMigration.migrate(support: root, defaults: d)
    expect(retry == .done, "once the old app quits, the move completes: \(retry)")
    cleanup(root, d)
}

// 5. No fresh identities while an old one waits (audit A5-7).
do {
    let (root, d) = makeOldSupport()
    if let why = DataMigration.mayCreateFreshWalletKey(support: root, defaults: d) {
        expect(why.count > 20, "creating a new wallet key is refused while the old handle waits: \(why)")
    } else {
        expect(false, "a new wallet key must be refused while the old handle is unmigrated")
    }
    if let why = DataMigration.mayStartNode(support: root, defaults: d) {
        expect(why.count > 20, "starting the node is refused while the old identity waits: \(why)")
    } else {
        expect(false, "the node must not start while the old identity is unmigrated")
    }
    _ = DataMigration.migrate(support: root, defaults: d)
    expect(DataMigration.mayCreateFreshWalletKey(support: root, defaults: d) == nil,
           "after a verified migration a new key may be made (fresh installs)")
    expect(DataMigration.mayStartNode(support: root, defaults: d) == nil,
           "after a verified migration the node may start")
    cleanup(root, d)
}
do {
    // A fresh install (no old data anywhere) allows everything immediately.
    let (root, d) = makeOldSupport()
    try? FileManager.default.removeItem(at: root.appending(path: "Aether"))
    try? FileManager.default.removeItem(at: root.appending(path: "AetherWallet"))
    expect(DataMigration.mayCreateFreshWalletKey(support: root, defaults: d) == nil,
           "no old data: a fresh key is allowed")
    expect(DataMigration.mayStartNode(support: root, defaults: d) == nil, "no old data: the node may start")
    expect(DataMigration.migrate(support: root, defaults: d) == DataMigration.Outcome.noOldData,
           "no old data: the migration says so and is done")
    expect(doneFlag(d), "a fresh install is marked done (nothing to retry)")
    cleanup(root, d)
}

// 6. A symlinked destination is refused outright (audit A5-7): the move must
//    not write through a link somebody pointed somewhere else.
do {
    let (root, d) = makeOldSupport()
    let elsewhere = FileManager.default.temporaryDirectory
        .appendingPathComponent("rename-migration-elsewhere-\(UUID().uuidString)", isDirectory: true)
    try? FileManager.default.createDirectory(at: elsewhere, withIntermediateDirectories: true)
    try? FileManager.default.createSymbolicLink(atPath: root.appending(path: "EastSea").path,
                                                withDestinationPath: elsewhere.path)
    let outcome = DataMigration.migrate(support: root, defaults: d)
    if case .failed(let why) = outcome {
        expect(why.lowercased().contains("symbolic"), "a symlinked destination is refused with a reason: \(why)")
    } else {
        expect(false, "a symlinked destination must fail the migration: \(outcome)")
    }
    expect(!FileManager.default.fileExists(atPath: elsewhere.appending(path: "node/data.db").path),
           "nothing was written through the symlink")
    expect(FileManager.default.fileExists(atPath: root.appending(path: "Aether/node/data.db").path),
           "the old tree is untouched by the refusal")
    try? FileManager.default.removeItem(at: root.appending(path: "EastSea"))
    try? FileManager.default.removeItem(at: elsewhere)
    cleanup(root, d)
}

exit(Int32(failures))
