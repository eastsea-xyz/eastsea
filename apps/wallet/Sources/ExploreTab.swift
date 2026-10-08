import SwiftUI
import WebKit

/// The Explore tab (docs/design/09-wallet.md "인앱 브라우저"): a curated home,
/// an address bar that opens external https after one warning per site, and
/// the block explorer bundled with the app. Pages' `window.aether` is
/// answered by the wallet itself through BrowserController.
struct ExplorePage: View {
    @EnvironmentObject var model: WalletModel
    @EnvironmentObject var browser: BrowserController
    /// Back to Home, always in the address bar: a page in the full-bleed
    /// web view must never be a dead end (founder report on 0.7.0).
    var goHome: (() -> Void)?

    var body: some View {
        VStack(spacing: 0) {
            addressBar
            if let n = browser.notice {
                Text(n).font(.aeFootnote).foregroundStyle(Color.warn)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16).padding(.vertical, 6)
                    .background(Color.warn.opacity(0.12))
            }
            Divider()
            if browser.webView == nil {
                home.padding(20)
            } else if let web = browser.webView {
                WebViewHolder(webView: web)
                    .id(browser.webViewGeneration)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .sheet(item: $browser.warning) { w in
            SiteWarningSheet(warning: w, browser: browser)
                .frame(width: 460)
        }
        .sheet(item: $browser.ask) { ask in
            ProviderAskSheet(ask: ask, browser: browser)
                .frame(width: 480)
                .onDisappear { browser.dismissAsk(ask) }
        }
    }

    private var addressBar: some View {
        HStack(spacing: 8) {
            if let goHome {
                Button(action: goHome) { Label("Home", systemImage: "house.fill") }
                    .buttonStyle(.bordered)
                    .help("Back to Home")
            }
            #if os(macOS)
            if browser.canGoBack {
                Button { browser.goBack() } label: { Image(systemName: "chevron.left") }
                    .buttonStyle(.borderless).help("Back")
            }
            #endif
            Image(systemName: "lock.fill").font(.aeCaption).foregroundStyle(.secondary)
            TextField("Enter a web address (https)", text: $browser.addressField)
                .textFieldStyle(.roundedBorder).font(.aeBody)
                .onSubmit { browser.open(browser.addressField) }
            Button("Go") { browser.open(browser.addressField) }
                .buttonStyle(.borderedProminent).disabled(browser.addressField.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .padding(.horizontal, 16).padding(.vertical, 10)
    }

    /// The curated home: what the tab is for, before any address is typed.
    private var home: some View {
        VStack(spacing: 16) {
            Card {
                VStack(alignment: .leading, spacing: 14) {
                    HStack(alignment: .top, spacing: 14) {
                        Image(systemName: "safari.fill").font(.system(size: 30)).foregroundStyle(Color.aether)
                        VStack(alignment: .leading, spacing: 4) {
                            Text("Explore the chain").font(.aeHeadline)
                            Text("The block explorer below is part of the app and reads this Mac's own node. Pages you open can connect to your wallet — every request asks first, and Security lists the sites you allowed.")
                                .font(.aeBody).foregroundStyle(.secondary)
                        }
                    }
                }
            }
            Card {
                VStack(alignment: .leading, spacing: 12) {
                    Button {
                        browser.openExplorer()
                    } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("Block explorer").font(.aeHeadline)
                            Text("Bundled with the app — reads your own node, signs nothing.")
                                .font(.aeBody).foregroundStyle(.secondary)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(.plain)
                    Divider()
                    ForEach(Array(BrowserOriginPolicy.curatedDomains).sorted(), id: \.self) { domain in
                        Button {
                            browser.load(URL(string: "https://\(domain)")!)
                        } label: {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(domain).font(.aeHeadline)
                                Text("The \(Brand.name) website.").font(.aeBody).foregroundStyle(.secondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            Spacer()
        }
        .frame(maxWidth: 620)
    }
}

/// Puts a WKWebView in the SwiftUI tree on both platforms.
private struct WebViewHolder: View {
    let webView: WKWebView

    var body: some View {
        #if os(macOS)
        WebViewRepresentable(webView: webView)
        #else
        // Inside the safe area, so the tab bar stays visible over the page.
        WebViewRepresentable(webView: webView)
        #endif
    }
}

#if os(macOS)
private struct WebViewRepresentable: NSViewRepresentable {
    let webView: WKWebView
    func makeNSView(context: Context) -> WKWebView { webView }
    func updateNSView(_ nsView: WKWebView, context: Context) {}
}
#else
private struct WebViewRepresentable: UIViewRepresentable {
    let webView: WKWebView
    func makeUIView(context: Context) -> WKWebView { webView }
    func updateUIView(_ uiView: WKWebView, context: Context) {}
}
#endif

/// The one-time warning before an unknown https site first loads.
struct SiteWarningSheet: View {
    let warning: BrowserController.SiteWarning
    @ObservedObject var browser: BrowserController
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Open this site in Explore?").font(.aeTitle)
            Text("You are about to open **\(warning.host)**. Explore shows pages like any browser: the site's content is the site's, not \(Brand.name)'s.")
                .font(.aeBody)
            if let like = warning.lookalike {
                // Label's title does not render Markdown; Text does (the bold host).
                Label {
                    Text("This address looks like **\(like)** but is not it. Check every letter before you connect a wallet.")
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill")
                }
                .font(.aeBody).foregroundStyle(Color.warn)
            }
            if warning.punycode {
                Label("This address mixes in characters that can hide inside look-alike letters.", systemImage: "character.cursor.ibeam")
                    .font(.aeBody).foregroundStyle(Color.warn)
            }
            Text("Pages cannot see your addresses until you approve them, and every payment asks again. This warning appears once per site.")
                .font(.aeFootnote).foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { browser.refuseWarning() }.keyboardShortcut(.cancelAction)
                Button("Open Site") { browser.approveWarning() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
    }
}

/// The confirmation sheet for a page's provider request: a connect, or a
/// transaction. Nothing is answered until the user decides, and a locked
/// wallet never shows this at all (the bridge refused it outright).
struct ProviderAskSheet: View {
    let ask: BrowserController.PendingAsk
    @EnvironmentObject var model: WalletModel
    @ObservedObject var browser: BrowserController

    @State private var confirmRevert = false
    /// Screens harness supplies a result without contacting a node.
    var previewSimulation: SimulatedPageTransaction? = nil

    private var simulation: SimulatedPageTransaction? { previewSimulation ?? browser.simulation }
    private var canApprove: Bool {
        guard !browser.approvalBusy, !model.busy else { return false }
        if case .send = ask.kind { return simulation?.result.canSign(extraConfirmation: confirmRevert) == true }
        return true
    }
    private var chainId: UInt64 { simulation?.context.chainId ?? ask.context?.chainId ?? model.status?.chainId ?? Brand.networkChainId }
    private var ticker: String { Brand.coinTicker(chainId: chainId) }
    private var approveLabel: String {
        switch ask.kind {
        case .connect: return String(localized: "Connect")
        case .send: return simulation?.result.success == false ? String(localized: "Sign failing transaction") : String(localized: "Send")
        case .typed: return String(localized: "Sign message")
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                switch ask.kind {
                case .connect(let origin, let host):
                    Text("Connect to \(host.isEmpty ? origin : host)?").font(.aeTitle)
                    Text("This site is asking which address this wallet controls. Saying yes shows it **\(Short.address(model.address))** — the address itself, not your key, and not your balances.")
                        .font(.aeBody)
                    Text("You can take this back any time in Security → Connected sites.")
                        .font(.aeFootnote).foregroundStyle(.secondary)
                case .send(let origin, let host, let tx, let feeWei):
                    Text("\(host.isEmpty ? origin : host) asks to send").font(.aeTitle)
                    VStack(alignment: .leading, spacing: 8) {
                        row("Network", "\(chainId)")
                        row("Action", CallDescribe.action(to: tx.to, data: tx.data, ticker: ticker), mono: false)
                        if !tx.to.isEmpty { row(tx.isPlainTransfer ? "To" : "Contract called", tx.to, mono: true) }
                        row("Amount", tx.valueWei == "0" ? "—" : "\(Wei.format(tx.valueWei)) \(ticker)")
                        if feeWei != nil {
                            row("Fee (maximum)", feeWei.map { "\(Wei.format($0)) \(ticker)" } ?? String(localized: "the network's fee at send time"))
                        }
                        row("Gas", simulation.map { "\($0.transaction.gas)" } ?? (tx.gas == 0 ? String(localized: "the wallet's default") : "\(tx.gas)"))
                        if tx.data != "0x" {
                            DisclosureGroup("Calldata") {
                                Text(tx.data).font(.aeFootnote.monospaced()).lineLimit(4)
                                    .truncationMode(.middle).textSelection(.enabled)
                            }
                        }
                    }
                    .padding(12)
                    .background(.background.tertiary, in: RoundedRectangle(cornerRadius: Radius.inner))
                    if let simulation {
                        DappSimulationView(simulation: simulation.result, chainId: simulation.context.chainId)
                        if !simulation.result.success {
                            Toggle("I understand this transaction is expected to fail and may still cost fees.", isOn: $confirmRevert)
                                .font(.aeFootnote).foregroundStyle(Color.warn)
                        }
                    } else if browser.simulationLoading {
                        ProgressView("Simulating before signing…").font(.aeFootnote)
                    } else {
                        Button("Retry simulation") { Task { await browser.simulateAsk(ask) } }
                    }
                    Label("Only continue if you started this on \(host.isEmpty ? origin : host). A refused request sends nothing.", systemImage: "exclamationmark.shield")
                        .font(.aeFootnote).foregroundStyle(Color.warn)
                case .typed(let origin, let host, let prepared, let fields):
                    Text("\(host.isEmpty ? origin : host) asks to sign a message").font(.aeTitle)
                    row("Account", prepared.account, mono: true)
                    row("Network", "\(prepared.chainId)")
                    TypedMessageFieldsView(fields: fields)
                    Label("A message signature can authorize spending or sign you in without sending a transaction. Only sign a request you understand.", systemImage: "exclamationmark.shield")
                        .font(.aeFootnote).foregroundStyle(Color.warn).fixedSize(horizontal: false, vertical: true)
                }
                if let notice = browser.approvalNotice { Text(notice).font(.aeFootnote).foregroundStyle(Color.warn).fixedSize(horizontal: false, vertical: true) }
                HStack {
                        Spacer()
                    Button("Refuse", role: .cancel) { browser.refuseAsk() }.keyboardShortcut(.cancelAction)
                    Button(approveLabel) { browser.approveAsk(extraConfirmation: confirmRevert) }
                        .keyboardShortcut(.defaultAction).buttonStyle(.borderedProminent).disabled(!canApprove)
                }
            }
            .padding(20)
        }
        .frame(maxHeight: 680)
        .task(id: ask.id) { if previewSimulation == nil { await browser.simulateAsk(ask) } }
        .onChange(of: browser.simulation) { _, _ in confirmRevert = false }
    }

    /// Addresses in a fixed-width face (easier to compare), words in the body face.
    private func row(_ label: LocalizedStringKey, _ value: String, mono: Bool = false) -> some View {
        HStack(alignment: .top) {
            Text(label).font(.aeFootnote).foregroundStyle(.secondary).frame(width: 110, alignment: .leading)
            Text(value).font(mono ? .aeBody.monospaced() : .aeBody.monospacedDigit()).textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
    }
}

/// Security's list of connected sites, each revocable (docs: per-origin
/// permissions stored and revocable in Settings).
struct ConnectedSitesSection: View {
    @EnvironmentObject var model: WalletModel

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .top, spacing: 14) {
                    Image(systemName: "safari").font(.system(size: 30)).foregroundStyle(Color.aether)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Connected sites").font(.aeHeadline)
                        Text(model.sitePermissions.sites.isEmpty
                             ? String(localized: "No site can see your address. When the Explore tab connects one, it appears here.")
                             : String(localized: "These sites may ask about your address. Disconnecting takes effect the next time they ask."))
                            .font(.aeBody).foregroundStyle(.secondary)
                    }
                }
                ForEach(model.sitePermissions.sites) { site in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(site.origin).font(.aeBody.monospaced())
                                Text("may see \(Short.address(site.address)) · connected \(site.grantedAt, style: .date)")
                                    .font(.aeFootnote).foregroundStyle(.secondary)
                            }
                            Spacer()
                            if site.address.lowercased() == model.address.lowercased() {
                                Button("Disconnect") { model.revokeSitePermission(origin: site.origin) }
                            } else {
                                // The grant names an address this wallet no
                                // longer holds: it stopped meaning anything.
                                Text("stale — was \(Short.address(site.address))").font(.aeFootnote).foregroundStyle(.secondary)
                            }
                        }
                        Divider()
                    }
                }
                if model.sitePermissions.sites.contains(where: { $0.address.lowercased() == model.address.lowercased() }) {
                    Button("Disconnect all") { model.revokeAllSitePermissions() }
                }
            }
        }
    }
}

extension BrowserController.PendingAsk.Kind {
    /// Whether this ask is the connect kind (the button reads differently).
    var isConnect: Bool {
        if case .connect = self { return true }
        return false
    }
}
