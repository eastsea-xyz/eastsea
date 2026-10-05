#if os(macOS)
import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The menu-bar panel: balance, whether it is verified, the node, the prover.
struct MenuBarPanel: View {
    @EnvironmentObject var model: WalletModel
    @EnvironmentObject var node: NodeController
    /// Layer 1 of the health signal: the menu bar is one of its three places.
    @EnvironmentObject var health: HealthMonitor
    @Environment(\.openWindow) private var openWindow

    private var balance: String {
        model.account.map { "\(Amount.text(Double(Wei.format($0.balanceWei)) ?? 0)) \(Brand.networkCoinTicker)" } ?? "…"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if model.developmentNetwork {
                Text("Dev network · 127.0.0.1")
                    .font(.caption.bold()).foregroundStyle(.orange)
            }
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
                } else if !health.healthyBadgeAllowed {
                    // L4: never "Verified" on a half-dead node (docs/design/32).
                    Image(systemName: "externaldrive.fill.badge.exclamationmark").foregroundStyle(Color.warn)
                    Text(HealthCheck.korean ? "저장 공간 부족 · 노드 멈춤" : "Storage low · node paused")
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
            if let alert = health.alert {
                // The banner's sentence, where a person looks without opening
                // the window (design §4.2: the five silent days began with a
                // warning only the open menu showed).
                Text(alert.sentence).font(.aeCaption).foregroundStyle(Color.warn)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Divider()
            Toggle("Node on this Mac", isOn: $node.enabled).toggleStyle(.switch).font(.aeBody)
            if node.enabled {
                Text(nodeLine).font(.aeCaption).foregroundStyle(.secondary)
                EarningsMenuLine()
            }
            if node.wrongLocation {
                // Red team #10: one sentence wherever the node would be, plus
                // a way to get to the bundle to move it.
                HStack(alignment: .top) {
                    Text(InstallLocation.moveSentence).font(.aeCaption).foregroundStyle(.orange)
                    Button {
                        InstallLocation.revealInFinder()
                    } label: {
                        Image(systemName: "folder")
                    }
                    .buttonStyle(.borderless)
                    .help("Show in Finder")
                }
            }
            if node.prove, let p = node.prover {
                VStack(alignment: .leading, spacing: 2) {
                    if !p.running { Text("Prover not running").foregroundStyle(.red) }
                    if let h = p.proving { Text("Proving block #\(h)…") }
                    if let h = p.last_height {
                        Text("Proved block #\(h) · \(p.last_txs ?? 0) tx · \(Int((p.last_seconds ?? 0).rounded())) s")
                    }
                    Text("\(p.proofs ?? 0) proofs this session\(p.lag.map { " · \($0) blocks behind" } ?? "")")
                    if p.proofs_failing == true {
                        Text(p.program_mismatch == true
                             ? "Proofs failing · proof program mismatch"
                             : (p.acceptance_rate_percent.map { "Proofs failing · \($0)% accepted recently" } ?? "Proofs failing"))
                            .foregroundStyle(.red)
                    }
                    if let paused = p.paused {
                        // docs/ops/resource-limits.md: the node's own words for why it holds proving.
                        Text(paused == "memory"
                             ? "Paused: over the memory cap, waiting out its pause"
                             : "Paused: \(paused == "program" ? (p.program_unknown == true ? "cannot confirm validator proof program" : "proof program differs from validators") : paused == "pressure" ? "system memory pressure" : paused == "battery" ? "on battery" : "disk space low")")
                            .foregroundStyle(.secondary)
                    }
                    if let r = p.last_reward {
                        Text("Last reward \(Wei.format(LocalRPC.decimal(r))) \(Brand.networkCoinTicker)").foregroundStyle(.green)
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
        if node.wrongLocation { return InstallLocation.moveSentence }
        switch node.state {
        case .off: return "Off"
        case .starting: return node.height > 0 ? "Catching up · block #\(node.height)" : "Starting…"
        case .running: return "Verifying · block #\(node.height)"
        case .waitingForPower: return "Paused until the Mac is on power"
        case .failed(let e): return e
        }
    }

    private func exportRewards() {
        Task {
            guard let csv = await node.rewardsCSV() else { return }
            let panel = NSSavePanel()
            panel.nameFieldStringValue = "eastsea-earnings.csv"
            panel.allowedContentTypes = [.commaSeparatedText]
            if panel.runModal() == .OK, let url = panel.url {
                try? csv.write(to: url, atomically: true, encoding: .utf8)
            }
        }
    }
}
#endif
