// One fixture is the URL contract for Swift, JavaScript and Rust. No app,
// network or signing key is needed to check name input and legacy actions.
import Foundation

func fields(_ link: SeaURL.NameLink) -> [String: Any] {
    ["kind": "name", "name": link.name, "canonicalURL": link.canonicalURL,
     "path": link.path, "query": link.query as Any? ?? NSNull(),
     "isLegacy": link.isLegacy, "registryName": link.registryName]
}

func parsed(_ raw: String, operation: String, chainID: UInt64) -> [String: Any] {
    do {
        if operation == "browser" {
            switch try SeaURL.browserInput(raw, chainID: chainID) {
            case .name(let link): return fields(link)
            case .action(let host, let raw): return ["kind": "action", "host": host, "raw": raw]
            case .web(let url): return ["kind": "web", "url": url.absoluteString]
            }
        }
        switch try SeaURL.parse(raw, chainID: chainID) {
        case .name(let link): return fields(link)
        case .action(let host, let raw): return ["kind": "action", "host": host, "raw": raw]
        }
    } catch {
        return ["error": String(describing: error)]
    }
}

let fixturePath = ProcessInfo.processInfo.environment["SEA_URL_FIXTURE"] ?? "tests/fixtures/sea-urls.json"
let data = try Data(contentsOf: URL(fileURLWithPath: fixturePath))
let fixture = try JSONSerialization.jsonObject(with: data) as! [String: Any]
let cases = fixture["cases"] as! [[String: Any]]
var failures = 0
for row in cases {
    let id = row["id"] as! String
    let raw = row["input"] as! String
    let chainID = (row["chainID"] as! NSNumber).uint64Value
    let actual = parsed(raw, operation: row["operation"] as! String, chainID: chainID)
    let expected = row["expected"] as! [String: Any]
    if !NSDictionary(dictionary: actual).isEqual(to: expected) {
        print("FAIL \(id): expected \(expected), got \(actual)")
        failures += 1
    }
    if let expectedHTTPS = row["suggestedHTTPS"] as? String {
        if SeaURL.suggestedHTTPS(raw)?.absoluteString != expectedHTTPS {
            print("FAIL \(id): HTTPS offer changed")
            failures += 1
        }
    }
}

// An HTTPS offer never turns an invalid authority into a different host.
for raw in ["sea://user@harbor.com", "sea://harbor.com:443", "sea://harbor.com\\evil", "sea://harbor.com/#pay"] {
    if SeaURL.suggestedHTTPS(raw) != nil {
        print("FAIL unsafe HTTPS offer for \(raw)")
        failures += 1
    }
}
print("sea-url: \(cases.count) shared cases, \(failures) failures")
exit(Int32(failures))
