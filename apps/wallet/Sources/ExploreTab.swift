import SwiftUI
import WebKit

/// The Explore tab (docs/design/09-wallet.md "인앱 브라우저"): chain search,
/// an address bar that opens external https after one warning per site, and
/// the block explorer bundled with the app. Pages' `window.aether` is
/// answered by the wallet itself through BrowserController.
struct ExplorePage: View {
    @EnvironmentObject var model: WalletModel
    @EnvironmentObject var browser: BrowserController
    @FocusState private var addressFocused: Bool
    /// Back to Home, always in the address bar: a page in the full-bleed
    /// web view must never be a dead end (founder report on 0.7.0).
    var goHome: (() -> Void)?

    var body: some View {
        VStack(spacing: 0) {
            addressBar
            if addressFocused && (browser.suggestionsBusy || !browser.searchSuggestions.isEmpty || browser.suggestionsFailure != nil
                || browser.suggestionsInfo?.incomplete == true || browser.suggestionsInfo?.usageComplete == false
                || browser.suggestionsInfo?.sourcesConfigured == false) {
                suggestions
            }
            if let n = browser.notice {
                Text(n).font(.aeFootnote).foregroundStyle(Color.warn)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16).padding(.vertical, 6)
                    .background(Color.warn.opacity(0.12))
            }
            Divider()
            if browser.searchQuery != nil {
                AppSearchPage(browser: browser)
            } else if browser.webView == nil {
                home.padding(20)
            } else if let web = browser.webView {
                WebViewHolder(webView: web)
                    .id(browser.webViewGeneration)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onChange(of: browser.addressField) { _, text in
            if addressFocused { browser.suggest(text) }
        }
        .onChange(of: addressFocused) { _, focused in
            if focused { browser.suggest(browser.addressField) }
            else { browser.dismissSuggestions() }
        }
        .onChange(of: model.nodeRpcPort) { _, _ in
            browser.refreshSearchForNode()
            if addressFocused { browser.suggest(browser.addressField) }
        }
        .onDisappear { browser.dismissSuggestions() }
        .sheet(item: $browser.warning) { w in
            SiteWarningSheet(warning: w, browser: browser)
                .frame(width: 460)
        }
        .sheet(item: $browser.ask) { ask in
            ProviderAskSheet(ask: ask, browser: browser)
                .frame(width: 480)
        }
        .sheet(item: $browser.searchRecord) { record in
            AppSearchRecordSheet(record: record, info: browser.searchRecordInfo) { browser.searchRecord = nil }
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
            if browser.searchQuery != nil || browser.canGoBack {
                Button { browser.goBack() } label: { Image(systemName: "chevron.left") }
                    .buttonStyle(.borderless).help("Back")
            }
            #endif
            Button {
                addressFocused = false
                if case .search(let query) = AppSearchInput.destination(for: browser.addressField) { browser.search(query) }
                else { browser.search() }
            } label: {
                Label("Search", systemImage: "magnifyingglass").labelStyle(.iconOnly)
            }
            .buttonStyle(.bordered).help("Search")
            TextField("Search apps and names, or enter an https address", text: $browser.addressField)
                .textFieldStyle(.roundedBorder).font(.aeBody)
                .focused($addressFocused)
                .onSubmit { submitAddress() }
            Button("Go") { submitAddress() }
                .buttonStyle(.borderedProminent).disabled(browser.addressField.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .padding(.horizontal, 16).padding(.vertical, 10)
    }

    private func submitAddress() {
        addressFocused = false
        browser.open(browser.addressField)
    }

    private var suggestions: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                if browser.suggestionsBusy {
                    ProgressView(String(localized: "Searching the chain…")).controlSize(.small)
                }
                if let failure = browser.suggestionsFailure {
                    Text(failure).font(.aeFootnote).foregroundStyle(Color.warn)
                }
                if !browser.suggestionsBusy && (!browser.searchSuggestions.isEmpty || browser.suggestionsInfo?.incomplete == true
                    || browser.suggestionsInfo?.usageComplete == false || browser.suggestionsInfo?.sourcesConfigured == false) {
                    AppSearchIndexNotice(info: browser.suggestionsInfo)
                }
                ForEach(Array(browser.searchSuggestions.enumerated()), id: \.offset) { _, record in
                    Button {
                        browser.showSearchRecord(record)
                        addressFocused = false
                    } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(verbatim: record.name).font(.aeHeadline)
                            if !record.title.isEmpty && record.title != record.name {
                                Text(verbatim: record.title).font(.aeFootnote).foregroundStyle(.secondary)
                            }
                            AppSearchHashLabel(present: record.verified)
                            AppSearchLookalikeWarning(name: record.lookalike)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    Divider()
                }
            }
            .padding(.horizontal, 16).padding(.vertical, 8)
        }
        .frame(maxHeight: 240)
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
                        addressFocused = false
                        browser.search()
                    } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            Label("Search", systemImage: "magnifyingglass").font(.aeHeadline)
                            Text("Find apps and .sea names using your own node.")
                                .font(.aeBody).foregroundStyle(.secondary)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(.plain)
                    Divider()
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

/// Results retain the node's ordering. Registered text is displayed verbatim;
/// it never becomes a WebKit document or a wallet transaction.
private struct AppSearchPage: View {
    @ObservedObject var browser: BrowserController

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("Search").font(.aeHeadline)
                    Spacer()
                    Button("Back to Explore") { browser.dismissSearch() }
                        .buttonStyle(.bordered)
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
                        Button("Try Again") { browser.search(query) }.buttonStyle(.bordered)
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

private struct AppSearchRecordSheet: View {
    let record: AppSearchResult
    let info: AppSearchInfo?
    let close: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("Search record").font(.aeHeadline)
                    Spacer()
                    Button("Close", action: close).buttonStyle(.bordered)
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

private struct AppSearchIndexNotice: View {
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

private struct AppSearchHashLabel: View {
    let present: Bool

    var body: some View {
        Label(present ? String(localized: "Content hash present") : String(localized: "Content hash not recorded"),
              systemImage: "number.circle")
            .font(.aeCaption).foregroundStyle(.secondary)
    }
}

private struct AppSearchLookalikeWarning: View {
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

    var body: some View {
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
                    row("Action", CallDescribe.action(to: tx.to, data: tx.data), mono: false)
                    if !tx.to.isEmpty { row("To", tx.to, mono: true) }
                    row("Amount", tx.valueWei == "0" ? "—" : "\(Wei.format(tx.valueWei)) \(Brand.networkCoinTicker)")
                    if tx.isPlainTransfer {
                        row("Fee (maximum)", feeWei.map { "\(Wei.format($0)) \(Brand.networkCoinTicker)" } ?? String(localized: "the network's fee at send time"))
                    }
                    row("Gas", tx.gas == 0 ? String(localized: "the wallet's default") : "\(tx.gas)")
                    if tx.data != "0x" {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("Calldata").font(.aeFootnote).foregroundStyle(.secondary)
                            Text(tx.data).font(.aeFootnote.monospaced()).lineLimit(4)
                                .truncationMode(.middle).textSelection(.enabled)
                        }
                    }
                }
                .padding(12)
                .background(.background.tertiary, in: RoundedRectangle(cornerRadius: Radius.inner))
                Label("Only continue if you started this on \(host.isEmpty ? origin : host). A refused request sends nothing.", systemImage: "exclamationmark.shield")
                    .font(.aeFootnote).foregroundStyle(Color.warn)
            }
            HStack {
                Spacer()
                Button("Refuse", role: .cancel) { browser.refuseAsk() }.keyboardShortcut(.cancelAction)
                Button(ask.kind.isConnect ? String(localized: "Connect") : String(localized: "Send")) { browser.approveAsk() }
                    .keyboardShortcut(.defaultAction).buttonStyle(.borderedProminent)
            }
        }
        .padding(20)
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
