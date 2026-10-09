import Foundation

/// A separate, read-only origin for the Network globe. No wallet provider is
/// installed here, and even loopback connections are forbidden by its CSP.
enum LiveGlobeBundlePolicy {
    static let scheme = "eastsea-globe"
    static let host = "network"
    static let entry = URL(string: "eastsea-globe://network/index.html")!
    static let contentSecurityPolicy = "default-src 'none'; script-src 'self'; script-src-attr 'none'; style-src 'self'; style-src-attr 'none'; connect-src 'none'; img-src 'none'; font-src 'none'; object-src 'none'; frame-src 'none'; media-src 'none'; worker-src 'none'; base-uri 'none'; form-action 'none'"

    private static let assets: Set<String> = [
        "index.html", "wallet-host.js", "wallet.css",
        "live-globe/countries.js", "live-globe/data.js", "live-globe/globe.js", "live-globe/subregions.js",
        "live-globe/land.js", "live-globe/live-globe.js", "live-globe/quality.js",
        "live-globe/globe.css", "live-globe/tokens.css",
    ]

    static func path(for url: URL) -> String? {
        guard let parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              parts.scheme == scheme, parts.host == host,
              parts.user == nil, parts.password == nil, parts.port == nil,
              parts.query == nil, parts.fragment == nil,
              !parts.percentEncodedPath.contains("%"),
              parts.path.hasPrefix("/") else { return nil }
        let path = String(parts.path.dropFirst())
        return assets.contains(path) ? path : nil
    }

    static func allowsNavigation(to url: URL?, mainFrame: Bool) -> Bool {
        mainFrame && url == entry
    }

    static func mimeType(for path: String) -> String {
        if path.hasSuffix(".js") { return "text/javascript; charset=utf-8" }
        if path.hasSuffix(".css") { return "text/css; charset=utf-8" }
        return "text/html; charset=utf-8"
    }
}

enum LiveGlobeRenderingPolicy {
    static func paused(windowVisible: Bool, viewHidden: Bool, lowPower: Bool) -> Bool {
        !windowVisible || viewHidden || lowPower
    }

    struct Update: Equatable {
        let aggregate: String?
        let state: String
        let paused: Bool
        let reducedMotion: Bool
        let dark: Bool
        let language: String
        let fixture: Bool
        let evidenceAvailable: Bool
        let width: Double

        /// A paused canvas holds its last snapshot. Initial data, clearing a
        /// network, appearance/geometry changes and resuming still get through.
        func shouldSend(after previous: Self?) -> Bool {
            guard let previous else { return true }
            guard self != previous else { return false }
            let presentationChanged = paused != previous.paused || reducedMotion != previous.reducedMotion
                || dark != previous.dark || language != previous.language || fixture != previous.fixture
                || evidenceAvailable != previous.evidenceAvailable || width != previous.width
            if paused && previous.paused && !presentationChanged,
               aggregate != nil && previous.aggregate != nil { return false }
            return true
        }

        var settings: [String: Any] {
            ["paused": paused, "reducedMotion": reducedMotion, "theme": dark ? "dark" : "light",
             "lang": language, "fixture": fixture, "evidenceAvailable": evidenceAvailable]
        }
    }
}
