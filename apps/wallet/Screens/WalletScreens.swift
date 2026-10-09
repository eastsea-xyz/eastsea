#if WALLET_SCREENS
import AppKit
import Darwin
import Sparkle
import SwiftUI
import Vision

/// The product QA renderer (scripts/wallet-screens.sh): every screen and sheet
/// of the Mac wallet drawn to PNG with the design-preview sample data, in the
/// language the process was launched in (`-AppleLanguages (ko)`), light and dark.
///
/// It is its own app (com.pipln.eastsea.screens), built with WALLET_SCREENS:
/// no node starts, no data folder is read or moved, the keychain is never
/// opened (DataMigration.ensure, NodeController.start and EnclaveAccount are
/// compiled out or refuse), and its windows sit off screen.
@main
enum WalletScreens {
    static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        let args = CommandLine.arguments
        let out = args.firstIndex(of: "-out").map { args[$0 + 1] } ?? "tmp/screens"
        let only = args.firstIndex(of: "-only").map { args[$0 + 1] }
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                let r = Renderer(out: URL(fileURLWithPath: out), only: only)
                r.renderAll()
                print("wrote \(r.written) screens to \(out)")
                exit(r.failed == 0 ? 0 : 1)
            }
        }
        app.run()
    }
}

/// The objects every screen reads, built fresh per screen so one sample
/// never leaks into the next.
@MainActor
struct Stage {
    let accountStore: AccountStore
    let model: WalletModel
    let node = NodeController()
    let earnings = Earnings()
    let health = HealthMonitor()
    let unattended = UnattendedDaemon()
    let updates = Updates(SPUStandardUpdaterController(startingUpdater: false, updaterDelegate: nil, userDriverDelegate: nil))
    let browser = BrowserController()

    init() {
        guard let root = ProcessInfo.processInfo.environment["WALLET_SCREEN_FIXTURE_ROOT"] else {
            fatalError("Run the renderer through scripts/wallet-screens.sh to isolate its fixtures.")
        }
        let directory = URL(fileURLWithPath: root).appendingPathComponent("accounts-\(UUID().uuidString)", isDirectory: true)
        do {
            accountStore = try DesignPreview.makeAccountStore(in: directory)
            model = WalletModel(accountStore: accountStore)
        } catch {
            fatalError("Could not prepare isolated screen accounts: \(error)")
        }
        accountStore.readBalances = DesignPreview.balances(for:)
        model.start()          // DesignPreview: loadPreview, nothing on the network
        node.loadPreview()
        earnings.attach(node, operatorAddress: { "" })
        browser.attach(model: model)
    }

    func wrap<V: View>(_ v: V) -> some View {
        v.environmentObject(model)
            .environmentObject(accountStore)
            .environmentObject(node)
            .environmentObject(earnings)
            .environmentObject(health)
            .environmentObject(unattended)
            .environmentObject(updates)
            .environmentObject(browser)
            .tint(.aether)
    }
}

@MainActor
final class Renderer {
    let out: URL
    let only: String?
    private(set) var written = 0
    private(set) var failed = 0
    private let lang: String

    init(out: URL, only: String?) {
        self.out = out
        self.only = only
        lang = AppLanguage.identifier
        try? FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
    }

    /// One sample setting for the next stage (UserDefaults of this renderer's
    /// own domain: the app's are never read).
    private func set(_ values: [String: Any?]) {
        let d = UserDefaults.standard
        for key in ["designPreview", "rewardStatus", "historyNotice", "previewIncomingRecovery", "previewWrongLocation",
                    "previewPage", "previewSheet", "previewSidebar", "previewExplorer", "previewAccounts", "previewSelectedAccount",
                    "developerMode", "proveBlocks", "nodeUnattended"] {
            d.removeObject(forKey: key)
        }
        d.set(Terms.version, forKey: "acceptedTerms")
        d.set(true, forKey: "votingInviteAnswered")
        d.set(true, forKey: "nodeEnabled")
        for (k, v) in values { if let v { d.set(v, forKey: k) } }
    }

    func renderAll() {
        for dark in (["en", "ko"].contains(lang) ? [false, true] : [false]) {
            // Pages, as the detail column shows them (760 pt readable width).
            page("home", dark) { HomePage(sheet: .constant(nil), showActivity: {}, showNetwork: {}) }
            page("home-empty", dark, ["designPreview": "empty"]) { HomePage(sheet: .constant(nil), showActivity: {}, showNetwork: {}) }
            page("home-paused", dark, ["designPreview": "paused"]) { HomePage(sheet: .constant(nil), showActivity: {}, showNetwork: {}) }
            page("home-verifying", dark, ["designPreview": "verifying", "proveBlocks": false]) { HomePage(sheet: .constant(nil), showActivity: {}, showNetwork: {}) }
            page("home-alerts", dark, ["previewIncomingRecovery": "1"], health: .diskPaused) {
                HomePage(sheet: .constant(nil), showActivity: {}, showNetwork: {})
            }
            page("home-narrow", dark, width: 380) { HomePage(sheet: .constant(nil), showActivity: {}, showNetwork: {}) }
            page("activity", dark, ["historyNotice": "1"]) { ActivityPage() }
            page("network", dark, ["rewardStatus": "1"]) { NetworkPage() }
            page("node-status", dark) { NetworkPage.nodeStatusForScreens() }
            stagePage("validator-candidate", dark) { s in
                Group {
                    if let candidate = s.node.candidate { NetworkPage.validatorCandidateForScreens(candidate) }
                }
            }
            page("network-verifying", dark, ["designPreview": "verifying"]) { NetworkPage() }
            page("security", dark) { SecurityPage() }
            page("explore", dark) { ExplorePage(goHome: {}).frame(height: 640) }
            page("developer", dark, width: 1000) { DeveloperView() }
            // The whole window, sidebar included, wide and narrow.
            window("window", dark, width: 1000, height: 780) { SimpleDashboard() }
            window("window-network", dark, ["previewPage": "network", "rewardStatus": "1"], width: 1000, height: 780) { SimpleDashboard() }
            window("window-wrong-place", dark, ["previewWrongLocation": "1"], width: 1000, height: 780) { SimpleDashboard() }
            window("window-narrow", dark, width: 420, height: 780) { SimpleDashboard() }
            // Explore must always have a way back to Home (founder report on 0.7.0).
            window("window-explore", dark, ["previewPage": "explore"], width: 1000, height: 780) { SimpleDashboard() }
            window("window-explore-open", dark, ["previewPage": "explore", "previewExplorer": "open"], width: 1000, height: 780) { SimpleDashboard() }
            window("window-explore-narrow", dark, ["previewPage": "explore", "previewExplorer": "open"], width: 420, height: 780) { SimpleDashboard() }
            window("window-explore-nosidebar", dark, ["previewPage": "explore", "previewExplorer": "open", "previewSidebar": "hidden"],
                   width: 1000, height: 780) { SimpleDashboard() }
            // The real switcher and a selected second account, including its
            // distinct balance, tokens, and activity in the whole window.
            stagePage("switcher", dark, ["previewAccounts": 2, "previewSelectedAccount": 2], width: 340, pad: false) { s in
                AccountSwitcherPanel(store: s.accountStore)
            }
            window("two-accounts", dark, ["previewAccounts": 2, "previewSelectedAccount": 2], width: 1000, height: 780) { SimpleDashboard() }
            stagePage("retire-blocked", dark, ["previewAccounts": 2, "previewSelectedAccount": 2], width: 360, pad: false) { s in
                RetireAccountView(store: s.accountStore, account: s.accountStore.activeAccount!)
            }
            // Every reason the node can stop, as the Node page shows it.
            page("node-stop-reasons", dark) {
                VStack(alignment: .leading, spacing: 18) {
                    ForEach(Array(Self.stopReasons.enumerated()), id: \.offset) { _, r in
                        NodeStopRow(reason: r)
                        Divider()
                    }
                }
            }
            page("sheet-archive", dark, width: 460, pad: false) { ArchiveRequirementsSheet(height: 184_210) {} }
            // Settings and the menu bar.
            page("settings", dark, ["nodeUnattended": true], width: 460, pad: false) { SettingsView() }
            page("settings-developer", dark, ["developerMode": true, "proveBlocks": true], width: 460, pad: false) { SettingsView() }
            page("menubar", dark, ["proveBlocks": true], width: 300, pad: false) { MenuBarPanel() }
            page("menubar-health", dark, health: .proverStalled, width: 300, pad: false) { MenuBarPanel() }
            page("menubar-qr", dark, ["previewAccounts": 2, "previewSelectedAccount": 2], width: 300, pad: false) { MenuBarPanel(showReceive: true) }
            // Sheets.
            page("sheet-send", dark, width: 460, pad: false) { SendSheet() }
            page("sheet-send-token", dark, width: 460, pad: false, prepare: { s in s.model.sendToken = s.model.tokens.last }) { SendSheet() }
            page("sheet-send-link", dark, width: 460, pad: false, prepare: { s in
                s.model.paymentRequest = PaymentRequest(to: "0x12ab00000000000000000000000000000000090ab", amount: "2.5",
                                                        memo: "Coffee beans · order 1042", callback: nil)
            }) { SendSheet() }
            page("sheet-receive", dark, width: 400, pad: false) { ReceiveSheet() }
            page("sheet-assets", dark, width: 460, pad: false) { AssetsSheet() }
            page("sheet-call", dark, width: 480, pad: false, prepare: { s in
                s.model.callRequest = CallRequest(to: "0x961f8add5ae93ff0700be8abd5f9f8ec69ba4347", value: "1",
                                                  data: "0x38ed1739" + String(repeating: "0", count: 128), gas: 200_000,
                                                  memo: "Swap on the EastSea DEX", origin: "https://eastsea.xyz", callback: nil)
            }) { CallSheet() }
            page("sheet-connect", dark, width: 440, pad: false, prepare: { s in
                s.model.connectRequest = ConnectRequest(origin: "https://eastsea.xyz", callback: URL(string: "https://eastsea.xyz/cb")!)
            }) { ConnectSheet() }
            page("sheet-terms", dark, width: 460, pad: false) { TermsSheet(accept: {}).frame(height: 900) }
            page("sheet-voting-invite", dark, width: 440, pad: false) { VotingNodeInvite(join: {}, later: {}) }
            stagePage("sheet-site-warning", dark, width: 460, pad: false) { s in
                SiteWarningSheet(warning: .init(url: URL(string: "https://eastsea-wallet.xyz")!, host: "eastsea-wallet.xyz",
                                                lookalike: "eastsea.xyz", punycode: true), browser: s.browser)
            }
            stagePage("sheet-site-connect", dark, width: 480, pad: false) { s in
                ProviderAskSheet(ask: .init(id: "1", kind: .connect(origin: "https://eastsea.xyz", host: "eastsea.xyz"), reply: { _ in }),
                                 browser: s.browser)
            }
            stagePage("sheet-site-send", dark, width: 480, pad: false) { s in
                ProviderAskSheet(ask: .init(id: "2", kind: .send(origin: "https://eastsea.xyz", host: "eastsea.xyz",
                                                                 tx: PageTransaction(to: "0x12ab00000000000000000000000000000000090ab",
                                                                                     valueWei: "1500000000000000000", data: "0x", gas: 0),
                                                                 feeWei: "21000000000000"), reply: { _ in }),
                                 browser: s.browser)
            }
            page("overlay-migration", dark, width: 640, prepare: { _ in MigrationStatus.shared.loadPreview(moving: true, problem: nil) }) {
                MigrationOverlay(status: MigrationStatus.shared).frame(height: 360)
            }
            page("overlay-migration-problem", dark, width: 640, prepare: { s in
                // A throwaway support folder that still holds the old app's node data.
                let support = s.accountStore.directory.appendingPathComponent("support", isDirectory: true)
                try? FileManager.default.createDirectory(at: support.appendingPathComponent("Aether/node"), withIntermediateDirectories: true)
                let why = DataMigration.mayStartNode(support: support, defaults: UserDefaults(suiteName: "screens.empty")!)
                MigrationStatus.shared.loadPreview(moving: false, problem: why ?? DataMigration.movingSentence())
            }) {
                MigrationOverlay(status: MigrationStatus.shared).frame(height: 360)
            }
            alert("alert-legacy-aether", dark) {
                let q = LegacyAether.question()
                let a = NSAlert()
                a.messageText = q.title
                a.informativeText = q.body + "\n\n/Applications/Aether.app"
                a.addButton(withTitle: q.confirm)
                a.addButton(withTitle: q.later)
                return a
            }
            alert("alert-install-place", dark) {
                let a = NSAlert()
                a.messageText = InstallLocation.moveSentence
                a.informativeText = String(localized: "\(Brand.name) is running from a temporary place.")
                a.addButton(withTitle: String(localized: "OK"))
                return a
            }
        }
    }

    static let stopReasons: [NodeStopReason] = [
        .switchedOff, .onBattery, .wrongLocation, .noHelper, .migrating, .migrationBlocked(DataMigration.movingSentence()),
        .otherNodeRunning, .diskFull(freeBytes: 3_000_000_000, resumeBytes: 7_000_000_000, volume: nil),
        .diskFull(freeBytes: 3_000_000_000, resumeBytes: 7_000_000_000, volume: "Samsung T7"),
        .diskMissing(volume: "Samsung T7"), .diskNoAccess(volume: "Samsung T7"), .restarting(inSeconds: 20),
        .crashLoop(.other, retryInSeconds: 300), .needsAttention(.database), .upgradeNeeded, .identityLost,
        .waitingForMacConfirmation, .keyElsewhere,
        .launchFailed("posix_spawn failed"), .movingStorage(percent: 42),
    ]

    // MARK: drawing

    private func wanted(_ name: String) -> Bool { only.map { name.hasPrefix($0) } ?? true }

    private func file(_ name: String, _ dark: Bool) -> URL {
        out.appendingPathComponent("\(name)-\(lang)-\(dark ? "dark" : "light").png")
    }

    /// A view at a fixed width and its own height.
    private func page<V: View>(_ name: String, _ dark: Bool, _ values: [String: Any?] = [:], health: HealthCheck.Issue? = nil,
                               width: CGFloat = 760, pad: Bool = true, prepare: ((Stage) -> Void)? = nil,
                               @ViewBuilder _ content: @escaping () -> V) {
        stagePage(name, dark, values, health: health, width: width, pad: pad, prepare: prepare) { _ in content() }
    }

    private func stagePage<V: View>(_ name: String, _ dark: Bool, _ values: [String: Any?] = [:], health: HealthCheck.Issue? = nil,
                                    width: CGFloat = 760, pad: Bool = true, prepare: ((Stage) -> Void)? = nil,
                                    _ content: @escaping (Stage) -> V) {
        guard wanted(name) else { return }
        set(values)
        let s = Stage()
        if let health { s.health.loadPreview(issue: health) }
        prepare?(s)
        let view = s.wrap(content(s).padding(pad ? 24 : 0).frame(width: width).background(.background))
        snap(view, name: name, dark: dark, width: width, height: nil)
    }

    /// The full window at a fixed size.
    private func window<V: View>(_ name: String, _ dark: Bool, _ values: [String: Any?] = [:],
                                 width: CGFloat, height: CGFloat, @ViewBuilder _ content: () -> V) {
        guard wanted(name) else { return }
        set(values)
        let s = Stage()
        snap(s.wrap(content().frame(width: width, height: height)), name: name, dark: dark, width: width, height: height)
    }

    private func alert(_ name: String, _ dark: Bool, _ make: () -> NSAlert) {
        guard wanted(name) else { return }
        let a = make()
        a.window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        a.layout()
        a.window.setFrameOrigin(NSPoint(x: -30_000, y: -30_000))
        a.window.orderFrontRegardless()
        settle(0.4)
        guard let view = a.window.contentView else { failed += 1; return }
        if let image = windowImage(a.window) {
            writeImage(image, name: name, dark: dark)
        } else {
            write(view, name: name, dark: dark)
        }
        writeText(view, name: name, dark: dark)
        a.window.orderOut(nil)
    }

    private func snap<V: View>(_ view: V, name: String, dark: Bool, width: CGFloat, height: CGFloat?) {
        let host = NSHostingView(rootView: view.environment(\.colorScheme, dark ? .dark : .light))
        let full = height != nil
        let win = NSWindow(contentRect: NSRect(x: -30_000, y: -30_000, width: width, height: height ?? 600),
                           styleMask: full ? [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView] : [.borderless],
                           backing: .buffered, defer: false)
        win.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        win.backgroundColor = .windowBackgroundColor
        win.isReleasedWhenClosed = false
        win.contentView = host
        win.orderFrontRegardless()
        settle(0.5)   // onAppear, the first data, and one layout pass
        if height == nil {
            let fit = host.fittingSize
            win.setContentSize(NSSize(width: width, height: min(max(fit.height, 40), 6_000)))
            settle(0.3)
        }
        // A whole window (sidebar, toolbar, materials) is composited by the
        // window server, which a view's own cacheDisplay cannot draw: ask
        // the server for this process's own window. Pages draw themselves.
        if !full, name != "explore" {
            let renderer = ImageRenderer(content: view.environment(\.colorScheme, dark ? .dark : .light))
            renderer.proposedSize = ProposedViewSize(width: width, height: host.bounds.height)
            renderer.scale = 4
            if let image = renderer.cgImage {
                writeImage(image, name: name, dark: dark)
            } else {
                write(host, name: name, dark: dark)
            }
        } else if full, let image = windowImage(win) {
            writeImage(image, name: name, dark: dark)
        } else {
            write(host, name: name, dark: dark)
        }
        writeText(win.contentView, name: name, dark: dark)
        win.orderOut(nil)
        win.contentView = nil
    }

    private func settle(_ seconds: TimeInterval) {
        RunLoop.main.run(until: Date().addingTimeInterval(seconds))
    }

    /// CGWindowListCreateImage, looked up at run time (the SDK marks it
    /// obsolete; it still captures a process's own windows).
    private func windowImage(_ win: NSWindow) -> CGImage? {
        typealias Fn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
        guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage") else { return nil }
        let fn = unsafeBitCast(sym, to: Fn.self)
        // .optionIncludingWindow = 1 << 3, .boundsIgnoreFraming = 1 << 0, .bestResolution = 1 << 3
        return fn(.null, 1 << 3, UInt32(win.windowNumber), (1 << 0) | (1 << 3))?.takeRetainedValue()
    }

    /// Every string the view shows, read back through accessibility (what
    /// VoiceOver would say): written next to the PNG, so
    /// scripts/check-wallet-screens-language.py can check the language of
    /// each render without reading pixels.
    private func writeText(_ root: Any?, name: String, dark: Bool) {
        var lines: [String] = []
        var seen = Set<ObjectIdentifier>()
        func walk(_ any: Any?, depth: Int) {
            guard depth < 60, let el = any as? NSObject else { return }
            guard seen.insert(ObjectIdentifier(el)).inserted else { return }
            if let e = el as? NSAccessibilityProtocol {
                for t in [e.accessibilityLabel(), e.accessibilityValue() as? String, e.accessibilityTitle()] {
                    if let t, !t.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { lines.append(t) }
                }
                for c in e.accessibilityChildren() ?? [] { walk(c, depth: depth + 1) }
            }
            if let v = el as? NSView { for c in v.subviews { walk(c, depth: depth + 1) } }
            if let t = el as? NSTextField { lines.append(t.stringValue) }
        }
        walk(root, depth: 0)
        let url = file(name, dark).deletingPathExtension().appendingPathExtension("txt")
        try? lines.joined(separator: "\n").write(to: url, atomically: true, encoding: .utf8)
    }

    private func writeImage(_ image: CGImage, name: String, dark: Bool) {
        let rep = NSBitmapImageRep(cgImage: image)
        guard let png = rep.representation(using: .png, properties: [:]) else { failed += 1; return }
        do {
            try png.write(to: file(name, dark))
            try writeText(image, name: name, dark: dark)
            written += 1
        } catch {
            print("could not write \(name): \(error)")
            failed += 1
        }
    }

    /// Visible text from the exact pixels being saved, including SwiftUI and
    /// embedded web content. No accessibility permission or real app is used.
    /// Keep the boxes/confidence for reviewing a language-check failure.
    private func writeText(_ image: CGImage, name: String, dark: Bool) throws {
        let languageNames = ["en": "en-US", "ko": "ko-KR", "ja": "ja-JP", "zh-Hans": "zh-Hans", "zh-Hant": "zh-Hant"]
        let supported = try VNRecognizeTextRequest().supportedRecognitionLanguages()
        func languages(_ preferred: [String]) -> [String] {
            var result: [String] = []
            for language in preferred where supported.contains(language) && !result.contains(language) {
                result.append(language)
            }
            return result
        }
        func recognize(_ languages: [String], automaticallyDetectsLanguage: Bool) throws -> [[String: Any]] {
            let request = VNRecognizeTextRequest()
            request.recognitionLevel = .accurate
            request.usesLanguageCorrection = !automaticallyDetectsLanguage
            if !automaticallyDetectsLanguage {
                request.customWords = ["EastSea", "Doubloon", "Aether", "DBLN", "Mac", "Touch ID", "Face ID",
                                       "Secure Enclave", "Apple", "DeviceCheck", "Pipln", "Sparkle", "Safari", "WebKit",
                                       "Samsung T7", "Finder", "FileVault", "iCloud", "macOS", "iOS", "Metal", "APFS",
                                       "Mac OS Extended", "USDX", "VVDBLN", "NEB", "ORB", "CMT", "WAETH", "AETH"]
            }
            request.automaticallyDetectsLanguage = automaticallyDetectsLanguage
            request.recognitionLanguages = languages
            try VNImageRequestHandler(cgImage: image, options: [:]).perform([request])
            return (request.results ?? []).compactMap { observation in
                guard let candidate = observation.topCandidates(1).first else { return nil }
                let box = observation.boundingBox
                return ["text": candidate.string, "confidence": candidate.confidence,
                        "box": [box.origin.x, box.origin.y, box.width, box.height]]
            }
        }
        // A broad model can convert Traditional Han glyphs to Simplified or
        // read an SF Symbol as kana. Keep both readings so QA can distinguish
        // recognizer disagreements from actual foreign copy in the pixels.
        let primaryLanguages = languages([languageNames[lang] ?? "en-US", "en-US"])
        let preferred = [languageNames[lang] ?? "en-US", "en-US", "ko-KR", "ja-JP", "zh-Hans", "zh-Hant"]
        let multilingualLanguages = languages(preferred)
        let lines = try recognize(primaryLanguages, automaticallyDetectsLanguage: false)
        let multilingualLines = try recognize(multilingualLanguages, automaticallyDetectsLanguage: true)
        var payload: [String: Any] = ["screen": name, "language": lang, "appearance": dark ? "dark" : "light",
                                     "engine": "Vision", "lines": lines, "multilingualLines": multilingualLines,
                                     "recognitionLanguages": primaryLanguages,
                                     "multilingualLanguages": multilingualLanguages]
        if name == "menubar-qr" || name == "sheet-receive" {
            // Decode the saved pixels, so the gate proves the QR carries the
            // selected account's address rather than merely resembling a QR.
            let barcodes = VNDetectBarcodesRequest()
            barcodes.symbologies = [.qr]
            try VNImageRequestHandler(cgImage: image, options: [:]).perform([barcodes])
            payload["qrPayloads"] = (barcodes.results ?? []).compactMap(\.payloadStringValue)
        }
        let data = try JSONSerialization.data(withJSONObject: payload, options: [.prettyPrinted, .sortedKeys])
        try data.write(to: file(name, dark).deletingPathExtension().appendingPathExtension("text.json"))
    }

    private func write(_ view: NSView, name: String, dark: Bool) {
        view.layoutSubtreeIfNeeded()
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { failed += 1; return }
        view.cacheDisplay(in: view.bounds, to: rep)
        guard let image = rep.cgImage else { failed += 1; return }
        writeImage(image, name: name, dark: dark)
    }
}
#endif
