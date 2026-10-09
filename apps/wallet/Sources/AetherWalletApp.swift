import SwiftUI
#if os(macOS)
import Combine
import os
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
        WindowGroup(Brand.name, id: "main") {
            #if os(macOS)
            ContentView()
                .environmentObject(model)
                .environmentObject(node)
                .environmentObject(unattended)
                .environmentObject(appDelegate.updates)
                .environmentObject(appDelegate.health)
                .environmentObject(earnings)
                .onAppear {
                    appDelegate.start(node: node, model: model, unattended: unattended)
                    earnings.attach(node, operatorAddress: { model.payoutAddress })
                    NSApp.setActivationPolicy(.regular)
                    #if DEBUG
                    if ResizeBenchmark.on { ResizeBenchmark.run() }
                    #endif
                }
                .environment(\.liveResize, resizeMonitor.active)
                // The Aether → EastSea data move, while it runs (M1).
                .overlay { MigrationOverlay(status: appDelegate.migration) }
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
            CommandMenu("Go") { PageCommands() }
            CommandGroup(after: .help) {
                Button(DiagnosticReport.copyLabel(helpMenu: true)) { appDelegate.health.copyDiagnostics() }
            }
        }
        #endif
        #if os(macOS)
        Settings {
            SettingsView().environmentObject(node).environmentObject(model).environmentObject(unattended).environmentObject(appDelegate.updates)
        }
        // Always in the menu bar: balance, node and prover at a glance; the window opens from here.
        MenuBarExtra {
            MenuBarPanel().environmentObject(model).environmentObject(node).environmentObject(earnings).environmentObject(unattended)
                .environmentObject(appDelegate.health)
                .onAppear {
                    appDelegate.start(node: node, model: model, unattended: unattended)
                    earnings.attach(node, operatorAddress: { model.payoutAddress })
                }
        } label: {
            Image(systemName: node.prover?.proving != nil ? "cube.transparent.fill" : "cube.transparent")
        }
        .menuBarExtraStyle(.window)
        #endif
    }
}

#if os(macOS)
/// Go ▸ Home … Security, ⌘1…⌘5: every page is one shortcut away, whatever
/// the window's width or whether its sidebar shows.
struct PageCommands: View {
    @FocusedBinding(\.dashboardPage) private var page

    var body: some View {
        ForEach(Array(SimpleDashboard.Page.allCases.enumerated()), id: \.element) { i, p in
            Button(p.title) { page = p }
                .keyboardShortcut(KeyEquivalent(Character("\(i + 1)")), modifiers: .command)
                .disabled(page == nil)
        }
    }
}

/// Aether lives in the menu bar: closing the window keeps it (and its node)
/// running; Quit stops both.
final class AppDelegate: NSObject, NSApplicationDelegate {
    /// First, before anything below calls `DataMigration.ensure()` (the
    /// tracker does, during this object's own initialisation): the move's
    /// progress and outcome must reach the window (release-070 review, M1).
    let migration = MigrationStatus.shared
    weak var node: NodeController?
    weak var model: WalletModel?
    private let releaseGate = ReleaseUpdateGate()
    /// Sparkle: checks the signed appcast on GitHub Releases and installs updates.
    lazy var updater = SPUStandardUpdaterController(startingUpdater: true, updaterDelegate: self, userDriverDelegate: nil)
    /// What the Network page shows: when updates were last checked, and a Check button.
    @MainActor lazy var updates = Updates(updater)
    /// Layer 1 of the health signal (docs/design/32-health-signal.md §4.2):
    /// the banner, the notifications, the 90 s re-discovery.
    @MainActor lazy var health = HealthMonitor()
    private var started = false
    /// Tells a validator Mac when the network pauses and resumes.
    private var pauseWatch: AnyCancellable?
    /// Update failures, persisted and retried by cause (red team #11):
    /// discover → download → verify → install → health, across relaunches.
    private var tracker = UpdateTracker()
    /// 30 s: the post-update health window and the scheduled retries.
    private var updateTickTimer: Timer?
    /// Sparkle's install-and-relaunch block for a downloaded, verified update,
    /// held until `UpdateWindow` says now (docs/design/34 §3.2, W1).
    private var heldInstall: (() -> Void)?
    private var heldVersion = ""
    private var heldReason: UpdateWindow.Reason?
    private var updateShutdownTask: Task<Void, Never>?
    private var updateShutdownID: UUID?
    private var updateShutdownReady = false
    static let updateLog = Logger(subsystem: "com.pipln.eastsea", category: "update")

    func applicationDidFinishLaunching(_ notification: Notification) {
        MainActor.assumeIsolated {
            // Whether the last update landed is a fact about the running
            // binary, not about what Sparkle last said (red team #11): resolve
            // the persisted record before the first check runs.
            let info = Bundle.main.infoDictionary
            tracker.relaunched(runningVersion: (info?["CFBundleShortVersionString"] as? String) ?? "",
                               runningBuild: (info?["CFBundleVersion"] as? String) ?? "")
            updates.installNotice = tracker.sentence
            if case .awaitingHealth(let version, let build, _) = tracker.state {
                Self.updateLog.notice("relaunched as \(version, privacy: .public) (\(build, privacy: .public)); the node starts through the normal start path")
            }
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
        node.onUpdateMomentChanged = { [weak self] in self?.installIfSafe() }
        node.authorizeKeyRebind = { [weak model] address, typed, directory in
            guard let model else { throw NodeKeyRebind.Refusal.ownerKeyUnavailable }
            return try await model.authorizeNodeKeyRebind(validatorAddress: address, typedAddress: typed,
                                                         dataDirectory: directory)
        }
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
        unattended.storageShards = node.storageShards
        node.unattended = unattended
        unattended.restore()
        unattended.refreshPower()
        node.restore()
        // A slow data move finishing in the background (M1) lets the node
        // start at once instead of at the next 30 s power tick.
        migration.onFinish = { [weak node, weak model] outcome in
            MainActor.assumeIsolated {
                model?.migrationFinished(outcome)
                // Failed node moves stay stopped. A completed node half may
                // resume while the protected wallet handle waits for unlock.
                if DataMigration.mayStartNode() == nil { node?.migrationFinished() }
            }
        }
        // An old Aether (<= 0.6.6) beside EastSea opens at login, holds the
        // old data's run.lock and runs a second node (B2): ask once to quit
        // it and move it to the Trash, then retry the move it was blocking.
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            LegacyAether.offerRemoval {
                DataMigration.Runner.shared.start()
            }
        }
        health.start(node: node, model: model, updateComing: { [weak self] in
            switch self?.tracker.state {
            case .found?, .downloading?, .verified?, .installing?: return true
            default: return false
            }
        }, updateUnhealthy: { [weak self] in
            self?.tracker.state.failedCause == .health
        }, openUpdates: { [weak self] in
            self?.updates.check()
        })
    }

    func applicationWillTerminate(_ notification: Notification) {
        MainActor.assumeIsolated {
            updateTickTimer?.invalidate()
            // Update termination was approved only after the async shutdown
            // acquired run.lock. Ordinary quit retains the daemon's behavior.
            node?.stop()
        }
    }

    /// Every 30 s (red team #11): end the health window when it passed, watch
    /// for the node coming up after an update, and re-check when a scheduled
    /// retry is due. The decisions live in `UpdateTracker`.
    @MainActor private func updateTick() {
        tracker.tick()
        if let node {
            tracker.nodeRunning(running: node.state == .running && node.rpcAnswering,
                                releaseVerified: node.updateReleaseVerified && !node.usePreviousBinary)
        }
        if tracker.retryDue() { updater.updater.checkForUpdatesInBackground() }
        installIfSafe()
        let notice = tracker.sentence
        if updates.installNotice != notice { updates.installNotice = notice }
    }

    /// Install the held update if this is a safe moment (`UpdateWindow`).
    /// Runs when Sparkle hands over the update and on every 30 s tick.
    @MainActor private func installIfSafe() {
        guard updateShutdownTask == nil, let install = heldInstall else { return }
        // Before `start` the node and wallet are unknown: wait for the tick.
        guard let node, let model else { return }
        node.refreshUpdateMembership()
        let moment = updateMoment(node: node, model: model)
        switch UpdateWindow.decide(moment) {
        case .wait(let reason):
            if heldReason != reason {
                heldReason = reason
                Self.updateLog.notice("\(reason.logLine, privacy: .public)")
            }
        case .installNow:
            beginUpdateShutdown(install: install, quit: false)
        }
    }

    @MainActor private func updateMoment(node: NodeController, model: WalletModel) -> UpdateWindow.Moment {
        return UpdateWindow.Moment(
            seated: node.updateMembership,
            // N1 (aether_status.restart) is not built: no chain-assigned slot
            // yet, so a seated Mac waits until it leaves the committee or quits.
            inOwnSlot: nil,
            sendSheetOpen: model.sendSheetOpen,
            signing: model.busy,
            migrating: migration.moving,
            // The block-data move (claude/node-status-storage) wires in here.
            storageMoving: node.storageMovePreparing)
    }

    /// AppKit asks this before willTerminate. A held Sparkle update cannot
    /// bypass the same storage/signing/membership gate merely because we quit.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        MainActor.assumeIsolated {
            guard !migration.moving, node?.storageMovePreparing != true, model?.busy != true else {
                if updateShutdownReady {
                    updateShutdownReady = false
                    self.node?.abortUpdatePreparation()
                    tracker.aborted(networkError: false)
                    syncUpdateNotice()
                }
                return .terminateCancel
            }
            if updateShutdownReady {
                guard let node, let model, UpdateWindow.decide(updateMoment(node: node, model: model)) == .installNow else {
                    updateShutdownReady = false
                    self.node?.abortUpdatePreparation()
                    tracker.aborted(networkError: false)
                    syncUpdateNotice()
                    return .terminateCancel
                }
                return .terminateNow
            }
            guard let install = heldInstall else { return .terminateNow }
            guard updateShutdownTask == nil, let node, let model,
                  UpdateWindow.decide(updateMoment(node: node, model: model)) == .installNow else { return .terminateCancel }
            beginUpdateShutdown(install: install, quit: true)
            return .terminateLater
        }
    }

    @MainActor private func beginUpdateShutdown(install: @escaping () -> Void, quit: Bool) {
        guard updateShutdownTask == nil, let node else { return }
        let version = heldVersion
        let id = UUID()
        updateShutdownID = id
        updateShutdownTask = Task { @MainActor [weak self, weak node] in
            guard let self, let node else { return }
            let prepared = await node.prepareForUpdate()
            guard self.updateShutdownID == id else {
                if quit { NSApp.reply(toApplicationShouldTerminate: false) }
                return
            }
            self.updateShutdownTask = nil
            self.updateShutdownID = nil
            guard prepared, !Task.isCancelled, self.heldInstall != nil, self.heldVersion == version,
                  let model = self.model,
                  UpdateWindow.decide(self.updateMoment(node: node, model: model)) == .installNow else {
                node.abortUpdatePreparation()
                if quit { NSApp.reply(toApplicationShouldTerminate: false) }
                return
            }
            // run.lock remains held, with CLOEXEC, until this process dies.
            // The root stub is suppressed and the app cannot start a writer.
            self.updateShutdownReady = true
            self.heldInstall = nil
            self.heldReason = nil
            self.tracker.installing()
            self.syncUpdateNotice()
            if quit { NSApp.reply(toApplicationShouldTerminate: true) }
            else { install() }
        }
    }

    @MainActor private func syncUpdateNotice() {
        let notice = tracker.sentence
        if updates.installNotice != notice { updates.installNotice = notice }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
}

extension AppDelegate: SPUUpdaterDelegate {
    /// The canary ring (docs/design/32-health-signal.md §5.2): channel items
    /// are offered only to a Mac set to `updateChannel = canary`. The release
    /// gate below applies to them unchanged.
    func allowedChannels(for updater: SPUUpdater) -> Set<String> {
        UpdateChannel.allowedChannels()
    }

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
            userInfo: [NSLocalizedDescriptionKey: String(localized: "This update is not approved by the network yet.")])
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
            self.updateShutdownTask?.cancel()
            self.updateShutdownTask = nil
            self.updateShutdownID = nil
            self.updateShutdownReady = false
            self.node?.abortUpdatePreparation()
            self.tracker.aborted(networkError: ns.domain == NSURLErrorDomain
                || ns.underlyingErrors.contains { ($0 as? NSError)?.domain == NSURLErrorDomain })
            self.syncUpdateNotice()
        }
    }

    /// Sparkle installs a downloaded update when the app quits, but EastSea
    /// stays in the menu bar with its node for days. Take the install over
    /// (return true: no Sparkle reminder or prompt) and run it at the first
    /// safe moment (`UpdateWindow`): the app relaunches on the new version and
    /// the node restarts with it. The gate (`shouldProceedWithUpdate`) and
    /// Sparkle's EdDSA check already passed for this item; nothing here
    /// weakens them. If no safe moment comes, Sparkle still installs at quit.
    func updater(_ updater: SPUUpdater, willInstallUpdateOnQuit item: SUAppcastItem, immediateInstallationBlock: @escaping () -> Void) -> Bool {
        MainActor.assumeIsolated {
            tracker.verified()
            syncUpdateNotice()
            heldVersion = "\((item.displayVersionString as String?) ?? "") (\(item.versionString))"
            Self.updateLog.notice("update downloaded: \(self.heldVersion, privacy: .public), verified")
            heldInstall = immediateInstallationBlock
            heldReason = nil
            installIfSafe()
        }
        return true
    }
}
#endif
