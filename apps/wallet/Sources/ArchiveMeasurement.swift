import Foundation

/// The full-history numbers the archive setting's requirements copy scales
/// by the current height. Measured read-only on 2026-10-07 04:30–04:45Z,
/// chain 7780: a follower synced from genesis keeps everything on 7780 (no
/// era files, no pruning), and its state.redb held 6,125,654,016 B at height
/// ~495,739 — 12,357 B per block, an upper bound (redb slack included). The
/// chain made 67,086 blocks in 19.37 h on poc-nas (≈ 83.1k blocks a day).
/// poc-nas's own `aether archive` started from a snapshot at 428,654, so its
/// 1.08 GB is not the full history and is not used here.
/// Re-measure when block contents change materially.
enum ArchiveMeasurement {
    static let measuredAt = "2026-10-07"
    static let bytesPerBlock: Double = 12_357
    static let blocksPerDay: Double = 83_100
    static let bytesPerDay: Double = bytesPerBlock * blocksPerDay   // ≈ 1.03 GB a day
}
