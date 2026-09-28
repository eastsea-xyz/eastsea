import Foundation

// The balance chart's data, as pure functions. The card caches the result outside
// `body`: recomputing it on every layout pass (each step of a window resize, each
// 2 s model tick) fed Swift Charts a slightly different series every time — a new
// "now" point each pass — so the chart kept re-scaling and re-animating its marks.
// With the math here it can be rebuilt only when the history, the range or the
// minute changes, and checked without a node or a window.

/// This device's observation of the balance, at the moment it changed (or once a
/// minute otherwise); one point of the dashboard chart.
struct BalancePoint: Codable, Identifiable, Equatable {
    var id: Date { date }
    let date: Date
    let aeth: Double
}

enum BalanceHistory {
    /// The points to draw for `history` in a window of `range` seconds (nil: all of it).
    /// The last earlier value is carried in at the window's start so the line begins at
    /// the left edge, and the last known balance is extended to `now` so even a single
    /// observation draws a line.
    static func points(_ history: [BalancePoint], range: TimeInterval?, now: Date) -> [BalancePoint] {
        guard let range else { return extend(history, to: now) }
        let from = now.addingTimeInterval(-range)
        let inRange = history.filter { $0.date >= from }
        var pts = inRange
        if let before = history.last(where: { $0.date < from }) {
            pts.insert(BalancePoint(date: from, aeth: before.aeth), at: 0)
        }
        return extend(pts, to: now)
    }

    private static func extend(_ pts: [BalancePoint], to now: Date) -> [BalancePoint] {
        guard let last = pts.last, now.timeIntervalSince(last.date) > 1 else { return pts }
        return pts + [BalancePoint(date: now, aeth: last.aeth)]
    }
}
