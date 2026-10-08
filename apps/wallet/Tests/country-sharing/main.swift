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

func restored(_ values: [String: Any]) -> PresenceCountry.Preference {
    // This exercises the production UserDefaults reader using an in-memory
    // domain. Tests never modify the person's app or wallet preferences.
    let defaults = UserDefaults()
    defaults.setVolatileDomain(values, forName: UserDefaults.argumentDomain)
    return PresenceCountry.preference(defaults: defaults)
}

check(PresenceCountry.Mode.configured(nil) == .defaultOn, "current founder default is a preselected notice")
check(PresenceCountry.Mode.configured("default-on") == .defaultOn, "one setting selects default-on")
check(PresenceCountry.Mode.configured("ask-before-sending") == .askBeforeSending, "one setting selects explicit choice")
for invalid in ["unknown", "", true, 1, ["default-on"]] as [Any] {
    check(PresenceCountry.Mode.configured(invalid) == .askBeforeSending, "malformed mode fails closed to explicit choice")
}

for mode in PresenceCountry.Mode.allCases {
    check(mode.initiallySelected == (mode == .defaultOn), "only the notice mode preselects its view-local switch")
    for country in ["", "KR", "US"] {
        let pending = restored([PresenceCountry.countryKey: country, "presenceShareCountry": true])
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
    check(chosen.markerFields == ["presence_country_choice": "share", "presence_country": "KR"],
          "\(mode) marker records the exact answered country for the wrapper")

    let declined = restored([PresenceCountry.choiceKey: "decline", PresenceCountry.countryKey: "KR", "presenceShareCountry": true])
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
    let preference = PresenceCountry.Preference(choice: "share", country: country)
    check(preference.flags.isEmpty && paramsJSON(preference) == "[null]", "invalid country reaches neither CLI nor RPC: \(country)")
}
for country in ["AQ", "BV", "GB", "HM", "SH", "SJ", "UM"] {
    let preference = PresenceCountry.Preference(choice: "share", country: country)
    check(preference.flags == ["--presence-country=\(country)"], "valid territory remains selectable after answer")
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
check(unattended.contains("presenceFlags: PresenceCountry.preference().flags") && unattended.contains("PresenceCountry.preference().markerFields"),
      "real saved unattended argv and marker use the same tested consent gate")
check(!controller.contains("PresenceCountry.flags(sharing:") && !unattended.contains("PresenceCountry.flags(sharing:"),
      "production paths cannot bypass the persisted screen answer with a sharing boolean")
check(onboarding.contains("_sharing = State(initialValue: mode.initiallySelected)") && onboarding.contains("node.answerPresenceCountry(sharing:"),
      "founder default stays view-local until the actual screen action")
check(content.contains("return node.needsCountryNotice") && content.contains("CountryNoticeSheet(country: node.presenceCountryCode)"),
      "new and existing users both see the country screen when no new answer exists")

let plistBytes = try Data(contentsOf: walletRoot.appendingPathComponent("Info-mac.plist"))
let info = try PropertyListSerialization.propertyList(from: plistBytes, options: [], format: nil) as! [String: Any]
check(info[PresenceCountry.modeKey] as? String == "default-on", "distribution currently chooses the default-on notice without Swift edits")

let catalogData = try Data(contentsOf: walletRoot.appendingPathComponent("Resources/Localizable.xcstrings"))
let catalog = try JSONSerialization.jsonObject(with: catalogData) as! [String: Any]
let strings = catalog["strings"] as! [String: [String: Any]]
let keys = ["Country sharing", "Choose country sharing…", "Share this Mac's country", "Country", "Choose a country",
            "EastSea can use the country in your Mac's Region setting to choose a broad region bucket. The country stays on this Mac. No country preference is sent until you answer here.",
            "Country sharing is selected below. You can turn it off before continuing.",
            "Choose whether to share a country. Declining keeps all wallet and node features available.",
            "The choice contributes only to broad regional counts. Public observations hide groups smaller than three. This choice is not saved on chain. Stop future sharing anytime in Settings; already received aggregate copies may remain.",
            "Country sharing is optional. Your selected country stays on this Mac and chooses a broad region bucket. Public observations show only counts for groups of at least three. Turning it off stops future use of the country preference. Your relay's region and connection IP address remain visible to peers.",
            "Continue", "Don't share country", "Share country"]
for language in ["en", "ko", "ja", "zh-Hans", "es"] {
    for key in keys {
        let localizations = strings[key]?["localizations"] as? [String: [String: Any]]
        let unit = localizations?[language]?["stringUnit"] as? [String: String]
        check(unit?["state"] == "translated" && unit?["value"]?.isEmpty == false,
              "\(language) translates \(key)")
    }
}
print("ok")
