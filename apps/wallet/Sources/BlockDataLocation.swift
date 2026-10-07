import Foundation

/// 블록 데이터 위치 / Block data location: where the node's bulky chain data
/// lives — the default (Application Support, internal disk) or a folder on a
/// disk the person picks. Keys never move: `validator.key`,
/// `node-account.key`, `node.identity`, the DeviceCheck token and the
/// follower's endpoint key stay in the node's data folder on the internal
/// disk; only the follower's chain state (and the archive, when on) moves,
/// passed to the node as `--chain-data`.
///
/// Pure (Foundation only): the validation rules, the copy the person reads,
/// the archive requirements and the node flags. Tests/block-data covers them.
enum BlockDataLocation {
    /// The folder the app creates inside the picked folder: the node's
    /// `--chain-data`. The node never creates it (an unplugged disk leaves
    /// /Volumes/X missing, and creating it there would fill the internal disk).
    static let folderName = "EastSea Block Data"
    /// What moves: the follower's state and the archive's.
    static let movedDirs = ["follow", "archive"]
    /// Files inside those folders that are keys and stay on the internal
    /// disk: the follower's endpoint key (`wallet-node.key`).
    static let keepInternal: Set<String> = ["wallet-node.key"]
    /// Formats the node can live on: APFS and Mac OS Extended. exFAT/FAT
    /// lack the locking and the crash safety the database needs; network
    /// shares come and go.
    static let supportedFormats: Set<String> = ["apfs", "hfs"]
    static let networkFormats: Set<String> = ["smbfs", "afpfs", "nfs", "webdav", "cifs", "ftp"]

    /// The facts about the disk a folder is on (statfs + URL resource values).
    struct Volume: Equatable {
        var name: String
        /// `statfs.f_fstypename`, lowercased ("apfs", "hfs", "exfat", "msdos", "smbfs", …).
        var format: String
        var isLocal: Bool
        var isReadOnly: Bool
        var isInternal: Bool
        var freeBytes: UInt64
    }

    enum Problem: Equatable {
        case networkShare
        case unsupportedFormat(String)
        case readOnly
        case notEnoughSpace(freeBytes: UInt64, neededBytes: UInt64)
        case inUse
    }

    /// Whether a disk can hold `dataBytes` of block data with the node's
    /// resume margin left over. Order: the kind of disk first (nothing else
    /// matters on a share), then the format, then writability, then space.
    static func validate(_ v: Volume, dataBytes: UInt64) -> Problem? {
        if !v.isLocal || networkFormats.contains(v.format) { return .networkShare }
        if !supportedFormats.contains(v.format) { return .unsupportedFormat(v.format) }
        if v.isReadOnly { return .readOnly }
        let needed = dataBytes + NodeResume.resumeBytes
        if v.freeBytes < needed { return .notEnoughSpace(freeBytes: v.freeBytes, neededBytes: needed) }
        return nil
    }

    /// A human name for a format.
    static func formatName(_ f: String) -> String {
        switch f {
        case "apfs": return "APFS"
        case "hfs": return "Mac OS Extended"
        case "exfat": return "exFAT"
        case "msdos": return "FAT"
        case "ntfs": return "NTFS"
        default: return f.uppercased()
        }
    }

    static func sentence(_ p: Problem, ko: Bool) -> String {
        switch p {
        case .networkShare:
            return ko ? "네트워크 공유 폴더에는 둘 수 없어요. 이 Mac에 직접 연결된 디스크를 골라 주세요."
                : "A network share cannot hold the block data. Pick a disk connected to this Mac."
        case .unsupportedFormat(let f):
            return ko ? "이 디스크는 \(formatName(f)) 형식이라 쓸 수 없어요. APFS나 Mac OS 확장 형식 디스크를 골라 주세요."
                : "This disk is formatted \(formatName(f)), which cannot hold the block data. Pick an APFS or Mac OS Extended disk."
        case .readOnly:
            return ko ? "이 디스크는 읽기 전용이에요. 쓸 수 있는 디스크를 골라 주세요."
                : "This disk is read-only. Pick one that can be written to."
        case .notEnoughSpace(let free, let needed):
            return ko ? "공간이 부족해요: \(NodeStopReason.gb(free)) 남음, \(NodeStopReason.gb(needed)) 필요해요."
                : "Not enough space: \(NodeStopReason.gb(free)) free, \(NodeStopReason.gb(needed)) needed."
        case .inUse:
            return ko ? "이미 이 위치를 쓰고 있어요." : "The block data is already there."
        }
    }

    /// The `--chain-data` folder for a picked folder (a picked folder that
    /// already is one is used as is).
    static func chainDir(picked: URL) -> URL {
        picked.lastPathComponent == folderName ? picked : picked.appendingPathComponent(folderName, isDirectory: true)
    }

    /// The node flags for the stored choices, appended to the shared argv.
    static func flags(chainDataPath: String, archive: Bool) -> [String] {
        var out: [String] = []
        if !chainDataPath.isEmpty { out += ["--chain-data", chainDataPath] }
        if archive { out += ["--archive"] }
        return out
    }

    /// The volume name in a /Volumes path ("/Volumes/외장 SSD/x" → "외장 SSD").
    static func volumeName(ofPath path: String) -> String? {
        let parts = (path as NSString).standardizingPath.split(separator: "/", omittingEmptySubsequences: true)
        guard parts.count >= 2, parts[0] == "Volumes" else { return nil }
        return String(parts[1])
    }
}

/// 전체 기록 보관 (아카이브) / Keep full history (archive): what turning it
/// on costs, on real numbers. The per-block size and the daily growth were
/// measured on the network's archive node (poc-nas); the size now scales
/// with the chain's current height. No reward of any kind is promised.
struct ArchiveRequirements: Equatable {
    /// Measured on the archive node (see `measured`).
    static let bytesPerBlock: Double = ArchiveMeasurement.bytesPerBlock
    static let bytesPerDay: Double = ArchiveMeasurement.bytesPerDay

    let height: UInt64

    var sizeNowBytes: UInt64 { UInt64(Double(height) * Self.bytesPerBlock) }
    var perMonthBytes: UInt64 { UInt64(Self.bytesPerDay * 30) }
    var perYearBytes: UInt64 { UInt64(Self.bytesPerDay * 365) }
    /// What the disk should have free to start: today's size plus a year of
    /// growth plus the node's resume margin.
    var recommendedFreeBytes: UInt64 { sizeNowBytes + perYearBytes + NodeResume.resumeBytes }

    func lines(ko: Bool) -> [String] {
        let now = NodeStopReason.gb(sizeNowBytes), month = NodeStopReason.gb(perMonthBytes)
        let rec = NodeStopReason.gb(recommendedFreeBytes)
        return ko ? [
            "지금 전체 기록은 약 \(now)이고, 한 달에 약 \(month)씩 늘어나요.",
            "\(rec) 이상 비어 있는 보조 디스크(외장 SSD 등)를 권해요. 위의 ‘블록 데이터 위치’에서 고를 수 있어요.",
            "처음부터 모든 블록을 다시 확인하므로 첫 동기화에 몇 시간에서 며칠이 걸려요. 그동안 지갑은 그대로 써요.",
            "보관에 대한 보상은 없어요. 네트워크의 전체 기록을 누구나 검증할 수 있게 돕는 선택이에요.",
        ] : [
            "The full history is about \(now) today and grows by about \(month) a month.",
            "A secondary disk (an external SSD, say) with at least \(rec) free is recommended. Pick it under Block data location above.",
            "Every block is re-checked from the very first, so the first sync takes hours to days. The wallet keeps working meanwhile.",
            "There is no reward for keeping it. It helps anyone verify the network's whole history.",
        ]
    }
}
