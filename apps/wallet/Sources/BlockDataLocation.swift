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

        /// The fix is erasing the disk as APFS in Disk Utility (one button).
        var fixInDiskUtility: Bool {
            if case .unsupportedFormat = self { return true }
            return false
        }
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

    static func sentence(_ p: Problem, locale: Locale = .current, bundle: Bundle = .main) -> String {
        switch p {
        case .networkShare:
            return String(localized: "A network share cannot hold the block data. Pick a disk connected to this Mac.", bundle: bundle, locale: locale)
        case .unsupportedFormat(let f):
            let format = formatName(f)
            return String(localized: "This disk's format (\(format)) is not safe for block data. Erase it as APFS in Disk Utility to use it (this deletes the files on the disk).", bundle: bundle, locale: locale)
        case .readOnly:
            return String(localized: "This disk is read-only. Pick one that can be written to.", bundle: bundle, locale: locale)
        case .notEnoughSpace(let free, let needed):
            let freeSpace = NodeStopReason.gb(free), neededSpace = NodeStopReason.gb(needed)
            return String(localized: "Not enough space: \(freeSpace) free, \(neededSpace) needed.", bundle: bundle, locale: locale)
        case .inUse:
            return String(localized: "The block data is already there.", bundle: bundle, locale: locale)
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

    func lines(locale: Locale = .current, bundle: Bundle = .main) -> [String] {
        let now = NodeStopReason.gb(sizeNowBytes), month = NodeStopReason.gb(perMonthBytes)
        let rec = NodeStopReason.gb(recommendedFreeBytes)
        return [
            String(localized: "The full history is about \(now) today and grows by about \(month) a month.", bundle: bundle, locale: locale),
            String(localized: "A secondary disk (an external SSD, say) with at least \(rec) free is recommended. Pick it under Block data location above.", bundle: bundle, locale: locale),
            String(localized: "Every block is re-checked from the very first, so the first sync takes hours to days. The wallet keeps working meanwhile.", bundle: bundle, locale: locale),
            String(localized: "There is no reward for keeping it. It helps anyone verify the network's whole history.", bundle: bundle, locale: locale),
        ]
    }
}
