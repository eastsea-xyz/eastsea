import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

/// Country preferences stay in memory; this test never writes the Mac's real
/// defaults or leaves a suite plist in ~/Library/Preferences.
final class PresenceMemoryDefaults: UserDefaults {
    private var values: [String: Any] = [:]
    init() { super.init(suiteName: nil)! }
    override func object(forKey key: String) -> Any? { values[key] }
    override func bool(forKey key: String) -> Bool { values[key] as? Bool ?? false }
    override func set(_ value: Any?, forKey key: String) { values[key] = value }
    override func set(_ value: Bool, forKey key: String) { set(value as Any?, forKey: key) }
    override func removeObject(forKey key: String) { values.removeValue(forKey: key) }
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
check(presence?.byRegion["asia"] == 4 && presence?.byCountry["KR"] == 1, "continent and shared country counts decode")
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

let defaults = PresenceMemoryDefaults()
check(PresenceCountry.enabled(defaults: defaults), "country sharing defaults on before any preference is saved")
check(PresenceCountry.shared(defaults: defaults, region: "KR") == "KR", "default country comes from the Mac region")
check(PresenceCountry.flags(defaults: defaults, region: "KR") == ["--presence-country=KR"], "default region produces the run flag")
check(PresenceCountry.shouldShowNotice(defaults: defaults, region: "KR"), "first launch presents the country notice")
check(PresenceCountry.shouldShowNotice(defaults: defaults, region: "KR")
      && defaults.object(forKey: PresenceCountry.noticeSeenKey) == nil,
      "checking or queuing the notice does not mark it seen")

defaults.set("US", forKey: "presenceCountryCode")
check(PresenceCountry.shared(defaults: defaults, region: "KR") == "KR", "legacy manual country cannot override the current Mac region")
check(PresenceCountry.shared(defaults: defaults, region: "JP") == "JP", "later Mac region changes do not keep a stale country")
defaults.set(false, forKey: PresenceCountry.sharingKey)
check(!PresenceCountry.enabled(defaults: defaults), "a saved explicit opt-out survives the new default")
check(PresenceCountry.flags(defaults: defaults, region: "KR").isEmpty, "saved opt-out removes the current region flag")
check(!PresenceCountry.shouldShowNotice(defaults: defaults, region: "KR"), "opted-out Macs do not see a claim that country is shown")
defaults.set(true, forKey: PresenceCountry.sharingKey)
check(PresenceCountry.shouldShowNotice(defaults: defaults, region: "KR"), "enabling sharing still offers an unseen notice")
PresenceCountry.markNoticeSeen(defaults: defaults)
check(!PresenceCountry.shouldShowNotice(defaults: defaults, region: "KR"), "acknowledging the visible notice prevents it returning")
check(defaults.bool(forKey: PresenceCountry.noticeSeenKey), "notice acknowledgment persists")
check(PresenceCountry.shared(defaults: defaults, region: nil) == nil, "missing Mac region safely omits country")
check(!PresenceCountry.shouldShowNotice(defaults: PresenceMemoryDefaults(), region: "EU"), "invalid Mac region never claims a country is shown")

check(PresenceCountry.flags(sharing: false, country: "KR").isEmpty, "explicit opt-out sends no country flag")
check(PresenceCountry.shared(sharing: true, country: "") == nil, "an unavailable region does not guess a country")
check(PresenceCountry.flags(sharing: true, country: "kr") == ["--presence-country=KR"], "lowercase regions normalize to node country codes")
check(PresenceCountry.codes.count == 249 && Set(PresenceCountry.codes).count == 249,
      "the region validator accepts exactly 249 unique ISO countries")
for country in ["AC", "CP", "DG", "EA", "IC", "TA", "UK"] {
    check(!PresenceCountry.codes.contains(country) && PresenceCountry.flags(sharing: true, country: country).isEmpty,
          "CLDR-only region is not passed to the strict ISO node: \(country)")
}
for country in ["AQ", "BV", "GB", "HM", "SH", "SJ", "UM"] {
    check(PresenceCountry.flags(sharing: true, country: country) == ["--presence-country=\(country)"],
          "valid ISO countries and territories remain accepted: \(country)")
}
for country in ["", "Asia", "AA", "123", "KR --archive", "../KR", "ZZ", "EU"] {
    check(PresenceCountry.flags(sharing: true, country: country).isEmpty, "invalid country produces no argument: \(country)")
}
let unattended = UnattendedDecision.nodeArgv(dataDir: "./tmp/presence-node", rpcPort: 18545, p2pPort: 19101,
    networkPath: nil, proverFlags: [], presenceFlags: PresenceCountry.flags(defaults: defaults, region: "KR"))
check(unattended.last == "--presence-country=KR", "unattended restart uses the same current Mac region")
defaults.set(false, forKey: PresenceCountry.sharingKey)
check(!UnattendedDecision.nodeArgv(dataDir: "./tmp/presence-node", rpcPort: 18545, p2pPort: 19101,
    networkPath: nil, proverFlags: [], presenceFlags: PresenceCountry.flags(defaults: defaults, region: "KR"))
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
            "Mac region", "Region unavailable", "Your country on the globe", "Done", "Network",
            "Your country is shown on the globe; you can turn it off in Settings",
            "Macs this node can see", "Open Network", "Live network unavailable", "Last available count",
            "The globe shows this Mac’s node view. Counts can differ between nodes.",
            "Live count: what this node can see. Regions come from relays, not a Mac's location.",
            "Country comes from this Mac's region setting. Turning it off keeps this Mac in its continent. The globe receives no IP addresses, cities or coordinates."]
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
let noticeKey = "Your country is shown on the globe; you can turn it off in Settings"
let notice = strings[noticeKey]!["localizations"] as! [String: [String: Any]]
check((notice["en"]!["stringUnit"] as! [String: String])["value"] == noticeKey,
      "English first-launch notice matches the founder's exact copy")
check((notice["ko"]!["stringUnit"] as! [String: String])["value"] == "지구본에 내 나라를 표시해요. 설정에서 끌 수 있어요.",
      "Korean first-launch notice matches the founder's exact copy")
print("ok")
