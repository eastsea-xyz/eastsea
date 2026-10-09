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
              Set(sample.byRegion.keys).isSubset(of: Set(PresenceRegion.observationCodes)),
              sample.byVersion.keys.allSatisfy({ $0 == "unknown" }),
              countsMatch(sample.byRole, total: sample.total),
              countsMatch(sample.byVersion, total: sample.total),
              countsMatch(sample.byRegion, total: sample.total) else { return nil }
        return sample
    }

    private static func countsMatch(_ counts: [String: Int], total: Int?) -> Bool {
        guard let total else { return counts.isEmpty }
        guard total >= 3, total <= 4096 else { return false }
        var remaining = total
        for count in counts.values {
            guard count >= 3, count <= remaining else { return false }
            remaining -= count
        }
        return remaining == 0
    }

    /// A single line, with regions in a stable order instead of whichever
    /// order the JSON dictionary happened to have on the latest poll.
    var line: String {
        guard let total else { return String(localized: "Counts withheld for privacy") }
        let count = String(localized: "\(Int64(total)) node observations")
        let regions = PresenceRegion.observationCodes
        let parts = regions.compactMap { region -> String? in
            guard let count = byRegion[region], count > 0 else { return nil }
            return String(localized: "\(Self.regionLabel(region)) \(Int64(count))")
        }
        return ([count] + parts).joined(separator: " · ")
    }

    private static func regionLabel(_ region: String) -> String {
        if PresenceRegion.codes.contains(region) { return PresenceRegion.label(region) }
        let label: String
        switch region {
        case "asia": label = String(localized: "Asia")
        case "europe": label = String(localized: "Europe")
        case "north_america": label = String(localized: "North America")
        case "south_america": label = String(localized: "South America")
        case "africa": label = String(localized: "Africa")
        case "oceania": label = String(localized: "Oceania")
        case "antarctica": label = String(localized: "Antarctica")
        case "world": return String(localized: "All regions")
        default: return String(localized: "Unknown region")
        }
        return String(localized: "\(label) (legacy broad region)")
    }
}

/// Default geography uses the official UN M49 Sub-region column, not its
/// Intermediate Region column. Unsupported ISO/CLDR areas stay unknown.
/// Source: https://unstats.un.org/unsd/methodology/m49/overview/
enum PresenceRegion {
    static let codes = ["015", "202", "021", "419", "143", "030", "035", "034", "145",
                        "151", "154", "039", "155", "053", "054", "057", "061"]
    static let observationCodes = codes + ["asia", "europe", "north_america", "south_america",
                                           "africa", "oceania", "antarctica", "unknown", "world"]
    private static let countryGroups = [
        "015": "DZ EG EH LY MA SD TN",
        "202": "AO BF BI BJ BW CD CF CG CI CM CV DJ ER ET GA GH GM GN GQ GW IO KE KM LR LS MG ML MR MU MW MZ NA NE NG RE RW SC SH SL SN SO SS ST SZ TD TF TG TZ UG YT ZA ZM ZW",
        "419": "AG AI AR AW BB BL BO BQ BR BS BV BZ CL CO CR CU CW DM DO EC FK GD GF GP GS GT GY HN HT JM KN KY LC MF MQ MS MX NI PA PE PR PY SR SV SX TC TT UY VC VE VG VI",
        "021": "BM CA GL PM US",
        "143": "KG KZ TJ TM UZ",
        "030": "CN HK JP KP KR MN MO",
        "035": "BN ID KH LA MM MY PH SG TH TL VN",
        "034": "AF BD BT IN IR LK MV NP PK",
        "145": "AE AM AZ BH CY GE IL IQ JO KW LB OM PS QA SA SY TR YE",
        "151": "BG BY CZ HU MD PL RO RU SK UA",
        "154": "AX DK EE FI FO GB GG IE IM IS JE LT LV NO SE SJ",
        "039": "AD AL BA ES GI GR HR IT ME MK MT PT RS SI SM VA",
        "155": "AT BE CH DE FR LI LU MC NL",
        "053": "AU CC CX HM NF NZ",
        "054": "FJ NC PG SB VU",
        "057": "FM GU KI MH MP NR PW UM",
        "061": "AS CK NU PF PN TK TO TV WF WS",
    ]
    private static let countries = Dictionary(uniqueKeysWithValues: countryGroups.flatMap { code, names in
        names.split(separator: " ").map { (String($0), code) }
    })

    static func fromRegion(_ region: String?) -> String? {
        guard let region else { return nil }
        if codes.contains(region) { return region }
        return PresenceCountry.normalize(region).flatMap { countries[$0] }
    }

    static func label(_ code: String?) -> String {
        switch code {
        case "015": return String(localized: "Northern Africa")
        case "202": return String(localized: "Sub-Saharan Africa")
        case "021": return String(localized: "Northern America")
        case "419": return String(localized: "Latin America and the Caribbean")
        case "143": return String(localized: "Central Asia")
        case "030": return String(localized: "Eastern Asia")
        case "035": return String(localized: "South-eastern Asia")
        case "034": return String(localized: "Southern Asia")
        case "145": return String(localized: "Western Asia")
        case "151": return String(localized: "Eastern Europe")
        case "154": return String(localized: "Northern Europe")
        case "039": return String(localized: "Southern Europe")
        case "155": return String(localized: "Western Europe")
        case "053": return String(localized: "Australia and New Zealand")
        case "054": return String(localized: "Melanesia")
        case "057": return String(localized: "Micronesia")
        case "061": return String(localized: "Polynesia")
        default: return String(localized: "Unknown region")
        }
    }
}

/// Country sharing requires a saved affirmative answer, independently of the
/// default Mac-region sub-region. Legacy sharing toggles never grant consent.
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
            guard let value else { return .askBeforeSending }
            return (value as? String).flatMap(Self.init(rawValue:)) ?? .askBeforeSending
        }

        var initiallySelected: Bool { self == .defaultOn }
    }

    /// The distribution and missing-setting fallback ask before sharing.
    /// Legacy configuration never bypasses the screen's explicit Share action.
    static var mode: Mode { Mode.configured(Bundle.main.object(forInfoDictionaryKey: modeKey)) }

    enum Choice: String, Sendable {
        case share
        case decline
    }

    struct Preference: Equatable, Sendable {
        let choice: Choice?
        let country: String
        let defaultRegion: String?

        init(choice: String?, country: String, region: String? = Locale.current.region?.identifier) {
            self.choice = choice.flatMap(Choice.init(rawValue:))
            self.country = country
            self.defaultRegion = PresenceRegion.fromRegion(region)
        }

        var answered: Bool { choice != nil }
        var shared: String? { choice == .share ? PresenceCountry.normalize(country) : nil }
        var effectiveRegion: String? {
            if let shared { return PresenceRegion.fromRegion(shared) }
            return defaultRegion
        }
        var flags: [String] {
            (defaultRegion.map { ["--presence-region=\($0)"] } ?? [])
                + (shared.map { ["--presence-country=\($0)"] } ?? [])
        }
        /// A null revokes a running node's country preference, including a
        /// node attached from an old unattended marker. Never infer consent.
        var controlParams: [Any] { [shared.map { $0 as Any } ?? NSNull()] }
        var regionControlParams: [Any] { [defaultRegion.map { $0 as Any } ?? NSNull()] }
        var markerFields: [String: String] {
            ["presence_country_choice": choice?.rawValue ?? "", "presence_country": shared ?? "",
             "presence_region": defaultRegion ?? ""]
        }
    }

    static func preference(defaults: UserDefaults = .standard,
                           region: String? = Locale.current.region?.identifier) -> Preference {
        Preference(choice: defaults.string(forKey: choiceKey),
                   country: defaults.string(forKey: countryKey) ?? "", region: region)
    }

    static func suggestedCountry(region: String?) -> String {
        region.flatMap(normalize) ?? ""
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

}
