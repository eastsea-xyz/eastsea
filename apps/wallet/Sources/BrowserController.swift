import Foundation
import Combine
import WebKit
#if os(macOS)
import AppKit
#else
import UIKit
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
    @Published private(set) var httpsOffer: URL?
    /// The one-time warning sheet for an external site.
    @Published var warning: SiteWarning?
    /// The confirmation sheet a page's request opened (one at a time).
    @Published var ask: PendingAsk?
    @Published private(set) var simulation: SimulatedPageTransaction?
    @Published private(set) var approvalNotice: String?
    @Published private(set) var approvalBusy = false
    @Published private(set) var submissionInFlight = false
    @Published private(set) var simulationLoading = false
    /// Whether the last read this tab answered was certificate-verified.
    @Published private(set) var lastReadVerified = true
    /// Bumped whenever the WebView had to be rebuilt (a store switch).
    @Published private(set) var webViewGeneration = 0
    @Published private(set) var canGoBack = false
    @Published private(set) var canGoForward = false
    @Published private(set) var isLoading = false
    @Published private(set) var estimatedProgress = 0.0
    @Published private(set) var title = ""
    @Published private(set) var canonicalOrigin = ""
    @Published private(set) var isSecureOrigin = false
    @Published private(set) var connectedAccount: String?
    @Published private(set) var requestedAllowances: [String] = []
    @Published private(set) var tokenAllowances: [BrowserTokenAllowance] = []
    @Published private(set) var zoomFactor = 1.0
    @Published var findQuery = ""
    @Published private(set) var findFound: Bool?
    @Published private(set) var downloadPrompt: DownloadPrompt?
    /// nil until something was loaded: while nil the curated home shows.
    @Published private(set) var currentURL: URL?
    @Published private(set) var appIdentity: AppBrowserIdentity?
    @Published private(set) var contentLoading = false
    /// Search stays native and preserves the node's record order and coverage.
    @Published private(set) var searchQuery: String?
    @Published private(set) var searchResults: [AppSearchResult] = []
    @Published private(set) var searchBusy = false
    @Published private(set) var searchFailure: String?
    @Published private(set) var searchInfo: AppSearchInfo?
    @Published private(set) var searchSuggestions: [AppSearchResult] = []
    @Published private(set) var suggestionsBusy = false
    @Published private(set) var suggestionsFailure: String?
    @Published private(set) var suggestionsInfo: AppSearchInfo?
    @Published var searchRecord: AppSearchResult?
    @Published private(set) var searchRecordInfo: AppSearchInfo?
    private var appBundle: AppBundle?
    private var appViewKey: String?
    private var contentTask: Task<Void, Never>?
    private var navigationID = UUID()

    /// Native display labels never expose the internal app permission key.
    var displayOrigin: String { appIdentity?.displayOrigin ?? canonicalOrigin }

    let isPrivate: Bool
    /// The session owns account-scoped persistence; a private tab never calls
    /// either persistence callback.
    var onCommit: ((URL, String) -> Void)?
    var onTitleChange: ((URL, String) -> Void)?
    var onHome: (() -> Void)?
    var zoomProvider: ((String) -> Double)?
    var onZoomChange: ((String, Double) -> Void)?
    var onSeaLink: ((URL) -> Void)?
    /// Returns the protected name this URL imitates. This runs even for a
    /// previously acknowledged or curated site.
    var phishingCheck: ((URL) -> String?)?

    struct NavigationItem: Identifiable {
        let id: UUID
        let url: URL?
        var title: String
    }

    struct DownloadPrompt: Identifiable {
        let id = UUID()
        let url: URL
        let filename: String
        let origin: String
        let isExecutable: Bool
    }

    @Published private(set) var navigationItems: [NavigationItem] = []
    private var navigationIndex = 0
    private var navigationTarget: UUID?
    var backHistory: [NavigationItem] { Array(navigationItems.prefix(navigationIndex).reversed()) }
    var forwardHistory: [NavigationItem] { Array(navigationItems.dropFirst(navigationIndex + 1)) }

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
            case typed(origin: String, host: String, prepared: PreparedTypedMessage, fields: TypedMessageFields)
        }
        let id: String
        let kind: Kind
        let chainID: UInt64
        let reply: (Result<Any?, ProviderError>) -> Void
        let accountID: Int?
        let accountAddress: String
        let documentGeneration: UInt64
        let privatePermissionGeneration: UInt64
        let context: DappRequestContext?
        let lifecycle: DappApprovalLifecycle

        init(id: String, kind: Kind, reply: @escaping (Result<Any?, ProviderError>) -> Void,
             accountID: Int? = nil, accountAddress: String = "", documentGeneration: UInt64 = 0,
             chainID: UInt64 = 0, privatePermissionGeneration: UInt64 = 0,
             context: DappRequestContext? = nil) {
            self.id = id
            self.kind = kind
            self.accountID = accountID
            self.accountAddress = accountAddress
            self.documentGeneration = documentGeneration
            self.privatePermissionGeneration = privatePermissionGeneration
            self.chainID = chainID
            self.context = context
            let lifecycle = DappApprovalLifecycle(reply: reply)
            self.lifecycle = lifecycle
            self.reply = { result in lifecycle.finish(result) }
        }

        var origin: String {
            switch kind {
            case .connect(let origin, _), .send(let origin, _, _, _), .typed(let origin, _, _, _): return origin
            }
        }
    }

    private(set) weak var model: WalletModel?
    /// The tab's current WebView (rebuilt when the data store has to change).
    @Published private(set) var webView: WKWebView?
    /// The external host the current non-persistent WebView belongs to.
    private var externalHost: String?
    private var askQueue: [PendingAsk] = []
    private var preparingAsks: [String: Int] = [:]
    private var busyAskId: String?
    /// The URL a just-acknowledged warning may load (one shot, so the same
    /// warning cannot loop).
    private var approvedURL: URL?
    private var searchTask: Task<Void, Never>?
    private var suggestionTask: Task<Void, Never>?
    private var searchRequestID = UUID()
    private var suggestionRequestID = UUID()
    private var searchNeedsRefresh = false
    private var observations: [NSKeyValueObservation] = []
    private var modelSubscriptions: Set<AnyCancellable> = []
    private var documentGeneration: UInt64 = 0
    private var committedOrigin: String?
    private var suspended = false
    private var closed = false
    private var pendingNavigation: PendingNavigation?
    private var interruptedNavigation: PendingNavigation?
    private var activeNavigation: WKNavigation?
    private var controllerPolicyPending = false
    /// Canceled tokens stay excluded while WebKit still retains them for
    /// callbacks. Weak membership prevents retaining completed navigations.
    private let discardedNavigations = NSHashTable<WKNavigation>.weakObjects()
    private var acceptsNavigationCallbacks = true
    private var findRequest = UUID()
    private var privateAcknowledged: Set<String> = []
    private var privatePermissions: [String: String] = [:]
    private var privatePermissionGeneration: UInt64 = 0
    private var privateZoom: [String: Double] = [:]
    private var allowances: [String: Set<SiteAllowance>] = [:]
    private var requestedTokenAllowances: [String: [BrowserTokenAllowance]] = [:]
    private var pendingDownloads: [PendingDownload] = []
    private var activeDownloads: [ObjectIdentifier: ActiveDownload] = [:]
    private var downloadViews: [ObjectIdentifier: WKWebView] = [:]

    private struct PendingNavigation {
        var url: URL
        let historyTarget: UUID?
        let accountID: Int?
        let accountAddress: String
        let chainID: UInt64?
        let wasApproved: Bool
    }

    private enum SiteAllowance: String, CaseIterable {
        case account, transactions, tokenApproval, verifiedRead, unverifiedRead
        var label: String {
            switch self {
            case .account: return String(localized: "Read your address")
            case .transactions: return String(localized: "Request transactions")
            case .tokenApproval: return String(localized: "Request token approval")
            case .verifiedRead: return String(localized: "Verify blockchain data")
            case .unverifiedRead: return String(localized: "Read unverified node data")
            }
        }
    }

    private struct PendingDownload {
        let prompt: DownloadPrompt
        let download: WKDownload
        let destination: (URL?) -> Void
    }

    private struct ActiveDownload {
        let download: WKDownload
        let stagingDirectory: URL
        let stagedFile: URL
        let downloadsDirectory: URL
        let filename: String
    }

    init(isPrivate: Bool = false) {
        self.isPrivate = isPrivate
        super.init()
        navigationItems = [NavigationItem(id: UUID(), url: nil, title: String(localized: "Start page"))]
    }

    private static let acknowledgedKey = "explore.acknowledged"
    private var acknowledged: Set<String> {
        get { isPrivate ? privateAcknowledged : Set(UserDefaults.standard.stringArray(forKey: Self.acknowledgedKey) ?? []) }
        set {
            if isPrivate { privateAcknowledged = newValue }
            else { UserDefaults.standard.set(Array(newValue), forKey: Self.acknowledgedKey) }
        }
    }

    /// Wire the wallet state the bridge answers from (called once, at attach).
    func attach(model: WalletModel) {
        guard !closed else { return }
        guard self.model !== model else { resume(); return }
        pendingNavigation = nil
        interruptedNavigation = nil
        modelSubscriptions.removeAll()
        self.model = model
        model.accountStore.activeAccountPublisher.map { $0?.id }.removeDuplicates()
            .sink { [weak self] _ in self?.accountDidChange() }.store(in: &modelSubscriptions)
        model.$networkChainId.removeDuplicates()
            .sink { [weak self] _ in self?.accountDidChange() }.store(in: &modelSubscriptions)
        model.$address.removeDuplicates().dropFirst().sink { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                self.accountDidChange()
            }
        }.store(in: &modelSubscriptions)
        model.$sitePermissions.sink { [weak self] _ in
            Task { @MainActor in self?.refreshSitePermissions() }
        }.store(in: &modelSubscriptions)
        refreshSitePermissions()
        resume()
    }

    func accountDidChange() {
        guard !closed else { return }
        cancelSearchRead()
        dismissSuggestions()
        searchRecord = nil
        searchRecordInfo = nil
        searchResults = []
        searchFailure = nil
        searchInfo = nil
        searchNeedsRefresh = searchQuery != nil
        let hadApp = appIdentity != nil || contentLoading
        cancelContentResolution()
        pendingNavigation = nil
        interruptedNavigation = nil
        discardActiveNavigation()
        acceptsNavigationCallbacks = false
        webView?.stopLoading()
        isLoading = false
        navigationTarget = nil
        warning = nil
        approvedURL = nil
        invalidateDocument()
        if hadApp {
            releaseWebView()
            appIdentity = nil
            appBundle = nil
            currentURL = nil
            addressField = ""
            title = ""
            canonicalOrigin = ""
            isSecureOrigin = false
            notice = String(localized: "The wallet context changed. Open the app again.")
            onHome?()
        }
        privatePermissions.removeAll()
        allowances.removeAll()
        requestedTokenAllowances.removeAll()
        committedOrigin = searchQuery == nil ? webView?.url.flatMap(browserOrigin(for:)) : nil
        refreshSitePermissions()
    }

    func suspend() {
        guard !suspended else { return }
        if searchBusy { searchNeedsRefresh = true }
        cancelSearchRead()
        dismissSuggestions()
        searchRecord = nil
        searchRecordInfo = nil
        if !closed { interruptedNavigation = pendingNavigation }
        cancelContentResolution()
        pendingNavigation = nil
        discardActiveNavigation()
        acceptsNavigationCallbacks = false
        suspended = true
        // Unlike the user's Stop action, tab suspension retains a permitted
        // GET that has not finished so an explicit tab resume can retry it.
        webView?.stopLoading()
        isLoading = false
        invalidateDocument()
        warning = nil
        approvedURL = nil
        refusePendingDownloads()
    }

    func resume() {
        guard !closed else { return }
        let interrupted = suspended ? interruptedNavigation : nil
        interruptedNavigation = nil
        suspended = false
        if let query = searchQuery, searchNeedsRefresh {
            searchNeedsRefresh = false
            search(query)
            return
        }
        committedOrigin = searchQuery == nil ? webView?.url.flatMap(browserOrigin(for:)) : nil
        refreshSitePermissions()
        guard let interrupted,
              interrupted.accountID == model?.accountStore.activeAccount?.id,
              interrupted.accountAddress == (model?.address.lowercased() ?? ""),
              interrupted.chainID == model?.networkChainId else { return }
        if interrupted.wasApproved { approvedURL = interrupted.url }
        performLoad(interrupted.url, historyTarget: interrupted.historyTarget)
    }

    func close() {
        guard !closed else { return }
        closed = true
        pendingNavigation = nil
        interruptedNavigation = nil
        suspend()
        for active in activeDownloads.values {
            active.download.delegate = nil
            active.download.cancel { _ in
                try? FileManager.default.removeItem(at: active.stagingDirectory)
            }
        }
        activeDownloads.removeAll()
        downloadViews.removeAll()
        releaseWebView()
        appIdentity = nil
        appBundle = nil
        currentURL = nil
        privatePermissions.removeAll()
        privateAcknowledged.removeAll()
        privateZoom.removeAll()
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
        if case .external(let host, _, _)? = classification { wantExternal = host.lowercased() } else { wantExternal = nil }

        if let view = webView {
            if let key = wantAppKey, key == appViewKey { return view }
            if wantAppKey == nil && appViewKey == nil {
                if wantExternal == nil && externalHost == nil { return view }
                if let host = wantExternal, let current = externalHost, host == current { return view }
            }
        }
        releaseWebView()
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
        // Pages cannot forge this handler: both it and the trusted-click
        // listener live in WebKit's isolated client world.
        ucc.add(WeakBrowserActionBridge(controller: self), contentWorld: .defaultClient, name: "browserAction")
        ucc.addUserScript(WKUserScript(source: Self.actionLinkScript, injectionTime: .atDocumentStart,
                                      forMainFrameOnly: true, in: .defaultClient))
        if let host = wantExternal {
            config.websiteDataStore = .nonPersistent()
            externalHost = host
        } else {
            externalHost = nil
            if isPrivate { config.websiteDataStore = .nonPersistent() }
        }
        appViewKey = wantAppKey
        let view = BrowserHistoryWebView(frame: .zero, configuration: config)
        view.backAvailable = { [weak self] in self?.canGoBack == true }
        view.forwardAvailable = { [weak self] in self?.canGoForward == true }
        view.onBack = { [weak self] in self?.goBack() }
        view.onForward = { [weak self] in self?.goForward() }
        view.navigationDelegate = self
        view.uiDelegate = self
        // Controller history also contains entries from rebuilt host stores.
        // Keep gesture traversal under that one owner, including repeated URLs.
        view.allowsBackForwardNavigationGestures = false
        webView = view
        observe(view)
        webViewGeneration += 1
        return view
    }

    private func releaseWebView() {
        observations.removeAll()
        webView?.stopLoading()
        webView?.navigationDelegate = nil
        webView?.uiDelegate = nil
        webView?.configuration.userContentController.removeAllScriptMessageHandlers()
        webView = nil
        externalHost = nil
        appViewKey = nil
    }

    private func observe(_ view: WKWebView) {
        let changed: () -> Void = { [weak self, weak view] in
            Task { @MainActor in
                guard let self, let view, self.webView === view else { return }
                self.isLoading = view.isLoading
                self.estimatedProgress = view.estimatedProgress
                guard self.committedOrigin != nil else { return }
                if let url = view.url, !view.isLoading, self.displayURL(for: url) != self.currentURL,
                   self.browserOrigin(for: url) == self.committedOrigin {
                    self.commitURL(url, in: view)
                }
                self.updateTitle(view.title)
            }
        }
        observations = [
            view.observe(\.isLoading, options: [.initial, .new]) { _, _ in changed() },
            view.observe(\.estimatedProgress, options: [.initial, .new]) { _, _ in changed() },
            view.observe(\.title, options: [.new]) { _, _ in changed() },
            view.observe(\.url, options: [.new]) { _, _ in changed() }
        ]
    }

    private static let actionLinkScript = """
        document.addEventListener('click', function(event) {
          if (!event.isTrusted || event.defaultPrevented) return;
          const node = event.target instanceof Element ? event.target : event.target.parentElement;
          const anchor = node && node.closest('a[href]');
          if (!anchor) return;
          try {
            const url = new URL(anchor.href, document.baseURI);
            if (!['sea:', 'aether:', 'eastsea:'].includes(url.protocol)) return;
            event.preventDefault();
            window.webkit.messageHandlers.browserAction.postMessage({url: url.href});
          } catch (_) {}
        }, true);
        """

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
        guard !closed else { return }
        interruptedNavigation = nil
        resume()
        openInput(text, historyTarget: nil)
    }

    private func openInput(_ text: String, historyTarget: UUID?) {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        cancelContentResolution()
        notice = nil
        httpsOffer = nil
        do {
            switch try SeaURL.browserInput(text, chainID: model?.browserChainID ?? Brand.networkChainId) {
            case .name(let link):
                navigationTarget = historyTarget
                guard let url = URL(string: link.canonicalURL), confirmSeaName(url) else { return }
                openName(link, historyTarget: historyTarget)
            case .web(let url): performLoad(url, historyTarget: historyTarget)
            case .action(let host, let raw):
                if ["pay", "call", "connect", "tx"].contains(host) { model?.open(link: raw) }
                else { notice = String(localized: "This wallet action is not available yet.") }
            }
        } catch {
            notice = SeaNameText.message(error)
            if error as? SeaURL.ParseError == .externalTLD { httpsOffer = SeaURL.suggestedHTTPS(text) }
        }
    }

    func openOfferedHTTPS() {
        guard let url = httpsOffer else { return }
        httpsOffer = nil
        load(url)
    }

    private func openName(_ link: SeaURL.NameLink, historyTarget: UUID?) {
        guard !suspended, !closed else { return }
        cancelContentResolution()
        discardActiveNavigation()
        invalidateDocument()
        let requestID = navigationID
        let chain = model?.browserChainID ?? Brand.networkChainId
        let port = model?.nodeRpcPort ?? 18545
        let accountID = model?.accountStore.activeAccount?.id
        let accountAddress = model?.address.lowercased() ?? ""
        let pins = SeaRegistrySources.bundled(chainID: chain)
        releaseWebView()
        appIdentity = nil
        appBundle = nil
        notice = nil
        warning = nil
        navigationTarget = historyTarget
        let url = URL(string: link.canonicalURL)!
        rememberPendingNavigation(url, historyTarget: historyTarget)
        acceptsNavigationCallbacks = false
        currentURL = url
        addressField = link.canonicalURL
        canonicalOrigin = BrowserCanonicalOrigin.string(for: url) ?? ""
        title = link.name
        isSecureOrigin = false
        isLoading = false
        estimatedProgress = 0
        refreshSitePermissions()
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
                guard !self.suspended, !self.closed, self.navigationID == requestID,
                      self.model?.browserChainID == chain, self.model?.nodeRpcPort == port,
                      self.model?.accountStore.activeAccount?.id == accountID,
                      self.model?.address.lowercased() == accountAddress,
                      let registry = pins?.apps.address else { return }
                let identity = try AppBrowserIdentity(appID: record.app.appID, name: link.name, chainID: chain, registry: registry)
                self.contentLoading = false
                try self.openAppBundle(source.bundle, identity: identity, path: link.path, query: link.query)
            } catch is CancellationError {
                return
            } catch {
                guard self.navigationID == requestID else { return }
                self.contentLoading = false
                self.contentTask = nil
                self.pendingNavigation = nil
                self.navigationTarget = nil
                self.notice = error is AppContentError ? error.localizedDescription : SeaNameText.message(error)
            }
        }
    }

    /// Search is a native surface. The retained page cannot ask the wallet
    /// or navigate underneath it while results are being displayed.
    func search(_ text: String = "") {
        guard !closed, !suspended else { return }
        cancelSearchRead()
        dismissSuggestions()
        searchRecord = nil
        searchRecordInfo = nil
        cancelContentResolution()
        pendingNavigation = nil
        interruptedNavigation = nil
        discardActiveNavigation()
        acceptsNavigationCallbacks = false
        invalidateDocument()
        webView?.stopLoading()
        isLoading = false
        warning = nil
        approvedURL = nil
        navigationTarget = nil
        refusePendingDownloads()
        refreshSitePermissions()
        notice = nil
        let query = AppSearchInput.seaName(in: text) ?? text.trimmingCharacters(in: .whitespacesAndNewlines)
        searchQuery = query
        canGoBack = true
        addressField = query
        searchResults = []
        searchFailure = nil
        searchInfo = nil
        searchNeedsRefresh = false
        guard !query.isEmpty else { return }
        let request: AppSearchRequest
        do { request = try AppSearchRequest(query: query) }
        catch { searchFailure = AppSearchFailure.queryTooLong.message; return }
        guard let port = model?.nodeRpcPort else {
            searchFailure = AppSearchFailure.unavailable.message
            return
        }
        let requestID = searchRequestID
        let generation = documentGeneration
        let accountID = model?.accountStore.activeAccount?.id
        let address = model?.address.lowercased()
        let chainID = model?.networkChainId
        searchBusy = true
        searchTask = Task { [weak self] in
            let result = await Self.readSearch(port: port, request: request)
            guard !Task.isCancelled, let self, !self.closed, !self.suspended,
                  self.searchRequestID == requestID, self.documentGeneration == generation,
                  self.searchQuery == query, self.model?.nodeRpcPort == port,
                  self.model?.accountStore.activeAccount?.id == accountID,
                  self.model?.address.lowercased() == address,
                  self.model?.networkChainId == chainID else { return }
            self.searchTask = nil
            self.searchBusy = false
            switch result {
            case .success(let response):
                self.searchResults = response.records
                self.searchInfo = response.info
            case .failure(let failure): self.searchFailure = failure.message
            }
        }
    }

    /// Editing, leaving the address field, suspension and navigation each
    /// invalidate the read, including repeated queries with different epochs.
    func suggest(_ text: String) {
        dismissSuggestions()
        guard !closed, !suspended, case .search(let query) = AppSearchInput.destination(for: text),
              !query.isEmpty else { return }
        let request: AppSearchRequest
        do { request = try AppSearchRequest(query: query, limit: 5) }
        catch { suggestionsFailure = AppSearchFailure.queryTooLong.message; return }
        guard let port = model?.nodeRpcPort else { return }
        let requestID = suggestionRequestID
        let accountID = model?.accountStore.activeAccount?.id
        let address = model?.address.lowercased()
        let chainID = model?.networkChainId
        suggestionTask = Task { [weak self] in
            do { try await Task.sleep(nanoseconds: 300_000_000) }
            catch { return }
            guard !Task.isCancelled, let self, !self.closed, !self.suspended,
                  self.suggestionRequestID == requestID else { return }
            self.suggestionsBusy = true
            let result = await Self.readSearch(port: port, request: request)
            guard !Task.isCancelled, !self.closed, !self.suspended,
                  self.suggestionRequestID == requestID, self.model?.nodeRpcPort == port,
                  self.model?.accountStore.activeAccount?.id == accountID,
                  self.model?.address.lowercased() == address,
                  self.model?.networkChainId == chainID,
                  AppSearchInput.destination(for: self.addressField) == .search(query) else { return }
            self.suggestionTask = nil
            self.suggestionsBusy = false
            switch result {
            case .success(let response):
                self.searchSuggestions = response.records
                self.suggestionsInfo = response.info
            case .failure(let failure): self.suggestionsFailure = failure.message
            }
        }
    }

    func dismissSuggestions() {
        suggestionTask?.cancel()
        suggestionTask = nil
        suggestionRequestID = UUID()
        searchSuggestions = []
        suggestionsBusy = false
        suggestionsFailure = nil
        suggestionsInfo = nil
    }

    func showSearchRecord(_ record: AppSearchResult) {
        guard !closed, !suspended else { return }
        cancelPendingAsks()
        refusePendingDownloads()
        warning = nil
        approvedURL = nil
        searchRecordInfo = suggestionsInfo
        dismissSuggestions()
        searchRecord = record
    }

    func dismissSearch() {
        let wasSearching = searchQuery != nil
        cancelSearchRead()
        dismissSuggestions()
        searchRecord = nil
        searchRecordInfo = nil
        searchQuery = nil
        canGoBack = navigationIndex > 0
        searchResults = []
        searchFailure = nil
        searchInfo = nil
        searchNeedsRefresh = false
        guard wasSearching else { return }
        addressField = webView?.url.map { urlBarText($0) } ?? currentURL?.absoluteString ?? ""
        if !closed, !suspended {
            committedOrigin = webView?.url.flatMap(browserOrigin(for:))
            refreshSitePermissions()
        }
    }

    func refreshSearchForNode() {
        searchRecord = nil
        searchRecordInfo = nil
        if let query = searchQuery {
            if suspended { searchNeedsRefresh = true }
            else { search(query) }
        } else { dismissSuggestions() }
    }

    private func cancelSearchRead() {
        searchTask?.cancel()
        searchTask = nil
        searchRequestID = UUID()
        searchBusy = false
    }

    private struct SearchResponse: Sendable {
        let records: [AppSearchResult]
        let info: AppSearchInfo?
    }

    nonisolated private static func readSearch(port: UInt16, request: AppSearchRequest) async -> Result<SearchResponse, AppSearchFailure> {
        async let recordsRead = nodeCall(port: port, method: AppSearchRequest.method, params: request.params)
        async let infoRead = nodeCall(port: port, method: "aether_searchInfo", params: [])
        let (recordsReply, infoReply) = await (recordsRead, infoRead)
        switch recordsReply {
        case .success(let value):
            let info: AppSearchInfo?
            if case .success(let value) = infoReply { info = try? AppSearchInfo.decode(from: value) }
            else { info = nil }
            do { return .success(SearchResponse(records: try request.results(from: value), info: info)) }
            catch { return .failure(.malformedResponse) }
        case .failure(let error):
            return .failure(error.code == -32601 ? .unsupported : .unavailable)
        }
    }

    /// Load a URL through the same rules a link goes through. Address input is
    /// normalized by BrowserSession; every URL is still checked here.
    func load(_ url: URL) {
        guard !closed else { return }
        // A new address supersedes the interrupted request; it must not
        // restart the old page as a side effect of activating this tab.
        interruptedNavigation = nil
        resume()
        performLoad(url, historyTarget: nil)
    }

    private func performLoad(_ url: URL, historyTarget: UUID?) {
        guard !suspended, !closed else { return }
        dismissSearch()
        cancelContentResolution()
        navigationTarget = historyTarget
        notice = nil
        httpsOffer = nil
        if Self.isActionLink(url) {
            openInput(url.absoluteString, historyTarget: historyTarget)
            return
        }
        guard navigationAllowed(url) else { return }
        discardActiveNavigation()
        invalidateDocument()
        if url.scheme?.lowercased() != AppBrowserIdentity.scheme {
            appIdentity = nil
            appBundle = nil
        }
        rememberPendingNavigation(url, historyTarget: historyTarget)
        acceptsNavigationCallbacks = true
        isLoading = true
        controllerPolicyPending = true
        activeNavigation = webViewFor(url: url).load(URLRequest(url: url))
    }

    private func discardActiveNavigation() {
        if let navigation = activeNavigation { discardedNavigations.add(navigation) }
        activeNavigation = nil
        controllerPolicyPending = false
    }

    private func rememberPendingNavigation(_ url: URL, historyTarget: UUID?) {
        let approved = approvedURL == url || (pendingNavigation?.url == url && pendingNavigation?.wasApproved == true)
        pendingNavigation = PendingNavigation(url: url, historyTarget: historyTarget,
            accountID: model?.accountStore.activeAccount?.id, accountAddress: model?.address.lowercased() ?? "",
            chainID: model?.networkChainId, wasApproved: approved)
    }

    private func navigationAllowed(_ url: URL) -> Bool {
        if url.scheme?.lowercased() == AppBrowserIdentity.scheme {
            guard browserOrigin(for: url) != nil,
                  appBundle?.documentPath(for: url, appKey: appIdentity?.appKey ?? "") != nil else {
                notice = String(localized: "This app has not been verified. Open its sea:// name first.")
                return false
            }
            return true
        }
        guard BrowserCanonicalOrigin.string(for: url) != nil else {
            notice = String(localized: "That is not a web address.")
            return false
        }
        switch BrowserOriginPolicy.classify(url) {
        case .bundled: return true
        case .external(let host, let lookalike, let punycode):
            let imitation = phishingCheck?(url) ?? lookalike
            if approvedURL == url { return true }
            if imitation != nil || (!BrowserOriginPolicy.isCurated(host) && !acknowledged.contains(host.lowercased())) {
                warning = SiteWarning(url: url, host: host, lookalike: imitation, punycode: punycode)
                return false
            }
            return true
        case .blocked(let why):
            notice = why
            return false
        }
    }

    /// The curated home's explorer entry.
    func openExplorer() {
        load(URL(string: "\(BrowserOriginPolicy.bundledScheme)://explorer/index.html")!)
    }

    func goBack() {
        if searchQuery != nil { dismissSearch(); return }
        guard navigationIndex > 0 else { return }
        navigate(to: navigationItems[navigationIndex - 1])
    }

    func goForward() {
        guard navigationIndex + 1 < navigationItems.count else { return }
        navigate(to: navigationItems[navigationIndex + 1])
    }

    func navigate(to item: NavigationItem) {
        guard !suspended, navigationItems.contains(where: { $0.id == item.id }) else { return }
        dismissSearch()
        navigationTarget = item.id
        guard let url = item.url else {
            showHome(historyTarget: item.id)
            return
        }
        if Self.isActionLink(url) {
            performLoad(url, historyTarget: item.id)
            return
        }
        guard navigationAllowed(url) else { return }
        // Retain WebKit's live document whenever the correct host's view has
        // this entry. Cross-host history still works after a store rebuild.
        if let view = webView,
           let entry = uniqueNativeEntry(url: url, in: view) {
            discardActiveNavigation()
            invalidateDocument()
            rememberPendingNavigation(url, historyTarget: item.id)
            acceptsNavigationCallbacks = true
            isLoading = true
            controllerPolicyPending = true
            activeNavigation = view.go(to: entry)
        } else {
            performLoad(url, historyTarget: item.id)
        }
    }

    private func uniqueNativeEntry(url: URL, in view: WKWebView) -> WKBackForwardListItem? {
        let current = view.backForwardList.currentItem.map { [$0] } ?? []
        let matches = (view.backForwardList.backList + current + view.backForwardList.forwardList)
            .filter { $0.url == url }
        guard matches.count == 1, matches[0] !== view.backForwardList.currentItem else { return nil }
        return matches[0]
    }

    func goHome() { showHome(historyTarget: nil) }

    private func showHome(historyTarget: UUID?) {
        guard !suspended, !closed else { return }
        dismissSearch()
        cancelContentResolution()
        pendingNavigation = nil
        interruptedNavigation = nil
        discardActiveNavigation()
        acceptsNavigationCallbacks = false
        invalidateDocument()
        releaseWebView()
        appIdentity = nil
        appBundle = nil
        currentURL = nil
        addressField = ""
        title = String(localized: "Start page")
        canonicalOrigin = ""
        isSecureOrigin = false
        isLoading = false
        estimatedProgress = 0
        warning = nil
        notice = nil
        httpsOffer = nil
        navigationTarget = historyTarget
        recordNavigation(url: nil, title: title)
        refreshSitePermissions()
        onHome?()
    }

    func reload() {
        if let query = searchQuery { search(query); return }
        guard !suspended, !closed, let url = currentURL else { return }
        guard let view = webView, let documentURL = view.url else {
            performLoad(url, historyTarget: nil)
            return
        }
        guard navigationAllowed(documentURL) else { return }
        cancelContentResolution()
        interruptedNavigation = nil
        navigationTarget = nil
        discardActiveNavigation()
        invalidateDocument()
        rememberPendingNavigation(documentURL, historyTarget: nil)
        acceptsNavigationCallbacks = true
        isLoading = true
        controllerPolicyPending = true
        activeNavigation = view.reload()
    }

    func stop() {
        if searchQuery != nil { dismissSearch(); return }
        cancelContentResolution()
        pendingNavigation = nil
        interruptedNavigation = nil
        discardActiveNavigation()
        acceptsNavigationCallbacks = false
        invalidateDocument()
        webView?.stopLoading()
        isLoading = false
        if let view = webView, let url = view.url, displayURL(for: url) == currentURL {
            committedOrigin = browserOrigin(for: url)
            refreshSitePermissions()
        }
    }

    private func recordNavigation(url: URL?, title: String) {
        if let target = navigationTarget,
           let index = navigationItems.firstIndex(where: { $0.id == target }) {
            navigationIndex = index
            navigationItems[index] = NavigationItem(id: target, url: url, title: title)
        } else if navigationItems.indices.contains(navigationIndex), navigationItems[navigationIndex].url == url {
            navigationItems[navigationIndex].title = title
        } else {
            navigationItems = Array(navigationItems.prefix(navigationIndex + 1))
            navigationItems.append(NavigationItem(id: UUID(), url: url, title: title))
            navigationIndex = navigationItems.count - 1
            if navigationItems.count > 100 {
                navigationItems.removeFirst(navigationItems.count - 100)
                navigationIndex = navigationItems.count - 1
            }
        }
        navigationTarget = nil
        canGoBack = navigationIndex > 0
        canGoForward = navigationIndex + 1 < navigationItems.count
    }

    private func updateTitle(_ pageTitle: String?, notify: Bool = true) {
        guard let url = currentURL else { return }
        let next = pageTitle.flatMap { $0.isEmpty ? nil : $0 } ?? url.host ?? ""
        let changed = title != next
        title = next
        if navigationItems.indices.contains(navigationIndex), navigationItems[navigationIndex].url == url {
            navigationItems[navigationIndex].title = title
        }
        if changed, notify, !isPrivate, appIdentity?.isDeveloper != true { onTitleChange?(url, title) }
    }

    /// Install only an immutable fully checked bundle (or an explicitly
    /// selected developer folder). The app never receives a filesystem URL.
    func openAppBundle(_ bundle: AppBundle, identity: AppBrowserIdentity, path: String = "/", query: String? = nil) throws {
        guard !closed, !suspended, identity.chainID == model?.networkChainId,
              bundle.isVerified || (identity.isDeveloper && developerModeEnabled) else {
            throw AppBrowserIdentity.Failure.invalidIdentity
        }
        _ = try AppBundleScheme(bundle: bundle, appKey: identity.appKey,
                                allowUnverifiedDeveloperContent: identity.isDeveloper)
        let url = try bundle.pageURL(appKey: identity.appKey, path: path, query: query)
        cancelContentResolution()
        appIdentity = identity
        appBundle = bundle
        notice = nil
        performLoad(url, historyTarget: navigationTarget)
    }

    private var developerModeEnabled: Bool { UserDefaults.standard.bool(forKey: "developerMode") }

    #if os(macOS)
    func openLocalAppFolder() {
        guard developerModeEnabled, !closed, !suspended else { return }
        let requestID = navigationID
        let accountID = model?.accountStore.activeAccount?.id
        let accountAddress = model?.address.lowercased() ?? ""
        let chainID = model?.networkChainId
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = String(localized: "Open a local app folder")
        panel.begin { [weak self] response in
            guard response == .OK, let folder = panel.url else { return }
            Task { @MainActor in
                guard let self, self.developerModeEnabled, !self.closed, !self.suspended,
                      self.navigationID == requestID,
                      self.model?.accountStore.activeAccount?.id == accountID,
                      self.model?.address.lowercased() == accountAddress,
                      self.model?.networkChainId == chainID else { return }
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
        accountDidChange()
        refreshSearchForNode()
    }

    private func cancelContentResolution() {
        if contentLoading { pendingNavigation = nil }
        contentTask?.cancel()
        contentTask = nil
        contentLoading = false
        navigationID = UUID()
    }

    /// The user acknowledged the warning: remember the host (once per site,
    /// per device) and load what was refused.
    func approveWarning(id: UUID? = nil) {
        guard let w = warning, id == nil || id == w.id else { return }
        warning = nil
        var ack = acknowledged
        ack.insert((w.url.host ?? w.host).lowercased())
        acknowledged = ack
        approvedURL = w.url
        performLoad(w.url, historyTarget: navigationTarget)
    }

    /// Sea names are checked before the resolver seam runs, so an unresolved
    /// lookalike still raises the same native warning as an HTTPS hostname.
    func confirmSeaName(_ url: URL) -> Bool {
        guard !suspended, url.scheme?.lowercased() == "sea" else { return false }
        if approvedURL == url { approvedURL = nil; return true }
        guard let lookalike = phishingCheck?(url) else { return true }
        let host = BrowserCanonicalOrigin.host(for: url) ?? url.host ?? ""
        let punycode = host.lowercased().split(separator: ".").contains { $0.hasPrefix("xn--") }
        warning = SiteWarning(url: url, host: host, lookalike: lookalike, punycode: punycode)
        return false
    }

    func refuseWarning(id: UUID? = nil) {
        guard id == nil || id == warning?.id else { return }
        warning = nil
        navigationTarget = nil
        approvedURL = nil
        notice = String(localized: "Not opened.")
    }

    // MARK: - Find, zoom, and page identity

    func find(_ query: String, backwards: Bool = false) {
        findQuery = query
        findFound = nil
        guard !query.isEmpty, let view = webView else { return }
        let request = UUID()
        findRequest = request
        let configuration = WKFindConfiguration()
        configuration.backwards = backwards
        configuration.wraps = true
        view.find(query, configuration: configuration) { [weak self, weak view] result in
            guard let self, let view, self.webView === view, self.findRequest == request else { return }
            self.findFound = result.matchFound
        }
    }

    func clearFind() {
        findRequest = UUID()
        findQuery = ""
        findFound = nil
        webView?.find("", configuration: WKFindConfiguration()) { _ in }
    }

    func zoomIn() { setZoom(zoomFactor + 0.1) }
    func zoomOut() { setZoom(zoomFactor - 0.1) }
    func resetZoom() { setZoom(1) }

    func setZoom(_ factor: Double) {
        guard factor.isFinite, !canonicalOrigin.isEmpty else { return }
        zoomFactor = min(3, max(0.5, factor))
        webView?.pageZoom = CGFloat(zoomFactor)
        if isPrivate { privateZoom[canonicalOrigin] = zoomFactor }
        else if appIdentity?.isDeveloper != true { onZoomChange?(zoomOrigin, zoomFactor) }
    }

    private var zoomOrigin: String {
        currentURL.flatMap(BrowserCanonicalOrigin.string(for:)) ?? canonicalOrigin
    }

    private func restoreZoom() {
        let factor = privateZoom[canonicalOrigin] ?? zoomProvider?(zoomOrigin) ?? 1
        zoomFactor = factor.isFinite ? min(3, max(0.5, factor)) : 1
        webView?.pageZoom = CGFloat(zoomFactor)
    }

    private func commitURL(_ url: URL, in view: WKWebView) {
        guard let origin = browserOrigin(for: url) else { return }
        let displayedURL = displayURL(for: url)
        pendingNavigation?.url = url
        currentURL = displayedURL
        committedOrigin = origin
        canonicalOrigin = origin
        isSecureOrigin = url.scheme?.lowercased() == "https"
            || (appIdentity?.isDeveloper == false && appBundle?.isVerified == true)
        addressField = urlBarText(url)
        lastReadVerified = true
        updateTitle(view.title, notify: false)
        recordNavigation(url: displayedURL, title: title)
        restoreZoom()
        refreshSitePermissions()
        if !isPrivate, appIdentity?.isDeveloper != true { onCommit?(displayedURL, title) }
    }

    private func invalidateDocument() {
        documentGeneration &+= 1
        navigationID = UUID()
        committedOrigin = nil
        findRequest = UUID()
        findFound = nil
        cancelPendingAsks()
    }

    private static func isActionLink(_ url: URL) -> Bool {
        ["sea", "aether", "eastsea"].contains(url.scheme?.lowercased() ?? "")
    }

    private func handleActionLink(_ url: URL, hasUserGesture: Bool) {
        guard BrowserSeaActionPolicy.allows(url, hasUserGesture: hasUserGesture) else {
            notice = String(localized: "Automatic wallet action links are blocked. Open the link yourself to continue.")
            return
        }
        if let onSeaLink { onSeaLink(url) }
        else { open(url.absoluteString) }
    }

    fileprivate func handleTrustedAction(_ message: WKScriptMessage) {
        guard validatedBridgeOrigin(message) != nil, let body = message.body as? [String: String],
              let value = body["url"], let url = URL(string: value), Self.isActionLink(url) else { return }
        handleActionLink(url, hasUserGesture: true)
    }

    private func validatedBridgeOrigin(_ message: WKScriptMessage) -> String? {
        guard !suspended, !closed, searchQuery == nil, searchRecord == nil, message.frameInfo.isMainFrame,
              let view = message.webView, view === webView,
              let url = view.url, let expected = committedOrigin,
              browserOrigin(for: url) == expected else { return nil }
        let actual = message.frameInfo.securityOrigin
        if let identity = appIdentity {
            guard identity.accepts(scheme: actual.protocol, host: actual.host, port: Int(actual.port),
                                   mainFrame: true, currentChainID: model?.networkChainId ?? 0,
                                   developerMode: developerModeEnabled) else { return nil }
            return identity.permissionKey == expected ? expected : nil
        }
        let key = BrowserCanonicalOrigin.string(scheme: actual.protocol, host: actual.host, port: Int(actual.port))
        return key == expected ? key : nil
    }

    private func browserOrigin(for url: URL) -> String? {
        guard url.scheme?.lowercased() == AppBrowserIdentity.scheme else {
            return BrowserCanonicalOrigin.string(for: url)
        }
        guard let identity = appIdentity, let bundle = appBundle,
              bundle.documentPath(for: url, appKey: identity.appKey) != nil,
              bundle.isVerified || (identity.isDeveloper && developerModeEnabled),
              identity.accepts(scheme: url.scheme ?? "", host: url.host ?? "", port: url.port ?? 0,
                               mainFrame: true, currentChainID: model?.networkChainId ?? 0,
                               developerMode: developerModeEnabled) else { return nil }
        return identity.permissionKey
    }

    /// Reopening a history entry resolves its name again; a raw app authority
    /// never becomes a bookmark or a transferable claim of certification.
    private func displayURL(for url: URL) -> URL {
        guard let identity = appIdentity, !identity.isDeveloper,
              url.scheme?.lowercased() == AppBrowserIdentity.scheme,
              var parts = URLComponents(string: identity.displayOrigin),
              let document = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return url }
        parts.percentEncodedPath = url.path == "/\(appBundle?.entryPoint ?? "index.html")" ? "/" : document.percentEncodedPath
        parts.percentEncodedQuery = document.percentEncodedQuery
        return parts.url ?? url
    }

    private func bridgeContext(_ message: WKScriptMessage) -> (key: String, displayOrigin: String)? {
        guard let key = validatedBridgeOrigin(message) else { return nil }
        return (key, appIdentity?.displayOrigin ?? message.frameInfo.securityOrigin.host)
    }

    private func connectedAddress(origin: String) -> String? {
        guard let model, !model.exploreLocked else { return nil }
        if isPrivate {
            guard let address = privatePermissions[origin], address.lowercased() == model.address.lowercased() else { return nil }
            return address
        }
        return model.connectedSiteAddress(origin: origin)
    }

    func refreshSitePermissions() {
        connectedAccount = committedOrigin.flatMap { connectedAddress(origin: $0) }
        let requested = allowances[canonicalOrigin] ?? []
        requestedAllowances = SiteAllowance.allCases.filter { requested.contains($0) }.map(\.label)
        tokenAllowances = requestedTokenAllowances[canonicalOrigin] ?? []
    }

    private func requestAllowance(_ allowance: SiteAllowance, origin: String) {
        allowances[origin, default: []].insert(allowance)
        refreshSitePermissions()
    }

    func disconnectSite() {
        guard let origin = committedOrigin else { return }
        disconnect(origin: origin)
    }

    private func disconnect(origin: String) {
        cancelPendingAsks()
        if isPrivate {
            privatePermissionGeneration &+= 1
            privatePermissions.removeValue(forKey: origin)
        }
        else { model?.revokeSitePermission(origin: origin) }
        refreshSitePermissions()
    }

    // MARK: - Sheet answers

    /// The user approved the sheet: the site may see this address, or the
    /// reviewed transaction/message enters the owner signing flow.
    func approveAsk(id: String? = nil, extraConfirmation: Bool = false) {
        guard let pending = ask, id == nil || id == pending.id, let model, !approvalBusy else { return }
        guard approvalIsCurrent(pending) else {
            finish(pending, .failure(staleRequestError()))
            return
        }
        switch pending.kind {
        case .connect(let origin, let displayOrigin):
            if isPrivate {
                privatePermissionGeneration &+= 1
                privatePermissions[origin] = pending.accountAddress
            } else {
                model.grantSitePermission(origin: origin, address: pending.accountAddress, displayOrigin: displayOrigin)
            }
            refreshSitePermissions()
            finish(pending, .success([pending.accountAddress]))
        case .send(let origin, let host, let tx, let feeWei):
            guard signingApprovalIsCurrent(pending) else {
                finish(pending, .failure(staleRequestError()))
                return
            }
            guard let reviewed = simulation, reviewed.context == pending.context,
                  reviewed.transaction.to == tx.to, reviewed.transaction.valueWei == tx.valueWei,
                  reviewed.transaction.data == tx.data, tx.gas == 0 || reviewed.transaction.gas == tx.gas,
                  reviewed.result.canSign(extraConfirmation: extraConfirmation), !model.busy else { return }
            approvalBusy = true
            busyAskId = pending.id
            Task {
                defer { endApproval(pending) }
                do {
                    let latest = try await model.simulatePageTransaction(reviewed.transaction, context: reviewed.context)
                    guard ask?.id == pending.id else { return }
                    guard signingApprovalIsCurrent(pending) else {
                        finish(pending, .failure(staleRequestError()))
                        return
                    }
                    guard latest == reviewed else {
                        simulation = latest
                        approvalNotice = String(localized: "The simulation changed. Review the new result before signing.")
                        return
                    }
                    let (hash, refusal) = await model.sendPageTransaction(latest.transaction, origin: origin, title: host,
                                                                        shownFeeWei: feeWei, simulation: latest,
                                                                        extraConfirmation: extraConfirmation,
                                                                        stillApproved: {
                                                                            self.ask?.id == pending.id && self.signingApprovalIsCurrent(pending)
                                                                        }, beginSubmission: {
                                                                            guard pending.lifecycle.beginSubmission(stillApproved:
                                                                                self.ask?.id == pending.id && self.signingApprovalIsCurrent(pending)) else { return false }
                                                                            self.submissionInFlight = true
                                                                            return true
                                                                        })
                    finish(pending, hash.map { .success($0) } ?? .failure(ProviderError(code: ProviderErrorCode.internalError,
                                                                                    message: refusal ?? String(localized: "The send failed."))))
                } catch {
                    guard ask?.id == pending.id else { return }
                    guard signingApprovalIsCurrent(pending) else {
                        finish(pending, .failure(staleRequestError()))
                        return
                    }
                    if let error = error as? ProviderError, error.code == 4901 {
                        finish(pending, .failure(error))
                        return
                    }
                    simulation = nil
                    approvalNotice = String(localized: "The node could not simulate this transaction.")
                }
            }
        case .typed(let origin, _, let prepared, _):
            guard signingApprovalIsCurrent(pending), let context = pending.context, !model.busy else {
                finish(pending, .failure(staleRequestError()))
                return
            }
            approvalBusy = true
            busyAskId = pending.id
            Task {
                defer { endApproval(pending) }
                do {
                    let signature = try await model.signPageTypedMessage(prepared, origin: origin, context: context,
                                                                         stillApproved: {
                                                                             self.ask?.id == pending.id && self.signingApprovalIsCurrent(pending)
                                                                         })
                    finish(pending, .success(signature))
                } catch let error as ProviderError { finish(pending, .failure(error)) }
                catch { finish(pending, .failure(ProviderError(code: ProviderErrorCode.params, message: WalletModel.ffiMessage(error)))) }
            }
        }
    }

    func simulateAsk(_ pending: PendingAsk) async {
        guard let model, case .send(_, _, let tx, _) = pending.kind, ask?.id == pending.id,
              let context = pending.context else { return }
        guard signingApprovalIsCurrent(pending) else {
            finish(pending, .failure(staleRequestError()))
            return
        }
        simulation = nil
        approvalNotice = nil
        simulationLoading = true
        defer { if ask?.id == pending.id { simulationLoading = false } }
        do {
            let result = try await model.simulatePageTransaction(tx, context: context)
            guard ask?.id == pending.id else { return }
            guard signingApprovalIsCurrent(pending) else {
                finish(pending, .failure(staleRequestError()))
                return
            }
            simulation = result
        } catch {
            guard ask?.id == pending.id else { return }
            guard signingApprovalIsCurrent(pending) else {
                finish(pending, .failure(staleRequestError()))
                return
            }
            if let error = error as? ProviderError, error.code == 4901 {
                finish(pending, .failure(error))
                return
            }
            approvalNotice = String(localized: "The node could not simulate this transaction.")
        }
    }

    private func finish(_ pending: PendingAsk, _ result: Result<Any?, ProviderError>) {
        guard ask?.id == pending.id else {
            pending.reply(result)
            return
        }
        endApproval(pending)
        ask = nil
        simulation = nil
        approvalNotice = nil
        simulationLoading = false
        pending.reply(result)
        drainAskQueue()
    }

    private func approvalIsCurrent(_ pending: PendingAsk) -> Bool {
        guard !suspended, !closed, searchQuery == nil, searchRecord == nil, let model, !model.exploreLocked,
              !pending.accountAddress.isEmpty,
              model.accountStore.activeAccount?.id == pending.accountID,
              model.address.lowercased() == pending.accountAddress,
              model.networkChainId == pending.chainID,
              documentGeneration == pending.documentGeneration,
              !isPrivate || privatePermissionGeneration == pending.privatePermissionGeneration,
              let context = pending.context, model.isCurrentDappContext(context),
              committedOrigin == pending.origin,
              let url = webView?.url,
              browserOrigin(for: url) == pending.origin else { return false }
        return true
    }

    /// This controller owns private grants and the current document; the model
    /// rechecks this closure after every asynchronous preparation before signing.
    private func signingApprovalIsCurrent(_ pending: PendingAsk) -> Bool {
        approvalIsCurrent(pending)
            && appIdentity?.permitsSigning(developerMode: developerModeEnabled) != false
            && connectedAddress(origin: pending.origin)?.lowercased() == pending.accountAddress
    }

    private static func changedPageError() -> ProviderError {
        ProviderError(code: ProviderErrorCode.denied,
                      message: String(localized: "This page changed before you approved the request. Try again on the current page."))
    }

    private func staleRequestError() -> ProviderError { Self.changedPageError() }

    private func cancelPendingAsks() {
        let pending = ask.map { [$0] } ?? []
        ask = nil
        let queued = askQueue
        askQueue.removeAll()
        busyAskId = nil
        approvalBusy = false
        submissionInFlight = false
        simulation = nil
        approvalNotice = nil
        simulationLoading = false
        for request in pending + queued { request.lifecycle.cancel(staleRequestError()) }
    }

    private func endApproval(_ pending: PendingAsk) {
        guard busyAskId == pending.id else { return }
        busyAskId = nil
        approvalBusy = false
        submissionInFlight = false
    }

    /// Refusal before submission: 4001, nothing broadcast.
    func refuseAsk(id: String? = nil) {
        guard let pending = ask, id == nil || id == pending.id, pending.lifecycle.canCancel else { return }
        finish(pending, .failure(ProviderError(code: ProviderErrorCode.denied, message: "The user rejected the request.")))
    }

    func dismissAsk(_ pending: PendingAsk) {
        if ask?.id == pending.id { refuseAsk(id: pending.id) }
        else {
            if ask == nil {
                endApproval(pending)
                simulation = nil
                approvalNotice = nil
                simulationLoading = false
            }
            pending.lifecycle.cancel(ProviderError(code: ProviderErrorCode.denied, message: "The user rejected the request."))
            drainAskQueue()
        }
    }

    /// One sheet at a time: whatever queued while one was open comes next.
    private func drainAskQueue() {
        guard ask == nil else { return }
        while !askQueue.isEmpty {
            let next = askQueue.removeFirst()
            if approvalIsCurrent(next) { ask = next; return }
            next.reply(.failure(staleRequestError()))
        }
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
        guard let source = message.webView, source === webView else {
            reply(.failure(ProviderError(code: ProviderErrorCode.locked, message: "the tab is gone")))
            return
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
        guard let context = bridgeContext(message) else {
            reply(.failure(staleRequestError()))
            return
        }
        let key = context.key
        let params = (body["params"] as? [Any]) ?? []

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
            case .typed(let o, _, _, _): return o
            }
        }
        let pendingForOrigin = askQueue.filter { originOf($0) == key }.count
            + (ask.flatMap(originOf) == key ? 1 : 0) + preparingAsks[key, default: 0]
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
            reply(.success(connectedAddress(origin: key).map { [$0] } ?? []))
        case .requestAccounts:
            requestAllowance(.account, origin: key)
            if let addr = connectedAddress(origin: key) {
                reply(.success([addr]))
            } else {
                enqueue(.connect(origin: key, host: context.displayOrigin), id: body["id"], reply: reply)
            }
        case .disconnect:
            disconnect(origin: key)
            reply(.success(NSNull()))
        case .send:
            guard appIdentity?.permitsSigning(developerMode: developerModeEnabled) != false else {
                reply(.failure(ProviderError(code: ProviderErrorCode.denied,
                                             message: "Local app files can sign only on the development network.")))
                return
            }
            guard let connected = connectedAddress(origin: key) else {
                reply(.failure(ProviderError(code: ProviderErrorCode.locked,
                                             message: "This site is not connected to an account.")))
                return
            }
            switch PageTransaction.parse(params.first, from: connected) {
            case .success(let tx):
                requestAllowance(.transactions, origin: key)
                if tx.data.lowercased().hasPrefix("0x095ea7b3") {
                    requestAllowance(.tokenApproval, origin: key)
                    if let allowance = BrowserTokenAllowance.parse(token: tx.to, data: tx.data) {
                        var entries = requestedTokenAllowances[key] ?? []
                        entries.removeAll { $0.token == allowance.token && $0.spender == allowance.spender }
                        entries.append(allowance)
                        requestedTokenAllowances[key] = Array(entries.suffix(20))
                        refreshSitePermissions()
                    }
                }
                guard let dappContext = model.dappContext else {
                    reply(.failure(ProviderError(code: ProviderErrorCode.locked, message: String(localized: "This approval is no longer valid. Ask the site to try again."))))
                    return
                }
                let gas = tx.gas == 0 ? (tx.isPlainTransfer ? UInt64(100_000) : UInt64(3_000_000)) : tx.gas
                let host = context.displayOrigin
                let accountID = model.accountStore.activeAccount?.id
                let chainID = model.networkChainId
                let transferFeeWei = tx.isPlainTransfer ? model.status?.transferFeeWei : nil
                let pageGeneration = documentGeneration
                let permissionGeneration = privatePermissionGeneration
                preparingAsks[key, default: 0] += 1
                Task {
                    defer { preparingAsks[key, default: 0] -= 1 }
                    let quotedFeeWei = await Task.detached { (try? dappTransactionQuote(to: tx.to, valueWei: tx.valueWei, dataHex: tx.data, gasLimit: gas))?.feeWei }.value
                    let feeWei = quotedFeeWei ?? transferFeeWei
                    let pending = PendingAsk(id: UUID().uuidString, kind: .send(origin: key, host: host, tx: tx, feeWei: feeWei),
                                             reply: reply, accountID: accountID, accountAddress: connected.lowercased(),
                                             documentGeneration: pageGeneration, chainID: chainID,
                                             privatePermissionGeneration: permissionGeneration, context: dappContext)
                    guard signingApprovalIsCurrent(pending) else {
                        pending.reply(.failure(staleRequestError()))
                        return
                    }
                    enqueue(pending)
                }
            case .failure(let e):
                reply(.failure(e))
            }
        case .typed:
            guard appIdentity?.permitsSigning(developerMode: developerModeEnabled) != false else {
                reply(.failure(ProviderError(code: ProviderErrorCode.denied,
                                             message: "Local app files can sign only on the development network.")))
                return
            }
            guard let dappContext = model.dappContext, let connected = connectedAddress(origin: key) else {
                reply(.failure(ProviderError(code: ProviderErrorCode.locked, message: String(localized: "This approval is no longer valid. Ask the site to try again."))))
                return
            }
            let host = context.displayOrigin
            let accountID = model.accountStore.activeAccount?.id
            let chainID = model.networkChainId
            let pageGeneration = documentGeneration
            let permissionGeneration = privatePermissionGeneration
            preparingAsks[key, default: 0] += 1
            Task {
                defer { preparingAsks[key, default: 0] -= 1 }
                do {
                    let json = try TypedMessageRequest.parse(params, context: dappContext)
                    let prepared = try await model.preparePageTypedMessage(json)
                    let fields = try TypedMessageFields.parse(prepared.typedDataJson)
                    let pending = PendingAsk(id: UUID().uuidString, kind: .typed(origin: key, host: host, prepared: prepared, fields: fields),
                                             reply: reply, accountID: accountID, accountAddress: connected.lowercased(),
                                             documentGeneration: pageGeneration, chainID: chainID,
                                             privatePermissionGeneration: permissionGeneration, context: dappContext)
                    guard signingApprovalIsCurrent(pending) else {
                        pending.reply(.failure(staleRequestError()))
                        return
                    }
                    enqueue(pending)
                } catch let error as ProviderError { reply(.failure(error)) }
                catch { reply(.failure(ProviderError(code: ProviderErrorCode.params, message: WalletModel.ffiMessage(error)))) }
            }
        case .read(let verified):
            requestAllowance(verified ? .verifiedRead : .unverifiedRead, origin: key)
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
        guard let source = message.webView, source === webView else {
            reply(.failure(ProviderError(code: ProviderErrorCode.locked, message: "the tab is gone")))
            return
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
        guard let key = validatedBridgeOrigin(message) else {
            reply(.failure(staleRequestError()))
            return
        }
        let origin = message.frameInfo.securityOrigin
        guard appIdentity != nil || VerifyBridge.allows(scheme: origin.protocol, connected: connectedAddress(origin: key) != nil) else {
            reply(.failure(ProviderError(code: ProviderErrorCode.unsupported,
                                         message: "This page cannot use EastSea verification.")))
            return
        }
        requestAllowance(.verifiedRead, origin: key)
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

    private func enqueue(_ kind: PendingAsk.Kind, id _: Any?, reply: @escaping (Result<Any?, ProviderError>) -> Void) {
        let pending = PendingAsk(id: UUID().uuidString,
                                 kind: kind, reply: reply, accountID: model?.accountStore.activeAccount?.id,
                                 accountAddress: model?.address.lowercased() ?? "", documentGeneration: documentGeneration,
                                 chainID: model?.networkChainId ?? 0,
                                 privatePermissionGeneration: privatePermissionGeneration, context: model?.dappContext)
        enqueue(pending)
    }

    private func enqueue(_ pending: PendingAsk) {
        guard approvalIsCurrent(pending) else {
            pending.reply(.failure(staleRequestError()))
            return
        }
        if ask == nil { ask = pending } else { askQueue.append(pending) }
    }

    // MARK: - Downloads

    func approveDownload(id: UUID? = nil) {
        guard let prompt = downloadPrompt,
              id == nil || id == prompt.id,
              let index = pendingDownloads.firstIndex(where: { $0.prompt.id == prompt.id }) else { return }
        let pending = pendingDownloads.remove(at: index)
        downloadPrompt = nil
        do {
            let manager = FileManager.default
            #if os(macOS)
            let downloads = try manager.url(for: .downloadsDirectory, in: .userDomainMask,
                                            appropriateFor: nil, create: true)
            #else
            let documents = try manager.url(for: .documentDirectory, in: .userDomainMask,
                                            appropriateFor: nil, create: true)
            let downloads = documents.appendingPathComponent("Downloads", isDirectory: true)
            try manager.createDirectory(at: downloads, withIntermediateDirectories: true)
            #endif
            let stagingRoot = downloads.appendingPathComponent(".eastsea-downloads", isDirectory: true)
            // Do not follow a pre-existing symlink into an unrelated folder.
            if manager.fileExists(atPath: stagingRoot.path) {
                let values = try stagingRoot.resourceValues(forKeys: [.isSymbolicLinkKey, .isDirectoryKey])
                guard values.isSymbolicLink != true, values.isDirectory == true else { throw CocoaError(.fileWriteInvalidFileName) }
            }
            let staging = stagingRoot.appendingPathComponent(prompt.id.uuidString, isDirectory: true)
            try manager.createDirectory(at: staging, withIntermediateDirectories: true)
            let file = staging.appendingPathComponent(prompt.filename, isDirectory: false)
            activeDownloads[ObjectIdentifier(pending.download)] = ActiveDownload(download: pending.download,
                stagingDirectory: staging, stagedFile: file, downloadsDirectory: downloads, filename: prompt.filename)
            pending.destination(file)
        } catch {
            pending.destination(nil)
            pending.download.cancel { _ in }
            notice = String(localized: "The download could not be saved.")
        }
        drainDownloads()
    }

    func refuseDownload(id: UUID? = nil) {
        guard let prompt = downloadPrompt,
              id == nil || id == prompt.id,
              let index = pendingDownloads.firstIndex(where: { $0.prompt.id == prompt.id }) else { return }
        let pending = pendingDownloads.remove(at: index)
        downloadPrompt = nil
        pending.destination(nil)
        pending.download.cancel { _ in }
        notice = String(localized: "Download canceled.")
        drainDownloads()
    }

    private func refusePendingDownloads() {
        let pending = pendingDownloads
        pendingDownloads.removeAll()
        downloadPrompt = nil
        for item in pending {
            item.destination(nil)
            item.download.cancel { _ in }
        }
    }

    private func drainDownloads() {
        if downloadPrompt == nil { downloadPrompt = pendingDownloads.first?.prompt }
    }

    private static func safeDownloadFilename(_ suggested: String) -> String {
        let basename = suggested.replacingOccurrences(of: "\\", with: "/").split(separator: "/").last.map(String.init) ?? "download"
        let clean = basename.unicodeScalars.filter {
            !CharacterSet.controlCharacters.contains($0) && !CharacterSet(charactersIn: ":/").contains($0)
                && !$0.properties.isDefaultIgnorableCodePoint
        }.map(String.init).joined().trimmingCharacters(in: .whitespacesAndNewlines)
        let name = String(clean.prefix(180))
        return name.isEmpty || name == "." || name == ".." || name.hasPrefix(".") ? "download" : name
    }

    private static func executableFilename(_ filename: String) -> Bool {
        let ext = URL(fileURLWithPath: filename).pathExtension.lowercased()
        return Set(["app", "dmg", "pkg", "exe", "msi", "com", "bat", "cmd", "sh", "command", "js", "jar", "workflow", "scpt"]).contains(ext)
    }

    private func downloadOrigin(for url: URL) -> String? {
        if case .external = BrowserOriginPolicy.classify(url) {
            return BrowserCanonicalOrigin.string(for: url)
        }
        if url.scheme?.lowercased() == "blob",
           let source = URL(string: String(url.absoluteString.dropFirst(5))),
           source.scheme?.lowercased() == "https",
           BrowserCanonicalOrigin.string(for: source) == committedOrigin {
            return committedOrigin
        }
        return nil
    }

    private func acceptDownload(_ download: WKDownload, from view: WKWebView) {
        guard view === webView, !suspended else { download.cancel { _ in }; return }
        download.delegate = self
    }

    private func startIsolatedDownload(_ url: URL) {
        guard !suspended, downloadOrigin(for: url) != nil else { return }
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        // No provider or bundled-page handler exists in a download-only view.
        // A cross-host download never enters the current page's cookie store.
        let view = WKWebView(frame: .zero, configuration: configuration)
        let generation = documentGeneration
        view.startDownload(using: URLRequest(url: url)) { [weak self, view] download in
            guard let self, !self.suspended, self.documentGeneration == generation else {
                download.cancel { _ in }
                return
            }
            self.downloadViews[ObjectIdentifier(download)] = view
            download.delegate = self
        }
    }

    private func finishDownload(_ download: WKDownload) {
        defer { downloadViews.removeValue(forKey: ObjectIdentifier(download)) }
        guard let active = activeDownloads.removeValue(forKey: ObjectIdentifier(download)) else { return }
        let manager = FileManager.default
        let original = URL(fileURLWithPath: active.filename)
        let ext = original.pathExtension
        let base = original.deletingPathExtension().lastPathComponent
        do {
            // moveItem does not overwrite. If another writer wins the race,
            // pick another name instead of deleting or replacing its file.
            for suffix in 0..<10_000 {
                let filename = suffix == 0 ? active.filename
                    : "\(base) (\(suffix))" + (ext.isEmpty ? "" : ".\(ext)")
                let destination = active.downloadsDirectory.appendingPathComponent(filename)
                if manager.fileExists(atPath: destination.path) { continue }
                do {
                    try manager.moveItem(at: active.stagedFile, to: destination)
                    try? manager.removeItem(at: active.stagingDirectory)
                    notice = String(localized: "Downloaded \(filename).")
                    return
                } catch let error as CocoaError where error.code == .fileWriteFileExists { continue }
            }
            throw CocoaError(.fileWriteFileExists)
        } catch {
            try? manager.removeItem(at: active.stagingDirectory)
            notice = String(localized: "The download could not be saved.")
        }
    }

    #if WALLET_SCREENS
    /// Native renderer state only: no WebView, network, cookies, or wallet
    /// permission writes are involved in screenshot fixtures.
    func configureScreenFixture(url: URL?, title: String, connectedAccount: String? = nil,
                                requestedAllowances: [String] = [], findQuery: String = "", findFound: Bool? = nil) {
        currentURL = url
        self.title = title
        canonicalOrigin = url.flatMap(BrowserCanonicalOrigin.string(for:)) ?? ""
        committedOrigin = canonicalOrigin.isEmpty ? nil : canonicalOrigin
        isSecureOrigin = url?.scheme?.lowercased() == "https"
        addressField = url?.absoluteString ?? ""
        self.connectedAccount = connectedAccount
        self.requestedAllowances = requestedAllowances
        self.findQuery = findQuery
        self.findFound = findFound
        if let url { recordNavigation(url: url, title: title) }
    }
    #endif

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

@MainActor
private final class WeakBrowserActionBridge: NSObject, WKScriptMessageHandler {
    weak var controller: BrowserController?
    init(controller: BrowserController) { self.controller = controller }
    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        controller?.handleTrustedAction(message)
    }
}

/// Controller-owned gestures traverse the same history as the buttons and
/// menus, including cross-host entries and repeated URLs. WebKit's independent
/// gesture traversal is disabled to keep the history index deterministic.
@MainActor
private final class BrowserHistoryWebView: WKWebView {
    var backAvailable: (() -> Bool)?
    var forwardAvailable: (() -> Bool)?
    var onBack: (() -> Void)?
    var onForward: (() -> Void)?

    #if os(macOS)
    private var trackingSwipe = false

    override func swipe(with event: NSEvent) {
        if event.deltaX > 0, backAvailable?() == true { onBack?() }
        else if event.deltaX < 0, forwardAvailable?() == true { onForward?() }
        else { super.swipe(with: event) }
    }

    override func scrollWheel(with event: NSEvent) {
        guard !trackingSwipe else { return }
        let backFallback = event.scrollingDeltaX > 0 && backAvailable?() == true
        let forwardFallback = event.scrollingDeltaX < 0 && forwardAvailable?() == true
        guard NSEvent.isSwipeTrackingFromScrollEventsEnabled,
              event.phase == .began || event.phase == .changed,
              abs(event.scrollingDeltaX) > abs(event.scrollingDeltaY),
              backFallback || forwardFallback else { super.scrollWheel(with: event); return }
        trackingSwipe = true
        event.trackSwipeEvent(options: [.lockDirection, .clampGestureAmount],
                             dampenAmountThresholdMin: -1, max: 1) { [weak self] amount, phase, complete, stop in
            guard let self else { stop.pointee = true; return }
            if phase == .cancelled { self.trackingSwipe = false; stop.pointee = true; return }
            guard complete else { return }
            self.trackingSwipe = false
            if amount > 0.5, self.backAvailable?() == true { self.onBack?() }
            else if amount < -0.5, self.forwardAvailable?() == true { self.onForward?() }
        }
    }
    #else
    override init(frame: CGRect, configuration: WKWebViewConfiguration) {
        super.init(frame: frame, configuration: configuration)
        installEdgeGestures()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        installEdgeGestures()
    }

    private func installEdgeGestures() {
        for edge in [UIRectEdge.left, UIRectEdge.right] {
            let gesture = UIScreenEdgePanGestureRecognizer(target: self, action: #selector(edgeSwipe(_:)))
            gesture.edges = edge
            gesture.delegate = self
            addGestureRecognizer(gesture)
        }
    }

    @objc private func edgeSwipe(_ gesture: UIScreenEdgePanGestureRecognizer) {
        guard gesture.state == .ended else { return }
        let distance = gesture.translation(in: self).x
        let speed = gesture.velocity(in: self).x
        if gesture.edges == .left, distance > 80 || speed > 400 { onBack?() }
        else if gesture.edges == .right, distance < -80 || speed < -400 { onForward?() }
    }
    #endif
}

#if !os(macOS)
extension BrowserHistoryWebView: UIGestureRecognizerDelegate {
    func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard let gesture = gestureRecognizer as? UIScreenEdgePanGestureRecognizer else { return false }
        if gesture.edges == .left { return backAvailable?() == true }
        return forwardAvailable?() == true
    }
}
#endif

extension BrowserController: WKNavigationDelegate, WKUIDelegate, WKDownloadDelegate {
    // MARK: - WKNavigationDelegate

    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                 preferences: WKWebpagePreferences) async -> WKNavigationActionPolicy {
        guard webView === self.webView, !suspended, searchQuery == nil, searchRecord == nil,
              let url = navigationAction.request.url else { return .cancel }
        if Self.isActionLink(url) {
            // Trusted anchor clicks are intercepted in an isolated world.
            // Reaching this delegate means an automatic/scripted action.
            handleActionLink(url, hasUserGesture: false)
            return .cancel
        }
        let mainFrame = navigationAction.targetFrame?.isMainFrame ?? true
        if appIdentity != nil {
            if url.scheme?.lowercased() == AppBrowserIdentity.scheme {
                guard navigationAllowed(url), !navigationAction.shouldPerformDownload else { return .cancel }
                if !mainFrame { return .allow }
            } else {
                guard mainFrame else { return .cancel }
                // Leaving an app also leaves its isolated store/content world,
                // even when the destination is the bundled explorer.
                restartNavigation(url, from: webView)
                return .cancel
            }
        }
        if navigationAction.shouldPerformDownload, downloadOrigin(for: url) != nil,
           url.scheme?.lowercased() == "blob" { return .download }
        if !mainFrame {
            // A subframe may neither replace the main frame's store nor
            // raise a native permission/warning on its behalf.
            switch BrowserOriginPolicy.classify(url) {
            case .bundled: return externalHost == nil ? .allow : .cancel
            case .external(let host, _, _): return externalHost == host.lowercased() ? .allow : .cancel
            case .blocked: return .cancel
            }
        }
        let controllerIssued = controllerPolicyPending
        controllerPolicyPending = false
        guard navigationAllowed(url) else { return .cancel }
        if navigationAction.shouldPerformDownload {
            if pendingNavigation?.url == url { pendingNavigation = nil }
            approvedURL = nil
            if url.host?.lowercased() != externalHost {
                startIsolatedDownload(url)
                return .cancel
            }
            return .download
        }
        if navigationAction.navigationType == .backForward, navigationTarget == nil {
            let matches = navigationItems.filter { $0.url == displayURL(for: url) }
            // Page-script history traversals expose only a URL on older SDKs.
            // An ambiguous URL cannot identify a controller history entry.
            if matches.count == 1 { navigationTarget = matches[0].id }
        }
        let nextHost: String?
        if case .external(let host, _, _) = BrowserOriginPolicy.classify(url) { nextHost = host.lowercased() }
        else { nextHost = nil }
        if nextHost != externalHost {
            restartNavigation(url, from: webView)
            return .cancel
        }
        if navigationAction.targetFrame == nil {
            restartNavigation(url, from: webView)
            return .cancel
        }
        if !controllerIssued {
            // A page reload can have the same URL and a different navigation.
            // The already-issued controller request consumed its own policy;
            // a later allowed action must adopt a newly started WebKit token.
            discardActiveNavigation()
            invalidateDocument()
        }
        if (navigationAction.request.httpMethod ?? "GET").uppercased() == "GET" {
            rememberPendingNavigation(url, historyTarget: navigationTarget)
        } else {
            // Do not replay form submissions or manufacture GET equivalents.
            pendingNavigation = nil
        }
        acceptsNavigationCallbacks = true
        approvedURL = nil
        return .allow
    }

    func webView(_ webView: WKWebView, decidePolicyFor navigationResponse: WKNavigationResponse) async -> WKNavigationResponsePolicy {
        guard webView === self.webView, !suspended, searchQuery == nil, searchRecord == nil,
              let url = navigationResponse.response.url else { return .cancel }
        if navigationResponse.isForMainFrame {
            if url.scheme?.lowercased() == AppBrowserIdentity.scheme {
                guard navigationAllowed(url) else { return .cancel }
            } else {
                guard appIdentity == nil else { return .cancel }
                switch BrowserOriginPolicy.classify(url) {
                case .blocked(let why): notice = why; return .cancel
                case .external(let host, _, _) where externalHost != host.lowercased():
                    // A redirect cannot silently bring another host into this
                    // store. Restart it in a newly isolated WebView.
                    restartNavigation(url, from: webView)
                    return .cancel
                case .bundled where externalHost != nil: return .cancel
                default: break
                }
            }
        }
        let attachment = (navigationResponse.response as? HTTPURLResponse)?
            .value(forHTTPHeaderField: "Content-Disposition")?.lowercased().contains("attachment") == true
        if attachment || (!navigationResponse.canShowMIMEType && navigationResponse.isForMainFrame) {
            if navigationResponse.isForMainFrame { pendingNavigation = nil }
            return downloadOrigin(for: url) == nil ? .cancel : .download
        }
        return .allow
    }

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        guard let navigation, webView === self.webView, !suspended, !closed, acceptsNavigationCallbacks,
              !discardedNavigations.contains(navigation),
              activeNavigation == nil || activeNavigation === navigation else { return }
        controllerPolicyPending = false
        activeNavigation = navigation
        invalidateDocument()
        isLoading = true
        estimatedProgress = webView.estimatedProgress
        refreshSitePermissions()
    }

    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        guard webView === self.webView, !suspended, !closed, acceptsNavigationCallbacks,
              activeNavigation === navigation, let url = webView.url else { return }
        commitURL(url, in: webView)
        notice = nil
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard webView === self.webView, !suspended, !closed, acceptsNavigationCallbacks,
              activeNavigation === navigation else { return }
        pendingNavigation = nil
        interruptedNavigation = nil
        activeNavigation = nil
        controllerPolicyPending = false
        isLoading = false
        estimatedProgress = 1
        updateTitle(webView.title)
        if !findQuery.isEmpty { find(findQuery) }
    }

    private func restartNavigation(_ url: URL, from source: WKWebView) {
        let generation = documentGeneration
        let target = navigationTarget
        Task { @MainActor [weak self, weak source] in
            guard let self, let source, !self.suspended, source === self.webView,
                  self.documentGeneration == generation else { return }
            self.performLoad(url, historyTarget: target)
        }
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        navigationFailed(in: webView, navigation: navigation, error: error)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        navigationFailed(in: webView, navigation: navigation, error: error)
    }

    private func navigationFailed(in view: WKWebView, navigation: WKNavigation?, error: Error) {
        guard view === webView, !suspended, !closed, acceptsNavigationCallbacks,
              activeNavigation === navigation else { return }
        activeNavigation = nil
        controllerPolicyPending = false
        isLoading = false
        if (error as NSError).code != NSURLErrorCancelled {
            pendingNavigation = nil
            interruptedNavigation = nil
            navigationTarget = nil
            notice = String(localized: "This page could not be loaded.")
        }
        // A failed provisional load can leave the previous document visible;
        // bind the bridge back to its real URL, never the failed request URL.
        if let url = view.url, currentURL == displayURL(for: url) {
            committedOrigin = browserOrigin(for: url)
            refreshSitePermissions()
        }
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        guard webView === self.webView, !closed else { return }
        pendingNavigation = nil
        interruptedNavigation = nil
        discardActiveNavigation()
        acceptsNavigationCallbacks = false
        invalidateDocument()
        isLoading = false
        notice = String(localized: "This page could not be loaded.")
    }

    func webView(_ webView: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) {
        acceptDownload(download, from: webView)
    }

    func webView(_ webView: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) {
        acceptDownload(download, from: webView)
    }

    // MARK: - WKDownloadDelegate

    func download(_ download: WKDownload, decideDestinationUsing response: URLResponse,
                  suggestedFilename: String, completionHandler: @escaping (URL?) -> Void) {
        guard !suspended, searchQuery == nil, searchRecord == nil, pendingDownloads.count < 3,
              let url = response.url ?? download.originalRequest?.url,
              let origin = downloadOrigin(for: url) else {
            completionHandler(nil)
            download.cancel { _ in }
            return
        }
        let filename = Self.safeDownloadFilename(suggestedFilename)
        let prompt = DownloadPrompt(url: url, filename: filename, origin: origin,
                                    isExecutable: Self.executableFilename(filename))
        pendingDownloads.append(PendingDownload(prompt: prompt, download: download, destination: completionHandler))
        drainDownloads()
    }

    func download(_ download: WKDownload, willPerformHTTPRedirection response: HTTPURLResponse,
                  newRequest request: URLRequest, decisionHandler: @escaping (WKDownload.RedirectPolicy) -> Void) {
        guard let url = request.url, url.scheme?.lowercased() == "https",
              downloadOrigin(for: url) != nil, phishingCheck?(url) == nil else {
            decisionHandler(.cancel)
            return
        }
        decisionHandler(.allow)
    }

    func downloadDidFinish(_ download: WKDownload) { finishDownload(download) }

    func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) {
        downloadViews.removeValue(forKey: ObjectIdentifier(download))
        if let active = activeDownloads.removeValue(forKey: ObjectIdentifier(download)) {
            try? FileManager.default.removeItem(at: active.stagingDirectory)
        }
        let waiting = pendingDownloads.filter { $0.download === download }
        pendingDownloads.removeAll { $0.download === download }
        for pending in waiting { pending.destination(nil) }
        if downloadPrompt != nil && !pendingDownloads.contains(where: { $0.prompt.id == downloadPrompt?.id }) {
            downloadPrompt = nil
            drainDownloads()
        }
        if (error as NSError).code != NSURLErrorCancelled { notice = String(localized: "The download could not be saved.") }
    }

    /// What the bar shows for a loaded page: the page's own URL, without a
    /// trailing slash theater.
    private func urlBarText(_ url: URL) -> String {
        if let identity = appIdentity, url.scheme == AppBrowserIdentity.scheme {
            var text = identity.displayOrigin
            if url.path != "/\(appBundle?.entryPoint ?? "index.html")" { text += url.path }
            if let query = url.query { text += "?\(query)" }
            return text
        }
        if url.scheme?.lowercased() == BrowserOriginPolicy.bundledScheme {
            return url.host == "explorer" ? String(localized: "Block explorer") : (url.host ?? "")
        }
        return url.absoluteString
    }

    // MARK: - WKUIDelegate

    /// No popups, no new windows: everything stays in this tab.
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        guard webView === self.webView, !suspended, searchQuery == nil, searchRecord == nil,
              navigationAction.sourceFrame.isMainFrame else { return nil }
        if navigationAction.targetFrame == nil, let url = navigationAction.request.url, Self.isActionLink(url) {
            handleActionLink(url, hasUserGesture: false)
        } else if navigationAction.targetFrame == nil, let url = navigationAction.request.url {
            load(url)   // a plain target=_blank link opens in the same tab
        }
        return nil
    }
}
