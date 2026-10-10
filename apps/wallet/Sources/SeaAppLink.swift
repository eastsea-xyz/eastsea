import Foundation

/// The AppRegistry's content address, separate from an on-chain .sea name.
/// Parsing proves canonical spelling only; the caller still verifies the pinned
/// registry release and bundle before giving a page an app identity.
struct SeaAppLink: Equatable, Sendable {
    let appID: String
    let appKey: String
    let canonicalURL: String
    let path: String
    let query: String?

    private init(appID: String, appKey: String, path: String, query: String?) {
        self.appID = appID
        self.appKey = appKey
        self.path = path
        self.query = query
        canonicalURL = "sea://" + appKey + path + (query.map { "?" + $0 } ?? "")
    }

    /// Reserve the 52-character bare authority for app IDs, including malformed
    /// encodings. An explicit .sea suffix continues through the name grammar.
    static func isAppCandidate(_ input: String) -> Bool {
        guard let authority = Self.authority(in: input), !authority.contains(".") else { return false }
        let host = authority.split(separator: "@", omittingEmptySubsequences: false).last?
            .split(separator: ":", omittingEmptySubsequences: false).first
        return host?.utf8.count == 52
    }

    static func parse(_ input: String) -> SeaAppLink? {
        guard let appKey = authority(in: input), let appID = decodedAppID(appKey),
              !input.contains("\\"), !input.contains("#"),
              !input.unicodeScalars.contains(where: {
                  CharacterSet.whitespacesAndNewlines.contains($0) || CharacterSet.controlCharacters.contains($0)
                      || $0.properties.isDefaultIgnorableCodePoint
              }) else { return nil }
        let tail = String(input.dropFirst(6 + appKey.count))
        let queryAt = tail.firstIndex(of: "?")
        let rawPath = queryAt.map { String(tail[..<$0]) } ?? tail
        let path = rawPath.isEmpty ? "/" : rawPath
        let query = queryAt.map { String(tail[tail.index(after: $0)...]) }
        guard safePath(path) else { return nil }
        let link = SeaAppLink(appID: appID, appKey: appKey, path: path, query: query)
        // Compare raw escapes before Foundation's checked query setter or URL
        // normalization can repair malformed input into different bytes.
        guard let components = URLComponents(string: link.canonicalURL),
              components.scheme == "sea", components.host == appKey,
              components.user == nil, components.password == nil, components.port == nil,
              components.fragment == nil, components.percentEncodedPath == path,
              components.percentEncodedQuery == query, components.url != nil else { return nil }
        return link
    }

    private static func authority(in input: String) -> String? {
        guard input.prefix(6).lowercased() == "sea://" else { return nil }
        let authority = String(input.dropFirst(6).prefix { !"/?#".contains($0) })
        return authority.isEmpty ? nil : authority
    }

    private static func decodedAppID(_ key: String) -> String? {
        guard key.utf8.count == 52 else { return nil }
        var buffer: UInt32 = 0, bits = 0
        var bytes: [UInt8] = []
        for character in key.utf8 {
            let value: UInt32
            switch character {
            case 97...122: value = UInt32(character - 97)
            case 50...55: value = UInt32(character - 50 + 26)
            default: return nil
            }
            buffer = (buffer << 5) | value
            bits += 5
            if bits >= 8 {
                bits -= 8
                bytes.append(UInt8((buffer >> bits) & 255))
                buffer &= (1 << bits) - 1
            }
        }
        // 256 payload bits occupy 52 symbols; the final four padding bits must
        // be zero so an app ID has exactly one accepted URL spelling.
        guard bytes.count == 32, bits == 4, buffer == 0 else { return nil }
        return "0x" + bytes.map { String(format: "%02x", $0) }.joined()
    }

    /// Match AppBundlePath's asset language without importing its CryptoKit
    /// bundle implementation into the Foundation-only search classifier.
    private static func safePath(_ path: String) -> Bool {
        if path == "/" { return true }
        guard path.hasPrefix("/") else { return false }
        let relative = path.dropFirst()
        guard (1...200).contains(relative.utf8.count), relative.utf8.allSatisfy({
            (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0)
                || $0 == 46 || $0 == 95 || $0 == 45 || $0 == 47
        }) else { return false }
        return relative.split(separator: "/", omittingEmptySubsequences: false)
            .allSatisfy { !$0.isEmpty && $0 != "." && $0 != ".." }
    }
}
