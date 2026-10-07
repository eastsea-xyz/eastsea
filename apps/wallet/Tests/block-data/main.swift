// 블록 데이터 위치 and 전체 기록 보관 (BlockDataLocation.swift): which disks
// may hold the block data, the node flags each choice makes, and the archive
// requirements on the archive node's real numbers.
//   scripts/test-swift-pure.sh   (run block-data)
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }
let GiB: UInt64 = 1_073_741_824

func vol(_ format: String, local: Bool = true, ro: Bool = false, free: UInt64 = 500 * GiB) -> BlockDataLocation.Volume {
    BlockDataLocation.Volume(name: "SSD", format: format, isLocal: local, isReadOnly: ro, isInternal: false, freeBytes: free)
}

// MARK: which disks

check(BlockDataLocation.validate(vol("apfs"), dataBytes: 20 * GiB) == nil, "APFS is fine")
check(BlockDataLocation.validate(vol("hfs"), dataBytes: 20 * GiB) == nil, "Mac OS Extended is fine")
check(BlockDataLocation.validate(vol("exfat"), dataBytes: 1) == .unsupportedFormat("exfat"), "exFAT is refused")
check(BlockDataLocation.validate(vol("msdos"), dataBytes: 1) == .unsupportedFormat("msdos"), "FAT is refused")
check(BlockDataLocation.validate(vol("smbfs"), dataBytes: 1) == .networkShare, "an SMB share is refused")
check(BlockDataLocation.validate(vol("apfs", local: false), dataBytes: 1) == .networkShare, "any non-local volume is refused")
check(BlockDataLocation.validate(vol("apfs", ro: true), dataBytes: 1) == .readOnly, "read-only is refused")
check(BlockDataLocation.validate(vol("apfs", free: 26 * GiB), dataBytes: 20 * GiB) == .notEnoughSpace(freeBytes: 26 * GiB, neededBytes: 27 * GiB),
      "the data plus the node's 7 GB resume margin must fit")
check(BlockDataLocation.validate(vol("apfs", free: 27 * GiB), dataBytes: 20 * GiB) == nil, "exactly enough is enough")
check(BlockDataLocation.sentence(.unsupportedFormat("exfat"), locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))
      == "이 디스크는 블록 저장에 안전하지 않은 형식(exFAT)이에요. 디스크 유틸리티에서 APFS로 지우면 쓸 수 있어요 (디스크 안의 파일은 지워져요).",
      "exFAT: why, and how to fix it in Disk Utility")
check(BlockDataLocation.sentence(.unsupportedFormat("exfat"), locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
      == "This disk's format (exFAT) is not safe for block data. Erase it as APFS in Disk Utility to use it (this deletes the files on the disk).",
      "exFAT in English")
check(BlockDataLocation.Problem.unsupportedFormat("msdos").fixInDiskUtility && !BlockDataLocation.Problem.readOnly.fixInDiskUtility,
      "only a format problem offers Disk Utility")
check(BlockDataLocation.sentence(.notEnoughSpace(freeBytes: 26 * GiB, neededBytes: 27 * GiB), locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == "Not enough space: 26.0 GB free, 27.0 GB needed.",
      "exact numbers")
for p in [BlockDataLocation.Problem.networkShare, .unsupportedFormat("ntfs"), .readOnly, .notEnoughSpace(freeBytes: 1, neededBytes: 2), .inUse] {
    for language in ["en", "ko", "ja"] { check(!BlockDataLocation.sentence(p, locale: walletTestLocale(language), bundle: walletTestBundle(language)).isEmpty, "\(p) has words in \(language)") }
}

// The former language branches resolve from the same catalog in Japanese.
let japaneseProblems: [(BlockDataLocation.Problem, String)] = [
    (.networkShare, "ネットワークの共有フォルダにはブロックデータを保存できません。このMacに直接接続したディスクを選んでください。"),
    (.readOnly, "このディスクは読み取り専用です。書き込めるディスクを選んでください。"),
    (.notEnoughSpace(freeBytes: 26 * GiB, neededBytes: 27 * GiB), "空き容量が足りません。空きは26.0 GB、必要な容量は27.0 GBです。"),
    (.inUse, "ブロックデータはすでにこの場所にあります。"),
]
for (problem, expected) in japaneseProblems {
    check(BlockDataLocation.sentence(problem, locale: walletTestLocale("ja"), bundle: walletTestBundle("ja")) == expected,
          "Japanese block-data sentence: \(problem)")
}

// MARK: the folder and the flags

let picked = URL(fileURLWithPath: "/Volumes/외장 SSD/Node")
check(BlockDataLocation.chainDir(picked: picked).path == "/Volumes/외장 SSD/Node/EastSea Block Data", "a folder is made inside the pick")
check(BlockDataLocation.chainDir(picked: BlockDataLocation.chainDir(picked: picked)) == BlockDataLocation.chainDir(picked: picked),
      "picking the data folder itself uses it as is")
check(BlockDataLocation.volumeName(ofPath: "/Volumes/외장 SSD/Node/EastSea Block Data") == "외장 SSD", "volume name from the path")
check(BlockDataLocation.volumeName(ofPath: "/Users/me/x") == nil, "not on /Volumes")
check(BlockDataLocation.flags(chainDataPath: "", archive: false) == [], "default: no flags, today's argv exactly")
check(BlockDataLocation.flags(chainDataPath: "/Volumes/X/EastSea Block Data", archive: false) == ["--chain-data", "/Volumes/X/EastSea Block Data"],
      "a chosen disk passes --chain-data")
check(BlockDataLocation.flags(chainDataPath: "", archive: true) == ["--archive"], "archive passes --archive")
check(BlockDataLocation.flags(chainDataPath: "/v", archive: true) == ["--chain-data", "/v", "--archive"], "both")
let argv = UnattendedDecision.nodeArgv(dataDir: "/d", rpcPort: 1, p2pPort: 2, networkPath: nil, proverFlags: [],
                                       storageFlag: "--max-shards=8", locationFlags: BlockDataLocation.flags(chainDataPath: "/v", archive: true))
check(argv == ["run", "--data", "/d", "--rpc-port", "1", "--port", "2", "--max-shards=8", "--chain-data", "/v", "--archive"],
      "the app's node and the daemon's take the same argv, location last: \(argv)")
check(argv[2] == "/d", "keys stay in --data (the internal disk)")
check(BlockDataLocation.movedDirs == ["follow", "archive"], "only the follower's and the archive's state move")
check(BlockDataLocation.keepInternal.contains("wallet-node.key"), "the follower's endpoint key stays on the internal disk")

// MARK: archive requirements, on measured numbers

check(ArchiveRequirements.bytesPerBlock > 0 && ArchiveRequirements.bytesPerDay > 0, "measured on the archive node, not zero")
let req = ArchiveRequirements(height: 500_000)
check(req.sizeNowBytes == UInt64(500_000 * ArchiveRequirements.bytesPerBlock), "size scales with the height")
check(req.recommendedFreeBytes == req.sizeNowBytes + req.perYearBytes + 7 * GiB, "now + a year + the resume margin")
for language in ["en", "ko", "ja"] {
    let lines = req.lines(locale: walletTestLocale(language), bundle: walletTestBundle(language))
    check(lines.count == 4, "size, disk, first sync, no reward")
    check(lines[0].contains(NodeStopReason.gb(req.sizeNowBytes)) && lines[0].contains(NodeStopReason.gb(req.perMonthBytes)), "real numbers: \(lines[0])")
    check(lines[1].contains(NodeStopReason.gb(req.recommendedFreeBytes)), "the recommended disk size")
    let all = lines.joined(separator: " ")
    for promise in ["earn", "벌", "수익", "APY", "%"] { check(!all.contains(promise), "no reward promise (\(promise)) language=\(language)") }
}
check(req.lines(locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))[3].contains("보상은 없어요"), "says plainly there is no reward")
check(req.lines(locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))[2].contains("몇 시간에서 며칠"), "hours to days")
print("OK block-data")
