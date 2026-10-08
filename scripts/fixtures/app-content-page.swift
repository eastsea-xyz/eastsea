import Foundation
import AppKit
import WebKit

/// Test-only loopback RPC adapter. SeaRegistryReader is the wallet's actual
/// resolver; compiling NodeController here would also pull in signing/UI code.
enum LocalRPC {
    static func call(port: UInt16, method: String, params: [Any]) async -> Any? {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:\(port)/")!)
        request.httpMethod = "POST"
        request.timeoutInterval = 10
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try? JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "id": 1, "method": method, "params": params])
        guard let (data, response) = try? await URLSession.shared.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200, data.count < 1_024 * 1_024,
              let envelope = try? JSONSerialization.jsonObject(with: data) as? [String: Any], envelope["error"] == nil else { return nil }
        return envelope["result"]
    }
}

struct AppPageFixture: Decodable {
    let rpcURL: URL
    let chainID: UInt64
    let link: String
    let namesAddress: String
    let namesRuntimeSHA256: String
    let registryAddress: String
    let registryRuntimeSHA256: String
    let appID: String
    let bundleHash: String
    let expectedMarker: String
}

enum FixtureFailure: Error { case assertion(String) }

@MainActor
final class AppPageCheck: NSObject, NSApplicationDelegate, WKNavigationDelegate, WKScriptMessageHandlerWithReply {
    private var view: WKWebView?
    private var window: NSWindow?
    private var identity: AppBrowserIdentity?
    private var chainID: UInt64 = 0
    private var navigationReply: CheckedContinuation<Void, Error>?
    private var bridgeReplies = 0

    func applicationDidFinishLaunching(_ notification: Notification) {
        Task { @MainActor in
            do { try await check(); print("OK app-content devnet: registered name → iroh → SHA-256 → private WK page → EIP-1193"); exit(0) }
            catch { fputs("FAIL app-content devnet page: \(error)\n", stderr); exit(1) }
        }
    }

    private func require(_ value: Bool, _ explanation: String) throws {
        if !value { throw FixtureFailure.assertion(explanation) }
    }

    private func check() async throws {
        guard CommandLine.arguments.count == 3, CommandLine.arguments[1] == "--fixture" else {
            throw FixtureFailure.assertion("expected --fixture <owned-devnet.json>")
        }
        let fixtureURL = URL(fileURLWithPath: CommandLine.arguments[2])
        let fixture = try JSONDecoder().decode(AppPageFixture.self, from: Data(contentsOf: fixtureURL))
        chainID = fixture.chainID
        guard case .name(let link) = try SeaURL.parse(fixture.link, chainID: chainID),
              let port = fixture.rpcURL.port, let nodePort = UInt16(exactly: port) else {
            throw FixtureFailure.assertion("invalid fixture location")
        }
        let pins = SeaRegistrySources(names: .init(address: fixture.namesAddress, codeSHA256: fixture.namesRuntimeSHA256),
                                      apps: .init(address: fixture.registryAddress, codeSHA256: fixture.registryRuntimeSHA256))
        let resolved = try await SeaRegistryReader.resolve(link, chainID: chainID, port: nodePort, sources: pins)
        try require(resolved.app.appID == fixture.appID, "on-chain name must select the published app")
        try require(AppBundleHash.normalized(resolved.app.bundleHash) == fixture.bundleHash, "registry must authorise the transferred bundle")
        let source = try await NodeAppContentSource(app: resolved.app, endpoint: fixture.rpcURL)
        try require(try await source.page(for: resolved.app, at: link) != nil && source.bundle.isVerified, "ContentSource must supply verified content")
        let identity = try AppBrowserIdentity(appID: resolved.app.appID, name: link.name, chainID: chainID, registry: pins.apps.address)
        self.identity = identity
        try require(identity.displayOrigin == fixture.link, "native origin must retain the real sea name")
        let config = WKWebViewConfiguration()
        let scheme = try AppBundleScheme(bundle: source.bundle, appKey: identity.appKey)
        do {
            _ = try source.bundle.pageURL(appKey: identity.appKey, path: "/asset.svg")
            throw FixtureFailure.assertion("active SVG entered the document navigation path")
        } catch AppContentError.invalidPath {}
        try require(source.bundle.documentPath(for: URL(string: "eastsea-app://\(identity.appKey)/asset.svg")!, appKey: identity.appKey) == nil,
                    "the same wallet navigation policy must refuse active XML")
        config.setURLSchemeHandler(scheme, forURLScheme: AppBrowserIdentity.scheme)
        config.websiteDataStore = .nonPersistent()
        let world = WKContentWorld.world(name: AppProviderBridge.worldName)
        let ucc = config.userContentController
        ucc.addScriptMessageHandler(self, contentWorld: world, name: "aether")
        ucc.addUserScript(WKUserScript(source: AppProviderBridge.relay, injectionTime: .atDocumentStart, forMainFrameOnly: true, in: world))
        let provider = try String(contentsOfFile: "apps/wallet/Resources/provider.js", encoding: .utf8)
        ucc.addUserScript(WKUserScript(source: AppProviderBridge.facade(provider: provider), injectionTime: .atDocumentStart, forMainFrameOnly: true))
        let view = WKWebView(frame: NSRect(x: 0, y: 0, width: 640, height: 480), configuration: config)
        self.view = view
        view.navigationDelegate = self
        window = NSWindow(contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window?.contentView = view
        try await withCheckedThrowingContinuation { continuation in
            navigationReply = continuation
            view.load(URLRequest(url: scheme.entryURL))
        }
        // The page's provider request is asynchronous after the document loads.
        var state: [String: Any] = [:]
        for _ in 0..<150 {
            state = (try await view.evaluateJavaScript("({marker:document.body.dataset.marker,module:document.body.dataset.module,chainId:document.body.dataset.chainId,privateHandler:!!(window.webkit&&window.webkit.messageHandlers&&window.webkit.messageHandlers.aether),inlineViolation:window.inlineViolation===true})")) as? [String: Any] ?? [:]
            if state["chainId"] as? String != nil && state["module"] as? String != nil { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        try require(state["marker"] as? String == fixture.expectedMarker, "verified app.js must execute")
        try require(state["module"] as? String == "module-loaded", "same-app JavaScript modules and imports must load with strict CSP")
        try require(state["chainId"] as? String == "0x\(String(chainID, radix: 16))", "EIP-1193 must reach the native bridge")
        try require(bridgeReplies == 1, "the native bridge must authenticate exactly one main-frame request")
        try require(state["privateHandler"] as? Bool == false, "the page world must not expose the privileged handler")
        try require(state["inlineViolation"] as? Bool == false, "CSP must reject inline publisher code")
        let count = try await view.evaluateJavaScript("document.getElementById('increment').click(); document.getElementById('count').textContent")
        try require(count as? String == "1", "the loaded app must respond to a user interaction")
        if let image = try? await view.takeSnapshot(configuration: nil), let tiff = image.tiffRepresentation, let bitmap = NSBitmapImageRep(data: tiff),
           let png = bitmap.representation(using: .png, properties: [:]) {
            let evidence = URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent("tmp/app-content/devnet-page.png")
            try png.write(to: evidence)
        }
        // An unverified snapshot must be refused by the ordinary scheme path.
        let unverified = try LocalAppFolder.load(from: fixtureURL.deletingLastPathComponent().appendingPathComponent("app"), developerModeEnabled: true)
        do {
            _ = try AppBundleScheme(bundle: unverified, appKey: identity.appKey)
            throw FixtureFailure.assertion("unverified local files entered the ordinary scheme")
        } catch AppContentError.unverifiedContent {}
    }

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage,
                               replyHandler: @escaping @MainActor @Sendable (Any?, String?) -> Void) {
        let origin = message.frameInfo.securityOrigin
        guard message.webView === view, let identity,
              identity.accepts(scheme: origin.protocol, host: origin.host, port: Int(origin.port),
                               mainFrame: message.frameInfo.isMainFrame, currentChainID: chainID, developerMode: false),
              let body = message.body as? [String: Any], body["method"] as? String == "eth_chainId" else {
            replyHandler(["error": ["code": 4100, "message": "Unexpected fixture origin or method"]], nil)
            return
        }
        bridgeReplies += 1
        replyHandler(["result": "0x\(String(chainID, radix: 16))"], nil)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        navigationReply?.resume(); navigationReply = nil
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        navigationReply?.resume(throwing: error); navigationReply = nil
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        navigationReply?.resume(throwing: error); navigationReply = nil
    }
}

@main
struct AppPageHarness {
    static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.prohibited)
        let check = AppPageCheck()
        app.delegate = check
        withExtendedLifetime(check) { app.run() }
    }
}
