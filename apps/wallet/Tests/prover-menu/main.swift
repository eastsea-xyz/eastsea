// The menu-bar prover lines (ProverMenuText.swift): plain words, no node
// error strings, no program ids, no block numbers; a stale reward is never
// shown as the latest.
//   scripts/test-swift-pure.sh   (run prover-menu)
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

// Independent expected wording catches a host-language fallback and preserves
// the reviewed English/Korean copy while Japanese is added to the catalog.
let localizedCases: [(name: String, facts: ProverFacts, expected: [String: String])] = [
    ("not running", ProverFacts(running: false), [
        "en": "The reward prover is not running. It restarts by itself.",
        "ko": "보상 증명 프로그램이 멈춰 있어요. 저절로 다시 시작해요.",
        "ja": "報酬の証明プログラムが止まっています。自動で再起動します。",
    ]),
    ("program", ProverFacts(paused: "program"), [
        "en": "This Mac is resting from proving blocks for now: the network cannot check this version's proofs yet. Nothing is lost.",
        "ko": "이 Mac은 지금 블록 증명을 쉬고 있어요. 네트워크가 이 버전의 증명을 아직 확인하지 못해서예요. 잃는 건 없어요.",
        "ja": "このMacでは今、ブロックの証明を休んでいます。ネットワークがこのバージョンの証明をまだ確認できないためです。失うものはありません。",
    ]),
    ("memory", ProverFacts(paused: "memory"), [
        "en": "Proving rests for a moment: it used a lot of memory. It resumes by itself.",
        "ko": "메모리를 많이 써서 증명을 잠시 쉬어요. 저절로 다시 시작해요.",
        "ja": "メモリを多く使ったため、証明を少し休んでいます。自動で再開します。",
    ]),
    ("pressure", ProverFacts(paused: "pressure"), [
        "en": "The Mac is busy, so proving rests. It resumes when things calm down.",
        "ko": "Mac이 바빠서 증명을 잠시 쉬어요. 한가해지면 다시 해요.",
        "ja": "Macが忙しいため、証明を休んでいます。落ち着くと再開します。",
    ]),
    ("battery", ProverFacts(paused: "battery"), [
        "en": "On battery, so proving rests. Plug in and it resumes.",
        "ko": "배터리 사용 중이라 증명을 쉬어요. 전원을 연결하면 다시 해요.",
        "ja": "バッテリー使用中のため、証明を休んでいます。電源につなぐと再開します。",
    ]),
    ("disk", ProverFacts(paused: "disk"), [
        "en": "Storage is low, so proving rests. It resumes when there is room.",
        "ko": "저장 공간이 부족해 증명을 쉬어요. 공간이 생기면 다시 해요.",
        "ja": "空き容量が少ないため、証明を休んでいます。空き容量ができると再開します。",
    ]),
    ("stalled", ProverFacts(paused: "stalled"), [
        "en": "The prover stopped answering; it is restarting.",
        "ko": "증명 프로그램이 응답하지 않아 다시 시작하고 있어요.",
        "ja": "証明プログラムが応答しないため、再起動しています。",
    ]),
    ("other pause", ProverFacts(paused: "other"), [
        "en": "Proving rests for now. It resumes by itself.",
        "ko": "증명을 잠시 쉬고 있어요. 저절로 다시 시작해요.",
        "ja": "今は証明を休んでいます。自動で再開します。",
    ]),
    ("rejected", ProverFacts(proofsFailing: true), [
        "en": "Your reward proofs are being rejected. You will be told as soon as a fixed version is out.",
        "ko": "보상 증명이 거절되고 있어요. 고친 버전이 나오면 업데이트를 알려 드릴게요.",
        "ja": "報酬の証明が受け入れられていません。修正版が出たら更新をお知らせします。",
    ]),
    ("proving", ProverFacts(proving: true, proofs: 3), [
        "en": "Proving blocks · 3 this session",
        "ko": "블록을 증명하는 중이에요 · 이번 실행 3개",
        "ja": "ブロックを証明中 · 今回の起動で3件",
    ]),
    ("ready", ProverFacts(proofs: 3), [
        "en": "Ready to prove · 3 this session",
        "ko": "증명 준비됨 · 이번 실행 3개",
        "ja": "証明の準備完了 · 今回の起動で3件",
    ]),
]
for language in ["en", "ko", "ja"] {
    let locale = walletTestLocale(language)
    let bundle = walletTestBundle(language)
    for c in localizedCases {
        let actual = ProverMenuText.line(c.facts, locale: locale, bundle: bundle).text
        check(actual == c.expected[language], "localized prover \(c.name) (\(language)): \(actual)")
    }
}

let enLocale = walletTestLocale("en"), enBundle = walletTestBundle("en")
let koLocale = walletTestLocale("ko"), koBundle = walletTestBundle("ko")

// The founder's 0.7.0 menu: proving paused because the validators cannot
// answer which program they verify. It used to show the node's raw error in
// red, cut off at two lines.
var f = ProverFacts()
f.paused = "program"; f.programUnknown = true
f.error = "cannot confirm validator proof program: method not found: aether_proverProgram"
f.networkProgram = "3e9c8976"
let line = ProverMenuText.line(f, locale: koLocale, bundle: koBundle)
check(line.text == "이 Mac은 지금 블록 증명을 쉬고 있어요. 네트워크가 이 버전의 증명을 아직 확인하지 못해서예요. 잃는 건 없어요.", "plain pause line")
check(!line.warn, "resting is not a red warning")
for language in ["en", "ko", "ja"] {
    for paused in [nil, "program", "memory", "pressure", "battery", "disk", "stalled", "other"] as [String?] {
        var g = f; g.paused = paused
        let t = ProverMenuText.line(g, locale: walletTestLocale(language), bundle: walletTestBundle(language)).text
        for jargon in ["aether_", "method not found", "3e9c", "block #", "#", "program id", "RPC"] {
            check(!t.contains(jargon), "no \(jargon) in \(paused ?? "nil") (language=\(language)): \(t)")
        }
    }
}
check(ProverMenuText.details(f)?.contains("method not found") == true && ProverMenuText.details(f)?.contains("3e9c8976") == true,
      "the raw words live only behind Details (developer mode)")
var failing = ProverFacts(); failing.proofsFailing = true
check(ProverMenuText.line(failing, locale: koLocale, bundle: koBundle).warn, "rejected proofs are a warning")
var busy = ProverFacts(); busy.proving = true; busy.proofs = 3
check(ProverMenuText.line(busy, locale: enLocale, bundle: enBundle).text == "Proving blocks · 3 this session", "proving line")

// A reward from 9-29 (0.6.6's proofs) must not read as today's.
var r = ProverFacts(); r.lastReward = "1.0 EAST"; r.lastRewardStale = true
check(ProverMenuText.reward(r, locale: koLocale, bundle: koBundle)?.hasPrefix("이 버전에서는 아직 받은 보상이 없어요") == true, "a stale reward is labelled old")
r.lastRewardStale = false
check(ProverMenuText.reward(r, locale: enLocale, bundle: enBundle) == "Last reward 1.0 EAST", "a current reward is shown plainly")
check(ProverMenuText.reward(ProverFacts(), locale: koLocale, bundle: koBundle) == nil, "no reward, no line")
print("OK prover-menu")
