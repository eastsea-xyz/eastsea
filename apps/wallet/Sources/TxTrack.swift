import Foundation

/// What an activity row — and a payment link's callback — makes of one status
/// answer for a submitted transaction (contracts-live bug #5; B5 review round
/// 2, finding 5). A drop is one node's observation: another node may still
/// hold and include the same transaction. So a drop, or a node that has never
/// heard of the hash, reads "not on chain yet" and the row keeps its context;
/// only chain facts settle it — a receipt, or the nonce used by another
/// transaction (`replaced`). Pure Foundation (Tests/tx-track).
enum TxTrack {
    enum Row: String, Codable { case pending, done, failed, notIncluded }

    /// The row for the FFI's `TxStatus.state` (`nil`: no answer at all).
    /// `unknownFor` is how long the network has had no record of it; it is
    /// kept for the log, never a reason to fail the row.
    static func row(state: String?, success: Bool?, unknownFor: TimeInterval) -> Row {
        switch state {
        case "included": return success == true ? .done : .failed
        case "replaced": return .failed
        case "dropped": return .notIncluded
        default: return .pending
        }
    }

    /// Only chain facts are final; a not-included row is reconciled later.
    static func isFinal(_ row: Row) -> Bool { row == .done || row == .failed }

    /// The `status` a payment link's callback hears.
    static func callbackStatus(_ row: Row) -> String {
        switch row {
        case .done: return "success"
        case .failed: return "failed"
        case .notIncluded: return "not_included"
        case .pending: return "pending"
        }
    }

    /// What a not-included row and callback say: not a permanent failure.
    static func notIncludedNote(locale: Locale = .current, bundle: Bundle = .main) -> String {
        String(localized: "Not processed (not on chain yet)", bundle: bundle, locale: locale)
    }

    static var notIncludedNote: String { notIncludedNote() }
}
