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
    // The committee's public file and the vote journals at the names the node
    // binary itself reads at its data root (supervisor.rs, main.rs).
    try? "{\"chain_id\":1,\"round\":0}".write(to: node.appending(path: "network.json"), atomically: true, encoding: .utf8)
    try? "validator-pub".write(to: node.appending(path: "validator.pub.json"), atomically: true, encoding: .utf8)
    let journal = node.appending(path: "aether-consensus-r1")
    try? FileManager.default.createDirectory(at: journal, withIntermediateDirectories: true)
    try? "vote-journal-bytes".write(to: journal.appending(path: "0"), atomically: true, encoding: .utf8)
    try? "dkg-round-0".write(to: node.appending(path: "dkg-agreement-genesis-0.journal"), atomically: true, encoding: .utf8)
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

// 7. Forced cross-volume fallback (audit 6, A6-5): the move cannot happen, so
//    a verified copy leaves the old tree behind — and the old tree must not
//    keep a usable signing identity for any binary to pick up.
do {
    let (root, d) = makeOldSupport()
    // An occupied destination forces moveItem to fail, the cross-volume path.
    let newNode = root.appending(path: "EastSea/node")
    try? FileManager.default.createDirectory(at: newNode, withIntermediateDirectories: true)
    try? "earlier partial attempt".write(to: newNode.appending(path: "data.db"), atomically: true, encoding: .utf8)
    let outcome = DataMigration.migrate(support: root, defaults: d)
    expect(outcome == .done, "the forced-copy migration completes: \(outcome)")
    expect(doneFlag(d), "done is set after the verified copy")
    let fm = FileManager.default
    let oldNode = root.appending(path: "Aether/node")
    expect(fm.fileExists(atPath: oldNode.path), "the copy fallback keeps the old tree")
    // The old root no longer holds anything a signer can use (A6-5).
    for gone in ["validator.key", "validator.pub.json", "node-account.key", "threshold.json",
                 "aether-consensus-r1", "dkg-agreement-genesis-0.journal"] {
        expect(!fm.fileExists(atPath: oldNode.appending(path: gone).path), "the old root no longer holds \(gone)")
    }
    // ...while the quarantine directory inside it does (recoverable).
    let quarantined = ((try? fm.contentsOfDirectory(atPath: oldNode.path)) ?? [])
        .filter { $0.hasPrefix("eastsea-quarantine-") }
    expect(quarantined.count == 1, "one quarantine directory: \(quarantined)")
    if let q = quarantined.first {
        for kept in ["validator.key", "validator.pub.json", "node-account.key", "threshold.json",
                     "aether-consensus-r1", "dkg-agreement-genesis-0.journal"] {
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
    _ = DataMigration.migrate(support: root, defaults: d)
    view = oldBinaryView(oldNode)
    expect(!view.canSign, "after the migration the old binary can no longer sign (no key, no share)")
    expect(view.refusesFreshIdentity, "the old root still refuses to mint a fresh identity (network.json stays)")
    // The same-volume path removes the old tree entirely — also unsignable.
    let (root2, d2) = makeOldSupport()
    _ = DataMigration.migrate(support: root2, defaults: d2)
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
    let outcome = DataMigration.migrate(support: root, defaults: d)
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
    expect(DataMigration.mayStartNode(support: root, defaults: d) != nil,
           "a partial destination tree still refuses the node start")
    // So does an empty placeholder directory.
    try? FileManager.default.removeItem(at: newNode.appending(path: "data.db"))
    expect(DataMigration.mayStartNode(support: root, defaults: d) != nil,
           "an empty destination directory still refuses the node start")
    // The same-volume completion clears the refusal.
    expect(DataMigration.migrate(support: root, defaults: d) == .done, "the migration completes over the placeholder")
    expect(DataMigration.mayStartNode(support: root, defaults: d) == nil, "a completed migration allows the node")
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
    expect(DataMigration.mayStartNode(support: root, defaults: d) != nil,
           "a half-quarantined old tree still refuses the node start")
    expect(oldBinaryView(oldNode).canSign == false, "with the key gone the old binary already cannot sign")
    let outcome = DataMigration.migrate(support: root, defaults: d)
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

exit(Int32(failures))
