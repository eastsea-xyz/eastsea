// Checks the balance chart's series math (no app, no node):
//   swiftc -o /tmp/balance-history-check apps/wallet/Sources/BalanceHistory.swift apps/wallet/Tests/balance-history/main.swift && /tmp/balance-history-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

let now = Date(timeIntervalSince1970: 1_790_000_000)
func pt(_ ago: TimeInterval, _ aeth: Double) -> BalancePoint { BalancePoint(date: now.addingTimeInterval(-ago), aeth: aeth) }

// "All" keeps everything and extends the last balance to now.
let h = [pt(7_200, 1), pt(3_600, 2), pt(60, 3)]
var p = BalanceHistory.points(h, range: nil, now: now)
check(p.count == 4 && p.map(\.aeth) == [1, 2, 3, 3], "all \(p.map(\.aeth))")
check(p.last!.date == now, "extended to now")

// A window keeps only the points inside it, plus the last earlier one carried in
// at the window's start so the line begins at the left edge.
p = BalanceHistory.points(h, range: 3_700, now: now)
check(p.count == 4, "window count \(p.count)")
check(p[0].date == now.addingTimeInterval(-3_700) && p[0].aeth == 1, "carried in at \(p[0])")
check(p.map(\.aeth) == [1, 2, 3, 3], "window \(p.map(\.aeth))")

// The extension to now holds for windows too (one observation alone still draws a line).
p = BalanceHistory.points([pt(3_600, 5)], range: 3_700, now: now)
check(p.map(\.aeth) == [5, 5] && p.last!.date == now, "single observation \(p.map(\.aeth))")

// Nothing older than the window: no carry-in point.
p = BalanceHistory.points([pt(100, 7)], range: 200, now: now)
check(p.count == 2 && p[0].aeth == 7 && p[1].date == now, "no carry-in \(p)")

// The last point is fresh (within a second): no duplicate "now" point.
p = BalanceHistory.points([pt(0.5, 9), pt(120, 8)].sorted { $0.date < $1.date }, range: nil, now: now)
check(p.count == 2, "fresh last \(p)")

// A window with only one point inside it: the earlier balance is still carried in
// at the left edge, so the step to the current one is visible.
p = BalanceHistory.points(h, range: 60, now: now)
check(p.map(\.aeth) == [2, 3, 3] && p[0].date == now.addingTimeInterval(-60), "narrow window \(p)")

// Empty stays empty; the card shows its placeholder instead of a chart.
check(BalanceHistory.points([], range: nil, now: now).isEmpty, "empty")
check(BalanceHistory.points([], range: 3_600, now: now).isEmpty, "empty window")

print("OK")
