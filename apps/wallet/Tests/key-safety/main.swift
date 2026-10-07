// Keys stay on this Mac and out of backups (design 36 §6.2, §11 N2/N3,
// KeySafety.swift): which files are keys, that the block-data mover never
// takes one, that no key lives on an external disk, that keys are excluded
// from Time Machine, and that nothing goes under an iCloud-synced Desktop or
// Documents.
//   scripts/test-swift-pure.sh   (run key-safety)
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

// N2: the keys, and the mover.
for k in ["validator.key", "validator.pub.json", "node-account.key", "threshold.json", "devicecheck-token", "key-binding.json"] {
    check(KeySafety.isKey(k), "\(k) is a key")
}
check(KeySafety.isKey("follow/wallet-node.key"), "the follower's endpoint key is a key")
check(!KeySafety.isKey("follow/state.redb") && !KeySafety.isKey("node.log"), "chain data is not a key")
for d in BlockDataLocation.movedDirs { check(!KeySafety.isKey(d), "the mover's \(d) is not a key") }
check(BlockDataLocation.keepInternal.allSatisfy { KeySafety.isKey("follow/" + $0) }, "every file the mover keeps back is a key")
// A chosen block-data folder that already holds keys is refused.
check(KeySafety.keysFound(in: ["follow", "validator.key", "x"]) == ["validator.key"], "keys found at the top of a chosen folder")
check(KeySafety.keysFound(in: ["follow", "archive", ".DS_Store"]).isEmpty, "a clean folder")
check(KeySafety.keysRefusal(found: ["validator.key"], ko: true)?.contains("키는 이 Mac에") == true, "plain refusal")
check(KeySafety.keysRefusal(found: [], ko: true) == nil, "no keys, no refusal")
// The key folder itself must be on the internal disk.
check(KeySafety.keyDirAllowed(path: "/Users/me/Library/Application Support/EastSea/node", isInternal: true), "internal: fine")
check(!KeySafety.keyDirAllowed(path: "/Volumes/SSD/EastSea/node", isInternal: false), "an external volume is refused")
check(!KeySafety.keyDirAllowed(path: "/Volumes/SSD/EastSea/node", isInternal: true), "anything under /Volumes is refused")

// N3: backups and iCloud.
let present = ["validator.key", "node-account.key", "follow/wallet-node.key", "node.log", "follow/state.redb", "devicecheck-token"]
check(KeySafety.backupExclusions(present: present) == ["devicecheck-token", "follow/wallet-node.key", "node-account.key", "validator.key"],
      "exactly the keys are excluded from Time Machine")
let home = "/Users/me"
check(KeySafety.iCloudRefusal(path: "/Users/me/Documents/Node", home: home, ubiquitous: true, ko: false) != nil,
      "an iCloud-synced Documents is refused")
check(KeySafety.iCloudRefusal(path: "/Users/me/Desktop/x", home: home, ubiquitous: true, ko: true) != nil, "and Desktop")
check(KeySafety.iCloudRefusal(path: "/Users/me/Documents/Node", home: home, ubiquitous: false, ko: false) == nil,
      "Documents without iCloud sync is fine")
check(KeySafety.iCloudRefusal(path: "/Users/me/Library/Mobile Documents/com~apple~CloudDocs/x", home: home, ubiquitous: true, ko: false) != nil,
      "iCloud Drive itself is refused")
check(KeySafety.iCloudRefusal(path: "/Volumes/SSD/x", home: home, ubiquitous: false, ko: false) == nil, "an external disk is not iCloud")
check(KeySafety.iCloudRefusal(path: "/Users/me/Documents-old/x", home: home, ubiquitous: false, ko: false) == nil, "a look-alike folder is not Documents")
print("OK key-safety")
