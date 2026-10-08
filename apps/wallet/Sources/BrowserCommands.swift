#if os(macOS)
import AppKit
import SwiftUI

struct BrowserActions {
    let session: BrowserSession
    let focusAddress: () -> Void
    let showFind: () -> Void
}

private struct BrowserActionsKey: FocusedValueKey { typealias Value = BrowserActions }
extension FocusedValues {
    var browserActions: BrowserActions? {
        get { self[BrowserActionsKey.self] }
        set { self[BrowserActionsKey.self] = newValue }
    }
}

/// Shortcuts belong to the active Explore page, across every native tab.
struct BrowserCommands: Commands {
    @FocusedValue(\.browserActions) private var actions
    var body: some Commands {
        CommandMenu("Browser") {
            Group {
            Button("Focus address bar") { actions?.focusAddress() }.keyboardShortcut("l")
            Button("New tab") { actions?.session.addTab() }.keyboardShortcut("t")
            Button("New private tab") { actions?.session.addTab(isPrivate: true) }.keyboardShortcut("n", modifiers: [.command, .shift])
            Button("Close tab") { actions?.session.closeTab() }.keyboardShortcut("w")
            Divider()
            Button("Back") { actions?.session.controller.goBack() }.keyboardShortcut("[")
                .disabled(actions?.session.controller.canGoBack != true)
            Button("Forward") { actions?.session.controller.goForward() }.keyboardShortcut("]")
                .disabled(actions?.session.controller.canGoForward != true)
            Button("Reload") { actions?.session.controller.reload() }.keyboardShortcut("r")
            Button("Stop loading") { actions?.session.controller.stop() }.keyboardShortcut(".")
            Divider()
            Button("Find in page") { actions?.showFind() }.keyboardShortcut("f")
            Button("Zoom in") { actions?.session.controller.zoomIn() }.keyboardShortcut("+")
            Button("Zoom out") { actions?.session.controller.zoomOut() }.keyboardShortcut("-")
            Button("Reset zoom") { actions?.session.controller.resetZoom() }.keyboardShortcut("0")
            }.disabled(actions == nil)
        }
    }
}

/// File > Close is a system command. Consume its shortcut only while this
/// page belongs to the event's key window, so it closes a tab first.
struct BrowserKeyHandler: NSViewRepresentable {
    var closeTab: () -> Void
    func makeCoordinator() -> Coordinator { Coordinator(closeTab: closeTab) }
    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        context.coordinator.view = view
        context.coordinator.install()
        return view
    }
    func updateNSView(_ nsView: NSView, context: Context) { context.coordinator.closeTab = closeTab }
    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) { coordinator.uninstall() }

    final class Coordinator {
        weak var view: NSView?
        var closeTab: () -> Void
        private var monitor: Any?
        init(closeTab: @escaping () -> Void) { self.closeTab = closeTab }
        func install() {
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                MainActor.assumeIsolated {
                    guard let self, let window = self.view?.window, event.window === window,
                          window.isKeyWindow, event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command,
                          event.charactersIgnoringModifiers?.lowercased() == "w" else { return event }
                    self.closeTab()
                    return nil
                }
            }
        }
        func uninstall() { if let monitor { NSEvent.removeMonitor(monitor) }; monitor = nil }
        deinit { uninstall() }
    }
}
#endif
