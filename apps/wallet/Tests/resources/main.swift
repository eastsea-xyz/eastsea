// Checks the resource-flag builder without an app or a node (설정 ▸ 리소스,
// docs/ops/resource-limits.md):
//   swiftc -o /tmp/resources-check apps/wallet/Sources/ProverFlags.swift apps/wallet/Tests/resources/main.swift && /tmp/resources-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

// Defaults add up to no flags at all: the node picks its own defaults
// (RAM의 25%, 코어의 절반) and an older bundled node accepts the start.
check(ProverFlags.build(memory: "auto", cores: "half", battery: false, activeProcessors: 10) == [], "auto defaults pass nothing")

// 증명 끄기: the prover never runs, whatever the toggle says.
check(ProverFlags.build(memory: "off", cores: "half", battery: false, activeProcessors: 10) == ["--prover-max-memory=0"], "off stops the prover")

// A chosen cap names the gigabytes.
check(ProverFlags.build(memory: "8", cores: "half", battery: false, activeProcessors: 10) == ["--prover-max-memory=8"], "8 GB cap")
check(ProverFlags.build(memory: "16", cores: "half", battery: false, activeProcessors: 10) == ["--prover-max-memory=16"], "16 GB cap")

// 전부: every core the Mac has (at least one, never zero).
check(ProverFlags.build(memory: "auto", cores: "all", battery: false, activeProcessors: 10) == ["--prover-threads=10"], "all cores")
check(ProverFlags.build(memory: "auto", cores: "all", battery: false, activeProcessors: 1) == ["--prover-threads=1"], "one core Mac")
check(ProverFlags.build(memory: "auto", cores: "all", battery: false, activeProcessors: 0) == ["--prover-threads=1"], "never zero threads")

// 배터리에서 증명 허용.
check(ProverFlags.build(memory: "auto", cores: "half", battery: true, activeProcessors: 10) == ["--prover-on-battery"], "battery allowed")

// Everything together, in a stable order.
check(
    ProverFlags.build(memory: "4", cores: "all", battery: true, activeProcessors: 8)
        == ["--prover-max-memory=4", "--prover-threads=8", "--prover-on-battery"],
    "all together"
)

// Unknown values fall back to silence (the node's defaults), never a bad flag.
check(ProverFlags.build(memory: "", cores: "", battery: false, activeProcessors: 8) == [], "unknown values pass nothing")

print("ok")
