import SwiftUI
import WebKit
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Native browser chrome stays outside page content and cannot be drawn by a site.
struct BrowserWorkspace: View {
    @ObservedObject var session: BrowserSession
    @ObservedObject var browser: BrowserController
    @AppStorage("developerMode") private var developerMode = false
    @FocusState private var addressFocused: Bool
    @State private var showLibrary = false
    @State private var showPermissions = false
    @State private var showFind = false
    @State private var showBackHistory = false
    @State private var showForwardHistory = false
    @State private var editingFavorite = false

    var body: some View {
        VStack(spacing: 0) {
            BrowserTabStrip(session: session)
            chrome
            if browser.searchQuery == nil, browser.appIdentity?.isDeveloper == true {
                Label("In development · not verified", systemImage: "exclamationmark.triangle.fill")
                    .font(.aeFootnote.bold()).foregroundStyle(DesignTokens.Palette.danger.color)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, DesignTokens.Space.s4).padding(.vertical, DesignTokens.Space.s2)
                    .background(DesignTokens.Palette.danger.color.opacity(0.12))
            }
            if browser.contentLoading {
                ProgressView("Fetching and verifying app files…")
                    .font(.aeFootnote).padding(DesignTokens.Space.s2)
            }
            if browser.isLoading {
                ProgressView(value: browser.estimatedProgress).progressViewStyle(.linear).frame(height: 2)
            }
            if showFind, browser.searchQuery == nil { BrowserFindBar(browser: browser) { closeFind() } }
            if let notice = browser.notice {
                Text(notice).font(.aeFootnote).foregroundStyle(Color.warn)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16).padding(.vertical, 6)
                    .background(Color.warn.opacity(0.12))
            }
            Divider()
            content
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onAppear { browser.resume() }
        .onDisappear { browser.suspend() }
        .onChange(of: browser.addressField) { _, text in
            if addressFocused { browser.suggest(text) }
        }
        .onChange(of: addressFocused) { _, focused in
            if focused { browser.suggest(browser.addressField) }
            else { browser.dismissSuggestions() }
        }
        .sheet(item: warningBinding) { warning in
            SiteWarningSheet(warning: warning, browser: browser).frame(idealWidth: 460)
        }
        .sheet(item: askBinding) { ask in
            ProviderAskSheet(ask: ask, browser: browser).frame(idealWidth: 480)
        }
        .sheet(item: downloadBinding) { prompt in
            BrowserDownloadSheet(prompt: prompt, browser: browser)
        }
        .sheet(item: searchRecordBinding) { record in
            AppSearchRecordSheet(record: record, info: browser.searchRecordInfo) { browser.searchRecord = nil }
        }
        .sheet(isPresented: $showLibrary) { BrowserLibraryPanel(session: session).frame(idealWidth: 540, idealHeight: 580) }
        .sheet(isPresented: $editingFavorite) { BrowserBookmarkEditor(session: session) }
        #if os(macOS)
        .focusedSceneValue(\.browserActions, BrowserActions(session: session, focusAddress: {
            if browser.isSearchHome { addressFocused = false; browser.focusSearch() }
            else { addressFocused = true }
        },
                                                           showFind: { showFind = true }))
        .background(BrowserKeyHandler(closeTab: { session.closeTab() }))
        #endif
    }

    private var askBinding: Binding<BrowserController.PendingAsk?> {
        let presentedID = browser.ask?.id
        return Binding(get: { browser.ask }, set: { value in
            if value == nil, browser.ask?.id == presentedID { browser.refuseAsk(id: presentedID) }
        })
    }

    private var warningBinding: Binding<BrowserController.SiteWarning?> {
        let presentedID = browser.warning?.id
        return Binding(get: { browser.warning }, set: { value in
            if value == nil, browser.warning?.id == presentedID { browser.refuseWarning(id: presentedID) }
        })
    }

    private var downloadBinding: Binding<BrowserController.DownloadPrompt?> {
        let presentedID = browser.downloadPrompt?.id
        return Binding(get: { browser.downloadPrompt }, set: { value in
            if value == nil, browser.downloadPrompt?.id == presentedID { browser.refuseDownload(id: presentedID) }
        })
    }

    private var searchRecordBinding: Binding<AppSearchResult?> {
        let presentedID = browser.searchRecord?.id
        return Binding(get: { browser.searchRecord }, set: { value in
            if value == nil, browser.searchRecord?.id == presentedID { browser.searchRecord = nil }
        })
    }

    private var chrome: some View {
        VStack(spacing: 8) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) { navigation; address; pageTools }
                VStack(spacing: 8) {
                    HStack { navigation; Spacer(minLength: 4); pageTools }
                    address
                }
            }
            if addressFocused {
                let suggestions = session.suggestions(for: browser.addressField)
                if !suggestions.isEmpty {
                    VStack(spacing: 0) {
                        ForEach(suggestions) { suggestion in
                            Button {
                                session.open(suggestion.url.absoluteString)
                                addressFocused = false
                            } label: {
                                HStack(spacing: 8) {
                                    Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                                    Text(suggestion.title).lineLimit(1)
                                    Spacer()
                                    Text(verbatim: BrowserCanonicalOrigin.string(for: suggestion.url) ?? suggestion.url.absoluteString)
                                        .foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                                }.font(.aeFootnote).padding(.vertical, 7).contentShape(Rectangle())
                            }.buttonStyle(.plain)
                        }
                    }
                }
                if browser.suggestionsBusy || !browser.searchSuggestions.isEmpty || browser.suggestionsFailure != nil
                    || browser.suggestionsInfo?.incomplete == true || browser.suggestionsInfo?.usageComplete == false
                    || browser.suggestionsInfo?.sourcesConfigured == false {
                    appSuggestions
                }
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 10)
        .background(.background.secondary)
    }

    private var navigation: some View {
        HStack(spacing: 12) {
            Button { session.goHome() } label: { Image(systemName: "house.fill") }
                .help("Search home").accessibilityLabel("Search home")
            Button { browser.goBack() } label: { Image(systemName: "chevron.left") }
                .disabled(!browser.canGoBack).help("Back").accessibilityLabel("Back")
                .onLongPressGesture { showBackHistory = true }
                .popover(isPresented: $showBackHistory) { navigationHistory(browser.backHistory) }
                .contextMenu { historyButtons(browser.backHistory) }
            Button { browser.goForward() } label: { Image(systemName: "chevron.right") }
                .disabled(!browser.canGoForward).help("Forward").accessibilityLabel("Forward")
                .onLongPressGesture { showForwardHistory = true }
                .popover(isPresented: $showForwardHistory) { navigationHistory(browser.forwardHistory) }
                .contextMenu { historyButtons(browser.forwardHistory) }
            Button { browser.isLoading || browser.contentLoading || browser.searchBusy ? browser.stop() : browser.reload() } label: {
                Image(systemName: browser.isLoading || browser.contentLoading || browser.searchBusy ? "xmark" : "arrow.clockwise")
            }.disabled(browser.webView == nil && browser.currentURL == nil && browser.searchQuery == nil)
                .help(browser.isLoading || browser.contentLoading || browser.searchBusy ? String(localized: "Stop loading") : String(localized: "Reload"))
                .accessibilityLabel(browser.isLoading || browser.contentLoading || browser.searchBusy ? String(localized: "Stop loading") : String(localized: "Reload"))
        }.buttonStyle(.plain).fixedSize()
    }

    private var appSuggestions: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s2) {
                if browser.suggestionsBusy {
                    ProgressView(String(localized: "Searching the chain…")).controlSize(.small)
                }
                if let failure = browser.suggestionsFailure {
                    Text(failure).font(.aeFootnote).foregroundStyle(Color.warn)
                }
                if !browser.suggestionsBusy {
                    AppSearchIndexNotice(info: browser.suggestionsInfo)
                }
                ForEach(Array(browser.searchSuggestions.enumerated()), id: \.offset) { _, record in
                    Button {
                        browser.showSearchRecord(record)
                        addressFocused = false
                    } label: {
                        VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                            Text(verbatim: record.name).font(.aeHeadline)
                            if !record.title.isEmpty && record.title != record.name {
                                Text(verbatim: record.title).font(.aeFootnote).foregroundStyle(.secondary)
                            }
                            AppSearchHashLabel(present: record.verified)
                            AppSearchLookalikeWarning(name: record.lookalike)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                    }.buttonStyle(.plain)
                    Divider()
                }
            }.padding(.vertical, DesignTokens.Space.s2)
        }.frame(maxHeight: 240)
    }

    private func navigationHistory(_ items: [BrowserController.NavigationItem]) -> some View {
        VStack(alignment: .leading, spacing: 12) { historyButtons(items) }.padding(14).frame(idealWidth: 320)
    }

    @ViewBuilder private func historyButtons(_ items: [BrowserController.NavigationItem]) -> some View {
        ForEach(items) { item in
            Button {
                browser.navigate(to: item)
                showBackHistory = false
                showForwardHistory = false
            } label: {
                VStack(alignment: .leading, spacing: 3) {
                    Text(item.title.isEmpty ? String(localized: "Start page") : item.title).lineLimit(1)
                    if let url = item.url {
                        Text(verbatim: BrowserCanonicalOrigin.string(for: url) ?? url.absoluteString).font(.aeCaption).foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    private var address: some View {
        HStack(spacing: 8) {
            Button { showPermissions.toggle() } label: {
                Image(systemName: browser.appIdentity?.isDeveloper == true ? "exclamationmark.triangle" :
                    (browser.connectedAccount != nil ? "link.circle.fill" : (browser.isSecureOrigin ? "lock.fill" : "globe")))
                    .foregroundStyle(browser.connectedAccount != nil ? Color.accentColor : Color.secondary)
            }.buttonStyle(.plain).accessibilityLabel("Site permissions").help("Site permissions")
                .disabled(browser.currentURL == nil || browser.searchQuery != nil)
                .popover(isPresented: $showPermissions) {
                    BrowserSitePermissionsPanel(browser: browser).frame(idealWidth: 360)
                }
            VStack(alignment: .leading, spacing: 2) {
                if browser.searchQuery == nil, !browser.displayOrigin.isEmpty {
                    HStack(spacing: 5) {
                        if browser.connectedAccount != nil { Text("Connected").foregroundStyle(Color.accentColor) }
                        Text(verbatim: browser.displayOrigin).textSelection(.enabled).truncationMode(.middle)
                    }.font(.aeCaption).lineLimit(1)
                }
                TextField("Search or enter a URL or sea name", text: $browser.addressField)
                    .textFieldStyle(.plain).font(.aeBody).focused($addressFocused)
                    .onSubmit { submitAddress() }
                    #if os(iOS)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.webSearch)
                    #endif
            }
            if session.selectedTab.isPrivate {
                Image(systemName: "eye.slash.fill").foregroundStyle(.secondary).accessibilityLabel("Private tab")
            }
            Button { submitAddress() } label: { Image(systemName: "arrow.right.circle.fill") }
                .buttonStyle(.plain).accessibilityLabel("Go").help("Go")
                .disabled(browser.addressField.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        .padding(.horizontal, 10).padding(.vertical, 8)
        .frame(minWidth: 150)
        .background(.background, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.primary.opacity(0.16), lineWidth: 0.5))
    }

    private var pageTools: some View {
        HStack(spacing: 12) {
            Button {
                addressFocused = false
                session.goHome()
            } label: { Image(systemName: "magnifyingglass") }
                .help("Search").accessibilityLabel("Search")
            #if os(macOS)
            if developerMode {
                Button { browser.openLocalAppFolder() } label: { Image(systemName: "folder") }
                    .help("Open a local app folder").accessibilityLabel("Open a local app folder")
            }
            #endif
            Button { session.toggleBookmark() } label: {
                Image(systemName: session.currentBookmark == nil ? "star" : "star.fill")
            }.disabled(browser.currentURL == nil || browser.searchQuery != nil)
                .help("Favorite this page").accessibilityLabel("Favorite this page")
            Button { showLibrary = true } label: { Image(systemName: "book") }
                .help("Bookmarks and history").accessibilityLabel("Bookmarks and history")
            Menu {
                Button("Find in page", systemImage: "magnifyingglass") { showFind = true }.disabled(browser.webView == nil || browser.searchQuery != nil)
                Button("Zoom in", systemImage: "plus.magnifyingglass") { browser.zoomIn() }.disabled(browser.webView == nil || browser.searchQuery != nil)
                Button("Zoom out", systemImage: "minus.magnifyingglass") { browser.zoomOut() }.disabled(browser.webView == nil || browser.searchQuery != nil)
                Button("Reset zoom", systemImage: "1.magnifyingglass") { browser.resetZoom() }.disabled(browser.webView == nil || browser.searchQuery != nil)
                Divider()
                if browser.searchQuery == nil, let url = browser.currentURL {
                    ShareLink(item: url) { Label("Share", systemImage: "square.and.arrow.up") }
                    Button("Open in Safari", systemImage: "safari") { BrowserExternal.openInSafari(url) }
                        .disabled(!["https", "http"].contains(url.scheme?.lowercased() ?? ""))
                    Button("Copy link", systemImage: "link") { BrowserExternal.copy(url) }
                }
                Divider()
                Picker("Search engine", selection: Binding(get: { session.searchEngine }, set: session.setSearchEngine)) {
                    ForEach(BrowserSearchEngine.allCases, id: \.self) { engine in Text(engine.title).tag(engine) }
                }
                Button("Add a favorite", systemImage: "star.badge.plus") { editingFavorite = true }
            } label: { Image(systemName: "ellipsis.circle") }
                .menuStyle(.borderlessButton).fixedSize().help("Page tools").accessibilityLabel("Page tools")
        }.buttonStyle(.plain).fixedSize()
    }

    @ViewBuilder private var content: some View {
        if browser.isSearchHome {
            SeaSearchPage(session: session, browser: browser)
        } else if browser.contentLoading {
            Color.clear.frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let webView = browser.webView {
            WebViewHolder(webView: webView).id(browser.webViewGeneration)
        } else if let url = browser.currentURL, url.scheme?.lowercased() == "sea" {
            ContentUnavailableView {
                Label("Not opened.", systemImage: "exclamationmark.triangle")
            } description: {
                Text(verbatim: url.absoluteString)
            } actions: {
                Button("Reload") { browser.reload() }.buttonStyle(EastSeaPrimaryButtonStyle())
            }
        } else {
            SeaSearchPage(session: session, browser: browser)
        }
    }

    private func submitAddress() { session.open(browser.addressField); addressFocused = false }
    private func closeFind() { showFind = false; browser.clearFind() }
}

struct BrowserTabStrip: View {
    @ObservedObject var session: BrowserSession

    var body: some View {
        HStack(spacing: 6) {
            ScrollView(.horizontal) {
                HStack(spacing: 4) {
                    ForEach(session.tabs) { tab in
                        HStack(spacing: 6) {
                            Button { session.selectTab(tab.id) } label: {
                                HStack(spacing: 5) {
                                    Image(systemName: tab.isPrivate ? "eye.slash" : "globe").font(.caption)
                                    Text(tab.title).font(.aeFootnote).lineLimit(1).truncationMode(.tail)
                                }.contentShape(Rectangle())
                            }.buttonStyle(.plain)
                            Button { session.closeTab(tab.id) } label: { Image(systemName: "xmark").font(.system(size: 9, weight: .semibold)) }
                                .buttonStyle(.plain).accessibilityLabel("Close tab").help("Close tab")
                        }
                        .padding(.horizontal, 10).padding(.vertical, 9)
                        .frame(minWidth: 92, maxWidth: 180)
                        .background(session.activeTabID == tab.id ? Color.primary.opacity(0.09) : Color.clear,
                                    in: RoundedRectangle(cornerRadius: 8))
                        .accessibilityAddTraits(session.activeTabID == tab.id ? .isSelected : [])
                    }
                }
            }.scrollIndicators(.hidden)
            Menu {
                Button("New tab", systemImage: "plus") { session.addTab() }
                Button("New private tab", systemImage: "eye.slash") { session.addTab(isPrivate: true) }
            } label: { Image(systemName: "plus") }
                .menuStyle(.borderlessButton).fixedSize().accessibilityLabel("New tab").help("New tab")
        }
        .padding(.horizontal, 8).padding(.vertical, 4)
        .background(.background.secondary)
    }
}

/// Retained for existing renderer entry points; product Home uses SeaSearchPage.
struct BrowserStartPage: View {
    @ObservedObject var session: BrowserSession
    var body: some View { SeaSearchPage(session: session, browser: session.controller) }
}

struct BrowserFindBar: View {
    @ObservedObject var browser: BrowserController
    var close: () -> Void = {}
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("Find in page", text: $browser.findQuery).textFieldStyle(.roundedBorder).focused($focused)
                .onChange(of: browser.findQuery) { _, query in browser.find(query) }
                .onSubmit { browser.find(browser.findQuery) }
            if let found = browser.findFound, !browser.findQuery.isEmpty {
                Text(found ? String(localized: "Match found") : String(localized: "No matches"))
                    .font(.aeCaption).foregroundStyle(.secondary)
            }
            Button { browser.find(browser.findQuery, backwards: true) } label: { Image(systemName: "chevron.up") }
                .disabled(browser.findQuery.isEmpty).help("Previous match").accessibilityLabel("Previous match")
            Button { browser.find(browser.findQuery) } label: { Image(systemName: "chevron.down") }
                .disabled(browser.findQuery.isEmpty).help("Next match").accessibilityLabel("Next match")
            Button { close() } label: { Image(systemName: "xmark") }
                .help("Close find bar").accessibilityLabel("Close find bar").keyboardShortcut(.escape, modifiers: [])
        }
        .buttonStyle(.plain).padding(12).background(.background.secondary)
        .onAppear { focused = true }
    }
}

struct BrowserSitePermissionsPanel: View {
    @ObservedObject var browser: BrowserController
    var origin: String? = nil
    var account: String? = nil
    var allowances: [String]? = nil

    private var displayedOrigin: String { origin ?? browser.displayOrigin }
    private var displayedAccount: String? { account ?? browser.connectedAccount }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Site permissions").font(.aeTitle)
            Text(verbatim: displayedOrigin.isEmpty ? String(localized: "Start page") : displayedOrigin)
                .font(.aeBody.monospaced()).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            Label(displayedAccount == nil ? String(localized: "Not connected") : String(localized: "Connected"),
                  systemImage: displayedAccount == nil ? "link.badge.plus" : "link.circle.fill")
                .font(.aeHeadline)
            Divider()
            VStack(alignment: .leading, spacing: 5) {
                Text("Connected account").font(.aeFootnote).foregroundStyle(.secondary)
                if let displayedAccount {
                    Text(verbatim: displayedAccount).font(.aeBody.monospaced()).textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                } else { Text("No account shared").font(.aeBody) }
            }
            VStack(alignment: .leading, spacing: 8) {
                Text("Requested permissions").font(.aeFootnote).foregroundStyle(.secondary)
                let requested = allowances ?? browser.requestedAllowances
                if requested.isEmpty { Text("No permissions requested").font(.aeBody) }
                ForEach(requested, id: \.self) { permission in
                    Label(permission, systemImage: "checklist").font(.aeBody)
                }
            }
            if !browser.tokenAllowances.isEmpty {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Requested token allowances").font(.aeFootnote).foregroundStyle(.secondary)
                    ScrollView {
                        VStack(alignment: .leading, spacing: 16) {
                            ForEach(browser.tokenAllowances) { allowance in
                                VStack(alignment: .leading, spacing: 6) {
                                    Text("Token").font(.aeCaption).foregroundStyle(.secondary)
                                    Text(verbatim: allowance.token).font(.aeFootnote.monospaced()).textSelection(.enabled)
                                    Text("Spender").font(.aeCaption).foregroundStyle(.secondary)
                                    Text(verbatim: allowance.spender).font(.aeFootnote.monospaced()).textSelection(.enabled)
                                    Text("Amount (base units)").font(.aeCaption).foregroundStyle(.secondary)
                                    Text(allowance.isUnlimited ? String(localized: "Unlimited") : allowance.amount)
                                        .font(.aeFootnote.monospaced()).textSelection(.enabled)
                                }.fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }.frame(maxHeight: 240)
                }
            }
            Text("Every transaction still asks for approval.").font(.aeFootnote).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if displayedAccount != nil {
                Button("Disconnect", role: .destructive) { browser.disconnectSite() }.buttonStyle(.bordered)
            }
        }.padding(20)
    }
}

struct BrowserLibraryPanel: View {
    @ObservedObject var session: BrowserSession
    @Environment(\.dismiss) private var dismiss
    @State private var historySelected = false
    @State private var query = ""
    @State private var range = BrowserHistoryRange.all
    @State private var editingFavorite = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text("Bookmarks and history").font(.aeTitle); Spacer(); Button("Done") { dismiss() } }
            Picker("Browser library", selection: $historySelected) {
                Text("Favorites").tag(false)
                Text("History").tag(true)
            }.pickerStyle(.segmented)
            if historySelected { history } else { favorites }
        }.padding(20).frame(minWidth: 300, minHeight: 420)
            .sheet(isPresented: $editingFavorite) { BrowserBookmarkEditor(session: session) }
    }

    private var favorites: some View {
        VStack(alignment: .leading, spacing: 12) {
            Button("Add a favorite", systemImage: "star.badge.plus") { editingFavorite = true }
            ScrollView {
                VStack(spacing: 12) {
                    ForEach(session.profile.bookmarks) { bookmark in
                        HStack {
                            Button { session.open(bookmark.url.absoluteString); dismiss() } label: {
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(bookmark.title).font(.aeBody)
                                    Text(verbatim: bookmark.url.absoluteString).font(.aeCaption).foregroundStyle(.secondary)
                                }.frame(maxWidth: .infinity, alignment: .leading)
                            }.buttonStyle(.plain)
                            Button { session.moveBookmark(bookmark.id, by: -1) } label: { Image(systemName: "chevron.up") }.accessibilityLabel("Move up")
                            Button { session.moveBookmark(bookmark.id, by: 1) } label: { Image(systemName: "chevron.down") }.accessibilityLabel("Move down")
                            Button(role: .destructive) { session.removeBookmark(bookmark.id) } label: { Image(systemName: "trash") }.accessibilityLabel("Remove favorite")
                        }
                    }
                }
            }
            if session.profile.bookmarks.isEmpty { Text("No favorites yet").foregroundStyle(.secondary) }
        }
    }

    private var history: some View {
        VStack(alignment: .leading, spacing: 12) {
            TextField("Search history", text: $query).textFieldStyle(.roundedBorder)
            HStack {
                Picker("Clear history", selection: $range) {
                    ForEach(BrowserHistoryRange.allCases, id: \.self) { value in Text(value.title).tag(value) }
                }
                Button("Clear", role: .destructive) { session.clearHistory(range) }.disabled(session.profile.history.isEmpty)
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(session.profile.searchHistory(query)) { entry in
                        Button { session.open(entry.url.absoluteString); dismiss() } label: {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(entry.title).font(.aeBody).lineLimit(1)
                                Text(verbatim: entry.url.absoluteString).font(.aeCaption).foregroundStyle(.secondary).lineLimit(1)
                                Text(entry.visitedAt, style: .date).font(.aeCaption).foregroundStyle(.secondary)
                            }.frame(maxWidth: .infinity, alignment: .leading)
                        }.buttonStyle(.plain)
                    }
                }
            }
            if session.profile.searchHistory(query).isEmpty { Text("No history found").foregroundStyle(.secondary) }
        }
    }
}

struct BrowserBookmarkEditor: View {
    @ObservedObject var session: BrowserSession
    @Environment(\.dismiss) private var dismiss
    @State private var title = ""
    @State private var address = ""
    @State private var invalid = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Add a favorite").font(.aeTitle)
            TextField("Name", text: $title).textFieldStyle(.roundedBorder)
            TextField("Web address", text: $address).textFieldStyle(.roundedBorder)
            if invalid { Text("Use a full web address for a favorite.").font(.aeFootnote).foregroundStyle(Color.warn) }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Add") { add() }.disabled(title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
            }
        }.padding(20).frame(idealWidth: 420)
    }

    private func add() {
        guard let destination = try? BrowserInput.normalize(address), case .url(let url) = destination else { invalid = true; return }
        if case .blocked = BrowserOriginPolicy.classify(url) { invalid = true; return }
        session.addBookmark(title: title.trimmingCharacters(in: .whitespacesAndNewlines), url: url)
        dismiss()
    }
}

struct BrowserDownloadSheet: View {
    let prompt: BrowserController.DownloadPrompt
    @ObservedObject var browser: BrowserController
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Save this download?").font(.aeTitle)
            Text(verbatim: prompt.origin).font(.aeBody.monospaced()).textSelection(.enabled)
            Text(verbatim: prompt.filename).font(.aeHeadline).fixedSize(horizontal: false, vertical: true)
            Text("The file will be saved in Downloads. Downloaded files never open automatically.")
                .font(.aeBody).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if prompt.isExecutable {
                Label("This file can run code. Open it only if you trust its source.", systemImage: "exclamationmark.triangle")
                    .font(.aeFootnote).foregroundStyle(Color.warn).fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { browser.refuseDownload(id: prompt.id) }.keyboardShortcut(.cancelAction)
                Button("Save to Downloads") { browser.approveDownload(id: prompt.id) }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
            }
        }.padding(20).frame(idealWidth: 460)
    }
}

enum BrowserExternal {
    static func copy(_ url: URL) {
        #if os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(url.absoluteString, forType: .string)
        #else
        UIPasteboard.general.url = url
        #endif
    }
    static func openInSafari(_ url: URL) {
        guard ["https", "http"].contains(url.scheme?.lowercased() ?? "") else { return }
        #if os(macOS)
        guard let safari = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.Safari") else { return }
        NSWorkspace.shared.open([url], withApplicationAt: safari, configuration: NSWorkspace.OpenConfiguration())
        #else
        UIApplication.shared.open(url)
        #endif
    }
}
