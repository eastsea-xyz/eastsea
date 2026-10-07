import Foundation

/// macOS chooses the app's localization, including its per-application
/// Language & Region setting. The app never writes an AppleLanguages override.
enum AppLanguage {
    static let supported = ["en", "ko", "ja", "zh-Hans", "zh-Hant"]

    static var identifier: String {
        Bundle.main.preferredLocalizations.first.flatMap { supported.contains($0) ? $0 : nil } ?? "en"
    }

    static var locale: Locale { Locale(identifier: identifier) }

    static func bundle(for language: String) -> Bundle {
        Bundle.main.path(forResource: language, ofType: "lproj").flatMap(Bundle.init(path:)) ?? .main
    }

    /// Names stay in their own language in every version of Settings.
    static var nativeName: String {
        ["en": "English", "ko": "한국어", "ja": "日本語", "zh-Hans": "简体中文", "zh-Hant": "繁體中文"][identifier] ?? "English"
    }
}
