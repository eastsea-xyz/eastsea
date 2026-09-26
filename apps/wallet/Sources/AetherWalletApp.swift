import SwiftUI
#if os(macOS)
import Sparkle
#endif

@main
struct AetherWalletApp: App {
    @StateObject private var model = WalletModel()
    #if os(macOS)
    @StateObject private var node = NodeController()
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    #endif

    var body: some Scene {
        WindowGroup("Aether") {
            #if os(macOS)
            ContentView()
                .environmentObject(model)
                .environmentObject(node)
                .onAppear {
                    appDelegate.node = node
                    node.restore()
                }
            #else
            ContentView().environmentObject(model)
            #endif
        }
        #if os(macOS)
        .windowResizability(.contentSize)
        .commands {
            CommandGroup(after: .appInfo) {
                Button("Check for Updates…") { appDelegate.updater.checkForUpdates(nil) }
            }
            CommandGroup(after: .appSettings) {
                Button("Install Command-Line Tools…") { CommandLineTools.install() }
            }
        }
        #endif
        #if os(macOS)
        Settings {
            SettingsView().environmentObject(node)
        }
        #endif
    }
}

#if os(macOS)
/// Aether ▸ Settings: how the node runs on this Mac.
struct SettingsView: View {
    @EnvironmentObject var node: NodeController

    var body: some View {
        Form {
            Toggle("Run a node on this Mac", isOn: $node.enabled)
            Toggle("Only while on the power adapter", isOn: $node.onlyOnPower)
                .help("On a laptop, pause the node on battery and resume on power.")
            Toggle("Open Aether at login", isOn: Binding(get: { node.startAtLogin }, set: { node.startAtLogin = $0 }))
            Text("Your node verifies every block itself and your wallet asks it instead of the network. Quitting Aether stops it.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .padding(20)
        .frame(width: 420)
    }
}

/// Quitting the app stops its node: nothing keeps running in the background.
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var node: NodeController?
    /// Sparkle: checks the signed appcast on GitHub Releases and installs updates.
    let updater = SPUStandardUpdaterController(startingUpdater: true, updaterDelegate: nil, userDriverDelegate: nil)

    func applicationWillTerminate(_ notification: Notification) {
        MainActor.assumeIsolated { node?.stop() }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
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
#endif
