import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

func object(_ presence: LiveGlobePresence) -> [String: Any] {
    try! JSONSerialization.jsonObject(with: Data(presence.aggregateJSON.utf8)) as! [String: Any]
}

func quality(_ scores: [Int]) -> [String: Any] {
    var bins = Array(repeating: 0, count: 20)
    for score in scores { bins[min(19, score / 50_000)] += 1 }
    return ["score_sum": scores.reduce(0, +), "histogram": bins]
}

func region(_ continent: String, country: String? = nil, scores: [Int]) -> [String: Any] {
    var result: [String: Any] = ["continent": continent, "count": scores.count, "quality": quality(scores)]
    if let country { result["country"] = country }
    return result
}

func aggregate(_ regions: [[String: Any]]) -> [String: Any] {
    let total = regions.reduce(0) { $0 + ($1["count"] as! Int) }
    return [
        "schema_version": 3, "scope": "node", "quality_version": 1, "total": total,
        "roles": ["validator": ["count": 0], "wallet": ["count": 0],
                  "candidate": ["count": 0], "follower": ["count": total]],
        "versions": ["0.7.4": total], "reserve_keys": ["standby": 0, "seated": 0],
        "regions": regions,
    ]
}

let now: TimeInterval = 1_000_000
typealias Node = (continent: String, country: String?, role: String, version: String)
func legacy(_ specifications: [Node]) -> [String: Any] {
    var roles = ["validator": 0, "candidate": 0, "follower": 0]
    var versions: [String: Int] = [:], regions: [String: Int] = [:], countries: [String: Int] = [:]
    let nodes: [[String: Any]] = specifications.enumerated().map { index, specification in
        roles[specification.role, default: 0] += 1
        versions[specification.version, default: 0] += 1
        regions[specification.continent, default: 0] += 1
        if let country = specification.country { countries[country, default: 0] += 1 }
        var result: [String: Any] = [
            "node_id": "private-node-\(index)", "role": specification.role, "version": specification.version,
            "region": specification.continent, "timestamp": now - 10, "last_seen": now - 2,
        ]
        if let country = specification.country { result["country"] = country }
        return result
    }
    return [
        "schema": 1, "available": true, "total": nodes.count,
        "nodes": nodes, "observer": "private-observer", "scope": "what this node can see",
        "observed_at": now, "ttl_seconds": 180,
        "by_role": roles, "by_version": versions, "by_region": regions, "by_country": countries,
    ]
}

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
    .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
let fixtureData = try Data(contentsOf: root.appendingPathComponent("apps/explorer/test/fixtures/presence-example.json"))
let fixture = try JSONSerialization.jsonObject(with: fixtureData) as! [String: Any]
let sample = LiveGlobePresence.parse(fixture, now: now)!
check(sample.total == 24 && sample.roles["validator"] == 6 && sample.roles["follower"] == 15,
      "canonical v3 fixture preserves population and role counts")
check(sample.hasQualityEvidence, "validated v3 producer summaries have evidence")
check(sample.versions == ["0.7.4": 21, "0.7.3": 3] && sample.recentBlocks.count == 3,
      "versions and known block-region sequence survive")
check(sample.regions.reduce(0) { $0 + $1.count } == 24, "region buckets remain disjoint")
let encoded = object(sample)
check(Set(encoded.keys) == Set(["schema_version", "scope", "quality_version", "total", "roles", "versions",
                               "reserve_keys", "regions", "recent_blocks"]), "bridge JSON has exact canonical root fields")
check(encoded["scope"] as? String == "node", "private observer scope never crosses the bridge")

// Unknown fields are discarded at every level; only counts can enter the page.
var dirty = fixture
dirty["nodes"] = [["node_id": "SECRET-root-node", "address": "SECRET-wallet", "country": "SECRET-country"]]
dirty["observer"] = "SECRET-observer"
dirty["relay_url"] = "SECRET-url"
dirty["timestamp"] = now
var dirtyRoles = dirty["roles"] as! [String: [String: Any]]
dirtyRoles["validator"]?["address"] = "SECRET-role"
dirtyRoles["invented"] = ["count": 999, "node_id": "SECRET-extra-role"]
dirty["roles"] = dirtyRoles
var dirtyReserves = dirty["reserve_keys"] as! [String: Any]
dirtyReserves["keys"] = ["SECRET-reserve-key"]
dirty["reserve_keys"] = dirtyReserves
var dirtyRegions = dirty["regions"] as! [[String: Any]]
dirtyRegions[0]["coordinates"] = [42.0, 3.0]
dirtyRegions[0]["node_id"] = "SECRET-region"
var dirtyQuality = dirtyRegions[0]["quality"] as! [String: Any]
dirtyQuality["beacon_epochs"] = [1234, 1235]
dirtyQuality["proof_address"] = "SECRET-proof"
dirtyRegions[0]["quality"] = dirtyQuality
dirty["regions"] = dirtyRegions
var dirtyBlocks = dirty["recent_blocks"] as! [[String: Any]]
dirtyBlocks[0]["hash"] = "SECRET-hash"
dirtyBlocks[0]["proposer"] = "SECRET-proposer"
dirty["recent_blocks"] = dirtyBlocks
let projected = LiveGlobePresence.parse(dirty, now: now)!
check(projected == sample && !projected.aggregateJSON.contains("SECRET"), "no source identifier or unknown field is serialized")
let encodedRegions = object(projected)["regions"] as! [[String: Any]]
check(encodedRegions.allSatisfy { Set($0.keys).isSubset(of: Set(["continent", "country", "count", "quality"])) },
      "region fields are allowlisted")
check(encodedRegions.allSatisfy { Set(($0["quality"] as! [String: Any]).keys) == Set(["score_sum", "histogram"]) },
      "quality fields are exactly canonical")

// Merging before k=3 preserves safe country groups. The same code in another
// relay continent cannot borrow population to disclose a small group.
let folded = LiveGlobePresence.parse(aggregate([
    region("asia", country: "KR", scores: [10_000]),
    region("asia", country: "KR", scores: [60_000, 110_000]),
    region("europe", country: "KR", scores: [160_000, 210_000]),
    region("asia", country: "US", scores: [260_000]),
    region("asia", scores: [310_000]),
]), now: now)!
check(folded.regions.map(\.continent) == ["asia", "asia", "europe"], "folding has stable canonical continent order")
check(folded.regions[0].country == nil && folded.regions[0].count == 2,
      "small country merges into the continent-only remainder")
check(folded.regions[0].quality.scoreSum == 570_000 && folded.regions[0].quality.histogram[5] == 1
      && folded.regions[0].quality.histogram[6] == 1, "folding preserves score sum and every histogram bin")
check(folded.regions[1].country == "KR" && folded.regions[1].count == 3
      && folded.regions[1].quality.scoreSum == 180_000, "duplicate country groups are combined before the threshold")
check(folded.regions[2].country == nil && folded.regions[2].count == 2
      && folded.regions[2].quality.scoreSum == 370_000, "k is enforced within each continent")

var overlapping = fixture
overlapping["roles"] = ["validator": ["count": 24], "wallet": ["count": 24],
                        "candidate": ["count": 24], "follower": ["count": 24]]
check(LiveGlobePresence.parse(overlapping, now: now)?.total == 24, "roles can overlap without inflating Mac total")
var releaseFixture = fixture
releaseFixture["versions"] = ["0.7.4-rc.1+mac.arm64": 24]
check(LiveGlobePresence.parse(releaseFixture, now: now)?.versions["0.7.4-rc.1+mac.arm64"] == 24,
      "full semver release identifiers are accepted")

for (key, value) in [("schema_version", 1), ("schema_version", 2), ("schema_version", 4),
                     ("quality_version", 0), ("quality_version", 2)] {
    var invalid = fixture
    invalid[key] = value
    check(LiveGlobePresence.parse(invalid, now: now) == nil, "unsupported schema or quality version is rejected: \(key)=\(value)")
}
let invalidCounts: [Any] = [-1, 1.5, true, "24", Double.nan, Double.infinity, 9_007_199_254_740_992]
for value in invalidCounts {
    var invalid = fixture
    invalid["total"] = value
    check(LiveGlobePresence.parse(invalid, now: now) == nil, "total must be a nonnegative safe integer")
}
for version in ["0.07.4", "0.7.4-01", "0.7.4\n", "0.٧.4", "node-message", String(repeating: "1", count: 65)] {
    var invalid = fixture
    invalid["versions"] = [version: 24]
    check(LiveGlobePresence.parse(invalid, now: now) == nil, "arbitrary or invalid release keys are rejected: \(version)")
}
var invalid = fixture
invalid["scope"] = "what this node can see"
check(LiveGlobePresence.parse(invalid, now: now) == nil, "v3 requires node aggregate scope")
invalid = fixture
invalid["versions"] = ["0.7.4": 23]
check(LiveGlobePresence.parse(invalid, now: now) == nil, "version totals must match Mac total")
invalid = fixture
invalid["roles"] = ["validator": ["count": 25], "wallet": ["count": 0],
                    "candidate": ["count": 0], "follower": ["count": 0]]
check(LiveGlobePresence.parse(invalid, now: now) == nil, "one role cannot exceed Mac total")
invalid = fixture
invalid["roles"] = ["validator": ["count": 6]]
check(LiveGlobePresence.parse(invalid, now: now) == nil, "all four canonical roles are required")
invalid = fixture
invalid["reserve_keys"] = ["standby": 9_007_199_254_740_991, "seated": 1]
check(LiveGlobePresence.parse(invalid, now: now) == nil, "reserve-key cumulative overflow is rejected")
invalid = fixture
invalid["regions"] = []
check(LiveGlobePresence.parse(invalid, now: now) == nil, "regions must account for the full population")
for key in ["continent", "country"] {
    var entries = fixture["regions"] as! [[String: Any]]
    entries[0][key] = key == "continent" ? "city" : "kr"
    invalid = fixture
    invalid["regions"] = entries
    check(LiveGlobePresence.parse(invalid, now: now) == nil, "only canonical geographic codes are accepted")
}
let invalidQualities: [[String: Any]] = [
    ["score_sum": 0, "histogram": [0]],
    ["score_sum": 0, "histogram": Array(repeating: 0, count: 20)],
    ["score_sum": 50_000, "histogram": [1] + Array(repeating: 0, count: 19)],
    ["score_sum": 49_999, "histogram": [0, 1] + Array(repeating: 0, count: 18)],
    ["score_sum": true, "histogram": [1] + Array(repeating: 0, count: 19)],
    ["score_sum": 0, "histogram": [-1, 2] + Array(repeating: 0, count: 18)],
]
for badQuality in invalidQualities {
    var entry = region("asia", scores: [0])
    entry["quality"] = badQuality
    check(LiveGlobePresence.parse(aggregate([entry]), now: now) == nil, "quality population and feasible bounds are validated")
}
var missingQuality = region("asia", scores: [0])
missingQuality.removeValue(forKey: "quality")
check(LiveGlobePresence.parse(aggregate([missingQuality]), now: now) == nil, "missing v3 quality is rejected, not invented")
let peak = LiveGlobePresence.parse(aggregate([region("asia", scores: [1_000_000])]), now: now)!
check(peak.regions[0].quality.histogram[19] == 1, "Q=1 is valid in the last numerical bin")
let maximum = 9_007_199_254_740_991
let largeRegion: [String: Any] = ["continent": "unknown", "count": maximum,
                                "quality": ["score_sum": 0, "histogram": [maximum] + Array(repeating: 0, count: 19)]]
check(LiveGlobePresence.parse(aggregate([largeRegion]), now: now)?.total == maximum,
      "large valid zero summaries do not overflow feasibility bounds")
var impossibleLargeRegion = largeRegion
impossibleLargeRegion["quality"] = ["score_sum": maximum, "histogram": Array(repeating: 0, count: 19) + [maximum]]
check(LiveGlobePresence.parse(aggregate([impossibleLargeRegion]), now: now) == nil,
      "large impossible bounds reject without overflowing native integers")
invalid = fixture
invalid["regions"] = Array(repeating: region("asia", scores: []), count: 1_025)
check(LiveGlobePresence.parse(invalid, now: now) == nil, "raw region collection is bounded")
invalid = fixture
invalid["recent_blocks"] = Array(repeating: ["height": 1, "continent": "asia"], count: 9)
check(LiveGlobePresence.parse(invalid, now: now) == nil, "recent blocks are bounded")
invalid = fixture
invalid["recent_blocks"] = [["height": 1, "continent": "city"]]
check(LiveGlobePresence.parse(invalid, now: now) == nil, "unknown block geography cannot synthesize an arc")
invalid = fixture
invalid["__proto__"] = ["total": 24]
check(LiveGlobePresence.parse(invalid, now: now) == nil, "prototype-sensitive record keys are rejected")

// Legacy local data is validated first, grouped natively and never bridged raw.
let specifications: [Node] = [
    ("asia", "KR", "validator", "0.7.4"), ("asia", "KR", "candidate", "0.7.4"),
    ("asia", "KR", "follower", "0.7.3"), ("europe", "KR", "follower", "0.7.4"),
    ("europe", "KR", "follower", "0.7.4"), ("asia", "US", "follower", "0.7.4"),
    ("asia", nil, "follower", "0.7.4"), ("unknown", nil, "follower", "0.7.4"),
]
let legacySource = legacy(specifications)
let local = LiveGlobePresence.parse(legacySource, now: now)!
check(local.total == 8 && local.roles == ["validator": 1, "wallet": 0, "candidate": 1, "follower": 6],
      "legacy records provide only observed roles, without inventing wallet overlap")
check(local.versions == ["0.7.4": 7, "0.7.3": 1], "legacy version counts are grouped from validated records")
check(!local.hasQualityEvidence && local.regions.allSatisfy { $0.quality.scoreSum == 0
      && $0.quality.histogram == [$0.count] + Array(repeating: 0, count: 19) },
      "missing chain evidence remains unmeasured with complete conservative summaries")
check(local.reserveKeys.standby == 0 && local.reserveKeys.seated == 0 && local.recentBlocks.isEmpty,
      "legacy pings cannot invent reserve keys or block sequences")
check(local.regions.filter { $0.country == "KR" }.map(\.continent) == ["asia"],
      "legacy country threshold is local to each relay continent")
check(!local.aggregateJSON.contains("private-") && !local.aggregateJSON.contains("by_country")
      && !local.aggregateJSON.contains("observed_at") && !local.aggregateJSON.contains("node_id"),
      "legacy identifiers, singleton maps and timestamps never enter JavaScript")
check(LiveGlobePresence.parse(object(local), now: now)?.aggregateJSON == local.aggregateJSON,
      "legacy projection is a complete valid canonical v3 payload")
var untrustedEvidence = legacySource
untrustedEvidence["quality_version"] = 1
untrustedEvidence["reserve_keys"] = ["standby": 999, "seated": 999]
untrustedEvidence["recent_blocks"] = [["height": 999, "continent": "asia"]]
check(LiveGlobePresence.parse(untrustedEvidence, now: now) == local,
      "legacy extra fields cannot fabricate quality evidence, reserve keys or arcs")

let invalidLegacyFields: [(String, Any)] = [("available", false), ("observed_at", now - 180),
                                          ("observed_at", now + 61), ("ttl_seconds", 181),
                                          ("by_role", ["validator": 8]), ("by_country", ["KR": 4])]
for (key, value) in invalidLegacyFields {
    var invalid = legacySource
    invalid[key] = value
    check(LiveGlobePresence.parse(invalid, now: now) == nil, "malformed or expired legacy reply is rejected: \(key)")
}
var duplicate = legacySource["nodes"] as! [[String: Any]]
duplicate[1]["node_id"] = duplicate[0]["node_id"]
invalid = legacySource
invalid["nodes"] = duplicate
check(LiveGlobePresence.parse(invalid, now: now) == nil, "duplicate legacy identities cannot inflate aggregate counts")
var expiredNodes = legacySource["nodes"] as! [[String: Any]]
expiredNodes[0]["timestamp"] = now - 180
invalid = legacySource
invalid["nodes"] = expiredNodes
check(LiveGlobePresence.parse(invalid, now: now) == nil, "fresh observer timestamps cannot hide expired pings")
check(LiveGlobePresence.parse(legacy([("asia", "ZZ", "follower", "0.7.4")]), now: now) == nil,
      "legacy country is validated against ISO before aggregation")
check(LiveGlobePresence.parse(legacy([("city", nil, "follower", "0.7.4")]), now: now) == nil,
      "legacy geography cannot become arbitrary strings")
check(LiveGlobePresence.parse(legacy([("asia", nil, "founder", "0.7.4")]), now: now) == nil,
      "legacy roles cannot introduce identity categories")
check(LiveGlobePresence.parse(legacy([("asia", nil, "follower", "arbitrary release")]), now: now) == nil,
      "legacy versions cannot carry arbitrary messages")

check(LiveGlobePresence.read(nil, retaining: nil, now: now).state == .unavailable,
      "first failed read is unavailable rather than an empty network")
let stale = LiveGlobePresence.read(nil, retaining: local, now: now + 180)
check(stale.state == .stale && stale.presence == local, "failed reads retain only the last safe aggregate as stale")
let recovered = LiveGlobePresence.read(fixture, retaining: local, now: now)
check(recovered.state == .ready && recovered.presence == sample, "fresh validated data clears stale state")
let empty = LiveGlobePresence.read(aggregate([]), retaining: sample, now: now)
check(empty.state == .ready && empty.presence?.total == 0 && empty.presence?.regions.isEmpty == true,
      "an explicitly empty aggregate is a successful read")
check(LiveGlobePresenceState.loading.rawValue == "loading" && LiveGlobePresenceState.stale.rawValue == "stale",
      "host state names are stable bridge inputs")
print("ok")
