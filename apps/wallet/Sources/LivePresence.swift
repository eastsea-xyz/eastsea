import Foundation

/// An unverified cohort observation, independent of chain state. The public
/// protocol contains thresholded counts, never individual node records.
struct LivePresence: Codable, Equatable, Sendable {
    let schema: Int
    let available: Bool
    let total: Int?
    let byRole: [String: Int]
    let byVersion: [String: Int]
    let byRegion: [String: Int]
    let ttlSeconds: Int
    let minimumBucketSize: Int
    let observedAt: TimeInterval
    let scope: String

    enum CodingKeys: String, CodingKey {
        case byRole = "by_role", byVersion = "by_version", byRegion = "by_region"
        case ttlSeconds = "ttl_seconds", observedAt = "observed_at"
        case minimumBucketSize = "minimum_bucket_size"
        case schema, available, total, scope
    }

    /// Old nodes, unavailable observers and expired samples leave the line
    /// hidden. A failed read must never look like a network with zero Macs.
    static func parse(_ value: Any?, now: TimeInterval = Date().timeIntervalSince1970) -> Self? {
        guard let value = value as? [String: Any],
              Set(value.keys) == Set(["schema", "available", "total", "by_role", "by_version", "by_region",
                                      "ttl_seconds", "minimum_bucket_size", "observed_at", "scope"]),
              JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(withJSONObject: value),
              let sample = try? JSONDecoder().decode(Self.self, from: data),
              sample.schema == 2, sample.available, sample.ttlSeconds == 600,
              sample.minimumBucketSize == 3, sample.scope == "unverified cohort observation",
              sample.observedAt.truncatingRemainder(dividingBy: 600) == 0,
              sample.observedAt > 0, sample.observedAt <= now + 60,
              now - sample.observedAt < Double(sample.ttlSeconds),
              Set(sample.byRole.keys).isSubset(of: ["validator", "candidate", "follower", "unknown", "other"]),
              Set(sample.byRegion.keys).isSubset(of: ["asia", "europe", "north_america", "south_america", "africa", "oceania", "unknown", "world"]),
              sample.byVersion.keys.allSatisfy({ $0 == "unknown" }),
              countsMatch(sample.byRole, total: sample.total),
              countsMatch(sample.byVersion, total: sample.total),
              countsMatch(sample.byRegion, total: sample.total) else { return nil }
        return sample
    }

    private static func countsMatch(_ counts: [String: Int], total: Int?) -> Bool {
        guard let total else { return counts.isEmpty }
        guard total >= 3 else { return false }
        var remaining = total
        for count in counts.values {
            guard count >= 3, count <= remaining else { return false }
            remaining -= count
        }
        return remaining == 0
    }

    /// A single line, with continents in a stable order instead of whichever
    /// order the JSON dictionary happened to have on the latest poll.
    var line: String {
        guard let total else { return String(localized: "Counts withheld for privacy") }
        let count = String(localized: "\(Int64(total)) node observations")
        let regions = ["asia", "europe", "north_america", "south_america", "africa", "oceania", "unknown", "world"]
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
        case "world": String(localized: "All regions")
        default: String(localized: "Unknown region")
        }
    }
}

/// Country comes from the Mac region setting by default, separate from its relay continent.
/// Codes match the node's strict ISO 3166-1 alpha-2 validator; CLDR's extra
/// region codes must never produce an argument that prevents node startup.
enum PresenceCountry {
    /// A legacy sharing toggle is not an answer to the new first-launch screen.
    static let choiceKey = "presenceCountryChoiceV1"
    static let countryKey = "presenceCountryCode"
    static let modeKey = "PresenceCountrySharingMode"

    enum Mode: String, CaseIterable, Sendable {
        case defaultOn = "default-on"
        case askBeforeSending = "ask-before-sending"

        static func configured(_ value: Any?) -> Self {
            guard let value else { return .defaultOn }
            return (value as? String).flatMap(Self.init(rawValue:)) ?? .askBeforeSending
        }

        var initiallySelected: Bool { self == .defaultOn }
    }

    /// Changing this one Info.plist setting selects the founder's policy;
    /// neither policy grants permission before a person answers the screen.
    static var mode: Mode { Mode.configured(Bundle.main.object(forInfoDictionaryKey: modeKey)) }

    enum Choice: String, Sendable {
        case share
        case decline
    }

    struct Preference: Equatable, Sendable {
        let choice: Choice?
        let country: String

        init(choice: String?, country: String) {
            self.choice = choice.flatMap(Choice.init(rawValue:))
            self.country = country
        }

        var answered: Bool { choice != nil }
        var shared: String? { choice == .share ? PresenceCountry.normalize(country) : nil }
        var flags: [String] { shared.map { ["--presence-country=\($0)"] } ?? [] }
        /// A null revokes a running node's country preference, including a
        /// node attached from an old unattended marker. Never infer consent.
        var controlParams: [Any] { [shared.map { $0 as Any } ?? NSNull()] }
        var markerFields: [String: String] {
            ["presence_country_choice": choice?.rawValue ?? "", "presence_country": shared ?? ""]
        }
    }

    static func preference(defaults: UserDefaults = .standard) -> Preference {
        Preference(choice: defaults.string(forKey: choiceKey),
                   country: defaults.string(forKey: countryKey) ?? "")
    }

    static func suggestedCountry(region: String?) -> String {
        region.flatMap(normalize) ?? ""
    }

    // Keep in sync with crates/net/src/presence.rs::normalize_country.
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

}
