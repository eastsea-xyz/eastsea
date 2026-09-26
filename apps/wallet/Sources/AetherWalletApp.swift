import SwiftUI

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
            CommandGroup(after: .appSettings) {
                Button("Install Command-Line Tools…") { CommandLineTools.install() }
            }
        }
        #endif
    }
}

#if os(macOS)
/// Quitting the app stops its node: nothing keeps running in the background.
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var node: NodeController?

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
