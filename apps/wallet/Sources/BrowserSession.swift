import Combine
import Foundation

/// Each tab owns its bridge and ephemeral WebKit stores. Account changes destroy
/// these controllers instead of reusing a page with another wallet underneath it.
@MainActor
final class BrowserTab: Identifiable {
    let id: UUID
    let isPrivate: Bool
    let controller: BrowserController
    var savedURL: URL?
    var savedTitle: String
    var hasLoaded = false

    init(id: UUID = UUID(), isPrivate: Bool = false, url: URL? = nil, title: String = "") {
        self.id = id
        self.isPrivate = isPrivate
        self.savedURL = url
        self.savedTitle = title
        self.controller = BrowserController(isPrivate: isPrivate)
    }

    var title: String {
        if !controller.title.isEmpty { return controller.title }
        if !savedTitle.isEmpty { return savedTitle }
        return isPrivate ? String(localized: "Private tab") : String(localized: "New tab")
    }

    var snapshot: BrowserTabSnapshot {
        BrowserTabSnapshot(id: id, title: title, url: controller.currentURL ?? savedURL, isPrivate: isPrivate)
    }
}

struct BrowserSuggestion: Identifiable {
    var id: String { url.absoluteString }
    let title: String
    let url: URL
}

@MainActor
final class BrowserSession: ObservableObject {
    @Published private(set) var tabs: [BrowserTab] = []
    @Published private(set) var activeTabID: UUID = UUID()
    @Published private(set) var profile = BrowserProfile()
    let registry: any BrowserAppRegistry
    var seaResolver: any SeaNameResolving
    private let defaults: UserDefaults
    private weak var model: WalletModel?
    private var store: BrowserProfileStore?
    private var accountSubscription: AnyCancellable?
    private var defaultsSubscription: AnyCancellable?
    private var tabSubscriptions: [UUID: AnyCancellable] = [:]
    private var resolutionTask: Task<Void, Never>?

    init(defaults: UserDefaults = .standard,
         registry: any BrowserAppRegistry = PlaceholderBrowserAppRegistry(),
         seaResolver: any SeaNameResolving = UnresolvedSeaNameResolver()) {
        self.defaults = defaults
        self.registry = registry
        self.seaResolver = seaResolver
        let tab = BrowserTab()
        tabs = [tab]
        activeTabID = tab.id
        defaultsSubscription = NotificationCenter.default.publisher(for: UserDefaults.didChangeNotification, object: defaults)
            .sink { [weak self] _ in
                Task { @MainActor [weak self] in self?.refreshSharedProfile() }
            }
    }

    var selectedTab: BrowserTab { tabs.first { $0.id == activeTabID } ?? tabs[0] }
    var controller: BrowserController { selectedTab.controller }
    var recentSites: [BrowserHistoryEntry] {
        var seen: Set<String> = []
        return profile.history.filter {
            seen.insert(BrowserCanonicalOrigin.string(for: $0.url) ?? $0.url.absoluteString).inserted
        }.prefix(6).map { $0 }
    }

    func attach(model: WalletModel) {
        guard self.model !== model else { return }
        self.model = model
        accountSubscription = model.accountStore.activeAccountPublisher
            .removeDuplicates { before, after in
                before?.id == after?.id && before?.address.lowercased() == after?.address.lowercased()
            }
            .sink { [weak self] account in self?.activate(account) }
    }

    private func activate(_ account: WalletAccount?) {
        persist()
        resolutionTask?.cancel()
        tabs.forEach { $0.controller.close() }
        tabSubscriptions.removeAll()
        store = account.map { BrowserProfileStore(accountID: $0.id, address: $0.address, defaults: defaults) }
        profile = store?.load() ?? BrowserProfile()
        tabs = profile.tabs.filter { !$0.isPrivate }.map {
            BrowserTab(id: $0.id, url: $0.url, title: $0.title)
        }
        if tabs.isEmpty { tabs = [BrowserTab()] }
        activeTabID = tabs.first { $0.id == profile.activeTabID }?.id ?? tabs[0].id
        tabs.forEach(configure)
        loadSelectedIfNeeded()
    }

    private func configure(_ tab: BrowserTab) {
        if let model { tab.controller.attach(model: model) }
        tab.controller.onCommit = { [weak self, weak tab] url, title in
            guard let self, let tab, self.tabs.contains(where: { $0 === tab }) else { return }
            tab.savedURL = url
            tab.savedTitle = title
            self.editProfile { $0.recordVisit(url: url, title: title, isPrivate: tab.isPrivate) }
        }
        tab.controller.onTitleChange = { [weak self, weak tab] url, title in
            guard let self, let tab, !tab.isPrivate, self.tabs.contains(where: { $0 === tab }) else { return }
            tab.savedTitle = title
            self.editProfile { $0.updateVisitTitle(url: url, title: title) }
        }
        tab.controller.onHome = { [weak self, weak tab] in
            guard let self, let tab, self.tabs.contains(where: { $0 === tab }) else { return }
            tab.savedURL = nil
            tab.savedTitle = ""
            self.persist()
        }
        tab.controller.zoomProvider = { [weak self, weak tab] origin in
            guard let self, let tab else { return 1 }
            if tab.isPrivate { return 1 }
            return self.profile.zoom[origin] ?? 1
        }
        tab.controller.onZoomChange = { [weak self, weak tab] origin, factor in
            guard let self, let tab, let url = URL(string: origin) else { return }
            guard !tab.isPrivate else { return }
            self.editProfile { $0.setZoom(factor, for: url, isPrivate: false) }
        }
        tab.controller.phishingCheck = { [weak self] url in
            guard let self else { return nil }
            return BrowserConfusables.warning(for: url, bookmarks: self.profile.bookmarks, apps: self.registry.apps)?.protectedName
        }
        tab.controller.onSeaLink = { [weak self, weak tab] url in
            guard let self, let tab, self.selectedTab.id == tab.id else { return }
            self.open(url.absoluteString)
        }
        tabSubscriptions[tab.id] = tab.controller.objectWillChange.sink { [weak self] _ in
            // WebKit's Published notifications occur before the value changes.
            Task { @MainActor [weak self] in self?.objectWillChange.send() }
        }
    }

    func addTab(isPrivate: Bool = false, url: URL? = nil) {
        resolutionTask?.cancel()
        controller.suspend()
        let tab = BrowserTab(isPrivate: isPrivate, url: url)
        tabs.append(tab)
        configure(tab)
        activeTabID = tab.id
        loadSelectedIfNeeded()
        persist()
    }

    func selectTab(_ id: UUID) {
        guard tabs.contains(where: { $0.id == id }), activeTabID != id else { return }
        resolutionTask?.cancel()
        controller.suspend()
        activeTabID = id
        controller.resume()
        loadSelectedIfNeeded()
        persist()
    }

    func closeTab(_ id: UUID? = nil) {
        let closingID = id ?? activeTabID
        guard let index = tabs.firstIndex(where: { $0.id == closingID }) else { return }
        let closing = tabs[index]
        closing.controller.close()
        tabs.remove(at: index)
        tabSubscriptions.removeValue(forKey: closingID)
        if tabs.isEmpty {
            let tab = BrowserTab()
            tabs = [tab]
            configure(tab)
        }
        if activeTabID == closingID {
            resolutionTask?.cancel()
            activeTabID = tabs[min(index, tabs.count - 1)].id
            controller.resume()
            loadSelectedIfNeeded()
        }
        persist()
    }

    private func loadSelectedIfNeeded() {
        let tab = selectedTab
        guard !tab.hasLoaded else { return }
        tab.hasLoaded = true
        #if !WALLET_SCREENS
        if let url = tab.savedURL { tab.controller.load(url) }
        #endif
    }

    func goHome() {
        resolutionTask?.cancel()
        selectedTab.savedURL = nil
        selectedTab.savedTitle = ""
        controller.goHome()
        persist()
    }

    func open(_ input: String) {
        resolutionTask?.cancel()
        let tab = selectedTab
        do {
            switch try BrowserInput.normalize(input, engine: profile.searchEngine) {
            case .url(let url), .search(let url, _):
                tab.hasLoaded = true
                tab.controller.load(url)
            case .sea(let request):
                guard tab.controller.confirmSeaName(request.url) else { return }
                let resolver = seaResolver
                resolutionTask = Task { [weak self, weak tab] in
                    do {
                        let resolution = try await resolver.resolve(request)
                        guard !Task.isCancelled, let self, let tab, self.selectedTab.id == tab.id else { return }
                        guard let resolution else {
                            tab.controller.notice = String(localized: "This sea name is not available yet.")
                            return
                        }
                        // The parser seam must produce a verified HTTPS destination;
                        // a name alone never changes the native signing origin.
                        guard resolution.isVerified, resolution.url.scheme?.lowercased() == "https" else {
                            tab.controller.notice = String(localized: "This sea name could not be verified.")
                            return
                        }
                        tab.hasLoaded = true
                        tab.controller.load(resolution.url)
                    } catch {
                        guard !Task.isCancelled else { return }
                        tab?.controller.notice = String(localized: "This sea name could not be verified.")
                    }
                }
            }
        } catch let error as BrowserInput.Failure {
            controller.notice = error.localizedDescription
        } catch {
            controller.notice = String(localized: "Enter a URL, a sea name, or search terms.")
        }
    }

    func suggestions(for query: String) -> [BrowserSuggestion] {
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !needle.isEmpty else { return [] }
        var candidates = profile.bookmarks.map { BrowserSuggestion(title: $0.title, url: $0.url) }
        if !selectedTab.isPrivate { candidates += profile.history.map { BrowserSuggestion(title: $0.title, url: $0.url) } }
        candidates += registry.apps.map { BrowserSuggestion(title: $0.title, url: $0.url) }
        var seen: Set<String> = []
        return candidates.filter {
            ($0.title.lowercased().contains(needle) || $0.url.absoluteString.lowercased().contains(needle))
                && seen.insert($0.id).inserted
        }.prefix(6).map { $0 }
    }

    var currentBookmark: BrowserBookmark? {
        guard let url = controller.currentURL else { return nil }
        return profile.bookmarks.first { $0.url == url }
    }

    func toggleBookmark() {
        guard let url = controller.currentURL else { return }
        editProfile { profile in
            if let bookmark = profile.bookmarks.first(where: { $0.url == url }) { profile.removeBookmark(id: bookmark.id) }
            else { profile.addBookmark(title: selectedTab.title, url: url) }
        }
    }

    func addBookmark(title: String, url: URL) { editProfile { $0.addBookmark(title: title, url: url) } }
    func removeBookmark(_ id: UUID) { editProfile { $0.removeBookmark(id: id) } }
    func moveBookmark(_ id: UUID, by offset: Int) {
        refreshSharedProfile()
        var ids = profile.bookmarks.map(\.id)
        guard let index = ids.firstIndex(of: id) else { return }
        let destination = index + offset
        guard ids.indices.contains(destination) else { return }
        ids.swapAt(index, destination)
        editProfile { $0.reorderBookmarks(ids) }
    }
    func clearHistory(_ range: BrowserHistoryRange) { editProfile { $0.clearHistory(range) } }
    func setSearchEngine(_ engine: BrowserSearchEngine) { editProfile { $0.searchEngine = engine } }

    /// Windows own their live tabs; account records have a single serialized
    /// read/modify/write on the main actor, so a stale window cannot erase them.
    private func refreshSharedProfile() {
        guard let store else { return }
        let latest = store.load()
        if profile.bookmarks != latest.bookmarks { profile.bookmarks = latest.bookmarks }
        if profile.history != latest.history { profile.history = latest.history }
        if profile.zoom != latest.zoom { profile.zoom = latest.zoom }
        if profile.searchEngine != latest.searchEngine { profile.searchEngine = latest.searchEngine }
    }

    private func editProfile(_ edit: (inout BrowserProfile) -> Void) {
        refreshSharedProfile()
        edit(&profile)
        persist(refreshShared: false)
    }

    private func persist(refreshShared: Bool = true) {
        if refreshShared { refreshSharedProfile() }
        profile.saveTabs(tabs.map(\.snapshot), activeID: activeTabID)
        do { try store?.save(profile) }
        catch { controller.notice = String(localized: "Browser changes could not be saved on this device.") }
    }

    #if WALLET_SCREENS
    func seedPreview() {
        profile = BrowserProfile()
        profile.addBookmark(title: String(localized: "Block explorer"), url: URL(string: "eastsea-page://explorer/index.html")!)
        profile.addBookmark(title: String(localized: "EastSea website"), url: URL(string: "https://eastsea.xyz")!)
        profile.recordVisit(url: URL(string: "https://eastsea.xyz")!, title: String(localized: "EastSea website"), isPrivate: false)
        let regular = BrowserTab()
        let explorer = BrowserTab(title: String(localized: "Block explorer"))
        let privateTab = BrowserTab(isPrivate: true)
        tabs = [regular, explorer, privateTab]
        tabSubscriptions.removeAll()
        tabs.forEach(configure)
        activeTabID = regular.id
    }
    #endif
}
