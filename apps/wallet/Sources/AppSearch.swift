import Foundation

/// The native search surface only displays chain records. It preserves the
/// node's order and warning; it neither ranks records nor opens app bundles.
struct AppSearchResult: Codable, Equatable, Identifiable {
    let name: String
    let title: String
    let description: String
    let category: String
    let publisher: String
    let url: String
    /// A content hash is present, not a wallet trust or certificate decision.
    let verified: Bool
    let usage7d: UInt64
    let createdAt: UInt64
    /// Older nodes may omit this; an explicit false always hides the signal.
    let usageComplete: Bool?
    /// The higher-ranked name with the same UTS-39 skeleton, from the node.
    let lookalike: String?

    var id: String { url + "|" + name + "|" + publisher }

    enum CodingKeys: String, CodingKey {
        case name, title, description, category, publisher, url, verified, lookalike
        case usage7d = "usage_7d"
        case createdAt = "created_at"
        case usageComplete = "usage_complete"
    }

    func usageAvailable(info: AppSearchInfo?) -> Bool {
        usageComplete != false && info?.usageComplete == true
    }
}

/// Node-reported index coverage is separate from each record's content hash.
struct AppSearchInfo: Codable, Equatable {
    let historyComplete: Bool
    let rejectedRecords: UInt64
    let usageComplete: Bool
    /// Older nodes omit this; only an explicit false means no configured sources.
    let sourcesConfigured: Bool?

    var incomplete: Bool { !historyComplete || rejectedRecords > 0 }

    enum CodingKeys: String, CodingKey {
        case historyComplete = "history_complete"
        case rejectedRecords = "rejected_records"
        case usageComplete = "usage_complete"
        case sourcesConfigured = "sources_configured"
    }

    static func decode(from value: Any?) throws -> AppSearchInfo {
        guard let object = value as? [String: Any], JSONSerialization.isValidJSONObject(object),
              let data = try? JSONSerialization.data(withJSONObject: object),
              let info = try? JSONDecoder().decode(Self.self, from: data) else {
            throw AppSearchFailure.malformedResponse
        }
        return info
    }
}

enum AppSearchFailure: Error, Equatable {
    case queryTooLong, malformedResponse, unsupported, unavailable

    var message: String {
        switch self {
        case .queryTooLong:
            return String(localized: "That search is too long. Use a shorter name or phrase.")
        case .malformedResponse:
            return String(localized: "The node returned an unreadable search response.")
        case .unsupported:
            return String(localized: "This node does not support app search yet. Update the node.")
        case .unavailable:
            return String(localized: "Search could not be read from your node. Try again when the node is running.")
        }
    }
}

struct AppSearchRequest: Equatable {
    static let method = "aether_search"
    static let maxQueryBytes = 256
    static let maxLimit = 50
    let query: String
    let limit: Int

    init(query: String, limit: Int = 20) throws {
        let clean = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard clean.utf8.count <= Self.maxQueryBytes else { throw AppSearchFailure.queryTooLong }
        self.query = clean
        self.limit = min(Self.maxLimit, max(1, limit))
    }

    var params: [Any] { [query, limit] }

    /// Reject an invalid response as a whole rather than quietly hiding rows.
    /// The bounded RPC result must contain the same number and order of records.
    func results(from value: Any?) throws -> [AppSearchResult] {
        guard let rows = value as? [Any], rows.count <= limit,
              JSONSerialization.isValidJSONObject(rows),
              let data = try? JSONSerialization.data(withJSONObject: rows),
              let records = try? JSONDecoder().decode([AppSearchResult].self, from: data),
              records.allSatisfy({ AppSearchInput.seaName(in: $0.url) != nil }) else {
            throw AppSearchFailure.malformedResponse
        }
        return records
    }
}

/// Address-bar input routing, kept pure so explicit web navigation and .sea
/// name lookup can be checked without starting a browser or a node.
enum AppSearchInput {
    enum Destination: Equatable {
        case empty, invalid
        case search(String)
        case web(URL)
    }

    static func destination(for text: String) -> Destination {
        let raw = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty else { return .empty }
        if raw.hasPrefix("@") {
            let name = String(raw.dropFirst())
            return name.isEmpty ? .empty : .search(name)
        }
        if raw.lowercased().hasPrefix("sea:") {
            return seaName(in: raw).map(Destination.search) ?? .invalid
        }
        // An explicit scheme must go through the existing web origin policy;
        // a forbidden scheme cannot become a search result or an https host.
        let authority = String(raw.prefix { !"/?#".contains($0) })
        let lowerAuthority = authority.lowercased()
        if raw.contains("://") || raw.range(of: "^[A-Za-z][A-Za-z0-9+.-]*:", options: .regularExpression) != nil {
            // A scheme-less host with a port still keeps its old https route.
            let hostAndPort = authority.split(separator: ":", omittingEmptySubsequences: false)
            if !raw.contains("://"), hostAndPort.count == 2,
               hostAndPort[0].contains(".") || hostAndPort[0].lowercased() == "localhost",
               Int(hostAndPort[1]) != nil {
                return URL(string: "https://\(raw)").map(Destination.web) ?? .invalid
            }
            return URL(string: raw).map(Destination.web) ?? .invalid
        }
        if lowerAuthority.hasSuffix(".sea") {
            return .search(authority)
        }
        if !raw.contains(where: { $0.isWhitespace }),
           authority.contains(".") || lowerAuthority == "localhost" || authority.hasPrefix("[") {
            return URL(string: "https://\(raw)").map(Destination.web) ?? .invalid
        }
        return .search(raw)
    }

    /// Preserve the chain spelling (including Unicode), rather than applying
    /// Foundation's DNS/punycode conversion to a name-service label.
    static func seaName(in text: String) -> String? {
        guard text.lowercased().hasPrefix("sea://") else { return nil }
        let authority = text.dropFirst(6).prefix { !"/?#".contains($0) }
        guard let name = String(authority).removingPercentEncoding, !name.isEmpty,
              !name.contains("@"), !name.contains(":"), !name.contains("/"),
              !name.contains("?"), !name.contains("#"),
              !name.unicodeScalars.contains(where: { CharacterSet.whitespacesAndNewlines.contains($0)
                  || CharacterSet.controlCharacters.contains($0) }) else { return nil }
        return name
    }
}
