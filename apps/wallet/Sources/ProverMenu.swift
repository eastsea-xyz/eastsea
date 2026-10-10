#if os(macOS)
import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Adapts the existing wallet services to the shared EastSea menu-bar panel.
struct MenuBarPanel: View {
    @EnvironmentObject var model: WalletModel
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var earnings: Earnings
    @EnvironmentObject var health: HealthMonitor
    @EnvironmentObject var unattended: UnattendedDaemon
    @Environment(\.openWindow) private var openWindow
    @AppStorage("developerMode") private var developerMode = false
    @State private var showReceive: Bool
    @State private var copied = false

    init(showReceive: Bool = false) {
        _showReceive = State(initialValue: showReceive)
    }

    private var amount: Decimal? {
        guard let account = model.account,
              account.address.caseInsensitiveCompare(model.address) == .orderedSame,
              let value = Decimal(string: Wei.exact(account.balanceWei), locale: Locale(identifier: "en_US_POSIX")),
              !value.isNaN else { return nil }
        return value
    }

    private var isVerified: Bool {
        amount != nil && model.keyError == nil && model.verifyError == nil
            && !model.networkOutdated && model.chainPausedSince == nil
            && health.healthyBadgeAllowed && !node.diskPaused
    }

    private var verificationText: String {
        if let error = model.keyError { return error }
        if let since = model.chainPausedSince {
            // WalletModel's existing observations refresh this sentence. The
            // panel adds no timer and retains the last certificate's balance.
            return NetworkPausedText.line(since: since, now: Date())
        }
        if !health.healthyBadgeAllowed || node.diskPaused { return health.pausedBadgeTitle }
        if model.networkOutdated { return String(localized: "Updating the app…") }
        return isVerified ? String(localized: "Verified on this Mac") : String(localized: "Verifying…")
    }

    private var snapshot: EastSeaDesign.MenuBarSnapshot {
        let accessibility = amount.map {
            "\($0.formatted(.number.precision(.fractionLength(0...18)))) \(Brand.networkCoinTicker)"
        } ?? String(localized: "Verifying…")
        // There is no confirmed receipt/status event ID in these services.
        // Never infer a reward from a balance delta or a prover reward amount.
        return .init(accountIdentity: "\(model.networkChainId):\(model.accountStore.activeAccount?.id ?? 0):\(model.address.lowercased())",
                     accountName: model.accountStore.activeAccount?.name ?? String(localized: "Accounts"),
                     balance: amount, balanceAccessibility: accessibility, currency: Brand.networkCoinTicker,
                     verificationText: verificationText, isVerified: isVerified,
                     nodeStatus: nodeStatus, nodeText: nodeLine,
                     address: model.address.isEmpty ? nil : model.address,
                     verificationHelp: model.chainPausedSince != nil ? NetworkPausedText.help : "")
    }

    private var labels: EastSeaDesign.MenuBarLabels {
        .init(brandName: Brand.name, balance: String(localized: "Balance"),
              node: String(localized: "Node on this Mac"), receive: String(localized: "Receive"),
              receiveHint: String(localized: "Scan to receive \(Brand.networkCoinTicker) and \(Brand.name) tokens."),
              copyAddress: String(localized: "Copy address"), openWallet: String(localized: "Open \(Brand.name)"),
              qrAccessibility: String(localized: "Receive address QR code"),
              receiveUnavailable: String(localized: "The wallet key is not ready yet."))
    }

    var body: some View {
        EastSeaDesign.MenuBarPanel(snapshot: snapshot, labels: labels, nodeEnabled: $node.enabled,
                                  formatBalance: { Amount.text($0) }, onCopyAddress: copyAddress,
                                  onOpenWallet: openWallet, hapticsEnabled: false,
                                  accountControl: { AccountSwitcherButton(store: model.accountStore, compact: true) },
                                  nodeDetails: { nodeDetails }, receiveDetails: { receiveDetails },
                                  footer: { footer })
            .eastSeaPresentation(.panel, value: showReceive)
            .onChange(of: model.address) { _, _ in copied = false }
    }

    private var hasNodeDetails: Bool {
        model.developmentNetwork || health.alert != nil
            || (node.stopReason != nil && node.stopReason != .switchedOff)
            || node.enabled || (node.prove && node.prover != nil)
            || unattended.approvalSentence != nil
    }

    @ViewBuilder private var nodeDetails: some View {
        if hasNodeDetails {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
                if model.developmentNetwork {
                    Text("Dev network · 127.0.0.1")
                        .font(DesignTokens.TypeScale.caption.font.weight(.semibold))
                        .foregroundStyle(DesignTokens.Palette.warn.color)
                }
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
                if let alert = health.alert {
                    Text(verbatim: alert.sentence).foregroundStyle(DesignTokens.Palette.warn.color)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let reason = node.stopReason, reason != .switchedOff {
                    DisclosureGroup(String(localized: "Details")) {
                        NodeStopRow(reason: reason, compact: true)
                            .padding(.top, DesignTokens.Space.s2)
                    }
                } else if node.enabled {
                    rewardLine
                }
                UnattendedApprovalLine(compact: true)
                if node.prove, let p = node.prover {
                    VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                        let facts = ProverFacts(p)
                        let line = ProverMenuText.line(facts)
                        Text(verbatim: line.text)
                            .foregroundStyle(line.warn ? DesignTokens.Palette.warn.color : DesignTokens.Palette.textMuted.color)
                            .fixedSize(horizontal: false, vertical: true)
                        if let reward = ProverMenuText.reward(facts) {
                            Text(verbatim: reward)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        if developerMode, let detail = ProverMenuText.details(facts) {
                            DisclosureGroup(String(localized: "Details")) {
                                Text(verbatim: detail).font(DesignTokens.TypeScale.caption.font.monospaced())
                                    .textSelection(.enabled)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                }
            }
            .font(DesignTokens.TypeScale.caption.font)
            .foregroundStyle(DesignTokens.Palette.textMuted.color)
            .padding(.bottom, DesignTokens.Space.s4)
        }
    }

    @ViewBuilder private var rewardLine: some View {
        let summary = earnings.summary
        if node.enabled, node.prove || summary.count > 0 {
            HStack(alignment: .firstTextBaseline, spacing: DesignTokens.Space.s2) {
                Text("+\(EarningsText.aeth(summary.todayWei)) \(EarningsText.unit) today")
                    .fontWeight(.semibold)
                Spacer(minLength: DesignTokens.Space.s1)
                Text("\(EarningsText.aeth(summary.totalWei)) total")
            }
            .monospacedDigit()
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder private var receiveDetails: some View {
        if showReceive {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
                Text(verbatim: model.address)
                    .font(DesignTokens.TypeScale.caption.font.monospaced())
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: DesignTokens.Space.s2) {
                    Button { copyAddress(model.address) } label: {
                        Label(copied ? String(localized: "Copied") : String(localized: "Copy address"),
                              systemImage: copied ? "checkmark" : "doc.on.doc")
                    }
                    .disabled(model.address.isEmpty)
                    ShareLink(item: model.address) { Label("Share", systemImage: "square.and.arrow.up") }
                        .disabled(model.address.isEmpty)
                    Spacer(minLength: 0)
                    Button { showReceive = false } label: { Label("Back", systemImage: "chevron.left") }
                        .keyboardShortcut(.cancelAction)
                }
                .buttonStyle(.bordered).controlSize(.small)
            }
            .padding(.bottom, DesignTokens.Space.s4)
        }
    }

    private var footer: some View {
        HStack(spacing: DesignTokens.Space.s3) {
            if !showReceive {
                Button { showReceive = true } label: { Label("Receive", systemImage: "qrcode") }
                    .buttonStyle(.plain)
                    .disabled(model.address.isEmpty)
            }
            Button(action: openNetwork) { Label("Network", systemImage: "globe") }.buttonStyle(.plain)
            Spacer()
            Menu {
                if node.prove { Button("Export Reward Records…") { exportRewards() } }
                Button("Quit \(Brand.name)") { NSApp.terminate(nil) }
            } label: { Image(systemName: "ellipsis.circle") }
                .menuStyle(.borderlessButton).fixedSize()
                .accessibilityLabel(String(localized: "More actions"))
        }
        .font(DesignTokens.TypeScale.caption.font)
        .foregroundStyle(DesignTokens.Palette.textMuted.color)
        .padding(.top, DesignTokens.Space.s3)
    }

    private func copyAddress(_ address: String) {
        Clipboard.copy(address)
        copied = true
    }

    private func openNetwork() {
        developerMode = false
        model.networkRequested = true
        NSApp.setActivationPolicy(.regular)
        openWindow(id: "main")
        NSApp.activate(ignoringOtherApps: true)
    }

    private func openWallet() {
        NSApp.setActivationPolicy(.regular)
        openWindow(id: "main")
        NSApp.activate(ignoringOtherApps: true)
    }

    private var nodeStatus: EastSeaNodeStatus {
        if !node.enabled { return .offline }
        if let reason = node.stopReason, reason != .switchedOff {
            return reason.isIncident ? .offline : .paused
        }
        if model.chainPausedSince != nil || node.diskPaused || !health.healthyBadgeAllowed { return .paused }
        switch node.state {
        case .off, .failed: return .offline
        case .starting: return .checking
        case .waitingForPower: return .paused
        case .running: return node.rpcAnswering && !node.networkCheckPending ? .connected : .checking
        }
    }

    /// A running node in plain words (no block numbers in the menu).
    private var nodeLine: String {
        if let reason = node.stopReason, reason != .switchedOff { return reason.copy().title }
        if !node.enabled { return String(localized: "Off") }
        if let since = model.chainPausedSince { return NetworkPausedText.line(since: since, now: Date()) }
        if node.diskPaused || !health.healthyBadgeAllowed { return health.pausedBadgeTitle }
        switch node.state {
        case .off: return String(localized: "Off")
        case .starting: return node.height > 0 ? String(localized: "Catching up with the network") : String(localized: "Starting…")
        case .running:
            if !node.rpcAnswering { return String(localized: "Checking the node…") }
            let line = String(localized: "Checking every block on this Mac")
            return node.networkCheckPending ? "\(line) \(node.pendingRouteNote)" : line
        case .waitingForPower: return NodeStopReason.onBattery.copy().title
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
        self.init(running: p.running, stale: p.stale == true, proving: p.proving != nil, proofs: p.proofs ?? 0, paused: p.paused,
                  programUnknown: p.program_unknown == true, programMismatch: p.program_mismatch == true,
                  proofsFailing: p.proofs_failing == true, acceptancePercent: p.acceptance_rate_percent,
                  lastReward: p.last_reward.map { "\(Wei.format(LocalRPC.decimal($0))) \(Brand.networkCoinTicker)" },
                  lastRewardStale: p.last_reward_stale == true, error: p.error, networkProgram: p.network_program)
    }
}
#endif
