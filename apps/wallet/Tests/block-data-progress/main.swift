// Standalone, real-I/O coverage for streamed block-data moves. Fixtures live
// under the test runner's repository tmp root; no app or node is started.
import Foundation
#if canImport(Darwin)
import Darwin
#else
import Glibc
#endif

let fm = FileManager.default
guard let tmpPath = ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"]
        ?? ProcessInfo.processInfo.environment["TMPDIR"], tmpPath.hasPrefix("/") else {
    fatalError("Set AETHER_AGENT_TEST_TMP or TMPDIR to the repository tmp directory")
}
let root = URL(fileURLWithPath: tmpPath, isDirectory: true)
    .appendingPathComponent("block-data-progress-\(UUID().uuidString)", isDirectory: true)

func check(_ condition: Bool, _ message: String) {
    guard condition else {
        fputs("FAIL block-data-progress: \(message)\n", stderr)
        try? fm.removeItem(at: root)
        exit(1)
    }
}

struct FileState: Equatable {
    let inode: UInt64
    let bytes: Int64
    let blocks: Int64
    let modifiedSeconds: Int64
    let modifiedNanoseconds: Int64

    init(_ url: URL) {
        var st = stat()
        check(lstat(url.path, &st) == 0, "stat \(url.lastPathComponent)")
        inode = UInt64(st.st_ino)
        bytes = Int64(st.st_size)
        blocks = Int64(st.st_blocks)
        #if canImport(Darwin)
        modifiedSeconds = Int64(st.st_mtimespec.tv_sec)
        modifiedNanoseconds = Int64(st.st_mtimespec.tv_nsec)
        #else
        modifiedSeconds = Int64(st.st_mtim.tv_sec)
        modifiedNanoseconds = Int64(st.st_mtim.tv_nsec)
        #endif
    }
}

func writeAt(_ fd: Int32, data: Data, offset: Int64) {
    let wrote = data.withUnsafeBytes { buffer in
        pwrite(fd, buffer.baseAddress, buffer.count, off_t(offset))
    }
    check(wrote == data.count, "fixture write at \(offset): errno \(errno)")
}

func readAt(_ url: URL, offset: Int64, count: Int) -> Data {
    let fd = open(url.path, O_RDONLY | O_CLOEXEC)
    check(fd >= 0, "open \(url.lastPathComponent) for fixture inspection")
    defer { close(fd) }
    var data = Data(count: count)
    let read = data.withUnsafeMutableBytes { buffer in
        pread(fd, buffer.baseAddress, buffer.count, off_t(offset))
    }
    check(read == count, "fixture read at \(offset): errno \(errno)")
    return data
}

struct SparseFile {
    let url: URL
    let bytes: Int64
    let probes: [(offset: Int64, data: Data)]

    init(at url: URL, bytes: Int64) throws {
        self.url = url
        self.bytes = bytes
        try fm.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        let fd = open(url.path, O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC, 0o600)
        check(fd >= 0, "create sparse source: errno \(errno)")
        defer { close(fd) }
        check(ftruncate(fd, off_t(bytes)) == 0, "size sparse source: errno \(errno)")
        let markers: [(offset: Int64, data: Data)] = [
            (0, Data((0..<4096).map { UInt8(truncatingIfNeeded: $0 * 17 + 19) })),
            (bytes / 2, Data((0..<4096).map { UInt8(truncatingIfNeeded: $0 * 23 + 47) })),
            (bytes - 4096, Data((0..<4096).map { UInt8(truncatingIfNeeded: $0 * 31 + 83) })),
        ]
        for marker in markers { writeAt(fd, data: marker.data, offset: marker.offset) }
        check(fsync(fd) == 0, "flush sparse fixture")
        probes = markers + [(bytes / 4, Data(count: 4096)), (bytes * 3 / 4, Data(count: 4096))]
    }

    func checkContents(at other: URL) {
        check(FileState(other).bytes == bytes, "the file retains its complete logical size")
        for probe in probes {
            check(readAt(other, offset: probe.offset, count: probe.data.count) == probe.data,
                  "copied marker or sparse hole differs at \(probe.offset)")
        }
    }
}

final class ProgressTrace {
    let expected: Int64
    let maximumGap: Int64 = 64 << 20
    private(set) var work: Int64 = 0
    private(set) var reports = 0
    private(set) var largestGap: Int64 = 0

    init(expected: Int64) { self.expected = expected }

    func observe(_ fraction: Double) {
        check(fraction.isFinite && fraction >= 0 && fraction <= 1, "progress stays in 0...1")
        let next = Int64((fraction * Double(expected)).rounded())
        check(next >= work, "progress cannot go backwards: \(work) -> \(next)")
        let gap = next - work
        check(gap <= maximumGap,
              "progress gap \(gap) bytes exceeds 64 MiB (previous \(work), next \(next)); the copy ran without chunk callbacks")
        // With reportEvery:1, a clamped repeated 100% callback exposes work
        // wrongly credited after the two real reads of a verified resume.
        check(next != work || next == 0, "progress credited work beyond its two-pass budget")
        largestGap = max(largestGap, gap)
        work = next
        if next > 0 { reports += 1 }
    }

    func checkComplete() {
        check(work == expected, "last callback reaches 100% at exactly \(expected) metered bytes, got \(work)")
        check(reports > 0, "a completed move reports progress")
    }
}

func migratingFiles(in directory: URL, file: String) -> [URL] {
    ((try? fm.contentsOfDirectory(atPath: directory.path)) ?? [])
        .filter { $0.hasPrefix(".\(file).migrating-") }
        .map { directory.appendingPathComponent($0) }
}

func largeCopyProgress() throws {
    let bytes: Int64 = 2 << 30
    let source = root.appendingPathComponent("large/source", isDirectory: true)
    let target = root.appendingPathComponent("large/target", isDirectory: true)
    let fixture = try SparseFile(at: source.appendingPathComponent("state.redb"), bytes: bytes)
    let sourceBefore = FileState(fixture.url)
    check(sourceBefore.blocks * 512 < 1 << 20, "the >=2 GiB source fixture is genuinely sparse")
    let destination = target.appendingPathComponent("state.redb")
    let budget = bytes * 2
    let trace = ProgressTrace(expected: budget)
    var sawPartialCopy = false
    let meter = DataMigration.ProgressMeter { fraction in
        trace.observe(fraction)
        if trace.work > 0 && !sawPartialCopy {
            check(!fm.fileExists(atPath: destination.path), "copy progress arrives before the final name is published")
            let staged = migratingFiles(in: target, file: "state.redb")
            check(staged.count == 1, "first progress arrives while a temporary destination is being copied")
            let written = FileState(staged[0]).bytes
            check(written > 0 && written < bytes, "first progress observes a partial copy, not a completed file")
            sawPartialCopy = true
        }
    }
    meter.expect(budget)
    check(DataMigration.syncTreeVerified(source, target, meter: meter), "the >=2 GiB streamed copy verifies")
    check(meter.total == budget, "a fresh copy budgets exactly copy + destination verification")
    trace.checkComplete()
    check(sawPartialCopy, "copying, not only its later verification, reports progress")
    check(trace.reports >= Int(budget / trace.maximumGap), "large copy progress reports every 64 MiB or sooner")
    check(FileState(fixture.url) == sourceBefore, "the source inode, length, allocation, and modification time remain untouched")
    fixture.checkContents(at: fixture.url)
    fixture.checkContents(at: destination)
    check(migratingFiles(in: target, file: "state.redb").isEmpty, "successful copy leaves no private temporary file")
    print("OK large sparse copy: \(bytes) bytes, \(trace.reports) callbacks, maximum gap \(trace.largestGap) bytes, \(trace.work) metered bytes")
    // Do not retain a fully written 2 GiB destination for the small fixtures.
    try fm.removeItem(at: root.appendingPathComponent("large"))
}

func verifiedResume() throws {
    let source = root.appendingPathComponent("resume/source", isDirectory: true)
    let target = root.appendingPathComponent("resume/target", isDirectory: true)
    try fm.createDirectory(at: source, withIntermediateDirectories: true)
    try fm.createDirectory(at: target, withIntermediateDirectories: true)
    let data = Data(repeating: 0x72, count: (1 << 20) + 17)
    let original = source.appendingPathComponent("state.redb")
    let destination = target.appendingPathComponent("state.redb")
    try data.write(to: original)
    try data.write(to: destination)
    let before = FileState(destination)
    let sourceBefore = FileState(original)
    let budget = Int64(data.count) * 2
    let trace = ProgressTrace(expected: budget)
    let meter = DataMigration.ProgressMeter(reportEvery: 1) { trace.observe($0) }
    meter.expect(budget)
    check(DataMigration.syncTreeVerified(source, target, meter: meter), "an already-verified file resumes")
    trace.checkComplete()
    check(meter.total == budget, "resume budgets only one source/destination hash pass")
    check(FileState(destination) == before, "resume skips the copy and retains the destination inode and modification time")
    check(FileState(original) == sourceBefore, "resume leaves the source untouched")
    check(try fm.contentsOfDirectory(atPath: target.path) == ["state.redb"], "resume creates no replacement or temporary copy")
    print("OK verified resume: original destination inode retained, \(trace.work) metered bytes")
}

func unevenTreeProgress() throws {
    let source = root.appendingPathComponent("uneven/source", isDirectory: true)
    let target = root.appendingPathComponent("uneven/target", isDirectory: true)
    let fixture = try SparseFile(at: source.appendingPathComponent("state.redb"), bytes: (70 << 20) + 17)
    let smallFile = source.appendingPathComponent("metadata")
    let small = Data(repeating: 0x3f, count: 17)
    try small.write(to: smallFile)
    let sourceBefore = FileState(fixture.url)
    let smallBefore = FileState(smallFile)
    let budget = (fixture.bytes + Int64(small.count)) * 2
    let trace = ProgressTrace(expected: budget)
    var actualWork: Int64 = 0
    var largestActualGap: Int64 = 0
    var actualReports = 0
    let meter = DataMigration.ProgressMeter(reportBytes: { done, total in
        check(total == budget, "uneven files retain a two-pass work budget")
        check(done > actualWork, "uneven-file byte callbacks advance monotonically")
        let gap = done - actualWork
        check(gap <= trace.maximumGap, "uneven-file actual progress gap \(gap) exceeds 64 MiB")
        check(trace.work == done, "fraction and byte callbacks report the same completed work")
        largestActualGap = max(largestActualGap, gap)
        actualWork = done
        actualReports += 1
    }) { trace.observe($0) }
    meter.expect(budget)
    check(DataMigration.syncTreeVerified(source, target, meter: meter), "an uneven-file tree copies and verifies")
    trace.checkComplete()
    check(meter.total == budget && actualWork == budget, "uneven-file completion counts exactly twice the file-size sum")
    check(actualReports == trace.reports && actualReports >= 3, "uneven-file progress crosses both 64 MiB boundaries and reports its tail")
    check(FileState(fixture.url) == sourceBefore && FileState(smallFile) == smallBefore, "uneven-file copying leaves both sources untouched")
    fixture.checkContents(at: target.appendingPathComponent("state.redb"))
    check(try Data(contentsOf: target.appendingPathComponent("metadata")) == small, "the short file copies intact")
    print("OK uneven-file progress: \(actualReports) callbacks, actual/inferred maximum gap \(largestActualGap)/\(trace.largestGap) bytes, \(actualWork) metered bytes")
}

func corruptResumeProgress() throws {
    let source = root.appendingPathComponent("corrupt-resume/source", isDirectory: true)
    let target = root.appendingPathComponent("corrupt-resume/target", isDirectory: true)
    try fm.createDirectory(at: source, withIntermediateDirectories: true)
    try fm.createDirectory(at: target, withIntermediateDirectories: true)
    let correct = Data(repeating: 0x31, count: (64 << 10) + 17)
    let corrupt = Data(repeating: 0xc4, count: correct.count)
    let original = source.appendingPathComponent("state.redb")
    let destination = target.appendingPathComponent("state.redb")
    try correct.write(to: original)
    try corrupt.write(to: destination)
    let sourceBefore = FileState(original)
    let corruptInode = FileState(destination).inode
    let initialBudget = Int64(correct.count) * 2
    let repairedBudget = Int64(correct.count) * 4
    var fractions: [Double] = []
    var actualWork: Int64 = 0
    var completions = 0
    let meter = DataMigration.ProgressMeter(reportEvery: 16 << 10, reportBytes: { done, total in
        check(done >= actualWork, "corrupt-resume byte callbacks cannot go backwards")
        check(total == initialBudget || total == repairedBudget, "corrupt resume budgets comparison and repair work")
        if fractions.last == 1 {
            check(done == repairedBudget && total == repairedBudget, "100% accounts for both comparison hashes and the repair copy/verification")
        }
        actualWork = done
    }) { fraction in
        check(fraction.isFinite && fraction > 0 && fraction <= 1, "corrupt-resume progress stays in 0...1")
        if let previous = fractions.last { check(fraction >= previous, "corrupt-resume fractions remain monotonic when repair work is discovered") }
        fractions.append(fraction)
        if fraction == 1 {
            // Completion can precede the rename. Accept the fully verified
            // temporary file, but never the original corrupt destination.
            let candidates = migratingFiles(in: target, file: "state.redb") + [destination]
            check(candidates.contains { (try? Data(contentsOf: $0)) == correct },
                  "100% is withheld until repaired bytes verify in the temporary or published destination")
            completions += 1
        }
    }
    meter.expect(initialBudget)
    check(DataMigration.syncTreeVerified(source, target, meter: meter), "a same-size corrupt resume repairs and verifies")
    check(meter.total == repairedBudget && actualWork == repairedBudget, "corrupt resume expands its budget from two to four file passes")
    check(fractions.count >= 3 && fractions.last == 1 && completions == 1, "corrupt-resume progress reaches 100% once, after repair")
    check(try Data(contentsOf: destination) == correct, "corrupt resume publishes the repaired source bytes")
    check(FileState(original) == sourceBefore, "corrupt resume leaves the source untouched")
    let asides = try fm.contentsOfDirectory(atPath: target.path)
        .filter { $0.hasPrefix("state.redb.eastsea-replaced-") }
    check(asides.count == 1 && FileState(target.appendingPathComponent(asides[0])).inode == corruptInode,
          "corrupt resume keeps the prior destination inode aside")
    check(try Data(contentsOf: target.appendingPathComponent(asides[0])) == corrupt, "corrupt resume preserves the prior destination bytes")
    print("OK corrupt-resume progress: \(fractions.count) monotonic callbacks, \(actualWork) metered bytes, no premature 100%")
}

func corruptExistingDestination() throws {
    let source = root.appendingPathComponent("replacement/source", isDirectory: true)
    let target = root.appendingPathComponent("replacement/target", isDirectory: true)
    try fm.createDirectory(at: source, withIntermediateDirectories: true)
    try fm.createDirectory(at: target, withIntermediateDirectories: true)
    let original = source.appendingPathComponent("state.redb")
    let destination = target.appendingPathComponent("state.redb")
    let correct = Data("0123456789abcdef".utf8)
    let corrupt = Data("fedcba9876543210".utf8)
    try correct.write(to: original)
    try corrupt.write(to: destination)
    let corruptInode = FileState(destination).inode
    let sourceBefore = FileState(original)
    check(!DataMigration.fileMatches(original, destination), "SHA-256 detects a same-size corrupted destination")
    check(DataMigration.syncTreeVerified(source, target), "a corrupt existing destination is replaced with a verified copy")
    check(FileState(destination).inode != corruptInode, "replacement publishes a new destination inode")
    check(try Data(contentsOf: destination) == correct, "replacement contains the source bytes")
    let asides = try fm.contentsOfDirectory(atPath: target.path)
        .filter { $0.hasPrefix("state.redb.eastsea-replaced-") }
    check(asides.count == 1, "the previous destination is kept aside")
    check(FileState(target.appendingPathComponent(asides[0])).inode == corruptInode, "the aside preserves the old destination inode")
    check(try Data(contentsOf: target.appendingPathComponent(asides[0])) == corrupt, "the aside preserves the old destination bytes")
    check(FileState(original) == sourceBefore, "repair leaves the source untouched")
    print("OK corruption and replacement: same-size damage detected, previous destination preserved")
}

func corruptCopyBeforeVerification() throws {
    let source = root.appendingPathComponent("copy-corruption/source", isDirectory: true)
    let target = root.appendingPathComponent("copy-corruption/target", isDirectory: true)
    let bytes: Int64 = (8 << 20) + 17
    let fixture = try SparseFile(at: source.appendingPathComponent("state.redb"), bytes: bytes)
    try fm.createDirectory(at: target, withIntermediateDirectories: true)
    let destination = target.appendingPathComponent("state.redb")
    let previous = Data("keep the existing destination".utf8)
    try previous.write(to: destination)
    let before = FileState(destination)
    let sourceBefore = FileState(fixture.url)
    var injected = false
    var reportedComplete = false
    let meter = DataMigration.ProgressMeter(reportEvery: 1) { fraction in
        if fraction == 1 { reportedComplete = true }
        guard !injected && fraction >= 0.5 else { return }
        let staged = migratingFiles(in: target, file: "state.redb")
        guard let temporary = staged.first, FileState(temporary).bytes == bytes else { return }
        let fd = open(temporary.path, O_WRONLY | O_CLOEXEC)
        check(fd >= 0, "open private destination for corruption fixture")
        writeAt(fd, data: Data([0xee]), offset: 0)
        close(fd)
        injected = true
    }
    meter.expect(bytes * 2)
    check(!DataMigration.syncTreeVerified(source, target, meter: meter), "a corrupted temporary destination fails hash verification")
    check(injected, "fixture corrupts the fully copied temporary destination before its verification")
    check(!reportedComplete, "failed destination verification never advertises 100%")
    check(FileState(destination) == before, "a failed verification leaves the existing destination in place")
    check(try Data(contentsOf: destination) == previous, "failed verification preserves existing destination bytes")
    check(FileState(fixture.url) == sourceBefore, "failed verification leaves the source untouched")
    fixture.checkContents(at: fixture.url)
    check(migratingFiles(in: target, file: "state.redb").isEmpty, "a failed verification removes only its private temporary file")
    print("OK copy corruption: damage before verification is detected without publishing or replacing data")
}

do {
    try fm.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? fm.removeItem(at: root) }
    try largeCopyProgress()
    try verifiedResume()
    try unevenTreeProgress()
    try corruptResumeProgress()
    try corruptExistingDestination()
    try corruptCopyBeforeVerification()
    print("block-data-progress: all checks passed")
} catch {
    check(false, "fixture setup failed: \(error)")
}
