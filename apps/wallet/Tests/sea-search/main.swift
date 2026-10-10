import Foundation

// Pure routing/privacy checks. This executable starts no wallet, WebKit or node.
var assertions = 0
var failures = 0
func check(_ condition: Bool, _ message: String) {
    assertions += 1
    if !condition { failures += 1; print("FAIL sea-search: \(message)") }
}

let address = "0xCe4F7dCEB0b51b83C7474713281eF2049f70D5CD"
let hash = "0x" + String(repeating: "ab", count: 32)
let goldenQueries: [(String, SeaSearch.Query)] = [
    ("", .empty), (" \n\t ", .empty), ("\u{FEFF}sea://search\u{FEFF}", .home),
    ("sea://search", .home), ("SEA://SEARCH/", .home),
    (" 7 ", .chain(.block(7))), ("00042", .chain(.block(42))), ("0", .chain(.block(0))),
    ("9007199254740991", .chain(.block(9_007_199_254_740_991))),
    ("9007199254740992", .invalid), ("18446744073709551616", .invalid),
    (address, .chain(.address(address.lowercased()))),
    (String(address.dropFirst(2)), .chain(.address(address.lowercased()))),
    (address.uppercased(), .chain(.address(address.lowercased()))),
    (hash, .chain(.transaction(hash))), (hash.uppercased(), .chain(.transaction(hash))),
    (String(hash.dropFirst(2)), .chain(.transaction(hash))),
    ("har", .app("har")), ("@harbor", .app("harbor")), ("@  EastSea  DEX ", .app("EastSea DEX")),
    ("@", .empty), ("0x1234", .app("0x1234")), ("바다", .app("바다")),
    ("  EastSea   DEX  ", .web("EastSea DEX")),
    ("동해 지갑", .web("동해 지갑")), ("東海 ウォレット", .web("東海 ウォレット")),
    ("东海 钱包", .web("东海 钱包")), ("東海 錢包", .web("東海 錢包")),
    ("site:example.com wallet", .web("site:example.com wallet")),
    ("C++ wallet & tokens", .web("C++ wallet & tokens")),
    ("example.com", .url(URL(string: "https://example.com/")!)),
    ("//EXAMPLE.COM/path", .url(URL(string: "https://example.com/path")!)),
    ("HTTPS://EXAMPLE.COM:443/a?x=1#part", .url(URL(string: "https://example.com/a?x=1#part")!)),
    ("example.com:8443/path", .url(URL(string: "https://example.com:8443/path")!)),
    ("https://alice.sea/path", .url(URL(string: "https://alice.sea/path")!)),
    ("http://example.com", .url(URL(string: "http://example.com/")!)),
    ("eastsea-page://explorer/index.html", .url(URL(string: "eastsea-page://explorer/index.html")!)),
    ("sea://pay?to=0x1234&amount=10", .action(host: "pay", raw: "sea://pay?to=0x1234&amount=10")),
    ("aether:pay?to=0x1234", .action(host: "pay", raw: "aether:pay?to=0x1234"))
]
for (input, expected) in goldenQueries {
    check(SeaSearch.classify(input) == expected, "golden classification: \(input)")
}

let name = SeaURL.NameLink(name: "harbor.sea", canonicalURL: "sea://harbor.sea/a%2Fb?q=%2f&x=one+two",
                          path: "/a%2Fb", query: "q=%2f&x=one+two", isLegacy: false, registryName: "harbor.sea")
for input in ["sea://harbor/a%2Fb?q=%2f&x=one+two", "harbor.sea/a%2Fb?q=%2f&x=one+two",
              "harbor/a%2Fb?q=%2f&x=one+two", "eastsea://harbor/a%2Fb?q=%2f&x=one+two"] {
    check(SeaSearch.classify(input) == .name(name), "name path/query bytes preserved: \(input)")
}
check(SeaSearch.classify("harbor").nameLink?.canonicalURL == "sea://harbor.sea/", "bare app text offers a verifiable exact name")
check(SeaSearch.classify("@harbor.sea").nameLink?.name == "harbor.sea", "explicit app query may also resolve a canonical name")
check(SeaSearch.classify("sea://search.sea").nameLink?.name == "search.sea", "search.sea is a registry name, separate from native home")
check(SeaSearch.classify("sea://search/path").nameLink?.path == "/path", "a path on the short search name is a registry lookup")
check(SeaSearch.classify("harbor.aeth", chainID: 7780).nameLink?.registryName == "harbor.aeth", "legacy lookup stays on chain 7780")
check(SeaSearch.classify("harbor.aeth", chainID: 1) == .invalid, "legacy name cannot enter another chain")

// Independent RFC 4648 vectors for the AppRegistry's base32(32-byte appID)
// authority. The final symbol can only be a or q; padding is never accepted.
let appVectors: [(String, String)] = [
    (String(repeating: "00", count: 32), String(repeating: "a", count: 52)),
    (String(repeating: "ff", count: 32), String(repeating: "7", count: 51) + "q"),
    ("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
     "aaaqeayeaudaocajbifqydiob4ibceqtcqkrmfyydenbwha5dypq"),
    (String(repeating: "ab", count: 32), "vov2xk5lvov2xk5lvov2xk5lvov2xk5lvov2xk5lvov2xk5lvovq")
]
for (hex, key) in appVectors {
    let raw = "sea://" + key + "/"
    guard let link = SeaAppLink.parse(raw) else { fatalError("valid app link refused: \(raw)") }
    check(link.appID == "0x" + hex && link.appKey == key && link.canonicalURL == raw,
          "canonical base32 app ID vector: \(hex)")
    check(link.path == "/" && link.query == nil, "root app link")
    let query = SeaSearch.classify(raw)
    check(query == .registryApp(link), "registry app IDs route before name parsing")
    check(query.localAppQuery == nil && query.nameLink == nil && query.webQuery == nil && query.directURL == nil,
          "registry app URL cannot become a name search or web navigation")
    check(SeaSearch.classify("sea://" + key + ".sea").nameLink?.name == key + ".sea",
          "explicit .sea remains a name, even when its label resembles an app ID")
    check(SeaSearch.classify(key) == .app(key), "bare app-key text remains native search until an explicit sea:// URL")
}
let appKey = appVectors[2].1
let appLink = SeaAppLink.parse("sea://\(appKey)/pages/My_page-2.html?q=%2f&x=one+two&x=three")!
check(appLink.path == "/pages/My_page-2.html" && appLink.query == "q=%2f&x=one+two&x=three", "app path and query bytes preserved")
check(appLink.canonicalURL == "sea://\(appKey)/pages/My_page-2.html?q=%2f&x=one+two&x=three", "canonical app URL retains encoded query")
check(SeaAppLink.parse("sea://\(appKey)")?.canonicalURL == "sea://\(appKey)/", "empty app path becomes root")
check(SeaAppLink.parse("SEA://\(appKey)/")?.canonicalURL == "sea://\(appKey)/", "scheme case does not change the strict app key")
check(SeaAppLink.parse("sea://\(appKey)?")?.query == "", "empty app query is preserved")
check(SeaAppLink.parse("sea://\(appKey)/" + String(repeating: "a", count: 200)) != nil, "app path at bundle byte bound")
let emptyValueLink = SeaAppLink.parse("sea://\(appKey)/index.html?bad=")!
check(SeaSearch.classify(" \nsea://\(appKey)/index.html?bad=\n") == .registryApp(emptyValueLink),
      "browser trims outer whitespace while preserving an empty query value")
for invalidKey in [String(appKey.dropLast()), appKey + "a", appKey + "====", appKey.uppercased(),
                   String(appKey.dropLast()) + "b", String(appKey.dropLast()) + "r",
                   String(appKey.dropLast()) + "8", String(appKey.dropLast()) + "0"] {
    let raw = "sea://" + invalidKey + "/"
    check(SeaAppLink.parse(raw) == nil, "invalid app alphabet/length/padding rejected: \(invalidKey)")
    if invalidKey.utf8.count == 52 {
        check(SeaSearch.classify(raw) == .invalid, "a malformed app-key authority cannot fall back to name resolution")
    }
}
for finalSymbol in "abcdefghijklmnopqrstuvwxyz234567" where finalSymbol != "a" && finalSymbol != "q" {
    let raw = "sea://" + String(appKey.dropLast()) + String(finalSymbol) + "/"
    check(SeaAppLink.parse(raw) == nil && SeaSearch.classify(raw) == .invalid, "all nonzero final base32 padding bits are refused")
}
for raw in ["sea://user@\(appKey)/", "sea://user:password@\(appKey)/", "sea://\(appKey):443/",
            "sea://\(appKey)/#fragment", "sea://\(appKey)./", "sea://\(appKey).sea/",
            "sea://\(appKey)/../index.html", "sea://\(appKey)/a/./index.html", "sea://\(appKey)/a//index.html",
            "sea://\(appKey)//index.html", "sea://\(appKey)/pages/", "sea://\(appKey)/a%2fb.html",
            "sea://\(appKey)/%2e%2e/index.html", "sea://\(appKey)/a%5cb.html", "sea://\(appKey)/바다.html",
            "sea://\(appKey)/index.html?bad=%ZZ", "sea://\(appKey)/index.html?bad=%", "sea://\(appKey)/index.html?bad=\n",
            "sea://\(appKey)/index.html?bad=one\ntwo",
            "sea://\(appKey)\\index.html", "sea://\(appKey)/" + String(repeating: "a", count: 201),
            "https://\(appKey)/", "eastsea://\(appKey)/", " sea://\(appKey)/ "] {
    check(SeaAppLink.parse(raw) == nil, "unsafe/non-app link refused: \(raw.debugDescription)")
    // The parser rejects raw outer whitespace; browser input trims it first.
    if SeaAppLink.isAppCandidate(raw), raw == raw.trimmingCharacters(in: .whitespacesAndNewlines) {
        check(SeaSearch.classify(raw) == .invalid, "unsafe app URLs never fall back into name resolution: \(raw.debugDescription)")
    }
}

for input in ["javascript:alert(1)", "javascript: alert(1)", "file:///etc/passwd", "data:text/html,hello", "ftp://example.com",
              "https://user:password@example.com", "https://example.com:0/", "https://example.com/%Q0",
              "sea://user@harbor.sea", "sea://harbor.sea:443", "sea://harbor.com", "sea://Harbor.sea",
              "sea://바다.sea", "sea://harbor.sea/#pay", "sea://a%2fb.sea", "sea:\\harbor"] {
    let query = SeaSearch.classify(input)
    check(query == .invalid && query.directURL == nil && query.webQuery == nil, "malformed/unsupported input never becomes a web option: \(input)")
}

check(SeaSearch.homeURL.absoluteString == "sea://search", "one native home address for Home and new tabs")
for value in ["sea://search", "SEA://SEARCH/"] {
    check(SeaSearch.isHome(URL(string: value)!), "native home route: \(value)")
}
for value in ["sea://search.sea", "sea://search/path", "sea://search?query=secret", "sea://search#fragment",
              "sea://user@search", "sea://search:443", "https://search", "eastsea://search"] {
    check(!SeaSearch.isHome(URL(string: value)!), "home detection cannot hide a different navigation: \(value)")
}
check(SeaSearch.Query.home.directURL == nil && SeaSearch.Query.home.localAppQuery == nil, "home stays native without a registry or web request")

let expectedRoutes: [(SeaSearch.ChainLookup, String)] = [
    (.address(address.lowercased()), "eastsea-page://explorer/index.html#/account/" + address.lowercased()),
    (.transaction(hash), "eastsea-page://explorer/index.html#/tx/" + hash),
    (.block(42), "eastsea-page://explorer/index.html#/block/42")
]
for (lookup, expected) in expectedRoutes {
    check(lookup.explorerURL.absoluteString == expected, "bundled explorer route parity: \(expected)")
    check(SeaSearch.Query.chain(lookup).directURL == lookup.explorerURL, "chain lookup opens the bundled page")
}

// The sea:// URL fixtures are also run by the extension and explorer. Native
// search must pass the shared grammar through unchanged for explicit names and
// wallet actions, including refusals; a bare app query may offer that same name.
let fixture = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: "tests/fixtures/sea-urls.json"))) as! [String: Any]
let sharedCases = fixture["cases"] as! [[String: Any]]
var sharedCount = 0
for row in sharedCases {
    let input = row["input"] as! String
    let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
    // Parser-only fixtures reject outer whitespace; browser input explicitly
    // trims it before parsing, so those cases belong to the sea-url suite.
    if row["operation"] as? String == "parse", input != trimmed { continue }
    let lower = trimmed.lowercased()
    let authority = lower.prefix { !"/?#".contains($0) }
    let explicitName = ["sea:", "eastsea:", "aether:"].contains(where: lower.hasPrefix)
        || authority.hasSuffix(".sea") || authority.hasSuffix(".aeth")
    let expected = row["expected"] as! [String: Any]
    guard explicitName else { continue }
    sharedCount += 1
    let actual = SeaSearch.classify(input, chainID: (row["chainID"] as! NSNumber).uint64Value)
    if expected["error"] != nil {
        check(actual == .invalid, "shared refusal: \(row["id"]!)")
    } else if expected["kind"] as? String == "action" {
        check(actual == .action(host: expected["host"] as! String, raw: expected["raw"] as! String), "shared consent action: \(row["id"]!)")
    } else if expected["kind"] as? String == "name" {
        let link = actual.nameLink
        check(link?.name == expected["name"] as? String
            && link?.canonicalURL == expected["canonicalURL"] as? String
            && link?.path == expected["path"] as? String
            && link?.query == expected["query"] as? String
            && link?.isLegacy == expected["isLegacy"] as? Bool
            && link?.registryName == expected["registryName"] as? String, "shared name parity: \(row["id"]!)")
    }
}
check(sharedCount > 30, "shared extension/explorer name cases were exercised")

// Catch an accidental eager URLSession request while typing/classifying. Any
// transport this pure model attempts is intercepted before reaching a server.
final class SearchNetworkTrap: URLProtocol, @unchecked Sendable {
    private static let lock = NSLock()
    private static var count = 0
    static var requestCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return count
    }
    override class func canInit(with request: URLRequest) -> Bool {
        lock.lock()
        count += 1
        lock.unlock()
        return true
    }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() { client?.urlProtocol(self, didFailWithError: URLError(.cancelled)) }
    override func stopLoading() {}
}
check(URLProtocol.registerClass(SearchNetworkTrap.self), "install the request trap")
for input in ["sea://harbor.sea", "sea://\(appKey)/", "harbor", "@harbor", "private words & secrets", "바다 지갑", address, hash, "42"] {
    let query = SeaSearch.classify(input)
    if query.webQuery != nil {
        check(query.directURL == nil && query.localAppQuery != nil, "plain text searches the node and offers web without navigating: \(input)")
    }
}
RunLoop.current.run(until: Date().addingTimeInterval(0.05))
check(SearchNetworkTrap.requestCount == 0, "no network request before the person chooses web search")

let searchText = "바다 C++ a/b?x=1 &affiliate=evil #frag"
let expectedEngines: [(BrowserSearchEngine, String, String, String)] = [
    (.duckDuckGo, "duckduckgo.com", "/", "q"), (.google, "www.google.com", "/search", "q"),
    (.bing, "www.bing.com", "/search", "q"), (.naver, "search.naver.com", "/search.naver", "query"),
    (.brave, "search.brave.com", "/search", "q")
]
check(BrowserSearchEngine.allCases == expectedEngines.map(\.0), "Settings offers all five engines in stable order")
for (engine, host, path, parameter) in expectedEngines {
    let url = SeaSearch.webSearchURL(for: .web(searchText), engine: engine)!
    let components = URLComponents(url: url, resolvingAgainstBaseURL: false)!
    check(components.scheme == "https" && components.host == host && components.path == path, "correct \(engine.rawValue) endpoint")
    check(components.queryItems == [URLQueryItem(name: parameter, value: searchText)], "\(engine.rawValue) sends only one query parameter")
    check(components.user == nil && components.password == nil && components.fragment == nil, "query text cannot escape into URL authority/fragment")
    check(url.absoluteString.contains("C%2B%2B") && url.absoluteString.contains("%26affiliate%3Devil"), "query operators and injected parameter text are encoded")
    check(try JSONDecoder().decode(BrowserSearchEngine.self, from: JSONEncoder().encode(engine)) == engine, "engine preference remains Codable")
}
check(SeaSearch.webSearchURL(for: "private words")?.host == "duckduckgo.com", "DuckDuckGo is the explicit web choice default")
for query in [SeaSearch.Query.empty, .invalid, .home, .name(name), .registryApp(appLink), .chain(.block(1)), .url(URL(string: "https://example.com/")!),
              .action(host: "pay", raw: "sea://pay?to=0x1234")] {
    check(SeaSearch.webSearchURL(for: query) == nil, "a non-text route cannot silently become web search")
}
check(SearchNetworkTrap.requestCount == 0, "choosing/encoding a web URL still performs no transport in the model")
URLProtocol.unregisterClass(SearchNetworkTrap.self)

if failures > 0 { print("sea-search: \(failures)/\(assertions) checks failed"); exit(1) }
print("sea-search OK (\(assertions) checks; \(sharedCount) shared name/action cases)")
