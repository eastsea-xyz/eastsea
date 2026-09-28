import SwiftUI
#if os(macOS)
import AppKit
#endif

// Window resizing redraws the whole tree many times a second. The canvases that
// animate continuously (the earnings aurora, the orbit spinner, the shimmer) would
// redraw their blurs on top of every one of those passes, so they pause instead:
// frozen in place for the length of the drag, resumed when it ends. Reduce Motion
// still wins over everything.

private struct LiveResizeKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// True while the user is dragging a window's edge or corner (macOS; always
    /// false on iPhone). Continuous animations read this and pause.
    var liveResize: Bool {
        get { self[LiveResizeKey.self] }
        set { self[LiveResizeKey.self] = newValue }
    }
}

#if os(macOS)
/// Reports live resizes of any window of the app (NSWindow notifications), so views
/// deeper down can pause heavy animation without knowing about windows.
@MainActor
final class WindowResizeMonitor: ObservableObject {
    @Published private(set) var active = false
    private var observers: [NSObjectProtocol] = []

    init() {
        let center = NotificationCenter.default
        observers = [
            center.addObserver(forName: NSWindow.willStartLiveResizeNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.begin() }
            },
            center.addObserver(forName: NSWindow.didEndLiveResizeNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.end() }
            },
            // A window closing mid-drag would otherwise leave `active` stuck on.
            center.addObserver(forName: NSWindow.willCloseNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.end() }
            },
        ]
    }

    deinit { observers.forEach(NotificationCenter.default.removeObserver) }

    func begin() { active = true }
    func end() { active = false }
}
#endif
