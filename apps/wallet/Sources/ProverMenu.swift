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
    @AppStorage("developerMode") private var developerMode = false

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
                    Text(health.pausedBadgeTitle).fixedSize(horizontal: false, vertical: true)
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
            Toggle(ko ? "이 Mac의 노드" : "Node on this Mac", isOn: $node.enabled).toggleStyle(.switch).font(.aeBody)
            if let reason = node.stopReason, reason != .switchedOff {
                // The same one reason as the sidebar, the Node page and the
                // banner — wrapped, never cut off with "…".
                NodeStopRow(reason: reason, compact: true)
            } else if node.enabled {
                Text(nodeLine).font(.aeCaption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                EarningsMenuLine()
            }
            UnattendedApprovalLine(compact: true)
            if node.prove, let p = node.prover {
                VStack(alignment: .leading, spacing: 4) {
                    let facts = ProverFacts(p)
                    let line = ProverMenuText.line(facts, ko: ko)
                    Text(line.text)
                        .foregroundStyle(line.warn ? Color.warn : Color.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let reward = ProverMenuText.reward(facts, ko: ko) {
                        Text(reward).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if developerMode, let detail = ProverMenuText.details(facts) {
                        // Raw node words and program ids: developer mode only.
                        DisclosureGroup(ko ? "자세히" : "Details") {
                            Text(detail).font(.caption.monospaced()).foregroundStyle(.secondary)
                                .textSelection(.enabled)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
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

    private var ko: Bool { HealthCheck.korean }

    /// A running node in plain words (no block numbers in the menu).
    private var nodeLine: String {
        switch node.state {
        case .off: return ko ? "꺼져 있음" : "Off"
        case .starting: return node.height > 0 ? (ko ? "네트워크를 따라잡는 중" : "Catching up with the network") : (ko ? "시작하는 중…" : "Starting…")
        case .running: return ko ? "블록을 이 Mac에서 직접 확인하는 중" : "Checking every block on this Mac"
        case .waitingForPower: return NodeStopReason.onBattery.copy(ko: ko).title
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

extension ProverFacts {
    /// The node's `aether_proverStatus`, in the menu's terms.
    init(_ p: NodeController.ProverStatus) {
        self.init(running: p.running, proving: p.proving != nil, proofs: p.proofs ?? 0, paused: p.paused,
                  programUnknown: p.program_unknown == true, programMismatch: p.program_mismatch == true,
                  proofsFailing: p.proofs_failing == true, acceptancePercent: p.acceptance_rate_percent,
                  lastReward: p.last_reward.map { "\(Wei.format(LocalRPC.decimal($0))) \(Brand.networkCoinTicker)" },
                  lastRewardStale: p.last_reward_stale == true, error: p.error, networkProgram: p.network_program)
    }
}
#endif
