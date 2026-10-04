import Foundation

/// Why the transaction history could not be read from the node, and what the
/// history page should say about it. A failed `aether_accountHistory` read
/// used to be swallowed (`catch {}`), leaving a stale or empty list on screen
/// as if it were the truth; the classifier turns the error the FFI throws
/// into one plain sentence instead. Pure Foundation, so the pure-test harness
/// covers the decision logic.
enum HistoryFailure: Equatable {
    /// The connected node predates the history RPC ("method not found"): it
    /// will never answer this build, so the list is knowingly incomplete.
    case unsupportedNode
    /// The node could not be reached at all (connect refused, timeout): the
    /// list shown is the last one that was read.
    case unreachable
    /// The node answered, but not with a page (pruned history, a limit…).
    case refused

    /// Classify the message of an error thrown by the account-history read —
    /// either the raw `String(describing:)` of the FFI error (which keeps the
    /// case, `Rejected("…")`) or its unwrapped message; both classify the same.
    static func classify(message: String) -> HistoryFailure {
        let m = message.lowercased()
        if m.contains("method not found") { return .unsupportedNode }
        if m.contains("local node:") || m.contains("connect") || m.contains("timed out")
            || m.contains("unreachable") || m.contains("offline") {
            return .unreachable
        }
        return .refused
    }

    /// The line the history page shows above the list.
    var notice: String {
        switch self {
        case .unsupportedNode:
            "This node cannot show your full history yet. Update the node or switch to another node."
        case .unreachable:
            "The history could not be read from the node just now, so this list may be out of date or incomplete."
        case .refused:
            "This node refused the history request, so this list may be incomplete."
        }
    }
}
