import Foundation

/// The prover's state in the menu-bar panel, in plain words: one line, a
/// reward line, and the raw node words only behind "자세히 / Details" in
/// developer mode. No block numbers, no program ids, no node error strings
/// (the founder's 0.7.0 report: a red "cannot confirm validator proof
/// program: method not found: aether_proverProgram", cut off with "…").
struct ProverFacts: Equatable {
    var running = true
    var proving = false
    var proofs: UInt64 = 0
    /// Why proving rests ("memory", "pressure", "battery", "disk", "program", "stalled").
    var paused: String?
    var programUnknown = false
    var programMismatch = false
    var proofsFailing = false
    var acceptancePercent: UInt8?
    /// The last reward, already formatted ("0.25 EAST"), and whether it
    /// predates this version's proving (then it is not shown as current).
    var lastReward: String?
    var lastRewardStale = false
    var error: String?
    var networkProgram: String?
}

enum ProverMenuText {
    /// The one line, and whether it is a warning.
    static func line(_ f: ProverFacts, ko: Bool) -> (text: String, warn: Bool) {
        if !f.running {
            return (ko ? "보상 증명 프로그램이 멈춰 있어요. 저절로 다시 시작해요." : "The reward prover is not running. It restarts by itself.", true)
        }
        if let paused = f.paused {
            switch paused {
            case "program":
                return (ko ? "이 Mac은 지금 블록 증명을 쉬고 있어요. 네트워크가 이 버전의 증명을 아직 확인하지 못해서예요. 잃는 건 없어요."
                        : "This Mac is resting from proving blocks for now: the network cannot check this version's proofs yet. Nothing is lost.", false)
            case "memory":
                return (ko ? "메모리를 많이 써서 증명을 잠시 쉬어요. 저절로 다시 시작해요." : "Proving rests for a moment: it used a lot of memory. It resumes by itself.", false)
            case "pressure":
                return (ko ? "Mac이 바빠서 증명을 잠시 쉬어요. 한가해지면 다시 해요." : "The Mac is busy, so proving rests. It resumes when things calm down.", false)
            case "battery":
                return (ko ? "배터리 사용 중이라 증명을 쉬어요. 전원을 연결하면 다시 해요." : "On battery, so proving rests. Plug in and it resumes.", false)
            case "disk":
                return (ko ? "저장 공간이 부족해 증명을 쉬어요. 공간이 생기면 다시 해요." : "Storage is low, so proving rests. It resumes when there is room.", true)
            case "stalled":
                return (ko ? "증명 프로그램이 응답하지 않아 다시 시작하고 있어요." : "The prover stopped answering; it is restarting.", true)
            default:
                return (ko ? "증명을 잠시 쉬고 있어요. 저절로 다시 시작해요." : "Proving rests for now. It resumes by itself.", false)
            }
        }
        if f.proofsFailing || f.programMismatch {
            return (ko ? "보상 증명이 거절되고 있어요. 고친 버전이 나오면 업데이트를 알려 드릴게요."
                    : "Your reward proofs are being rejected. You will be told as soon as a fixed version is out.", true)
        }
        if f.proving {
            return (ko ? "블록을 증명하는 중이에요 · 이번 실행 \(f.proofs)개" : "Proving blocks · \(f.proofs) this session", false)
        }
        return (ko ? "증명 준비됨 · 이번 실행 \(f.proofs)개" : "Ready to prove · \(f.proofs) this session", false)
    }

    /// The reward line: a stale reward (from an older version's proofs) is
    /// never shown as the latest one.
    static func reward(_ f: ProverFacts, ko: Bool) -> String? {
        guard let r = f.lastReward else { return nil }
        if f.lastRewardStale {
            return ko ? "이 버전에서는 아직 받은 보상이 없어요 (마지막 보상 \(r)은 이전 버전 때예요)."
                : "No reward yet on this version (the last one, \(r), was from an earlier version)."
        }
        return ko ? "마지막 보상 \(r)" : "Last reward \(r)"
    }

    /// The node's own words, for developer mode only.
    static func details(_ f: ProverFacts) -> String? {
        var parts: [String] = []
        if let e = f.error { parts.append("error: \(e)") }
        if let p = f.paused { parts.append("paused: \(p)\(f.programUnknown ? " (program unknown)" : "")\(f.programMismatch ? " (program mismatch)" : "")") }
        if let n = f.networkProgram { parts.append("network program: \(n)") }
        if let a = f.acceptancePercent { parts.append("accepted recently: \(a)%") }
        return parts.isEmpty ? nil : parts.joined(separator: "\n")
    }
}
