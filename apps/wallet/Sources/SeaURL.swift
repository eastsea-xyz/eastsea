import Foundation

/// Pure classification of EastSea links and browser address-bar input.
/// Actions retain their original bytes; callers still own the approval UI.
enum SeaURL {
    struct NameLink: Equatable, Sendable {
        /// Canonical DNS hostname, always ending in .sea.
        let name: String
        let canonicalURL: String
        /// Raw path and query, including the spelling of percent escapes.
        let path: String
        let query: String?
        let isLegacy: Bool
        /// The .aeth spelling for an explicitly requested 7780 alias.
        let registryName: String
    }

    enum Link: Equatable, Sendable {
        case name(NameLink)
        case action(host: String, raw: String)
    }

    enum BrowserInput: Equatable, Sendable {
        case name(NameLink)
        case action(host: String, raw: String)
        case web(URL)
    }

    enum ParseError: Swift.Error, Equatable, Sendable {
        case invalidURL
        case invalidName
        case externalTLD
        case reservedName
        case legacyNameUnsupported
        case unsupportedScheme
    }

    static let reservedHosts: Set<String> = [
        "pay", "call", "connect", "tx", "app", "follow", "name", "wallet", "settings",
        "send", "receive", "sign", "deploy", "open"
    ]

    // Match JavaScript String.trim, including its BOM handling. Platform
    // whitespace sets differ on NEL and BOM; names must not depend on that.
    private static let inputWhitespace = CharacterSet(charactersIn:
        "\u{0009}\u{000A}\u{000B}\u{000C}\u{000D}\u{0020}\u{00A0}\u{1680}" +
        "\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}" +
        "\u{2008}\u{2009}\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}")

    static func parse(_ raw: String, chainID: UInt64 = 1) throws -> Link {
        try checkRaw(raw)
        guard let (rawScheme, body) = schemeParts(raw) else { throw ParseError.invalidURL }
        let scheme = rawScheme.lowercased()
        guard ["sea", "eastsea", "aether"].contains(scheme) else { throw ParseError.unsupportedScheme }

        // URLComponents historically accepted aether:pay?… and
        // eastsea:pay?… through the path fallback. Preserve those too.
        guard body.hasPrefix("//") else {
            let host = String(body.prefix { $0 != "?" && $0 != "#" })
            if scheme != "sea", reservedHosts.contains(host) {
                return .action(host: host, raw: raw)
            }
            throw ParseError.invalidURL
        }

        let authority = String(body.dropFirst(2))
        let (host, _) = try authorityParts(authority)
        if reservedHosts.contains(host) { return .action(host: host, raw: raw) }
        guard scheme != "aether" else { throw ParseError.unsupportedScheme }
        return .name(try nameLink(authority, chainID: chainID))
    }

    static func browserInput(_ raw: String, chainID: UInt64 = 1) throws -> BrowserInput {
        let input = raw.trimmingCharacters(in: inputWhitespace)
        try checkRaw(input)
        if let (scheme, body) = schemeParts(input) {
            if ["http", "https"].contains(scheme.lowercased()), body.hasPrefix("//") {
                guard let url = URL(string: input), let host = url.host, !host.isEmpty else {
                    throw ParseError.invalidURL
                }
                return .web(url)
            }
            switch try parse(input, chainID: chainID) {
            case .name(let name): return .name(name)
            case .action(let host, let raw): return .action(host: host, raw: raw)
            }
        }
        // A bare reserved word must not silently become an action request.
        return .name(try nameLink(input, chainID: chainID))
    }

    /// HTTPS is offered only when refusal is solely about an external TLD.
    /// Invalid authorities never get repaired into a different site's URL.
    static func suggestedHTTPS(_ raw: String) -> URL? {
        let input = raw.trimmingCharacters(in: inputWhitespace)
        do {
            _ = try browserInput(input)
            return nil
        } catch ParseError.externalTLD {
            // Continue with the same already-validated raw authority.
        } catch {
            return nil
        }

        var body = input
        if let (scheme, value) = schemeParts(input),
           ["sea", "eastsea"].contains(scheme.lowercased()), value.hasPrefix("//") {
            body = String(value.dropFirst(2))
        }
        guard let (host, tail) = try? authorityParts(body) else { return nil }
        let suffix = tail.hasPrefix("/") ? tail : "/" + tail
        return URL(string: "https://" + host + suffix)
    }

    // This is resolution grammar. The registry separately enforces the
    // 3–32 character limit for a registrable second-level name.
    private static func nameLink(_ raw: String, chainID: UInt64) throws -> NameLink {
        try checkRaw(raw)
        let (host, tail) = try authorityParts(raw)
        guard !tail.contains("#"), validPercentEscapes(tail) else { throw ParseError.invalidURL }
        guard host.utf8.count <= 253 else { throw ParseError.invalidName }
        let labels = host.split(separator: ".", omittingEmptySubsequences: false)
        for label in labels {
            let bytes = Array(label.utf8)
            guard !bytes.isEmpty, bytes.count <= 63,
                  bytes.first != 45, bytes.last != 45,
                  bytes.allSatisfy({ isLowercaseLetter($0) || isDigit($0) || $0 == 45 }) else {
                throw ParseError.invalidName
            }
        }

        let name: String
        let isLegacy: Bool
        if labels.count == 1 {
            name = host + ".sea"
            isLegacy = false
        } else if labels.last == "sea" {
            name = host
            isLegacy = false
        } else if labels.last == "aeth" {
            guard chainID == 7780 else { throw ParseError.legacyNameUnsupported }
            name = String(host.dropLast(5)) + ".sea"
            isLegacy = true
        } else {
            throw ParseError.externalTLD
        }
        guard name.utf8.count <= 253 else { throw ParseError.invalidName }
        guard let secondLevel = name.split(separator: ".").dropLast().last,
              !reservedHosts.contains(String(secondLevel)) else { throw ParseError.reservedName }

        let queryAt = tail.firstIndex(of: "?")
        let rawPath = queryAt.map { String(tail[..<$0]) } ?? tail
        let path = rawPath.isEmpty ? "/" : rawPath
        let query = queryAt.map { String(tail[tail.index(after: $0)...]) }
        let canonicalURL = "sea://" + name + path + (query.map { "?" + $0 } ?? "")
        return NameLink(name: name, canonicalURL: canonicalURL, path: path, query: query,
                        isLegacy: isLegacy, registryName: isLegacy ? host : name)
    }

    /// Inspect the raw authority before Foundation can lowercase, decode,
    /// strip userinfo or otherwise reinterpret its hostname.
    private static func authorityParts(_ raw: String) throws -> (host: String, tail: String) {
        let end = raw.firstIndex { $0 == "/" || $0 == "?" || $0 == "#" } ?? raw.endIndex
        let host = String(raw[..<end])
        let tail = String(raw[end...])
        guard !host.isEmpty, !host.contains(where: { "@:[]\\".contains($0) }) else {
            throw ParseError.invalidURL
        }
        return (host, tail)
    }

    private static func checkRaw(_ raw: String) throws {
        guard !raw.isEmpty, !raw.utf8.contains(where: { $0 <= 32 || $0 == 127 || $0 == 92 }) else {
            throw ParseError.invalidURL
        }
    }

    private static func schemeParts(_ raw: String) -> (scheme: String, body: String)? {
        guard let colon = raw.firstIndex(of: ":") else { return nil }
        let scheme = String(raw[..<colon])
        let bytes = Array(scheme.utf8)
        guard let first = bytes.first, isLetter(first),
              bytes.dropFirst().allSatisfy({ isLetter($0) || isDigit($0) || [43, 45, 46].contains($0) }) else {
            return nil
        }
        return (scheme, String(raw[raw.index(after: colon)...]))
    }

    private static func validPercentEscapes(_ raw: String) -> Bool {
        let bytes = Array(raw.utf8)
        for index in bytes.indices where bytes[index] == 37 {
            if index + 2 >= bytes.count || !isHex(bytes[index + 1]) || !isHex(bytes[index + 2]) {
                return false
            }
        }
        return true
    }

    private static func isLowercaseLetter(_ byte: UInt8) -> Bool { byte >= 97 && byte <= 122 }
    private static func isLetter(_ byte: UInt8) -> Bool { isLowercaseLetter(byte) || (byte >= 65 && byte <= 90) }
    private static func isDigit(_ byte: UInt8) -> Bool { byte >= 48 && byte <= 57 }
    private static func isHex(_ byte: UInt8) -> Bool {
        isDigit(byte) || (byte >= 65 && byte <= 70) || (byte >= 97 && byte <= 102)
    }
}
