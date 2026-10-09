import Foundation

/// What recovery cannot do (contracts audit 2026-10-05, F-05): with EIP-7702
/// delegation the account's original signer keeps its authority after a
/// guardian recovery or an added owner key. Recovery saves a LOST key; it
/// never revokes a STOLEN one, so the wallet says so wherever recovery or the
/// key's custody is explained, and tells people to move their funds instead.
/// Pure Foundation, so scripts/test-swift-pure.sh can check the wording.
enum KeyExposureNotice {
    /// Next to recovery (devices, words, "recover a lost account").
    case recovery
    /// Next to where the key lives and what protects it.
    case keyCustody

    func text(locale: Locale = .current, bundle: Bundle = .main) -> String {
        switch self {
        case .recovery:
            return String(localized: "Recovery moves funds when a key is lost; it does not lock this account's original key out. If that key may be exposed (someone had this Mac unlocked, or a backup leaked), move all your funds to a new account.", bundle: bundle, locale: locale)
        case .keyCustody:
            return String(localized: "If this account's original key may be exposed, recovery devices and words cannot lock it out. Move all your funds to a new account right away.", bundle: bundle, locale: locale)
        }
    }

    var text: String { text() }
}
