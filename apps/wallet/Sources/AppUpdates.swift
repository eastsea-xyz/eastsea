#if os(macOS)
import AppKit
import Sparkle
import SwiftUI

/// Link the bundled `aether` and `aether-agent` into ~/.local/bin.
enum CommandLineTools {
    static func install() {
        // Red team #10: a symlink into a bundle that disappears (a DMG, a
        // translocated copy) is a command line that breaks at the next
        // unmount — the move sentence says what to do instead.
        guard InstallLocation.currentIsRunnable else {
            let alert = NSAlert()
            alert.messageText = InstallLocation.moveSentence
            alert.informativeText = String(localized: "\(Brand.name) is running from a temporary place.")
            alert.runModal()
            return
        }
        let helpers = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers")
        let bin = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".local/bin")
        var done: [String] = []
        do {
            try FileManager.default.createDirectory(at: bin, withIntermediateDirectories: true)
            for tool in ["aether", "aether-agent"] {
                let src = helpers.appendingPathComponent(tool)
                guard FileManager.default.isExecutableFile(atPath: src.path) else { continue }
                let dst = bin.appendingPathComponent(tool)
                try? FileManager.default.removeItem(at: dst)
                try FileManager.default.createSymbolicLink(at: dst, withDestinationURL: src)
                done.append(dst.path)
            }
        } catch {
            done.append(String(localized: "Failed: \(error.localizedDescription)"))
        }
        let alert = NSAlert()
        alert.messageText = done.isEmpty ? String(localized: "No command-line tools in this build") : String(localized: "Command-line tools installed")
        alert.informativeText = done.joined(separator: "\n") + "\n\n" + String(localized: "Make sure ~/.local/bin is on your PATH.")
        alert.runModal()
    }
}
/// The app's update state for the UI (Sparkle does the checking and installing).
@MainActor
final class Updates: ObservableObject {
    private let controller: SPUStandardUpdaterController
    @Published var pendingRelease: PendingRelease?
    @Published var approvalIssue: String?
    /// The update tracker's one honest sentence (red team #11): a failure and
    /// what happens next, or the post-update health check in progress.
    @Published var installNotice: String?

    init(_ controller: SPUStandardUpdaterController) {
        self.controller = controller
    }

    var lastCheck: Date? { controller.updater.lastUpdateCheckDate }
    var version: String {
        let info = Bundle.main.infoDictionary
        return "\(info?["CFBundleShortVersionString"] as? String ?? "?") (\(info?["CFBundleVersion"] as? String ?? "?"))"
    }

    /// Check now, showing Sparkle's window (up to date, or the new version).
    func check() {
        controller.checkForUpdates(nil)
        objectWillChange.send()
    }
}
#endif
