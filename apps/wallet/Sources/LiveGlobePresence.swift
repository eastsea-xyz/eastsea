import Foundation
import CoreFoundation

enum LiveGlobePresenceState: String, Equatable, Sendable {
    case loading, ready, unavailable, stale, withheld
}

/// The only presence value allowed to cross into the bundled globe. Schema-2
/// cohorts cross unchanged; schema-3 aggregate fixtures contain only public fields.
struct LiveGlobePresence: Equatable, Sendable {
    struct Quality: Encodable, Equatable, Sendable {
        let scoreSum: Int
        let histogram: [Int]

        enum CodingKeys: String, CodingKey {
            case scoreSum = "score_sum", histogram
        }

        /// Schema 2 carries no chain evidence. This native placeholder stays
        /// outside its bridge JSON; hasQualityEvidence keeps it unmeasured.
        fileprivate static func unmeasured(count: Int) -> Self {
            Self(scoreSum: 0, histogram: [count] + Array(repeating: 0, count: 19))
        }
    }

    struct Region: Encodable, Equatable, Sendable {
        let continent: String
        let country: String?
        let count: Int
        let quality: Quality
    }

    struct RecentBlock: Encodable, Equatable, Sendable {
        let height: Int
        let continent: String
    }

    struct ReserveKeys: Encodable, Equatable, Sendable {
        let standby: Int
        let seated: Int
    }

    struct Reading: Equatable, Sendable {
        let presence: LiveGlobePresence?
        let state: LiveGlobePresenceState
    }

    let total: Int
    let roles: [String: Int]
    let versions: [String: Int]
    let reserveKeys: ReserveKeys
    let regions: [Region]
    let recentBlocks: [RecentBlock]
    /// True only for a validated v3 summary, never inferred from cohort counts.
    let hasQualityEvidence: Bool
    let aggregateJSON: String

    static let continents = ["africa", "asia", "europe", "north_america", "south_america",
                             "oceania", "antarctica", "unknown"]
    private static let roleNames = ["validator", "wallet", "candidate", "follower"]
    private static let safeInteger = 9_007_199_254_740_991
    private static let maxRegions = 1_024
    private static let maxVersions = 128
    private static let maxNodes = 4_096
    private static let release = try! NSRegularExpression(pattern:
        #"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"#)

    private struct Role: Encodable { let count: Int }
    private struct Payload: Encodable {
        let schemaVersion = 3
        let scope = "node"
        let qualityVersion = 1
        let total: Int
        let roles: [String: Role]
        let versions: [String: Int]
        let reserveKeys: ReserveKeys
        let regions: [Region]
        let recentBlocks: [RecentBlock]

        enum CodingKeys: String, CodingKey {
            case schemaVersion = "schema_version", qualityVersion = "quality_version"
            case reserveKeys = "reserve_keys", recentBlocks = "recent_blocks"
            case scope, total, roles, versions, regions
        }
    }

    private init(total: Int, roles: [String: Int], versions: [String: Int],
                 reserveKeys: ReserveKeys, regions: [Region], recentBlocks: [RecentBlock],
                 hasQualityEvidence: Bool, aggregateJSON: String? = nil) throws {
        self.total = total
        self.roles = roles
        self.versions = versions
        self.reserveKeys = reserveKeys
        self.regions = regions
        self.recentBlocks = recentBlocks
        self.hasQualityEvidence = hasQualityEvidence
        if let aggregateJSON {
            self.aggregateJSON = aggregateJSON
            return
        }
        let payload = Payload(total: total, roles: roles.mapValues { Role(count: $0) }, versions: versions,
                              reserveKeys: reserveKeys, regions: regions, recentBlocks: recentBlocks)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        self.aggregateJSON = String(decoding: try encoder.encode(payload), as: UTF8.self)
    }

    /// Reject malformed or expired local replies and discard every unlisted field.
    static func parse(_ value: Any?, now: TimeInterval = Date().timeIntervalSince1970) -> Self? {
        do {
            let source = try record(value)
            if source["schema_version"] != nil { return try aggregate(source) }
            return try cohort(source, now: now)
        } catch { return nil }
    }

    /// Failed reads retain the last aggregate with an explicit stale state.
    /// A node that has never answered cannot look like an empty network.
    static func read(_ value: Any?, retaining previous: Self?,
                     now: TimeInterval = Date().timeIntervalSince1970) -> Reading {
        if let sample = LivePresence.parse(value, now: now), sample.total == nil {
            return Reading(presence: nil, state: .withheld)
        }
        if let presence = parse(value, now: now) {
            return Reading(presence: presence, state: .ready)
        }
        return Reading(presence: previous, state: previous == nil ? .unavailable : .stale)
    }

    private enum Invalid: Error { case presence }

    private static func record(_ value: Any?) throws -> [String: Any] {
        guard let result = value as? [String: Any],
              !["__proto__", "constructor", "prototype"].contains(where: { result[$0] != nil })
        else { throw Invalid.presence }
        return result
    }

    private static func count(_ value: Any?) throws -> Int {
        guard let number = value as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID() else {
            throw Invalid.presence
        }
        let value = number.doubleValue
        guard value.isFinite, value >= 0, value <= Double(safeInteger), value.rounded(.towardZero) == value else {
            throw Invalid.presence
        }
        return Int(value)
    }

    private static func add(_ left: Int, _ right: Int) throws -> Int {
        guard left >= 0, right >= 0, left <= safeInteger, right <= safeInteger - left else {
            throw Invalid.presence
        }
        return left + right
    }

    private static func continent(_ value: Any?) throws -> String {
        guard let code = value as? String, continents.contains(code) else { throw Invalid.presence }
        return code
    }

    private static func country(_ value: Any?) throws -> String? {
        if value == nil || value is NSNull { return nil }
        guard let code = value as? String, PresenceCountry.codes.contains(code) else { throw Invalid.presence }
        return code
    }

    private static func validRelease(_ value: String) -> Bool {
        let range = NSRange(value.startIndex..<value.endIndex, in: value)
        return value.utf8.count <= 64 && release.firstMatch(in: value, range: range)?.range == range
    }

    private static func quality(_ value: Any?, count size: Int) throws -> Quality {
        let source = try record(value)
        let scoreSum = try count(source["score_sum"])
        guard let entries = source["histogram"] as? [Any], entries.count == 20 else { throw Invalid.presence }
        let histogram = try entries.map { try count($0) }
        var population = 0, minimum = 0, maximum = 0
        for (bin, quantity) in histogram.enumerated() {
            population = try add(population, quantity)
            let lower = bin * 50_000
            let upper = bin == 19 ? 1_000_000 : (bin + 1) * 50_000 - 1
            // Bounds can exceed safe JSON integers. Compare against the safe
            // score sum before multiplying, so large valid counts cannot overflow.
            if lower > 0 {
                guard quantity <= (scoreSum - minimum) / lower else { throw Invalid.presence }
                minimum += quantity * lower
            }
            if maximum < scoreSum {
                let remaining = scoreSum - maximum
                if quantity > remaining / upper {
                    maximum = scoreSum
                } else {
                    maximum += quantity * upper
                }
            }
        }
        guard population == size, scoreSum >= minimum, scoreSum <= maximum else { throw Invalid.presence }
        return Quality(scoreSum: scoreSum, histogram: histogram)
    }

    private static func aggregate(_ source: [String: Any]) throws -> Self {
        guard try count(source["schema_version"]) == 3, source["scope"] as? String == "node",
              try count(source["quality_version"]) == 1 else { throw Invalid.presence }
        let total = try count(source["total"])
        let roleSource = try record(source["roles"])
        var roles: [String: Int] = [:]
        for role in roleNames {
            let size = try count(record(roleSource[role])["count"])
            guard size <= total else { throw Invalid.presence }
            roles[role] = size
        }
        let reserveSource = try record(source["reserve_keys"])
        let reserveKeys = ReserveKeys(standby: try count(reserveSource["standby"]), seated: try count(reserveSource["seated"]))
        _ = try add(reserveKeys.standby, reserveKeys.seated)
        let versionSource = try record(source["versions"])
        guard versionSource.count <= maxVersions else { throw Invalid.presence }
        var versions: [String: Int] = [:], versionTotal = 0
        for version in versionSource.keys {
            guard validRelease(version) else { throw Invalid.presence }
            let size = try count(versionSource[version])
            versionTotal = try add(versionTotal, size)
            versions[version] = size
        }
        guard versionTotal == total,
              let regionSource = source["regions"] as? [Any], regionSource.count <= maxRegions else { throw Invalid.presence }
        var regions: [Region] = [], regionTotal = 0
        for value in regionSource {
            let source = try record(value)
            let size = try count(source["count"])
            regionTotal = try add(regionTotal, size)
            regions.append(Region(continent: try continent(source["continent"]), country: try country(source["country"]),
                                  count: size, quality: try quality(source["quality"], count: size)))
        }
        guard regionTotal == total else { throw Invalid.presence }
        var recentBlocks: [RecentBlock] = []
        if let value = source["recent_blocks"] {
            guard let entries = value as? [Any], entries.count <= 8 else { throw Invalid.presence }
            for value in entries {
                let source = try record(value)
                recentBlocks.append(RecentBlock(height: try count(source["height"]), continent: try continent(source["continent"])))
            }
        }
        return try Self(total: total, roles: roles, versions: versions, reserveKeys: reserveKeys,
                        regions: fold(regions), recentBlocks: recentBlocks, hasQualityEvidence: true)
    }

    private static func cohort(_ source: [String: Any], now: TimeInterval) throws -> Self {
        guard now.isFinite, now > 0, let sample = LivePresence.parse(source, now: now),
              let total = sample.total, total <= maxNodes else { throw Invalid.presence }
        let regions = PresenceRegion.observationCodes.compactMap { name -> Region? in
            guard let size = sample.byRegion[name] else { return nil }
            return Region(continent: name, country: nil, count: size, quality: .unmeasured(count: size))
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let aggregateJSON = String(decoding: try encoder.encode(sample), as: UTF8.self)
        return try Self(total: total, roles: sample.byRole, versions: sample.byVersion,
                        reserveKeys: ReserveKeys(standby: 0, seated: 0), regions: regions, recentBlocks: [],
                        hasQualityEvidence: false, aggregateJSON: aggregateJSON)
    }

    /// Merge duplicate country buckets within each relay continent before k=3.
    private static func fold(_ source: [Region]) throws -> [Region] {
        var buckets: [String: [String: Region]] = [:]
        func merged(_ left: Region, _ right: Region, country: String?) throws -> Region {
            let bins = try zip(left.quality.histogram, right.quality.histogram).map { pair in try add(pair.0, pair.1) }
            return Region(continent: left.continent, country: country, count: try add(left.count, right.count),
                          quality: Quality(scoreSum: try add(left.quality.scoreSum, right.quality.scoreSum), histogram: bins))
        }
        for region in source {
            let key = region.country ?? ""
            if let previous = buckets[region.continent]?[key] {
                buckets[region.continent, default: [:]][key] = try merged(previous, region, country: region.country)
            } else { buckets[region.continent, default: [:]][key] = region }
        }
        var result: [Region] = []
        for continent in continents {
            let countries = buckets[continent, default: [:]]
            var remainder = countries[""] ?? Region(continent: continent, country: nil, count: 0, quality: .unmeasured(count: 0))
            var visible: [Region] = []
            for code in countries.keys.filter({ !$0.isEmpty }).sorted() {
                let region = countries[code]!
                if region.count < 3 { remainder = try merged(remainder, region, country: nil) }
                else { visible.append(region) }
            }
            if remainder.count > 0 { result.append(remainder) }
            result.append(contentsOf: visible)
        }
        return result
    }
}
