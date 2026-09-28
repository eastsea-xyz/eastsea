// Checks the pure earnings math (no app, no node):
//   swiftc -o /tmp/earnings-check apps/wallet/Sources/EarningsModel.swift apps/wallet/Tests/earnings/main.swift && /tmp/earnings-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }
check(WeiMath.add("999", "1") == "1000", "add carry")
check(WeiMath.add("0", "0") == "0", "add zero")
check(WeiMath.add("500000000000000000000000000000", "500000000000000000000000000000") == "1000000000000000000000000000000", "add big")
check(WeiMath.subtract("1000", "1") == "999", "sub")
check(WeiMath.subtract("1", "5") == "0", "sub neg")
check(WeiMath.decimal("0x6f05b59d3b20000") == "500000000000000000", "hex")
check(WeiMath.decimal(NSNumber(value: 42)) == "42", "num")
check(WeiMath.aeth("1500000000000000000") == 1.5, "aeth")
var cal = Calendar(identifier: .gregorian); cal.timeZone = TimeZone(identifier: "UTC")!
let now = Date(timeIntervalSince1970: 1_790_000_000)
let half = "500000000000000000"
let es = [
  RewardEntry(proven: 1, amountWei: half, height: 10, time: now.addingTimeInterval(-86_400 * 2)),
  RewardEntry(proven: 2, amountWei: half, height: 20, time: cal.startOfDay(for: now).addingTimeInterval(60)),
  RewardEntry(proven: 3, amountWei: half, height: 30, time: now.addingTimeInterval(-600)),
]
let s = EarningsSummary.aggregate(es, now: now, calendar: cal)
check(s.totalWei == "1500000000000000000", "total \(s.totalWei)")
check(s.lastHourWei == half, "hour")
check(s.todayWei == "1000000000000000000", "today \(s.todayWei)")
check(s.count == 3 && s.latestHeight == 30 && s.lastRewardAt == es[2].time, "latest")
let more = EarningsSummary.aggregate(es + [RewardEntry(proven: 4, amountWei: half, height: 31, time: now)], now: now, calendar: cal)
check(more.arrived(since: s) == half, "arrived")
check(s.arrived(since: s) == nil, "no arrival")
check(s.arrived(since: .empty) == "1500000000000000000", "first")
let row: [String: Any] = ["proven": 5, "amount": "0x6f05b59d3b20000", "height": 7, "timestamp_ms": 1_790_000_000_000]
check(RewardEntry(json: row)?.amountWei == half, "json")
check(EarningsSummary.aggregate([], now: now) == .empty, "empty")
print("OK")
