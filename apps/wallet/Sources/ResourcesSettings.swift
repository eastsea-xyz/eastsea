#if os(macOS)
import SwiftUI

/// 설정 ▸ 리소스 (docs/ops/resource-limits.md): the proving sidecar's budgets —
/// what the 2026-09-29 incident asked for (a 14 GB prover, swap at 95%, the
/// validators on the same Mac crawling). Choices persist in UserDefaults and
/// go to the node as flags on its next start (ProverFlags.build).
struct ResourcesSection: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var model: WalletModel
    /// In Simple mode the budgets themselves are hidden: the node's own safe
    /// defaults (RAM의 25%, 코어의 절반) hold, while the prover switch and the
    /// warnings stay — a hidden cost switch or a silent pause would be worse.
    @AppStorage("developerMode") private var developerMode = false

    /// One footprint or cap for a caption line: "5.3 GB".
    private func gbytes(_ bytes: UInt64?) -> String {
        bytes.map { String(format: "%.1f GB", Double($0) / 1_073_741_824) } ?? "—"
    }

    var body: some View {
        Section("리소스") {
            Toggle("증명(Prover) 사용", isOn: Binding(get: { node.prove }, set: {
                if $0 { node.proveAddress = model.address }
                node.prove = $0
            }))
            .disabled(model.address.isEmpty)
            .help("Your node proves recent blocks with Metal. The first valid proof of a block gets a test AETH reward in this wallet.")
            if developerMode {
                Picker("최대 메모리", selection: $node.proverMemory) {
                    Text("자동 (RAM의 25%)").tag("auto")
                    Text("4 GB").tag("4")
                    Text("8 GB").tag("8")
                    Text("16 GB").tag("16")
                    Text("끄기").tag("off")
                }
                .help("The prover is stopped past this memory and restarted after a pause that doubles each time (a minute to half an hour). 끄기 stops proving entirely.")
                Picker("최대 CPU", selection: $node.proverCores) {
                    Text("절반").tag("half")
                    Text("전부").tag("all")
                }
                .help("Proving also runs at a lower scheduler priority, so the node and this Mac's work come first.")
                Toggle("배터리에서 증명 허용", isOn: $node.proverOnBattery)
                    .help("Off: proving pauses on battery and resumes five minutes after the power adapter returns.")
            } else {
                Text("Proving uses at most a quarter of this Mac's memory and half its cores; it stops itself there. Developer mode (⇧⌘D) adds the budgets.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if let p = node.prover, p.running {
                Text("현재 메모리 \(gbytes(p.memory_bytes)) / \(gbytes(p.memory_cap))")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if node.prover?.paused == "memory" {
                Label("메모리 부족으로 일시 정지", systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if node.diskLow {
                Label("디스크 공간 부족", systemImage: "externaldrive.badge.exclamationmark")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}
#endif
