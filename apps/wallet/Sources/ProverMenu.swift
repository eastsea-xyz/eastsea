#if os(macOS)
import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The menu-bar panel: balance, whether it is verified, the node, the prover.
struct MenuBarPanel: View {
    @EnvironmentObject var model: WalletModel
    @EnvironmentObject var node: NodeController
    @Environment(\.openWindow) private var openWindow

    private var balance: String {
        model.account.map { "\(Amount.text(Double(Wei.format($0.balanceWei)) ?? 0)) \(Brand.coinTicker)" } ?? "…"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("\(Brand.project)").font(.headline)
                Spacer()
                Text(Short.address(model.address)).font(.caption.monospaced()).foregroundStyle(.secondary)
            }
            Text(balance).font(.aeTitle).monospacedDigit()
            HStack(spacing: 6) {
                if let since = model.chainPausedSince {
                    // The balance above is the last verified one; nothing is lost.
                    Image(systemName: "pause.circle.fill").foregroundStyle(Color.warn)
                    TimelineView(.periodic(from: .now, by: 30)) { tl in
                        Text(NetworkPausedText.line(since: since, now: tl.date))
                    }
                } else if model.account != nil && model.verifyError == nil {
                    Image(systemName: "checkmark.shield.fill")
                    Text("Verified on this Mac")
                } else {
                    OrbitSpinner().frame(width: 12, height: 12)
                    Text(model.networkOutdated ? "Updating the app…" : "Verifying…")
                }
            }
            .font(.aeCaption).foregroundStyle(.secondary)
            .help(model.chainPausedSince != nil ? NetworkPausedText.help : "")
            Divider()
            Toggle("Node on this Mac", isOn: $node.enabled).toggleStyle(.switch).font(.aeBody)
            if node.enabled {
                Text(nodeLine).font(.aeCaption).foregroundStyle(.secondary)
                EarningsMenuLine()
            }
            if node.prove, let p = node.prover {
                VStack(alignment: .leading, spacing: 2) {
                    if let h = p.proving { Text("Proving block #\(h)…") }
                    if let h = p.last_height {
                        Text("Proved block #\(h) · \(p.last_txs ?? 0) tx · \(Int((p.last_seconds ?? 0).rounded())) s")
                    }
                    Text("\(p.proofs ?? 0) proofs this session\(p.lag.map { " · \($0) blocks behind" } ?? "")")
                    if let paused = p.paused {
                        // docs/ops/resource-limits.md: the node's own words for why it holds proving.
                        Text(paused == "memory"
                             ? "Paused: over the memory cap, waiting out its pause"
                             : "Paused: \(paused == "pressure" ? "system memory pressure" : paused == "battery" ? "on battery" : "disk space low")")
                            .foregroundStyle(.secondary)
                    }
                    if let r = p.last_reward {
                        Text("Last reward \(Wei.format(LocalRPC.decimal(r))) \(Brand.coinTicker)").foregroundStyle(.green)
                    }
                    if let e = p.error { Text(e).foregroundStyle(.red).lineLimit(2) }
                }.font(.aeCaption)
            }
            Divider()
            HStack {
                Button("Open \(Brand.project)") {
                    NSApp.setActivationPolicy(.regular)
                    openWindow(id: "main")
                    NSApp.activate(ignoringOtherApps: true)
                }
                Button("Copy Address") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(model.address, forType: .string)
                }
                Spacer()
                Menu {
                    if node.prove { Button("Export Reward Records…") { exportRewards() } }
                    Button("Quit \(Brand.project)") { NSApp.terminate(nil) }
                } label: { Image(systemName: "ellipsis.circle") }
                    .menuStyle(.borderlessButton).fixedSize()
            }
        }
        .padding(16)
        .frame(width: 300)
    }

    private var nodeLine: String {
        switch node.state {
        case .off: "Off"
        case .starting: node.height > 0 ? "Catching up · block #\(node.height)" : "Starting…"
        case .running: "Verifying · block #\(node.height)"
        case .waitingForPower: "Paused until the Mac is on power"
        case .failed(let e): e
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
