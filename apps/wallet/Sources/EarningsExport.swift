import Foundation

/// "Export earnings (CSV)": what this app saw the node pay this Mac, written
/// as a plain CSV file. Pure Foundation, so the escaping, the exact 18-decimal
/// amounts and the header wording are checked by the pure-test harness
/// (no node, no window, no save panel).
///
/// A reward row names no transaction — the block that paid it stands in for
/// one (docs/research/node-reward-tax-2026.md) — so the reference is the pair
/// of blocks: `block_height` paid, `proven_block` earned. Timestamps are the
/// paying block's own time as the node reported it; when the node sent none
/// the column stays empty rather than being guessed from the height (block
/// times drift, and a guess would be a wrong record).
enum EarningsCSV {
    /// One document: the comment header, the column names, then one row per
    /// reward, oldest first (spreadsheet order).
    static func document(_ entries: [RewardEntry]) -> String {
        var rows = [
            "# EastSea earnings export — a record of what this app saw this Mac earn, not tax advice.",
            "# Rewards are paid by blocks, not transactions: block_height is the block that paid; proven_block is the block whose work earned it (kind \"node\": an epoch's distribution, \"proof\": a proven block's reward).",
            "# time_utc is the paying block's time as the node reported it (ISO 8601, UTC); the column is empty when the node reported none.",
            "block_height,time_utc,kind,amount_dbln,amount_base_units,proven_block",
        ]
        for e in entries.sorted(by: { ($0.height, $0.proven) < ($1.height, $1.proven) }) {
            rows.append([String(e.height), iso(e.timestampMs), e.kind, dbln(e.amountWei), e.amountWei, String(e.proven)]
                .map(field).joined(separator: ","))
        }
        return rows.joined(separator: "\n") + "\n"
    }

    /// One activity export: every history row this wallet loaded for its own
    /// address — rewards, transfers in and out, fees — oldest first
    /// (spreadsheet order). Same rules as the earnings export: exact amounts,
    /// no guessed times (a seconds timestamp from the legacy chain is scaled
    /// back to milliseconds first; a missing one stays empty).
    static func activityDocument(_ entries: [ChainHistoryEntry]) -> String {
        var rows = [
            "# EastSea activity export — every history row this app loaded for this account, not tax advice.",
            "# time_utc is the block's time as the node reported it (ISO 8601, UTC); the column is empty when the node reported none.",
            "# fee_dbln is what the transaction cost its sender (exec + prove + state fee): only the sender's row carries one, and rows written before the column existed export it empty.",
            "block_height,time_utc,kind,direction,from,to,value_dbln,value_base_units,fee_dbln,tx",
        ]
        for e in entries.sorted(by: { ($0.height, $0.txIndex) < ($1.height, $1.txIndex) }) {
            rows.append([String(e.height), iso(Timestamp.normalizeMs(e.timestampMs)), e.kind, e.direction,
                         e.from ?? "", e.to ?? "", dbln(e.valueWei), e.valueWei, e.feeWei.map(dbln) ?? "", e.txHash]
                .map(field).joined(separator: ","))
        }
        return rows.joined(separator: "\n") + "\n"
    }

    /// One CSV field, quoted only when it contains a character that would
    /// break the row (RFC 4180); a quote inside is doubled.
    static func field(_ s: String) -> String {
        if s.contains(where: { $0 == "," || $0 == "\"" || $0 == "\n" || $0 == "\r" }) {
            return "\"" + s.replacingOccurrences(of: "\"", with: "\"\"") + "\""
        }
        return s
    }

    /// Wei as DBLN with all 18 decimals — exact, never rounded.
    static func dbln(_ wei: String) -> String {
        let padded = String(repeating: "0", count: max(0, 19 - wei.count)) + wei
        let whole = padded.dropLast(18).drop(while: { $0 == "0" })
        return "\(whole.isEmpty ? "0" : String(whole)).\(padded.suffix(18))"
    }

    /// A block timestamp in milliseconds as ISO 8601 UTC (seconds); empty
    /// when unknown (0).
    static func iso(_ ms: UInt64) -> String {
        guard ms > 0 else { return "" }
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f.string(from: Date(timeIntervalSince1970: Double(ms) / 1000))
    }
}
