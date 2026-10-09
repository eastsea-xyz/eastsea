import SwiftUI
import WebKit

/// Explore owns native browser chrome. Each session tab answers the page's
/// `window.aether` bridge through its own origin-checked BrowserController.
struct ExplorePage: View {
    @EnvironmentObject var session: BrowserSession
    /// Back to Home, always in the address bar: a page in the full-bleed
    /// web view must never be a dead end (founder report on 0.7.0).
    var goHome: (() -> Void)?

    var body: some View {
        BrowserWorkspace(session: session, browser: session.controller, goHome: goHome)
            .id(session.activeTabID)
    }
}

/// Results retain the node's ordering. Registered text is displayed verbatim;
/// it never becomes a WebKit document or a wallet transaction.
struct AppSearchPage: View {
    @ObservedObject var browser: BrowserController

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("Search").font(.aeHeadline)
                    Spacer()
                    Button("Back to Explore") { browser.dismissSearch() }
                        .buttonStyle(EastSeaQuietButtonStyle())
                }
                Text("Apps and names recorded on chain").font(.aeBody).foregroundStyle(.secondary)
                DisclosureGroup(String(localized: "How results are ordered")) {
                    Text("Search reads your own node. Exact names come first, then prefixes and title or description tokens, then distinct contract callers in the last 7 days, then older records. There are no paid placements, publisher boosts or hidden blocklists.")
                        .font(.aeFootnote).foregroundStyle(.secondary).padding(.top, 6)
                }
                if let query = browser.searchQuery, !query.isEmpty {
                    Text("Results for \(query)").font(.aeBody).textSelection(.enabled)
                    if !browser.searchBusy && browser.searchFailure == nil {
                        AppSearchIndexNotice(info: browser.searchInfo)
                    }
                    if browser.searchBusy {
                        ProgressView(String(localized: "Searching the chain…"))
                    } else if let failure = browser.searchFailure {
                        Text(failure).font(.aeBody).foregroundStyle(Color.warn)
                        Button("Try Again") { browser.search(query) }.buttonStyle(EastSeaPrimaryButtonStyle())
                    } else if browser.searchResults.isEmpty {
                        Text("No matching apps or names.").font(.aeBody).foregroundStyle(.secondary)
                    } else {
                        ForEach(Array(browser.searchResults.enumerated()), id: \.offset) { _, record in
                            Card { AppSearchRecordDetails(record: record, info: browser.searchInfo) }
                        }
                    }
                } else {
                    Text("Search by app name, .sea name or description.").font(.aeBody)
                }
                Text("The content hash label only reports that a hash is recorded on chain. It does not establish the publisher's identity or the app's safety.")
                    .font(.aeFootnote).foregroundStyle(.secondary)
            }
            .padding(20).frame(maxWidth: 680, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

struct AppSearchRecordSheet: View {
    let record: AppSearchResult
    let info: AppSearchInfo?
    let close: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("Search record").font(.aeHeadline)
                    Spacer()
                    Button("Close", action: close).buttonStyle(EastSeaQuietButtonStyle())
                }
                AppSearchIndexNotice(info: info)
                AppSearchRecordDetails(record: record, info: info)
                Text("The content hash label only reports that a hash is recorded on chain. It does not establish the publisher's identity or the app's safety.")
                    .font(.aeFootnote).foregroundStyle(.secondary)
            }
            .padding(20)
        }
        .frame(idealWidth: 560, idealHeight: 540)
    }
}

private struct AppSearchRecordDetails: View {
    let record: AppSearchResult
    let info: AppSearchInfo?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(verbatim: record.name).font(.aeHeadline).textSelection(.enabled)
            if !record.title.isEmpty && record.title != record.name {
                Text(verbatim: record.title).font(.aeBody).textSelection(.enabled)
            }
            AppSearchLookalikeWarning(name: record.lookalike)
            AppSearchHashLabel(present: record.verified)
            if !record.description.isEmpty {
                Text(verbatim: record.description).font(.aeBody).textSelection(.enabled)
            }
            AppSearchField(label: String(localized: "App address"), value: record.url)
            AppSearchField(label: String(localized: "Category"), value: record.category)
            AppSearchField(label: String(localized: "Publisher"), value: record.publisher)
            AppSearchField(label: String(localized: "Distinct callers in the last 7 days"),
                           value: record.usageAvailable(info: info) ? record.usage7d.formatted() : String(localized: "Activity signal unavailable"))
            VStack(alignment: .leading, spacing: 3) {
                Text("Registered").font(.aeCaption).foregroundStyle(.secondary)
                Text(Date(timeIntervalSince1970: TimeInterval(record.createdAt)), format: .dateTime.year().month().day())
                    .font(.aeFootnote).textSelection(.enabled)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct AppSearchIndexNotice: View {
    let info: AppSearchInfo?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let info {
                if info.sourcesConfigured == false {
                    Label("This node has no app or name registry sources configured.", systemImage: "exclamationmark.triangle")
                }
                if info.incomplete {
                    Label("This node's search index is incomplete. Some chain records may be missing from these results.", systemImage: "exclamationmark.triangle")
                }
                if !info.usageComplete {
                    Label("Activity signal unavailable", systemImage: "exclamationmark.triangle")
                }
            } else {
                Label("Search index status unavailable. Results may be incomplete.", systemImage: "exclamationmark.triangle")
            }
        }
        .font(.aeFootnote).foregroundStyle(Color.warn)
    }
}

private struct AppSearchField: View {
    let label: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label).font(.aeCaption).foregroundStyle(.secondary)
            Text(verbatim: value).font(.aeFootnote).textSelection(.enabled)
        }
    }
}

struct AppSearchHashLabel: View {
    let present: Bool

    var body: some View {
        Label(present ? String(localized: "Content hash present") : String(localized: "Content hash not recorded"),
              systemImage: "number.circle")
            .font(.aeCaption).foregroundStyle(.secondary)
    }
}

struct AppSearchLookalikeWarning: View {
    let name: String?

    var body: some View {
        if let name {
            VStack(alignment: .leading, spacing: 4) {
                Label("Lookalike warning", systemImage: "exclamationmark.triangle.fill").font(.aeFootnote)
                Text("This name resembles \(name). It may be a phishing attempt. Check the name and publisher.")
                    .font(.aeFootnote).textSelection(.enabled)
            }
            .foregroundStyle(Color.warn).frame(maxWidth: .infinity, alignment: .leading)
            .padding(8).background(Color.warn.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
        }
    }
}

/// Puts a WKWebView in the SwiftUI tree on both platforms.
struct WebViewHolder: View {
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
        VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
            HStack(alignment: .top, spacing: DesignTokens.Space.s3) {
                Image(systemName: "exclamationmark.shield").font(.aeTitle).foregroundStyle(Color.warn)
                Text("Open this site in Explore?").font(.aeTitle)
            }
            Text("You are about to open **\(warning.host)**. Explore shows pages like any browser: the site's content is the site's, not \(Brand.name)'s.")
                .font(.aeBody)
                .fixedSize(horizontal: false, vertical: true)
                .padding(DesignTokens.Space.s4)
                .background(DesignTokens.Palette.surfaceSunken.color, in: RoundedRectangle(cornerRadius: DesignTokens.Radius.md))
            if let like = warning.lookalike {
                // Label's title does not render Markdown; Text does (the bold host).
                Label {
                    Text("This address looks like **\(like)** but is not it. Check every letter before you connect a wallet.")
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill")
                }
                .font(.aeBody).foregroundStyle(Color.warn).fixedSize(horizontal: false, vertical: true)
            }
            if warning.punycode {
                Label("This address mixes in characters that can hide inside look-alike letters.", systemImage: "character.cursor.ibeam")
                    .font(.aeBody).foregroundStyle(Color.warn).fixedSize(horizontal: false, vertical: true)
            }
            Text("Pages cannot see your addresses until you approve them, and every payment asks again. This warning appears once per site.")
                .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { browser.refuseWarning(id: warning.id) }
                    .buttonStyle(EastSeaQuietButtonStyle()).keyboardShortcut(.cancelAction)
                Button("Open Site") { browser.approveWarning(id: warning.id) }
                    .buttonStyle(EastSeaPrimaryButtonStyle()).keyboardShortcut(.defaultAction)
            }
        }
        .padding(DesignTokens.Space.s6)
        .eastSeaSheet()
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
            VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
                switch ask.kind {
                case .connect(let origin, let host):
                    Text("Connect to \(host.isEmpty ? origin : host)?").font(.aeTitle)
                    originLabel(origin)
                    Text("This site is asking which address this wallet controls. Saying yes shows it **\(Short.address(ask.accountAddress.isEmpty ? model.address : ask.accountAddress))** — the address itself, not your key, and not your balances.")
                        .font(.aeBody).fixedSize(horizontal: false, vertical: true)
                    Text("You can take this back any time in Security → Connected sites.")
                        .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                case .send(let origin, let host, let tx, let feeWei):
                    Text("\(host.isEmpty ? origin : host) asks to send").font(.aeTitle)
                    originLabel(origin)
                    Grid(alignment: .topLeading, horizontalSpacing: DesignTokens.Space.s4, verticalSpacing: DesignTokens.Space.s3) {
                        row("Network", "\(chainId)")
                        row("Action", CallDescribe.action(to: tx.to, data: tx.data, ticker: ticker), mono: false)
                        if !tx.to.isEmpty { row(tx.isPlainTransfer ? "To" : "Contract called", tx.to, mono: true) }
                        row("Amount", tx.valueWei == "0" ? "—" : "\(Wei.format(tx.valueWei)) \(ticker)")
                        if feeWei != nil {
                            row("Fee (maximum)", feeWei.map { "\(Wei.format($0)) \(ticker)" } ?? String(localized: "the network's fee at send time"))
                        }
                        row("Gas", simulation.map { "\($0.transaction.gas)" } ?? (tx.gas == 0 ? String(localized: "the wallet's default") : "\(tx.gas)"))
                        if tx.data != "0x" {
                            GridRow(alignment: .top) {
                                Text("Calldata").font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                                    .frame(width: 96, alignment: .leading)
                                Text(tx.data).font(.aeFootnote.monospaced()).lineLimit(4)
                                    .truncationMode(.middle).textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                    }
                    .padding(DesignTokens.Space.s4)
                    .background(DesignTokens.Palette.surfaceSunken.color, in: RoundedRectangle(cornerRadius: Radius.inner))
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
                        .font(.aeFootnote).foregroundStyle(Color.warn).fixedSize(horizontal: false, vertical: true)
                case .typed(let origin, let host, let prepared, let fields):
                    Text("\(host.isEmpty ? origin : host) asks to sign a message").font(.aeTitle)
                    originLabel(origin)
                    Grid(alignment: .topLeading, horizontalSpacing: DesignTokens.Space.s4, verticalSpacing: DesignTokens.Space.s3) {
                        row("Account", prepared.account, mono: true)
                        row("Network", "\(prepared.chainId)")
                    }
                    TypedMessageFieldsView(fields: fields)
                    Label("A message signature can authorize spending or sign you in without sending a transaction. Only sign a request you understand.", systemImage: "exclamationmark.shield")
                        .font(.aeFootnote).foregroundStyle(Color.warn).fixedSize(horizontal: false, vertical: true)
                }
                if let notice = browser.approvalNotice {
                    Text(notice).font(.aeFootnote).foregroundStyle(Color.warn).fixedSize(horizontal: false, vertical: true)
                }
                HStack {
                    Spacer()
                    Button("Refuse", role: .cancel) { browser.refuseAsk(id: ask.id) }
                        .buttonStyle(EastSeaQuietButtonStyle()).keyboardShortcut(.cancelAction)
                        .disabled(browser.submissionInFlight)
                    Button(approveLabel) { browser.approveAsk(id: ask.id, extraConfirmation: confirmRevert) }
                        .keyboardShortcut(.defaultAction).buttonStyle(EastSeaPrimaryButtonStyle()).disabled(!canApprove)
                }
            }
            .padding(DesignTokens.Space.s6)
        }
        .frame(maxHeight: 680)
        .eastSeaSheet()
        .interactiveDismissDisabled(browser.submissionInFlight)
        .task(id: ask.id) { if previewSimulation == nil { await browser.simulateAsk(ask) } }
        .onChange(of: browser.simulation) { _, _ in confirmRevert = false }
        .onDisappear { browser.dismissAsk(ask) }
    }

    private func originLabel(_ origin: String) -> some View {
        Text(verbatim: browser.appIdentity?.displayOrigin ?? origin).font(.aeBody.monospaced()).textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
    }

    /// Addresses in a fixed-width face (easier to compare), words in the body face.
    private func row(_ label: LocalizedStringKey, _ value: String, mono: Bool = false) -> some View {
        GridRow(alignment: .top) {
            Text(label).font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color).frame(width: 96, alignment: .leading)
            Text(value).font(mono ? .aeBody.monospaced() : .aeBody.monospacedDigit()).textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// Security's list of connected sites, each revocable (docs: per-origin
/// permissions stored and revocable in Settings).
struct ConnectedSitesSection: View {
    @EnvironmentObject var model: WalletModel

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
                HStack(alignment: .top, spacing: DesignTokens.Space.s3) {
                    Image(systemName: "safari").font(.aeTitle).foregroundStyle(Color.aether)
                    VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                        Text("Connected sites").font(.aeHeadline)
                        Text(model.sitePermissions.sites.isEmpty
                             ? String(localized: "No site can see your address. When the Explore tab connects one, it appears here.")
                             : String(localized: "These sites may ask about your address. Disconnecting takes effect the next time they ask."))
                            .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                ForEach(model.sitePermissions.sites) { site in
                    VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
                        HStack {
                            VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                                Text(site.displayOrigin ?? site.origin).font(.aeBody.monospaced())
                                    .fixedSize(horizontal: false, vertical: true)
                                Text("may see \(Short.address(site.address)) · connected \(site.grantedAt, style: .date)")
                                    .font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
                            }
                            Spacer()
                            if site.address.lowercased() == model.address.lowercased() {
                                Button("Disconnect") { model.revokeSitePermission(origin: site.origin) }
                                    .buttonStyle(EastSeaQuietButtonStyle())
                            } else {
                                // The grant names an address this wallet no
                                // longer holds: it stopped meaning anything.
                                Text("stale — was \(Short.address(site.address))").font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                            }
                        }
                        Rectangle().fill(DesignTokens.Palette.line.color).frame(height: 1)
                    }
                }
                if model.sitePermissions.sites.contains(where: { $0.address.lowercased() == model.address.lowercased() }) {
                    Button("Disconnect all") { model.revokeAllSitePermissions() }.buttonStyle(EastSeaQuietButtonStyle())
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
