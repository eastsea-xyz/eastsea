import Foundation

enum BrowserSearchEngine: String, Codable, CaseIterable, Identifiable {
    case duckDuckGo, google, bing
    var id: String { rawValue }
    var title: String {
        switch self {
        case .duckDuckGo: return String(localized: "DuckDuckGo")
        case .google: return String(localized: "Google")
        case .bing: return String(localized: "Bing")
        }
    }

    /// Only the query is sent. Affiliate, source, account and tracking parameters
    /// never travel from the wallet to a search engine.
    func searchURL(for query: String) -> URL {
        let host: String
        switch self {
        case .duckDuckGo: host = "duckduckgo.com"
        case .google: host = "www.google.com"
        case .bing: host = "www.bing.com"
        }
        var components = URLComponents()
        components.scheme = "https"
        components.host = host
        components.path = self == .duckDuckGo ? "/" : "/search"
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-._~"))
        components.percentEncodedQuery = "q=" + (query.addingPercentEncoding(withAllowedCharacters: allowed) ?? "")
        // All components are generated above, rather than interpreted as a URL.
        return components.url!
    }
}

struct SeaNameRequest: Equatable, Codable {
    let name: String
    let url: URL
}

struct SeaNameResolution: Equatable {
    let url: URL
    /// A resolver must verify the name/content binding before setting this.
    /// Merely returning a URL does not grant a secure sea:// indicator.
    let isVerified: Bool
}

protocol SeaNameResolving {
    func resolve(_ request: SeaNameRequest) async throws -> SeaNameResolution?
}

/// Deliberately unresolved until the sea-names registry/parser lane lands.
struct UnresolvedSeaNameResolver: SeaNameResolving {
    func resolve(_ request: SeaNameRequest) async throws -> SeaNameResolution? { nil }
}

enum BrowserInput {
    private static let addressSchemes: Set<String> = ["http", "https", "sea", "eastsea-page"]
    private static let refusedURISchemes: Set<String> = ["javascript", "data", "file", "about", "aether", "eastsea", "mailto", "tel", "ftp"]
    private static let searchOperators: Set<String> = [
        "site", "intitle", "allintitle", "inurl", "allinurl", "intext", "allintext",
        "filetype", "ext", "before", "after", "cache", "define", "related", "source",
        "author", "stocks", "map"
    ]

    enum Destination: Equatable {
        case url(URL)
        case search(URL, query: String)
        case sea(SeaNameRequest)

        var url: URL {
            switch self {
            case .url(let url), .search(let url, _): return url
            case .sea(let request): return request.url
            }
        }
    }

    enum Failure: Error, LocalizedError, Equatable {
        case empty, invalidAddress, credentials, unsupportedScheme, invalidSeaName
        var errorDescription: String? {
            switch self {
            case .empty: return String(localized: "Enter a web address, a sea:// name, or a search term.")
            case .invalidAddress: return String(localized: "That address is not valid.")
            case .credentials: return String(localized: "Addresses with a username or password cannot be opened.")
            case .unsupportedScheme: return String(localized: "Explore only opens web pages and sea:// names.")
            case .invalidSeaName: return String(localized: "That sea:// name is not valid.")
            }
        }
    }

    static func normalize(_ input: String, engine: BrowserSearchEngine = .duckDuckGo) throws -> Destination {
        let text = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { throw Failure.empty }
        let scheme = explicitScheme(in: text)
        if let scheme {
            guard addressSchemes.contains(scheme) else { throw Failure.unsupportedScheme }
            let url = try canonicalURL(text, scheme: scheme)
            if scheme == "sea" {
                guard url.port == nil, let host = url.host, !host.isEmpty else { throw Failure.invalidSeaName }
                return .sea(SeaNameRequest(name: BrowserPunycode.unicodeHost(host), url: url))
            }
            return .url(url)
        }
        if text.hasPrefix("//") { return .url(try canonicalURL("https:" + text, scheme: "https")) }
        if looksLikeAddress(text) { return .url(try canonicalURL("https://" + text, scheme: "https")) }
        let query = text.components(separatedBy: .whitespacesAndNewlines).filter { !$0.isEmpty }.joined(separator: " ")
        return .search(engine.searchURL(for: query), query: query)
    }

    /// Shared by persistence and the visible origin. It refuses credentials
    /// instead of stripping them and turning a deceptive address into a URL.
    static func canonicalURL(_ url: URL) -> URL? {
        guard let scheme = url.scheme?.lowercased(), ["http", "https", "sea", "eastsea-page"].contains(scheme) else { return nil }
        return try? canonicalURL(url.absoluteString, scheme: scheme)
    }

    private static func canonicalURL(_ text: String, scheme: String) throws -> URL {
        let invalid: Failure = scheme == "sea" ? .invalidSeaName : .invalidAddress
        guard !text.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) || $0.properties.isDefaultIgnorableCodePoint }),
              !text.contains("\\"),
              text.range(of: "%(?![A-Fa-f0-9]{2})", options: .regularExpression) == nil,
              var components = URLComponents(string: text), components.scheme?.lowercased() == scheme else { throw invalid }
        guard components.user == nil, components.password == nil else { throw Failure.credentials }
        guard let rawHost = components.host, let host = canonicalHost(rawHost), !host.isEmpty else { throw invalid }
        // An empty, nonnumeric or out-of-range port must not be interpreted as
        // a search or silently dropped. URLComponents alone permits port zero.
        guard let marker = text.range(of: "://") else { throw invalid }
        let authority = text[marker.upperBound...].prefix { $0 != "/" && $0 != "?" && $0 != "#" }
        let portText: Substring?
        if authority.hasPrefix("[") {
            guard let closing = authority.firstIndex(of: "]") else { throw invalid }
            let suffix = authority[authority.index(after: closing)...]
            guard suffix.isEmpty || suffix.hasPrefix(":") else { throw invalid }
            portText = suffix.isEmpty ? nil : suffix.dropFirst()
        } else {
            portText = authority.lastIndex(of: ":").map { authority[authority.index(after: $0)...] }
        }
        if let spelling = portText {
            guard !spelling.isEmpty, spelling.utf8.allSatisfy({ (48...57).contains($0) }),
                  let port = Int(spelling), (1...65535).contains(port), components.port == port else { throw invalid }
        }
        if let port = components.port, !(1...65535).contains(port) { throw invalid }
        if scheme == "sea", components.port != nil { throw Failure.invalidSeaName }
        components.scheme = scheme
        components.host = host
        if (scheme == "https" && components.port == 443) || (scheme == "http" && components.port == 80) { components.port = nil }
        if components.path.isEmpty, scheme == "https" || scheme == "http" { components.path = "/" }
        guard let url = components.url else { throw invalid }
        return url
    }

    static func canonicalHost(_ raw: String) -> String? {
        let decoded = raw.removingPercentEncoding ?? raw
        let lower = decoded.precomposedStringWithCanonicalMapping.lowercased()
        let unbracketed = lower.hasPrefix("[") && lower.hasSuffix("]") ? String(lower.dropFirst().dropLast()) : lower
        if unbracketed.contains(":") {
            guard validIPv6(unbracketed) else { return nil }
            return "[\(unbracketed)]"
        }
        let host = lower.hasSuffix(".") ? String(lower.dropLast()) : lower
        guard !host.isEmpty else { return nil }
        let labels = host.split(separator: ".", omittingEmptySubsequences: false)
        var ascii: [String] = []
        for label in labels {
            guard !label.isEmpty else { return nil }
            let name = String(label)
            let value: String
            if name.hasPrefix("xn--") {
                guard let unicode = BrowserPunycode.decode(String(name.dropFirst(4))),
                      validUnicodeLabel(unicode), !unicode.unicodeScalars.allSatisfy(\.isASCII),
                      BrowserPunycode.encode(unicode).map({ "xn--" + $0 }) == name else { return nil }
                value = name
            } else if name.unicodeScalars.allSatisfy(\.isASCII) {
                guard name.utf8.allSatisfy({ (97...122).contains($0) || (48...57).contains($0) || $0 == 45 }) else { return nil }
                value = name
            } else {
                guard validUnicodeLabel(name), let encoded = BrowserPunycode.encode(name) else { return nil }
                value = "xn--" + encoded
            }
            guard !value.hasPrefix("-"), !value.hasSuffix("-"), value.utf8.count <= 63 else { return nil }
            ascii.append(value)
        }
        let result = ascii.joined(separator: ".")
        guard result.utf8.count <= 253 else { return nil }
        if ascii.count == 4, ascii.allSatisfy({ $0.allSatisfy(\.isNumber) }) {
            guard ascii.allSatisfy({ Int($0).map { (0...255).contains($0) } ?? false }) else { return nil }
        }
        return result
    }

    private static func validUnicodeLabel(_ text: String) -> Bool {
        let allowed = CharacterSet.letters.union(.decimalDigits).union(.nonBaseCharacters).union(CharacterSet(charactersIn: "-"))
        return !text.isEmpty && !text.hasPrefix("-") && !text.hasSuffix("-") && text.unicodeScalars.allSatisfy {
            allowed.contains($0) && !$0.properties.isDefaultIgnorableCodePoint
        }
    }

    private static func explicitScheme(in text: String) -> String? {
        guard let colon = text.firstIndex(of: ":") else { return nil }
        let prefix = String(text[..<colon]).lowercased()
        guard prefix.range(of: "^[a-z][a-z0-9+.-]*$", options: .regularExpression) != nil else { return nil }
        // A dotted host or localhost followed by a port is a schemeless URL.
        let suffix = text[text.index(after: colon)...]
        // Explicit URI syntax wins over search operators, even for an unknown
        // scheme. Unsafe schemes stay URIs when punctuation is followed by a
        // space, so "javascript: alert(1)" cannot become a search request.
        if suffix.hasPrefix("//") { return prefix }
        if prefix.contains(".") || prefix == "localhost" { return nil }
        if addressSchemes.contains(prefix) || refusedURISchemes.contains(prefix) { return prefix }
        if searchOperators.contains(prefix) { return nil }
        if suffix.first?.isWhitespace == true, !suffix.contains("@") { return nil }
        return prefix
    }

    private static func looksLikeAddress(_ text: String) -> Bool {
        let authority = text.prefix { $0 != "/" && $0 != "?" && $0 != "#" }
        if let colon = authority.firstIndex(of: ":") {
            let prefix = authority[..<colon].lowercased()
            if searchOperators.contains(prefix) { return false }
            // A malformed/space-padded host port remains an address failure;
            // it must not fall through to the search engine.
            if !prefix.contains(where: \.isWhitespace), prefix.contains(".") || prefix == "localhost" { return true }
        }
        if authority.hasPrefix("[") { return true }
        guard !authority.contains(where: \.isWhitespace) else { return false }
        return authority.contains(".") || authority.contains("@") || authority.hasPrefix("[") ||
            authority == "localhost" || authority.hasPrefix("localhost:")
    }

    private static func validIPv6(_ host: String) -> Bool {
        guard !host.contains("%"), host.filter({ $0 == ":" }).count >= 2 else { return false }
        let pieces = host.components(separatedBy: "::")
        guard pieces.count <= 2 else { return false }
        var count = 0
        for (index, part) in pieces.enumerated() {
            if part.isEmpty { continue }
            let groups = part.split(separator: ":", omittingEmptySubsequences: false)
            for (groupIndex, group) in groups.enumerated() {
                if group.contains(".") {
                    guard index == pieces.count - 1, groupIndex == groups.count - 1 else { return false }
                    let octets = group.split(separator: ".", omittingEmptySubsequences: false)
                    guard octets.count == 4, octets.allSatisfy({ !$0.isEmpty && $0.allSatisfy(\.isNumber) && Int($0).map { (0...255).contains($0) } == true }) else { return false }
                    count += 2
                } else {
                    guard (1...4).contains(group.count), group.allSatisfy(\.isHexDigit) else { return false }
                    count += 1
                }
            }
        }
        return pieces.count == 2 ? count < 8 : count == 8
    }
}

enum BrowserCanonicalOrigin {
    static func string(for url: URL) -> String? {
        guard let canonical = BrowserInput.canonicalURL(url), let scheme = canonical.scheme, let host = canonical.host else { return nil }
        return string(scheme: scheme, host: host, port: canonical.port ?? 0)
    }

    static func host(for url: URL) -> String? {
        BrowserInput.canonicalURL(url)?.host.flatMap(BrowserInput.canonicalHost)
    }

    /// WKSecurityOrigin uses a raw IPv6 host and port zero for a default port;
    /// URL.host can include brackets. Both must identify the same permission.
    static func string(scheme: String, host: String, port: Int) -> String? {
        let scheme = scheme.lowercased()
        guard ["https", "http", "sea", "eastsea-page"].contains(scheme),
              (0...65535).contains(port), let canonicalHost = BrowserInput.canonicalHost(host),
              scheme != "sea" || port == 0 else { return nil }
        let isDefault = port == 0 || (scheme == "https" && port == 443) || (scheme == "http" && port == 80)
        return "\(scheme)://\(canonicalHost)" + (isDefault ? "" : ":\(port)")
    }

    static func isSecure(_ url: URL, verifiedSea: Bool = false) -> Bool {
        guard BrowserInput.canonicalURL(url) != nil else { return false }
        return url.scheme?.lowercased() == "https" || (url.scheme?.lowercased() == "sea" && verifiedSea)
    }
}

enum BrowserSeaActionPolicy {
    /// Without a resolved name/action distinction, every sea:// navigation
    /// requires a gesture. The resolver seam can narrow this after verification.
    /// Legacy wallet action links follow the same rule.
    static func allows(_ url: URL, hasUserGesture: Bool) -> Bool {
        let scheme = url.scheme?.lowercased() ?? ""
        if scheme == "sea" { return hasUserGesture }
        if ["eastsea", "aether"].contains(scheme) { return hasUserGesture }
        return true
    }
}

/// RFC 3492 bootstring encoding for the ASCII origin and decoding for the
/// confusable check. URL validation happens separately; this is not an IDNA
/// identity or registration policy.
enum BrowserPunycode {
    static func unicodeHost(_ host: String) -> String {
        host.split(separator: ".", omittingEmptySubsequences: false).map { label in
            let lower = label.lowercased()
            return lower.hasPrefix("xn--") ? (decode(String(lower.dropFirst(4))) ?? lower) : lower
        }.joined(separator: ".")
    }

    static func encode(_ text: String) -> String? {
        let scalars = text.unicodeScalars.map { Int($0.value) }
        var output = scalars.filter { $0 < 128 }.compactMap(UnicodeScalar.init).map(String.init).joined()
        var handled = output.utf8.count
        let basic = handled
        if basic > 0 { output += "-" }
        var n = 128, delta = 0, bias = 72
        while handled < scalars.count {
            guard let m = scalars.filter({ $0 >= n }).min() else { return nil }
            let step = (m - n).multipliedReportingOverflow(by: handled + 1)
            guard !step.overflow, !delta.addingReportingOverflow(step.partialValue).overflow else { return nil }
            delta += step.partialValue
            n = m
            for scalar in scalars {
                if scalar < n {
                    guard delta < Int.max else { return nil }
                    delta += 1
                }
                if scalar == n {
                    var q = delta, k = 36
                    while true {
                        let threshold = k <= bias ? 1 : (k >= bias + 26 ? 26 : k - bias)
                        if q < threshold { break }
                        output.append(digit(threshold + (q - threshold) % (36 - threshold)))
                        q = (q - threshold) / (36 - threshold)
                        k += 36
                    }
                    output.append(digit(q))
                    bias = adapt(delta, points: handled + 1, first: handled == basic)
                    delta = 0
                    handled += 1
                }
            }
            guard delta < Int.max, n < Int.max else { return nil }
            delta += 1
            n += 1
        }
        return output
    }

    static func decode(_ text: String) -> String? {
        let bytes = Array(text.utf8)
        var output: [UInt32] = []
        var position = 0
        if let delimiter = bytes.lastIndex(of: 45) {
            guard bytes[..<delimiter].allSatisfy({ $0 < 128 }) else { return nil }
            output = bytes[..<delimiter].map(UInt32.init)
            position = delimiter + 1
        }
        var n = 128, i = 0, bias = 72
        while position < bytes.count {
            let old = i
            var weight = 1, k = 36
            while true {
                guard position < bytes.count, let value = value(bytes[position]) else { return nil }
                position += 1
                let step = value.multipliedReportingOverflow(by: weight)
                guard !step.overflow, !i.addingReportingOverflow(step.partialValue).overflow else { return nil }
                i += step.partialValue
                let threshold = k <= bias ? 1 : (k >= bias + 26 ? 26 : k - bias)
                if value < threshold { break }
                let next = weight.multipliedReportingOverflow(by: 36 - threshold)
                guard !next.overflow else { return nil }
                weight = next.partialValue
                k += 36
            }
            let points = output.count + 1
            bias = adapt(i - old, points: points, first: old == 0)
            let next = n.addingReportingOverflow(i / points)
            guard !next.overflow, let scalar = UnicodeScalar(next.partialValue) else { return nil }
            n = next.partialValue
            i %= points
            output.insert(scalar.value, at: i)
            i += 1
        }
        guard !output.isEmpty else { return nil }
        return String(String.UnicodeScalarView(output.compactMap(UnicodeScalar.init)))
    }

    private static func adapt(_ delta: Int, points: Int, first: Bool) -> Int {
        var value = first ? delta / 700 : delta / 2
        value += value / points
        var k = 0
        while value > 455 { value /= 35; k += 36 }
        return k + 36 * value / (value + 38)
    }

    private static func digit(_ value: Int) -> Character {
        Character(UnicodeScalar(value < 26 ? value + 97 : value - 26 + 48)!)
    }

    private static func value(_ digit: UInt8) -> Int? {
        switch digit {
        case 97...122: return Int(digit - 97)
        case 65...90: return Int(digit - 65)
        case 48...57: return Int(digit - 48) + 26
        default: return nil
        }
    }
}
