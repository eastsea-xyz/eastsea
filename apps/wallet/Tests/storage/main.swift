// The history-storage setting (설정 ▸ 역사 보관, docs/design/15-node-rewards.md
// "C. 보관") without an app or a node: the GB↔shard dictionary, the
// free-space guard, the default by role, and the one flag both the app's node
// and the daemon's node are built from.
//   swiftc -o ./tmp/storage-check apps/wallet/Sources/StorageSetting.swift apps/wallet/Sources/UnattendedDecision.swift apps/wallet/Tests/storage/main.swift && ./tmp/storage-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }
let gib = 1_073_741_824

// The dictionary stands on the design's arithmetic: the node's own default
// (DEFAULT_MAX_SHARDS = 64) is the 50 GB setting, so a shard is 800 MiB and
// every offered budget lands on a whole shard count.
check(StorageSetting.bytesPerShard == 800 * 1_048_576, "a shard is 800 MiB")
check(StorageSetting.defaultShards * StorageSetting.bytesPerShard == 50 * gib, "64 shards ≈ 50 GB, the docs' equivalence")
check(StorageSetting.choicesGB == [25, 50, 100, 200, 500], "the offered budgets")
for (gb, shards) in [(25, 32), (50, 64), (100, 128), (200, 256), (500, 640)] {
    check(StorageSetting.shards(forGB: gb) == shards, "\(gb) GB → \(shards) shards")
    check(StorageSetting.gigabytes(forShards: shards) > Double(gb) - 1 && StorageSetting.gigabytes(forShards: shards) < Double(gb) + 1,
          "\(shards) shards reads back as \(gb) GB")
}
check(StorageSetting.gigabytes(forShards: StorageSetting.defaultShards) > 49 && StorageSetting.gigabytes(forShards: StorageSetting.defaultShards) < 51,
      "the node's own default reads back as the 50 GB row")

// The default choice passes no flag at all: the node decides, and an older
// bundled node (before this setting existed) still accepts the start.
check(StorageSetting.defaultChoice == "50", "the stored default is the 50 GB row")
check(StorageSetting.resolve(choice: StorageSetting.defaultChoice, registered: false, freeBytes: nil) == StorageSetting.defaultShards,
      "the default choice resolves to the node's own default")
check(StorageSetting.flag(shards: StorageSetting.defaultShards) == nil, "the node's own default passes no flag")

// 보관 안 함 by role (등록 후보만 보관): an unregistered Mac may hold nothing —
// the node gets 0, assigns it nothing and drops what it already holds; a
// registered Mac has no "off" and runs the default.
check(StorageSetting.resolve(choice: "off", registered: false, freeBytes: nil) == 0, "off holds nothing while unregistered")
check(StorageSetting.flag(shards: 0) == "--max-shards=0", "off names itself on the command line")
check(StorageSetting.resolve(choice: "off", registered: true, freeBytes: nil) == StorageSetting.defaultShards,
      "a registered Mac's off runs as the default")
check(StorageSetting.flag(shards: StorageSetting.resolve(choice: "off", registered: true, freeBytes: nil)) == nil,
      "so it passes no flag either")

// Every other budget names itself.
check(StorageSetting.flag(shards: StorageSetting.resolve(choice: "200", registered: false, freeBytes: nil)) == "--max-shards=256",
      "200 GB names its count")
check(StorageSetting.resolve(choice: "garbage", registered: false, freeBytes: nil) == StorageSetting.defaultShards,
      "an unreadable stored choice falls back to the default")
check(StorageSetting.resolve(choice: "25", registered: true, freeBytes: nil) == 32, "registration does not touch a chosen budget")

// The free-space guard: 20 GB stays free, or a tenth of the budget when that
// is more, and shards already written are not new pressure.
check(StorageSetting.safetyMarginBytes(forBudgetBytes: 25 * gib) == 20 * gib, "small budgets keep 20 GB free")
check(StorageSetting.safetyMarginBytes(forBudgetBytes: 500 * gib) == 50 * gib, "a tenth of a big budget is more")
check(StorageSetting.allows(gb: 25, freeBytes: 45 * gib, heldBytes: 0), "25 GB fits with 20 GB to spare")
check(!StorageSetting.allows(gb: 25, freeBytes: 44 * gib, heldBytes: 0), "25 GB needs 45 GB free (25 + 20)")
check(StorageSetting.allows(gb: 100, freeBytes: 100 * gib, heldBytes: 40 * gib),
      "the 40 GB already held credits the 100 GB choice (60 new + 20 margin ≤ 100)")
check(!StorageSetting.allows(gb: 100, freeBytes: 79 * gib, heldBytes: 0), "100 GB is refused just under its margin")
check(StorageSetting.allows(gb: 500, freeBytes: 550 * gib, heldBytes: 0), "500 GB fits on a roomy volume")
check(!StorageSetting.allows(gb: 500, freeBytes: 549 * gib, heldBytes: 0), "500 GB keeps a 50 GB margin")

// 남는 공간 사용: fill the volume down to the reserve — 20 GB, or a tenth of
// what is free when that is more — never below it.
check(StorageSetting.freeSpaceReserve(freeBytes: 100 * gib) == 20 * gib, "a small volume keeps 20 GB free")
check(StorageSetting.freeSpaceReserve(freeBytes: 500 * gib) == 50 * gib, "a tenth of a big volume stays free")
check(StorageSetting.freeSpaceShards(freeBytes: 100 * gib) == 102, "100 GB free → 102 shards (80 GB usable)")
check(StorageSetting.freeSpaceShards(freeBytes: 500 * gib) == 576, "500 GB free → 576 shards (450 GB usable)")
check(StorageSetting.freeSpaceShards(freeBytes: 25 * gib) == 6, "a nearly full volume holds what fits above 20 GB")
check(StorageSetting.freeSpaceShards(freeBytes: 15 * gib) == 0, "below the reserve itself, nothing is held")
check(StorageSetting.freeSpaceShards(freeBytes: 0) == 0, "no free space, no shards")
check(StorageSetting.resolve(choice: "free", registered: false, freeBytes: 500 * gib) == 576,
      "the free choice resolves against the volume's free space")
check(StorageSetting.resolve(choice: "free", registered: false, freeBytes: nil) == StorageSetting.defaultShards,
      "an unreadable volume falls back to the default, never a guess")

// One budget for both nodes: the flag the app's own child starts with is the
// flag the daemon's marker carries (UnattendedDecision.nodeArgv is the single
// source), appended last in a stable order.
for (choice, free) in [("100", 0), ("off", 0), ("free", 500 * gib), ("50", 0)] {
    let flag = StorageSetting.flag(shards: StorageSetting.resolve(choice: choice, registered: false, freeBytes: free))
    let appArgv = UnattendedDecision.nodeArgv(dataDir: "/tmp/n", rpcPort: 18545, p2pPort: 19101,
                                              networkPath: "/tmp/network.json", proverFlags: ["--prover-threads=8"],
                                              storageFlag: flag)
    let daemonArgv = UnattendedDecision.nodeArgv(dataDir: "/tmp/n", rpcPort: 18545, p2pPort: 19101,
                                                 networkPath: "/tmp/network.json", proverFlags: ["--prover-threads=8"],
                                                 storageFlag: flag)
    check(appArgv == daemonArgv, "the app's node and the daemon's node take the same argv (\(choice))")
    if let flag {
        check(appArgv.last == flag, "the storage flag rides last (\(flag))")
    } else {
        check(!appArgv.contains { $0.hasPrefix("--max-shards") }, "the default passes no storage flag (\(choice))")
    }
}

// A fresh install has no data dir yet (the node has never run); the volume's
// free space is still readable through the nearest existing ancestor.
check((StorageSetting.freeBytes(atPath: "/no-such-dir-\(UUID().uuidString)/data") ?? 0) > 0,
      "a not-yet-created data dir still reports its volume's free space")

print("ok")
