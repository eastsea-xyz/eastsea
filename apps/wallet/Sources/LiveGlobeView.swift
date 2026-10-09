#if os(macOS)
import AppKit
import SwiftUI
import WebKit

/// The canonical local globe. The native wallet is the only presence reader;
/// WebKit receives aggregate JSON and display preferences as named arguments.
struct LiveGlobeView: View {
    var searchHome = false
    @EnvironmentObject private var model: WalletModel
    @EnvironmentObject private var node: NodeController
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var height: CGFloat = 680
    @State private var loadFailed = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !searchHome {
            LabeledContent("This Mac's local sub-region", value: node.presenceLocalRegionLabel)
                .font(.aeCaption)
            if let country = node.presenceSelectedCountryLabel {
                LabeledContent("Selected country", value: country).font(.aeCaption)
            }
            Text("Local preference only; this Mac is not added to published counts.")
                .font(.aeCaption).foregroundStyle(.secondary)
            }
            globeContent
        }
    }

    private var globeContent: some View {
        LiveGlobeWebContent(presence: model.liveGlobePresence,
                            state: model.liveGlobeState.rawValue,
                            dark: colorScheme == .dark, reduceMotion: reduceMotion, searchHome: searchHome,
                            height: $height, loadFailed: $loadFailed)
            .frame(height: height)
            .overlay {
                if model.liveGlobeState == .withheld {
                    Text("Counts withheld for privacy")
                        .font(.aeBody).foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .background(.background)
                } else if loadFailed {
                    Text("Live network unavailable")
                        .font(.aeBody).foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .background(.background)
                }
            }
    }
}

private struct LiveGlobeWebContent: NSViewRepresentable {
    let presence: LiveGlobePresence?
    let state: String
    let dark: Bool
    let reduceMotion: Bool
    let searchHome: Bool
    @Binding var height: CGFloat
    @Binding var loadFailed: Bool

    func makeCoordinator() -> Coordinator { Coordinator(height: $height, loadFailed: $loadFailed) }

    func makeNSView(context: Context) -> LiveGlobeWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.setURLSchemeHandler(LiveGlobeScheme(), forURLScheme: LiveGlobeBundlePolicy.scheme)
        // This configuration deliberately has no script message handlers.
        let view = LiveGlobeWebView(frame: .zero, configuration: config)
        view.identifier = NSUserInterfaceItemIdentifier("wallet-live-globe")
        view.navigationDelegate = context.coordinator
        view.allowsLinkPreview = false
        view.underPageBackgroundColor = .windowBackgroundColor
        context.coordinator.install(view)
        context.coordinator.update(self)
        view.load(URLRequest(url: LiveGlobeBundlePolicy.entry))
        return view
    }

    func updateNSView(_ view: LiveGlobeWebView, context: Context) {
        context.coordinator.update(self)
    }

    static func dismantleNSView(_ view: LiveGlobeWebView, coordinator: Coordinator) {
        coordinator.stop()
        view.stopLoading()
        view.navigationDelegate = nil
        view.renderStateChanged = nil
    }

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate {
        private weak var view: LiveGlobeWebView?
        private var height: Binding<CGFloat>
        private var loadFailed: Binding<Bool>
        private var presence: LiveGlobePresence?
        private var state = "loading"
        private var dark = false
        private var reduceMotion = false
        private var searchHome = false
        private var loaded = false
        private var stopped = false
        private var bootstrapTask: Task<Void, Never>?
        private var lastSent: LiveGlobeRenderingPolicy.Update?
        private var pushGeneration = 0

        init(height: Binding<CGFloat>, loadFailed: Binding<Bool>) {
            self.height = height
            self.loadFailed = loadFailed
        }

        func install(_ view: LiveGlobeWebView) {
            self.view = view
            view.renderStateChanged = { [weak self] in self?.push() }
            for notification in [
                NSWindow.didChangeOcclusionStateNotification, NSWindow.didMiniaturizeNotification,
                NSWindow.didDeminiaturizeNotification, NSApplication.didHideNotification,
                NSApplication.didUnhideNotification, Notification.Name.NSProcessInfoPowerStateDidChange,
            ] {
                NotificationCenter.default.addObserver(self, selector: #selector(renderStateChanged),
                                                       name: notification, object: nil)
            }
            NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(renderStateChanged),
                name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil)
        }

        func update(_ content: LiveGlobeWebContent) {
            presence = content.presence
            state = content.state
            dark = content.dark
            reduceMotion = content.reduceMotion
            if searchHome != content.searchHome { lastSent = nil }
            searchHome = content.searchHome
            push()
        }

        @objc private func renderStateChanged() { push() }

        private var fixture: Bool {
            #if DEBUG
            DesignPreview.on
            #else
            false
            #endif
        }

        private func push() {
            guard loaded, !stopped, let view else { return }
            let visible = view.window.map { $0.isVisible && !$0.isMiniaturized && $0.occlusionState.contains(.visible) } ?? false
            let update = LiveGlobeRenderingPolicy.Update(
                aggregate: presence?.aggregateJSON, state: state,
                paused: fixture || LiveGlobeRenderingPolicy.paused(windowVisible: visible && !NSApp.isHidden,
                    viewHidden: view.isHiddenOrHasHiddenAncestor, lowPower: ProcessInfo.processInfo.isLowPowerModeEnabled),
                reducedMotion: reduceMotion || NSWorkspace.shared.accessibilityDisplayShouldReduceMotion,
                dark: dark, language: Bundle.main.preferredLocalizations.first ?? "en", fixture: fixture,
                evidenceAvailable: presence?.hasQualityEvidence ?? false, width: view.bounds.width)
            guard update.shouldSend(after: lastSent) else { return }
            lastSent = update
            pushGeneration += 1
            let generation = pushGeneration
            var settings = update.settings
            settings["searchHome"] = searchHome
            view.callAsyncJavaScript("""
                if (!globalThis.eastseaGlobe?.ready) return null;
                globalThis.eastseaGlobe.configure(settings);
                if (aggregate !== null) globalThis.eastseaGlobe.update(JSON.parse(aggregate));
                else globalThis.eastseaGlobe.reset();
                if (state !== 'ready') globalThis.eastseaGlobe.configure({state});
                return globalThis.eastseaGlobe.height();
                """, arguments: ["settings": settings, "aggregate": update.aggregate as Any? ?? NSNull(), "state": state],
                in: nil, in: .page) { [weak self] result in
                    guard let self, !self.stopped, self.pushGeneration == generation else { return }
                    switch result {
                    case .success(let value):
                        guard let number = value as? NSNumber else { self.lastSent = nil; return }
                        let next = CGFloat(number.doubleValue)
                        guard next.isFinite, next >= 40, next <= 6_000 else { return }
                        if abs(self.height.wrappedValue - next) > 1 { self.height.wrappedValue = next }
                    case .failure:
                        self.lastSent = nil
                        self.loadFailed.wrappedValue = true
                    }
                }
        }

        func stop() {
            stopped = true
            bootstrapTask?.cancel()
            bootstrapTask = nil
            NotificationCenter.default.removeObserver(self)
            NSWorkspace.shared.notificationCenter.removeObserver(self)
            view?.callAsyncJavaScript("globalThis.eastseaGlobe?.configure({paused:true});", arguments: [:], in: nil, in: .page)
        }

        func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                     decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            decisionHandler(LiveGlobeBundlePolicy.allowsNavigation(to: navigationAction.request.url,
                mainFrame: navigationAction.targetFrame?.isMainFrame == true) ? .allow : .cancel)
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            bootstrapTask?.cancel()
            // didFinish can precede the bundled ES modules' ready API. A
            // one-shot push then loses the initial aggregate and preferences.
            loaded = false
            bootstrapTask = Task { [weak self, weak webView] in
                for _ in 0..<80 {
                    guard !Task.isCancelled, let self, !self.stopped, let webView else { return }
                    let ready = (try? await webView.evaluateJavaScript("Boolean(globalThis.eastseaGlobe?.ready)")) as? Bool == true
                    guard !Task.isCancelled, !self.stopped else { return }
                    if ready {
                        self.loaded = true
                        self.lastSent = nil
                        self.loadFailed.wrappedValue = false
                        self.push()
                        self.bootstrapTask = nil
                        return
                    }
                    do { try await Task.sleep(nanoseconds: 50_000_000) }
                    catch { return }
                }
                self?.loadFailed.wrappedValue = true
                self?.bootstrapTask = nil
            }
        }

        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
            loadFailed.wrappedValue = true
        }

        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
            loadFailed.wrappedValue = true
        }

        func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
            bootstrapTask?.cancel()
            bootstrapTask = nil
            loaded = false
            lastSent = nil
            webView.load(URLRequest(url: LiveGlobeBundlePolicy.entry))
        }
    }
}

private final class LiveGlobeScheme: NSObject, WKURLSchemeHandler {
    private let root = Bundle.main.resourceURL?.appendingPathComponent("LiveGlobe", isDirectory: true)

    func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        guard let url = task.request.url, let path = LiveGlobeBundlePolicy.path(for: url), let root else {
            task.didFailWithError(URLError(.badURL))
            return
        }
        let file = root.appendingPathComponent(path).resolvingSymlinksInPath()
        guard file.path.hasPrefix(root.resolvingSymlinksInPath().path + "/"),
              let data = try? Data(contentsOf: file),
              let response = HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: [
                "Content-Type": LiveGlobeBundlePolicy.mimeType(for: path),
                "Content-Security-Policy": LiveGlobeBundlePolicy.contentSecurityPolicy,
                "X-Content-Type-Options": "nosniff",
                "Cache-Control": "no-store",
              ]) else {
            task.didFailWithError(URLError(.fileDoesNotExist))
            return
        }
        task.didReceive(response)
        task.didReceive(data)
        task.didFinish()
    }

    func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}
}

final class LiveGlobeWebView: WKWebView {
    var renderStateChanged: (() -> Void)?
    override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); renderStateChanged?() }
    override func viewDidHide() { super.viewDidHide(); renderStateChanged?() }
    override func viewDidUnhide() { super.viewDidUnhide(); renderStateChanged?() }
    override func setFrameSize(_ newSize: NSSize) {
        let changed = abs(frame.width - newSize.width) > 1
        super.setFrameSize(newSize)
        if changed { renderStateChanged?() }
    }
}
#endif
