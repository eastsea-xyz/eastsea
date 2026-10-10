import Foundation

// Pure browser policy/data checks. The parent serializes swiftc with the
// shared team compile gate; this suite never starts an app, WebKit or a node.
var assertions = 0
var failures = 0
func check(_ condition: Bool, _ message: String) {
    assertions += 1
    if !condition { failures += 1; print("FAIL browser-plus: \(message)") }
}

func rejected(_ text: String, _ expected: BrowserInput.Failure? = nil) {
    do {
        _ = try BrowserInput.normalize(text)
        check(false, "refuse \(text)")
    } catch let failure as BrowserInput.Failure {
        check(expected == nil || failure == expected, "refuse \(text) with \(expected.map { String(describing: $0) } ?? "an input failure")")
    } catch { check(false, "unexpected error for \(text)") }
}

let normalized = try BrowserInput.normalize("  HTTPS://EXAMPLE.COM:443/a?x=1#part  ")
check(normalized.url.absoluteString == "https://example.com/a?x=1#part", "trim, lowercase host, strip default port, retain path/query/fragment")
check(try BrowserInput.normalize("example.com").url.absoluteString == "https://example.com/", "schemeless URL defaults to HTTPS")
check(try BrowserInput.normalize("//EXAMPLE.COM/path").url.absoluteString == "https://example.com/path", "protocol-relative URL defaults to HTTPS")
check(try BrowserInput.normalize("example.com:8443/path").url.port == 8443, "schemeless host and nondefault port")
check(try BrowserInput.normalize("localhost:8443").url.host == "localhost", "localhost port is an address rather than a scheme")
check(try BrowserInput.normalize("https://example.com./").url.host == "example.com", "canonical host removes DNS trailing dot")
check(try BrowserInput.normalize("https://example.com/a b").url.absoluteString.contains("a%20b"), "path spaces are encoded")
check(try BrowserInput.normalize("HTTP://example.com:80").url.absoluteString == "http://example.com/", "HTTP input preserved for the existing TLS policy to refuse")
check(BrowserCanonicalOrigin.string(for: try BrowserInput.normalize("https://[::1]:443/").url) == "https://[::1]", "IPv6 bracket form stays unambiguous")
check(try BrowserInput.normalize("https://[2001:db8::1]:8443/").url.port == 8443, "valid IPv6 with port")
check(try BrowserInput.normalize("https://bücher.example/").url.host == "xn--bcher-kva.example", "Unicode domain becomes ASCII origin")
check(try BrowserInput.normalize("https://xn--bcher-kva.example/").url.host == "xn--bcher-kva.example", "IDN ASCII spelling stays canonical")
check(try BrowserInput.normalize("https://동해.example/블록").url.host?.unicodeScalars.allSatisfy(\.isASCII) == true, "Korean domain becomes ASCII while path is retained")
check(BrowserPunycode.encode("bücher") == "bcher-kva", "RFC3492 encoding vector")
check(BrowserPunycode.decode("bcher-kva") == "bücher", "RFC3492 decoding vector")
check(BrowserPunycode.decode("a!") == nil, "invalid punycode is rejected")
check(BrowserPunycode.decode(String(repeating: "9", count: 120)) == nil, "overflowing punycode is rejected")

for text in ["https://example.com:0/", "https://example.com:65536/", "https://example.com:-1/", "https://example.com:bad/", "https://example.com:/",
             "example.com:999999999999999999999", "https:///path", "https://exa_mple.com", "https://-example.com", "https://example..com",
             "https://example.com/%Q0", "https://example.com\\evil", "https://[:::1]/", "https://256.1.1.1/"] { rejected(text) }
rejected("https://user@example.com/", .credentials)
rejected("https://user:password@example.com/", .credentials)
rejected("https://user%40trusted.example@evil.example/", .credentials)
for text in ["javascript:alert(1)", "file:///etc/passwd", "ftp://example.com/", "about:blank", "data:text/html,hello", "aether://pay?to=0x00"] { rejected(text, .unsupportedScheme) }
rejected(" \n\t ", .empty)
rejected("https://exam\nple.com")
rejected("https://exam\u{202E}ple.com")

if case .search(let url, let query) = try BrowserInput.normalize("  C++   wallet\nsearch & tokens  ") {
    let components = URLComponents(url: url, resolvingAgainstBaseURL: false)!
    check(query == "C++ wallet search & tokens", "search whitespace normalization")
    check(components.host == "duckduckgo.com", "default search engine is DuckDuckGo")
    check(components.queryItems == [URLQueryItem(name: "q", value: query)], "only query parameter is sent")
    check(url.absoluteString.contains("C%2B%2B"), "literal plus is encoded, never form-decoded as a space")
} else { check(false, "plain text routes to search") }
for engine in BrowserSearchEngine.allCases {
    let url = try BrowserInput.normalize("동해 지갑", engine: engine).url
    check(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems == [URLQueryItem(name: engine == .naver ? "query" : "q", value: "동해 지갑")], "\(engine.rawValue) encodes Unicode query without tracking parameters")
    check(url.scheme == "https", "\(engine.rawValue) search is HTTPS")
}
if case .search(_, let query) = try BrowserInput.normalize("site:example.com wallet") {
    check(query == "site:example.com wallet", "search operators are search terms rather than URL schemes")
} else { check(false, "search operator routes to search") }
if case .search(_, let query) = try BrowserInput.normalize("site:example.com") {
    check(query == "site:example.com", "single search operator remains a search query")
} else { check(false, "single search operator routes to search") }
if case .search(_, let query) = try BrowserInput.normalize("weather: tomorrow") {
    check(query == "weather: tomorrow", "colon punctuation in search text remains searchable")
} else { check(false, "colon punctuation routes to search") }
for text in ["site:example.com/path", "SITE:example.com wallet", "intitle:\"secure wallet\"", "inurl:example.com/login",
             "filetype:pdf wallet", "ext:pdf", "before:2026-10-01", "after:2026-09-01 wallet", "intext:동해",
             "allintitle:secure wallet", "related:example.com", "source:news.example", "weather: Seoul", "note: 12:34:56", "topic: 동해 지갑"] {
    if case .search(let url, let query) = try BrowserInput.normalize(text) {
        check(query == text, "operator/punctuation search preserves \(text)")
        check(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems == [URLQueryItem(name: "q", value: text)], "operator/punctuation query is encoded as one search value")
    } else { check(false, "operator/punctuation routes to search: \(text)") }
}
for text in ["javascript: alert(1)", "data: text/html,hello", "file: /etc/passwd", "about: blank", "aether: pay", "eastsea: pay",
             "mailto: someone@example.com", "tel: +821012345678", "tel:+821012345678", "ftp: example.com",
             "custom://example.com", "site://example.com", "filetype://pdf"] { rejected(text, .unsupportedScheme) }
for text in ["example.com:bad", "example.com: nonsense", "example.com: 443", "localhost: +443", "[::1]: bad",
             "user:password@example.com", "user: password@example.com", "https://user:password@example.com/"] { rejected(text) }
if case .sea(let request) = try BrowserInput.normalize("SEA://Alice/profile?view=1") {
    check(request.name == "alice", "sea name is canonical")
    check(request.url.absoluteString == "sea://alice/profile?view=1", "sea path and query stay at the resolver seam")
} else { check(false, "sea input reaches resolver") }
if case .sea(let request) = try BrowserInput.normalize("sea://동해") {
    check(request.name == "동해", "sea request retains Unicode name through punycode decode")
} else { check(false, "Unicode sea name reaches resolver") }
rejected("sea://", .invalidSeaName)
rejected("sea://alice:443", .invalidSeaName)
rejected("sea://user@alice", .credentials)
let resolver: any SeaNameResolving = UnresolvedSeaNameResolver()
_ = resolver
check(BrowserCanonicalOrigin.string(for: normalized.url) == "https://example.com", "origin contains no path/query/fragment")
check(BrowserCanonicalOrigin.string(for: URL(string: "https://user@example.com")!) == nil, "deceptive credentials never acquire an origin")
check(BrowserCanonicalOrigin.string(scheme: "HTTPS", host: "EXAMPLE.COM.", port: 443) == "https://example.com", "WebKit origin canonicalizes scheme/host/default port")
check(BrowserCanonicalOrigin.string(scheme: "https", host: "::1", port: 0) == "https://[::1]", "raw WebKit IPv6 matches URL IPv6")
check(BrowserCanonicalOrigin.string(scheme: "https", host: "[::1]", port: 8443) == "https://[::1]:8443", "bracketed IPv6 has one pair of brackets")
check(BrowserCanonicalOrigin.string(scheme: "https", host: "bücher.example", port: 443) == "https://xn--bcher-kva.example", "WebKit Unicode origin matches URL ASCII origin")
check(BrowserCanonicalOrigin.string(scheme: "https", host: "example.com", port: -1) == nil, "invalid WebKit port refused")
check(BrowserCanonicalOrigin.string(scheme: "https", host: "example.com", port: 65536) == nil, "out-of-range WebKit port refused")
check(BrowserCanonicalOrigin.string(scheme: "javascript", host: "example.com", port: 0) == nil, "unsupported WebKit scheme refused")
check(BrowserCanonicalOrigin.string(scheme: "https", host: "example.com@evil.com", port: 0) == nil, "malformed WebKit host refused")
check(BrowserCanonicalOrigin.isSecure(URL(string: "https://example.com")!), "HTTPS origin gets secure indicator")
check(!BrowserCanonicalOrigin.isSecure(URL(string: "http://example.com")!), "HTTP never gets secure indicator")
check(!BrowserCanonicalOrigin.isSecure(URL(string: "sea://alice")!), "unverified sea name never gets secure indicator")
check(BrowserCanonicalOrigin.isSecure(URL(string: "sea://alice")!, verifiedSea: true), "verified sea content can get secure indicator")
check(!BrowserCanonicalOrigin.isSecure(URL(string: "eastsea-page://explorer/index.html")!), "bundled origin is not mislabeled as TLS")
for scheme in ["sea", "eastsea", "aether"] {
    let action = URL(string: "\(scheme)://pay?to=0x00&amount=1")!
    check(!BrowserSeaActionPolicy.allows(action, hasUserGesture: false), "automatic \(scheme) wallet action blocked")
    check(BrowserSeaActionPolicy.allows(action, hasUserGesture: true), "user gesture permits \(scheme) handoff to confirmation")
}
check(BrowserSeaActionPolicy.allows(URL(string: "https://example.com")!, hasUserGesture: false), "normal web navigation remains available")

let now = Date(timeIntervalSince1970: 1_800_000_000)
let site = URL(string: "https://example.com/a")!
let other = URL(string: "https://other.example/b")!
let privateSite = URL(string: "https://private-only.example/secret")!
var profile = BrowserProfile()
profile.recordVisit(url: site, title: "First", visitedAt: now.addingTimeInterval(-9 * 24 * 3600))
profile.recordVisit(url: other, title: "Older", visitedAt: now.addingTimeInterval(-2 * 24 * 3600))
profile.recordVisit(url: site, title: "Recent", visitedAt: now.addingTimeInterval(-30 * 60))
profile.recordVisit(url: privateSite, title: "Private secret", isPrivate: true, visitedAt: now)
check(profile.history.count == 3, "private visit is not recorded")
check(profile.searchHistory("RECENT").count == 1, "history searches titles case-insensitively")
check(profile.searchHistory("other.example").count == 1, "history searches URLs")
check(profile.searchHistory("  ").count == 3, "empty history search returns all visits")
profile.updateVisitTitle(url: site, title: "Loaded title")
check(profile.history.count == 3 && profile.history[0].title == "Loaded title", "page title update changes the newest matching entry without a visit")
check(profile.history.last?.title == "First", "page title update leaves old matching visits unchanged")
profile.updateVisitTitle(url: site, title: "Private changed title", isPrivate: true)
check(profile.history[0].title == "Loaded title", "private title update cannot alter history")
var hour = profile
hour.clearHistory(.lastHour, now: now)
check(hour.history.count == 2 && hour.history.allSatisfy { $0.visitedAt < now.addingTimeInterval(-3600) }, "clear last hour includes cutoff and keeps older visits")
var week = profile
week.clearHistory(.lastWeek, now: now)
check(week.history.count == 1 && week.history[0].title == "First", "clear seven days keeps earlier history")
var calendar = Calendar(identifier: .gregorian)
calendar.timeZone = TimeZone(secondsFromGMT: 0)!
var today = BrowserProfile()
today.recordVisit(url: site, title: "Yesterday", visitedAt: calendar.startOfDay(for: now).addingTimeInterval(-1))
today.recordVisit(url: other, title: "Today", visitedAt: calendar.startOfDay(for: now))
today.clearHistory(.today, now: now, calendar: calendar)
check(today.history.count == 1 && today.history[0].title == "Yesterday", "today range begins at calendar midnight")
today.clearHistory(.all, now: now)
check(today.history.isEmpty, "clear all removes all history")

let firstBookmark = profile.addBookmark(title: "Example", url: site)
let secondBookmark = profile.addBookmark(title: "Other", url: other)
let duplicate = profile.addBookmark(title: "Renamed", url: URL(string: "https://EXAMPLE.COM:443/a")!)
check(profile.bookmarks.count == 2 && duplicate.id == firstBookmark.id, "canonical duplicate bookmark updates the existing favorite")
check(profile.reorderBookmarks([secondBookmark.id, firstBookmark.id]), "valid bookmark permutation accepted")
check(profile.bookmarks.map(\.id) == [secondBookmark.id, firstBookmark.id], "favorite order applied")
check(!profile.reorderBookmarks([secondBookmark.id, secondBookmark.id]), "duplicate reorder rejected")
check(!profile.reorderBookmarks([firstBookmark.id]), "incomplete reorder rejected")
check(profile.bookmarks.map(\.id) == [secondBookmark.id, firstBookmark.id], "invalid reorder preserves favorites")
profile.removeBookmark(id: secondBookmark.id)
check(profile.bookmarks.map(\.id) == [firstBookmark.id], "remove bookmark by identity")

profile.setZoom(1.5, for: site)
check(profile.zoomFactor(for: URL(string: "https://EXAMPLE.COM:443/other")!) == 1.5, "zoom remembered per canonical origin")
check(profile.zoomFactor(for: URL(string: "https://example.com:8443")!) == 1, "zoom isolates nondefault ports")
check(profile.zoomFactor(for: URL(string: "http://example.com")!) == 1, "zoom isolates schemes")
profile.setZoom(9, for: site)
check(profile.zoomFactor(for: site) == 3, "zoom upper bound")
profile.setZoom(0.1, for: site)
check(profile.zoomFactor(for: site) == 0.5, "zoom lower bound")
profile.setZoom(2, for: privateSite, isPrivate: true)
check(profile.zoomFactor(for: privateSite) == 1 && profile.zoom["https://private-only.example"] == nil, "private zoom never persists")
profile.setZoom(.infinity, for: site)
check(profile.zoomFactor(for: site) == 1 && profile.zoom["https://example.com"] == nil, "nonfinite zoom resets safely")
profile.setZoom(1.25, for: site)

let regularTab = BrowserTabSnapshot(title: "Example", url: site)
let secondTab = BrowserTabSnapshot(title: "Other", url: other)
let privateTab = BrowserTabSnapshot(title: "Private secret", url: privateSite, isPrivate: true)
profile.saveTabs([regularTab, privateTab, secondTab], activeID: privateTab.id)
check(profile.tabs.map(\.id) == [regularTab.id, secondTab.id], "private tab excluded from snapshots")
check(profile.activeTabID == regularTab.id, "private selected tab falls back to regular tab on relaunch")
profile.searchEngine = .bing
let encoded = try JSONEncoder().encode(profile)
let reopened = try JSONDecoder().decode(BrowserProfile.self, from: encoded)
check(reopened == profile, "bookmarks/history/tabs/zoom/search engine survive profile roundtrip")
var unsafeProfile = profile
unsafeProfile.tabs.append(privateTab)
unsafeProfile.activeTabID = privateTab.id
let safeBytes = try JSONEncoder().encode(unsafeProfile)
check(!String(decoding: safeBytes, as: UTF8.self).contains("private-only") && !String(decoding: safeBytes, as: UTF8.self).contains("Private secret"), "profile encoder removes private URL/title even if caller bypasses saveTabs")
do {
    _ = try JSONEncoder().encode(privateTab)
    check(false, "standalone private snapshot refuses serialization")
} catch BrowserTabSnapshot.PersistenceFailure.privateTab { check(true, "standalone private snapshot refuses serialization") }

final class MemoryDefaults: UserDefaults {
    var values: [String: Any] = [:]
    init() { super.init(suiteName: "browser-plus-unused-memory")! }
    override func object(forKey key: String) -> Any? { values[key] }
    override func data(forKey key: String) -> Data? { values[key] as? Data }
    override func set(_ value: Any?, forKey key: String) { values[key] = value }
    override func removeObject(forKey key: String) { values.removeValue(forKey: key) }
}
let defaults = MemoryDefaults()
let accountA = "0x00000000000000000000000000000000000000aa"
let accountB = "0x00000000000000000000000000000000000000bb"
let storeA = BrowserProfileStore(accountID: 1, address: accountA, defaults: defaults)
let aliasA = BrowserProfileStore(accountID: 1, address: accountA.uppercased(), defaults: defaults)
let storeB = BrowserProfileStore(accountID: 2, address: accountB, defaults: defaults)
let reusedID = BrowserProfileStore(accountID: 1, address: accountB, defaults: defaults)
try storeA.save(profile)
check(aliasA.load() == profile, "account address casing shares the same browser profile")
check(storeB.load() == BrowserProfile(), "new account cannot read another account's browser profile")
check(reusedID.load() == BrowserProfile(), "same handle id with different address stays isolated")
var otherProfile = BrowserProfile()
otherProfile.searchEngine = .google
otherProfile.addBookmark(title: "Other account", url: other)
try storeB.save(otherProfile)
check(storeA.load() == profile && storeB.load() == otherProfile, "independent account profiles stay durable")
let secondWindow = BrowserProfileStore(accountID: 1, address: accountA, defaults: defaults)
var secondWindowProfile = secondWindow.load()
secondWindowProfile.addBookmark(title: "Second window", url: other)
try secondWindow.save(secondWindowProfile)
var firstWindowReloaded = storeA.load()
firstWindowReloaded.setZoom(2, for: other)
try storeA.save(firstWindowReloaded)
check(secondWindow.load().bookmarks.count == 2 && secondWindow.load().zoomFactor(for: other) == 2, "reloading shared profile before a window mutation retains other window's data")
let ownerless = BrowserProfileStore(accountID: 0, address: "", defaults: defaults)
check(ownerless.load() == BrowserProfile(), "missing account cannot read a global profile")
do { try ownerless.save(profile); check(false, "ownerless save refused") }
catch BrowserProfileStore.Failure.noAccount { check(true, "ownerless save refused") }
defaults.set(Data("broken".utf8), forKey: storeA.key)
check(storeA.load() == BrowserProfile(), "corrupt browser metadata falls back safely without touching keys")
check(storeB.load() == otherProfile, "corrupt account profile leaves other account unaffected")

let resourceRoot = URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent("apps/wallet/Resources")
let confusables = try BrowserConfusables(
    confusables: String(contentsOf: resourceRoot.appendingPathComponent("BrowserConfusables-15.1.txt"), encoding: .utf8),
    defaultIgnorables: String(contentsOf: resourceRoot.appendingPathComponent("BrowserDefaultIgnorables-15.1.txt"), encoding: .utf8))
check(confusables.skeleton("paypal") == confusables.skeleton("pаypаl"), "UTS39 mixed-script confusable vector")
check(confusables.skeleton("scope") == confusables.skeleton("ѕсоре"), "UTS39 whole-script Cyrillic confusable vector")
check(confusables.skeleton("m") == confusables.skeleton("rn"), "UTS39 multi-character prototype is retained")
check(confusables.skeleton("é") == confusables.skeleton("e\u{301}"), "UTS39 canonical decomposition")
check(confusables.skeleton("pay\u{200D}pal") == confusables.skeleton("paypal"), "UTS39 removes bundled default-ignorable codepoint")
check(confusables.skeleton("paypal") != confusables.skeleton("paypa1x"), "unrelated names have distinct skeletons")
let paypal = BrowserBookmark(title: "PayPal favorite", url: URL(string: "https://paypal.com")!)
let spoof = try BrowserInput.normalize("https://pаypal.com/login").url
check(confusables.warning(for: spoof, bookmarks: [paypal], apps: [])?.protectedName == "PayPal favorite", "punycode lookalike warns against favorite")
let spoofSubdomain = try BrowserInput.normalize("https://www.pаypal.com/").url
check(confusables.warning(for: spoofSubdomain, bookmarks: [paypal], apps: []) != nil, "confusable subdomain warns against favorite")
check(confusables.warning(for: paypal.url, bookmarks: [paypal], apps: []) == nil, "exact favorite never gets a warning")
check(confusables.warning(for: URL(string: "https://app.paypal.com")!, bookmarks: [paypal], apps: []) == nil, "legitimate favorite subdomain never gets a warning")
check(confusables.warning(for: URL(string: "https://other.example")!, bookmarks: [paypal], apps: []) == nil, "unrelated origin has no favorite warning")
let builtins = BuiltinBrowserApps.apps
check(builtins.count == 1 && builtins[0].id == "explorer", "only the real bundled explorer is built in")
check(builtins.allSatisfy { $0.url.scheme == "eastsea-page" }, "built-ins cannot pretend to be registry names")
let realApp = BrowserToolboxApp(id: "dex", title: "Registered DEX", detail: "", url: URL(string: "sea://dex.sea")!)
let toolboxSpoof = try BrowserInput.normalize("sea://dеx.sea").url
check(confusables.warning(for: toolboxSpoof, bookmarks: [], apps: [realApp])?.protectedName == realApp.title, "real app names participate in skeleton warning")
check(confusables.warning(for: realApp.url, bookmarks: [], apps: [realApp]) == nil, "exact app name does not warn")

let token = "0x00000000000000000000000000000000000000aa"
let spender = "0x00000000000000000000000000000000000000bb"
let approvePrefix = "0x095ea7b3" + String(repeating: "0", count: 24) + String(spender.dropFirst(2))
let approveOne = approvePrefix + String(repeating: "0", count: 63) + "1"
check(BrowserTokenAllowance.parse(token: token, data: approveOne)?.spender == spender, "ERC20 requested spender decoded")
check(BrowserTokenAllowance.parse(token: token, data: approveOne)?.amount == "1", "ERC20 finite base-unit amount decoded")
let unlimited = BrowserTokenAllowance.parse(token: token, data: approvePrefix + String(repeating: "f", count: 64))
check(unlimited?.isUnlimited == true && unlimited?.amount == "115792089237316195423570985008687907853269984665640564039457584007913129639935", "ERC20 uint256 unlimited request described without integer overflow")
check(BrowserTokenAllowance.parse(token: token, data: "0xa9059cbb" + String(approveOne.dropFirst(10))) == nil, "transfer never described as an allowance")
check(BrowserTokenAllowance.parse(token: token, data: approveOne + "00") == nil, "noncanonical ABI approval refused")
check(BrowserTokenAllowance.parse(token: "0x00", data: approveOne) == nil, "malformed token address refused")

if failures > 0 { print("browser-plus: \(failures)/\(assertions) checks failed"); exit(1) }
print("browser-plus OK (\(assertions) checks)")
