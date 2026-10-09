import Foundation

/// Pure wallet tests use the real catalog exported to tmp by wallet-l10n.py.
/// Selecting a language bundle makes the test independent of the Mac's language.
func walletTestBundle(_ language: String) -> Bundle {
    guard let root = ProcessInfo.processInfo.environment["WALLET_TEST_BUNDLE"],
          let bundle = Bundle(url: URL(fileURLWithPath: root).appendingPathComponent("\(language).lproj")) else {
        fatalError("WALLET_TEST_BUNDLE must point to the generated catalog fixture")
    }
    return bundle
}

func walletTestLocale(_ language: String) -> Locale {
    Locale(identifier: language)
}

func walletExpectedTranslation(_ key: String, language: String) -> String {
    walletTestBundle(language).localizedString(forKey: key, value: nil, table: "Localizable")
}
