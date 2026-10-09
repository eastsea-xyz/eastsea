import Foundation

/// Activity copy comes from the catalog. A local queue result must leave the
/// chain outcome unresolved; backend prose never determines the UI language.
enum TxStatusText {
    static func sentence(state: String, reason: String?, success: Bool?, message: String,
                         locale: Locale = .current, bundle: Bundle = .main) -> String {
        switch state {
        case "included":
            return success == true
                ? String(localized: "Done.", bundle: bundle, locale: locale)
                : String(localized: "It made it into a block, but running it failed.", bundle: bundle, locale: locale)
        case "replaced":
            return String(localized: "Another transaction used this account's transaction number on chain. This transaction can no longer be processed.", bundle: bundle, locale: locale)
        case "pending":
            switch reason {
            case "state_price_above_cap"?:
                return String(localized: "The network is busy: the fee right now is above the most this payment allows. Waiting for the fee to come down. If it is still waiting after 10 minutes, you may need to send it again at the current fee.", bundle: bundle, locale: locale)
            case "nonce_gap"?:
                return String(localized: "Waiting for an earlier payment to go through first.", bundle: bundle, locale: locale)
            case "fee_cap_below_base"?:
                return String(localized: "The network fee went up for a moment. Waiting for it to come down.", bundle: bundle, locale: locale)
            default:
                return String(localized: "Processing.", bundle: bundle, locale: locale)
            }
        case "dropped":
            switch reason {
            case "state_price_above_cap"?:
                return String(localized: "The network got busy and the fee rose above the most this payment allows. This payment is not recorded on chain yet. You can send it again at the current fee.", bundle: bundle, locale: locale)
            case "fee_cap_below_base"?:
                return String(localized: "The network fee went up. This payment is not recorded on chain yet. You can send it again at the current fee.", bundle: bundle, locale: locale)
            case "nonce_gap"?:
                return String(localized: "It was waiting for an earlier payment. This payment is not recorded on chain yet. You can send it again.", bundle: bundle, locale: locale)
            case "expired"?:
                return String(localized: "It waited for a long time. This payment is not recorded on chain yet. You can send it again.", bundle: bundle, locale: locale)
            case "evicted"?:
                return String(localized: "The network's queue was full. This payment is not recorded on chain yet. You can send it again.", bundle: bundle, locale: locale)
            case "replaced"?:
                return String(localized: "Another payment with the same number may have taken its place. This payment is not recorded on chain yet.", bundle: bundle, locale: locale)
            case "unaffordable"?:
                return String(localized: "The balance was too low when this payment was checked. This payment is not recorded on chain yet.", bundle: bundle, locale: locale)
            default:
                return String(localized: "This payment is not recorded on chain yet.", bundle: bundle, locale: locale)
            }
        case "unknown":
            return String(localized: "There is no record of this payment yet. Not processed (not on chain yet).", bundle: bundle, locale: locale)
        default:
            return unknown(locale: locale, bundle: bundle)
        }
    }

    static func unknown(locale: Locale = .current, bundle: Bundle = .main) -> String {
        String(localized: "The network has no record of this payment.", bundle: bundle, locale: locale)
    }

    static func timedOut(locale: Locale = .current, bundle: Bundle = .main) -> String {
        String(localized: "It waited a long time. The outcome is still unconfirmed.", bundle: bundle, locale: locale)
    }
}
