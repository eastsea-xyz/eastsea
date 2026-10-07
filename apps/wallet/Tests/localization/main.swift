import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL localization:", message); exit(1) }
}

// Nine former boolean branches, plus a reward interpolation. The expected
// values are read independently from the real catalog's per-language bundle;
// a wrong key, omitted bundle, or missing translation therefore fails.
let pauseKeys: [(String, String)] = [
    ("program", "This Mac is resting from proving blocks for now: the network cannot check this version's proofs yet. Nothing is lost."),
    ("memory", "Proving rests for a moment: it used a lot of memory. It resumes by itself."),
    ("pressure", "The Mac is busy, so proving rests. It resumes when things calm down."),
    ("battery", "On battery, so proving rests. Plug in and it resumes."),
    ("disk", "Storage is low, so proving rests. It resumes when there is room."),
    ("stalled", "The prover stopped answering; it is restarting."),
    ("other", "Proving rests for now. It resumes by itself."),
]
for language in ["en", "ko", "ja"] {
    let locale = walletTestLocale(language)
    let bundle = walletTestBundle(language)
    func expected(_ key: String) -> String {
        let value = walletExpectedTranslation(key, language: language)
        check(language == "en" || value != key, "missing \(language) translation for \(key)")
        return value
    }
    for (pause, key) in pauseKeys {
        var facts = ProverFacts(); facts.paused = pause
        check(ProverMenuText.line(facts, locale: locale, bundle: bundle).text == expected(key),
              "\(language) pause \(pause) resolves from the catalog")
    }
    var stopped = ProverFacts(); stopped.running = false
    check(ProverMenuText.line(stopped, locale: locale, bundle: bundle).text
          == expected("The reward prover is not running. It restarts by itself."), "\(language) stopped prover")
    var rejected = ProverFacts(); rejected.proofsFailing = true
    check(ProverMenuText.line(rejected, locale: locale, bundle: bundle).text
          == expected("Your reward proofs are being rejected. You will be told as soon as a fixed version is out."),
          "\(language) rejected proofs")
    var reward = ProverFacts(); reward.lastReward = "1 DBLN"
    let rewardKey = "Last reward %@"
    let wanted = String(format: expected(rewardKey), locale: locale, "1 DBLN")
    check(ProverMenuText.reward(reward, locale: locale, bundle: bundle) == wanted, "\(language) last reward interpolation")
}
// Preserve the reviewed Korean copy as well as the independent catalog check.
var memory = ProverFacts(); memory.paused = "memory"
check(ProverMenuText.line(memory, locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")).text
      == "메모리를 많이 써서 증명을 잠시 쉬어요. 저절로 다시 시작해요.", "reviewed Korean memory line stays unchanged")
print("OK localization: 10 former branches × en/ko/ja, explicit locale and catalog bundle")
