import Foundation
#if os(macOS)
import AppKit
#endif

/// The old app next to EastSea (release-070 review, B2). The rename changed
/// the bundle id, so `Aether.app` (com.pipln.aether) is a different app that
/// stays installed beside EastSea, with its own `SMAppService.mainApp` login
/// item — which EastSea cannot unregister (that API only acts on the calling
/// bundle). An Aether 0.6.6 left there opens at login, takes the old data's
/// run.lock (the migration then defers on every launch) and, after a
/// migration, starts an identity-less follower that re-syncs gigabytes and
/// fights over the node's ports.
///
/// The Aether 0.6.7 bridge (apps/bridge) is harmless: it runs no node and
/// unregistered its own login item. Anything older is the problem, and the
/// fix is the user's one click: quit it and move the bundle (not the data)
/// to the Trash, where a login item can no longer start it.
enum LegacyAether {
    static let bundleID = "com.pipln.aether"
    /// The first Aether that runs no node: the bridge.
    static let bridgeVersion = "0.6.7"

    /// Dotted numeric comparison ("0.6.10" > "0.6.9"); missing parts are 0,
    /// non-numeric parts compare as 0.
    static func compare(_ a: String, _ b: String) -> ComparisonResult {
        let x = a.split(separator: ".").map { Int($0) ?? 0 }
        let y = b.split(separator: ".").map { Int($0) ?? 0 }
        for i in 0..<max(x.count, y.count) {
            let l = i < x.count ? x[i] : 0, r = i < y.count ? y[i] : 0
            if l != r { return l < r ? .orderedAscending : .orderedDescending }
        }
        return .orderedSame
    }

    /// Whether an installed or running Aether of this version can still run
    /// a node (and hold the old data's run.lock). An unreadable version is
    /// treated as old: it is safer to ask once too often.
    static func runsNode(version: String?) -> Bool {
        guard let version, !version.isEmpty else { return true }
        return compare(version, bridgeVersion) == .orderedAscending
    }

    /// What the user is asked, in their language.
    static func question(ko: Bool) -> (title: String, body: String, confirm: String, later: String) {
        ko ? ("이전 Aether 앱이 아직 이 Mac에 있습니다",
              "\(Brand.projectKo)가 Aether를 대신합니다. Aether가 남아 있으면 로그인할 때 열려 이전 데이터를 붙잡고 노드를 하나 더 돌립니다. "
                + "Aether를 종료하고 휴지통으로 옮길까요? 데이터는 지우지 않습니다 — 지갑과 노드 데이터는 \(Brand.projectKo)가 옮겨 둡니다.",
              "Aether 종료 후 휴지통으로", "나중에")
           : ("The old Aether app is still on this Mac",
              "\(Brand.project) replaces Aether. While Aether is installed it opens at login, holds on to the old data "
                + "and runs a second node. Quit Aether and move it to the Trash? No data is deleted — "
                + "\(Brand.project) moves your wallet and node data over.",
              "Quit Aether and Move to Trash", "Not Now")
    }

    #if os(macOS)
    struct Found {
        let url: URL
        let version: String?
        let running: [NSRunningApplication]
    }

    /// Every installed or running Aether that can still run a node.
    static func nodeRunningCopies() -> [Found] {
        let running = NSRunningApplication.runningApplications(withBundleIdentifier: bundleID)
        var urls = NSWorkspace.shared.urlsForApplications(withBundleIdentifier: bundleID)
        for app in running { if let u = app.bundleURL, !urls.contains(u) { urls.append(u) } }
        return urls.compactMap { url in
            let version = Bundle(url: url)?.infoDictionary?["CFBundleShortVersionString"] as? String
            guard runsNode(version: version) else { return nil }
            // A copy inside a mounted DMG or the Trash is not what logs in.
            if url.path.hasPrefix("/Volumes/") || url.path.contains("/.Trash/") { return nil }
            return Found(url: url, version: version,
                         running: running.filter { $0.bundleURL?.standardizedFileURL == url.standardizedFileURL })
        }
    }

    /// Ask once per launch; on yes, quit every copy and move its bundle to
    /// the Trash (recoverable), then call `done` (the migration retries).
    static func offerRemoval(done: @escaping () -> Void) {
        let copies = nodeRunningCopies()
        guard !copies.isEmpty else { return }
        let q = question(ko: Locale.preferredLanguages.first?.hasPrefix("ko") ?? false)
        let alert = NSAlert()
        alert.messageText = q.title
        alert.informativeText = q.body + "\n\n" + copies.map { $0.url.path }.joined(separator: "\n")
        alert.addButton(withTitle: q.confirm)
        alert.addButton(withTitle: q.later)
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        for copy in copies { copy.running.forEach { $0.terminate() } }
        // A clean quit lets the old node (its child) exit and release run.lock.
        DispatchQueue.main.asyncAfter(deadline: .now() + 5) {
            NSWorkspace.shared.recycle(copies.map(\.url)) { _, error in
                if let error {
                    let fail = NSAlert()
                    fail.messageText = q.title
                    fail.informativeText = error.localizedDescription
                    fail.runModal()
                }
                done()
            }
        }
    }
    #endif
}
