// The earnings CSV export: escaping, exact 18-decimal amounts, timestamps
// and the header that keeps the file honest. Pure Foundation — no node, no
// window, no save panel.
import Foundation

func expect(_ name: String, _ cond: Bool, _ detail: String = "") {
    if cond { print("ok   \(name)") } else { print("FAIL \(name) \(detail)"); exit(1) }
}

// RFC 4180 escaping: only the characters that break a row get quoted.
expect("plain field unchanged", EarningsCSV.field("184210") == "184210")
expect("comma quoted", EarningsCSV.field("a,b") == "\"a,b\"")
expect("quote doubled", EarningsCSV.field("say \"hi\"") == "\"say \"\"hi\"\"\"")
expect("newline quoted", EarningsCSV.field("a\nb").hasPrefix("\"a\nb\""))
expect("unicode letter passes through", EarningsCSV.field("독도") == "독도")

// Exact DBLN, all 18 decimals, no rounding anywhere.
expect("sub-unit amount", EarningsCSV.dbln("140000000000000000") == "0.140000000000000000")
expect("zero", EarningsCSV.dbln("0") == "0.000000000000000000")
expect("whole with fraction", EarningsCSV.dbln("1000000000000000000001") == "1000.000000000000000001")
expect("one wei", EarningsCSV.dbln("1") == "0.000000000000000001")

// Timestamps: ISO 8601 UTC; unknown stays empty rather than guessed.
expect("unknown timestamp is empty", EarningsCSV.iso(0) == "")
expect("known instant", EarningsCSV.iso(1_735_689_600_000) == "2025-01-01T00:00:00Z")

// A document: comment header, fixed columns, rows oldest first.
let rows: [[String: Any]] = [
    ["kind": "proof", "proven": 184_188, "amount": "500000000000000000", "height": 184_190, "timestamp_ms": 1_735_689_612_000],
    ["kind": "node", "proven": 184_210, "amount": "140000000000000000", "height": 184_210, "timestamp_ms": 1_735_689_600_000],
]
let entries = rows.compactMap { RewardEntry(json: $0) }
let csv = EarningsCSV.document(entries)
let lines = csv.split(separator: "\n").map(String.init)
expect("three comment lines", lines[0].hasPrefix("#") && lines[1].hasPrefix("#") && lines[2].hasPrefix("#"))
expect("not-tax-advice in the header", (0..<3).contains { lines[$0].contains("not tax advice") })
expect("column header", lines[3] == "block_height,time_utc,kind,amount_dbln,amount_base_units,proven_block")
expect("oldest row first", lines[4] == "184190,2025-01-01T00:00:12Z,proof,0.500000000000000000,500000000000000000,184188")
expect("newest row second", lines[5] == "184210,2025-01-01T00:00:00Z,node,0.140000000000000000,140000000000000000,184210")
expect("trailing newline", csv.hasSuffix("\n"))

// The node's kind label is kept; without one it is derived (a distribution
// row pays at its own height; a proof reward never does).
expect("kind read from the row", entries.first?.kind == "proof" && entries.last?.kind == "node")
expect("kind derived node", RewardEntry(json: ["proven": 200, "amount": "1", "height": 200, "timestamp_ms": 5])?.kind == "node")
expect("kind derived proof", RewardEntry(json: ["proven": 190, "amount": "1", "height": 200])?.kind == "proof")

// A row without a reported timestamp leaves the column empty — never a
// height→time guess (block times drift; a guess would be a wrong record).
let noTime = RewardEntry(json: ["kind": "proof", "proven": 1, "amount": "2", "height": 3])!
expect("no timestamp → empty column", EarningsCSV.document([noTime]).split(separator: "\n").map(String.init)[4] == "3,,proof,0.000000000000000002,2,1")

// Empty input still writes the header (a valid, honest file).
expect("empty export keeps header", EarningsCSV.document([]).split(separator: "\n").count == 4)

print("earnings-export: all ok")
