// "진단 정보 복사" (docs/design/32-health-signal.md §4.2, W4): the layer-2
// fields as local text, and a property test that no address, balance, node ID
// or exact height ever reaches it. Pure logic:
//   swiftc -o ./tmp/diagnostic-report apps/wallet/Sources/Brand.swift apps/wallet/Sources/Clock.swift apps/wallet/Sources/NodeWatchdog.swift apps/wallet/Sources/HealthCheck.swift apps/wallet/Sources/DiagnosticReport.swift apps/wallet/Tests/diagnostic-report/main.swift && ./tmp/diagnostic-report
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

// MARK: the fields, by example (design §2.3)

var s = DiagnosticReport.Snapshot()
s.appVersion = "1.4.2"
s.nodeProtocol = 3
s.newestScheduled = 3
s.networkProgram = "0x3f629756aa01bb02cc03dd04ee05ff06aa01bb02cc03dd04ee05ff06aa01bb02"
s.programMatches = true
s.nodeRunning = true
s.proving = true
s.localHeight = 1_234_500
s.networkHeight = 1_234_500
s.finalizedAge = 4
s.osMajor = 15
s.failures = ["prover_program_mismatch": 7, "proof_rejected": 3]
s.now = Date(timeIntervalSince1970: 1_791_288_000)  // 2026-10-06 12:00 UTC
let text = DiagnosticReport.text(s, ko: false)
let lines = text.split(separator: "\n").map(String.init)
check(lines.contains("v: 1"), "format version")
check(lines.contains("day: 2026-10-06"), "the UTC day, no time of day")
check(lines.contains("app: 1.4.2"), "a public release version as is")
check(lines.contains("release: null"), "release stays null (layer 2 checks the manifest)")
check(lines.contains("protocol: 3 / 3"), "node protocol / newest scheduled")
check(lines.contains("prover_program: 3f629756 (matches_network: true)"), "eight hex digits of the program ID")
check(lines.contains("role: wallet, follower, prover"), "roles")
check(lines.contains("lag: 0"), "lag bucket")
check(lines.contains("finalized_age: <1m"), "finalized age bucket")
check(lines.contains("os: macos-15"), "OS major only")
check(lines.contains("failures: proof_rejected 2-5, prover_program_mismatch 6+"), "failure buckets, sorted")
check(!text.contains(":00") && !text.contains("12:"), "no time of day anywhere")
check(DiagnosticReport.text(s, ko: true).hasPrefix("동해 진단 정보"), "Korean header")
// Remote diagnosis (the founder's MacBook, 2026-10-07): the stop reason travels in the copy.
var stopped = s
stopped.nodeStop = "disk_missing"; stopped.lastStop = "crash_loop"
check(DiagnosticReport.text(stopped, ko: false).contains("node_stop: disk_missing")
      && DiagnosticReport.text(stopped, ko: false).contains("last_stop: crash_loop"), "the stop reasons are in the copy (codes only)")
check(DiagnosticReport.text(s, ko: false).contains("node_stop: none"), "none when running")

// Buckets at their edges (§2.2).
check(DiagnosticReport.lag(local: 100, network: 100) == "0", "lag 0")
check(DiagnosticReport.lag(local: 110, network: 100) == "0", "ahead counts as 0")
check(DiagnosticReport.lag(local: 90, network: 100) == "1-10", "lag 10")
check(DiagnosticReport.lag(local: 89, network: 100) == "11-100", "lag 11")
check(DiagnosticReport.lag(local: 0, network: 100) == "11-100", "lag 100")
check(DiagnosticReport.lag(local: 0, network: 101) == "101-1000", "lag 101")
check(DiagnosticReport.lag(local: 0, network: 1_000) == "101-1000", "lag 1000")
check(DiagnosticReport.lag(local: 0, network: 1_001) == "1000+", "lag 1001")
check(DiagnosticReport.lag(local: nil, network: 5) == "unknown", "lag unknown")
check(DiagnosticReport.age(59) == "<1m" && DiagnosticReport.age(60) == "1-10m", "age 1 min edge")
check(DiagnosticReport.age(599) == "1-10m" && DiagnosticReport.age(600) == "10-60m", "age 10 min edge")
check(DiagnosticReport.age(3_599) == "10-60m" && DiagnosticReport.age(3_600) == ">1h", "age 1 h edge")
check(DiagnosticReport.age(nil) == "unknown" && DiagnosticReport.age(-5) == "unknown" && DiagnosticReport.age(.nan) == "unknown", "age unknown")
check(DiagnosticReport.failures([:]) == "none", "no failures")
check(DiagnosticReport.failures(["crash_loop": 1, "follower_stuck": 5, "disk_full": 6]) == "crash_loop 1, disk_full 6+, follower_stuck 2-5", "count buckets 1 / 2-5 / 6+")
check(DiagnosticReport.failures(["zero": 0]) == "none", "zero counts are not failures")

var dev = s
dev.publicBuild = false
check(DiagnosticReport.text(dev).contains("app: custom") && DiagnosticReport.text(dev).contains("prover_program: custom"), "a development build is custom")
for odd in ["1.4.2-beta", "v1.4", "", "1..2", "1.2.3.4.5", "12345.1", "0x1234"] {
    var v = s
    v.appVersion = odd
    check(DiagnosticReport.app(v) == "custom", "\"\(odd)\" is not a public release string")
}
var mismatched = s
mismatched.programMatches = false
check(DiagnosticReport.program(mismatched) == "unknown", "on a mismatch this Mac's own program is not the network's: unknown")
var noNode = s
noNode.nodeRunning = false
noNode.proving = false
check(DiagnosticReport.text(noNode).contains("role: wallet\n") && DiagnosticReport.text(noNode).contains("lag: unknown"), "wallet only: no node roles, lag unknown")
check(!DiagnosticReport.text(noNode).contains("prover_program:"), "no prover line without proving")

// MARK: property: nothing identifying ever reaches the text

/// SplitMix64: deterministic, so a failure reproduces.
struct Rng {
    var state: UInt64
    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
    mutating func hex(_ n: Int) -> String { String((0..<n).map { _ in "0123456789abcdef".randomElement(using: &self)! }) }
    mutating func digits(_ n: Int) -> String { String((0..<n).map { i in (i == 0 ? "123456789" : "0123456789").randomElement(using: &self)! }) }
    mutating func int(_ r: ClosedRange<UInt64>) -> UInt64 { r.lowerBound + next() % (r.upperBound - r.lowerBound + 1) }
    mutating func bool() -> Bool { next() & 1 == 1 }
}
extension Rng: RandomNumberGenerator {}

/// Every 12-character window of a secret: a leak of any recognisable part
/// fails, not only of the whole string.
func windows(_ secret: String) -> [String] {
    let chars = Array(secret)
    guard chars.count > 12 else { return [secret] }
    return (0...(chars.count - 12)).map { String(chars[$0..<($0 + 12)]) }
}

var rng = Rng(state: 0xEA57_5EA0)
for round in 0..<2_000 {
    var r = DiagnosticReport.Snapshot()
    let address = "0x" + rng.hex(40)
    let balance = rng.digits(Int(rng.int(18...30)))
    let nodeId = rng.hex(64)
    r.address = address
    r.balanceWei = balance
    r.nodeId = nodeId
    r.localHeight = rng.int(1_000_000...9_000_000_000)
    r.networkHeight = rng.bool() ? r.localHeight! + rng.int(0...5_000) : rng.int(1_000_000...9_000_000_000)
    r.nodeRunning = rng.bool()
    r.voting = rng.bool()
    r.proving = rng.bool()
    r.publicBuild = rng.bool()
    r.programMatches = rng.bool() ? rng.bool() : nil
    r.nodeProtocol = rng.int(1...9)
    r.newestScheduled = rng.int(1...9)
    r.osMajor = Int(rng.int(14...26))
    r.finalizedAge = Double(rng.int(0...100_000))
    r.now = Date(timeIntervalSince1970: Double(rng.int(1_700_000_000...1_900_000_000)))
    // Adversarial: secrets placed in every free-text input the text reads.
    switch round % 5 {
    case 0: r.appVersion = address
    case 1: r.appVersion = balance
    case 2: r.appVersion = nodeId
    case 3: r.appVersion = "1.\(rng.int(0...99)).\(rng.int(0...99))"
    default: r.appVersion = "\(balance.prefix(4)).1"
    }
    r.networkProgram = [address, nodeId, "0x" + nodeId, rng.hex(64)][Int(rng.int(0...3))]
    r.failures = [address: 2, nodeId: 1, balance: 9, "crash_loop": Int(rng.int(0...8))]
    for ko in [true, false] {
        let out = DiagnosticReport.text(r, ko: ko)
        let lowered = out.lowercased()
        for secret in [address, String(address.dropFirst(2)), balance, nodeId] {
            for w in windows(secret) {
                check(!lowered.contains(w.lowercased()), "round \(round): '\(w)' of a secret leaked into:\n\(out)")
            }
        }
        for h in [r.localHeight!, r.networkHeight!] {
            check(!out.contains(String(h)), "round \(round): the exact height \(h) leaked")
        }
        // Only the closed set of keys, one per line.
        let keys = out.split(separator: "\n").dropFirst().map { $0.split(separator: ":").first.map(String.init) ?? "" }
        check(Set(keys).isSubset(of: ["v", "day", "app", "release", "protocol", "prover_program", "role", "lag", "finalized_age", "os", "failures", "node_stop", "last_stop"]),
              "round \(round): only layer-2 field names, got \(keys)")
        check(out.utf8.count < 1_024, "round \(round): the text stays short")
    }
}

print("diagnostic-report: all checks passed")
