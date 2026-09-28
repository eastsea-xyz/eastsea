import Charts
import CoreImage.CIFilterBuiltins
import SwiftUI

/// Everyday wallet, modeled on Phantom (centered balance, round actions, token
/// list) and IPFS Desktop (sidebar, "connected" status page with stat tiles and
/// traffic chart). macOS: sidebar; iOS: bottom tabs. Proofs, roots and raw logs
/// live in Developer mode.
struct SimpleDashboard: View {
    @EnvironmentObject var model: WalletModel
    @State private var page: Page? = .home
    @State private var sheet: Sheet?
    @AppStorage("acceptedTerms") private var acceptedTerms = 0
    #if os(macOS)
    @EnvironmentObject var node: NodeController
    /// Asked once whether this Mac should become a voting node.
    @AppStorage("votingInviteAnswered") private var inviteAnswered = false
    #endif
    #if os(macOS)
    /// Narrow window (iPhone-like): the sidebar folds away and a toolbar picker switches pages.
    @State private var compact = false
    @State private var columns: NavigationSplitViewVisibility = .all
    #endif

    enum Page: String, CaseIterable, Identifiable {
        case home = "Home", activity = "Activity", network = "Network", security = "Security"
        var id: String { rawValue }
        var icon: String {
            switch self {
            case .home: "house.fill"
            case .activity: "clock.arrow.circlepath"
            case .network: "point.3.connected.trianglepath.dotted"
            case .security: "lock.shield.fill"
            }
        }
    }

    enum Sheet: String, Identifiable {
        case send, receive, assets, call, connect, votingInvite
        var id: String { rawValue }
    }

    var body: some View {
        shell
            .tint(.aether)
            .sheet(item: $sheet) { s in
                switch s {
                case .send: SendSheet()
                case .receive: ReceiveSheet()
                case .assets: AssetsSheet()
                case .call: CallSheet()
                case .connect: ConnectSheet()
                case .votingInvite:
                    #if os(macOS)
                    VotingNodeInvite(join: {
                        inviteAnswered = true
                        sheet = nil
                        if let c = node.candidate { model.registerNode(c, node: node) }
                    }, later: {
                        inviteAnswered = true
                        sheet = nil
                    })
                    #else
                    EmptyView()
                    #endif
                }
            }
            #if DEBUG
            // `-previewSheet assets` / `-previewPage network` (with -designPreview) for screenshots.
            .onAppear {
                guard DesignPreview.on else { return }
                if let p = UserDefaults.standard.string(forKey: "previewPage").flatMap({ Page(rawValue: $0.capitalized) }) { page = p }
                if let s = UserDefaults.standard.string(forKey: "previewSheet").flatMap(Sheet.init(rawValue:)) { sheet = s }
            }
            #endif
            .onChange(of: model.callRequest) { _, r in if r != nil { sheet = .call } }
            .onChange(of: model.connectRequest) { _, r in if r != nil { sheet = .connect } }
            // A payment link (aether://pay?...) opens the send sheet, filled in, for approval.
            .onChange(of: model.paymentRequest) { _, r in if r != nil { sheet = .send } }
            #if os(macOS)
            // Once the node has caught up and this Mac is not registered, ask once.
            .onChange(of: node.voting) { _, _ in inviteIfReady() }
            .onChange(of: node.state) { _, _ in inviteIfReady() }
            #endif
    }

    #if os(macOS)
    private func inviteIfReady() {
        guard !inviteAnswered, acceptedTerms >= Terms.version, sheet == nil, node.state == .running,
              node.candidate != nil, node.voting?.registered == false, model.registration == nil else { return }
        sheet = .votingInvite
    }

    private var shell: some View {
        NavigationSplitView(columnVisibility: $columns) {
            List(Page.allCases, selection: $page) { p in
                Label(p.rawValue, systemImage: p.icon).tag(p)
            }
            .navigationSplitViewColumnWidth(min: 170, ideal: 190)
            .safeAreaInset(edge: .bottom) { SidebarStatus().padding(12) }
            .toolbar(removing: compact ? .sidebarToggle : nil)
        } detail: {
            ScrollView {
                // Same gutter left and right, content kept to a readable width.
                pageView(page ?? .home)
                    .frame(maxWidth: 760)
                    .padding(.horizontal, compact ? 16 : 24)
                    .padding(.vertical, 24)
                    .frame(maxWidth: .infinity)
            }
            // The scroller never sits on top of a card.
            .scrollIndicators(.hidden)
            .measuringNarrowLayout()
            .navigationTitle(page?.rawValue ?? "Home")
            .toolbar {
                if compact {
                    ToolbarItem(placement: .principal) { pagePicker }
                }
            }
        }
        .frame(minWidth: 380, minHeight: 520)
        .onGeometryChange(for: Bool.self) { $0.size.width < LayoutWidth.compactWindow } action: { narrow in
            compact = narrow
            columns = narrow ? .detailOnly : .all
        }
    }

    /// The sidebar's pages as a compact segmented control (narrow windows only).
    private var pagePicker: some View {
        Picker("Page", selection: Binding(get: { page ?? .home }, set: { page = $0 })) {
            ForEach(Page.allCases) { p in
                Image(systemName: p.icon).help(p.rawValue).tag(p)
            }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .fixedSize()
    }
    #else
    private var shell: some View {
        TabView(selection: Binding(get: { page ?? .home }, set: { page = $0 })) {
            ForEach(Page.allCases) { p in
                NavigationStack {
                    ScrollView { pageView(p).padding(16) }
                        .measuringNarrowLayout()
                        .navigationTitle(p == .home ? "" : p.rawValue)
                }
                .tabItem { Label(p.rawValue, systemImage: p.icon) }
                .tag(p)
            }
        }
    }
    #endif

    @ViewBuilder private func pageView(_ p: Page) -> some View {
        switch p {
        case .home: HomePage(sheet: $sheet, showActivity: { page = .activity }, showNetwork: { page = .network })
        case .activity: ActivityPage()
        case .network: NetworkPage()
        case .security: SecurityPage()
        }
    }
}

extension Color {
    /// Aether accent: a violet in the family of Phantom's, readable in light and dark.
    static let aether = Color(red: 0.49, green: 0.40, blue: 0.95)
}

// MARK: - Pages

private struct HomePage: View {
    @EnvironmentObject var model: WalletModel
    @Binding var sheet: SimpleDashboard.Sheet?
    let showActivity: () -> Void
    let showNetwork: () -> Void
    @Environment(\.narrowLayout) private var narrow

    private var balance: Double { model.account.flatMap { Double(Wei.format($0.balanceWei)) } ?? 0 }

    /// The balance chart earns its place only once the balance has actually changed.
    private var hasHistory: Bool { Set(model.history.map(\.aeth)).count >= 2 }

    /// One flat list of sections, the balance first and largest; no card inside
    /// a card, and nothing measures its own geometry (the page-wide
    /// `narrowLayout` environment switches layouts instead).
    var body: some View {
        VStack(spacing: 24) {
            IncomingRecoveryAlert()
            VStack(spacing: 8) {
                accountButton
                balanceText
                VerifiedBadge()
            }
            .padding(.top, 8)
            HStack(spacing: narrow ? 20 : 28) {
                RoundAction(title: "Receive", icon: "qrcode") { sheet = .receive }.disabled(model.address.isEmpty)
                RoundAction(title: "Send", icon: "paperplane.fill") { sheet = .send }.disabled(model.busy || model.account == nil)
                RoundAction(title: "Assets", icon: "square.stack.3d.up.fill") { sheet = .assets }.disabled(model.address.isEmpty)
            }
            #if os(macOS)
            HomeEarnings(open: showNetwork)
            #else
            // No node on iPhone: when the wallet has received node rewards, one
            // line — never a card that competes with the balance above it.
            NodeRewardsLine(showActivity: showActivity)
            #endif
            if hasHistory { BalanceCard() }
            recentActivity
        }
    }

    private var accountButton: some View {
        Button {
            Clipboard.copy(model.address)
        } label: {
            HStack(spacing: 6) {
                Circle().fill(LinearGradient(colors: [.aether, .pink], startPoint: .topLeading, endPoint: .bottomTrailing)).frame(width: 20, height: 20)
                Text("Account 1").font(.aeFootnote.weight(.semibold)).lineLimit(1)
                Text(Short.address(model.address)).font(.aeFootnote.monospaced()).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle)
                Image(systemName: "doc.on.doc").font(.aeCaption).foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12).padding(.vertical, 6)
            .background(.background.secondary, in: Capsule())
        }
        .buttonStyle(.plain)
        .help("Copy address")
    }

    @ViewBuilder private var balanceText: some View {
        if model.account == nil, model.chainPausedSince != nil {
            // Paused before anything could be verified: nothing to show yet.
            Text("– AETH").font(.display).foregroundStyle(.secondary)
        } else if model.account == nil {
            // Loading: a soft shimmer where the balance will appear.
            ShimmerBar().frame(maxWidth: 220).frame(height: 52)
        } else {
            Text("\(Amount.text(balance)) AETH")
                .font(.display)
                .monospacedDigit()
                .lineLimit(1)
                .minimumScaleFactor(0.4)
                .contentTransition(.numericText())
        }
    }

    /// Directly on the page (Phantom-style), not boxed in another card: Home
    /// already has the balance card above it.
    private var recentActivity: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Recent activity").font(.aeHeadline)
                Spacer()
                if !model.activity.isEmpty { Button("See all", action: showActivity).buttonStyle(.borderless) }
            }
            ActivityList(limit: 3)
        }
    }
}

#if os(iOS)
/// "● Node rewards · +0.5 AETH ›" — the one line iPhone Home gives rewards,
/// only once the wallet has received some (they come from a Mac's node).
private struct NodeRewardsLine: View {
    @EnvironmentObject var model: WalletModel
    let showActivity: () -> Void

    private var rewardTotal: Double? {
        let rewards = model.activity.filter(\.isNodeReward)
        guard !rewards.isEmpty else { return nil }
        return rewards.compactMap(\.amount).reduce(0, +)
    }

    var body: some View {
        if let total = rewardTotal {
            Button(action: showActivity) {
                HStack(spacing: 8) {
                    Circle().fill(Color.aether).frame(width: 8, height: 8)
                    Text("Node rewards · +\(Amount.text(total)) AETH")
                    Spacer(minLength: 4)
                    Image(systemName: "chevron.right").foregroundStyle(.tertiary)
                }
                .font(.aeBody).foregroundStyle(.secondary)
                .padding(.horizontal, 16).padding(.vertical, 12)
                .background(.background.secondary, in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}
#endif

private struct ActivityPage: View {
    var body: some View {
        VStack(spacing: 12) {
            Card { ActivityList(limit: 100) }
            #if os(macOS)
            RewardsExportCard()
            #endif
        }
    }
}

#if os(macOS)
/// Every reward this Mac earned, as a CSV for the user's own records, and the
/// one line that keeps Aether honest: it is not a tax adviser
/// (docs/research/node-reward-tax-2026.md).
private struct RewardsExportCard: View {
    @EnvironmentObject var node: NodeController
    @State private var working = false

    var body: some View {
        Card {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "square.and.arrow.down").font(.aeTitle).foregroundStyle(Color.aether)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Reward records").font(.aeHeadline)
                    Text("Every reward this Mac's proofs earned — time, block, amount and kind — as a CSV for your own records. Aether does not give tax advice.")
                        .font(.aeBody).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 8)
                Button(working ? "Exporting…" : "Export CSV…") { export() }
                    .disabled(working || node.proveAddress.isEmpty)
                    .help("Saves aether-rewards.csv. The node keeps the records; nothing leaves this Mac.")
            }
        }
    }

    private func export() {
        working = true
        Task { @MainActor in
            defer { working = false }
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

private struct NetworkPage: View {
    @EnvironmentObject var model: WalletModel

    @Environment(\.narrowLayout) private var narrow

    private var blockTime: String {
        // The oldest and newest of what is kept (no sort: this runs on every layout pass).
        guard let first = model.blocks.min(by: { $0.height < $1.height }),
              let last = model.blocks.max(by: { $0.height < $1.height }),
              model.blocks.count > 1, last.timestampMs > first.timestampMs else { return "—" }
        let s = Double(last.timestampMs - first.timestampMs) / 1000 / Double(model.blocks.count - 1)
        return String(format: "%.1f s", s)
    }

    var body: some View {
        VStack(spacing: 16) {
            Card {
                HStack(spacing: 16) {
                    Image(systemName: model.status == nil ? "antenna.radiowaves.left.and.right.slash" : model.chainPausedSince != nil ? "pause.circle.fill" : "checkmark.circle.fill")
                        .font(.system(size: narrow ? 30 : 40)).foregroundStyle(model.status == nil || model.chainPausedSince != nil ? Color.warn : Color.aether)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(model.status == nil ? "Connecting to Aether…" : model.chainPausedSince != nil ? "Network paused" : "Connected to Aether")
                            .font(.aeTitle)
                        Text("Found the validators on the public DHT. Your balance is checked on this device against their group signature.")
                            .font(.aeBody).foregroundStyle(.secondary)
                    }
                }
            }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: narrow ? 130 : 150), spacing: 12)], spacing: 12) {
                Tile(value: model.status.map { "#\($0.height)" } ?? "—", label: "Latest block", icon: "cube")
                Tile(value: "\(model.validators)", label: "Validators", icon: "person.3.fill")
                Tile(value: blockTime, label: "Block time", icon: "timer")
                Tile(value: model.status.map { Amount.fee($0.transferFeeWei) } ?? "—", label: "Transfer fee", icon: "flame")
                Tile(value: model.status.map { "\($0.mempool)" } ?? "—", label: "Waiting txs", icon: "tray.full")
            }
            NetworkCard()
            #if os(macOS)
            // The full earnings card lives here; Home shows it only once there is a reward.
            NodeEarningsCard()
            NodeCard()
            UpdateCard()
            #else
            DeveloperModeCard()
            #endif
        }
    }
}

#if os(iOS)
/// Developer mode (proofs, roots, raw logs), out of the way at the bottom of Network.
private struct DeveloperModeCard: View {
    @AppStorage("developerMode") private var developerMode = false

    var body: some View {
        Card {
            Toggle(isOn: $developerMode.animation(.easeInOut(duration: 0.2))) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Developer mode").font(.aeHeadline)
                    Text("Proofs, state roots, raw logs and blocks.").font(.aeFootnote).foregroundStyle(.secondary)
                }
            }
        }
    }
}
#endif

/// Shown when someone started recovering THIS account: cancel it if it was not you.
private struct IncomingRecoveryAlert: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.narrowLayout) private var narrow

    var body: some View {
        if let r = model.incomingRecovery {
            AdaptiveStack(spacing: 12) {
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: "exclamationmark.shield.fill").font(.title).foregroundStyle(Color.warn)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Your recovery devices started moving your funds").font(.aeHeadline)
                        Text("If this was not you, cancel it. It can run after \(Date(timeIntervalSince1970: TimeInterval(r.readyAt)).formatted(date: .abbreviated, time: .shortened)).")
                            .font(.aeBody).foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                VStack(alignment: narrow ? .leading : .trailing, spacing: 6) {
                    Button("Cancel it") { model.cancelIncomingRecovery() }.buttonStyle(.borderedProminent).tint(Color.warn).disabled(model.busy)
                    // A recovery you did not start means a recovery key is in other hands.
                    Button("Cancel and remove all recovery keys") { model.removeRecoveryKeys() }.font(.aeFootnote).disabled(model.busy)
                }
                .padding(.leading, narrow ? 40 : 0)
            }
            .padding(16)
            .background(Color.warn.opacity(0.12), in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
        }
    }
}

private struct SecurityPage: View {
    var body: some View {
        VStack(spacing: 16) {
            IncomingRecoveryAlert()
            Card {
                HStack(alignment: .top, spacing: 14) {
                    Image(systemName: "lock.shield.fill").font(.system(size: 34)).foregroundStyle(Color.aether)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Protected by this device").font(.aeHeadline)
                        Text("Your key was created inside the Secure Enclave and can never be copied out. Every payment asks for Touch ID or your password. If you lose every device and have no recovery set up, nobody can restore the funds.")
                            .font(.aeBody).foregroundStyle(.secondary)
                    }
                }
            }
            Card { PaperKeyPanel() }
            Card { RecoveryPanel() }
        }
    }
}

/// Recovery words: 24 words that stand in for a recovery device.
private struct PaperKeyPanel: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.narrowLayout) private var narrow

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Recovery words").font(.aeHeadline)
            Text("Your key never leaves this device, so there is no seed phrase to back up. Instead, write down 24 recovery words: if you lose every device, they move your funds to a new Mac after a 48-hour safety delay. If someone else finds them, they can only start that delay, and any of your devices can cancel it.")
                .font(.aeBody).foregroundStyle(.secondary)
            if let words = model.paperWords {
                let list = words.split(separator: " ").map(String.init)
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), alignment: .leading), count: narrow ? 2 : 4), alignment: .leading, spacing: 6) {
                    ForEach(Array(list.enumerated()), id: \.offset) { i, w in
                        Text("\(i + 1). \(w)").font(.aeBody.monospaced()).lineLimit(1).minimumScaleFactor(0.7)
                    }
                }
                .padding(12)
                .background(.background.tertiary, in: RoundedRectangle(cornerRadius: Radius.inner))
                Text("Write them on paper, in order. Do not photograph or store them on this device.").font(.aeFootnote).foregroundStyle(Color.warn)
                ViewThatFits(in: .horizontal) {
                    HStack {
                        Button("I wrote them down: register") { model.registerPaperKey() }.buttonStyle(.borderedProminent).disabled(model.busy)
                        Button("Cancel") { model.paperWords = nil }
                    }
                    VStack(alignment: .leading) {
                        Button("I wrote them down: register") { model.registerPaperKey() }.buttonStyle(.borderedProminent).disabled(model.busy)
                        Button("Cancel") { model.paperWords = nil }
                    }
                }
            } else {
                Button("Create recovery words") { model.createPaperKey() }.disabled(model.busy)
            }
        }
    }
}

#if os(macOS)
/// The node switch, explained (Network page), and joining as a voting node.
private struct NodeCard: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var model: WalletModel

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .top, spacing: 14) {
                    Image(systemName: "server.rack").font(.system(size: 30)).foregroundStyle(Color.aether)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Run a node on this Mac").font(.aeHeadline)
                        Text("Your Mac checks every block itself and your wallet asks it instead of the network. It stops when you quit Aether.")
                            .font(.aeBody).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Toggle("", isOn: $node.enabled).toggleStyle(.switch).labelsHidden()
                }
                if node.enabled, let c = node.candidate {
                    Divider()
                    VotingNodeRow(candidate: c)
                }
            }
        }
    }
}

/// The installed version, when updates were last checked, and a Check button.
/// Updates also arrive by themselves (hourly, and when the chain schedules one).
private struct UpdateCard: View {
    @EnvironmentObject var updates: Updates

    var body: some View {
        Card {
            HStack(alignment: .center, spacing: 14) {
                Image(systemName: "arrow.triangle.2.circlepath").font(.system(size: 26)).foregroundStyle(Color.aether)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Aether \(updates.version)").font(.aeHeadline)
                    // Re-read every half minute so "checked 1 hour ago" stays true.
                    TimelineView(.periodic(from: .now, by: 30)) { context in
                        Text(checked(at: context.date)).font(.aeBody).foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Button("Check for Updates") { updates.check() }
            }
        }
    }

    private func checked(at now: Date) -> String {
        guard let last = updates.lastCheck else { return "Updates install by themselves. Not checked yet." }
        let ago = RelativeDateTimeFormatter().localizedString(for: last, relativeTo: now)
        return "Updates install by themselves. Last checked \(ago)."
    }
}

/// Voting node: nobody appoints validators. Registered Macs prove they are
/// alive every epoch; the network picks the longest-running ones, no owner
/// holding a third, and they switch over by themselves.
private struct VotingNodeRow: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var model: WalletModel
    let candidate: NodeController.Candidate

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: icon).font(.system(size: 26)).foregroundStyle(Color.aether)
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.aeHeadline)
                Text(detail).font(.aeBody).foregroundStyle(.secondary)
                switch model.registration {
                case .working?:
                    HStack(spacing: 7) { OrbitSpinner().frame(width: 14, height: 14); Text("Registering… confirm with Touch ID.") }.font(.aeBody)
                case .failed(let why)?:
                    Label(why, systemImage: "exclamationmark.triangle.fill").font(.aeBody).foregroundStyle(Color.warn)
                        .fixedSize(horizontal: false, vertical: true)
                case nil:
                    EmptyView()
                }
                DisclosureGroup("Planned mainnet rules") {
                    Text(VotingRules.mainnetRewardsRule + " " + VotingRules.founderReserveRule).font(.aeFootnote).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .font(.aeFootnote)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if node.voting?.registered == false, model.registration != .working {
                Button(model.registration == nil ? "Join" : "Try again") { model.registerNode(candidate, node: node) }
                    .buttonStyle(.borderedProminent)
                    .disabled(model.busy)
                    .help("Registers this Mac with Apple DeviceCheck (one Mac, one voting node) and signs with Touch ID.")
            }
        }
    }

    private var icon: String {
        switch node.voting {
        case .some(let v) where v.voting: "checkmark.seal.fill"
        case .some(let v) where v.registered: "clock.badge.checkmark"
        default: "person.badge.plus"
        }
    }

    private var title: String {
        switch node.voting {
        case .some(let v) where v.voting: "Voting · this Mac signs blocks"
        case .some(let v) where v.registered: "Candidate · \(min(v.streak, VotingRules.minStreakEpochs)) of \(VotingRules.minStreakEpochs) hours online"
        case .some: "Become a voting node"
        case .none: "Voting node"
        }
    }

    private var detail: String {
        switch node.voting {
        case .some(let v) where v.voting: "Picked by the network for its long uptime. Keep the node on: stopping hands the seat to the next Mac."
        case .some(let v) where v.registered:
            v.candidates < VotingRules.minCandidates
                ? "Your Mac proves it is online every hour, for free. The network starts drawing voting Macs once \(VotingRules.minCandidates) Macs are registered (\(v.candidates) so far) and each has been online \(VotingRules.minStreakEpochs) hours in a row."
                : "Your Mac proves it is online every hour, for free. After \(VotingRules.minStreakEpochs) hours in a row it enters the daily draw of voting Macs (\(v.candidates) registered)."
        case .some: "One Mac, one voting node. Your Mac proves it is alive every epoch; the longest-running Macs are picked to sign blocks, and no single operator address can hold a third of the seats."
        case .none: "Checking the network…"
        }
    }
}
#endif

// MARK: - Sidebar status (IPFS Desktop style)

private struct SidebarStatus: View {
    @EnvironmentObject var model: WalletModel
    #if os(macOS)
    @EnvironmentObject var node: NodeController
    #endif

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            #if os(macOS)
            Toggle(isOn: $node.enabled) {
                VStack(alignment: .leading, spacing: 1) {
                    Text("Node on this Mac").font(.aeCaption.weight(.semibold))
                    Text(nodeLine).font(.aeCaption).foregroundStyle(.secondary).lineLimit(2)
                }
            }
            .toggleStyle(.switch)
            .controlSize(.small)
            .help("Verify every block on this Mac and let the wallet use it. Off when the app quits.")
            EarningsSidebarBadge()
            #endif
            HStack(spacing: 8) {
                Circle().fill(model.status == nil || model.chainPausedSince != nil ? Color.warn : Color.aether).frame(width: 8, height: 8)
                VStack(alignment: .leading, spacing: 1) {
                    Text(model.status == nil ? "Connecting" : model.chainPausedSince != nil ? "Network paused" : "Connected").font(.aeCaption.weight(.semibold))
                    Text(model.status.map { "Block #\($0.height)" } ?? "Searching DHT…").font(.aeCaption).foregroundStyle(.secondary)
                }
                Spacer()
            }
        }
    }

    #if os(macOS)
    /// Short on purpose: the sidebar is 170–190 pt, and the block number (which
    /// the "Connected" line below already shows) is what got truncated here.
    private var nodeLine: String {
        switch node.state {
        case .off: "Off"
        case .starting: node.height > 0 ? "Catching up" : "Starting…"
        case .running: "Verifying blocks"
        case .waitingForPower: "Paused on battery"
        case .failed(let m): m
        }
    }
    #endif
}

// MARK: - Cards

struct Card<Content: View>: View {
    let content: Content
    @Environment(\.narrowLayout) private var narrow
    init(@ViewBuilder _ content: () -> Content) { self.content = content() }

    var body: some View {
        content
            .padding(narrow ? CardPadding.narrow : CardPadding.wide)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.background.secondary, in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
    }
}

private struct BalanceCard: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.narrowLayout) private var narrow
    @State private var range: Range = .day
    /// The series to draw, kept out of `body`: a window resize re-layouts the chart
    /// many times a second, and recomputing points there (with a fresh `Date()`)
    /// re-fed Swift Charts new data on every pass, re-scaling it each time.
    @State private var points: [BalancePoint] = []

    enum Range: String, CaseIterable, Identifiable {
        case hour = "1H", day = "1D", week = "1W", all = "All"
        var id: String { rawValue }
        var seconds: TimeInterval? {
            switch self {
            case .hour: 3_600
            case .day: 86_400
            case .week: 604_800
            case .all: nil
            }
        }
    }

    /// Rebuild the series when the history or the range changed (and once on
    /// appear). Between those, layout passes — resizing included — reuse it.
    private func rebuildPoints() {
        points = BalanceHistory.points(model.history, range: range.seconds, now: Date())
    }

    private var balance: Double { model.account.flatMap { Double(Wei.format($0.balanceWei)) } ?? 0 }

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 12) {
                if narrow {
                    VStack(alignment: .leading, spacing: 10) {
                        change
                        rangePicker
                    }
                } else {
                    HStack {
                        change
                        Spacer()
                        rangePicker
                    }
                }
                chart.frame(height: narrow ? 140 : 170)
            }
        }
        .onAppear { rebuildPoints() }
        .onChange(of: model.history) { _, _ in rebuildPoints() }
        .onChange(of: range) { _, _ in rebuildPoints() }
    }

    private var rangePicker: some View {
        Picker("Range", selection: $range) {
            ForEach(Range.allCases) { Text($0.rawValue).tag($0) }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .fixedSize()
    }

    @ViewBuilder private var change: some View {
        let period = range == .all ? "total" : "in \(range.rawValue)"
        if let first = points.first, points.count > 1, balance != first.aeth {
            let d = balance - first.aeth
            Label("\(d > 0 ? "+" : "")\(Amount.text(d)) AETH \(period)", systemImage: d > 0 ? "arrow.up.right" : "arrow.down.right")
                .font(.aeBody.weight(.medium))
                .foregroundStyle(d > 0 ? Color.green : Color.secondary)
        } else if points.count > 1 {
            Text("No change \(period)").font(.aeBody).foregroundStyle(.secondary)
        } else {
            Text("Your balance history appears here as it changes.").font(.aeBody).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var chart: some View {
        if points.count > 1 {
            Chart(points) { p in
                AreaMark(x: .value("Time", p.date), y: .value("AETH", p.aeth))
                    .interpolationMethod(.stepEnd)
                    .foregroundStyle(.linearGradient(colors: [.aether.opacity(0.35), .aether.opacity(0.02)], startPoint: .top, endPoint: .bottom))
                LineMark(x: .value("Time", p.date), y: .value("AETH", p.aeth))
                    .interpolationMethod(.stepEnd)
                    .lineStyle(StrokeStyle(lineWidth: 2.5))
                    .foregroundStyle(Color.aether)
            }
            .chartYScale(domain: .automatic(includesZero: true))
            .chartXAxis {
                // Short labels ("10:00", "9/27"), and fewer of them when narrow:
                // the default formatter ran out of width and cut to "Sep 24…".
                AxisMarks(values: .automatic(desiredCount: narrow ? 3 : 5)) { _ in
                    AxisValueLabel(format: axisFormat)
                }
            }
        } else {
            RoundedRectangle(cornerRadius: Radius.inner).fill(.quaternary.opacity(0.5))
                .overlay(Image(systemName: "chart.xyaxis.line").font(.largeTitle).foregroundStyle(.tertiary))
        }
    }

    /// Time of day inside a day, month and day over longer windows.
    private var axisFormat: Date.FormatStyle {
        switch range {
        case .hour, .day: .dateTime.hour().minute()
        case .week, .all: .dateTime.month(.defaultDigits).day()
        }
    }
}

private struct VerifiedBadge: View {
    @EnvironmentObject var model: WalletModel

    var body: some View {
        if let keyError = model.keyError {
            Label(keyError, systemImage: "lock.fill")
                .font(.aeCaption.weight(.semibold)).foregroundStyle(Color.warn)
                .padding(.horizontal, 10).padding(.vertical, 4)
                .help("The wallet key lives in this device's Secure Enclave, which only creates keys while the device is unlocked. Aether retries by itself.")
        } else if let since = model.chainPausedSince {
            NetworkPausedBadge(since: since)
        } else if model.account != nil && model.verifyError == nil {
            Label("Verified", systemImage: "checkmark.shield.fill")
                .font(.aeCaption.weight(.semibold)).foregroundStyle(.secondary)
                .padding(.horizontal, 10).padding(.vertical, 4)
                .help("This device checked the balance itself against the validators' signature, using the committee key shipped with the app instead of a server's word.")
        } else if model.networkOutdated {
            Label("This app is out of date · updating", systemImage: "arrow.down.circle.fill")
                .font(.aeCaption.weight(.semibold)).foregroundStyle(Color.warn)
                .padding(.horizontal, 10).padding(.vertical, 4)
                .background(Color.warn.opacity(0.14), in: Capsule())
                .help("The network moved to a new version. The update is being fetched; it applies on the next launch.")
        } else {
            let slow = model.verifyFailingSince.map { Date().timeIntervalSince($0) > 20 } ?? false
            HStack(spacing: 7) {
                OrbitSpinner().frame(width: 13, height: 13)
                Text(model.status == nil ? "Connecting" : slow ? "Still verifying" : "Verifying")
            }
            .font(.aeCaption.weight(.semibold)).foregroundStyle(Color.aether)
            .padding(.horizontal, 11).padding(.vertical, 5)
            .background(Color.aether.opacity(0.10), in: Capsule())
            .help(model.verifyError ?? "Checking the balance against the validators' signature on this device.")
        }
    }
}

private struct NetworkCard: View {
    @EnvironmentObject var model: WalletModel
    /// Sorted (and the busiest block's txs) kept out of `body`, so a window resize
    /// re-layouts the chart without re-sorting and re-feeding it data.
    @State private var blocks: [BlockInfo] = []
    @State private var busiest: UInt32 = 0

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 12) {
                Text("Activity on the network").font(.aeHeadline)
                if !blocks.isEmpty {
                    Text("Transactions in the last \(blocks.count) blocks").font(.aeFootnote).foregroundStyle(.secondary)
                    Chart(blocks, id: \.height) { b in
                        // Empty blocks show as a short stub so the rhythm of blocks stays visible.
                        BarMark(x: .value("Block", String(b.height)), y: .value("Txs", b.txs > 0 ? Double(b.txs) : Double(max(busiest, 1)) * 0.04))
                            .foregroundStyle(b.txs > 0 ? Color.aether : Color.secondary.opacity(0.35))
                            .cornerRadius(2)
                    }
                    .chartXAxis(.hidden)
                    .chartYScale(domain: 0...Double(max(busiest, 1)))
                    .chartYAxis { AxisMarks(values: .automatic(desiredCount: 3)) }
                    .frame(height: 140)
                    if busiest == 0 {
                        Text("Quiet: no transactions right now.").font(.aeCaption).foregroundStyle(.secondary)
                    }
                }
            }
        }
        .onAppear { sync(model.blocks) }
        .onChange(of: model.blocks) { _, fresh in sync(fresh) }
    }

    private func sync(_ fresh: [BlockInfo]) {
        let sorted = fresh.sorted { $0.height < $1.height }
        if sorted != blocks {
            blocks = sorted
            busiest = sorted.map(\.txs).max() ?? 0
        }
    }
}

private struct ActivityRow: View {
    let item: ActivityItem

    private var icon: (String, Color) {
        switch item.kind {
        case .sent: ("arrow.up.right.circle.fill", .secondary)
        case .received: ("arrow.down.left.circle.fill", .green)
        case .security: ("lock.shield.fill", .secondary)
        }
    }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: icon.0).font(.title2).foregroundStyle(icon.1)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.title).font(.aeBody.weight(.medium)).lineLimit(2)
                Text(item.date, style: .relative).font(.aeFootnote).foregroundStyle(.secondary)
                    + Text(" ago").font(.aeFootnote).foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 2) {
                if let a = item.amount {
                    Text("\(a >= 0 ? "+" : "")\(Amount.text(a))").font(.aeBody.weight(.semibold).monospacedDigit())
                        .foregroundStyle(a > 0 ? .green : .primary)
                }
                switch item.state {
                case .pending: Text("Confirming…").font(.aeFootnote).foregroundStyle(Color.warn)
                case .done: Text("Done").font(.aeFootnote).foregroundStyle(.secondary)
                case .failed: Text("Failed").font(.aeFootnote).foregroundStyle(.red)
                }
            }
        }
    }
}


private struct RoundAction: View {
    let title: String
    let icon: String
    let action: () -> Void
    @Environment(\.isEnabled) private var enabled

    var body: some View {
        Button(action: action) {
            VStack(spacing: 8) {
                Image(systemName: icon).font(.aeTitle)
                    .frame(width: 54, height: 54)
                    .background(Color.aether.opacity(0.14), in: Circle())
                    .foregroundStyle(Color.aether)
                Text(title).font(.aeFootnote.weight(.medium)).foregroundStyle(.primary)
            }
            .opacity(enabled ? 1 : 0.4)
        }
        .buttonStyle(.plain)
    }
}

struct TokenRow: View {
    let symbol: String
    let name: String
    let amount: Double?
    let verified: Bool

    var body: some View {
        HStack(spacing: 12) {
            Text("Æ").font(.aeHeadline).foregroundStyle(.white)
                .frame(width: 40, height: 40)
                .background(LinearGradient(colors: [.aether, .pink], startPoint: .topLeading, endPoint: .bottomTrailing), in: Circle())
            VStack(alignment: .leading, spacing: 2) {
                Text(name).font(.aeBody.weight(.semibold))
                HStack(spacing: 4) {
                    Text(symbol).font(.aeFootnote).foregroundStyle(.secondary)
                    if verified { Label("Verified", systemImage: "checkmark.shield.fill").font(.aeCaption).foregroundStyle(.secondary) }
                }
            }
            Spacer()
            Text(amount.map { "\(Amount.text($0)) \(symbol)" } ?? "—").font(.aeBody.weight(.semibold).monospacedDigit())
        }
    }
}

private struct Tile: View {
    let value: String
    let label: String
    let icon: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Image(systemName: icon).foregroundStyle(Color.aether)
            Text(value).font(.aeTitle.monospacedDigit()).lineLimit(1).minimumScaleFactor(0.6)
            Text(label).font(.aeFootnote).foregroundStyle(.secondary)
        }
        .padding(CardPadding.narrow)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
    }
}

private struct ActivityList: View {
    @EnvironmentObject var model: WalletModel
    let limit: Int

    var body: some View {
        let items = Array(model.activity.prefix(limit))
        if items.isEmpty {
            Text("Nothing yet. Payments you send and receive show up here.").font(.aeBody).foregroundStyle(.secondary)
        } else {
            VStack(spacing: 10) {
                ForEach(items) { item in
                    ActivityRow(item: item)
                    if item.id != items.last?.id { Divider() }
                }
            }
        }
    }
}

// MARK: - Sheets

private struct SendSheet: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.dismiss) private var dismiss

    private var amount: Double? { Double(model.paymentRequest?.amount ?? model.sendAmount) }
    private var balance: Double { model.account.flatMap { Double(Wei.format($0.balanceWei)) } ?? 0 }
    private var recipient: String { model.paymentRequest?.to ?? model.sendTo }
    private var valid: Bool { !recipient.isEmpty && (amount ?? 0) > 0 && (amount ?? 0) <= balance }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Send AETH").font(.aeTitle)
            if let r = model.paymentRequest {
                Label(r.memo.map { "A page asked for this payment: \($0)" } ?? "A page asked for this payment. Check the address and amount.", systemImage: "link")
                    .font(.aeBody).foregroundStyle(Color.warn)
                    .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.warn.opacity(0.14), in: RoundedRectangle(cornerRadius: Radius.inner))
            }
            if let r = model.paymentRequest {
                // A requested payment is shown as asked and cannot be edited here.
                VStack(alignment: .leading, spacing: 6) {
                    Text("To").font(.aeFootnote).foregroundStyle(.secondary)
                    Text(r.to).font(.aeBody.monospaced()).textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                    Text("Amount").font(.aeFootnote).foregroundStyle(.secondary).padding(.top, 6)
                    Text("\(r.amount) AETH").font(.aeTitle.monospacedDigit())
                }
            } else {
                VStack(alignment: .leading, spacing: 6) {
                    Text("To").font(.aeFootnote).foregroundStyle(.secondary)
                    TextField("0x… (several: separate with commas)", text: $model.sendTo)
                        .textFieldStyle(.roundedBorder).font(.aeBody.monospaced())
                }
                VStack(alignment: .leading, spacing: 6) {
                    Text("Amount (each)").font(.aeFootnote).foregroundStyle(.secondary)
                    HStack {
                        TextField("0", text: $model.sendAmount).textFieldStyle(.roundedBorder).font(.aeTitle.monospacedDigit())
                        Text("AETH").foregroundStyle(.secondary)
                        Button("Max") { model.sendAmount = Amount.text(max(0, balance - 0.001)) }.buttonStyle(.borderless)
                    }
                }
            }
            HStack {
                Text("Available").foregroundStyle(.secondary)
                Spacer()
                Text("\(Amount.text(balance)) AETH").monospacedDigit()
            }.font(.aeBody)
            if let s = model.status {
                HStack {
                    Text("Network fee").foregroundStyle(.secondary)
                    Spacer()
                    Text("≈ \(Amount.fee(s.transferFeeWei))").monospacedDigit()
                }.font(.aeBody)
            }
            HStack {
                Button("Cancel") {
                    model.paymentRequest = nil
                    dismiss()
                }.keyboardShortcut(.cancelAction)
                Spacer()
                Button {
                    model.send()
                    dismiss()
                } label: { Label("Send", systemImage: "touchid").frame(minWidth: 100) }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!valid || model.busy)
            }
        }
        .padding(24)
        .macMinSize(width: 420)
        .sheetScroll()
    }
}

/// A page asks to sign a contract call: what it does, where to, how much; Touch ID to approve.
private struct CallSheet: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Approve a request").font(.aeTitle)
            if let r = model.callRequest {
                Label("\(r.origin ?? "A page") asks you to sign this. Check it before you approve.", systemImage: "link")
                    .font(.aeBody).foregroundStyle(Color.warn)
                    .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.warn.opacity(0.14), in: RoundedRectangle(cornerRadius: Radius.inner))
                row("Action", r.method)
                if !r.to.isEmpty { row("Contract", r.to, mono: true) }
                row("Sends", "\(r.value) AETH")
                if let m = r.memo { row("Note", m) }
                DisclosureGroup("Call data (\((r.data.count - 2) / 2) bytes)") {
                    ScrollView { Text(r.data).font(.aeFootnote.monospaced()).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
                        .frame(maxHeight: 120)
                }.font(.aeFootnote)
            }
            HStack {
                Button("Reject") {
                    model.callRequest = nil
                    dismiss()
                }.keyboardShortcut(.cancelAction)
                Spacer()
                Button {
                    model.approveCall()
                    dismiss()
                } label: { Label("Approve", systemImage: "touchid").frame(minWidth: 100) }
                    .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                    .disabled(model.busy || model.callRequest == nil)
            }
        }
        .padding(24)
        .macMinSize(width: 460)
        .sheetScroll()
    }

    private func row(_ k: String, _ v: String, mono: Bool = false) -> some View {
        HStack(alignment: .top) {
            Text(k).foregroundStyle(.secondary).frame(width: 72, alignment: .leading)
            Text(v).font(mono ? .body.monospaced() : .body).textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
        }.font(.aeBody)
    }
}

/// A page asks for this wallet's address.
private struct ConnectSheet: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Connect").font(.aeTitle)
            Text("\(model.connectRequest?.origin ?? "A page") wants to see your address \(Short.address(model.address)). It cannot move funds: every payment or call still asks you here.")
                .font(.aeBody).foregroundStyle(.secondary)
            HStack {
                Button("Cancel") {
                    model.connectRequest = nil
                    dismiss()
                }.keyboardShortcut(.cancelAction)
                Spacer()
                Button("Connect") {
                    model.approveConnect()
                    dismiss()
                }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .macMinSize(width: 420)
        .sheetScroll()
    }
}

private struct ReceiveSheet: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.dismiss) private var dismiss
    @State private var copied = false

    var body: some View {
        VStack(spacing: 16) {
            Text("Receive AETH").font(.aeTitle)
            QRCode(text: model.address).frame(maxWidth: 200, maxHeight: 200).aspectRatio(1, contentMode: .fit)
            Text(model.address).font(.aeBody.monospaced()).multilineTextAlignment(.center).textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Button {
                    Clipboard.copy(model.address)
                    copied = true
                } label: { Label(copied ? "Copied" : "Copy address", systemImage: copied ? "checkmark" : "doc.on.doc") }
                    .buttonStyle(.borderedProminent)
                Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
            }
        }
        .padding(24)
        .macMinSize(width: 380)
        .sheetScroll()
    }
}

private struct RecoveryPanel: View {
    @EnvironmentObject var model: WalletModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Recovery device").font(.aeHeadline)
            Text("If you lose this device, a second device you trust (your other Mac or iPhone) can move your funds to itself.")
                .font(.aeBody).foregroundStyle(.secondary)
            step(1, "On the other device, copy its code", "Open Aether there, tap Recovery device, and copy \"This device's code\".")
            HStack {
                TextField("Paste the other device's code", text: $model.guardianInput).textFieldStyle(.roundedBorder).font(.aeFootnote.monospaced())
                Button("Trust it") { model.setRecoveryKey() }.buttonStyle(.borderedProminent).disabled(model.busy || model.guardianInput.isEmpty)
            }
            Button("Remove all my recovery devices and words") { model.removeRecoveryKeys() }
                .font(.aeFootnote).disabled(model.busy)
                .help("Use this if a recovery device or your recovery words may be in someone else's hands, then add trusted ones again.")
            Divider()
            step(2, "This device's code", "Give it to someone who wants this device as their recovery device.")
            HStack {
                Text(model.recoveryCode.isEmpty ? "…" : "\(model.recoveryCode.prefix(24))…").font(.aeFootnote.monospaced())
                    .lineLimit(1).truncationMode(.middle)
                Spacer()
                Button { Clipboard.copy(model.recoveryCode) } label: { Label("Copy", systemImage: "doc.on.doc") }.disabled(model.recoveryCode.isEmpty)
            }
            Divider()
            step(3, "Recover a lost account", "Only works if that account trusted this device. The funds move after its safety delay (48 h by default); its owner can stop it meanwhile.")
            if let p = model.outgoingRecovery {
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Recovering \(Short.address(p.request.lost)): \(Wei.format(p.request.valueWei)) AETH").font(.aeBody.weight(.medium))
                        Text(p.isReady ? "Ready to finish" : "Can finish \(p.readyAt.formatted(date: .abbreviated, time: .shortened))")
                            .font(.aeFootnote).foregroundStyle(p.isReady ? Color.aether : .secondary)
                    }
                    Spacer()
                    Button("Finish recovery") { model.finishRecovery() }.buttonStyle(.borderedProminent).disabled(model.busy || !p.isReady)
                }
            } else {
                HStack {
                    TextField("0x lost account", text: $model.lostInput).textFieldStyle(.roundedBorder).font(.aeFootnote.monospaced())
                    Button("Start recovery") { model.recover() }.disabled(model.busy || model.lostInput.isEmpty)
                }
                Text("Lost every device? Use your 24 recovery words instead (with the lost account above).").font(.aeFootnote).foregroundStyle(.secondary)
                HStack {
                    SecureField("24 recovery words", text: $model.paperWordsInput).textFieldStyle(.roundedBorder).font(.aeFootnote.monospaced())
                    Button("Recover with words") { model.recoverWithWords() }
                        .disabled(model.busy || model.lostInput.isEmpty || model.paperWordsInput.split(separator: " ").count != 24)
                }
            }
        }
    }

    private func step(_ n: Int, _ title: String, _ detail: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Text("\(n)").font(.aeFootnote.bold()).frame(width: 22, height: 22).background(Color.aether.opacity(0.15), in: Circle())
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.aeBody.weight(.semibold))
                Text(detail).font(.aeFootnote).foregroundStyle(.secondary)
            }
        }
    }
}

private struct QRCode: View {
    let text: String

    var body: some View {
        if let img = Self.render(text) {
            Image(decorative: img, scale: 1).interpolation(.none).resizable().scaledToFit()
        } else {
            Image(systemName: "qrcode").resizable().scaledToFit().foregroundStyle(.tertiary)
        }
    }

    static func render(_ s: String) -> CGImage? {
        guard !s.isEmpty else { return nil }
        let f = CIFilter.qrCodeGenerator()
        f.message = Data(s.utf8)
        f.correctionLevel = "M"
        guard let out = f.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)) else { return nil }
        return CIContext().createCGImage(out, from: out.extent)
    }
}

// MARK: - Formatting

enum Amount {
    static func text(_ v: Double) -> String {
        v.formatted(.number.precision(.fractionLength(0...4)))
    }

    /// Fee in AETH with enough digits to be non-zero.
    static func fee(_ wei: String) -> String {
        let aeth = (Double(wei) ?? 0) / 1e18
        if aeth == 0 { return "free" }
        return "\(aeth.formatted(.number.precision(.significantDigits(1...3)))) AETH"
    }
}
