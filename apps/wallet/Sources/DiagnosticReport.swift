import Foundation

/// "진단 정보 복사" (docs/design/32-health-signal.md §4.2, W4): the same
/// fields the opt-in layer-2 report would carry (§2.2), as text a person
/// reads, for the clipboard only. Nothing here touches the network: whether
/// to share it, and where, is the person's choice.
///
/// The snapshot holds what the app has at hand — including the address, the
/// balance, the node ID and exact heights — and the text keeps only the
/// buckets and public values of §2.2. That is the point of taking the whole
/// state as input: the property test (Tests/diagnostic-report) fills those
/// fields with random values and proves none of them ever reaches the text,
/// so a later edit that prints one fails it. Free-text inputs (the version
/// string, the program ID, failure kinds) are reduced to closed forms for
/// the same reason.
///
/// Layer 2's `anchor` and `pow` are anti-spam for a network submission and
/// have no meaning in a local copy, so they are left out; `release` stays
/// `null` until the app checks its binaries against a ReleaseLog manifest
/// (§2.2 — layer 2's job).
enum DiagnosticReport {
    struct Snapshot {
        /// CFBundleShortVersionString.
        var appVersion = ""
        /// False for a development build (`custom`, like layer 2).
        var publicBuild = true
        var nodeProtocol: UInt64?
        var newestScheduled: UInt64?
        /// The validators' guest program ID (`aether_proverStatus.network_program`).
        var networkProgram: String?
        /// `program_mismatch` inverted; nil when this Mac does not prove.
        var programMatches: Bool?
        var nodeRunning = false
        var voting = false
        var proving = false
        /// This node's height and the network's: only their difference is
        /// reported, in buckets — never either height.
        var localHeight: UInt64?
        var networkHeight: UInt64?
        /// How old the newest finalized block this Mac saw is.
        var finalizedAge: TimeInterval?
        /// macOS major version.
        var osMajor = 0
        /// Failure kind → count over the last 24 hours (`HealthCheck.failureCounts`).
        var failures: [String: Int] = [:]
        var now = Date()
        // In the snapshot, never in the text:
        var address = ""
        var balanceWei = ""
        var nodeId = ""
        /// Why the node is not running now (`NodeStopReason.code`), and the
        /// last stop's code from node-status.log — codes only: no time, no paths.
        var nodeStop: String?
        var lastStop: String?
    }

    /// The closed failure kinds of §2.4; anything else is counted as `other`.
    static let failureKinds = [
        "proof_rejected", "prover_program_mismatch", "prover_stalled", "disk_floor_pause", "disk_full",
        "follower_stuck", "rpc_unreachable", "crash_loop", "upgrade_required", "update_unhealthy", "node_stopped",
    ]

    /// The text for the clipboard. Keys are layer 2's field names, so a
    /// developer reading a pasted copy maps it one to one.
    static func text(_ s: Snapshot, ko: Bool = HealthCheck.korean) -> String {
        var lines = [
            ko ? "\(Brand.projectKo) 진단 정보 — 주소·잔액·노드 ID는 들어 있지 않아요. 이 글은 이 Mac 밖으로 보내지지 않았어요."
                : "\(Brand.project) diagnostics — no address, balance or node ID inside. Nothing was sent from this Mac.",
            "v: 1",
            "day: \(day(s.now))",
            "app: \(app(s))",
            "release: null",
            "protocol: \(s.nodeProtocol.map(String.init) ?? "unknown") / \(s.newestScheduled.map(String.init) ?? "unknown")",
        ]
        if s.proving {
            lines.append("prover_program: \(program(s)) (matches_network: \(s.programMatches.map { $0 ? "true" : "false" } ?? "unknown"))")
        }
        lines += [
            "role: \(roles(s).joined(separator: ", "))",
            "lag: \(s.nodeRunning ? lag(local: s.localHeight, network: s.networkHeight) : "unknown")",
            "finalized_age: \(age(s.finalizedAge))",
            "os: macos-\(s.osMajor)",
            "failures: \(failures(s.failures))",
            "node_stop: \(s.nodeStop ?? "none")",
            "last_stop: \(s.lastStop ?? "none")",
        ]
        return lines.joined(separator: "\n")
    }

    /// The UTC date only — never a time of day.
    static func day(_ date: Date) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC") ?? .current
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }

    /// A public release string (`1.4.2`) as is; anything else is `custom`.
    static func app(_ s: Snapshot) -> String {
        let parts = s.appVersion.split(separator: ".", omittingEmptySubsequences: false)
        let numeric = (1...4).contains(parts.count)
            && parts.allSatisfy { !$0.isEmpty && $0.count <= 4 && $0.allSatisfy(\.isASCIIDigit) }
        return s.publicBuild && numeric ? s.appVersion : "custom"
    }

    /// The first 8 hex digits of the guest program ID (the same value on every
    /// install of a public build), or `custom`/`unknown`.
    static func program(_ s: Snapshot) -> String {
        guard s.publicBuild else { return "custom" }
        var hex = (s.networkProgram ?? "").lowercased()
        if hex.hasPrefix("0x") { hex.removeFirst(2) }
        guard s.programMatches == true, hex.count >= 8, hex.allSatisfy(\.isHexDigit) else { return "unknown" }
        return String(hex.prefix(8))
    }

    static func roles(_ s: Snapshot) -> [String] {
        var roles = ["wallet"]
        if s.nodeRunning { roles.append("follower") }
        if s.nodeRunning && s.voting { roles.append("voting") }
        if s.nodeRunning && s.proving { roles.append("prover") }
        return roles
    }

    /// §2.2's lag buckets, in blocks.
    static func lag(local: UInt64?, network: UInt64?) -> String {
        guard let local, let network else { return "unknown" }
        let behind = network > local ? network - local : 0
        switch behind {
        case 0: return "0"
        case 1...10: return "1-10"
        case 11...100: return "11-100"
        case 101...1_000: return "101-1000"
        default: return "1000+"
        }
    }

    /// §2.2's finalized-age buckets.
    static func age(_ seconds: TimeInterval?) -> String {
        guard let seconds, seconds.isFinite, seconds >= 0 else { return "unknown" }
        switch seconds {
        case ..<60: return "<1m"
        case ..<600: return "1-10m"
        case ..<3_600: return "10-60m"
        default: return ">1h"
        }
    }

    /// §2.2's count buckets: 1 / 2-5 / 6+, known kinds only, sorted.
    static func failures(_ counts: [String: Int]) -> String {
        var merged: [String: Int] = [:]
        for (kind, n) in counts where n > 0 {
            merged[failureKinds.contains(kind) ? kind : "other", default: 0] += n
        }
        guard !merged.isEmpty else { return "none" }
        return merged.keys.sorted().map { kind in
            let n = merged[kind] ?? 0
            return "\(kind) \(n == 1 ? "1" : n <= 5 ? "2-5" : "6+")"
        }.joined(separator: ", ")
    }
}

private extension Character {
    var isASCIIDigit: Bool { ("0"..."9").contains(self) }
}
