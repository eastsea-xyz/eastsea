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
                    .padding(.horizontal, DesignTokens.Space.s4).padding(.vertical, DesignTokens.Space.s2)
                    .background(DesignTokens.Palette.surfaceSunken.color)
            }
            Rectangle().fill(DesignTokens.Palette.line.color).frame(height: 1)
            if browser.webView == nil {
                home.padding(DesignTokens.Space.s5)
            } else if let web = browser.webView {
                WebViewHolder(webView: web)
                    .id(browser.webViewGeneration)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .eastSeaPage()
        .sheet(item: $browser.warning) { w in
            SiteWarningSheet(warning: w, browser: browser)
                .frame(width: 460)
        }
        .sheet(item: $browser.ask) { ask in
            ProviderAskSheet(ask: ask, browser: browser)
                .frame(width: 480)
        }
    }

    private var addressBar: some View {
        HStack(spacing: DesignTokens.Space.s2) {
            if let goHome {
                Button(action: goHome) { Label("Home", systemImage: "house.fill") }
                    .buttonStyle(EastSeaQuietButtonStyle()).fixedSize()
                    .help("Back to Home")
            }
            #if os(macOS)
            if browser.canGoBack {
                Button { browser.goBack() } label: { Image(systemName: "chevron.left") }
                    .buttonStyle(.borderless).help("Back")
            }
            #endif
            Image(systemName: "lock.fill").font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
            TextField("Enter a web address (https)", text: $browser.addressField)
                .textFieldStyle(EastSeaTextFieldStyle()).font(.aeBody)
                .onSubmit { browser.open(browser.addressField) }
            Button("Go") { browser.open(browser.addressField) }
                .buttonStyle(EastSeaPrimaryButtonStyle()).disabled(browser.addressField.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .padding(.horizontal, DesignTokens.Space.s4).padding(.vertical, DesignTokens.Space.s3)
        .background(DesignTokens.Palette.surface.color)
    }

    /// The curated home: what the tab is for, before any address is typed.
    private var home: some View {
        VStack(spacing: DesignTokens.Space.s4) {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
                HStack(spacing: DesignTokens.Space.s3) {
                    EastSeaDawnMark().frame(width: 40, height: 40)
                    Text("Explore the chain").font(DesignTokens.TypeScale.title2.font)
                }
                Text("The block explorer below is part of the app and reads this Mac's own node. Pages you open can connect to your wallet — every request asks first, and Security lists the sites you allowed.")
                    .font(.aeBody).foregroundStyle(DesignTokens.Palette.plateSoft.color)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(DesignTokens.Space.s6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .foregroundStyle(DesignTokens.Palette.plateInk.color)
            .eastSeaNavyPlate(cornerRadius: DesignTokens.Radius.lg)
            Card {
                VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
                    Button {
                        browser.openExplorer()
                    } label: {
                        HStack(spacing: DesignTokens.Space.s3) {
                            Image(systemName: "square.stack.3d.up").font(.aeTitle).foregroundStyle(Color.aether)
                                .frame(width: 32)
                            VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                                Text("Block explorer").font(.aeHeadline)
                                Text("Bundled with the app — reads your own node, signs nothing.")
                                    .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                            }
                            Spacer(minLength: DesignTokens.Space.s2)
                            Image(systemName: "chevron.right").font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.vertical, DesignTokens.Space.s2)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    Rectangle().fill(DesignTokens.Palette.line.color).frame(height: 1)
                    ForEach(Array(BrowserOriginPolicy.curatedDomains).sorted(), id: \.self) { domain in
                        Button {
                            browser.load(URL(string: "https://\(domain)")!)
                        } label: {
                            HStack(spacing: DesignTokens.Space.s3) {
                                Image(systemName: "globe").font(.aeTitle).foregroundStyle(Color.aether)
                                    .frame(width: 32)
                                VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                                    Text(domain).font(.aeHeadline)
                                    Text("The \(Brand.name) website.").font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                                }
                                Spacer(minLength: DesignTokens.Space.s2)
                                Image(systemName: "arrow.up.right").font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.vertical, DesignTokens.Space.s2)
                            .contentShape(Rectangle())
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
                Button("Cancel", role: .cancel) { browser.refuseWarning() }
                    .buttonStyle(EastSeaQuietButtonStyle()).keyboardShortcut(.cancelAction)
                Button("Open Site") { browser.approveWarning() }
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

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
            switch ask.kind {
            case .connect(let origin, let host):
                Text("Connect to \(host.isEmpty ? origin : host)?").font(.aeTitle)
                Text("This site is asking which address this wallet controls. Saying yes shows it **\(Short.address(model.address))** — the address itself, not your key, and not your balances.")
                    .font(.aeBody).fixedSize(horizontal: false, vertical: true)
                Text("You can take this back any time in Security → Connected sites.")
                    .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
            case .send(let origin, let host, let tx, let feeWei):
                Text("\(host.isEmpty ? origin : host) asks to send").font(.aeTitle)
                Grid(alignment: .topLeading, horizontalSpacing: DesignTokens.Space.s4, verticalSpacing: DesignTokens.Space.s3) {
                    row("Action", CallDescribe.action(to: tx.to, data: tx.data), mono: false)
                    if !tx.to.isEmpty { row("To", tx.to, mono: true) }
                    row("Amount", tx.valueWei == "0" ? "—" : "\(Wei.format(tx.valueWei)) \(Brand.networkCoinTicker)")
                    if tx.isPlainTransfer {
                        row("Fee (maximum)", feeWei.map { "\(Wei.format($0)) \(Brand.networkCoinTicker)" } ?? String(localized: "the network's fee at send time"))
                    }
                    row("Gas", tx.gas == 0 ? String(localized: "the wallet's default") : "\(tx.gas)")
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
                Label("Only continue if you started this on \(host.isEmpty ? origin : host). A refused request sends nothing.", systemImage: "exclamationmark.shield")
                    .font(.aeFootnote).foregroundStyle(Color.warn).fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Spacer()
                Button("Refuse", role: .cancel) { browser.refuseAsk() }
                    .buttonStyle(EastSeaQuietButtonStyle()).keyboardShortcut(.cancelAction)
                Button(ask.kind.isConnect ? String(localized: "Connect") : String(localized: "Send")) { browser.approveAsk() }
                    .keyboardShortcut(.defaultAction).buttonStyle(EastSeaPrimaryButtonStyle())
            }
        }
        .padding(DesignTokens.Space.s6)
        .eastSeaSheet()
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
                                Text(site.origin).font(.aeBody.monospaced())
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
