import Foundation

/// The activity row's sentence about a submitted transaction, resolved through
/// the app's catalog. Backend messages are data for the few numeric estimates;
/// their text never passes through to a screen in another language.
enum TxStatusText {
    static func sentence(state: String, reason: String?, success: Bool?, message: String,
                         locale: Locale = .current, bundle: Bundle = .main) -> String {
        switch state {
        case "included":
            if success == true {
                return String(localized: "Done.", bundle: bundle, locale: locale)
            }
            return String(localized: "It made it into a block, but running it failed.", bundle: bundle, locale: locale)
        case "pending":
            switch reason {
            case "state_price_above_cap"?:
                if let wait = waitEstimate(in: message, locale: locale, bundle: bundle) {
                    return String(localized: "The network is busy: the fee right now is above the most this payment allows. It may go through in about \(wait), when the fee comes down. If it has not come down within 10 minutes, it may not go through; you can send it again at the current fee.", bundle: bundle, locale: locale)
                }
                return String(localized: "The network is busy: the fee right now is above the most this payment allows. It goes through once the fee comes down. If it still waits after 10 minutes it is cancelled, and no money leaves your wallet.", bundle: bundle, locale: locale)
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
                return String(localized: "The network got busy and the fee rose above the most this payment allows, so it did not go through. No money left your wallet. You can send it again at the current fee.", bundle: bundle, locale: locale)
            case "fee_cap_below_base"?:
                return String(localized: "The network fee went up, so it did not go through. No money left your wallet. You can send it again at the current fee.", bundle: bundle, locale: locale)
            case "nonce_gap"?:
                return String(localized: "An earlier payment did not go through, so this one did not either. No money left your wallet. You can send it again.", bundle: bundle, locale: locale)
            case "expired"?:
                return String(localized: "It waited too long and was cancelled. No money left your wallet. You can send it again.", bundle: bundle, locale: locale)
            case "evicted"?:
                return String(localized: "The network's queue was full, so it did not go through. No money left your wallet. You can send it again.", bundle: bundle, locale: locale)
            case "replaced"?:
                return String(localized: "Another payment with the same number went through instead. No money left through this one.", bundle: bundle, locale: locale)
            case "unaffordable"?:
                return String(localized: "The balance was too low, so it did not go through. No money left your wallet.", bundle: bundle, locale: locale)
            default:
                return String(localized: "It did not go through. No money left your wallet.", bundle: bundle, locale: locale)
            }
        case "replaced":
            if let number = capturedNumber(in: message, pattern: #"\(([0-9]+)\)"#) {
                return String(localized: "Another payment with the same number (\(number)) went through instead. No money left through this one.", bundle: bundle, locale: locale)
            }
            return String(localized: "Another payment with the same number went through instead. No money left through this one.", bundle: bundle, locale: locale)
        case "unknown":
            return String(localized: "There is no record of this payment yet. Not processed (not on chain yet).", bundle: bundle, locale: locale)
        default:
            return unknown(locale: locale, bundle: bundle)
        }
    }

    /// The node has no record of the hash.
    static func unknown(locale: Locale = .current, bundle: Bundle = .main) -> String {
        String(localized: "The network has no record of this payment.", bundle: bundle, locale: locale)
    }

    /// The wallet stopped waiting.
    static func timedOut(locale: Locale = .current, bundle: Bundle = .main) -> String {
        String(localized: "It waited a long time and did not go through.", bundle: bundle, locale: locale)
    }

    /// The FFI currently supplies its wait estimate in Korean. Read only its
    /// number and unit, then resolve the whole user sentence from the catalog.
    private static func waitEstimate(in message: String, locale: Locale, bundle: Bundle) -> String? {
        if let amount = capturedNumber(in: message, pattern: "([0-9]+)\u{CD08}") {
            return String(localized: "\(amount) seconds", bundle: bundle, locale: locale)
        }
        if let amount = capturedNumber(in: message, pattern: "([0-9]+)\u{BD84}"), message.contains("10\u{BD84}") {
            // The sentence always ends with its ten-minute limit. A lone
            // limit is not an estimate, so require two number/unit matches.
            let pattern = "[0-9]+[\u{CD08}\u{BD84}]"
            let regex = try? NSRegularExpression(pattern: pattern)
            let range = NSRange(message.startIndex..., in: message)
            guard (regex?.numberOfMatches(in: message, range: range) ?? 0) > 1 else { return nil }
            return String(localized: "\(amount) minutes", bundle: bundle, locale: locale)
        }
        return nil
    }

    private static func capturedNumber(in message: String, pattern: String) -> String? {
        guard let regex = try? NSRegularExpression(pattern: pattern),
              let match = regex.firstMatch(in: message, range: NSRange(message.startIndex..., in: message)),
              let range = Range(match.range(at: 1), in: message) else { return nil }
        return String(message[range])
    }
}
