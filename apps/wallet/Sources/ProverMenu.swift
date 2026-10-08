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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @AppStorage("developerMode") private var developerMode = false
    @State private var showReceive: Bool

    init(showReceive: Bool = false) {
        _showReceive = State(initialValue: showReceive)
    }

    private var balance: String {
        model.account.map { "\(Amount.text(Double(Wei.format($0.balanceWei)) ?? 0)) \(Brand.networkCoinTicker)" } ?? "…"
    }

    var body: some View {
        ZStack(alignment: .topLeading) {
            if showReceive {
                receivePage.transition(reduceMotion ? .opacity : .asymmetric(
                    insertion: .move(edge: .trailing).combined(with: .opacity),
                    removal: .move(edge: .trailing).combined(with: .opacity)))
            } else {
                statusPage.transition(reduceMotion ? .opacity : .asymmetric(
                    insertion: .move(edge: .leading).combined(with: .opacity),
                    removal: .move(edge: .leading).combined(with: .opacity)))
            }
        }
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: showReceive)
        .padding(16)
        .frame(width: 300)
        .clipped()
    }

    private var statusPage: some View {
        VStack(alignment: .leading, spacing: 12) {
            if model.developmentNetwork {
                Text("Dev network · 127.0.0.1")
                    .font(.caption.bold()).foregroundStyle(.orange)
            }
            HStack {
                Text(Brand.name).font(.headline)
                Spacer()
            }
            AccountSwitcherButton(store: model.accountStore, compact: true)
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
                    Text(model.networkOutdated ? String(localized: "Updating the app…") : String(localized: "Verifying…"))
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
            Button(action: openNetwork) {
                HStack(spacing: 12) {
                    Image(systemName: "globe.europe.africa.fill")
                        .font(.system(size: 34)).foregroundStyle(Color.aether)
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 3) {
                        Text("Network")
                            .font(.aeCaption).foregroundStyle(.secondary)
                        if let presence = model.liveGlobePresence {
                            Text("\(Int64(presence.total)) node observations")
                                .font(.aeBody.weight(.semibold)).monospacedDigit()
                            if model.liveGlobeState == .stale {
                                Text("Last available count").font(.aeCaption).foregroundStyle(.secondary)
                            }
                        } else if model.liveGlobeState == .withheld {
                            Text("Counts withheld for privacy").font(.aeBody).foregroundStyle(.secondary)
                        } else {
                            Text("Live network unavailable").font(.aeBody).foregroundStyle(.secondary)
                        }
                    }
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right").font(.caption).foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Open Network")
            .accessibilityElement(children: .combine)
            .accessibilityHint("Open Network")
            Divider()
            Toggle(String(localized: "Node on this Mac"), isOn: $node.enabled).toggleStyle(.switch).font(.aeBody)
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
                    let line = ProverMenuText.line(facts)
                    Text(line.text)
                        .foregroundStyle(line.warn ? Color.warn : Color.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let reward = ProverMenuText.reward(facts) {
                        Text(reward).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if developerMode, let detail = ProverMenuText.details(facts) {
                        // Raw node words and program ids: developer mode only.
                        DisclosureGroup("Details") {
                            Text(detail).font(.caption.monospaced()).foregroundStyle(.secondary)
                                .textSelection(.enabled)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }.font(.aeCaption)
            }
            Divider()
            HStack {
                Button("Open \(Brand.name)") {
                    NSApp.setActivationPolicy(.regular)
                    openWindow(id: "main")
                    NSApp.activate(ignoringOtherApps: true)
                }
                Button { showReceive = true } label: { Label("Receive", systemImage: "qrcode") }
                    .disabled(model.address.isEmpty)
                Spacer()
                Menu {
                    if node.prove { Button("Export Reward Records…") { exportRewards() } }
                    Button("Quit \(Brand.name)") { NSApp.terminate(nil) }
                } label: { Image(systemName: "ellipsis.circle") }
                    .menuStyle(.borderlessButton).fixedSize()
            }
        }
    }

    private var receivePage: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Button { showReceive = false } label: { Label("Back", systemImage: "chevron.left") }
                    .buttonStyle(.plain)
                    .keyboardShortcut(.cancelAction)
                Spacer()
                Text("Receive").font(.headline)
            }
            AccountSwitcherButton(store: model.accountStore, compact: true)
            ReceiveAddressView(address: model.address, compact: true)
        }
    }


    private func openNetwork() {
        developerMode = false
        model.networkRequested = true
        NSApp.setActivationPolicy(.regular)
        openWindow(id: "main")
        NSApp.activate(ignoringOtherApps: true)
    }

    /// A running node in plain words (no block numbers in the menu).
    private var nodeLine: String {
        switch node.state {
        case .off: return String(localized: "Off")
        case .starting: return node.height > 0 ? String(localized: "Catching up with the network") : String(localized: "Starting…")
        case .running: return String(localized: "Checking every block on this Mac")
        case .waitingForPower: return NodeStopReason.onBattery.copy().title
        case .failed(let e): return e
        }
    }

    /// Why proving holds, in the node's own categories (docs/ops/resource-limits.md).
    private static func pausedText(_ paused: String, programUnknown: Bool) -> String {
        switch paused {
        case "memory": String(localized: "Paused: over the memory limit, waiting a moment before trying again")
        case "program": programUnknown
            ? String(localized: "Paused: cannot confirm which proving program the network uses")
            : String(localized: "Paused: this Mac's proving program differs from the network's")
        case "stalled": String(localized: "Paused: the prover stopped answering; it restarts itself")
        case "pressure": String(localized: "Paused: this Mac is short on memory")
        case "battery": String(localized: "Paused: on battery")
        default: String(localized: "Paused: disk space low")
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
        self.init(running: p.running, stale: p.stale == true, proving: p.proving != nil, proofs: p.proofs ?? 0, paused: p.paused,
                  programUnknown: p.program_unknown == true, programMismatch: p.program_mismatch == true,
                  proofsFailing: p.proofs_failing == true, acceptancePercent: p.acceptance_rate_percent,
                  lastReward: p.last_reward.map { "\(Wei.format(LocalRPC.decimal($0))) \(Brand.networkCoinTicker)" },
                  lastRewardStale: p.last_reward_stale == true, error: p.error, networkProgram: p.network_program)
    }
}
#endif
