import Foundation

// What this Mac has earned, from the node's own reward records (`aether_rewards`).
// Pure values and functions only, so the numbers the hero card shows can be checked
// without a node, a window or a clock.

/// One reward the chain paid to this Mac's prover address.
struct RewardEntry: Equatable, Sendable {
    /// The block whose proof earned it.
    let proven: UInt64
    /// Wei, as a decimal string (exact; can exceed 64 bits).
    let amountWei: String
    /// The block that paid it.
    let height: UInt64
    let time: Date
    /// "node" (an epoch's distribution paid this operator) or "proof" (a
    /// proven block's reward): the node's own label, derived from
    /// proven == height when the row did not carry one.
    let kind: String
    /// The paying block's timestamp as the node reported it (0: not reported —
    /// the export then leaves the time column empty rather than guessing).
    let timestampMs: UInt64

    /// One row of `aether_rewards` (numbers or "0x…" hex); nil when a field is missing.
    init?(json row: [String: Any]) {
        guard let height = Self.uint(row["height"]) else { return nil }
        self.proven = Self.uint(row["proven"]) ?? 0
        self.amountWei = WeiMath.decimal(row["amount"])
        self.height = height
        let ms = Self.uint(row["timestamp_ms"]) ?? 0
        self.timestampMs = ms
        self.time = Date(timeIntervalSince1970: Double(ms) / 1000)
        if let k = row["kind"] as? String, k == "node" || k == "proof" {
            self.kind = k
        } else {
            self.kind = proven == height ? "node" : "proof"
        }
    }

    init(proven: UInt64, amountWei: String, height: UInt64, time: Date, kind: String? = nil, timestampMs: UInt64 = 0) {
        self.proven = proven
        self.amountWei = amountWei
        self.height = height
        self.time = time
        self.kind = kind ?? (proven == height ? "node" : "proof")
        self.timestampMs = timestampMs
    }

    private static func uint(_ v: Any?) -> UInt64? {
        if let n = v as? NSNumber { return n.uint64Value }
        guard let s = v as? String else { return nil }
        return s.hasPrefix("0x") ? UInt64(s.dropFirst(2), radix: 16) : UInt64(s)
    }
}

/// The chain's node-rewards standing, from `aether_rewardStatus [operator]`
/// (docs/design/15-node-rewards.md "상태 조회 RPC"). A testnet without node
/// rewards answers `{"enabled": false}` — the card then shows nothing new.
struct RewardStatus: Equatable, Sendable {
    var enabled = false
    /// N: the operators that shared the last epoch's pool.
    var operatorsOnline = 0
    /// The per-operator cap: one operator gets at most 1/max_share.
    var maxShare = 16
    /// This operator's best Mac's warm-up, as the chain reports it: "정상 몫의 %".
    var warmupPercent: Int?
    /// Whole days of warm-up left at one level a day (nil: no Mac of ours listed).
    var warmupDaysLeft: Int?
    /// This operator's share of the last epoch's pool (wei).
    var expectedShareWei: String?
    var capped = false

    /// One `aether_rewardStatus` answer. Numbers arrive as NSNumber or string.
    init(json: [String: Any]) {
        enabled = (json["enabled"] as? Bool) ?? false
        operatorsOnline = Self.int(json["operators_online_last_epoch"]) ?? 0
        maxShare = Self.int(json["max_share"]) ?? 16
        if let op = json["operator"] as? [String: Any] {
            let macs = (op["macs"] as? [[String: Any]]) ?? []
            warmupPercent = macs.compactMap { Self.int($0["warmup_percent"]) }.max()
            if let level = macs.compactMap({ Self.int($0["warmup_level"]) }).max() {
                warmupDaysLeft = max(0, 14 - min(level, 14))
            }
            expectedShareWei = op["expected_share_last_epoch"] as? String
            capped = (op["capped"] as? Bool) ?? false
        }
    }

    private static func int(_ v: Any?) -> Int? {
        if let n = v as? NSNumber { return n.intValue }
        return (v as? String).flatMap(Int.init)
    }
}

/// Totals over the rewards actually received. Never a projection.
struct EarningsSummary: Equatable, Sendable {
    var totalWei = "0"
    var lastHourWei = "0"
    var todayWei = "0"
    var count = 0
    var lastRewardAt: Date?
    var lastRewardWei: String?
    /// The paying block of the newest reward: a new reward arrived when this grows.
    var latestHeight: UInt64?

    static let empty = EarningsSummary()

    /// Sum `entries` as of `now`; "today" is the calendar day of `now` in `calendar`.
    static func aggregate(_ entries: [RewardEntry], now: Date, calendar: Calendar = .current) -> EarningsSummary {
        let hourAgo = now.addingTimeInterval(-3_600)
        let midnight = calendar.startOfDay(for: now)
        var s = EarningsSummary()
        for e in entries {
            s.totalWei = WeiMath.add(s.totalWei, e.amountWei)
            if e.time > hourAgo && e.time <= now { s.lastHourWei = WeiMath.add(s.lastHourWei, e.amountWei) }
            if e.time >= midnight && e.time <= now { s.todayWei = WeiMath.add(s.todayWei, e.amountWei) }
        }
        s.count = entries.count
        if let newest = entries.max(by: { ($0.height, $0.time) < ($1.height, $1.time) }) {
            s.lastRewardAt = newest.time
            s.lastRewardWei = newest.amountWei
            s.latestHeight = newest.height
        }
        return s
    }

    /// The reward(s) that arrived since `old`: the amount to celebrate, or nil if nothing new.
    /// (The caller skips the very first poll, so history is not celebrated.)
    func arrived(since old: EarningsSummary) -> String? {
        guard let h = latestHeight, h > (old.latestHeight ?? 0) || count > old.count else { return nil }
        let delta = WeiMath.subtract(totalWei, old.totalWei)
        return delta == "0" ? lastRewardWei : delta
    }
}

/// Exact arithmetic on non-negative decimal strings of wei.
enum WeiMath {
    static func add(_ a: String, _ b: String) -> String {
        let x = Array(a.utf8.reversed()), y = Array(b.utf8.reversed())
        var out: [UInt8] = []
        var carry: UInt8 = 0
        for i in 0..<max(x.count, y.count) {
            let d = (i < x.count ? x[i] - 48 : 0) + (i < y.count ? y[i] - 48 : 0) + carry
            out.append(d % 10 + 48)
            carry = d / 10
        }
        if carry > 0 { out.append(carry + 48) }
        return trim(String(decoding: out.reversed(), as: UTF8.self))
    }

    /// a - b, or "0" when b >= a.
    static func subtract(_ a: String, _ b: String) -> String {
        guard compare(a, b) > 0 else { return "0" }
        let x = Array(a.utf8.reversed()), y = Array(b.utf8.reversed())
        var out: [UInt8] = []
        var borrow = 0
        for i in 0..<x.count {
            var d = Int(x[i]) - 48 - (i < y.count ? Int(y[i]) - 48 : 0) - borrow
            borrow = d < 0 ? 1 : 0
            if d < 0 { d += 10 }
            out.append(UInt8(d + 48))
        }
        return trim(String(decoding: out.reversed(), as: UTF8.self))
    }

    static func compare(_ a: String, _ b: String) -> Int {
        let a = trim(a), b = trim(b)
        if a.count != b.count { return a.count < b.count ? -1 : 1 }
        return a == b ? 0 : (a < b ? -1 : 1)
    }

    /// A U256 from JSON ("0x…" hex, a decimal string or a number) as decimal wei.
    static func decimal(_ v: Any?) -> String {
        if let n = v as? NSNumber { return n.stringValue }
        guard let s = v as? String else { return "0" }
        guard s.hasPrefix("0x") else { return s.allSatisfy(\.isNumber) && !s.isEmpty ? trim(s) : "0" }
        var digits: [UInt8] = [0]  // little-endian base 10
        for c in s.dropFirst(2) {
            guard let d = c.hexDigitValue else { return "0" }
            var carry = d
            for i in digits.indices {
                let x = Int(digits[i]) * 16 + carry
                digits[i] = UInt8(x % 10)
                carry = x / 10
            }
            while carry > 0 {
                digits.append(UInt8(carry % 10))
                carry /= 10
            }
        }
        return trim(digits.reversed().map(String.init).joined())
    }

    /// Wei as AETH, for display and the count-up animation (not for sums).
    static func aeth(_ wei: String) -> Double {
        let padded = String(repeating: "0", count: max(0, 19 - wei.count)) + wei
        return Double("\(padded.dropLast(18)).\(padded.suffix(18))") ?? 0
    }

    private static func trim(_ s: String) -> String {
        let t = s.drop(while: { $0 == "0" })
        return t.isEmpty ? "0" : String(t)
    }
}
