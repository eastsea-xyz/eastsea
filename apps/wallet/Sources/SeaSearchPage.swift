import SwiftUI

/// This view is native; registry strings never become HTML or wallet requests.
struct SeaSearchPage: View {
    @ObservedObject var session: BrowserSession
    @ObservedObject var browser: BrowserController
    @FocusState private var queryFocused: Bool

    var body: some View {
        ScrollView {
            VStack(spacing: DesignTokens.Space.s8) {
                VStack(spacing: DesignTokens.Space.s4) {
                    HStack(spacing: DesignTokens.Space.s3) {
                        EastSeaDawnMark().frame(width: 36, height: 36).accessibilityHidden(true)
                        Text("EastSea Search").font(DesignTokens.TypeScale.title2.font)
                    }
                    queryBox
                    Text("Names, apps and chain lookups use your own node; web search starts only when you choose it.")
                        .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                        .multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                    if session.selectedTab.isPrivate {
                        Label("Private browsing keeps no history or cookies after this tab closes.", systemImage: "eye.slash")
                            .font(.aeFootnote).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                results
                #if os(macOS)
                LiveGlobeView(searchHome: true)
                    .frame(maxWidth: 580)
                #endif
            }
            .frame(maxWidth: 720)
            .padding(.horizontal, DesignTokens.Space.s6)
            .padding(.top, DesignTokens.Space.s12)
            .padding(.bottom, DesignTokens.Space.s8)
            .frame(maxWidth: .infinity)
        }
        .background(DesignTokens.Palette.bg.color)
        .onAppear {
            if !browser.isSearchHome {
                let notice = browser.notice
                browser.goHome()
                browser.notice = notice
            }
            queryFocused = true
        }
        .onChange(of: browser.searchFocusRequest) { _, _ in queryFocused = true }
    }

    private var queryBox: some View {
        HStack(spacing: DesignTokens.Space.s3) {
            Image(systemName: "magnifyingglass").foregroundStyle(DesignTokens.Palette.accent.color)
                .accessibilityHidden(true)
            TextField("Search", text: Binding(
                get: { browser.searchQuery ?? "" }, set: browser.updateSearchQuery))
                .textFieldStyle(.plain).font(DesignTokens.TypeScale.bodyLg.font).focused($queryFocused)
                .accessibilityLabel("Search names, apps, chain or web")
                .help("Name, app, address, block or URL")
                .onSubmit { browser.submitSearch() }
                #if os(iOS)
                .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.webSearch)
                #endif
            Button { browser.submitSearch() } label: {
                Image(systemName: "arrow.right").font(.aeHeadline)
                    .frame(width: 36, height: 36)
            }
            .buttonStyle(.plain).accessibilityLabel("Open first result").help("Open first result")
            .disabled(browser.classifiedSearch == .empty)
        }
        .padding(.horizontal, DesignTokens.Space.s5).padding(.vertical, DesignTokens.Space.s3)
        .background(DesignTokens.Palette.surface.color, in: RoundedRectangle(cornerRadius: DesignTokens.Radius.lg))
        .overlay(RoundedRectangle(cornerRadius: DesignTokens.Radius.lg)
            .stroke(queryFocused ? DesignTokens.Palette.focus.color : DesignTokens.Palette.lineControl.color,
                    lineWidth: queryFocused ? 2 : 1))
    }

    @ViewBuilder private var results: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
            if browser.searchBusy {
                ProgressView("Searching the chain…").font(.aeFootnote)
            }
            if let failure = browser.searchFailure {
                Text(failure).font(.aeFootnote).foregroundStyle(Color.warn)
                Button("Try Again") { browser.search(browser.searchQuery ?? "") }
                    .buttonStyle(EastSeaQuietButtonStyle())
            }
            if let failure = browser.searchNameFailure {
                Text(failure).font(.aeFootnote).foregroundStyle(Color.warn)
            }
            if browser.classifiedSearch == .invalid {
                Text("That address is not valid.").font(.aeFootnote).foregroundStyle(Color.warn)
            }
            if let name = browser.searchName {
                result(title: name.link.name, detail: name.address, icon: "at", kind: "sea:// name") {
                    browser.open(name.link.canonicalURL)
                }
            }
            directResult
            ForEach(browser.searchResults) { record in
                VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                    result(title: record.primaryURL, detail: record.title.isEmpty ? record.name : record.title,
                           icon: "square.stack", kind: "On-chain app or name") { browser.open(record.url) }
                    if let key = record.registryKey { registryKeyDetail(key) }
                    if !record.description.isEmpty {
                        Text(verbatim: record.description).font(.aeFootnote).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    AppSearchLookalikeWarning(name: record.lookalike)
                    HStack {
                        AppSearchHashLabel(present: record.verified)
                        Spacer()
                        Button("Search record") { browser.showSearchRecord(record) }
                            .font(.aeCaption).buttonStyle(.plain)
                    }
                }
                Divider()
            }
            ForEach(browser.classifiedSearch == .empty ? session.builtInApps : browser.searchBuiltins) { app in
                result(title: app.title, detail: app.detail, icon: "cube", kind: "Built-in") { browser.load(app.url) }
            }
            if browser.classifiedSearch.webQuery != nil {
                Button { browser.chooseWebSearch(engine: session.searchEngine) } label: {
                    HStack(spacing: DesignTokens.Space.s3) {
                        Image(systemName: "globe").frame(width: 24).accessibilityHidden(true)
                        Text("Search the web").font(.aeHeadline)
                        Spacer(minLength: 4)
                        Text(verbatim: session.searchEngine.title).font(.aeFootnote)
                        Image(systemName: "arrow.up.right").accessibilityHidden(true)
                    }.padding(.vertical, DesignTokens.Space.s3).contentShape(Rectangle())
                }
                .buttonStyle(.plain).foregroundStyle(DesignTokens.Palette.accent.color)
                .accessibilityHint("Send this query to your selected web search engine")
            }
            if !browser.searchBusy, browser.searchFailure == nil,
               browser.classifiedSearch.localAppQuery != nil {
                if browser.searchName == nil && browser.searchResults.isEmpty && browser.searchBuiltins.isEmpty {
                    Text("No matching apps or names.").font(.aeFootnote).foregroundStyle(.secondary)
                }
                DisclosureGroup("About these results") {
                    AppSearchIndexNotice(info: browser.searchInfo)
                    Text("Search reads your own node. Exact names come first, then prefixes and title or description tokens, then distinct contract callers in the last 7 days, then older records. There are no paid placements, publisher boosts or hidden blocklists.")
                        .font(.aeFootnote).foregroundStyle(.secondary)
                    Text("The content hash label only reports that a hash is recorded on chain. It does not establish the publisher's identity or the app's safety.")
                        .font(.aeFootnote).foregroundStyle(.secondary)
                }.font(.aeCaption)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private var directResult: some View {
        switch browser.classifiedSearch {
        case .chain(let lookup):
            switch lookup {
            case .address(let address):
                result(title: String(localized: "Address"), detail: address, icon: "person.crop.circle", kind: "Chain lookup") { browser.submitSearch() }
            case .transaction(let hash):
                result(title: String(localized: "Transaction"), detail: hash, icon: "arrow.left.arrow.right", kind: "Chain lookup") { browser.submitSearch() }
            case .block(let height):
                result(title: String(localized: "Block"), detail: String(height), icon: "cube", kind: "Chain lookup") { browser.submitSearch() }
            }
        case .url(let url):
            result(title: url.host ?? url.absoluteString, detail: url.absoluteString, icon: "link", kind: "Web address") { browser.submitSearch() }
        case .registryApp(let link):
            if let record = browser.searchResults.first(where: { SeaAppLink.parse($0.url)?.appID == link.appID }),
               record.primaryURL != record.url {
                result(title: record.primaryURL, detail: record.title.isEmpty ? record.name : record.title,
                       icon: "square.stack", kind: "On-chain app or name") { browser.submitSearch() }
                registryKeyDetail(link.appKey)
            } else {
                result(title: link.appKey, detail: link.canonicalURL, icon: "square.stack", kind: "On-chain app or name") { browser.submitSearch() }
            }
        case .action(let host, _):
            result(title: host, detail: browser.searchQuery ?? "", icon: "wallet.pass", kind: "Wallet action") { browser.submitSearch() }
        default: EmptyView()
        }
    }

    private func registryKeyDetail(_ key: String) -> some View {
        HStack(spacing: DesignTokens.Space.s2) {
            Text(verbatim: key).font(.aeCaption.monospaced()).foregroundStyle(.secondary)
                .lineLimit(1).truncationMode(.middle).textSelection(.enabled)
                .frame(maxWidth: 240, alignment: .leading)
            Button { Clipboard.copy(key) } label: {
                Image(systemName: "doc.on.doc").font(.aeCaption)
            }
            .buttonStyle(.plain).accessibilityLabel("Copy registry hash").help("Copy registry hash")
        }
    }

    private func result(title: String, detail: String, icon: String, kind: LocalizedStringKey,
                        action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: DesignTokens.Space.s3) {
                Image(systemName: icon).foregroundStyle(DesignTokens.Palette.accent.color)
                    .frame(width: 24).accessibilityHidden(true)
                VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                    HStack {
                        Text(verbatim: title).font(.aeHeadline).lineLimit(1)
                        Spacer(minLength: 4)
                        Text(kind).font(.aeCaption).foregroundStyle(.secondary)
                    }
                    Text(verbatim: detail).font(.aeFootnote).foregroundStyle(.secondary)
                        .lineLimit(2).truncationMode(.middle)
                }
                Image(systemName: "arrow.up.right").font(.aeCaption).foregroundStyle(.secondary)
                    .accessibilityHidden(true)
            }
            .padding(.vertical, DesignTokens.Space.s3).contentShape(Rectangle())
        }
        .buttonStyle(.plain).accessibilityElement(children: .combine).accessibilityHint("Open result")
    }
}
