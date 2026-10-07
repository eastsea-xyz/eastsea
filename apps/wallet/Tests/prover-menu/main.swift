// The menu-bar prover lines (ProverMenuText.swift): plain words, no node
// error strings, no program ids, no block numbers; a stale reward is never
// shown as the latest.
//   scripts/test-swift-pure.sh   (run prover-menu)
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

// The founder's 0.7.0 menu: proving paused because the validators cannot
// answer which program they verify. It used to show the node's raw error in
// red, cut off at two lines.
var f = ProverFacts()
f.paused = "program"; f.programUnknown = true
f.error = "cannot confirm validator proof program: method not found: aether_proverProgram"
f.networkProgram = "3e9c8976"
let line = ProverMenuText.line(f, ko: true)
check(line.text == "이 Mac은 지금 블록 증명을 쉬고 있어요. 네트워크가 이 버전의 증명을 아직 확인하지 못해서예요. 잃는 건 없어요.", "plain pause line")
check(!line.warn, "resting is not a red warning")
for ko in [true, false] {
    for paused in [nil, "program", "memory", "pressure", "battery", "disk", "stalled", "other"] as [String?] {
        var g = f; g.paused = paused
        let t = ProverMenuText.line(g, ko: ko).text
        for jargon in ["aether_", "method not found", "3e9c", "block #", "#", "program id", "RPC"] {
            check(!t.contains(jargon), "no \(jargon) in \(paused ?? "nil") (ko=\(ko)): \(t)")
        }
    }
}
check(ProverMenuText.details(f)?.contains("method not found") == true && ProverMenuText.details(f)?.contains("3e9c8976") == true,
      "the raw words live only behind Details (developer mode)")
var failing = ProverFacts(); failing.proofsFailing = true
check(ProverMenuText.line(failing, ko: true).warn, "rejected proofs are a warning")
var busy = ProverFacts(); busy.proving = true; busy.proofs = 3
check(ProverMenuText.line(busy, ko: false).text == "Proving blocks · 3 this session", "proving line")

// A reward from 9-29 (0.6.6's proofs) must not read as today's.
var r = ProverFacts(); r.lastReward = "1.0 EAST"; r.lastRewardStale = true
check(ProverMenuText.reward(r, ko: true)?.hasPrefix("이 버전에서는 아직 받은 보상이 없어요") == true, "a stale reward is labelled old")
r.lastRewardStale = false
check(ProverMenuText.reward(r, ko: false) == "Last reward 1.0 EAST", "a current reward is shown plainly")
check(ProverMenuText.reward(ProverFacts(), ko: true) == nil, "no reward, no line")
print("OK prover-menu")
