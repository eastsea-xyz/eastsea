import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

func argv(_ preference: PresenceCountry.Preference) -> [String] {
    UnattendedDecision.nodeArgv(dataDir: "./tmp/country-node", rpcPort: 18545, p2pPort: 19101,
                               networkPath: nil, proverFlags: [], presenceFlags: preference.flags)
}

func paramsJSON(_ preference: PresenceCountry.Preference) -> String {
    let data = try! JSONSerialization.data(withJSONObject: preference.controlParams)
    return String(data: data, encoding: .utf8)!
}

func regionParamsJSON(_ preference: PresenceCountry.Preference) -> String {
    let data = try! JSONSerialization.data(withJSONObject: preference.regionControlParams)
    return String(data: data, encoding: .utf8)!
}

func restored(_ values: [String: Any]) -> PresenceCountry.Preference {
    // This exercises the production UserDefaults reader using an in-memory
    // domain. Tests never modify the person's app or wallet preferences.
    let defaults = UserDefaults()
    defaults.setVolatileDomain(values, forName: UserDefaults.argumentDomain)
    return PresenceCountry.preference(defaults: defaults, region: "KR")
}

check(PresenceCountry.Mode.configured(nil) == .askBeforeSending, "missing policy asks before country sharing")
check(PresenceCountry.Mode.configured("default-on") == .defaultOn, "one setting selects default-on")
check(PresenceCountry.Mode.configured("ask-before-sending") == .askBeforeSending, "one setting selects explicit choice")
for invalid in ["unknown", "", true, 1, ["default-on"]] as [Any] {
    check(PresenceCountry.Mode.configured(invalid) == .askBeforeSending, "malformed mode fails closed to explicit choice")
}

for mode in PresenceCountry.Mode.allCases {
    for country in ["", "KR", "US"] {
        let pending = restored([PresenceCountry.countryKey: country, "presenceShareCountry": true])
        check(pending.defaultRegion == "030" && pending.effectiveRegion == "030"
              && pending.flags == ["--presence-region=030"] && regionParamsJSON(pending) == "[\"030\"]",
              "the default Mac sub-region is independent of an unanswered country choice")
        check(!pending.answered && pending.shared == nil, "\(mode) ignores legacy opt-in before the new screen")
        check(paramsJSON(pending) == "[null]", "\(mode) sends only a local revocation, never a country before answer")
        for role in ["follower", "candidate", "validator"] {
            check(!argv(pending).contains { $0.hasPrefix("--presence-country") },
                  "\(mode) \(role) app/supervisor/unattended argv is country-free before answer")
        }
        check(pending.markerFields["presence_country_choice"] == "" && pending.markerFields["presence_country"] == "",
              "\(mode) pending unattended marker carries no country or permission")
    }

    let chosen = restored([PresenceCountry.choiceKey: "share", PresenceCountry.countryKey: "kr"])
    check(chosen.answered && chosen.shared == "KR", "\(mode) affirmative screen answer permits only normalized country")
    check(paramsJSON(chosen) == "[\"KR\"]", "\(mode) actual local RPC params contain country only after answer")
    check(argv(chosen).last == "--presence-country=KR", "\(mode) app and saved unattended argv share the affirmative choice")
    check(chosen.markerFields == ["presence_country_choice": "share", "presence_country": "KR", "presence_region": "030"],
          "\(mode) marker records the exact answered country for the wrapper")

    let declined = restored([PresenceCountry.choiceKey: "decline", PresenceCountry.countryKey: "KR", "presenceShareCountry": true])
    check(declined.defaultRegion == "030" && declined.effectiveRegion == "030"
          && declined.flags == ["--presence-region=030"], "decline preserves the default Mac sub-region")
    check(declined.answered && declined.shared == nil && paramsJSON(declined) == "[null]",
          "\(mode) decline and later revocation remove country immediately")
    check(argv(declined) == argv(restored([:])), "\(mode) declining changes no ordinary node launch arguments")
    check(declined.markerFields["presence_country"] == "", "\(mode) saved unattended marker cannot restore an opted-out country")

    // The persisted fields survive an actual property-list round trip; a
    // founder mode change does not reverse the person's saved choice.
    for answer in ["share", "decline"] {
        let values = [PresenceCountry.choiceKey: answer, PresenceCountry.countryKey: "KR"]
        let bytes = try! PropertyListSerialization.data(fromPropertyList: values, format: .xml, options: 0)
        let saved = try! PropertyListSerialization.propertyList(from: bytes, options: [], format: nil) as! [String: Any]
        check(restored(saved) == restored(values), "\(mode) saved \(answer) choice survives a background restart")
    }
}

for (country, group) in [("KR", "030"), ("US", "021"), ("RU", "151"), ("TR", "145"),
                         ("CY", "145"), ("KZ", "143"), ("MX", "419"), ("ZA", "202"), ("AU", "053")] {
    check(PresenceRegion.fromRegion(country) == group, "official M49 parent is used for \(country)")
    let preference = PresenceCountry.Preference(choice: "share", country: country, region: "KR")
    check(preference.defaultRegion == "030" && preference.effectiveRegion == group,
          "explicit selected country uses its canonical parent without losing the independent default")
    check(preference.flags == ["--presence-region=030", "--presence-country=\(country)"],
          "country selection does not replace the default region argument")
}
check(PresenceRegion.codes.count == 17 && Set(PresenceRegion.codes).count == 17,
      "only the 17 official sub-region codes are accepted")
for invalid in [nil, "EU", "AQ", "TW", "014", "029", "999", "KR --archive"] as [String?] {
    check(PresenceRegion.fromRegion(invalid) == nil, "unlisted areas and intermediate groups are unknown")
}
let unavailableRegion = PresenceCountry.Preference(choice: nil, country: "KR", region: nil)
check(unavailableRegion.flags.isEmpty && regionParamsJSON(unavailableRegion) == "[null]",
      "an unavailable Mac region is never invented")
let unmappedChoice = PresenceCountry.Preference(choice: "share", country: "TW", region: "KR")
check(unmappedChoice.effectiveRegion == nil && unmappedChoice.defaultRegion == "030",
      "an explicit unmapped country is unknown without discarding the later opt-out default")

for unknown in ["", "true", "yes", "default-on", "future-choice"] {
    let preference = restored([PresenceCountry.choiceKey: unknown, PresenceCountry.countryKey: "KR"])
    check(!preference.answered && argv(preference).last != "--presence-country=KR" && paramsJSON(preference) == "[null]",
          "invalid persisted choice asks again without silent opt-in")
}
check(PresenceCountry.suggestedCountry(region: "kr") == "KR", "Region setting can prefill the view-local picker")
for invalid in [nil, "EU", "UK", "AA"] as [String?] {
    check(PresenceCountry.suggestedCountry(region: invalid).isEmpty, "unsupported Region settings never produce a startup flag")
}
check(PresenceCountry.codes.count == 249 && Set(PresenceCountry.codes).count == 249,
      "the picker offers the 249 unique ISO countries accepted by the node")
for country in ["AC", "CP", "DG", "EA", "IC", "TA", "UK", "", "Asia", "AA", "123", "KR --archive", "../KR", "ZZ", "EU"] {
    let preference = PresenceCountry.Preference(choice: "share", country: country, region: "KR")
    check(preference.flags == ["--presence-region=030"] && paramsJSON(preference) == "[null]", "invalid country reaches neither CLI nor RPC: \(country)")
}
for country in ["AQ", "BV", "GB", "HM", "SH", "SJ", "UM"] {
    let preference = PresenceCountry.Preference(choice: "share", country: country, region: "KR")
    check(preference.flags == ["--presence-region=030", "--presence-country=\(country)"], "valid territory remains selectable after answer")
}

let walletRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .deletingLastPathComponent().deletingLastPathComponent()
let source = walletRoot.appendingPathComponent("Sources")
let controller = try String(contentsOf: source.appendingPathComponent("NodeController.swift"))
let unattended = try String(contentsOf: source.appendingPathComponent("UnattendedDaemon.swift"))
let onboarding = try String(contentsOf: source.appendingPathComponent("Onboarding.swift"))
let content = try String(contentsOf: source.appendingPathComponent("ContentView.swift"))
check(controller.contains("presenceFlags: presenceCountryPreference.flags") && controller.contains("let params = preference.controlParams"),
      "real launch and local RPC call sites use the tested consent-aware payload builders")
check(controller.contains("method: \"aether_setPresenceRegion\", params: regionParams")
      && controller.contains("let regionParams = preference.regionControlParams")
      && controller.contains("latest != preference"), "the real region and country controls share the serialized update gate")
check(unattended.contains("presenceFlags: PresenceCountry.preference().flags") && unattended.contains("PresenceCountry.preference().markerFields"),
      "real saved unattended argv and marker use the same tested consent gate")
check(!controller.contains("PresenceCountry.flags(sharing:") && !unattended.contains("PresenceCountry.flags(sharing:"),
      "production paths cannot bypass the persisted screen answer with a sharing boolean")
check(onboarding.contains("Button(\"Share country\") { answer(sharing: true) }") && onboarding.contains("node.answerPresenceCountry(sharing:")
      && !onboarding.contains("_sharing = State"), "the real screen requires an affirmative country action without preselection")
check(content.contains("return node.needsCountryNotice") && content.contains("CountryNoticeSheet(country: node.presenceCountryCode)"),
      "new and existing users both see the country screen when no new answer exists")

let plistBytes = try Data(contentsOf: walletRoot.appendingPathComponent("Info-mac.plist"))
let info = try PropertyListSerialization.propertyList(from: plistBytes, options: [], format: nil) as! [String: Any]
check(info[PresenceCountry.modeKey] as? String == "ask-before-sending", "distribution asks before country sharing")

let catalogData = try Data(contentsOf: walletRoot.appendingPathComponent("Resources/Localizable.xcstrings"))
let catalog = try JSONSerialization.jsonObject(with: catalogData) as! [String: Any]
let strings = catalog["strings"] as! [String: [String: Any]]
let keys = ["Country sharing", "Choose country sharing…", "Share this Mac's country", "Country", "Choose a country",
            "Your Mac's Region setting chooses a UN M49 sub-region by default. Country sharing is off until you choose Share country.",
            "Choose whether to share a country. Declining keeps all wallet and node features available.",
            "Your chosen country stays on this Mac and can select its M49 sub-region. Public presence publishes only thresholded regional observations, not country codes or individual Macs. Turning country sharing off restores the default sub-region; already received aggregates may remain.",
            "The default UN M49 sub-region follows this Mac's Region setting independently of country sharing. Country sharing requires an explicit choice and can select a different sub-region. Public presence does not publish country codes. Turning it off keeps the default sub-region. Peers and relays still see connection IP addresses.",
            "Default sub-region", "This Mac's local sub-region", "Selected country",
            "Local preference only; this Mac is not added to published counts.", "Don't share country", "Share country"]
for language in ["en", "ko", "ja", "zh-Hans", "zh-Hant"] {
    for key in keys {
        let localizations = strings[key]?["localizations"] as? [String: [String: Any]]
        let unit = localizations?[language]?["stringUnit"] as? [String: String]
        check(unit?["state"] == "translated" && unit?["value"]?.isEmpty == false,
              "\(language) translates \(key)")
    }
}
print("ok")
