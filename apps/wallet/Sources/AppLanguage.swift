import Foundation

/// The one rule for which language the app speaks: the localization the app
/// bundle itself is shown in (Korean on a Mac that prefers Korean, English
/// otherwise). SwiftUI's `Text`, `String(localized:)` and every hand-written
/// ko/en pair follow it, so a screen never mixes the two. Never decide by
/// `Locale.preferredLanguages`: that list can say Korean while the bundle
/// falls back to English.
///
/// Pure files compiled alone by scripts/test-swift-pure.sh spell the same
/// expression inline (`Bundle.main.preferredLocalizations.first?.hasPrefix("ko")`).
enum AppLanguage {
    static var korean: Bool { Bundle.main.preferredLocalizations.first?.hasPrefix("ko") ?? false }
}
