import Foundation

// "Where does my balance come from?", as the wallet renders it. The sums are
// exact wei computed by the Rust core (`balance_sources`, integer math only);
// this file is the pure Swift side — decoding, timestamps and the shapes the
// views show — so all of it is checked without a node, a window or a clock.

/// Block timestamps as this wallet renders them — see `Timestamp` in
/// EarningsModel.swift (a missing time renders as nothing, never as 1970).

/// The itemized answer to "where did my coins come from": per-source wei
/// totals that either add up to the certificate-verified balance exactly, or
/// leave an honest difference (history not loaded yet, pruned rows).
struct BalanceBreakdown: Decodable, Equatable, Sendable {
    let proofRewardsWei: String
    let nodeRewardsWei: String
    let faucetWei: String
    let receivedWei: String
    let unwrappedWei: String
    let sentWei: String
    let feesWei: String
    let totalInWei: String
    let totalOutWei: String
    let balanceWei: String
    /// balance + out − in: "0" when every coin is accounted for; otherwise the
    /// remainder this wallet cannot yet name (a "-" prefix means the loaded
    /// rows claim more than the balance holds — e.g. the index is mid-write).
    let differenceWei: String
    let itemizesCompletely: Bool
    let rows: Int

    static func decode(_ json: String) throws -> Self {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(Self.self, from: Data(json.utf8))
    }
}

/// One line of the breakdown card: a source, and what it added or took.
struct BalanceSourceLine: Identifiable, Equatable {
    let name: String
    let wei: String
    let negative: Bool
    var id: String { name }
}

/// Pure shaping of a `BalanceBreakdown` for display.
enum BalanceBreakdownText {
    /// The sources that moved anything, in a stable order: every inflow first,
    /// then sent and fees as deductions. Sources at zero are not shown (the
    /// card never lists what never happened).
    static func lines(_ b: BalanceBreakdown) -> [BalanceSourceLine] {
        let inflow = [
            (String(localized: "Proof rewards"), b.proofRewardsWei),
            (String(localized: "Node rewards"), b.nodeRewardsWei),
            (String(localized: "Received"), b.receivedWei),
            (String(localized: "Faucet"), b.faucetWei),
            (String(localized: "Unwrapped"), b.unwrappedWei),
        ]
        let outflow = [
            (String(localized: "Sent"), b.sentWei),
            (String(localized: "Fees"), b.feesWei),
        ]
        return (inflow.map { ($0.0, $0.1, false) } + outflow.map { ($0.0, $0.1, true) })
            .filter { $0.1 != "0" }
            .map { BalanceSourceLine(name: $0.0, wei: $0.1, negative: $0.2) }
    }

    /// The honest line under the card: nothing when the sums are exact, the
    /// unexplained remainder otherwise. A negative difference (rows claiming
    /// more than the balance) is named as it is, not silently flipped.
    static func notYetItemized(_ b: BalanceBreakdown) -> String? {
        guard !b.itemizesCompletely else { return nil }
        if b.differenceWei.hasPrefix("-") {
            let over = ChainActivity.units(String(b.differenceWei.dropFirst()))
            return String(localized: "\(over) \(Brand.networkCoinTicker) more itemized than the balance shows — try again in a moment")
        }
        let missing = ChainActivity.units(b.differenceWei)
        return String(localized: "\(missing) \(Brand.networkCoinTicker) not yet itemized — load older activity to account for it")
    }
}

/// One day's rewards, grouped for the Activity page: the count and the exact
/// sum up front, the individual blocks expanded underneath.
struct RewardDay: Identifiable, Equatable {
    /// The calendar day (nil when the node reported no time for every row in
    /// the group — those keep their place instead of being dropped).
    let day: Date?
    let count: Int
    let proofWei: String
    let nodeWei: String
    let rows: [ChainHistoryEntry]

    var totalWei: String { WeiMath.add(proofWei, nodeWei) }
    var id: String { day.map { String(Int($0.timeIntervalSince1970)) } ?? "unknown" }
}

/// Pure grouping of reward rows into calendar days (the 7780 unit fix is
/// applied here too, so backed-up seconds still land on their day).
enum RewardDays {
    static func group(_ rows: [ChainHistoryEntry], calendar: Calendar = .current) -> [RewardDay] {
        var byDay: [(Date?, RewardDay)] = []
        for row in rows where row.kind == "proof_reward" || row.kind == "node_reward" {
            let date = Timestamp.date(fromMs: row.timestampMs)
            let key = date.map { calendar.startOfDay(for: $0) }
            let node = row.kind == "node_reward"
            if let i = byDay.firstIndex(where: { $0.0 == key }) {
                let old = byDay[i].1
                byDay[i].1 = RewardDay(day: key,
                                       count: old.count + 1,
                                       proofWei: node ? old.proofWei : WeiMath.add(old.proofWei, row.valueWei),
                                       nodeWei: node ? WeiMath.add(old.nodeWei, row.valueWei) : old.nodeWei,
                                       rows: old.rows + [row])
            } else {
                byDay.append((key, RewardDay(day: key, count: 1,
                                             proofWei: node ? "0" : row.valueWei,
                                             nodeWei: node ? row.valueWei : "0",
                                             rows: [row])))
            }
        }
        // Newest day first; the unknown-time group stays last, never first
        // (an unknown time is not the newest thing the wallet knows).
        return byDay.sorted(by: newestFirst).map(\.1)
    }

    private static func newestFirst(_ a: (Date?, RewardDay), _ b: (Date?, RewardDay)) -> Bool {
        switch (a.0, b.0) {
        case (nil, _): return false
        case (_, nil): return true
        case (let x?, let y?): return x > y
        }
    }
}
