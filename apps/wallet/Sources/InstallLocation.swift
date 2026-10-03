import Foundation
#if os(macOS)
import AppKit
#endif

/// Whether the app is running from a place it can live in (red team #10):
/// started straight from a mounted DMG, from a quarantined Downloads copy
/// (App Translocation), or from any read-only volume. From such a place
/// Sparkle can never install updates, and the node's data and login-item
/// setup point into a bundle that disappears at the next unmount or
/// quarantine cleanup. Pure decision on the bundle path and the volume's
/// read-only flag (Tests/install-location) — nothing here talks to the
/// system, so the rule is a plain unit test.
struct InstallLocation {
    /// The rule, in one place:
    /// 1. an App Translocation path (a quarantined app runs from a randomized
    ///    read-only shadow under /private/var/folders/…) is never runnable;
    /// 2. a read-only volume (a mounted DMG, a locked disk) is never runnable;
    /// 3. a developer build is runnable and must not be nagged: Xcode's
    ///    per-user DerivedData and a repository's `build/` output are the
    ///    places we build and run the app from ourselves;
    /// 4. /Applications and ~/Applications (subfolders included) are runnable;
    /// 5. anything else (Downloads, Desktop, Documents, /Volumes/…) is not.
    static func isRunnableLocation(bundlePath: String, homeApplicationsPath: String, volumeIsReadOnly: Bool) -> Bool {
        let path = (bundlePath as NSString).standardizingPath
        if path.contains("/AppTranslocation/") { return false }
        if volumeIsReadOnly { return false }
        if path.contains("/DerivedData/") || path.contains("/build/") { return true }
        if isContained("/Applications", path) { return true }
        if isContained(homeApplicationsPath, path) { return true }
        return false
    }

    /// `path` is `container` itself or inside it, with a real path-separator
    /// boundary: "/Applications-Backup" is not "/Applications".
    private static func isContained(_ container: String, _ path: String) -> Bool {
        let c = (container as NSString).standardizingPath
        return !c.isEmpty && (path == c || path.hasPrefix(c + "/"))
    }

    /// One plain sentence (layer 4): what to do, in the app's language.
    static var moveSentence: String {
        let ko = Locale.preferredLanguages.first?.hasPrefix("ko") ?? false
        return ko ? "\(Brand.projectKo)를 응용 프로그램 폴더로 옮긴 뒤 실행해 주세요."
            : "Move \(Brand.project) to your Applications folder to run it."
    }

    #if os(macOS)
    /// The verdict for this running app. An unreadable volume flag fails open
    /// (treated as writable): the path rules above catch the real wrong
    /// places, and an unknown flag must not nag a correctly installed app.
    static var currentIsRunnable: Bool {
        isRunnableLocation(bundlePath: Bundle.main.bundlePath,
                           homeApplicationsPath: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Applications").path,
                           volumeIsReadOnly: volumeIsReadOnly)
    }

    /// Whether this bundle's volume reports itself read-only.
    static var volumeIsReadOnly: Bool {
        (try? URL(fileURLWithPath: Bundle.main.bundlePath)
            .resourceValues(forKeys: [.volumeIsReadOnlyKey]))?.volumeIsReadOnly ?? false
    }

    /// Show the running app in Finder, where the user can move it from.
    static func revealInFinder() {
        let bundle = Bundle.main.bundleURL
        NSWorkspace.shared.selectFile(bundle.path, inFileViewerRootedAtPath: bundle.deletingLastPathComponent().path)
    }
    #endif
}
