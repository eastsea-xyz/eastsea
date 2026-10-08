import Foundation
import WebKit

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
    @Published private(set) var simulation: SimulatedPageTransaction?
    @Published private(set) var approvalNotice: String?
    @Published private(set) var approvalBusy = false
    @Published private(set) var simulationLoading = false
    /// Whether the last read this tab answered was certificate-verified.
    @Published private(set) var lastReadVerified = true
    /// Bumped whenever the WebView had to be rebuilt (a store switch).
    @Published private(set) var webViewGeneration = 0
    @Published private(set) var canGoBack = false
    /// nil until something was loaded: while nil the curated home shows.
    @Published private(set) var currentURL: URL?

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
        let reply: (Result<Any?, ProviderError>) -> Void
        var context: DappRequestContext? = nil

        init(id: String, kind: Kind, reply: @escaping (Result<Any?, ProviderError>) -> Void,
             context: DappRequestContext? = nil) {
            self.id = id
            self.kind = kind
            self.context = context
            var answered = false
            self.reply = { result in
                guard !answered else { return }
                answered = true
                reply(result)
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
        let wantExternal: String?
        if case .external(let host, _, _)? = classification { wantExternal = host } else { wantExternal = nil }

        if let view = webView {
            if wantExternal == nil && externalHost == nil { return view }
            if let host = wantExternal, let current = externalHost, host == current { return view }
        }
        let config = WKWebViewConfiguration()
        config.setURLSchemeHandler(BundledPageScheme(root: BundledPageScheme.defaultRoot()
            ?? URL(fileURLWithPath: "/nonexistent")), forURLScheme: BrowserOriginPolicy.bundledScheme)
        let ucc = config.userContentController
        if let providerURL = Bundle.main.url(forResource: "provider", withExtension: "js"),
           let provider = try? String(contentsOf: providerURL, encoding: .utf8) {
            ucc.addUserScript(WKUserScript(source: provider, injectionTime: .atDocumentStart,
                                           forMainFrameOnly: true))
        }
        // The bundled explorer reads the app's own node: point its saved
        // endpoint at whatever port this Mac's node is on before it boots.
        if let port = model?.nodeRpcPort {
            ucc.addUserScript(WKUserScript(source: Self.explorerBootstrap(port: port),
                                           injectionTime: .atDocumentStart, forMainFrameOnly: true))
        }
        ucc.addScriptMessageHandler(WeakReplyBridge(controller: self, route: Self.providerRoute), contentWorld: .page, name: "aether")
        ucc.addScriptMessageHandler(WeakReplyBridge(controller: self, route: Self.verifyRoute), contentWorld: .page, name: "eastsea")
        if let host = wantExternal {
            config.websiteDataStore = .nonPersistent()
            externalHost = host
        } else {
            externalHost = nil
        }
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

    /// The address bar's Go: scheme-less text is treated as a host.
    func open(_ text: String) {
        let raw = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty else { return }
        let withScheme = raw.contains("://") ? raw : "https://\(raw)"
        guard let url = URL(string: withScheme) else {
            notice = String(localized: "That is not a web address.")
            return
        }
        load(url)
    }

    /// Load a URL through the same rules a link goes through.
    func load(_ url: URL) {
        notice = nil
        switch BrowserOriginPolicy.classify(url) {
        case .bundled, .external:
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
        webView?.goBack()
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
    func approveAsk(extraConfirmation: Bool = false) {
        guard let pending = ask, let model, !approvalBusy, !model.exploreLocked else { return }
        switch pending.kind {
        case .connect(let origin, _):
            if let context = pending.context, model.isCurrentDappContext(context) {
                model.grantSitePermission(origin: origin, address: model.address)
                finish(pending, .success([model.address]))
            } else {
                finish(pending, .failure(ProviderError(code: ProviderErrorCode.locked,
                                                      message: String(localized: "This approval is no longer valid. Ask the site to try again."))))
            }
        case .send(let origin, _, let tx, let feeWei):
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
                    guard model.isCurrentDappContext(reviewed.context, origin: origin) else {
                        finish(pending, .failure(ProviderError(code: ProviderErrorCode.locked,
                                                             message: String(localized: "This approval is no longer valid. Ask the site to try again."))))
                        return
                    }
                    guard latest == reviewed else {
                        simulation = latest
                        approvalNotice = String(localized: "The simulation changed. Review the new result before signing.")
                        return
                    }
                    let (hash, refusal) = await model.sendPageTransaction(latest.transaction, origin: origin, title: origin,
                                                                        shownFeeWei: feeWei, simulation: latest,
                                                                        extraConfirmation: extraConfirmation,
                                                                        stillApproved: { self.ask?.id == pending.id })
                    finish(pending, hash.map { .success($0) } ?? .failure(ProviderError(code: ProviderErrorCode.internalError,
                                                                                    message: refusal ?? String(localized: "The send failed."))))
                } catch {
                    guard ask?.id == pending.id else { return }
                    if let error = error as? ProviderError, error.code == 4901 {
                        finish(pending, .failure(error))
                        return
                    }
                    simulation = nil
                    approvalNotice = String(localized: "The node could not simulate this transaction.")
                }
            }
        case .typed(let origin, _, let prepared, _):
            guard let context = pending.context else { return }
            approvalBusy = true
            busyAskId = pending.id
            Task {
                defer { endApproval(pending) }
                do {
                    let signature = try await model.signPageTypedMessage(prepared, origin: origin, context: context,
                                                                         stillApproved: { self.ask?.id == pending.id })
                    finish(pending, .success(signature))
                } catch let error as ProviderError { finish(pending, .failure(error)) }
                catch { finish(pending, .failure(ProviderError(code: ProviderErrorCode.params, message: WalletModel.ffiMessage(error)))) }
            }
        }
    }

    func simulateAsk(_ pending: PendingAsk) async {
        guard let model, case .send(_, _, let tx, _) = pending.kind, ask?.id == pending.id,
              let context = pending.context else { return }
        simulation = nil
        approvalNotice = nil
        simulationLoading = true
        defer { if ask?.id == pending.id { simulationLoading = false } }
        do {
            let result = try await model.simulatePageTransaction(tx, context: context)
            guard ask?.id == pending.id else { return }
            guard model.isCurrentDappContext(context) else {
                finish(pending, .failure(ProviderError(code: ProviderErrorCode.locked,
                                                      message: String(localized: "This approval is no longer valid. Ask the site to try again."))))
                return
            }
            simulation = result
        } catch {
            guard ask?.id == pending.id else { return }
            if let error = error as? ProviderError, error.code == 4901 {
                finish(pending, .failure(error))
                return
            }
            approvalNotice = String(localized: "The node could not simulate this transaction.")
        }
    }

    private func finish(_ pending: PendingAsk, _ result: Result<Any?, ProviderError>) {
        guard ask?.id == pending.id else { return }
        endApproval(pending)
        ask = nil
        simulation = nil
        approvalNotice = nil
        simulationLoading = false
        pending.reply(result)
        drainAskQueue()
    }

    private func endApproval(_ pending: PendingAsk) {
        guard busyAskId == pending.id else { return }
        busyAskId = nil
        approvalBusy = false
    }

    /// The user refused the sheet (or dismissed it): 4001, nothing signed.
    func refuseAsk() {
        guard let pending = ask else { return }
        finish(pending, .failure(ProviderError(code: ProviderErrorCode.denied, message: "The user rejected the request.")))
    }

    func dismissAsk(_ pending: PendingAsk) {
        if ask?.id == pending.id { refuseAsk() }
        else {
            if ask == nil {
                endApproval(pending)
                simulation = nil
                approvalNotice = nil
                simulationLoading = false
            }
            pending.reply(.failure(ProviderError(code: ProviderErrorCode.denied, message: "The user rejected the request.")))
            drainAskQueue()
        }
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
        guard message.frameInfo.isMainFrame else {
            reply(.failure(ProviderError(code: ProviderErrorCode.locked,
                                         message: "This frame cannot talk to the EastSea wallet provider.")))
            return
        }
        guard let body = message.body as? [String: Any], let method = body["method"] as? String else {
            reply(.failure(ProviderError(code: ProviderErrorCode.notAString, message: "method must be a string")))
            return
        }
        let origin = message.frameInfo.securityOrigin
        let params = (body["params"] as? [Any]) ?? []
        let key = BrowserOriginPolicy.permissionKey(scheme: origin.protocol, host: origin.host, port: Int(origin.port))

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
            reply(.success(model.connectedSiteAddress(origin: key).map { [$0] } ?? []))
        case .requestAccounts:
            if let addr = model.connectedSiteAddress(origin: key) {
                reply(.success([addr]))
            } else {
                enqueue(.connect(origin: key, host: origin.host), id: body["id"], reply: reply)
            }
        case .disconnect:
            model.revokeSitePermission(origin: key)
            reply(.success(NSNull()))
        case .send:
            guard let connected = model.connectedSiteAddress(origin: key) else {
                reply(.failure(ProviderError(code: ProviderErrorCode.locked,
                                             message: "This site is not connected to an account.")))
                return
            }
            switch PageTransaction.parse(params.first, from: connected) {
            case .success(let tx):
                guard let context = model.dappContext else {
                    reply(.failure(ProviderError(code: ProviderErrorCode.locked, message: String(localized: "This approval is no longer valid. Ask the site to try again."))))
                    return
                }
                let gas = tx.gas == 0 ? (tx.isPlainTransfer ? UInt64(100_000) : UInt64(3_000_000)) : tx.gas
                let host = origin.host
                preparingAsks[key, default: 0] += 1
                Task {
                    defer { preparingAsks[key, default: 0] -= 1 }
                    let feeWei = await Task.detached { (try? dappTransactionQuote(to: tx.to, valueWei: tx.valueWei, dataHex: tx.data, gasLimit: gas))?.feeWei }.value
                    guard model.isCurrentDappContext(context, origin: key) else {
                        reply(.failure(ProviderError(code: ProviderErrorCode.locked, message: String(localized: "This approval is no longer valid. Ask the site to try again."))))
                        return
                    }
                    enqueue(.send(origin: key, host: host, tx: tx, feeWei: feeWei), id: nil, reply: reply, context: context)
                }
            case .failure(let e):
                reply(.failure(e))
            }
        case .typed:
            guard let context = model.dappContext, model.connectedSiteAddress(origin: key) != nil else {
                reply(.failure(ProviderError(code: ProviderErrorCode.locked, message: String(localized: "This approval is no longer valid. Ask the site to try again."))))
                return
            }
            let host = origin.host
            preparingAsks[key, default: 0] += 1
            Task {
                defer { preparingAsks[key, default: 0] -= 1 }
                do {
                    let json = try TypedMessageRequest.parse(params, context: context)
                    let prepared = try await model.preparePageTypedMessage(json)
                    let fields = try TypedMessageFields.parse(prepared.typedDataJson)
                    guard model.isCurrentDappContext(context, origin: key) else {
                        reply(.failure(ProviderError(code: ProviderErrorCode.locked, message: String(localized: "This approval is no longer valid. Ask the site to try again."))))
                        return
                    }
                    enqueue(.typed(origin: key, host: host, prepared: prepared, fields: fields), id: nil, reply: reply, context: context)
                } catch let error as ProviderError { reply(.failure(error)) }
                catch { reply(.failure(ProviderError(code: ProviderErrorCode.params, message: WalletModel.ffiMessage(error)))) }
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
        guard message.frameInfo.isMainFrame else {
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
        let key = BrowserOriginPolicy.permissionKey(scheme: origin.protocol, host: origin.host, port: Int(origin.port))
        guard VerifyBridge.allows(scheme: origin.protocol, connected: model.connectedSiteAddress(origin: key) != nil) else {
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

    private func enqueue(_ kind: PendingAsk.Kind, id: Any?, reply: @escaping (Result<Any?, ProviderError>) -> Void,
                         context: DappRequestContext? = nil) {
        let pending = PendingAsk(id: UUID().uuidString,
                                 kind: kind, reply: reply, context: context ?? model?.dappContext)
        if ask == nil { ask = pending } else { askQueue.append(pending) }
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
        if url.scheme?.lowercased() == BrowserOriginPolicy.bundledScheme {
            return url.host == "explorer" ? String(localized: "Block explorer") : (url.host ?? "")
        }
        return url.absoluteString
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
