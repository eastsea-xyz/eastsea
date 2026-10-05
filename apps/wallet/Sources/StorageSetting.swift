#if os(macOS)
import Foundation

/// 설정 ▸ 역사 보관 (docs/design/15-node-rewards.md "C. 보관"): how much of
/// the network's past this Mac keeps. The UI speaks only in gigabytes; the
/// node speaks only in `--max-shards`, and this is the whole dictionary
/// between them, pure so a standalone test covers every rule (Tests/storage).
enum StorageSetting {
    /// One shard's nominal footprint. The design's arithmetic is
    /// DEFAULT_MAX_SHARDS = 64 shards ≈ the 50 GB default setting
    /// (crates/node/src/shards.rs), so a shard is 800 MiB and every offered
    /// budget maps to a whole shard count.
    static let bytesPerShard = 838_860_800
    /// The node's own default (`aether_node::shards::DEFAULT_MAX_SHARDS`),
    /// the 50 GB row. A choice that resolves to it passes no flag at all, so
    /// an older bundled node still accepts the start (the ProverFlags rule).
    static let defaultShards = 64
    /// The fixed budgets the picker offers, in GB.
    static let choicesGB = [25, 50, 100, 200, 500]
    /// The stored choice: "off", a budget's GB as text ("50"), or "free"
    /// (use the volume's free space down to the reserve).
    static let defaultChoice = "50"

    static func bytes(forGB gb: Int) -> Int { gb * 1_073_741_824 }

    /// GB → shards: 25→32, 50→64 (the node's own default), 100→128, 200→256,
    /// 500→640 — every offered budget lands on a whole shard.
    static func shards(forGB gb: Int) -> Int { gb * defaultShards / 50 }

    /// Shards → the GB the UI would show for them (the inverse of the above).
    static func gigabytes(forShards shards: Int) -> Double {
        Double(shards) * Double(bytesPerShard) / 1_073_741_824
    }

    /// What must stay free on the data volume after a choice: 20 GB, or a
    /// tenth of the chosen budget when that is more. A Mac that fills its
    /// disk for the network's past stops being usable for anything else.
    static func safetyMarginBytes(forBudgetBytes budget: Int) -> Int {
        max(bytes(forGB: 20), budget / 10)
    }

    /// Whether `gb` may be chosen: the part of the budget already on disk
    /// (`heldBytes` of shards written) is not new pressure, and what is new
    /// must still leave the safety margin free.
    static func allows(gb: Int, freeBytes: Int, heldBytes: Int) -> Bool {
        let extra = max(0, bytes(forGB: gb) - heldBytes)
        return freeBytes - extra >= safetyMarginBytes(forBudgetBytes: bytes(forGB: gb))
    }

    /// What "남는 공간 사용" keeps free on the volume: 20 GB, or a tenth of
    /// what is free when that is more — the bigger the disk, the bigger the
    /// untouched headroom.
    static func freeSpaceReserve(freeBytes: Int) -> Int {
        max(bytes(forGB: 20), freeBytes / 10)
    }

    /// The shard count "남는 공간 사용" resolves to: everything free above
    /// the reserve. Never negative, never above what the volume can hold.
    static func freeSpaceShards(freeBytes: Int) -> Int {
        max(0, (freeBytes - freeSpaceReserve(freeBytes: freeBytes)) / bytesPerShard)
    }

    /// The shard count a stored choice resolves to. "off" is honored only
    /// while this Mac is not a registered candidate — a voting Mac keeps the
    /// network's history by default (docs/design/15 "C. 보관": 등록 후보만
    /// 보관). "free" needs the volume's current free space; without it the
    /// default holds. Anything unreadable falls back to the default too.
    static func resolve(choice: String, registered: Bool, freeBytes: Int?) -> Int {
        switch choice {
        case "off": return registered ? defaultShards : 0
        case "free": return freeBytes.map(freeSpaceShards(freeBytes:)) ?? defaultShards
        default:
            guard let gb = Int(choice), choicesGB.contains(gb) else { return defaultShards }
            return shards(forGB: gb)
        }
    }

    /// The node flag for a resolved count: nothing at the node's own default,
    /// the count spelled out otherwise (`0` holds nothing and lets the node
    /// drop whatever it already holds beyond an empty assignment).
    static func flag(shards: Int) -> String? {
        shards == defaultShards ? nil : "--max-shards=\(shards)"
    }

    /// Free space on the volume holding `path` (the node's data dir), in
    /// bytes. The dir need not exist yet (a fresh install has no data dir,
    /// but its volume is already the one the choice will land on): walk up to
    /// the nearest existing ancestor. `nil` when even that fails; the caller
    /// then falls back to the default instead of guessing.
    static func freeBytes(atPath path: String) -> Int? {
        var url = URL(fileURLWithPath: path)
        let fm = FileManager.default
        while !fm.fileExists(atPath: url.path) {
            guard url.pathComponents.count > 1 else { return nil }
            url.deleteLastPathComponent()
        }
        let attrs = try? fm.attributesOfFileSystem(forPath: url.path)
        return (attrs?[.systemFreeSize] as? NSNumber)?.intValue
    }
}
#endif
