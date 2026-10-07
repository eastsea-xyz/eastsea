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
    static func line(_ f: ProverFacts, locale: Locale = .current, bundle: Bundle = .main) -> (text: String, warn: Bool) {
        if !f.running {
            return (String(localized: "The reward prover is not running. It restarts by itself.", bundle: bundle, locale: locale), true)
        }
        if let paused = f.paused {
            switch paused {
            case "program":
                return (String(localized: "This Mac is resting from proving blocks for now: the network cannot check this version's proofs yet. Nothing is lost.", bundle: bundle, locale: locale), false)
            case "memory":
                return (String(localized: "Proving rests for a moment: it used a lot of memory. It resumes by itself.", bundle: bundle, locale: locale), false)
            case "pressure":
                return (String(localized: "The Mac is busy, so proving rests. It resumes when things calm down.", bundle: bundle, locale: locale), false)
            case "battery":
                return (String(localized: "On battery, so proving rests. Plug in and it resumes.", bundle: bundle, locale: locale), false)
            case "disk":
                return (String(localized: "Storage is low, so proving rests. It resumes when there is room.", bundle: bundle, locale: locale), true)
            case "stalled":
                return (String(localized: "The prover stopped answering; it is restarting.", bundle: bundle, locale: locale), true)
            default:
                return (String(localized: "Proving rests for now. It resumes by itself.", bundle: bundle, locale: locale), false)
            }
        }
        if f.proofsFailing || f.programMismatch {
            return (String(localized: "Your reward proofs are being rejected. You will be told as soon as a fixed version is out.", bundle: bundle, locale: locale), true)
        }
        if f.proving {
            return (String(localized: "Proving blocks · \(String(f.proofs)) this session", bundle: bundle, locale: locale), false)
        }
        return (String(localized: "Ready to prove · \(String(f.proofs)) this session", bundle: bundle, locale: locale), false)
    }

    /// The reward line: a stale reward (from an older version's proofs) is
    /// never shown as the latest one.
    static func reward(_ f: ProverFacts, locale: Locale = .current, bundle: Bundle = .main) -> String? {
        guard let r = f.lastReward else { return nil }
        if f.lastRewardStale {
            return String(localized: "No reward yet on this version (the last one, \(r), was from an earlier version).", bundle: bundle, locale: locale)
        }
        return String(localized: "Last reward \(r)", bundle: bundle, locale: locale)
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
