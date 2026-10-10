import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

let observed: TimeInterval = 999_600
let now = observed + 400
let value: [String: Any] = [
    "schema": 2, "available": true, "total": 9,
    "by_role": ["validator": 3, "other": 6],
    "by_version": ["unknown": 9],
    "by_region": ["asia": 3, "europe": 3, "world": 3],
    "ttl_seconds": 600, "minimum_bucket_size": 3, "observed_at": observed,
    "scope": "unverified cohort observation"
]

let presence = LivePresence.parse(value, now: now)
check(presence?.total == 9 && presence?.byRole["other"] == 6, "schema 2 decodes thresholded role counts")
check(presence?.byRegion["asia"] == 3 && presence?.byRegion["world"] == 3, "folded region buckets decode")
check(presence?.line == "9 node observations · Asia (legacy broad region) 3 · Europe (legacy broad region) 3 · All regions 3",
      "headline does not describe unverified cohorts as distinct physical Macs")
var m49 = value
m49["by_region"] = ["030": 3, "202": 3, "419": 3]
let regional = LivePresence.parse(m49, now: now)
check(regional?.byRegion["030"] == 3 && regional?.line.contains("Eastern Asia 3") == true,
      "canonical M49 sub-regions are accepted and labeled")
for intermediate in ["014", "017", "029", "013", "005"] {
    m49["by_region"] = [intermediate: 9]
    check(LivePresence.parse(m49, now: now) == nil, "intermediate-region codes are not sub-regions")
}
check(LivePresence.parse(nil, now: now) == nil, "missing RPC is unavailable, never zero")
check(LivePresence.parse(["code": -32601, "message": "Method not found"], now: now) == nil, "old node RPC is unavailable")

var suppressed = value
suppressed["total"] = NSNull()
for key in ["by_role", "by_region", "by_version"] { suppressed[key] = [String: Int]() }
let withheld = LivePresence.parse(suppressed, now: now)
check(withheld != nil && withheld?.total == nil && withheld?.line == "Counts withheld for privacy",
      "an under-k cohort is withheld, never represented as zero")

var changed = value
changed["available"] = false
check(LivePresence.parse(changed, now: now) == nil, "unavailable samples stay hidden")
for schema in [1, 3] {
    changed = value
    changed["schema"] = schema
    check(LivePresence.parse(changed, now: now) == nil, "unsupported or legacy per-node schemas stay hidden")
}
changed = value
changed["observed_at"] = observed - 600
check(LivePresence.parse(changed, now: now) == nil, "a frozen sample expires at its ten-minute TTL")
changed = value
changed["observed_at"] = observed + 600
check(LivePresence.parse(changed, now: now) == nil, "a far-future sample cannot look current")
changed = value
changed["observed_at"] = observed + 1
check(LivePresence.parse(changed, now: now) == nil, "exact observation timestamps are rejected")
changed = value
changed["minimum_bucket_size"] = 2
check(LivePresence.parse(changed, now: now) == nil, "a smaller producer privacy threshold is rejected")
changed = value
changed["ttl_seconds"] = 180
check(LivePresence.parse(changed, now: now) == nil, "legacy high-resolution samples are rejected")

for key in ["by_role", "by_region", "by_version"] {
    for rare in [-1, 0, 1, 2] {
        changed = value
        changed[key] = ["unknown": rare, key == "by_region" ? "world" : "other": 9 - rare]
        check(LivePresence.parse(changed, now: now) == nil, "\(key) rejects a bucket below k=3")
    }
    changed = value
    changed[key] = ["unknown": 3]
    check(LivePresence.parse(changed, now: now) == nil, "\(key) must partition the disclosed total")
    changed = suppressed
    changed[key] = ["unknown": 3]
    check(LivePresence.parse(changed, now: now) == nil, "a withheld cohort cannot expose \(key)")
}
for total in [-1, 0, 1, 2] {
    changed = suppressed
    changed["total"] = total
    check(LivePresence.parse(changed, now: now) == nil, "an under-k numeric total must be null")
}
for key in ["nodes", "observer", "by_country", "country", "timestamp", "last_seen"] {
    changed = value
    changed[key] = key == "nodes" ? [Any]() : "forbidden"
    check(LivePresence.parse(changed, now: now) == nil, "legacy identifying field \(key) fails closed")
}
changed = value
changed["by_version"] = ["0.7.4": 9]
check(LivePresence.parse(changed, now: now) == nil, "individual build versions are not a public quality bucket")
changed = value
changed["by_region"] = ["KR": 9]
check(LivePresence.parse(changed, now: now) == nil, "country codes are not public region buckets")

let walletRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .deletingLastPathComponent().deletingLastPathComponent()
let catalogData = try Data(contentsOf: walletRoot.appendingPathComponent("Resources/Localizable.xcstrings"))
let catalog = try JSONSerialization.jsonObject(with: catalogData) as! [String: Any]
let strings = catalog["strings"] as! [String: [String: Any]]
let keys = ["%lld node observations", "%@ %lld", "Asia", "Europe", "North America", "South America",
            "Africa", "Oceania", "Unknown region", "All regions", "Counts withheld for privacy",
            "Unverified cohort observations, not a count of distinct Macs. Broad regions use a local country choice or relays; small groups are folded together."]
for language in ["en", "ko", "ja", "zh-Hans", "zh-Hant"] {
    for key in keys {
        let localizations = strings[key]?["localizations"] as? [String: [String: Any]]
        let unit = localizations?[language]?["stringUnit"] as? [String: String]
        check(unit?["state"] == "translated" && unit?["value"]?.isEmpty == false,
              "\(language) translates \(key)")
    }
}
print("ok")
