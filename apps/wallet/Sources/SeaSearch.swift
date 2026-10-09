import Foundation

/// Native Explore routing. Classification has no transport: app and name reads
/// belong to the node, and web text gains a destination only on explicit choice.
enum SeaSearch {
    static let homeURL = URL(string: "sea://search")!
    /// The bundled explorer parses heights as JavaScript Numbers.
    static let maxBlockHeight: UInt64 = 9_007_199_254_740_991

    enum ChainLookup: Equatable, Sendable {
        case address(String)
        case transaction(String)
        case block(UInt64)

        var explorerURL: URL {
            let route: String
            switch self {
            case .address(let value): route = "account/" + value
            case .transaction(let value): route = "tx/" + value
            case .block(let height): route = "block/" + String(height)
            }
            var components = URLComponents()
            components.scheme = "eastsea-page"
            components.host = "explorer"
            components.path = "/index.html"
            components.fragment = "/" + route
            return components.url!
        }
    }

    enum Query: Equatable, Sendable {
        case empty, invalid, home
        case name(SeaURL.NameLink)
        case registryApp(SeaAppLink)
        case app(String)
        case chain(ChainLookup)
        case url(URL)
        case web(String)
        /// Existing wallet consent links retain their raw bytes and approval UI.
        case action(host: String, raw: String)

        var localAppQuery: String? {
            switch self {
            case .name(let link): return link.name
            case .app(let text), .web(let text): return text
            default: return nil
            }
        }

        /// Bare app/prefix text may also be an exact name. The caller verifies
        /// the registry binding before treating it as a navigable name result.
        var nameLink: SeaURL.NameLink? {
            switch self {
            case .name(let link): return link
            case .app(let text):
                if let destination = try? SeaURL.browserInput(text), case .name(let link) = destination { return link }
                return nil
            default: return nil
            }
        }

        /// Text and names cannot start navigation through this property.
        /// Chain pages are bundled; external URLs still pass the origin policy.
        var directURL: URL? {
            switch self {
            case .chain(let lookup): return lookup.explorerURL
            case .url(let url): return url
            default: return nil
            }
        }

        var webQuery: String? {
            switch self {
            case .app(let text), .web(let text): return text
            default: return nil
            }
        }
    }

    static func isHome(_ url: URL) -> Bool {
        url.scheme?.lowercased() == "sea" && url.host?.lowercased() == "search"
            && url.user == nil && url.password == nil && url.port == nil
            && (url.path.isEmpty || url.path == "/") && url.query == nil && url.fragment == nil
    }

    static func classify(_ input: String, chainID: UInt64 = 1) -> Query {
        let whitespace = CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: "\u{FEFF}"))
        let text = input.trimmingCharacters(in: whitespace)
        guard !text.isEmpty else { return .empty }
        if let url = URL(string: text), isHome(url), BrowserInput.canonicalURL(url) != nil { return .home }

        if text.utf8.allSatisfy({ (48...57).contains($0) }) {
            guard let height = UInt64(text), height <= maxBlockHeight else { return .invalid }
            return .chain(.block(height))
        }
        let hex = text.lowercased().hasPrefix("0x") ? String(text.dropFirst(2)) : text
        if [40, 64].contains(hex.utf8.count), hex.utf8.allSatisfy({
            (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0)
        }) {
            let value = "0x" + hex.lowercased()
            return .chain(hex.utf8.count == 40 ? .address(value) : .transaction(value))
        }

        if text.hasPrefix("@") {
            let query = normalizedText(String(text.dropFirst()))
            return query.isEmpty ? .empty : .app(query)
        }

        if SeaAppLink.isAppCandidate(text) {
            return SeaAppLink.parse(text).map(Query.registryApp) ?? .invalid
        }

        let scheme = text.firstIndex(of: ":").map { String(text[..<$0]).lowercased() }
        let authority = text.prefix { !"/?#".contains($0) }.lowercased()
        let explicitName = ["sea", "eastsea", "aether"].contains(scheme ?? "")
            || authority.hasSuffix(".sea") || authority.hasSuffix(".aeth")
        if explicitName {
            guard let destination = try? SeaURL.browserInput(input, chainID: chainID) else { return .invalid }
            switch destination {
            case .name(let link): return .name(link)
            case .action(let host, let raw): return .action(host: host, raw: raw)
            case .web: return .invalid
            }
        }

        guard let destination = try? BrowserInput.normalize(text) else { return .invalid }
        switch destination {
        case .url(let url): return .url(url)
        case .sea: return .invalid
        case .search(_, let query):
            // A short name with an explicit path is still a name request. Keep
            // its untouched path/query rather than feeding it to app search.
            if !text.contains(where: \.isWhitespace), text.contains("/") || text.contains("?"),
               let nameDestination = try? SeaURL.browserInput(input, chainID: chainID),
               case .name(let link) = nameDestination {
                return .name(link)
            }
            return query.contains(where: \.isWhitespace) ? .web(query) : .app(query)
        }
    }

    /// Call only when the person chooses the web result. This creates a URL,
    /// never a request, and cannot turn a name/action/chain result into web text.
    static func webSearchURL(for query: Query, engine: BrowserSearchEngine = .duckDuckGo) -> URL? {
        query.webQuery.map { engine.searchURL(for: $0) }
    }

    static func webSearchURL(for input: String, engine: BrowserSearchEngine = .duckDuckGo) -> URL? {
        webSearchURL(for: classify(input), engine: engine)
    }

    private static func normalizedText(_ text: String) -> String {
        text.components(separatedBy: .whitespacesAndNewlines).filter { !$0.isEmpty }.joined(separator: " ")
    }
}
