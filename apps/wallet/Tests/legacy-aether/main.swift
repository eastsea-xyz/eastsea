// legacy-aether: which old Aether.app next to EastSea can still run a node
// (release-070 review, B2). Aether <= 0.6.6 opens at login, takes the old
// data's run.lock and starts a second follower; the 0.6.7 bridge runs none.
import Foundation

var failures = 0
func expect(_ ok: Bool, _ what: String) {
    print(ok ? "ok   \(what)" : "FAIL \(what)")
    if !ok { failures += 1 }
}

expect(LegacyAether.compare("0.6.10", "0.6.9") == .orderedDescending, "0.6.10 is newer than 0.6.9 (numeric, not text)")
expect(LegacyAether.compare("0.6.7", "0.6.7.0") == .orderedSame, "missing parts count as 0")
expect(LegacyAether.compare("0.6.6", "0.6.7") == .orderedAscending, "0.6.6 is older than the bridge")
expect(LegacyAether.runsNode(version: "0.6.6"), "Aether 0.6.6 still runs a node")
expect(LegacyAether.runsNode(version: "0.5.0"), "older Aethers run a node")
expect(!LegacyAether.runsNode(version: "0.6.7"), "the 0.6.7 bridge runs no node")
expect(!LegacyAether.runsNode(version: "0.6.8"), "a later bridge fix runs no node")
expect(LegacyAether.runsNode(version: nil) && LegacyAether.runsNode(version: ""), "an unreadable version is treated as old")
let en = LegacyAether.question(locale: walletTestLocale("en"), bundle: walletTestBundle("en")), ko = LegacyAether.question(locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))
expect(en.body.contains("No data is deleted") && ko.body.contains("데이터는 지우지 않아요"),
       "the question says the data stays, in both languages")
let ja = LegacyAether.question(locale: walletTestLocale("ja"), bundle: walletTestBundle("ja"))
expect(ja.title == "以前のAetherアプリがまだこのMacにあります", "the removal question resolves in Japanese")
expect(ja.body.contains("データは削除されません"), "the Japanese question keeps the data assurance")
expect(LegacyAether.removalFailure(locale: walletTestLocale("en"), bundle: walletTestBundle("en"))
       == "Aether could not be moved to the Trash. Move it there in Finder, then reopen EastSea.", "the removal failure gives a plain next step")
expect(LegacyAether.removalFailure(locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))
       == "Aether를 휴지통으로 옮기지 못했어요. Finder에서 직접 옮긴 뒤 동해를 다시 열어 주세요.", "the removal failure resolves in Korean")
exit(Int32(failures))
