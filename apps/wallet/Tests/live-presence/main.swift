import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

let now: TimeInterval = 1_000_000
let nodes: [[String: Any]] = (0..<6).map { index in
    var node: [String: Any] = [
        "node_id": String(repeating: String(index), count: 64),
        "role": index < 4 ? "validator" : "follower",
        "version": "0.7.4", "timestamp": now - 10, "last_seen": now - 2,
        "region": index < 4 ? "asia" : "europe"
    ]
    if index == 0 { node["country"] = "KR" }
    return node
}
let value: [String: Any] = [
    "schema": 1, "available": true, "total": 6,
    "by_role": ["validator": 4, "candidate": 0, "follower": 2],
    "by_version": ["0.7.4": 6],
    "by_region": ["asia": 4, "europe": 2, "north_america": 0, "south_america": 0,
                  "africa": 0, "oceania": 0, "unknown": 0],
    "by_country": ["KR": 1], "nodes": nodes,
    "ttl_seconds": 180, "observed_at": now, "observer": nodes[0]["node_id"]!,
    "scope": "what this node can see"
]

let presence = LivePresence.parse(value, now: now)
check(presence?.total == 6 && presence?.byRole["follower"] == 2, "schema 1 decodes role counts")
check(presence?.byRegion["asia"] == 4 && presence?.byCountry["KR"] == 1, "continent and opt-in country counts decode")
check(presence?.nodes.last?.country == nil, "country may be omitted when sharing is off")
check(presence?.line == "6 Macs online now · Asia 4 · Europe 2", "one line shows live Macs and continents in stable order")
check(LivePresence.parse(nil, now: now) == nil, "missing RPC is unavailable, never zero")
check(LivePresence.parse(["code": -32601, "message": "Method not found"], now: now) == nil, "old node RPC is unavailable")

var changed = value
changed["available"] = false
check(LivePresence.parse(changed, now: now) == nil, "an unavailable observer never displays zero")
changed = value
changed["schema"] = 2
check(LivePresence.parse(changed, now: now) == nil, "an unknown schema is hidden")
changed = value
changed["observed_at"] = now - 180
check(LivePresence.parse(changed, now: now) == nil, "a sample expires at its TTL")
changed = value
changed["observed_at"] = now + 61
check(LivePresence.parse(changed, now: now) == nil, "a far-future sample cannot look live")
changed = value
changed["by_role"] = ["validator": -1, "follower": 7]
check(LivePresence.parse(changed, now: now) == nil, "negative counts are rejected")
changed = value
changed["by_region"] = ["asia": 5]
check(LivePresence.parse(changed, now: now) == nil, "partial region counts cannot masquerade as a full observation")
changed = value
changed["by_country"] = ["KR": 7]
check(LivePresence.parse(changed, now: now) == nil, "country counts cannot exceed the live count")
var duplicate = nodes
duplicate[5]["node_id"] = nodes[0]["node_id"]
changed = value
changed["nodes"] = duplicate
check(LivePresence.parse(changed, now: now) == nil, "duplicate node ids cannot inflate the wallet count")

check(PresenceCountry.flags(sharing: false, country: "KR").isEmpty, "default off sends no country flag even with a saved selection")
check(PresenceCountry.shared(sharing: true, country: "") == nil, "turning sharing on does not guess a country")
check(PresenceCountry.flags(sharing: true, country: "kr") == ["--presence-country=KR"], "explicit country produces the run flag")
check(PresenceCountry.codes.count == 249 && Set(PresenceCountry.codes).count == 249,
      "the picker offers the 249 unique ISO countries accepted by the node")
for country in ["AC", "CP", "DG", "EA", "IC", "TA", "UK"] {
    check(!PresenceCountry.codes.contains(country) && PresenceCountry.flags(sharing: true, country: country).isEmpty,
          "CLDR-only region is not offered or passed to the strict ISO node: \(country)")
}
for country in ["AQ", "BV", "GB", "HM", "SH", "SJ", "UM"] {
    check(PresenceCountry.flags(sharing: true, country: country) == ["--presence-country=\(country)"],
          "valid ISO countries and territories remain selectable: \(country)")
}
for country in ["", "Asia", "AA", "123", "KR --archive", "../KR", "ZZ", "EU"] {
    check(PresenceCountry.flags(sharing: true, country: country).isEmpty, "invalid country produces no argument: \(country)")
}
let unattended = UnattendedDecision.nodeArgv(dataDir: "./tmp/presence-node", rpcPort: 18545, p2pPort: 19101,
    networkPath: nil, proverFlags: [], presenceFlags: PresenceCountry.flags(sharing: true, country: "KR"))
check(unattended.last == "--presence-country=KR", "unattended restart keeps the same explicit country")
check(!UnattendedDecision.nodeArgv(dataDir: "./tmp/presence-node", rpcPort: 18545, p2pPort: 19101,
    networkPath: nil, proverFlags: [], presenceFlags: PresenceCountry.flags(sharing: false, country: "KR"))
    .contains { $0.hasPrefix("--presence-country") }, "opt-out removes the unattended country argument")

// Verify the five catalog translations and real placeholders without starting
// an app, querying location or touching wallet data.
let walletRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .deletingLastPathComponent().deletingLastPathComponent()
let catalogData = try Data(contentsOf: walletRoot.appendingPathComponent("Resources/Localizable.xcstrings"))
let catalog = try JSONSerialization.jsonObject(with: catalogData) as! [String: Any]
let strings = catalog["strings"] as! [String: [String: Any]]
let keys = ["%lld Macs online now", "%@ %lld", "Asia", "Europe", "North America", "South America",
            "Africa", "Oceania", "Unknown region", "Live network privacy", "Share this Mac's country",
            "Country", "Choose a country",
            "Live count: what this node can see. Regions come from relays, not a Mac's location.",
            "Your relay's continent is shared automatically. Country sharing is off by default and uses only the country you choose. No IP address, city or coordinates are shared."]
for language in ["en", "ko", "ja", "zh-Hans", "es"] {
    for key in keys {
        let localizations = strings[key]?["localizations"] as? [String: [String: Any]]
        let unit = localizations?[language]?["stringUnit"] as? [String: String]
        check(unit?["state"] == "translated" && unit?["value"]?.isEmpty == false,
              "\(language) translates \(key)")
    }
}
let korean = strings["%lld Macs online now"]!["localizations"] as! [String: [String: Any]]
let koreanUnit = korean["ko"]!["stringUnit"] as! [String: String]
check(String(format: koreanUnit["value"]!, Int64(6)) == "지금 연결된 Mac 6대", "Korean live count formats the real number")
print("ok")
