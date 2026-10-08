import Foundation

struct BrowserToolboxApp: Identifiable, Equatable {
    let id: String
    let title: String
    let detail: String
    let url: URL
}

protocol BrowserAppRegistry {
    var apps: [BrowserToolboxApp] { get }
}

/// Requested ERC20 approve arguments, for the per-site panel. This describes
/// the page's request, never a claim that an on-chain approval already exists.
struct BrowserTokenAllowance: Identifiable, Equatable {
    let token: String
    let spender: String
    let amount: String
    let isUnlimited: Bool
    var id: String { token + "." + spender }

    static func parse(token: String, data: String) -> BrowserTokenAllowance? {
        let token = token.lowercased(), data = data.lowercased()
        guard token.count == 42, token.hasPrefix("0x"), token.dropFirst(2).allSatisfy(\.isHexDigit),
              data.count == 138, data.hasPrefix("0x095ea7b3"), data.dropFirst(2).allSatisfy(\.isHexDigit) else { return nil }
        let words = String(data.dropFirst(10))
        let addressWord = String(words.prefix(64))
        guard addressWord.prefix(24).allSatisfy({ $0 == "0" }) else { return nil }
        let amountWord = String(words.suffix(64))
        var decimal = [0]
        for character in amountWord {
            guard let digit = character.hexDigitValue else { return nil }
            var carry = digit
            for index in decimal.indices {
                let value = decimal[index] * 16 + carry
                decimal[index] = value % 10
                carry = value / 10
            }
            while carry > 0 { decimal.append(carry % 10); carry /= 10 }
        }
        return BrowserTokenAllowance(token: token, spender: "0x" + String(addressWord.suffix(40)),
                                     amount: decimal.reversed().map(String.init).joined(),
                                     isUnlimited: amountWord.allSatisfy({ $0 == "f" }))
    }
}

/// The registry lane replaces this inventory through BrowserAppRegistry.
/// The bundled explorer is available now; the sea:// entries deliberately go
/// through the unresolved name seam, rather than inventing trusted websites.
struct PlaceholderBrowserAppRegistry: BrowserAppRegistry {
    var apps: [BrowserToolboxApp] {
        [
            BrowserToolboxApp(id: "explorer", title: String(localized: "Explorer"),
                              detail: String(localized: "Browse blocks and transactions."),
                              url: URL(string: "eastsea-page://explorer/index.html")!),
            BrowserToolboxApp(id: "dex", title: String(localized: "EastSea DEX"),
                              detail: String(localized: "Discover pools and token prices."),
                              url: URL(string: "sea://dex")!),
            BrowserToolboxApp(id: "names", title: String(localized: "EastSea Names"),
                              detail: String(localized: "Find apps by their sea:// name."),
                              url: URL(string: "sea://names")!)
        ]
    }
}

struct BrowserBookmark: Identifiable, Codable, Equatable {
    let id: UUID
    var title: String
    var url: URL
    let createdAt: Date

    init(id: UUID = UUID(), title: String, url: URL, createdAt: Date = Date()) {
        self.id = id
        self.title = title
        self.url = url
        self.createdAt = createdAt
    }
}

struct BrowserHistoryEntry: Identifiable, Codable, Equatable {
    let id: UUID
    var title: String
    let url: URL
    var visitedAt: Date

    init(id: UUID = UUID(), title: String, url: URL, visitedAt: Date = Date()) {
        self.id = id
        self.title = title
        self.url = url
        self.visitedAt = visitedAt
    }
}

enum BrowserHistoryRange: String, CaseIterable, Identifiable {
    case lastHour, today, lastWeek, all
    var id: String { rawValue }
    var title: String {
        switch self {
        case .lastHour: return String(localized: "Last hour")
        case .today: return String(localized: "Today")
        case .lastWeek: return String(localized: "Last 7 days")
        case .all: return String(localized: "All history")
        }
    }

    func cutoff(now: Date, calendar: Calendar = .current) -> Date? {
        switch self {
        case .lastHour: return now.addingTimeInterval(-3600)
        case .today: return calendar.startOfDay(for: now)
        case .lastWeek: return now.addingTimeInterval(-7 * 24 * 3600)
        case .all: return nil
        }
    }
}

struct BrowserTabSnapshot: Identifiable, Codable, Equatable {
    let id: UUID
    var title: String
    var url: URL?
    var isPrivate: Bool

    init(id: UUID = UUID(), title: String = String(localized: "New tab"), url: URL? = nil, isPrivate: Bool = false) {
        self.id = id
        self.title = title
        self.url = url
        self.isPrivate = isPrivate
    }

    private enum CodingKeys: String, CodingKey { case id, title, url, isPrivate }
    enum PersistenceFailure: Error { case privateTab }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(UUID.self, forKey: .id)
        title = try values.decode(String.self, forKey: .title)
        url = try values.decodeIfPresent(URL.self, forKey: .url).flatMap(BrowserInput.canonicalURL)
        // Read old/untrusted records, but BrowserProfile removes them on load.
        isPrivate = try values.decodeIfPresent(Bool.self, forKey: .isPrivate) ?? false
    }

    func encode(to encoder: Encoder) throws {
        guard !isPrivate else { throw PersistenceFailure.privateTab }
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(id, forKey: .id)
        try values.encode(title, forKey: .title)
        try values.encodeIfPresent(url.flatMap(BrowserInput.canonicalURL), forKey: .url)
    }
}

/// One account's non-secret browser metadata. Tab snapshots and visit/zoom
/// methods enforce privacy here as well as at their UI callers, so a future
/// controller cannot accidentally persist a private URL or per-site setting.
struct BrowserProfile: Codable, Equatable {
    var tabs: [BrowserTabSnapshot] = []
    var activeTabID: UUID?
    var bookmarks: [BrowserBookmark] = []
    var history: [BrowserHistoryEntry] = []
    var zoom: [String: Double] = [:]
    var searchEngine: BrowserSearchEngine = .duckDuckGo

    init(tabs: [BrowserTabSnapshot] = [], activeTabID: UUID? = nil,
         bookmarks: [BrowserBookmark] = [], history: [BrowserHistoryEntry] = [],
         zoom: [String: Double] = [:], searchEngine: BrowserSearchEngine = .duckDuckGo) {
        self.tabs = tabs
        self.activeTabID = activeTabID
        self.bookmarks = bookmarks
        self.history = history
        self.zoom = zoom
        self.searchEngine = searchEngine
    }

    @discardableResult
    mutating func addBookmark(title: String, url: URL) -> BrowserBookmark {
        let canonical = BrowserInput.canonicalURL(url)
        let cleanTitle = title.trimmingCharacters(in: .whitespacesAndNewlines)
        let record = BrowserBookmark(title: cleanTitle.isEmpty ? (BrowserCanonicalOrigin.string(for: url) ?? url.absoluteString) : cleanTitle,
                                     url: canonical ?? url)
        guard let canonical else { return record }
        if let index = bookmarks.firstIndex(where: { BrowserInput.canonicalURL($0.url) == canonical }) {
            bookmarks[index].title = record.title
            return bookmarks[index]
        }
        bookmarks.append(record)
        return record
    }

    mutating func removeBookmark(id: UUID) { bookmarks.removeAll { $0.id == id } }

    @discardableResult
    mutating func reorderBookmarks(_ ids: [UUID]) -> Bool {
        guard ids.count == bookmarks.count, Set(ids).count == ids.count, Set(ids) == Set(bookmarks.map(\.id)) else { return false }
        let records = Dictionary(uniqueKeysWithValues: bookmarks.map { ($0.id, $0) })
        bookmarks = ids.compactMap { records[$0] }
        return true
    }

    mutating func recordVisit(url: URL, title: String, isPrivate: Bool = false, visitedAt: Date = Date()) {
        guard !isPrivate, let canonical = BrowserInput.canonicalURL(url) else { return }
        let cleanTitle = title.trimmingCharacters(in: .whitespacesAndNewlines)
        let shown = cleanTitle.isEmpty ? (BrowserCanonicalOrigin.string(for: canonical) ?? canonical.absoluteString) : cleanTitle
        if let first = history.first, first.url == canonical, abs(visitedAt.timeIntervalSince(first.visitedAt)) < 1 {
            history[0].title = shown
            history[0].visitedAt = visitedAt
        } else {
            history.insert(BrowserHistoryEntry(title: shown, url: canonical, visitedAt: visitedAt), at: 0)
        }
        if history.count > 2000 { history.removeLast(history.count - 2000) }
    }

    func searchHistory(_ query: String) -> [BrowserHistoryEntry] {
        let term = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !term.isEmpty else { return history }
        return history.filter { $0.title.localizedCaseInsensitiveContains(term) || $0.url.absoluteString.localizedCaseInsensitiveContains(term) }
    }

    mutating func updateVisitTitle(url: URL, title: String, isPrivate: Bool = false) {
        let clean = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !isPrivate, !clean.isEmpty, let canonical = BrowserInput.canonicalURL(url),
              let index = history.firstIndex(where: { BrowserInput.canonicalURL($0.url) == canonical }) else { return }
        history[index].title = clean
    }

    mutating func clearHistory(_ range: BrowserHistoryRange, now: Date = Date(), calendar: Calendar = .current) {
        guard let cutoff = range.cutoff(now: now, calendar: calendar) else { history.removeAll(); return }
        history.removeAll { $0.visitedAt >= cutoff }
    }

    mutating func saveTabs(_ snapshots: [BrowserTabSnapshot], activeID: UUID?) {
        tabs = snapshots.filter { !$0.isPrivate }
        activeTabID = tabs.contains(where: { $0.id == activeID }) ? activeID : tabs.first?.id
    }

    func zoomFactor(for url: URL) -> Double {
        guard let origin = BrowserCanonicalOrigin.string(for: url), let factor = zoom[origin] else { return 1 }
        return Self.clampedZoom(factor)
    }

    mutating func setZoom(_ factor: Double, for url: URL, isPrivate: Bool = false) {
        guard !isPrivate, let origin = BrowserCanonicalOrigin.string(for: url) else { return }
        let value = Self.clampedZoom(factor)
        if value == 1 { zoom.removeValue(forKey: origin) } else { zoom[origin] = value }
    }

    static func clampedZoom(_ value: Double) -> Double { value.isFinite ? min(3, max(0.5, value)) : 1 }

    private enum CodingKeys: String, CodingKey { case version, tabs, activeTabID, bookmarks, history, zoom, searchEngine }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        tabs = try values.decodeIfPresent([BrowserTabSnapshot].self, forKey: .tabs) ?? []
        activeTabID = try values.decodeIfPresent(UUID.self, forKey: .activeTabID)
        bookmarks = try values.decodeIfPresent([BrowserBookmark].self, forKey: .bookmarks) ?? []
        history = try values.decodeIfPresent([BrowserHistoryEntry].self, forKey: .history) ?? []
        zoom = try values.decodeIfPresent([String: Double].self, forKey: .zoom) ?? [:]
        searchEngine = (try? values.decode(BrowserSearchEngine.self, forKey: .searchEngine)) ?? .duckDuckGo
        sanitize()
    }

    func encode(to encoder: Encoder) throws {
        var profile = self
        profile.sanitize()
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(1, forKey: .version)
        try values.encode(profile.tabs, forKey: .tabs)
        try values.encodeIfPresent(profile.activeTabID, forKey: .activeTabID)
        try values.encode(profile.bookmarks, forKey: .bookmarks)
        try values.encode(profile.history, forKey: .history)
        try values.encode(profile.zoom, forKey: .zoom)
        try values.encode(profile.searchEngine, forKey: .searchEngine)
    }

    private mutating func sanitize() {
        var tabIDs: Set<UUID> = []
        tabs = tabs.filter { !$0.isPrivate && tabIDs.insert($0.id).inserted }
        if !tabs.contains(where: { $0.id == activeTabID }) { activeTabID = tabs.first?.id }
        var bookmarkIDs: Set<UUID> = [], bookmarkURLs: Set<URL> = []
        bookmarks = bookmarks.compactMap { item in
            guard let url = BrowserInput.canonicalURL(item.url), bookmarkIDs.insert(item.id).inserted,
                  bookmarkURLs.insert(url).inserted else { return nil }
            var clean = item
            clean.url = url
            return clean
        }
        var historyIDs: Set<UUID> = []
        history = Array(history.filter { historyIDs.insert($0.id).inserted && BrowserInput.canonicalURL($0.url) != nil }
            .sorted { $0.visitedAt > $1.visitedAt }.prefix(2000))
        var cleanZoom: [String: Double] = [:]
        for (key, factor) in zoom {
            guard let url = URL(string: key), let origin = BrowserCanonicalOrigin.string(for: url), origin == key else { continue }
            let value = Self.clampedZoom(factor)
            if value != 1 { cleanZoom[origin] = value }
        }
        zoom = cleanZoom
    }
}

/// Pass AccountStore.activeAccount.id/address when switching accounts. A
/// missing selection cannot read or write an ownerless/global browser profile.
/// Inject a task-owned defaults suite for WalletScreens and pure tests.
struct BrowserProfileStore {
    let accountID: Int
    let address: String
    private let defaults: UserDefaults
    var key: String { "browserProfile.v1.\(accountID).\(address)" }
    private var hasOwner: Bool {
        accountID > 0 && address.count == 42 && address.hasPrefix("0x") && address.dropFirst(2).allSatisfy(\.isHexDigit)
    }

    enum Failure: Error, LocalizedError {
        case noAccount
        var errorDescription: String? { String(localized: "Choose an account before saving browser data.") }
    }

    init(accountID: Int, address: String, defaults: UserDefaults = .standard) {
        self.accountID = accountID
        self.address = address.lowercased()
        self.defaults = defaults
    }

    func load() -> BrowserProfile {
        guard hasOwner, let data = defaults.data(forKey: key), let profile = try? JSONDecoder().decode(BrowserProfile.self, from: data) else { return BrowserProfile() }
        return profile
    }

    func save(_ profile: BrowserProfile) throws {
        guard hasOwner else { throw Failure.noAccount }
        let data = try JSONEncoder().encode(profile)
        defaults.set(data, forKey: key)
    }

    func remove() { if hasOwner { defaults.removeObject(forKey: key) } }
}
