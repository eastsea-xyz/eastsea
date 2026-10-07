import Foundation

/// The activity row's one sentence about a submitted transaction (contracts-live
/// bug #5), in the app's language. The core (crates/ffi tx_status.rs) answers a
/// plain Korean `message` and a technical English `detail` for agents and logs;
/// an English screen must not show the Korean one, and a consumer must not see
/// the technical one. So the Korean sentence passes through, and English gets
/// the same answer here, by the node's state and reason.
enum TxStatusText {
    static func sentence(state: String, reason: String?, success: Bool?, message: String,
                         ko: Bool = Bundle.main.preferredLocalizations.first?.hasPrefix("ko") ?? false) -> String {
        if ko { return message }
        let notRecorded = "This payment is not recorded on chain yet."
        switch state {
        case "included":
            return success == true ? "Done." : "It made it into a block, but running it failed."
        case "pending":
            switch reason {
            case "state_price_above_cap"?:
                return "The network is busy: the fee right now is above the most this payment allows. Waiting for the fee to come down. If it is still waiting after 10 minutes, you may need to send it again at the current fee."
            case "nonce_gap"?: return "Waiting for an earlier payment to go through first."
            case "fee_cap_below_base"?: return "The network fee went up for a moment. Waiting for it to come down."
            default: return "Processing."
            }
        case "dropped":
            switch reason {
            case "state_price_above_cap"?:
                return "The network got busy and the fee rose above the most this payment allows. \(notRecorded) You can send it again at the current fee."
            case "fee_cap_below_base"?:
                return "The network fee went up. \(notRecorded) You can send it again at the current fee."
            case "nonce_gap"?:
                return "It was waiting for an earlier payment. \(notRecorded) You can send it again."
            case "expired"?:
                return "It waited for a long time. \(notRecorded) You can send it again."
            case "evicted"?:
                return "The network's queue was full. \(notRecorded) You can send it again."
            case "replaced"?:
                return "Another payment with the same number may have taken its place. \(notRecorded)"
            case "unaffordable"?:
                return "The balance was too low when this payment was checked. \(notRecorded)"
            default:
                return notRecorded
            }
        default:
            return unknown(ko: ko)
        }
    }

    /// The node has no record of the hash.
    static func unknown(ko: Bool = Bundle.main.preferredLocalizations.first?.hasPrefix("ko") ?? false) -> String {
        ko ? "네트워크에서 이 거래를 찾지 못했어요." : "The network has no record of this payment."
    }

    /// The wallet stopped waiting.
    static func timedOut(ko: Bool = Bundle.main.preferredLocalizations.first?.hasPrefix("ko") ?? false) -> String {
        ko ? "오래 기다렸지만 처리 결과를 아직 확인하지 못했어요." : "It waited a long time. The outcome is still unconfirmed."
    }
}
