// "Where does my balance come from?": decoding the Rust core's exact-wei
// itemization, the card's lines and its honest remainder, timestamps that
// never render 1970 ("56y ago"), and the day-by-day reward grouping. Pure
// Foundation — no node, no window, no clock.
import Foundation

func expect(_ name: String, _ cond: Bool, _ detail: String = "") {
    if cond { print("ok   \(name)") } else { print("FAIL \(name) \(detail)"); exit(1) }
}

// MARK: - Timestamps

// A missing time renders as nothing (nil), never as the epoch; a seconds
// value (legacy 7780 rows) is scaled back up to milliseconds.
expect("zero timestamp has no date", Timestamp.date(fromMs: 0) == nil)
if let d = Timestamp.date(fromMs: 1_735_689_600_000) {
    expect("milliseconds pass through", d.timeIntervalSince1970 == 1_735_689_600)
} else { expect("milliseconds pass through", false) }
if let d = Timestamp.date(fromMs: 1_735_689_600) {
    expect("seconds are scaled to ms", d.timeIntervalSince1970 == 1_735_689_600)
} else { expect("seconds are scaled to ms", false) }
expect("normalize keeps ms", Timestamp.normalizeMs(1_735_689_600_000) == 1_735_689_600_000)
expect("normalize lifts seconds", Timestamp.normalizeMs(1_735_689_600) == 1_735_689_600_000)
expect("normalize leaves zero alone", Timestamp.normalizeMs(0) == 0)

// MARK: - BalanceBreakdown.decode

let breakdownJSON = """
{"proof_rewards_wei":"2583029731800000000000","node_rewards_wei":"0","faucet_wei":"1000000000000000000",
"received_wei":"500000000000000000","unwrapped_wei":"0","sent_wei":"154750000000000000","fees_wei":"1000000000000000",
"total_in_wei":"3083529731800000000000","total_out_wei":"155750000000000000",
"balance_wei":"3083373981800000000000","difference_wei":"0","itemizes_completely":true,"rows":2583}
"""
let breakdown = try? BalanceBreakdown.decode(breakdownJSON)
expect("decodes snake_case json", breakdown != nil)
expect("rows counted", breakdown?.rows == 2583)
expect("complete when difference is zero", breakdown?.itemizesCompletely == true)

// MARK: - the card's lines

if let b = breakdown {
    let lines = BalanceBreakdownText.lines(b)
    expect("zero sources are not listed", !lines.contains { $0.name == "Node rewards" || $0.name == "Unwrapped" })
    expect("inflow first, deductions last", lines.first?.name == "Proof rewards" && lines.last?.name == "Fees")
    expect("line count", lines.count == 5, "\(lines.map(\.name))")
    expect("sent marked negative", lines.first { $0.name == "Sent" }?.negative == true)
    expect("proof marked positive", lines.first { $0.name == "Proof rewards" }?.negative == false)
    expect("exact wei on the line", lines.first { $0.name == "Proof rewards" }?.wei == "2583029731800000000000")
    expect("nothing missing when complete", BalanceBreakdownText.notYetItemized(b) == nil)
}

// An honest remainder: the gap is named, in coins, with what to do about it.
let gapJSON = """
{"proof_rewards_wei":"1000000000000000000","node_rewards_wei":"0","faucet_wei":"0","received_wei":"0",
"unwrapped_wei":"0","sent_wei":"0","fees_wei":"0","total_in_wei":"1000000000000000000","total_out_wei":"0",
"balance_wei":"3000000000000000000","difference_wei":"2000000000000000000","itemizes_completely":false,"rows":1}
"""
if let gap = try? BalanceBreakdown.decode(gapJSON) {
    let note = BalanceBreakdownText.notYetItemized(gap)
    expect("remainder is named", note == "2 DBLN not yet itemized — load older activity to account for it", note ?? "nil")
    expect("remainder keeps its units", BalanceBreakdownText.lines(gap).count == 1)
}
// A negative difference (rows claiming more than the balance holds) is said
// as it is, never flipped into a positive remainder.
let overJSON = """
{"proof_rewards_wei":"1000000000000000000","node_rewards_wei":"0","faucet_wei":"0","received_wei":"0",
"unwrapped_wei":"0","sent_wei":"0","fees_wei":"0","total_in_wei":"1000000000000000000","total_out_wei":"0",
"balance_wei":"100000000000000000","difference_wei":"-900000000000000000","itemizes_completely":false,"rows":1}
"""
if let over = try? BalanceBreakdown.decode(overJSON) {
    let note = BalanceBreakdownText.notYetItemized(over)
    expect("over-count named as such", note == "0.9 DBLN more itemized than the balance shows — try again in a moment", note ?? "nil")
}

// MARK: - reward rows and day grouping

func row(_ fields: String) -> ChainHistoryEntry {
    let json = """
    {"entries":[{"address":"0xaa","tx_index":0,"direction":"in","success":true,"tokens":[],\(fields)}],"history_start":1,"indexed_height":200}
    """
    return try! ChainHistoryPage.decode(json).entries[0]
}
let day1 = 1_735_689_600_000        // 2025-01-01T00:00:00Z
let proofA = row("\"height\":100,\"tx_hash\":\"0xa\",\"timestamp_ms\":\(day1),\"kind\":\"proof_reward\",\"value_wei\":\"500000000000000000\"")
let proofB = row("\"height\":101,\"tx_hash\":\"0xb\",\"timestamp_ms\":\(day1 + 3_600_000),\"kind\":\"proof_reward\",\"value_wei\":\"250000000000000000\"")
let nodeA  = row("\"height\":102,\"tx_hash\":\"0xc\",\"timestamp_ms\":\(day1 + 7_200_000),\"kind\":\"node_reward\",\"value_wei\":\"140000000000000000\"")
let day2   = day1 + 86_400_000
let proofC = row("\"height\":103,\"tx_hash\":\"0xd\",\"timestamp_ms\":\(day2),\"kind\":\"proof_reward\",\"value_wei\":\"1000000000000000000\"")
// No time at all: kept, grouped apart, and never "the newest".
let noTime = row("\"height\":104,\"tx_hash\":\"0xe\",\"timestamp_ms\":0,\"kind\":\"proof_reward\",\"value_wei\":\"700000000000000000\"")

var utc = Calendar(identifier: .gregorian)
utc.timeZone = TimeZone(identifier: "UTC")!
let days = RewardDays.group([noTime, proofC, nodeA, proofB, proofA], calendar: utc)
expect("one group per day", days.count == 3, "\(days.count)")
expect("non-reward rows dropped", RewardDays.group([row("\"height\":110,\"tx_hash\":\"0xz\",\"timestamp_ms\":\(day1),\"kind\":\"native_transfer\",\"value_wei\":\"1\"")], calendar: utc).isEmpty)
if days.count == 3 {
    expect("newest day first", days[0].rows.allSatisfy { $0.txHash == "0xd" })
    expect("unknown time stays last", days[2].day == nil && days[2].rows.allSatisfy { $0.txHash == "0xe" })
    let first = days[1]
    expect("day groups both kinds", first.count == 3 && first.rows.count == 3)
    expect("proof and node summed apart", first.proofWei == "750000000000000000" && first.nodeWei == "140000000000000000")
    expect("day total is exact wei", first.totalWei == "890000000000000000")
    expect("day date is start of day", first.day == utc.startOfDay(for: Date(timeIntervalSince1970: TimeInterval(day1) / 1000)))
}
// A seconds timestamp (legacy 7780 rows) lands on its own day, not 1970.
let legacy = row("\"height\":105,\"tx_hash\":\"0xf\",\"timestamp_ms\":\(day1 / 1000),\"kind\":\"proof_reward\",\"value_wei\":\"1\"")
let legacyDays = RewardDays.group([legacy], calendar: utc)
expect("legacy seconds grouped on their day", legacyDays.count == 1 && legacyDays[0].day == utc.startOfDay(for: Date(timeIntervalSince1970: TimeInterval(day1) / 1000)))

// MARK: - the activity CSV

let csv = EarningsCSV.activityDocument([proofB, proofA, noTime])
let csvLines = csv.split(separator: "\n").map(String.init)
expect("activity header columns", csvLines[3] == "block_height,time_utc,kind,direction,from,to,value_dbln,value_base_units,fee_dbln,tx")
expect("activity rows oldest first", csvLines[4].hasPrefix("100,2025-01-01T00:00:00Z,proof_reward"))
expect("missing time exports empty", csvLines[6].hasPrefix("104,,proof_reward"))
expect("value exact", csvLines[4].contains(",0.500000000000000000,500000000000000000,"))
expect("fee empty when the column is absent", csvLines[4].hasSuffix(",0xa"))
expect("trailing newline", csv.hasSuffix("\n"))
// A fee the node recorded lands on the sender's row, exact.
let feeRow = row("\"height\":106,\"tx_hash\":\"0xg\",\"timestamp_ms\":\(day1),\"direction\":\"out\",\"kind\":\"native_transfer\",\"value_wei\":\"1000000000000000\",\"fee_wei\":\"1500000000000000\",\"to\":\"0xbb\",\"from\":\"0xaa\"")
expect("fee exported exact", EarningsCSV.activityDocument([feeRow]).split(separator: "\n").map(String.init)[4].contains(",0.001500000000000000,0xg"))
expect("empty activity keeps header", EarningsCSV.activityDocument([]).split(separator: "\n").count == 4)

// MARK: - reward entries and "last one …"

// A seconds timestamp is normalized on the way in; a missing one never
// becomes "last one 56y ago".
let secEntry = RewardEntry(json: ["kind": "proof", "proven": 1, "amount": "5", "height": 2, "timestamp_ms": 1_735_689_600])
expect("reward entry normalizes seconds", secEntry?.timestampMs == 1_735_689_600_000)
let noTimeEntry = RewardEntry(json: ["kind": "proof", "proven": 1, "amount": "5", "height": 2])!
let timedEntry = RewardEntry(json: ["kind": "proof", "proven": 1, "amount": "7", "height": 3, "timestamp_ms": day1])!
let summary = EarningsSummary.aggregate([noTimeEntry, timedEntry], now: Date(timeIntervalSince1970: 2_000_000_000))
expect("summary counts every row", summary.count == 2)
expect("last reward keeps a real time only", summary.lastRewardAt != nil && summary.lastRewardAt == timedEntry.time)
expect("newest row without a time has none", EarningsSummary.aggregate([noTimeEntry], now: Date()).lastRewardAt == nil)
expect("amount of the newest row", summary.lastRewardWei == "7")

print("balance-sources: all ok")
