import Foundation
import WebKit
#if os(macOS)
import AppKit
#endif

/// One Explore tab: the WKWebView, the warnings that gate navigation, and
/// the message bridge that answers `window.aether` (Resources/provider.js —
/// the extension's provider surface, run by the wallet instead of a content
/// script). Every rule that can be pure lives in BrowserPolicy and
/// BrowserOriginPolicy; this class is the WebKit half (docs/design/09-wallet.md
/// "인앱 브라우저").
@MainActor
final class BrowserController: NSObject, ObservableObject {
    /// What the address bar shows (kept in sync on navigation).
    @Published var addressField = ""
    /// The one-line refusal under the address bar (why a URL did not load).
    @Published var notice: String?
    /// The one-time warning sheet for an external site.
    @Published var warning: SiteWarning?
    /// The confirmation sheet a page's request opened (one at a time).
    @Published var ask: PendingAsk?
    /// Whether the last read this tab answered was certificate-verified.
    @Published private(set) var lastReadVerified = true
    /// Bumped whenever the WebView had to be rebuilt (a store switch).
    @Published private(set) var webViewGeneration = 0
    @Published private(set) var canGoBack = false
    /// nil until something was loaded: while nil the curated home shows.
    @Published private(set) var currentURL: URL?
    @Published private(set) var appIdentity: AppBrowserIdentity?
    @Published private(set) var contentLoading = false
    private var appBundle: AppBundle?
    private var appViewKey: String?
    private var contentTask: Task<Void, Never>?
    private var navigationID = UUID()

    struct SiteWarning: Identifiable {
        let id = UUID()
        let url: URL
        let host: String
        /// The curated domain this host imitates, for a louder warning.
        let lookalike: String?
        let punycode: Bool
    }

    /// A page's request waiting on the native sheet: what to show, and the
    /// bridge reply to hand back when the user decides.
    struct PendingAsk: Identifiable {
        enum Kind {
            /// eth_requestAccounts: show the address this site would see.
            case connect(origin: String, host: String)
            /// eth_sendTransaction: origin, the parsed transaction, and the
            /// fee snapshot the sheet displays (nil: the status maximum).
            case send(origin: String, host: String, tx: PageTransaction, feeWei: String?)
        }
        let id: String
        let kind: Kind
        let navigationID: UUID
        let account: String
        let chainID: UInt64
        let reply: (Result<Any?, ProviderError>) -> Void
    }

    private(set) weak var model: WalletModel?
    /// The tab's current WebView (rebuilt when the data store has to change).
    @Published private(set) var webView: WKWebView?
    /// The external host the current non-persistent WebView belongs to.
    private var externalHost: String?
    private var askQueue: [PendingAsk] = []
    /// The URL a just-acknowledged warning may load (one shot, so the same
    /// warning cannot loop).
    private var approvedURL: URL?

    private static let acknowledgedKey = "explore.acknowledged"
    private var acknowledged: Set<String> {
        get { Set(UserDefaults.standard.stringArray(forKey: Self.acknowledgedKey) ?? []) }
        set { UserDefaults.standard.set(Array(newValue), forKey: Self.acknowledgedKey) }
    }

    /// Wire the wallet state the bridge answers from (called once, at attach).
    func attach(model: WalletModel) {
        self.model = model
    }

    // MARK: - WebView lifecycle

    /// The WebView, built for where the tab is going: bundled pages share the
    /// app's own store; an external site gets a fresh non-persistent one each
    /// time the host changes, so no site's cookies or storage meet another's.
    func webViewFor(url: URL?) -> WKWebView {
        let classification = url.map(BrowserOriginPolicy.classify)
        let wantApp = url?.scheme == AppBrowserIdentity.scheme ? appIdentity : nil
        let wantAppKey = wantApp.map { "\($0.permissionKey)/\(appBundle?.bundleHash ?? "")" }
        let wantExternal: String?
        if case .external(let host, _, _)? = classification { wantExternal = host } else { wantExternal = nil }

        if let view = webView {
            if let key = wantAppKey, key == appViewKey { return view }
            if wantAppKey == nil && appViewKey == nil {
                if wantExternal == nil && externalHost == nil { return view }
                if let host = wantExternal, let current = externalHost, host == current { return view }
            }
        }
        let config = WKWebViewConfiguration()
        config.setURLSchemeHandler(BundledPageScheme(root: BundledPageScheme.defaultRoot()
            ?? URL(fileURLWithPath: "/nonexistent")), forURLScheme: BrowserOriginPolicy.bundledScheme)
        if let identity = wantApp, let bundle = appBundle,
           let scheme = try? AppBundleScheme(bundle: bundle, appKey: identity.appKey,
                                            allowUnverifiedDeveloperContent: identity.isDeveloper) {
            config.setURLSchemeHandler(scheme, forURLScheme: AppBrowserIdentity.scheme)
            config.websiteDataStore = WKWebsiteDataStore(forIdentifier: identity.storeID)
        }
        let ucc = config.userContentController
        let world = wantApp == nil ? WKContentWorld.page : WKContentWorld.world(name: AppProviderBridge.worldName)
        if let providerURL = Bundle.main.url(forResource: "provider", withExtension: "js"),
           let provider = try? String(contentsOf: providerURL, encoding: .utf8) {
            if wantApp != nil {
                ucc.addUserScript(WKUserScript(source: AppProviderBridge.relay, injectionTime: .atDocumentStart,
                                               forMainFrameOnly: true, in: world))
            }
            ucc.addUserScript(WKUserScript(source: wantApp == nil ? provider : AppProviderBridge.facade(provider: provider), injectionTime: .atDocumentStart,
                                           forMainFrameOnly: true))
        }
        // The bundled explorer reads the app's own node: point its saved
        // endpoint at whatever port this Mac's node is on before it boots.
        if wantApp == nil, url?.scheme == BrowserOriginPolicy.bundledScheme, let port = model?.nodeRpcPort {
            ucc.addUserScript(WKUserScript(source: Self.explorerBootstrap(port: port),
                                           injectionTime: .atDocumentStart, forMainFrameOnly: true))
        }
        ucc.addScriptMessageHandler(WeakReplyBridge(controller: self, route: Self.providerRoute), contentWorld: world, name: "aether")
        ucc.addScriptMessageHandler(WeakReplyBridge(controller: self, route: Self.verifyRoute), contentWorld: world, name: "eastsea")
        if let host = wantExternal {
            config.websiteDataStore = .nonPersistent()
            externalHost = host
        } else {
            externalHost = nil
        }
        appViewKey = wantAppKey
        let view = WKWebView(frame: .zero, configuration: config)
        view.navigationDelegate = self
        view.uiDelegate = self
        webView = view
        webViewGeneration += 1
        return view
    }

    /// The bootstrap the bundled explorer runs before its own script: its
    /// node endpoint is always this app's node (the page never had a choice
    /// the user made — this is the whole point of bundling it).
    private static func explorerBootstrap(port: UInt16) -> String {
        """
        try { localStorage.setItem('aether-explorer.node', 'http://127.0.0.1:\(port)') } catch (e) {}
        """
    }

    // MARK: - Navigation

    /// sea:// names and https pages share the native address bar.
    func open(_ text: String) {
        do {
            switch try SeaURL.browserInput(text, chainID: model?.networkChainId ?? Brand.networkChainId) {
            case .name(let link): openName(link)
            case .web(let url): load(url)
            case .action(_, let raw):
                model?.open(link: raw)
            }
        } catch {
            notice = SeaNameText.message(error)
        }
    }

    private func openName(_ link: SeaURL.NameLink) {
        contentTask?.cancel()
        cancelPendingRequests()
        let requestID = navigationID
        let chain = model?.networkChainId ?? Brand.networkChainId
        let port = model?.nodeRpcPort ?? 18545
        let pins = SeaRegistrySources.bundled(chainID: chain)
        webView?.stopLoading()
        webView = nil
        appViewKey = nil
        appIdentity = nil
        appBundle = nil
        notice = nil
        currentURL = URL(string: link.canonicalURL)
        addressField = link.canonicalURL
        contentLoading = true
        contentTask = Task { [weak self] in
            guard let self else { return }
            do {
                let record = try await SeaRegistryReader.resolve(link, chainID: chain, port: port, sources: pins)
                let source = try await NodeAppContentSource(app: record.app, endpoint: URL(string: "http://127.0.0.1:\(port)/")!)
                guard try await source.page(for: record.app, at: link) != nil else { throw AppContentError.unverifiedContent }
                // A release/name change during transfer cannot quietly open a
                // bundle that is no longer the name's active release.
                let latest = try await SeaRegistryReader.resolve(link, chainID: chain, port: port, sources: pins)
                guard latest == record else { throw SeaNameResolver.Failure.unstable }
                try Task.checkCancellation()
                guard self.navigationID == requestID, self.model?.networkChainId == chain,
                      let registry = pins?.apps.address else { return }
                let identity = try AppBrowserIdentity(appID: record.app.appID, name: link.name, chainID: chain, registry: registry)
                self.contentLoading = false
                try self.openAppBundle(source.bundle, identity: identity, path: link.path, query: link.query)
            } catch is CancellationError {
                return
            } catch {
                guard self.navigationID == requestID else { return }
                self.contentLoading = false
                self.notice = error is AppContentError ? error.localizedDescription : SeaNameText.message(error)
            }
        }
    }

    /// Load a URL through the same rules a link goes through.
    func load(_ url: URL) {
        if ["sea", "eastsea"].contains(url.scheme?.lowercased() ?? "") {
            open(url.absoluteString)
            return
        }
        cancelPendingRequests()
        contentTask?.cancel()
        contentLoading = false
        notice = nil
        if url.scheme == AppBrowserIdentity.scheme {
            guard let identity = appIdentity, url.host == identity.appKey else {
                notice = String(localized: "This app has not been verified. Open its sea:// name first.")
                return
            }
            webViewFor(url: url).load(URLRequest(url: url))
            return
        }
        switch BrowserOriginPolicy.classify(url) {
        case .bundled, .external:
            appIdentity = nil
            appBundle = nil
            webViewFor(url: url).load(URLRequest(url: url))
        case .blocked(let why):
            notice = why
        }
    }

    /// The curated home's explorer entry.
    func openExplorer() {
        load(URL(string: "\(BrowserOriginPolicy.bundledScheme)://explorer/index.html")!)
    }

    func goBack() {
        cancelPendingRequests()
        webView?.goBack()
    }

    /// Install only an immutable fully checked bundle (or an explicitly
    /// selected developer folder). The app never receives a filesystem URL.
    func openAppBundle(_ bundle: AppBundle, identity: AppBrowserIdentity, path: String = "/", query: String? = nil) throws {
        guard bundle.isVerified || (identity.isDeveloper && developerModeEnabled) else {
            throw AppBrowserIdentity.Failure.invalidIdentity
        }
        _ = try AppBundleScheme(bundle: bundle, appKey: identity.appKey,
                                allowUnverifiedDeveloperContent: identity.isDeveloper)
        let url = try bundle.pageURL(appKey: identity.appKey, path: path, query: query)
        cancelPendingRequests()
        appIdentity = identity
        appBundle = bundle
        notice = nil
        webViewFor(url: url).load(URLRequest(url: url))
    }

    private var developerModeEnabled: Bool { UserDefaults.standard.bool(forKey: "developerMode") }

    #if os(macOS)
    func openLocalAppFolder() {
        guard developerModeEnabled else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = String(localized: "Open a local app folder")
        panel.begin { [weak self] response in
            guard response == .OK, let folder = panel.url else { return }
            Task { @MainActor in
                guard let self, self.developerModeEnabled else { return }
                do {
                    let bundle = try LocalAppFolder.load(from: folder, developerModeEnabled: true)
                    let identity = AppBrowserIdentity(developerSession: UUID(), chainID: self.model?.networkChainId ?? 0)
                    try self.openAppBundle(bundle, identity: identity)
                } catch {
                    self.notice = error.localizedDescription
                }
            }
        }
    }
    #endif

    /// Account, network, lock and developer-mode changes invalidate an app's
    /// provider and outstanding sheets before another request can be accepted.
    func environmentDidChange() {
        contentTask?.cancel()
        contentLoading = false
        cancelPendingRequests()
        if appIdentity != nil {
            webView?.stopLoading()
            webView = nil
            appIdentity = nil
            appBundle = nil
            appViewKey = nil
            currentURL = nil
            notice = String(localized: "The wallet context changed. Open the app again.")
        }
    }

    private func cancelPendingRequests() {
        navigationID = UUID()
        let pending = (ask.map { [$0] } ?? []) + askQueue
        ask = nil
        askQueue.removeAll()
        for item in pending {
            item.reply(.failure(ProviderError(code: ProviderErrorCode.denied,
                                             message: "The page or wallet context changed.")))
        }
    }

    /// The user acknowledged the warning: remember the host (once per site,
    /// per device) and load what was refused.
    func approveWarning() {
        guard let w = warning else { return }
        warning = nil
        var ack = acknowledged
        ack.insert(w.url.host ?? w.host)
        acknowledged = ack
        approvedURL = w.url
        load(w.url)
    }

    func refuseWarning() {
        warning = nil
        notice = String(localized: "Not opened.")
    }

    // MARK: - Sheet answers

    /// The user approved the sheet: the site may see this address, or the
    /// transaction goes to the send flow. The next queued request, if any,
    /// opens its sheet right after.
    func approveAsk() {
        guard let pending = ask else { return }
        ask = nil
        guard let currentModel = model, !currentModel.exploreLocked, pending.navigationID == navigationID,
              pending.account == currentModel.address, pending.chainID == currentModel.networkChainId else {
            pending.reply(.failure(ProviderError(code: ProviderErrorCode.denied, message: "The page or wallet context changed.")))
            drainAskQueue()
            return
        }
        switch pending.kind {
        case .connect(let origin, let displayOrigin):
            if let model = model, !model.address.isEmpty {
                model.grantSitePermission(origin: origin, address: model.address, displayOrigin: displayOrigin)
                pending.reply(.success([model.address]))
            } else {
                pending.reply(.failure(ProviderError(code: ProviderErrorCode.internalError,
                                                      message: "The wallet is not ready.")))
            }
        case .send(let origin, let host, let tx, let feeWei):
            guard appIdentity?.permitsSigning(developerMode: developerModeEnabled) != false else {
                pending.reply(.failure(ProviderError(code: ProviderErrorCode.denied,
                                                     message: "Local app files can sign only on the development network.")))
                drainAskQueue()
                return
            }
            if let model {
                let shown = tx.isPlainTransfer ? feeWei ?? model.status?.transferFeeWei : nil
                Task {
                    guard pending.navigationID == self.navigationID, pending.account == model.address,
                          pending.chainID == model.networkChainId, !model.exploreLocked else {
                        pending.reply(.failure(ProviderError(code: ProviderErrorCode.denied,
                                                             message: "The page or wallet context changed.")))
                        return
                    }
                    let (hash, refusal) = await model.sendPageTransaction(tx, origin: origin,
                                                                          title: host, shownFeeWei: shown)
                    if let hash {
                        pending.reply(.success(hash))
                    } else {
                        pending.reply(.failure(ProviderError(code: ProviderErrorCode.internalError,
                                                              message: refusal ?? "The send failed.")))
                    }
                }
            } else {
                pending.reply(.failure(ProviderError(code: ProviderErrorCode.internalError,
                                                      message: "The wallet is not ready.")))
            }
        }
        drainAskQueue()
    }

    /// The user refused the sheet (or dismissed it): 4001, nothing signed.
    func refuseAsk() {
        guard let pending = ask else { return }
        ask = nil
        pending.reply(.failure(ProviderError(code: ProviderErrorCode.denied, message: "The user rejected the request.")))
        drainAskQueue()
    }

    /// One sheet at a time: whatever queued while one was open comes next.
    private func drainAskQueue() {
        guard ask == nil, !askQueue.isEmpty else { return }
        ask = askQueue.removeFirst()
    }

    // MARK: - The bridge

    /// The reply closure WebKit hands over. This SDK imports the reply's error
    /// channel as an errorMessage string (WebKit turns a non-nil one into a JS
    /// rejection); this bridge always resolves with a {result}/{error} object
    /// instead, so the second parameter stays nil.
    fileprivate func handleBridgeMessage(_ message: WKScriptMessage,
                                         replyHandler: @escaping @MainActor @Sendable (Any?, String?) -> Void) {
        let reply: (Result<Any?, ProviderError>) -> Void = { result in
            switch result {
            case .success(let value):
                replyHandler(["result": value ?? NSNull()], nil)
            case .failure(let e):
                replyHandler(["error": ["code": e.code, "message": e.message]], nil)
            }
        }
        guard let model else {
            reply(.failure(ProviderError(code: ProviderErrorCode.internalError, message: "The wallet is not ready.")))
            return
        }
        // The provider lives in the main frame only; a subframe asking is a
        // page trying to look like its parent.
        guard message.frameInfo.isMainFrame, message.webView === webView else {
            reply(.failure(ProviderError(code: ProviderErrorCode.locked,
                                         message: "This frame cannot talk to the EastSea wallet provider.")))
            return
        }
        guard let body = message.body as? [String: Any], let method = body["method"] as? String else {
            reply(.failure(ProviderError(code: ProviderErrorCode.notAString, message: "method must be a string")))
            return
        }
        let params = (body["params"] as? [Any]) ?? []
        guard let context = bridgeContext(message) else {
            reply(.failure(ProviderError(code: ProviderErrorCode.locked, message: "This page no longer has a wallet provider.")))
            return
        }
        let key = context.key

        // Locked: nothing is answered, reads included.
        if let locked = ProviderGate.check(locked: model.exploreLocked) {
            reply(.failure(locked))
            return
        }

        // Only the methods the extension's provider surface answers.
        guard ProviderMethod.supported.contains(method) else {
            reply(.failure(ProviderError(code: ProviderErrorCode.unsupported,
                                         message: "EastSea Wallet does not support \(method).")))
            return
        }

        // One origin cannot stack sheets: three unanswered requests is enough.
        func originOf(_ a: PendingAsk) -> String? {
            switch a.kind {
            case .connect(let o, _): return o
            case .send(let o, _, _, _): return o
            }
        }
        let pendingForOrigin = askQueue.filter { originOf($0) == key }.count
            + (ask.flatMap(originOf) == key ? 1 : 0)
        if pendingForOrigin >= ProviderRouter.maxPendingPerOrigin {
            reply(.failure(ProviderError(code: ProviderErrorCode.timeout,
                                         message: "Too many requests from this site are already waiting.")))
            return
        }

        switch ProviderRouter.route(method: method, params: params) {
        case .chainId:
            Task.detached {
                let id = (try? configuredChainId()) ?? 0
                await MainActor.run { reply(.success("0x\(String(id, radix: 16))")) }
            }
        case .accounts:
            reply(.success(model.connectedSiteAddress(origin: key).map { [$0] } ?? []))
        case .requestAccounts:
            if let addr = model.connectedSiteAddress(origin: key) {
                reply(.success([addr]))
            } else {
                enqueue(.connect(origin: key, host: context.displayOrigin), id: body["id"], reply: reply)
            }
        case .disconnect:
            model.revokeSitePermission(origin: key)
            reply(.success(NSNull()))
        case .send:
            guard appIdentity?.permitsSigning(developerMode: developerModeEnabled) != false else {
                reply(.failure(ProviderError(code: ProviderErrorCode.denied,
                                             message: "Local app files can sign only on the development network.")))
                return
            }
            guard let connected = model.connectedSiteAddress(origin: key) else {
                reply(.failure(ProviderError(code: ProviderErrorCode.locked,
                                             message: "This site is not connected to an account.")))
                return
            }
            switch PageTransaction.parse(params.first, from: connected) {
            case .success(let tx):
                let feeWei: String?
                if tx.isPlainTransfer {
                    feeWei = (try? transferQuote(recipient: tx.to, validators: model.validators))?.feeWei
                        ?? model.status?.transferFeeWei
                } else {
                    feeWei = nil
                }
                enqueue(.send(origin: key, host: context.displayOrigin, tx: tx, feeWei: feeWei), id: body["id"], reply: reply)
            case .failure(let e):
                reply(.failure(e))
            }
        case .read(let verified):
            lastReadVerified = verified
            Task.detached { [weak self] in
                let result = await Self.performRead(method: method, params: params, verified: verified,
                                                    validators: await self?.model?.validators ?? 4,
                                                    port: await self?.model?.nodeRpcPort ?? 18545)
                await MainActor.run { reply(result) }
            }
        case .refused:
            reply(.failure(ProviderError(code: ProviderErrorCode.unsupported,
                                         message: "EastSea Wallet does not support \(method).")))
        }
    }

    // MARK: - The verify bridge

    /// The static entry points the weak bridge boxes hand messages to.
    private static func providerRoute(_ controller: BrowserController, _ message: WKScriptMessage,
                                      replyHandler: @escaping @MainActor @Sendable (Any?, String?) -> Void) {
        controller.handleBridgeMessage(message, replyHandler: replyHandler)
    }

    private static func verifyRoute(_ controller: BrowserController, _ message: WKScriptMessage,
                                    replyHandler: @escaping @MainActor @Sendable (Any?, String?) -> Void) {
        controller.handleVerifyMessage(message, replyHandler: replyHandler)
    }

    /// The {result}/{error} envelope both bridges resolve the page's promise
    /// with; the reply's error channel stays nil (see handleBridgeMessage).
    fileprivate static func envelope(_ replyHandler: @escaping @MainActor @Sendable (Any?, String?) -> Void)
        -> (Result<Any?, ProviderError>) -> Void {
        { result in
            switch result {
            case .success(let value):
                replyHandler(["result": value ?? NSNull()], nil)
            case .failure(let e):
                replyHandler(["error": ["code": e.code, "message": e.message]], nil)
            }
        }
    }

    /// `window.eastsea.verify` (Resources/provider.js): a bundled or connected
    /// page's block / account / receipt check, answered by the native verifier
    /// — the certificate checks of VerifyBridge in Rust, not the wasm module a
    /// public page would load (docs/design/09-wallet.md "인앱 브라우저").
    fileprivate func handleVerifyMessage(_ message: WKScriptMessage,
                                         replyHandler: @escaping @MainActor @Sendable (Any?, String?) -> Void) {
        let reply = Self.envelope(replyHandler)
        guard let model else {
            reply(.failure(ProviderError(code: ProviderErrorCode.internalError, message: "The wallet is not ready.")))
            return
        }
        // The provider lives in the main frame only; a subframe asking is a
        // page trying to look like its parent.
        guard message.frameInfo.isMainFrame, message.webView === webView else {
            reply(.failure(ProviderError(code: ProviderErrorCode.locked,
                                         message: "This frame cannot talk to the EastSea wallet provider.")))
            return
        }
        guard let body = message.body as? [String: Any] else {
            reply(.failure(ProviderError(code: ProviderErrorCode.params, message: "expected {what, param}")))
            return
        }
        // The same lock rule the provider answers under: nothing while locked.
        if let locked = ProviderGate.check(locked: model.exploreLocked) {
            reply(.failure(locked))
            return
        }
        // Bundled pages always; an external page only over https, and only
        // while its origin is connected to an account (VerifyBridge.allows).
        let origin = message.frameInfo.securityOrigin
        guard let context = bridgeContext(message) else {
            reply(.failure(ProviderError(code: ProviderErrorCode.locked, message: "This page no longer has a wallet provider.")))
            return
        }
        let key = context.key
        guard appIdentity != nil || VerifyBridge.allows(scheme: origin.protocol, connected: model.connectedSiteAddress(origin: key) != nil) else {
            reply(.failure(ProviderError(code: ProviderErrorCode.unsupported,
                                         message: "This page cannot use EastSea verification.")))
            return
        }
        switch VerifyBridge.parse(body) {
        case .failure(let e):
            reply(.failure(e))
        case .success(.receipt):
            // No block commits to receipts yet, so this is the one honest
            // answer any verifier could give (VerifyBridge.Verdict.notCommitted).
            reply(.success(VerifyBridge.Verdict.notCommitted.asDictionary()))
        case .success(.account(let address)):
            let validators = model.validators
            Task.detached {
                do {
                    let account = try verifiedAccount(address: address, validators: validators)
                    await MainActor.run { reply(.success(VerifyBridge.Verdict.certified(height: account.certifiedBlock).asDictionary())) }
                } catch {
                    await MainActor.run { reply(.success(VerifyBridge.Verdict.refused(WalletModel.ffiMessage(error)).asDictionary())) }
                }
            }
        case .success(.block(let height)):
            Task.detached {
                do {
                    let block = try verifiedBlock(height: height)
                    await MainActor.run { reply(.success(VerifyBridge.Verdict.certified(height: block.height).asDictionary())) }
                } catch {
                    await MainActor.run { reply(.success(VerifyBridge.Verdict.refused(WalletModel.ffiMessage(error)).asDictionary())) }
                }
            }
        }
    }

    private func enqueue(_ kind: PendingAsk.Kind, id: Any?, reply: @escaping (Result<Any?, ProviderError>) -> Void) {
        let pending = PendingAsk(id: (id as? String).map { "ask-\($0)" } ?? UUID().uuidString,
                                 kind: kind, navigationID: navigationID,
                                 account: model?.address ?? "", chainID: model?.networkChainId ?? 0, reply: reply)
        if ask == nil { ask = pending } else { askQueue.append(pending) }
    }

    private func bridgeContext(_ message: WKScriptMessage) -> (key: String, displayOrigin: String)? {
        guard message.webView === webView, message.frameInfo.isMainFrame else { return nil }
        let origin = message.frameInfo.securityOrigin
        if let identity = appIdentity {
            guard identity.accepts(scheme: origin.protocol, host: origin.host, port: Int(origin.port),
                                   mainFrame: true, currentChainID: model?.networkChainId ?? 0,
                                   developerMode: developerModeEnabled) else { return nil }
            return (identity.permissionKey, identity.displayOrigin)
        }
        guard origin.protocol == "https" || origin.protocol == BrowserOriginPolicy.bundledScheme else { return nil }
        return (BrowserOriginPolicy.permissionKey(scheme: origin.protocol, host: origin.host, port: Int(origin.port)), origin.host)
    }

    // MARK: - Reads

    /// A read goes to the FFI's certificate-backed paths when one answers it
    /// exactly (verified), or to this Mac's node as-is (unverified — nothing
    /// here vouches for the answer, and the tab says so).
    nonisolated private static func performRead(method: String, params: [Any], verified: Bool,
                                                validators: UInt32, port: UInt16) async -> Result<Any?, ProviderError> {
        if !verified { return await nodeCall(port: port, method: method, params: params) }
        do {
            switch method {
            case "eth_blockNumber":
                return .success("0x\(String(try verifiedHeight(), radix: 16))")
            case "eth_getBalance":
                guard let addr = params.first as? String else { throw ProviderError(code: ProviderErrorCode.params, message: "expected an address") }
                let account = try verifiedAccount(address: addr, validators: validators)
                return .success(PageTransaction.hex(fromDecimal: account.balanceWei))
            case "net_version":
                return .success(String(try configuredChainId()))
            case "aether_accountHistory":
                guard let addr = params.first as? String else { throw ProviderError(code: ProviderErrorCode.params, message: "expected an address") }
                let cursor = params.count > 1 ? params[1] as? String : nil
                let limit = params.count > 2 ? (params[2] as? Int).map(UInt32.init) : nil
                let json = try accountHistory(address: addr, cursor: cursor, limit: limit ?? 200)
                guard let data = json.data(using: .utf8),
                      let obj = try JSONSerialization.jsonObject(with: data) as? Any else {
                    throw ProviderError(code: ProviderErrorCode.internalError, message: "the history came back unreadable")
                }
                return .success(obj)
            case "eth_call":
                guard let call = params.first as? [String: Any],
                      let to = call["to"] as? String else {
                    throw ProviderError(code: ProviderErrorCode.params, message: "expected a call object")
                }
                let data = (call["data"] as? String) ?? "0x"
                return .success(try ethCall(to: to, dataHex: data))
            default:
                return await nodeCall(port: port, method: method, params: params)
            }
        } catch let e as ProviderError {
            return .failure(e)
        } catch {
            return .failure(ProviderError(code: ProviderErrorCode.internalError,
                                          message: WalletModel.ffiMessage(error)))
        }
    }

    /// Forward a read to this Mac's node, answer with whatever it said.
    nonisolated private static func nodeCall(port: UInt16, method: String, params: [Any]) async -> Result<Any?, ProviderError> {
        let request = ["jsonrpc": "2.0", "id": 1, "method": method, "params": params] as [String: Any]
        guard let body = try? JSONSerialization.data(withJSONObject: request),
              let url = URL(string: "http://127.0.0.1:\(port)/") else {
            return .failure(ProviderError(code: ProviderErrorCode.internalError, message: "the request could not be built"))
        }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = body
        req.timeoutInterval = 10
        do {
            let (data, response) = try await URLSession.shared.data(for: req)
            guard (response as? HTTPURLResponse)?.statusCode == 200 else {
                return .failure(ProviderError(code: ProviderErrorCode.internalError,
                                               message: "the node on this Mac did not answer"))
            }
            guard let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                return .failure(ProviderError(code: ProviderErrorCode.internalError, message: "the node's answer was unreadable"))
            }
            if let error = obj["error"] as? [String: Any] {
                return .failure(ProviderError(code: (error["code"] as? Int) ?? ProviderErrorCode.internalError,
                                              message: (error["message"] as? String) ?? "the node refused the read"))
            }
            return .success(obj["result"] ?? NSNull())
        } catch {
            return .failure(ProviderError(code: ProviderErrorCode.internalError,
                                           message: "The node on this Mac is not running. Turn it on in Network."))
        }
    }
}

/// `addScriptMessageHandler` keeps a strong reference; this box keeps the
/// controller weak so the tab can go away without a cycle. `route` is which
/// bridge it answers (a static method reference, so the box captures nothing).
@MainActor
private final class WeakReplyBridge: NSObject, WKScriptMessageHandlerWithReply {
    weak var controller: BrowserController?
    private let route: @MainActor (BrowserController, WKScriptMessage,
                                   @escaping @MainActor @Sendable (Any?, String?) -> Void) -> Void

    init(controller: BrowserController,
         route: @escaping @MainActor (BrowserController, WKScriptMessage,
                                      @escaping @MainActor @Sendable (Any?, String?) -> Void) -> Void) {
        self.controller = controller
        self.route = route
        super.init()
    }

    func userContentController(_ userContentController: WKUserContentController,
                               didReceive message: WKScriptMessage,
                               replyHandler: @escaping @MainActor @Sendable (Any?, String?) -> Void) {
        guard let controller else {
            replyHandler(["error": ["code": ProviderErrorCode.internalError, "message": "the tab is gone"]], nil)
            return
        }
        route(controller, message, replyHandler)
    }
}

extension BrowserController: WKNavigationDelegate, WKUIDelegate {
    // MARK: - WKNavigationDelegate

    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                 preferences: WKWebpagePreferences) async -> WKNavigationActionPolicy {
        guard let url = navigationAction.request.url else { return .cancel }
        if url.scheme == AppBrowserIdentity.scheme {
            guard let identity = appIdentity, url.host == identity.appKey,
                  !identity.isDeveloper || developerModeEnabled,
                  let bundle = appBundle,
                  bundle.documentPath(for: url, appKey: identity.appKey) != nil else {
                notice = String(localized: "This app has not been verified. Open its sea:// name first.")
                return .cancel
            }
            if navigationAction.targetFrame?.isMainFrame == true { cancelPendingRequests() }
            return .allow
        }
        if appIdentity != nil, navigationAction.targetFrame?.isMainFrame != false {
            load(url)
            return .cancel
        }
        if navigationAction.targetFrame?.isMainFrame == true { cancelPendingRequests() }
        switch BrowserOriginPolicy.classify(url) {
        case .bundled:
            return .allow
        case .external(let host, let lookalike, let punycode):
            // Curated entries and already-acknowledged sites load without
            // another word; anything else waits for the one-time warning.
            if BrowserOriginPolicy.isCurated(host) || acknowledged.contains(host) || approvedURL == url {
                approvedURL = nil
                return .allow
            }
            if navigationAction.navigationType == .backForward { return .allow }
            warning = SiteWarning(url: url, host: host, lookalike: lookalike, punycode: punycode)
            return .cancel
        case .blocked(let why):
            notice = why
            return .cancel
        }
    }

    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        currentURL = webView.url
        addressField = webView.url.map { urlBarText($0) } ?? ""
        canGoBack = webView.canGoBack
        notice = nil
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        canGoBack = webView.canGoBack
    }

    /// What the bar shows for a loaded page: the page's own URL, without a
    /// trailing slash theater.
    private func urlBarText(_ url: URL) -> String {
        if let identity = appIdentity, url.scheme == AppBrowserIdentity.scheme {
            var text = identity.displayOrigin
            if url.path != "/index.html" { text += url.path }
            if let query = url.query { text += "?\(query)" }
            return text
        }
        if url.scheme?.lowercased() == BrowserOriginPolicy.bundledScheme {
            return url.host == "explorer" ? String(localized: "Block explorer") : (url.host ?? "")
        }
        return url.absoluteString
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        guard webView === self.webView else { return }
        notice = error.localizedDescription
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        guard webView === self.webView else { return }
        notice = error.localizedDescription
    }

    // MARK: - WKUIDelegate

    /// No popups, no new windows: everything stays in this tab.
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if navigationAction.targetFrame == nil, let url = navigationAction.request.url {
            load(url)   // a plain target=_blank link opens in the same tab
        }
        return nil
    }
}
