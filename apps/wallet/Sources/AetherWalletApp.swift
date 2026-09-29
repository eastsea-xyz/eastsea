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
    @StateObject private var earnings = Earnings()
    /// Views pause their continuous animations while a window is live-resizing.
    @StateObject private var resizeMonitor = WindowResizeMonitor()
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @AppStorage("developerMode") private var developerMode = false
    #endif

    var body: some Scene {
        WindowGroup("Aether", id: "main") {
            #if os(macOS)
            ContentView()
                .environmentObject(model)
                .environmentObject(node)
                .environmentObject(appDelegate.updates)
                .environmentObject(earnings)
                .onAppear {
                    appDelegate.start(node: node, model: model)
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
            SettingsView().environmentObject(node).environmentObject(model)
        }
        // Always in the menu bar: balance, node and prover at a glance; the window opens from here.
        MenuBarExtra {
            MenuBarPanel().environmentObject(model).environmentObject(node).environmentObject(earnings)
                .onAppear {
                    appDelegate.start(node: node, model: model)
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
    @AppStorage("developerMode") private var developerMode = false

    var body: some View {
        Form {
            Toggle("Run a node on this Mac", isOn: $node.enabled)
            Toggle("Only while on the power adapter", isOn: $node.onlyOnPower)
                .help("On a laptop, pause the node on battery and resume on power.")
            Label(node.awakeNote, systemImage: node.keepsAwake ? "sun.max.fill" : "moon.zzz")
                .font(.caption).foregroundStyle(.secondary)
            Toggle("Open Aether at login", isOn: Binding(get: { node.startAtLogin }, set: { node.startAtLogin = $0 }))
            ResourcesSection()
            Text("Your node verifies every block itself and your wallet asks it instead of the network. Quitting Aether stops it.")
                .font(.caption).foregroundStyle(.secondary)
            // Honest power ranges (docs/research/mac-power-cost-2026.md): the node is
            // cheap; GPU proving is the costly part. No won figure — electricity
            // prices vary, and the range is the honest statement.
            Text("Power: roughly 5–6 W while only verifying (about 4 kWh a month); proving on the GPU adds roughly 28–50 W (about 20–36 kWh a month).")
                .font(.caption).foregroundStyle(.secondary)
            Divider()
            Toggle("Developer mode (proofs, state roots, raw logs)", isOn: $developerMode)
                .help("Also in View ▸ Developer Mode (⇧⌘D)")
        }
        .padding(20)
        .frame(width: 420)
    }
}

/// Aether lives in the menu bar: closing the window keeps it (and its node)
/// running; Quit stops both.
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var node: NodeController?
    /// Sparkle: checks the signed appcast on GitHub Releases and installs updates.
    lazy var updater = SPUStandardUpdaterController(startingUpdater: true, updaterDelegate: self, userDriverDelegate: nil)
    /// What the Network page shows: when updates were last checked, and a Check button.
    @MainActor lazy var updates = Updates(updater)
    private var started = false
    /// Tells a validator Mac when the network pauses and resumes.
    private var pauseWatch: AnyCancellable?

    func applicationDidFinishLaunching(_ notification: Notification) {
        #if DEBUG
        if DesignPreview.on { return }
        #endif
        _ = updater  // start checking right away (hourly, and when the chain schedules a newer protocol)
    }

    /// Once per launch, from whichever appears first (window or menu-bar panel).
    @MainActor func start(node: NodeController, model: WalletModel) {
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
        let check: () -> Void = { [weak self] in self?.updater.updater.checkForUpdatesInBackground() }
        node.onUpgradeNeeded = check
        model.onOutdated = check
        pauseWatch = model.$chainPausedSince
            .removeDuplicates { ($0 == nil) == ($1 == nil) }
            .sink { [weak node] since in
                MainActor.assumeIsolated { node?.networkPaused(since: since) }
            }
        // Open at login by default (Settings can turn it off).
        if !UserDefaults.standard.bool(forKey: "loginItemDefaultApplied") {
            UserDefaults.standard.set(true, forKey: "loginItemDefaultApplied")
            node.startAtLogin = true
        }
        node.restore()
    }

    func applicationWillTerminate(_ notification: Notification) {
        MainActor.assumeIsolated { node?.stop() }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
}

/// Link the bundled `aether` and `aether-agent` into ~/.local/bin.
enum CommandLineTools {
    static func install() {
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
    /// Sparkle installs a downloaded update when the app quits, but Aether stays
    /// in the menu bar with its node for days. Install now instead: the app
    /// relaunches on the new version and the node restarts with it, so a Mac
    /// that never quits still follows protocol upgrades.
    func updater(_ updater: SPUUpdater, willInstallUpdateOnQuit item: SUAppcastItem, immediateInstallationBlock: @escaping () -> Void) -> Bool {
        immediateInstallationBlock()
        return true
    }
}
#endif
