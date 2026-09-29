#if os(macOS)
import Foundation

/// The resource-limit flags the app passes to its node (설정 ▸ 리소스,
/// docs/ops/resource-limits.md). Pure builder so a standalone test can check
/// the exact flag list without an app or a node.
enum ProverFlags {
    /// The `--prover-max-memory` settings: "auto" (RAM의 25%, the node's
    /// default), "4"/"8"/"16" (GB), "off" (증명 끄기, `=0`).
    static let memoryChoices = ["auto", "4", "8", "16", "off"]
    /// The `--prover-threads` settings: "half" (the node's default), "all".
    static let coreChoices = ["half", "all"]

    /// The flags for one node start. Nothing the node would default to itself
    /// is passed, so an older bundled node (no such flags) also accepts them —
    /// except "off"/"all", which name the choice explicitly.
    static func build(memory: String, cores: String, battery: Bool, activeProcessors: Int) -> [String] {
        var out: [String] = []
        switch memory {
        case "off": out += ["--prover-max-memory=0"]
        case "auto", "": break
        default: out += ["--prover-max-memory=\(memory)"]
        }
        if cores == "all" {
            out += ["--prover-threads=\(max(1, activeProcessors))"]
        }
        if battery {
            out += ["--prover-on-battery"]
        }
        return out
    }
}
#endif
