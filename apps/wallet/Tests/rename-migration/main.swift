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

/// The test's defaults: in memory only. A real `UserDefaults(suiteName:)`
/// leaves a plist in ~/Library/Preferences even after removePersistentDomain
/// (cfprefsd writes the emptied domain back, asynchronously) — ~580 of them
/// had piled up, some holding copies of the real Aether preferences. Every
/// accessor the migration uses is answered here; nothing reaches cfprefsd.
final class MemoryDefaults: UserDefaults {
    private var store: [String: Any] = [:]
    private let lock = NSLock()
    init() { super.init(suiteName: nil)! }
    override func object(forKey key: String) -> Any? { lock.lock(); defer { lock.unlock() }; return store[key] }
    override func set(_ value: Any?, forKey key: String) {
        lock.lock(); defer { lock.unlock() }
        if let value { store[key] = value } else { store.removeValue(forKey: key) }
    }
    override func set(_ value: Bool, forKey key: String) { set(value as Any?, forKey: key) }
    override func bool(forKey key: String) -> Bool { (object(forKey: key) as? Bool) ?? false }
    override func removeObject(forKey key: String) { set(nil as Any?, forKey: key) }
}
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
    // The committee's public file and the vote journals at the names the node
    // binary itself reads at its data root (supervisor.rs, main.rs).
    try? "{\"chain_id\":1,\"round\":0}".write(to: node.appending(path: "network.json"), atomically: true, encoding: .utf8)
    try? "validator-pub".write(to: node.appending(path: "validator.pub.json"), atomically: true, encoding: .utf8)
    let journal = node.appending(path: "aether-consensus-r1")
    try? FileManager.default.createDirectory(at: journal, withIntermediateDirectories: true)
    try? "vote-journal-bytes".write(to: journal.appending(path: "0"), atomically: true, encoding: .utf8)
    try? "dkg-round-0".write(to: node.appending(path: "dkg-agreement-genesis-0.journal"), atomically: true, encoding: .utf8)
    FileManager.default.createFile(atPath: node.appending(path: "run.lock").path, contents: Data())
    // The follower's database and its own endpoint key (release-070 review, L1).
    try? FileManager.default.createDirectory(at: node.appending(path: "follow"), withIntermediateDirectories: true)
    try? "follow-db".write(to: node.appending(path: "follow/state.redb"), atomically: true, encoding: .utf8)
    try? "wallet-node-key".write(to: node.appending(path: "follow/wallet-node.key"), atomically: true, encoding: .utf8)
    try? FileManager.default.createDirectory(at: root.appending(path: "AetherWallet"), withIntermediateDirectories: true)
    try? "enclave-handle".write(to: root.appending(path: "AetherWallet/enclave-key.dat"), atomically: true, encoding: .utf8)
    try? "{}".write(to: root.appending(path: "Aether/update-state.json"), atomically: true, encoding: .utf8)
    return (root, MemoryDefaults())
}
func doneFlag(_ d: UserDefaults) -> Bool { d.bool(forKey: "renameMigrationDone") }
/// No real preferences ever reach a test suite: the "old app" domain the
/// tests copy from does not exist (the real com.pipln.aether holds a wallet's
/// history and balances; earlier runs copied them into ~580 suite plists).
let noOldDomain = "rename-migration-test-no-old-app-\(UUID().uuidString)"
func migrate(_ root: URL, _ d: UserDefaults, forceCopy: Bool = false,
             meter: DataMigration.ProgressMeter? = nil) -> DataMigration.Outcome {
    DataMigration.migrate(support: root, defaults: d, oldPreferencesDomain: noOldDomain, forceCopy: forceCopy, meter: meter, locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
}
func cleanup(_ root: URL, _ d: UserDefaults) {
    try? FileManager.default.removeItem(at: root)
}
/// Every preferences plist this test binary could have caused: none may
/// exist after it (its own domain, or a suite named like the old test's).
let prefsDir = FileManager.default.homeDirectoryForCurrentUser.appending(path: "Library/Preferences")
func ownPlists() -> Set<String> {
    let names = (try? FileManager.default.contentsOfDirectory(atPath: prefsDir.path)) ?? []
    let me = Bundle.main.bundleIdentifier ?? ProcessInfo.processInfo.processName
    return Set(names.filter { $0.hasPrefix("rename-migration-test-") || $0 == "\(me).plist" })
}
let plistsBefore = ownPlists()

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
    let outcome = migrate(root, d)
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
    let outcome = migrate(root, d)
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
    let retry = migrate(root, d)
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
    let outcome = migrate(root, d)
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
    let outcome = migrate(root, d)
    if case .deferred(let why) = outcome {
        expect(why.contains("Quit the old Aether app"), "the deferral tells the user what to do: \(why)")
    } else {
        expect(false, "a held run.lock must defer the migration: \(outcome)")
    }
    expect(!doneFlag(d), "a deferred migration does not set done")
    expect(!FileManager.default.fileExists(atPath: root.appending(path: "EastSea/node").path),
           "a deferred migration touches nothing")
    close(fd)
    let retry = migrate(root, d)
    expect(retry == .done, "once the old app quits, the move completes: \(retry)")
    cleanup(root, d)
}

// 5. No fresh identities while an old one waits (audit A5-7).
do {
    let (root, d) = makeOldSupport()
    if let why = DataMigration.mayCreateFreshWalletKey(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) {
        expect(why.count > 20, "creating a new wallet key is refused while the old handle waits: \(why)")
    } else {
        expect(false, "a new wallet key must be refused while the old handle is unmigrated")
    }
    if let why = DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) {
        expect(why.count > 20, "starting the node is refused while the old identity waits: \(why)")
    } else {
        expect(false, "the node must not start while the old identity is unmigrated")
    }
    _ = migrate(root, d)
    expect(DataMigration.mayCreateFreshWalletKey(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil,
           "after a verified migration a new key may be made (fresh installs)")
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil,
           "after a verified migration the node may start")
    cleanup(root, d)
}
do {
    // A fresh install (no old data anywhere) allows everything immediately.
    let (root, d) = makeOldSupport()
    try? FileManager.default.removeItem(at: root.appending(path: "Aether"))
    try? FileManager.default.removeItem(at: root.appending(path: "AetherWallet"))
    expect(DataMigration.mayCreateFreshWalletKey(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil,
           "no old data: a fresh key is allowed")
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil, "no old data: the node may start")
    expect(migrate(root, d) == DataMigration.Outcome.noOldData,
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
    let outcome = migrate(root, d)
    if case .failed(let why) = outcome {
        expect(why.contains("Remove the link"), "a symlinked destination is refused with a reason: \(why)")
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

// 7. The cross-volume path (audit 6, A6-5; release-070 review M2): forced
//    here with `forceCopy`, which takes exactly the branch two different
//    volume identifiers take — the verified copy, with an earlier partial
//    attempt in the destination as well. The old tree stays behind and must
//    not keep a usable signing identity for any binary to pick up.
do {
    let (root, d) = makeOldSupport()
    let newNode = root.appending(path: "EastSea/node")
    try? FileManager.default.createDirectory(at: newNode, withIntermediateDirectories: true)
    try? "earlier partial attempt".write(to: newNode.appending(path: "data.db"), atomically: true, encoding: .utf8)
    let outcome = migrate(root, d, forceCopy: true)
    expect(outcome == .done, "the forced-copy migration completes: \(outcome)")
    expect(doneFlag(d), "done is set after the verified copy")
    let fm = FileManager.default
    let oldNode = root.appending(path: "Aether/node")
    expect(fm.fileExists(atPath: oldNode.path), "the copy fallback keeps the old tree")
    // The old root no longer holds anything a signer can use (A6-5).
    for gone in ["validator.key", "validator.pub.json", "node-account.key", "threshold.json",
                 "aether-consensus-r1", "dkg-agreement-genesis-0.journal", "follow/wallet-node.key"] {
        expect(!fm.fileExists(atPath: oldNode.appending(path: gone).path), "the old root no longer holds \(gone)")
    }
    expect(fm.fileExists(atPath: newNode.appending(path: "follow/wallet-node.key").path),
           "the new tree carries the follower's endpoint key")
    // ...while the quarantine directory inside it does (recoverable).
    let quarantined = ((try? fm.contentsOfDirectory(atPath: oldNode.path)) ?? [])
        .filter { $0.hasPrefix("eastsea-quarantine-") }
    expect(quarantined.count == 1, "one quarantine directory: \(quarantined)")
    if let q = quarantined.first {
        for kept in ["validator.key", "validator.pub.json", "node-account.key", "threshold.json",
                     "aether-consensus-r1", "dkg-agreement-genesis-0.journal", "follow/wallet-node.key"] {
            expect(fm.fileExists(atPath: oldNode.appending(path: q).appending(path: kept).path),
                   "the quarantine holds \(kept) for recovery")
        }
    }
    // network.json and run.lock stay: the old binary's identity guard keeps
    // refusing a fresh key (candidate.rs registered_identity).
    expect(fm.fileExists(atPath: oldNode.appending(path: "network.json").path), "network.json stays in the old root")
    expect(fm.fileExists(atPath: oldNode.appending(path: "run.lock").path), "run.lock stays in the old root")
    expect(fm.fileExists(atPath: oldNode.appending(path: "MIGRATED-TO-EASTSEA").path), "the marker is left")
    for f in ["validator.key", "threshold.json", "network.json", "data.db"] {
        expect(fm.fileExists(atPath: newNode.appending(path: f).path), "the new tree carries \(f)")
    }
    expect(try String(contentsOf: newNode.appending(path: "data.db"), encoding: .utf8)
        == String(repeating: "chain-data ", count: 500), "the partial destination file was replaced with the real database")
    cleanup(root, d)
}

/// The old binary's own gates, as source facts (candidate.rs
/// `registered_identity`, supervisor.rs `role`/`my_key`): signing needs a
/// readable validator.key AND threshold.json, and a fresh key is only ever
/// minted in a directory that never held an identity.
func oldBinaryView(_ node: URL) -> (canSign: Bool, refusesFreshIdentity: Bool) {
    let fm = FileManager.default
    let marks = ["validator.key", "validator.pub.json", "node-account.key", "network.json", "threshold.json"]
    let registered = marks.contains { fm.fileExists(atPath: node.appending(path: $0).path) }
        || ((try? fm.contentsOfDirectory(atPath: node.path)) ?? []).contains {
            $0.hasPrefix("stale-") || $0.hasPrefix("corrupt-")
        }
    let canSign = fm.fileExists(atPath: node.appending(path: "validator.key").path)
        && fm.fileExists(atPath: node.appending(path: "threshold.json").path)
    return (canSign, registered)
}

// 8. The old-binary view of the old tree after a copy migration (audit 6,
//    A6-5): before it, the old app can sign; after the verified copy and
//    quarantine, it can neither sign nor mint a fresh identity.
do {
    let (root, d) = makeOldSupport()
    let oldNode = root.appending(path: "Aether/node")
    var view = oldBinaryView(oldNode)
    expect(view.canSign, "before the migration the old tree can sign")
    expect(view.refusesFreshIdentity, "before the migration the old root is a registered identity")
    // Force the copy path, as in scenario 7.
    let newNode = root.appending(path: "EastSea/node")
    try? FileManager.default.createDirectory(at: newNode, withIntermediateDirectories: true)
    try? "x".write(to: newNode.appending(path: "data.db"), atomically: true, encoding: .utf8)
    _ = migrate(root, d)
    view = oldBinaryView(oldNode)
    expect(!view.canSign, "after the migration the old binary can no longer sign (no key, no share)")
    expect(view.refusesFreshIdentity, "the old root still refuses to mint a fresh identity (network.json stays)")
    // The same-volume path removes the old tree entirely — also unsignable.
    let (root2, d2) = makeOldSupport()
    _ = migrate(root2, d2)
    expect(!FileManager.default.fileExists(atPath: root2.appending(path: "Aether/node").path),
           "the same-volume move leaves no old tree at all")
    cleanup(root, d)
    cleanup(root2, d2)
}

// 9. A same-size, different-content file beyond the old 4 MiB hash limit
//    (audit 6, A6-6): the streaming hash must catch it and the migration must
//    replace the destination file before ever marking done.
do {
    let (root, d) = makeOldSupport()
    let oldNode = root.appending(path: "Aether/node")
    let newNode = root.appending(path: "EastSea/node")
    // 5 MiB files, one byte apart, both beyond 4 MiB.
    let big = Data(repeating: 0x41, count: 5 << 20)
    try? big.write(to: oldNode.appending(path: "big.db"))
    var poisoned = big
    poisoned[1 << 20] = 0x42
    try? FileManager.default.createDirectory(at: newNode, withIntermediateDirectories: true)
    try? poisoned.write(to: newNode.appending(path: "big.db"))
    try? "earlier attempt".write(to: newNode.appending(path: "data.db"), atomically: true, encoding: .utf8)
    expect(!DataMigration.fileMatches(oldNode.appending(path: "big.db"), newNode.appending(path: "big.db")),
           "two 5 MiB files one byte apart do NOT match")
    let outcome = migrate(root, d)
    expect(outcome == .done, "the migration recopies and verifies: \(outcome)")
    expect(DataMigration.fileMatches(oldNode.appending(path: "big.db"), newNode.appending(path: "big.db")),
           "the poisoned destination was replaced with a verified copy")
    expect(try Data(contentsOf: newNode.appending(path: "big.db")) == big, "the recopied bytes are exact")
    cleanup(root, d)
}

// 10. A partially-copied destination is not a green light for the node (audit
//     6, A6-6): while the old tree waits, mayStartNode refuses even when
//     EastSea/node already exists.
do {
    let (root, d) = makeOldSupport()
    let newNode = root.appending(path: "EastSea/node")
    // A half-copied tree from an interrupted run.
    try? FileManager.default.createDirectory(at: newNode, withIntermediateDirectories: true)
    try? "chain-da".write(to: newNode.appending(path: "data.db"), atomically: true, encoding: .utf8)
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) != nil,
           "a partial destination tree still refuses the node start")
    // So does an empty placeholder directory.
    try? FileManager.default.removeItem(at: newNode.appending(path: "data.db"))
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) != nil,
           "an empty destination directory still refuses the node start")
    // The same-volume completion clears the refusal.
    expect(migrate(root, d) == .done, "the migration completes over the placeholder")
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil, "a completed migration allows the node")
    cleanup(root, d)
}

// 11. An interrupted quarantine resumes (audit 6, A6-5): the old root may sit
//     mid-quarantine — some signing files moved, the rest still in place.
//     Re-running must finish the job idempotently, not fail or skip.
do {
    let (root, d) = makeOldSupport()
    let oldNode = root.appending(path: "Aether/node")
    let newNode = root.appending(path: "EastSea/node")
    // Force the copy path, then let the verified copy finish but "crash"
    // right after the first quarantine rename.
    try? FileManager.default.createDirectory(at: newNode, withIntermediateDirectories: true)
    try? "x".write(to: newNode.appending(path: "data.db"), atomically: true, encoding: .utf8)
    expect(DataMigration.syncTreeVerified(oldNode, newNode), "the tree copy verifies on its own")
    let half = oldNode.appending(path: "eastsea-quarantine-12345")
    try? FileManager.default.createDirectory(at: half, withIntermediateDirectories: true)
    try? FileManager.default.moveItem(at: oldNode.appending(path: "validator.key"), to: half.appending(path: "validator.key"))
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) != nil,
           "a half-quarantined old tree still refuses the node start")
    expect(oldBinaryView(oldNode).canSign == false, "with the key gone the old binary already cannot sign")
    let outcome = migrate(root, d)
    expect(outcome == .done, "the resumed migration finishes: \(outcome)")
    expect(doneFlag(d), "done is set after the resumed quarantine")
    for gone in ["validator.key", "threshold.json", "node-account.key", "validator.pub.json",
                 "aether-consensus-r1", "dkg-agreement-genesis-0.journal"] {
        expect(!FileManager.default.fileExists(atPath: oldNode.appending(path: gone).path),
               "the resumed quarantine removed \(gone) from the old root")
    }
    expect(FileManager.default.fileExists(atPath: half.appending(path: "validator.key").path),
           "the earlier half-quarantine is respected, not duplicated away")
    let quarantines = ((try? FileManager.default.contentsOfDirectory(atPath: oldNode.path)) ?? [])
        .filter { $0.hasPrefix("eastsea-quarantine-") }
    var restored = 0
    for q in quarantines {
        for name in ["validator.key", "threshold.json", "node-account.key", "validator.pub.json",
                     "aether-consensus-r1", "dkg-agreement-genesis-0.journal"]
            where FileManager.default.fileExists(atPath: oldNode.appending(path: q).appending(path: name).path) {
            restored += 1
        }
    }
    expect(restored == 6, "every quarantined item is recoverable from the quarantine directories: \(restored)")
    expect(FileManager.default.fileExists(atPath: oldNode.appending(path: "network.json").path),
           "network.json stays for the identity guard")
    cleanup(root, d)
}

// 12. Pre-audit 7, H2: a failure AFTER the old tree's quarantine (here the
//     migrated marker cannot be written — a directory squats on its name)
//     must not strand the new node. The node's own completion state is set,
//     the new node may start, exactly one usable signer remains, and the
//     next launch finishes the cosmetic tail and sets done.
do {
    let (root, d) = makeOldSupport()
    let oldNode = root.appending(path: "Aether/node")
    let newNode = root.appending(path: "EastSea/node")
    // Force the copy path, as in scenario 7.
    try? FileManager.default.createDirectory(at: newNode, withIntermediateDirectories: true)
    try? "x".write(to: newNode.appending(path: "data.db"), atomically: true, encoding: .utf8)
    // The marker write after the quarantine will fail: a plain file write
    // cannot replace a directory squatting on the marker's name.
    try? FileManager.default.createDirectory(at: oldNode.appending(path: "MIGRATED-TO-EASTSEA"),
                                             withIntermediateDirectories: true)
    let outcome = migrate(root, d)
    if case .failed(let why) = outcome {
        expect(why.contains("The data move could not be completed"), "the post-quarantine marker failure reports itself: \(why)")
    } else {
        expect(false, "a marker write failure after the quarantine must fail the run: \(outcome)")
    }
    expect(!doneFlag(d), "done is not set while the marker tail is pending")
    expect(d.bool(forKey: "renameNodeMigrationDone"), "the node's own completion state IS set (the quarantine finished)")
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil,
           "H2: the verified new node may start after the quarantine, marker or no marker")
    // Exactly one usable signer: the old tree cannot, the new tree can.
    expect(oldBinaryView(oldNode).canSign == false, "the quarantined old tree cannot sign")
    expect(FileManager.default.fileExists(atPath: newNode.appending(path: "validator.key").path)
        && FileManager.default.fileExists(atPath: newNode.appending(path: "threshold.json").path),
           "the new tree holds the signing pair (the one usable signer)")
    // The next launch clears the obstacle and finishes the tail.
    try? FileManager.default.removeItem(at: oldNode.appending(path: "MIGRATED-TO-EASTSEA"))
    expect(migrate(root, d) == .done, "the next launch finishes the tail")
    expect(doneFlag(d), "done is set once the tail completes")
    cleanup(root, d)
}

// 13. Pre-audit 7, H2: a failure BEFORE the quarantine (here the wallet key
//     handle cannot be copied — the destination folder is unreadable) must
//     leave the old app a usable signer and keep the new node off. Nothing
//     is quarantined while anything fallible can still fail.
do {
    let (root, d) = makeOldSupport()
    let oldNode = root.appending(path: "Aether/node")
    let newNode = root.appending(path: "EastSea/node")
    // Force the copy path, then break the small-file copy: the destination
    // folder exists but cannot be written into.
    try? FileManager.default.createDirectory(at: newNode, withIntermediateDirectories: true)
    try? "x".write(to: newNode.appending(path: "data.db"), atomically: true, encoding: .utf8)
    try? FileManager.default.createDirectory(at: root.appending(path: "EastSeaWallet"),
                                             withIntermediateDirectories: true)
    try? FileManager.default.setAttributes([.posixPermissions: 0],
                                           ofItemAtPath: root.appending(path: "EastSeaWallet").path)
    let outcome = migrate(root, d)
    if case .failed(let why) = outcome {
        expect(why.contains("The data move could not be completed"), "the pre-quarantine copy failure reports itself: \(why)")
    } else {
        expect(false, "a small-file copy failure must fail the run: \(outcome)")
    }
    expect(!doneFlag(d), "done is not set")
    expect(!d.bool(forKey: "renameNodeMigrationDone"), "the node completion state is not set: nothing was quarantined")
    expect(oldBinaryView(oldNode).canSign, "H2: the old tree is still a usable signer (nothing was stranded)")
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) != nil,
           "the new node stays off while the old signer lives (A6-5)")
    let quarantines = ((try? FileManager.default.contentsOfDirectory(atPath: oldNode.path)) ?? [])
        .filter { $0.hasPrefix("eastsea-quarantine-") }
    expect(quarantines.isEmpty, "no quarantine happened before the fallible copies finished")
    // Repair and retry: everything completes.
    try? FileManager.default.setAttributes([.posixPermissions: 0o755],
                                           ofItemAtPath: root.appending(path: "EastSeaWallet").path)
    expect(migrate(root, d) == .done, "the repaired retry completes")
    expect(oldBinaryView(oldNode).canSign == false, "after the full run the old tree no longer signs")
    cleanup(root, d)
}

// 14. Pre-audit 7, H2, the crash window: the process dies between the last
//     quarantine rename and the flag write. On the next launch the state is
//     read from the trees themselves — a marked, signer-clean old root and
//     a verified new tree let the node start, and the idempotent re-run
//     finishes the migration.
do {
    let (root, d) = makeOldSupport()
    let newNode = root.appending(path: "EastSea/node")
    // Force the copy path and run the migration to completion…
    try? FileManager.default.createDirectory(at: newNode, withIntermediateDirectories: true)
    try? "x".write(to: newNode.appending(path: "data.db"), atomically: true, encoding: .utf8)
    expect(migrate(root, d) == .done, "the migration completes first")
    // …then simulate the crash: neither flag made it to disk.
    d.removeObject(forKey: "renameMigrationDone")
    d.removeObject(forKey: "renameNodeMigrationDone")
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil,
           "the crash window heals: a marked, signer-clean old tree lets the verified node start")
    expect(migrate(root, d) == .done, "the re-run finishes idempotently")
    expect(doneFlag(d), "done is set again after the heal")
    cleanup(root, d)
}

// 15. Release-070 review, B4 — the exact scenario on the founder's Mac: a done
//     flag left from an earlier EastSea build (renameMigrationDone = true)
//     while the live data is back in Aether/ and EastSea/ does not exist.
//     The old code returned .done at once, moved nothing, and opened both
//     gates: the next launch minted a new wallet key and a new validator.
do {
    let (root, d) = makeOldSupport()
    let fm = FileManager.default
    try? "aabb:0xbeaconer".write(to: root.appending(path: "Aether/node.identity"), atomically: true, encoding: .utf8)
    d.set(true, forKey: "renameMigrationDone")
    d.set(true, forKey: "renameNodeMigrationDone")
    let waiting = DataMigration.unmigratedOldData(support: root)
    expect(waiting.contains("Aether/node") && waiting.contains("AetherWallet/enclave-key.dat")
           && waiting.contains("Aether/node.identity"), "B4: the disk says the old data is unmigrated: \(waiting)")
    expect(DataMigration.mayCreateFreshWalletKey(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) != nil,
           "B4: a stale done flag does not let a new wallet key be minted over an unmigrated old one")
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) != nil,
           "B4: a stale done flag does not let the node (and candidate-info) mint a new validator identity")
    let outcome = migrate(root, d)
    expect(outcome == .done, "B4: the migration runs despite the stale flag: \(outcome)")
    expect(!fm.fileExists(atPath: root.appending(path: "Aether/node").path), "B4: the old tree moved (same volume)")
    expect((try? String(contentsOf: root.appending(path: "EastSea/node/validator.key"), encoding: .utf8)) == "validator-key-bytes",
           "B4: the old validator identity is in the new home")
    expect((try? String(contentsOf: root.appending(path: "EastSeaWallet/enclave-key.dat"), encoding: .utf8)) == "enclave-handle",
           "B4: the old wallet key handle is in the new home")
    expect((try? String(contentsOf: root.appending(path: "EastSea/node.identity"), encoding: .utf8)) == "aabb:0xbeaconer",
           "B4: the identity guard came along, so the new node can never mint a fresh identity there")
    expect(fm.fileExists(atPath: root.appending(path: "Aether/node.identity").path), "the old identity guard stays")
    expect(DataMigration.unmigratedOldData(support: root).isEmpty, "nothing is waiting any more")
    expect(DataMigration.mayCreateFreshWalletKey(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil
           && DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil, "after the real migration both gates open")
    expect(doneFlag(d), "done is set again, now truthfully")
    cleanup(root, d)
}

// 16. B4 with the old app still running: the stale flag is dropped and the
//     run defers — the gates stay shut until the move really happens.
do {
    let (root, d) = makeOldSupport()
    d.set(true, forKey: "renameMigrationDone")
    d.set(true, forKey: "renameNodeMigrationDone")
    let fd = open(root.appending(path: "Aether/node/run.lock").path, O_RDWR)
    _ = flock(fd, LOCK_EX | LOCK_NB)
    let outcome = migrate(root, d)
    if case .deferred = outcome { expect(true, "B4: a stale flag with the old app running defers") }
    else { expect(false, "B4: a stale flag with the old app running must defer: \(outcome)") }
    expect(!doneFlag(d) && !d.bool(forKey: "renameNodeMigrationDone"), "B4: the stale flags are gone")
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) != nil
           && DataMigration.mayCreateFreshWalletKey(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) != nil,
           "B4: both gates stay shut while the deferred move waits")
    close(fd)
    expect(migrate(root, d) == .done, "once the old app quits, the move completes")
    cleanup(root, d)
}

// 17. B4, a tester who already fell into the trap: an earlier EastSea build
//     minted a DIFFERENT identity in EastSea/node (and pinned it in
//     EastSea/node.identity) while the flag hid the old one. The old identity
//     (the one the chain knows) wins; the minted one is set aside, never
//     deleted, and the new guard pinning it moves aside too.
do {
    let (root, d) = makeOldSupport()
    let fm = FileManager.default
    let newNode = root.appending(path: "EastSea/node")
    try? fm.createDirectory(at: newNode, withIntermediateDirectories: true)
    try? "minted-key".write(to: newNode.appending(path: "validator.key"), atomically: true, encoding: .utf8)
    try? "minted-account".write(to: newNode.appending(path: "node-account.key"), atomically: true, encoding: .utf8)
    try? "minted:0xnew".write(to: root.appending(path: "EastSea/node.identity"), atomically: true, encoding: .utf8)
    try? "aabb:0xold".write(to: root.appending(path: "Aether/node.identity"), atomically: true, encoding: .utf8)
    // …and a second wallet key handle minted the same way.
    try? fm.createDirectory(at: root.appending(path: "EastSeaWallet"), withIntermediateDirectories: true)
    try? "minted-handle".write(to: root.appending(path: "EastSeaWallet/enclave-key.dat"), atomically: true, encoding: .utf8)
    d.set(true, forKey: "renameMigrationDone")
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) != nil,
           "B4: an old root that still holds an identity keeps the node off, flag or not")
    expect(migrate(root, d) == .done, "B4: the resumed migration completes over the minted identity")
    expect((try? String(contentsOf: newNode.appending(path: "validator.key"), encoding: .utf8)) == "validator-key-bytes",
           "B4: the old identity is the one in the new home")
    let aside = ((try? fm.contentsOfDirectory(atPath: newNode.path)) ?? []).filter { $0.hasPrefix("eastsea-replaced-") }
    expect(aside.count == 1, "the minted identity is set aside in one place: \(aside)")
    if let a = aside.first {
        expect((try? String(contentsOf: newNode.appending(path: "\(a)/validator.key"), encoding: .utf8)) == "minted-key"
               && fm.fileExists(atPath: newNode.appending(path: "\(a)/node-account.key").path),
               "the minted keys are kept for recovery, not deleted")
    }
    expect((try? String(contentsOf: root.appending(path: "EastSea/node.identity"), encoding: .utf8)) == "aabb:0xold",
           "the new identity guard now pins the old identity")
    let guards = ((try? fm.contentsOfDirectory(atPath: root.appending(path: "EastSea").path)) ?? [])
        .filter { $0.hasPrefix("node.identity.eastsea-replaced-") }
    expect(guards.count == 1, "the guard that pinned the minted identity is kept aside: \(guards)")
    expect(oldBinaryView(root.appending(path: "Aether/node")).canSign == false, "exactly one signer: the old tree is quarantined")
    expect((try? String(contentsOf: root.appending(path: "EastSeaWallet/enclave-key.dat"), encoding: .utf8)) == "enclave-handle",
           "B4: the old wallet key (the first address) is the wallet again")
    let handles = (try? fm.contentsOfDirectory(atPath: root.appending(path: "EastSeaWallet").path)) ?? []
    expect(handles.contains { $0.hasPrefix("enclave-key.dat.eastsea-replaced-") },
           "the minted handle is kept aside (its Secure Enclave key stays usable by hand): \(handles)")
    cleanup(root, d)
}

// 18. B4, only the identity guard is left: Aether/node.identity exists, the
//     old tree is gone and EastSea/node holds no identity. A node started now
//     would mint a new identity (no EastSea/node.identity): refused, and the
//     migration brings the guard over so the node's own check refuses too.
do {
    let (root, d) = makeOldSupport()
    let fm = FileManager.default
    try? fm.removeItem(at: root.appending(path: "Aether/node"))
    try? fm.removeItem(at: root.appending(path: "AetherWallet"))
    try? fm.removeItem(at: root.appending(path: "Aether/update-state.json"))
    try? "aabb:0xold".write(to: root.appending(path: "Aether/node.identity"), atomically: true, encoding: .utf8)
    try? fm.createDirectory(at: root.appending(path: "EastSea/node"), withIntermediateDirectories: true)
    d.set(true, forKey: "renameMigrationDone")
    expect(DataMigration.unmigratedOldData(support: root) == ["Aether/node.identity"], "B4: the lone guard is noticed")
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) != nil, "B4: no node start that would mint a second identity")
    expect(migrate(root, d) == .done, "the guard-only migration completes")
    expect(fm.fileExists(atPath: root.appending(path: "EastSea/node.identity").path), "the guard now sits beside the new tree")
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil, "the gate opens: the node's own guard takes over")
    cleanup(root, d)
}

// 19. B4 without false alarms: after a real migration the old app (still
//     installed, opened by its login item) recreates Aether/node as an
//     identity-less follower. That is not data to move: the flag holds and
//     the new tree is left alone.
do {
    let (root, d) = makeOldSupport()
    let fm = FileManager.default
    try? "aabb:0xold".write(to: root.appending(path: "Aether/node.identity"), atomically: true, encoding: .utf8)
    expect(migrate(root, d) == .done, "the first migration completes")
    let recreated = root.appending(path: "Aether/node/follow")
    try? fm.createDirectory(at: recreated, withIntermediateDirectories: true)
    try? "resynced".write(to: recreated.appending(path: "state.redb"), atomically: true, encoding: .utf8)
    expect(DataMigration.unmigratedOldData(support: root).isEmpty, "a recreated identity-less old tree is not unmigrated data")
    expect(migrate(root, d) == .done, "the flag holds")
    expect((try? String(contentsOf: root.appending(path: "EastSea/node/follow/state.redb"), encoding: .utf8)) == "follow-db",
           "the new tree is untouched by the old app's resync")
    expect(DataMigration.mayStartNode(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil, "the node may start")
    cleanup(root, d)
}

// 20. M2: the path is chosen by volume identifiers. One volume renames (no
//     bytes hashed, nothing left behind); `forceCopy` (what two different
//     identifiers choose) hashes every byte it copies and leaves the old tree.
do {
    let tmp = FileManager.default.temporaryDirectory
    expect(DataMigration.sameVolume(tmp, tmp) == true, "a directory is on its own volume")
    expect(DataMigration.sameVolume(tmp, URL(fileURLWithPath: "/nonexistent-\(UUID().uuidString)")) == nil,
           "an unreadable volume is unknown (callers then take the verified copy)")
    let (root, d) = makeOldSupport()
    var fractions: [Double] = []
    let meter = DataMigration.ProgressMeter { fractions.append($0) }
    expect(migrate(root, d, meter: meter) == .done, "the same-volume run completes")
    expect(meter.total == 0, "a same-volume rename hashes nothing (no copy work expected)")
    expect(!FileManager.default.fileExists(atPath: root.appending(path: "Aether/node").path), "and leaves no old tree")
    cleanup(root, d)

    let (root2, d2) = makeOldSupport()
    let big = Data(repeating: 0x5a, count: 70 << 20)   // enough for a progress report
    try? big.write(to: root2.appending(path: "Aether/node/state.redb"))
    let meter2 = DataMigration.ProgressMeter { fractions.append($0) }
    expect(migrate(root2, d2, forceCopy: true, meter: meter2) == .done, "the forced verified copy completes")
    expect(meter2.total >= Int64(big.count) * 3, "the copy path expects copy + two hashes of every byte: \(meter2.total)")
    expect(!fractions.isEmpty && fractions.allSatisfy { $0 > 0 && $0 <= 1 }, "progress was reported as a fraction: \(fractions)")
    expect(FileManager.default.fileExists(atPath: root2.appending(path: "Aether/node/MIGRATED-TO-EASTSEA").path),
           "the copied-from tree stays, marked")
    expect(!FileManager.default.fileExists(atPath: root2.appending(path: "EastSea/node/run.lock").path), "run.lock was not copied")
    cleanup(root2, d2)
}

// 21. L2: concurrent callers are serialized. Eight threads ask at once; every
//     one sees .done (none a spurious "quit the old app" from contending for
//     the old run.lock, none a .failed from racing over the same files).
do {
    let (root, d) = makeOldSupport()
    let runner = DataMigration.Runner(support: root, defaults: d, oldPreferencesDomain: noOldDomain, locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
    let results = NSMutableArray()
    DispatchQueue.concurrentPerform(iterations: 8) { _ in
        let o = runner.runNow()
        objc_sync_enter(results); results.add("\(o)"); objc_sync_exit(results)
    }
    let all = results.compactMap { $0 as? String }
    expect(all.count == 8 && all.allSatisfy { $0 == "done" }, "L2: eight concurrent callers all see .done: \(all)")
    expect(FileManager.default.fileExists(atPath: root.appending(path: "EastSea/node/validator.key").path), "and the move happened once")
    cleanup(root, d)
}

// 22. M1: from the main thread the slow path never blocks — it answers
//     .running at once, the gates refuse with the "moving" sentence while it
//     works, and its result arrives through onFinish.
do {
    expect(Thread.isMainThread, "this test runs on the main thread")
    let (root, d) = makeOldSupport()
    try? Data(repeating: 0x33, count: 8 << 20).write(to: root.appending(path: "Aether/node/state.redb"))
    let runner = DataMigration.Runner(support: root, defaults: d, oldPreferencesDomain: noOldDomain, forceCopy: true, locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
    expect(runner.expectsLongRun(), "a copy across volumes is the long path")
    let finished = DispatchSemaphore(value: 0)
    var final: DataMigration.Outcome?
    runner.onFinish = { final = $0; finished.signal() }
    let started = Date()
    let first = runner.ensureFromMain()
    let returnedIn = Date().timeIntervalSince(started)
    if case .running = first { expect(true, "M1: the main thread gets .running at once (\(Int(returnedIn * 1000)) ms)") }
    else { expect(false, "M1: the main thread must not run the slow path inline: \(first)") }
    if runner.isRunning {
        expect(DataMigration.mayStartNode(support: root, defaults: d, runner: runner, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == DataMigration.movingSentence(locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
               && DataMigration.mayCreateFreshWalletKey(support: root, defaults: d, runner: runner, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == DataMigration.movingSentence(locale: walletTestLocale("en"), bundle: walletTestBundle("en")),
               "M1: both gates say the data is moving while it moves")
    }
    if case .running = runner.ensureFromMain() { expect(true, "a second call does not start a second run") }
    else if runner.isRunning { expect(false, "a second call while running must answer .running") }
    expect(finished.wait(timeout: .now() + 60) == .success, "the background run finishes")
    expect(final == .done, "M1: the background run's outcome is reported: \(String(describing: final))")
    expect(!runner.isRunning && runner.ensureFromMain() == .done, "after it, the process is settled")
    expect(DataMigration.mayStartNode(support: root, defaults: d, runner: runner, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil, "the gates open after the run")
    cleanup(root, d)
}

// 23. A different wallet handle already at the new home during a first
//     (not stale) migration — the remains of an interrupted copy — is set
//     aside beside it, never deleted, and the old handle lands atomically.
do {
    let (root, d) = makeOldSupport()
    let fm = FileManager.default
    let newWallet = root.appending(path: "EastSeaWallet")
    try? fm.createDirectory(at: newWallet, withIntermediateDirectories: true)
    try? "encl".write(to: newWallet.appending(path: "enclave-key.dat"), atomically: true, encoding: .utf8)
    expect(migrate(root, d) == .done, "the migration completes over a partial handle")
    expect((try? String(contentsOf: newWallet.appending(path: "enclave-key.dat"), encoding: .utf8)) == "enclave-handle",
           "the old handle is in place")
    let names = (try? fm.contentsOfDirectory(atPath: newWallet.path)) ?? []
    expect(names.contains { $0.hasPrefix("enclave-key.dat.eastsea-replaced-") }, "the previous file is kept aside: \(names)")
    expect(!names.contains { $0.contains(".migrating-") }, "no temporary copy is left behind: \(names)")
    cleanup(root, d)
}

// 24. The founder's MacBook (2026-10-07): EastSea 0.7.0 launched while
//     Aether 0.6.6 still held the old run.lock, so the move deferred and the
//     node switch (copied with the old preferences only at the end of the
//     move) read off when the node resumed at launch. 0.6.6 then quit; the
//     next main-thread `ensure()` finished the move inline — the fast path —
//     and told nobody: no onFinish, so the node was never asked to start.
//     The "quit the old app" button's own retry found the run settled and was
//     a no-op, silent too. Every run that settles reports it, and a start()
//     that finds the run already settled reports .done.
do {
    expect(Thread.isMainThread, "this test runs on the main thread")
    let (root, d) = makeOldSupport()
    let runner = DataMigration.Runner(support: root, defaults: d, oldPreferencesDomain: noOldDomain, locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
    var reported: [DataMigration.Outcome] = []
    runner.onFinish = { reported.append($0) }
    let fd = open(root.appending(path: "Aether/node/run.lock").path, O_RDWR)
    _ = flock(fd, LOCK_EX | LOCK_NB)
    if case .deferred = runner.ensureFromMain() { expect(true, "the old app's lock defers the move") }
    else { expect(false, "a held lock must defer") }
    close(fd)
    expect(runner.ensureFromMain() == .done, "once it quits, an inline run completes the move")
    expect(reported.count == 2 && reported.last == .done, "the deferral and then the inline settle are each reported once: \(reported)")
    let settledReport = DispatchSemaphore(value: 0)
    runner.onFinish = { reported.append($0); settledReport.signal() }
    runner.start()
    _ = settledReport.wait(timeout: .now() + 5)
    expect(reported.count == 3 && reported.last == .done, "a start() after the run settled still reports .done: \(reported)")
    cleanup(root, d)
}

// 25–29. poc-m3 (2026-10-07, migration-stall-pocm3.md §5): a 0.6.6 → 0.7.1
//     move ran while the screen was locked. The node tree moved, but the
//     wallet handle (complete file protection) could not be read, the run
//     returned .failed before the preferences copy, nodeEnabled never
//     arrived, and nothing said so. chmod 000 stands in for the lock (EACCES
//     here, EPERM under protection: both are "unreadable").
do {
    expect(Thread.isMainThread, "this test runs on the main thread")
    let (root, d) = makeOldSupport()
    let handle = root.appending(path: "AetherWallet/enclave-key.dat")
    chmod(handle.path, 0o000)
    let outcome = DataMigration.migrate(support: root, defaults: d, oldPreferencesDomain: noOldDomain,
                                        oldPreferences: ["acceptedTerms": 3, "nodeEnabled": true], locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
    if case .waitingForUnlock(let why) = outcome {
        expect(why.contains("Unlock"), "25: a locked handle waits for unlock and says so: \(why)")
    } else {
        expect(false, "25: a locked handle must wait for unlock, not fail: \(outcome)")
    }
    expect(FileManager.default.fileExists(atPath: handle.path)
           && !FileManager.default.fileExists(atPath: root.appending(path: "EastSeaWallet/enclave-key.dat").path),
           "25: the old handle stays, no new one")
    expect(!doneFlag(d) && DataMigration.mayCreateFreshWalletKey(support: root, defaults: d, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) != nil,
           "25: not done, and no fresh wallet key meanwhile")
    expect(FileManager.default.fileExists(atPath: root.appending(path: "EastSea/node/validator.key").path), "25: the node tree moved")
    expect(d.bool(forKey: "nodeEnabled") && (d.object(forKey: "acceptedTerms") as? Int) == 3,
           "26: the preferences arrive even while the wallet handle waits")
    let idle = DataMigration.Runner(support: root, defaults: d, oldPreferencesDomain: noOldDomain, locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
    expect(DataMigration.mayStartNode(support: root, defaults: d, runner: idle, locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == nil, "27: the moved node may start whatever the wallet does")
    var outcomes: [DataMigration.Outcome] = []
    idle.onFinish = { outcomes.append($0) }
    let inline = idle.ensureFromMain()
    if case .waitingForUnlock = inline, outcomes.count == 1, case .waitingForUnlock = outcomes[0] {
        expect(true, "28: an inline outcome that did not settle is reported")
    } else {
        expect(false, "28: an inline waitingForUnlock must reach onFinish: \(inline) \(outcomes)")
    }
    chmod(handle.path, 0o644)   // the unlock
    expect(idle.ensureFromMain() == .done, "29: after the unlock the move completes")
    expect(DataMigration.fileMatches(handle, root.appending(path: "EastSeaWallet/enclave-key.dat")) && doneFlag(d),
           "29: the handle arrived byte for byte, and the move is done")
    cleanup(root, d)
}

// Migration notices accept a requested language without depending on this Mac.
expect(DataMigration.movingSentence(locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
       == "EastSea is moving your data over from Aether. This takes a moment; the wallet and the node start as soon as it is done.", "the reviewed English moving sentence")
expect(DataMigration.movingSentence(locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))
       == "동해가 Aether의 데이터를 옮기고 있어요. 잠시면 끝나고, 끝나는 대로 지갑과 노드가 시작돼요.", "the reviewed Korean moving sentence")
expect(DataMigration.movingSentence(locale: walletTestLocale("ja"), bundle: walletTestBundle("ja"))
       == "EastSeaがAetherからデータを移しています。少しお待ちください。終わるとウォレットとノードが起動します。", "the Japanese moving sentence")

// Test hygiene: this run wrote no preferences plist at all.
Thread.sleep(forTimeInterval: 1)   // cfprefsd writes asynchronously
let leftover = ownPlists().subtracting(plistsBefore)
expect(leftover.isEmpty, "no preferences plist was written by this run: \(leftover)")

exit(Int32(failures))
