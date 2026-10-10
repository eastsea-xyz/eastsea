// Native search contract and address-bar routing, without an app or a node.
// scripts/test-swift-pure.sh compiles AppSearch.swift and its pure URL parsers.
import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

func webURL(_ text: String) -> URL {
    guard case .web(let url) = AppSearchInput.destination(for: text) else {
        check(false, "web route for \(text)")
        fatalError()
    }
    return url
}

func refusedByWebPolicy(_ text: String) {
    if case .blocked = BrowserOriginPolicy.classify(webURL(text)) { return }
    check(false, "the existing web policy must refuse \(text)")
}

// Search terms, names and subdomains stay native. Unicode chain names keep
// their spelling rather than becoming DNS/punycode labels.
check(AppSearchInput.destination(for: " \n ") == .empty, "empty bar")
check(AppSearchInput.destination(for: " tidepay ") == .search("tidepay"), "bare app name")
check(AppSearchInput.destination(for: "@tidepay") == .search("tidepay"), "name mention")
check(AppSearchInput.destination(for: "@alice.example") == .search("alice.example"), "an explicit name mention stays native")
check(AppSearchInput.destination(for: "tide games") == .search("tide games"), "phrase search")
check(AppSearchInput.destination(for: "games v1.2") == .search("games v1.2"), "a period in a phrase is not a web host")
check(AppSearchInput.destination(for: "alice.sea") == .search("alice.sea"), ".sea name")
check(AppSearchInput.destination(for: "@alice.sea") == .search("alice.sea"), ".sea name mention")
check(AppSearchInput.destination(for: "shop.alice.SEA") == .search("shop.alice.SEA"), ".sea subdomain")
check(AppSearchInput.destination(for: "shop.alice.sea/catalog") == .search("shop.alice.sea"), "name path")
check(AppSearchInput.destination(for: "SEA://shop.alice.sea/path?q=1#x") == .search("shop.alice.sea"), "sea link searches its name")
check(AppSearchInput.destination(for: "sea://바다.sea") == .search("바다.sea"), "Unicode name stays Unicode")
check(AppSearchInput.destination(for: "sea://%EB%B0%94%EB%8B%A4.sea") == .search("바다.sea"), "encoded Unicode spelling")

// HTTPS remains explicit navigation, including a .sea-looking host. No new
// search path bypasses the existing warning and forbidden-scheme policy.
check(webURL("https://alice.sea/path").absoluteString == "https://alice.sea/path", "explicit https takes precedence")
check(webURL("HTTPS://example.test/path").scheme?.lowercased() == "https", "scheme case")
check(webURL("example.test/path").absoluteString == "https://example.test/path", "scheme-less web host")
check(webURL("example.test:8443/path").port == 8443, "scheme-less host port")
check(webURL("localhost:443/path").host == "localhost", "localhost with a port")
for input in ["http://example.test", "file:///etc/passwd", "javascript:alert(1)", "javascript:evil.test:123", "data:text/html,hello", "about:blank", "ftp://example.test"] {
    refusedByWebPolicy(input)
}
for input in ["sea:", "sea:alice.sea", "sea:/alice.sea", "sea://", "sea://alice.sea@evil.sea", "sea://alice.sea:443", "sea://a%2fb.sea", "sea://a%0ab.sea", "sea://a%3fb.sea"] {
    check(AppSearchInput.destination(for: input) == .invalid, "ambiguous sea authority refused: \(input)")
}

// Request bounds use UTF-8 bytes, and never truncate a name to another name.
let defaultRequest = try AppSearchRequest(query: " tidepay ")
check(defaultRequest.query == "tidepay" && defaultRequest.limit == 20, "trimmed request and default limit")
check(AppSearchRequest.method == "aether_search", "node method")
check(defaultRequest.params[0] as? String == "tidepay" && defaultRequest.params[1] as? Int == 20, "RPC parameter order")
check(try AppSearchRequest(query: "x", limit: 1000).limit == 50, "result cap")
check(try AppSearchRequest(query: "x", limit: 0).limit == 1, "minimum limit")
let exactly256Bytes = String(repeating: "海", count: 85) + "a"
check(try AppSearchRequest(query: exactly256Bytes).query == exactly256Bytes, "multibyte name at byte bound")
for query in [String(repeating: "x", count: 257), String(repeating: "海", count: 86)] {
    do {
        _ = try AppSearchRequest(query: query)
        check(false, "overlong query must fail")
    } catch let error as AppSearchFailure { check(error == .queryTooLong, "byte-bound failure") }
}

func row(name: String, verified: Bool, usage: UInt64, lookalike: Any = NSNull()) -> [String: Any] {
    ["name": name, "title": "App \(name)", "description": "Publisher supplied text",
     "category": "games", "publisher": "0x1111111111111111111111111111111111111111",
     "url": "sea://\(name)", "verified": verified, "usage_7d": usage,
     "created_at": 1_700_000_000 as UInt64, "lookalike": lookalike]
}

// Preserve every node field and its exact rank. A content hash or larger
// usage signal does not let the wallet promote a lower-ranked record.
let first = row(name: "tide.sea", verified: false, usage: 1)
let second = row(name: "tіde.sea", verified: true, usage: 50_000, lookalike: "tide.sea")
let third = row(name: "tide-tools", verified: true, usage: 999)
let records = try defaultRequest.results(from: [first, second, third])
check(records.map(\.name) == ["tide.sea", "tіde.sea", "tide-tools"], "node order preserved")
check(!records[0].verified && records[1].verified, "hash-present flag preserved without promotion")
check(records[0].lookalike == nil && records[1].lookalike == "tide.sea", "node confusable warning preserved")
check(records[1].url == "sea://tіde.sea" && records[1].id != records[0].id, "confusable identity stays distinct")
check(records[0].title == "App tide.sea" && records[0].description == "Publisher supplied text", "publisher text retained")
check(records[0].publisher == "0x1111111111111111111111111111111111111111" && records[0].category == "games", "publisher and category retained")
check(records[1].usage7d == 50_000 && records[1].createdAt == 1_700_000_000, "chain usage and timestamp")
check(try defaultRequest.results(from: []).isEmpty, "empty result is valid")

// Registered app names are primary, even though navigation is pinned to the
// registry key. The full key remains available for the secondary copy action.
let appKey = "aaaqeayeaudaocajbifqydiob4ibceqtcqkrmfyydenbwha5dypq"
let appURL = "sea://" + appKey + "/pages/My_page-2.html?q=%2f&x=one+two"
var namedAppRow = row(name: "eastsea", verified: true, usage: 3)
namedAppRow["url"] = appURL
namedAppRow["title"] = "EastSea"
let namedApp = try defaultRequest.results(from: [namedAppRow])[0]
check(namedApp.primaryURL == "sea://eastsea", "registered readable name replaces the raw hash address")
check(namedApp.registryKey == appKey && namedApp.registryKey?.count == 52, "secondary copy keeps the full registry key")
check(namedApp.url == appURL, "presentation does not change navigation path or raw query spelling")
check(namedApp.name == "eastsea" && namedApp.title == "EastSea", "presentation does not overwrite registry or publisher fields")
var reservedAppRow = namedAppRow
reservedAppRow["name"] = "search"
check(try defaultRequest.results(from: [reservedAppRow])[0].primaryURL == "sea://search.sea",
      "the registered search name cannot display the native home address")
namedAppRow["verified"] = false
check(try defaultRequest.results(from: [namedAppRow])[0].primaryURL == "sea://eastsea", "content hash presence does not decide readable names")
for name in ["", appKey, "0x" + String(repeating: "a", count: 64), "not a name", "pay", "eastsea/other"] {
    var unnamedRow = namedAppRow
    unnamedRow["name"] = name
    unnamedRow["title"] = "eastsea"
    let unnamedApp = try defaultRequest.results(from: [unnamedRow])[0]
    check(unnamedApp.primaryURL == appURL, "missing or invalid registered name cannot come from a title: \(name)")
    check(unnamedApp.registryKey == appKey && unnamedApp.url == appURL, "hash-only fallback retains full copy and navigation values")
}
check(records[0].primaryURL == "sea://tide.sea" && records[0].registryKey == nil, "named URL needs no duplicate hash detail")
check(records[1].primaryURL == "sea://tіde.sea", "Unicode URL identity is not rewritten into another name")

// Coverage metadata is independently decoded. Missing/invalid metadata is
// unknown; the wallet cannot turn a capped or unavailable activity index into 0.
let completeInfo = try AppSearchInfo.decode(from: ["history_complete": true, "rejected_records": 0,
                                                  "usage_complete": true, "max_records": 100_000])
check(!completeInfo.incomplete && completeInfo.usageComplete, "complete index and extra cap metadata")
check(completeInfo.sourcesConfigured == nil, "older index metadata leaves source configuration unknown")
let noSources = try AppSearchInfo.decode(from: ["history_complete": true, "rejected_records": 0,
                                               "usage_complete": true, "sources_configured": false])
check(noSources.sourcesConfigured == false, "explicit missing-source flag survives decoding")
let configuredSources = try AppSearchInfo.decode(from: ["history_complete": true, "rejected_records": 0,
                                                       "usage_complete": true, "sources_configured": true,
                                                       "sources": ["app_registry": "0x1111111111111111111111111111111111111111"]])
check(configuredSources.sourcesConfigured == true, "source provenance metadata permits extra canonical-source fields")
check(records[0].usageComplete == nil && records[0].usageAvailable(info: completeInfo), "older record default with known coverage")
check(!records[0].usageAvailable(info: nil), "unknown metadata cannot establish activity coverage")
let missingHistory = try AppSearchInfo.decode(from: ["history_complete": false, "rejected_records": 0, "usage_complete": true])
let rejectedRecords = try AppSearchInfo.decode(from: ["history_complete": true, "rejected_records": 1, "usage_complete": true])
let cappedUsage = try AppSearchInfo.decode(from: ["history_complete": true, "rejected_records": 0, "usage_complete": false])
check(missingHistory.incomplete && rejectedRecords.incomplete, "both incomplete-record conditions")
check(!cappedUsage.incomplete && !records[0].usageAvailable(info: cappedUsage), "activity overflow hides a misleading signal")
var incompleteUsageRow = first
incompleteUsageRow["usage_complete"] = false
let incompleteRecord = try defaultRequest.results(from: [incompleteUsageRow])[0]
check(!incompleteRecord.usageAvailable(info: completeInfo), "record-specific incomplete activity hides the signal")
let malformedInfo: [Any?] = [nil, ["history_complete": true, "rejected_records": 0],
                             ["history_complete": true, "rejected_records": -1, "usage_complete": true],
                             ["history_complete": "true", "rejected_records": 0, "usage_complete": true],
                             ["history_complete": true, "rejected_records": 0, "usage_complete": true, "sources_configured": "false"]]
for value in malformedInfo {
    do {
        _ = try AppSearchInfo.decode(from: value)
        check(false, "incomplete or invalid metadata must be unknown")
    } catch let error as AppSearchFailure { check(error == .malformedResponse, "metadata decoding refusal") }
    catch { check(false, "unexpected metadata decoding error") }
}

func malformed(_ value: Any?, request: AppSearchRequest = defaultRequest) {
    do {
        _ = try request.results(from: value)
        check(false, "malformed response must fail as a whole")
    } catch let error as AppSearchFailure { check(error == .malformedResponse, "malformed response classification") }
    catch { check(false, "unexpected decoding error") }
}

malformed(nil)
malformed(["result": [first]])
malformed([first, second], request: try AppSearchRequest(query: "tide", limit: 1))
malformed(Array(repeating: first, count: 51), request: try AppSearchRequest(query: "tide", limit: 50))
for (field, value) in [("verified", "true" as Any), ("usage_7d", -1 as Any),
                       ("url", "https://evil.test" as Any), ("url", "javascript:alert(1)" as Any),
                       ("url", "sea://alice.sea@evil.sea" as Any)] {
    var bad = second
    bad[field] = value
    malformed([first, bad])
}
var missingPublisher = second
missingPublisher.removeValue(forKey: "publisher")
malformed([first, missingPublisher])

print("app-search OK")
