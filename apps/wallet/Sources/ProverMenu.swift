#if os(macOS)
import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The menu-bar view of this Mac's prover: what it is proving and what it proved.
struct ProverMenu: View {
    @EnvironmentObject var node: NodeController

    var body: some View {
        VStack(alignment: .leading) {
            if let p = node.prover {
                if let h = p.proving {
                    Text("Proving block #\(h)…")
                }
                if let h = p.last_height {
                    Text("Proved block #\(h) · \(p.last_txs ?? 0) tx · \(Int((p.last_seconds ?? 0).rounded())) s")
                }
                Text("\(p.proofs ?? 0) proofs this session")
                if let e = p.error {
                    Text("Last error: \(e)").foregroundStyle(.red)
                }
            } else {
                Text(node.state == .running ? "Waiting for the prover…" : "Node starting…")
            }
            Divider()
            Button("Export Reward Records…") { exportRewards() }
            Button("Stop Proving") { node.prove = false }
        }
    }

    private func exportRewards() {
        Task {
            guard let csv = await node.rewardsCSV() else { return }
            let panel = NSSavePanel()
            panel.nameFieldStringValue = "aether-rewards.csv"
            panel.allowedContentTypes = [.commaSeparatedText]
            if panel.runModal() == .OK, let url = panel.url {
                try? csv.write(to: url, atomically: true, encoding: .utf8)
            }
        }
    }
}
#endif
