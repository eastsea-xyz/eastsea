import Foundation

/// Which URLs the Explore tab may load, and which origin a page's permission
/// belongs to (docs/design/09-wallet.md "인앱 브라우저"). Pure logic on purpose:
/// no WebKit types, so every rule here runs in tests without a view. The
/// look-alike part mirrors the extension's safety.js (normalized, contains,
/// edit-distance 1) over the curated domains instead of token symbols.
enum BrowserOriginPolicy {
    /// The private scheme the bundled explorer is served under (BundledPageScheme).
    static let bundledScheme = "eastsea-page"
    /// Domains the curated home links to; look-alikes of these are what the
    /// phishing guard exists for. `eastsea.xyz` is the project's own site.
    static let curatedDomains: Set<String> = ["eastsea.xyz"]

    /// What a URL in the Explore tab is.
    enum Classification: Equatable {
        /// A page shipped inside the app (the block explorer).
        case bundled
        /// An external site the tab may open, by host — with the curated
        /// domain it imitates (if any) and whether it hides characters in
        /// punycode, for the one-time warning.
        case external(host: String, lookalike: String?, punycode: Bool)
        /// Refused, with the sentence to show under the address bar.
        case blocked(why: String)
    }

    /// The address bar and every link go through here: bundled pages and
    /// https only. http (everywhere — the bundled pages are not http),
    /// file://, and every other scheme are refused, not warned about.
    static func classify(_ url: URL) -> Classification {
        let scheme = url.scheme?.lowercased() ?? ""
        if scheme == bundledScheme { return .bundled }
        guard let host = url.host, !host.isEmpty else { return .blocked(why: String(localized: "That is not a web address.")) }
        guard scheme == "https" else {
            return .blocked(why: scheme == "http"
                ? String(localized: "This address is http, not https. Explore only opens secure pages.")
                : String(localized: "Explore only opens https pages (and the explorer bundled with the app)."))
        }
        let lookalike = looksLikeCurated(host)
        let punycode = host.lowercased().split(separator: ".").contains { $0.hasPrefix("xn--") }
        return .external(host: host, lookalike: lookalike, punycode: punycode)
    }

    /// The key a page's stored permission and pending-request cap hang on:
    /// scheme://host[:port] with the default port left out (WKSecurityOrigin
    /// reports 0 for a default port, so both sides normalize here).
    static func permissionKey(scheme: String, host: String, port: Int) -> String {
        let s = scheme.lowercased()
        let defaultPort = (s == "https" && port == 443) || (s == "http" && port == 80)
        let suffix = (port == 0 || defaultPort) ? "" : ":\(port)"
        return "\(s)://\(host.lowercased())\(suffix)"
    }

    /// The curated domain this host imitates, nil when it does not. A domain
    /// or subdomain of a curated one is trusted outright, exactly like it.
    private static func looksLikeCurated(_ host: String) -> String? {
        let h = host.lowercased()
        for domain in curatedDomains {
            if h == domain || h.hasSuffix(".\(domain)") { return nil }
            if resembles(h, domain) { return domain }
        }
        return nil
    }

    /// Lowercase ASCII alphanumerics only, accents folded, so look-alike
    /// checks see through "EastSeа" with a foreign а (same fold as safety.js).
    static func normalized(_ s: String) -> String {
        let folded = (s.applyingTransform(StringTransform("Latin-ASCII"), reverse: false) ?? s)
            .applyingTransform(.stripCombiningMarks, reverse: false) ?? s
        return folded.lowercased().filter { $0.isASCII && $0.isLetter || $0.isNumber }
    }

    /// Equal, one edit away (hosts of 3+ chars), or containing a curated
    /// domain of 4+ chars — "eastsea.xyz.evil.com" and "eeastsea.xyz" both
    /// trip it (safety.js `resembles`).
    static func resembles(_ a: String, _ b: String) -> Bool {
        let x = normalized(a), y = normalized(b)
        if x == y { return true }
        if y.count >= 4, x.contains(y) { return true }
        return min(x.count, y.count) >= 3 && editDistance(x, y) <= 1
    }

    /// Levenshtein distance (safety.js `editDistance`).
    static func editDistance(_ a: String, _ b: String) -> Int {
        let x = Array(normalized(a)), y = Array(normalized(b))
        var prev = Array(0...y.count)
        for i in 1...max(1, x.count) {
            var cur = [i] + Array(repeating: 0, count: y.count)
            for j in 1...y.count {
                cur[j] = min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (x[i - 1] == y[j - 1] ? 0 : 1))
            }
            prev = cur
            if i >= x.count { break }
        }
        return prev[y.count]
    }

    /// A host or subdomain of a curated domain — the curated entries the
    /// home page links to are trusted without the per-site warning.
    static func isCurated(_ host: String) -> Bool {
        let h = host.lowercased()
        return curatedDomains.contains { h == $0 || h.hasSuffix(".\($0)") }
    }
}

/// Path rules for the bundled pages (BundledPageScheme serves what these
/// accept): plain functions, no WebKit, so they run in tests without a view.
enum BundledPagePath {
    /// `eastsea-page://<dir>/<path>` → the file to read under the bundled
    /// root, or nil when the URL tries to escape (traversal, empty dir).
    static func safePath(dir: String, path: String) -> String? {
        let clean = dir.filter { $0.isLetter || $0.isNumber }
        guard !clean.isEmpty else { return nil }
        var parts: [String] = []
        for seg in path.split(separator: "/") {
            switch seg {
            case "", ".": continue
            case "..": guard !parts.isEmpty else { return nil }; parts.removeLast()
            default: parts.append(String(seg))
            }
        }
        if parts.isEmpty { parts = ["index.html"] }
        return "\(clean)/\(parts.joined(separator: "/"))"
    }

    /// The content type for a bundled file (anything else is refused).
    static func mimeType(for path: String) -> String? {
        switch path.split(separator: ".").last.map(String.init)?.lowercased() {
        case "html": "text/html; charset=utf-8"
        case "js": "text/javascript; charset=utf-8"
        case "css": "text/css; charset=utf-8"
        case "json": "application/json; charset=utf-8"
        case "svg": "image/svg+xml"
        case "png": "image/png"
        case "ico": "image/x-icon"
        case "woff2": "font/woff2"
        default: nil
        }
    }

    /// The header that keeps a bundled page from reaching anywhere but itself
    /// and the node ports on this Mac.
    static let contentSecurityPolicy =
        "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; " +
        "img-src 'self' data:; font-src 'self'; connect-src 'self' http://127.0.0.1:18545 http://127.0.0.1:18546; " +
        "form-action 'none'; base-uri 'none'; frame-ancestors 'none'"
}
