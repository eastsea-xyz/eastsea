import Foundation

/// A node's view of recent, signed presence pings. This is an observation,
/// independent of block signing and balances, and is never saved as chain state.
struct LivePresence: Decodable, Equatable, Sendable {
    let schema: Int
    let available: Bool
    let total: Int
    let byRole: [String: Int]
    let byVersion: [String: Int]
    let byRegion: [String: Int]
    let byCountry: [String: Int]
    let nodes: [Node]
    let ttlSeconds: Int
    let observedAt: TimeInterval
    let observer: String
    let scope: String

    struct Node: Decodable, Equatable, Sendable {
        let nodeID: String
        let role: String
        let version: String
        let timestamp: TimeInterval
        let lastSeen: TimeInterval
        let region: String
        let country: String?

        enum CodingKeys: String, CodingKey {
            case nodeID = "node_id", lastSeen = "last_seen"
            case role, version, timestamp, region, country
        }
    }

    enum CodingKeys: String, CodingKey {
        case byRole = "by_role", byVersion = "by_version", byRegion = "by_region", byCountry = "by_country"
        case ttlSeconds = "ttl_seconds", observedAt = "observed_at"
        case schema, available, total, nodes, observer, scope
    }

    /// Old nodes, unavailable observers and expired samples leave the line
    /// hidden. A failed read must never look like a network with zero Macs.
    static func parse(_ value: Any?, now: TimeInterval = Date().timeIntervalSince1970) -> Self? {
        guard let value, JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(withJSONObject: value),
              let sample = try? JSONDecoder().decode(Self.self, from: data),
              sample.schema == 1, sample.available, sample.ttlSeconds == 180,
              sample.observedAt > 0, sample.observedAt <= now + 60,
              now - sample.observedAt < Double(sample.ttlSeconds),
              sample.total >= 0, sample.nodes.count == sample.total,
              Set(sample.nodes.map(\.nodeID)).count == sample.total,
              sample.nodes.allSatisfy({ !$0.nodeID.isEmpty }),
              countsMatch(sample.byRole, total: sample.total),
              countsMatch(sample.byVersion, total: sample.total),
              countsMatch(sample.byRegion, total: sample.total),
              countsMatch(sample.byCountry, total: sample.total, exact: false) else { return nil }
        return sample
    }

    private static func countsMatch(_ counts: [String: Int], total: Int, exact: Bool = true) -> Bool {
        var remaining = total
        for count in counts.values {
            guard count >= 0, count <= remaining else { return false }
            remaining -= count
        }
        return !exact || remaining == 0
    }

    /// A single line, with continents in a stable order instead of whichever
    /// order the JSON dictionary happened to have on the latest poll.
    var line: String {
        let count = String(localized: "\(Int64(total)) Macs online now")
        let regions = ["asia", "europe", "north_america", "south_america", "africa", "oceania", "unknown"]
        let parts = regions.compactMap { region -> String? in
            guard let count = byRegion[region], count > 0 else { return nil }
            return String(localized: "\(Self.regionLabel(region)) \(Int64(count))")
        }
        return ([count] + parts).joined(separator: " · ")
    }

    private static func regionLabel(_ region: String) -> String {
        switch region {
        case "asia": String(localized: "Asia")
        case "europe": String(localized: "Europe")
        case "north_america": String(localized: "North America")
        case "south_america": String(localized: "South America")
        case "africa": String(localized: "Africa")
        case "oceania": String(localized: "Oceania")
        default: String(localized: "Unknown region")
        }
    }
}

/// Country comes from the Mac region setting by default, separate from its relay continent.
/// Codes match the node's strict ISO 3166-1 alpha-2 validator; CLDR's extra
/// region codes must never produce an argument that prevents node startup.
enum PresenceCountry {
    static let sharingKey = "presenceShareCountry"
    static let noticeSeenKey = "presenceCountryNoticeSeen"

    static var regionCode: String { normalize(Locale.current.region?.identifier ?? "") ?? "" }

    static func enabled(defaults: UserDefaults = .standard) -> Bool {
        defaults.object(forKey: sharingKey) as? Bool ?? true
    }

    /// Resolve against the current Mac region on every read. Older wallets
    /// saved a manual country code; that value is deliberately no longer used.
    static func shared(defaults: UserDefaults = .standard,
                       region: String? = Locale.current.region?.identifier) -> String? {
        shared(sharing: enabled(defaults: defaults), country: region ?? "")
    }

    static func flags(defaults: UserDefaults = .standard,
                      region: String? = Locale.current.region?.identifier) -> [String] {
        flags(sharing: enabled(defaults: defaults), country: region ?? "")
    }

    /// Do not describe a country as shown when sharing is off or the Mac's
    /// region is not a valid node country. Queuing a sheet never marks it seen.
    static func shouldShowNotice(defaults: UserDefaults = .standard,
                                 region: String? = Locale.current.region?.identifier) -> Bool {
        !defaults.bool(forKey: noticeSeenKey) && shared(defaults: defaults, region: region) != nil
    }

    static func markNoticeSeen(defaults: UserDefaults = .standard) {
        defaults.set(true, forKey: noticeSeenKey)
    }

    // Keep in sync with crates/node/src/presence.rs::validate_country.
    static let codes = """
        AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ
        CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET
        FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU
        ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY
        MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ
        OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ
        TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW
        """.split(whereSeparator: { $0.isWhitespace }).map(String.init)

    static func normalize(_ code: String) -> String? {
        let value = code.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        return codes.contains(value) ? value : nil
    }

    static func shared(sharing: Bool, country: String) -> String? {
        sharing ? normalize(country) : nil
    }

    static func flags(sharing: Bool, country: String) -> [String] {
        shared(sharing: sharing, country: country).map { ["--presence-country=\($0)"] } ?? []
    }
}
