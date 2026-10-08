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

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
            switch ask.kind {
            case .connect(let origin, let host):
                Text("Connect to \(host.isEmpty ? origin : host)?").font(.aeTitle)
                Text(verbatim: origin).font(.aeBody.monospaced()).textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                Text("This site is asking which address this wallet controls. Saying yes shows it **\(Short.address(model.address))** — the address itself, not your key, and not your balances.")
                    .font(.aeBody).fixedSize(horizontal: false, vertical: true)
                Text("You can take this back any time in Security → Connected sites.")
                    .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
            case .send(let origin, let host, let tx, let feeWei):
                Text("\(host.isEmpty ? origin : host) asks to send").font(.aeTitle)
                Text(verbatim: origin).font(.aeBody.monospaced()).textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
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
                Button("Refuse", role: .cancel) { browser.refuseAsk(id: ask.id) }
                    .buttonStyle(EastSeaQuietButtonStyle()).keyboardShortcut(.cancelAction)
                Button(ask.kind.isConnect ? String(localized: "Connect") : String(localized: "Send")) { browser.approveAsk(id: ask.id) }
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
