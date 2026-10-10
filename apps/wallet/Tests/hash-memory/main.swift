// Run only a test executable, never the wallet or a real node. Each hasher
// gets a fresh process and a detached task with one outer autorelease pool,
// reproducing the lifetime of the app's background file hashing.
import Foundation
import CryptoKit
import Darwin

let mebibyte: UInt64 = 1 << 20
let fixtureBytes: Int64 = (2 << 30) + 73
// Independently generated with Python hashlib over 2048 zeroed 1 MiB blocks
// followed by 73 zero bytes. The unaligned tail exercises incremental padding.
let fixtureDigest = "4f82ac3c99c5294fcfbda4d2b040c589ff4d1a12596346dd3352a735da31833c"
// Independent Python hashlib vector over exactly 2048 zeroed 1 MiB blocks.
let artifactLimitDigest = "a7c744c13cc101ed66c29f672f92455547889cc586ce6d44fe76ae824958ea51"
let memoryLimit = 64 * mebibyte

func footprint() -> UInt64 {
    var info = task_vm_info_data_t()
    var count = mach_msg_type_number_t(MemoryLayout.size(ofValue: info) / MemoryLayout<integer_t>.size)
    let result = withUnsafeMutablePointer(to: &info) { ptr in
        ptr.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
            task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
        }
    }
    precondition(result == KERN_SUCCESS, "task_info must succeed; never pass with an unmeasured footprint")
    return info.phys_footprint
}

final class PeakMemory {
    let before = footprint()
    private let lock = NSLock()
    private var peak: UInt64 = 0
    private var stopped = false
    private let joined = DispatchSemaphore(value: 0)
    private let label: String

    init(_ label: String) {
        self.label = label
        peak = before
        Thread { [self] in
            while true {
                lock.lock()
                let done = stopped
                lock.unlock()
                if done { break }
                let current = sample()
                // Protect this development Mac during the deliberately failing
                // baseline. The acceptance limit remains 64 MiB, not 256 MiB.
                if current > before + 256 * mebibyte {
                    let format = "FAIL %@ safety stop: before=%.2f MiB peak=%.2f MiB"
                        + " growth=%.2f MiB (limit 64 MiB)"
                    print(String(format: format,
                                 label, Double(before) / Double(mebibyte), Double(current) / Double(mebibyte),
                                 Double(current - before) / Double(mebibyte)))
                    fflush(stdout)
                    exit(1)
                }
                Thread.sleep(forTimeInterval: 0.005)
            }
            joined.signal()
        }.start()
    }

    @discardableResult func sample() -> UInt64 {
        let current = footprint()
        lock.lock()
        peak = max(peak, current)
        lock.unlock()
        return current
    }

    func finish() -> (peak: UInt64, after: UInt64) {
        let after = sample()
        lock.lock()
        stopped = true
        lock.unlock()
        joined.wait()
        return (peak, after)
    }
}

func measure(_ algorithm: String, file: URL, bytes: Int64, expected: String?) -> Int32 {
    let finished = DispatchSemaphore(value: 0)
    var status: Int32 = 1
    Task.detached {
        autoreleasepool {
            let memory = PeakMemory(algorithm)
            let start = ProcessInfo.processInfo.systemUptime
            let digest: String?
            var sizeLimitRefused = false
            if algorithm == "migration" {
                var progress = 0.0
                let meter = DataMigration.ProgressMeter(reportEvery: 1) { progress = $0 }
                meter.expect(bytes)
                digest = DataMigration.streamSHA256(file, meter: meter)
                precondition(progress == 1, "hashing must report all file bytes")
            } else if algorithm.hasPrefix("artifact") {
                do { digest = try ReleaseArtifact.digest(file: file) }
                catch {
                    digest = nil
                    sizeLimitRefused = (error as? ChainReleaseFailure) == .hashMismatch
                }
            } else {
                digest = try? ReleaseUpdateGate.archiveSHA256(file)
            }
            let elapsed = ProcessInfo.processInfo.systemUptime - start
            let result = memory.finish()
            let growth = result.peak - memory.before
            let format = "%@ bytes=%lld before=%.2f MiB peak=%.2f MiB after=%.2f MiB"
                + " growth=%.2f MiB seconds=%.3f MB/s=%.2f sha256=%@"
            print(String(format: format,
                         algorithm, bytes, Double(memory.before) / Double(mebibyte),
                         Double(result.peak) / Double(mebibyte),
                         Double(result.after) / Double(mebibyte), Double(growth) / Double(mebibyte), elapsed,
                         Double(bytes) / elapsed / 1_000_000, digest ?? (sizeLimitRefused ? "SIZE LIMIT" : "READ FAILED")))
            let digestPassed = algorithm == "artifact-over-limit"
                ? sizeLimitRefused && digest == nil
                : digest != nil && (expected == nil || digest == expected)
            let passed = digestPassed && growth < memoryLimit
            status = passed ? 0 : 1
            let verdict = status == 0 ? "ok  " : "FAIL"
            print("\(verdict) \(algorithm) digest and memory <64 MiB")
        }
        print(String(format: "%@ after pool drain=%.2f MiB", algorithm, Double(footprint()) / Double(mebibyte)))
        finished.signal()
    }
    finished.wait()
    return status
}

let arguments = CommandLine.arguments
if arguments.count >= 5, arguments[1] == "--measure" {
    exit(measure(arguments[2], file: URL(fileURLWithPath: arguments[3]), bytes: Int64(arguments[4])!,
                 expected: arguments.count > 5 ? arguments[5] : nil))
}

func makeSparse(_ file: URL, size: Int64) throws {
    let descriptor = open(file.path, O_RDWR | O_CREAT | O_TRUNC, 0o600)
    guard descriptor >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno)!) }
    defer { close(descriptor) }
    guard ftruncate(descriptor, off_t(size)) == 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno)!) }
}

func verifyBoundaries(_ file: URL, check: (Bool, String) -> Void) throws {
    // Pin the two public vectors and every padding/block/chunk boundary against
    // CryptoKit, including a Data slice whose indices do not start at zero.
    check(DataMigration.sha256Hex(Data()) == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
          "empty SHA-256 vector")
    let abcDigest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    check(DataMigration.sha256Hex(Data("abc".utf8)) == abcDigest,
          "abc SHA-256 vector")
    let lengths = [0, 1, 55, 56, 57, 63, 64, 65, 119, 120, 127, 128, (1 << 20) - 1, 1 << 20, (1 << 20) + 73]
    for count in lengths {
        try autoreleasepool {
            let source = Data((0..<(count + 17)).map { UInt8(truncatingIfNeeded: $0 &* 37) })
            let data = source.dropFirst(17)
            let expected = CryptoKit.SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
            check(DataMigration.sha256Hex(data) == expected, "SHA-256 slice/padding boundary \(count)")
            for chunkSize in [1, 7, 63, 64, 65, 127] where count <= 128 {
                var sha = DataMigration.SHA256()
                var start = data.startIndex
                while start < data.endIndex {
                    let end = min(data.endIndex, start + chunkSize)
                    sha.update(data[start..<end])
                    sha.update(Data())
                    start = end
                }
                check(sha.finalHex() == expected, "incremental tail \(count) bytes / \(chunkSize)-byte updates")
            }
            try data.write(to: file)
            check(DataMigration.streamSHA256(file) == expected, "migration file boundary \(count)")
            check((try? ReleaseUpdateGate.archiveSHA256(file)) == expected, "release file boundary \(count)")
            check((try? ReleaseArtifact.digest(file: file)) == expected, "artifact file boundary \(count)")
        }
    }
}

func runTests() throws -> Int32 {
    var failures = 0
    func check(_ passed: Bool, _ message: String) {
        print(passed ? "ok   \(message)" : "FAIL \(message)")
        if !passed { failures += 1 }
    }

    let temporaryRoot = ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"]
        ?? "\(FileManager.default.currentDirectoryPath)/tmp"
    let base = URL(fileURLWithPath: temporaryRoot)
    let directory = base.appendingPathComponent("hash-memory-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let file = directory.appendingPathComponent("state.redb")

    try verifyBoundaries(file, check: check)
    let missing = directory.appendingPathComponent("missing")
    check(DataMigration.streamSHA256(missing) == nil && DataMigration.streamSHA256(directory) == nil,
          "migration fails closed on open/read errors")
    check((try? ReleaseUpdateGate.archiveSHA256(missing)) == nil
          && (try? ReleaseUpdateGate.archiveSHA256(directory)) == nil, "release fails closed on open/read errors")
    check((try? ReleaseArtifact.digest(file: missing)) == nil
          && (try? ReleaseArtifact.digest(file: directory)) == nil, "artifact fails closed on open/read errors")

    check(ChainReleasePolicy.maximumArchiveSize == 2 << 30, "artifact retains the 2 GiB archive cap")
    let measurements: [(String, Int64, String?)] = [
        ("migration", fixtureBytes, fixtureDigest),
        ("release", fixtureBytes, fixtureDigest),
        ("artifact", Int64(ChainReleasePolicy.maximumArchiveSize), artifactLimitDigest),
        ("artifact-over-limit", fixtureBytes, nil),
    ]
    for (algorithm, bytes, expected) in measurements {
        try makeSparse(file, size: bytes)
        fflush(stdout)
        let child = Process()
        child.executableURL = URL(fileURLWithPath: arguments[0]).standardizedFileURL
        var measurementArguments = ["--measure", algorithm, file.path, String(bytes)]
        if let expected { measurementArguments.append(expected) }
        child.arguments = measurementArguments
        try child.run()
        child.waitUntilExit()
        check(child.terminationReason == .exit && child.terminationStatus == 0,
              "\(algorithm) handles >=2 GiB within memory budget and artifact size policy")
    }
    return Int32(failures)
}
exit(try runTests())
