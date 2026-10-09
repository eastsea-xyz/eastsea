import Foundation

/// 블록 데이터 위치 / Block data location: where the node's bulky chain data
/// lives — the default (Application Support, internal disk) or a folder on a
/// disk the person picks. Keys never move: `validator.key`,
/// `node-account.key`, `node.identity`, the DeviceCheck token and the
/// follower's endpoint key stay in the node's data folder on the internal
/// disk; the follower's chain state (and archive history) starts fresh,
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
    private static let protectedNames = Set(KeySafety.keyFiles.map { URL(fileURLWithPath: $0).lastPathComponent })
        .union(["node.identity", "network.json", "settings.json", "key-creation.json", "run.lock"])
    /// Reserved key names stay internal even when APFS stores a casing alias.
    /// Folding conservatively also retains an unused alias on a case-sensitive disk.
    static func keepsInternal(_ name: String) -> Bool {
        let folded = name.lowercased()
        return protectedNames.contains(folded) || KeySafety.isKey(folded)
    }

    /// Initial fresh-state allowance plus the same free-space margin used
    /// by a new node. Old database allocation and archive history do not
    /// contribute; snapshot sync also enforces its actual wire-size budget.
    static let freshDataBytes: UInt64 = 1_073_741_824
    static let freshFootprintBytes = freshDataBytes + NodeResume.resumeBytes

    static func validateFresh(_ volume: Volume) -> Problem? {
        validate(volume, dataBytes: freshDataBytes)
    }

    static func confirmation(archive: Bool, locale: Locale = .current, bundle: Bundle = .main) -> String {
        let message = String(localized: "The node starts empty here and syncs from the network. The old block data is deleted only after the new node answers and follows a certified block. Your keys, identity and settings stay on this Mac. The wallet keeps working while it syncs.", bundle: bundle, locale: locale)
        guard archive else { return message }
        return message + "\n\n" + String(localized: "In archive mode, the extra history is rebuilt from peers and era files over time.", bundle: bundle, locale: locale)
    }
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
        let (sum, overflow) = dataBytes.addingReportingOverflow(NodeResume.resumeBytes)
        let needed = overflow ? UInt64.max : sum
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

    static func sentence(_ p: Problem, returningToDefault: Bool = false, locale: Locale = .current, bundle: Bundle = .main) -> String {
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
            if returningToDefault {
                return String(localized: "Cannot move back to default: the internal disk has \(freeSpace) free; \(neededSpace) is needed to start fresh.", bundle: bundle, locale: locale)
            }
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

    /// Resolve existing ancestors too when the proposed leaf does not exist.
    static func resolvedRoot(_ url: URL) -> URL {
        var ancestor = url.standardizedFileURL
        var suffix: [String] = []
        while !FileManager.default.fileExists(atPath: ancestor.path), ancestor.pathComponents.count > 1 {
            suffix.insert(ancestor.lastPathComponent, at: 0)
            ancestor.deleteLastPathComponent()
        }
        return suffix.reduce(ancestor.resolvingSymlinksInPath()) { $0.appendingPathComponent($1) }.standardizedFileURL
    }

    /// Fresh-start and cleanup roots must never overlap in either direction.
    static func disjoint(_ a: URL, _ b: URL) -> Bool {
        let x = resolvedRoot(a).path, y = resolvedRoot(b).path
        return x != y && !x.hasPrefix(y + "/") && !y.hasPrefix(x + "/") && x != "/" && y != "/"
    }

    /// Existing chain data is never merged into or made rollback cargo.
    /// The default home may already contain the endpoint key kept internal.
    static func destinationAvailable(_ root: URL, preservingInternalKeys: Bool) -> Bool {
        let fm = FileManager.default
        guard (try? fm.destinationOfSymbolicLink(atPath: root.path)) == nil else { return false }
        for name in movedDirs {
            let dir = root.appendingPathComponent(name)
            if (try? fm.destinationOfSymbolicLink(atPath: dir.path)) != nil { return false }
            guard fm.fileExists(atPath: dir.path) else { continue }
            guard preservingInternalKeys, containsOnlyInternalFiles(dir) else { return false }
        }
        return true
    }

    /// Cleanup can retain nested keys. A return move accepts those keys in
    /// place, while refusing any old database or link as a fresh destination.
    private static func containsOnlyInternalFiles(_ dir: URL) -> Bool {
        guard let entries = try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: [.isDirectoryKey, .isRegularFileKey, .isSymbolicLinkKey]) else { return false }
        return entries.allSatisfy { item in
            guard let values = try? item.resourceValues(forKeys: [.isDirectoryKey, .isRegularFileKey, .isSymbolicLinkKey]),
                  values.isSymbolicLink != true else { return false }
            if values.isDirectory == true { return containsOnlyInternalFiles(item) }
            return values.isRegularFile == true && keepsInternal(item.lastPathComponent)
        }
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
