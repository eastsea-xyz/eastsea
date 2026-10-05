import SwiftUI
#if os(macOS)
import Combine
import Sparkle
#endif

@main
struct AetherWalletApp: App {
    @StateObject private var model = WalletModel()
    #if os(macOS)
    @StateObject private var node = NodeController()
    /// The unattended-restart half of the node (docs/design/29): the
    /// LaunchDaemon registration, its marker, and the honest power facts.
    @StateObject private var unattended = UnattendedDaemon()
    @StateObject private var earnings = Earnings()
    /// Views pause their continuous animations while a window is live-resizing.
    @StateObject private var resizeMonitor = WindowResizeMonitor()
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @AppStorage("developerMode") private var developerMode = false
    #endif

    var body: some Scene {
        WindowGroup("\(Brand.project)", id: "main") {
            #if os(macOS)
            ContentView()
                .environmentObject(model)
                .environmentObject(node)
                .environmentObject(unattended)
                .environmentObject(appDelegate.updates)
                .environmentObject(earnings)
                .onAppear {
                    appDelegate.start(node: node, model: model, unattended: unattended)
                    earnings.attach(node, operatorAddress: { model.address })
                    NSApp.setActivationPolicy(.regular)
                    #if DEBUG
                    if ResizeBenchmark.on { ResizeBenchmark.run() }
                    #endif
                }
                .environment(\.liveResize, resizeMonitor.active)
                // Closing the window keeps Aether in the menu bar (the node keeps running).
                .onDisappear { NSApp.setActivationPolicy(.accessory) }
                .onOpenURL { model.open(url: $0) }
                // aether:// links go to the open window instead of opening another one.
                .handlesExternalEvents(preferring: ["*"], allowing: ["*"])
            #else
            ContentView()
                .environmentObject(model)
                .onOpenURL { model.open(url: $0) }
                #if DEBUG
                // `-spinnerGallery` (debug builds only) shows every loader on one screen.
                .overlay {
                    if ProcessInfo.processInfo.arguments.contains("-spinnerGallery") {
                        SpinnerGallery().frame(maxWidth: .infinity, maxHeight: .infinity).background(.background)
                    }
                }
                // `-earningsPreview` (debug builds only): the Mac's earnings cards with sample rewards.
                .overlay {
                    if ProcessInfo.processInfo.arguments.contains("-earningsPreview") {
                        EarningsPreviewHarness()
                    }
                }
                #endif
            #endif
        }
        #if os(macOS)
        // Resizable from an iPhone-wide window up; the content sets the minimum.
        .defaultSize(width: 1000, height: 720)
        .windowResizability(.contentMinSize)
        .handlesExternalEvents(matching: ["*"])
        .commands {
            CommandGroup(after: .appInfo) {
                Button("Check for Updates…") { appDelegate.updater.checkForUpdates(nil) }
            }
            CommandGroup(after: .appSettings) {
                Button("Install Command-Line Tools…") { CommandLineTools.install() }
            }
            CommandGroup(after: .sidebar) {
                Toggle("Developer Mode", isOn: $developerMode)
                    .keyboardShortcut("d", modifiers: [.command, .shift])
            }
        }
        #endif
        #if os(macOS)
        Settings {
            SettingsView().environmentObject(node).environmentObject(model).environmentObject(unattended).environmentObject(appDelegate.updates)
        }
        // Always in the menu bar: balance, node and prover at a glance; the window opens from here.
        MenuBarExtra {
            MenuBarPanel().environmentObject(model).environmentObject(node).environmentObject(earnings)
                .onAppear {
                    appDelegate.start(node: node, model: model, unattended: unattended)
                    earnings.attach(node, operatorAddress: { model.address })
                }
        } label: {
            Image(systemName: node.prover?.proving != nil ? "cube.transparent.fill" : "cube.transparent")
        }
        .menuBarExtraStyle(.window)
        #endif
    }
}

#if os(macOS)
/// Aether ▸ Settings: how the node runs on this Mac.
struct SettingsView: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var model: WalletModel
    @EnvironmentObject var updates: Updates
    @EnvironmentObject var unattended: UnattendedDaemon
    @AppStorage("developerMode") private var developerMode = false
    @AppStorage("useDevelopmentNetwork") private var useDevelopmentNetwork = false
    @AppStorage("developmentNetworkPort") private var developmentNetworkPort = 18546

    var body: some View {
        Form {
            if model.developmentNetwork {
                Text("Dev network · 127.0.0.1:\(developmentNetworkPort)")
                    .font(.caption.bold()).foregroundStyle(.orange)
            }
            Toggle("Run a node on this Mac", isOn: $node.enabled)
            Toggle("Only while on the power adapter", isOn: $node.onlyOnPower)
                .help("On a laptop, pause the node on battery and resume on power.")
            Label(node.awakeNote, systemImage: node.keepsAwake ? "sun.max.fill" : "moon.zzz")
                .font(.caption).foregroundStyle(.secondary)
            Toggle("Open \(Brand.project) at login", isOn: Binding(get: { node.startAtLogin }, set: { node.startAtLogin = $0 }))
            UnattendedSection()
            ResourcesSection()
            Text("Your node verifies every block itself and your wallet asks it instead of the network. Quitting \(Brand.project) stops it.")
                .font(.caption).foregroundStyle(.secondary)
            // Honest power ranges (docs/research/mac-power-cost-2026.md): the node is
            // cheap; GPU proving is the costly part. No won figure — electricity
            // prices vary, and the range is the honest statement.
            Text("Power: roughly 5–6 W while only verifying (about 4 kWh a month); proving on the GPU adds roughly 28–50 W (about 20–36 kWh a month).")
                .font(.caption).foregroundStyle(.secondary)
            Divider()
            Toggle("Developer mode (proofs, state roots, raw logs)", isOn: $developerMode)
                .help("Also in View ▸ Developer Mode (⇧⌘D)")
            if developerMode {
                Picker("Network", selection: $useDevelopmentNetwork) {
                    Text("Default").tag(false)
                    Text("Local development network").tag(true)
                }
                Stepper("Local RPC: http://127.0.0.1:\(developmentNetworkPort)", value: $developmentNetworkPort, in: 1024...65535)
                    .disabled(!useDevelopmentNetwork)
            }
            if let pending = updates.pendingRelease {
                Divider()
                Text("Approved release \(pending.version) (\(pending.build))")
                Text("SHA-256: \(pending.fingerprint)")
                    .font(.caption.monospaced()).fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                Text("Published in block \(pending.publishedBlock)")
                    .font(.caption).foregroundStyle(.secondary)
                if pending.emergency {
                    Label("Emergency release · all three builders signed", systemImage: "exclamationmark.shield")
                } else if let date = pending.availableAt {
                    Text("Installable after \(date.formatted())")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            if let issue = updates.approvalIssue {
                Text(issue).font(.caption).foregroundStyle(.orange)
            }
        }
        .onChange(of: useDevelopmentNetwork) { _, dev in
            model.selectNetwork(development: dev, port: UInt16(developmentNetworkPort))
            if !dev { node.refreshWalletRoute() }
        }
        .onChange(of: developmentNetworkPort) { _, port in
            if useDevelopmentNetwork { model.selectNetwork(development: true, port: UInt16(port)) }
        }
        .onChange(of: developerMode) { _, enabled in
            if !enabled { useDevelopmentNetwork = false; model.selectNetwork(development: false); node.refreshWalletRoute() }
        }
        .padding(20)
        .frame(width: 420)
    }
}

/// Settings ▸ the "keep this Mac's node running after restarts" switch
/// (docs/design/29-unattended-restart.md): the toggle, the one-time system
/// approval it needs, and the honest power sentences — what comes back after
/// a power cut, and where macOS itself stops (FileVault).
struct UnattendedSection: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var unattended: UnattendedDaemon

    var body: some View {
        Group {
            Toggle("Keep this Mac's node running after restarts", isOn: $unattended.enabled)
                .help("After a reboot the node — and this Mac's vote — come back by themselves, without anyone logging in, everywhere macOS allows it. A FileVault cold boot waits for one unlock first.")
            switch unattended.status {
            case .needsApproval:
                Label("Registered. Allow EastSea in System Settings ▸ Login Items — until then the daemon does not run.", systemImage: "hand.raised")
                    .font(.caption).foregroundStyle(.orange)
                Button("Open System Settings") { unattended.openApprovalPane() }
            case .failed(let why):
                Label(why, systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.orange)
            case .off, .approved:
                EmptyView()
            }
            if unattended.enabled {
                Label("This Mac is in, or can enter, the voting set: that is why this is on by default. While nobody is logged in the node keeps verifying and voting; daily reward re-attestation resumes when the app is open again.", systemImage: "arrow.clockwise")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            ForEach(UnattendedDecision.powerLines(unattended.power), id: \.self) { line in
                Label(line, systemImage: "bolt")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .onAppear {
            // The user approves outside the app; re-read both facts whenever
            // Settings appears (docs/design/29).
            unattended.refreshStatus()
            unattended.refreshPower()
        }
    }
}

/// Aether lives in the menu bar: closing the window keeps it (and its node)
/// running; Quit stops both.
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var node: NodeController?
    weak var model: WalletModel?
    private let releaseGate = ReleaseUpdateGate()
    /// Sparkle: checks the signed appcast on GitHub Releases and installs updates.
    lazy var updater = SPUStandardUpdaterController(startingUpdater: true, updaterDelegate: self, userDriverDelegate: nil)
    /// What the Network page shows: when updates were last checked, and a Check button.
    @MainActor lazy var updates = Updates(updater)
    private var started = false
    /// Tells a validator Mac when the network pauses and resumes.
    private var pauseWatch: AnyCancellable?
    /// Update failures, persisted and retried by cause (red team #11):
    /// discover → download → verify → install → health, across relaunches.
    private var tracker = UpdateTracker()
    /// 30 s: the post-update health window and the scheduled retries.
    private var updateTickTimer: Timer?

    func applicationDidFinishLaunching(_ notification: Notification) {
        MainActor.assumeIsolated {
            // Whether the last update landed is a fact about the running
            // binary, not about what Sparkle last said (red team #11): resolve
            // the persisted record before the first check runs.
            let info = Bundle.main.infoDictionary
            tracker.relaunched(runningVersion: (info?["CFBundleShortVersionString"] as? String) ?? "",
                               runningBuild: (info?["CFBundleVersion"] as? String) ?? "")
            updates.installNotice = tracker.sentence
        }
        #if DEBUG
        if DesignPreview.on { return }
        #endif
        _ = updater  // start checking right away (hourly, and when the chain schedules a newer protocol)
    }

    /// Once per launch, from whichever appears first (window or menu-bar panel).
    @MainActor func start(node: NodeController, model: WalletModel, unattended: UnattendedDaemon) {
        guard !started else { return }
        started = true
        #if DEBUG
        if DesignPreview.on {
            node.loadPreview()
            let w = UserDefaults.standard.double(forKey: "previewWidth")
            let target = w > 0 ? w : 1000
            // The window may appear well after launch (shared saved state decides),
            // so keep trying briefly: size it once it exists.
            var tries = 0
            Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { t in
                tries += 1
                if let win = NSApp.windows.first(where: { $0.isVisible && $0.canBecomeMain }) {
                    win.setContentSize(NSSize(width: target, height: 780))
                    win.center()
                    t.invalidate()
                } else if tries > 24 {
                    t.invalidate()
                }
            }
            return
        }
        #endif
        self.node = node
        self.model = model
        let check: () -> Void = { [weak self] in self?.updater.updater.checkForUpdatesInBackground() }
        node.onUpgradeNeeded = check
        model.onOutdated = check
        pauseWatch = model.$chainPausedSince
            .removeDuplicates { ($0 == nil) == ($1 == nil) }
            .sink { [weak node] since in
                MainActor.assumeIsolated { node?.networkPaused(since: since) }
            }
        updateTickTimer = Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.updateTick() }
        }
        // Open at login by default (Settings can turn it off). Not from a
        // wrong place (red team #10): a login item pointing into a DMG or a
        // translocated copy is gone at the next unmount, and the "applied"
        // flag stays unset too, so the first run from a proper Applications
        // folder still takes the default.
        if !node.wrongLocation, !UserDefaults.standard.bool(forKey: "loginItemDefaultApplied") {
            UserDefaults.standard.set(true, forKey: "loginItemDefaultApplied")
            node.startAtLogin = true
        }
        // The unattended-restart half (docs/design/29): the node owns the
        // switch, this owns the daemon. Wire them before the node resumes,
        // so its first start already writes the marker — and if a daemon node
        // survived the reboot, the node's run.lock exit turns into attach.
        unattended.nodeEnabled = node.enabled
        unattended.wrongLocation = node.wrongLocation
        node.unattended = unattended
        unattended.refreshStatus()
        unattended.refreshPower()
        node.restore()
    }

    func applicationWillTerminate(_ notification: Notification) {
        MainActor.assumeIsolated {
            updateTickTimer?.invalidate()
            node?.stop()
        }
    }

    /// Every 30 s (red team #11): end the health window when it passed, watch
    /// for the node coming up after an update, and re-check when a scheduled
    /// retry is due. The decisions live in `UpdateTracker`.
    @MainActor private func updateTick() {
        tracker.tick()
        if let node, node.state == .running || node.state == .starting {
            tracker.nodeRunning()
        }
        if tracker.retryDue() { updater.updater.checkForUpdatesInBackground() }
        let notice = tracker.sentence
        if updates.installNotice != notice { updates.installNotice = notice }
    }

    @MainActor private func syncUpdateNotice() {
        let notice = tracker.sentence
        if updates.installNotice != notice { updates.installNotice = notice }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
}

/// Link the bundled `aether` and `aether-agent` into ~/.local/bin.
enum CommandLineTools {
    static func install() {
        // Red team #10: a symlink into a bundle that disappears (a DMG, a
        // translocated copy) is a command line that breaks at the next
        // unmount — the move sentence says what to do instead.
        guard InstallLocation.currentIsRunnable else {
            let alert = NSAlert()
            alert.messageText = InstallLocation.moveSentence
            alert.informativeText = Brand.project + " is running from a temporary place."
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
            done.append("failed: \(error.localizedDescription)")
        }
        let alert = NSAlert()
        alert.messageText = done.isEmpty ? "No command-line tools in this build" : "Command-line tools installed"
        alert.informativeText = done.joined(separator: "\n") + "\n\nMake sure ~/.local/bin is on your PATH."
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

extension AppDelegate: SPUUpdaterDelegate {
    func updater(_ updater: SPUUpdater, didFindValidUpdate item: SUAppcastItem) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            _ = self.tracker.found(key: ReleaseUpdateGate.trackerKey(item),
                version: (item.displayVersionString as String?) ?? "", build: item.versionString)
            self.syncUpdateNotice()
            self.startReleasePreflight(item)
        }
    }

    @MainActor private func startReleasePreflight(_ item: SUAppcastItem) {
        releaseGate.inspect(item, validators: model?.validators ?? 0) { [weak self] pending, issue, ready in
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.updates.pendingRelease = pending
                self.updates.approvalIssue = issue
                // The preflight concluded without an approval and without even
                // a pending window: this exact item is refused (red team #11).
                if pending == nil, issue != nil { self.tracker.refused() }
                if ready { self.updater.updater.checkForUpdatesInBackground() }
                self.syncUpdateNotice()
            }
        }
    }

    /// This selector is Sparkle's synchronous gate before its own download.
    /// The preflight hashes the archive and binds its EdDSA signature to the
    /// manifest; Sparkle then verifies that exact signature on its download.
    /// Swift imports Sparkle's `BOOL ... error:` selector as a throwing
    /// method: returning proceeds, throwing stops the update.
    func updater(_ updater: SPUUpdater, shouldProceedWithUpdate item: SUAppcastItem,
                 updateCheck: SPUUpdateCheck) throws {
        if releaseGate.mayProceed(item) {
            Task { @MainActor [weak self] in
                self?.tracker.downloading()  // the gate passed: download and verify
                self?.syncUpdateNotice()
            }
            return
        }
        Task { @MainActor [weak self] in self?.startReleasePreflight(item) }
        throw NSError(domain: "AetherReleaseApproval", code: 1,
            userInfo: [NSLocalizedDescriptionKey: "This update is not approved on chain yet"])
    }

    /// Sparkle gave up on this cycle (download error, verification failure,
    /// install error). The tracker sorts it by cause and schedules the retry
    /// (red team #11); the gate's own two-step refusal arrives here too and is
    /// not a failure of the update itself.
    func updater(_ updater: SPUUpdater, didAbortWithError error: Error) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            let ns = error as NSError
            guard ns.domain != "AetherReleaseApproval" else { return }
            self.tracker.aborted(networkError: ns.domain == NSURLErrorDomain
                || ns.underlyingErrors.contains { ($0 as? NSError)?.domain == NSURLErrorDomain })
            self.syncUpdateNotice()
        }
    }

    /// Sparkle installs a downloaded update when the app quits, but Aether stays
    /// in the menu bar with its node for days. Install now instead: the app
    /// relaunches on the new version and the node restarts with it, so a Mac
    /// that never quits still follows protocol upgrades.
    func updater(_ updater: SPUUpdater, willInstallUpdateOnQuit item: SUAppcastItem, immediateInstallationBlock: @escaping () -> Void) -> Bool {
        // The record must say "installing" before the block runs: the app can
        // be killed inside it, and the next launch decides by comparing
        // versions (red team #11).
        MainActor.assumeIsolated {
            tracker.verified()
            tracker.installing()
            syncUpdateNotice()
        }
        immediateInstallationBlock()
        return true
    }
}
#endif
