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
        case send, receive, call, connect, votingInvite
        var id: String { rawValue }
    }

    var body: some View {
        shell
            .tint(.aether)
            .sheet(item: $sheet) { s in
                switch s {
                case .send: SendSheet()
                case .receive: ReceiveSheet()
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
                pageView(page ?? .home)
                    .padding(compact ? 16 : 28)
                    .frame(maxWidth: 820)
                    .frame(maxWidth: .infinity)
            }
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
        case .home: HomePage(sheet: $sheet) { page = .activity }
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
    @Environment(\.narrowLayout) private var narrow

    private var balance: Double { model.account.flatMap { Double(Wei.format($0.balanceWei)) } ?? 0 }

    var body: some View {
        VStack(spacing: 22) {
            IncomingRecoveryAlert()
            #if os(macOS)
            NodeEarningsCard()
            #endif
            hero
            HStack(spacing: narrow ? 20 : 28) {
                RoundAction(title: "Receive", icon: "qrcode") { sheet = .receive }.disabled(model.address.isEmpty)
                RoundAction(title: "Send", icon: "paperplane.fill") { sheet = .send }.disabled(model.busy || model.account == nil)
                RoundAction(title: "Get AETH", icon: "drop.fill") { model.faucet() }.disabled(model.busy || model.address.isEmpty)
            }
            BalanceCard()
            Card {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Tokens").font(.headline)
                    TokenRow(symbol: "AETH", name: "Aether", amount: model.account == nil ? nil : balance, verified: model.verifyError == nil && model.account != nil)
                }
            }
            Card {
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Text("Recent activity").font(.headline)
                        Spacer()
                        if !model.activity.isEmpty { Button("See all", action: showActivity).buttonStyle(.borderless) }
                    }
                    ActivityList(limit: 3)
                }
            }
        }
    }

    private var hero: some View {
        VStack(spacing: 8) {
            Button {
                Clipboard.copy(model.address)
            } label: {
                HStack(spacing: 6) {
                    Circle().fill(LinearGradient(colors: [.aether, .pink], startPoint: .topLeading, endPoint: .bottomTrailing)).frame(width: 20, height: 20)
                    Text("Account 1").font(.callout.weight(.semibold)).lineLimit(1)
                    Text(Short.address(model.address)).font(.callout.monospaced()).foregroundStyle(.secondary)
                        .lineLimit(1).truncationMode(.middle)
                    Image(systemName: "doc.on.doc").font(.caption).foregroundStyle(.secondary)
                }
                .padding(.horizontal, 12).padding(.vertical, 6)
                .background(.background.secondary, in: Capsule())
            }
            .buttonStyle(.plain)
            .help("Copy address")
            if model.account == nil {
                // Loading: a soft shimmer where the balance will appear.
                ShimmerBar().frame(maxWidth: 220).frame(height: 52)
            } else {
                Text("\(Amount.text(balance)) AETH")
                    .font(.system(size: 52, weight: .bold, design: .rounded))
                    .lineLimit(1)
                    .minimumScaleFactor(0.4)
                    .contentTransition(.numericText())
            }
            VerifiedBadge()
        }
        .padding(.top, 8)
    }
}

private struct ActivityPage: View {
    var body: some View {
        Card { ActivityList(limit: 100) }
    }
}

private struct NetworkPage: View {
    @EnvironmentObject var model: WalletModel

    @Environment(\.narrowLayout) private var narrow

    private var blockTime: String {
        let b = model.blocks.sorted { $0.height < $1.height }
        guard b.count > 1, let first = b.first, let last = b.last, last.timestampMs > first.timestampMs else { return "—" }
        let s = Double(last.timestampMs - first.timestampMs) / 1000 / Double(b.count - 1)
        return String(format: "%.1f s", s)
    }

    var body: some View {
        VStack(spacing: 16) {
            Card {
                HStack(spacing: 16) {
                    Image(systemName: model.status == nil ? "antenna.radiowaves.left.and.right.slash" : "checkmark.circle.fill")
                        .font(.system(size: narrow ? 30 : 40)).foregroundStyle(model.status == nil ? Color.orange : Color.green)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(model.status == nil ? "Connecting to Aether…" : "Connected to Aether")
                            .font(narrow ? .title3.bold() : .title2.bold())
                        Text("Found the validators on the public DHT. Your balance is checked on this device against their group signature.")
                            .font(.callout).foregroundStyle(.secondary)
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
            NodeCard()
            UpdateCard()
            #endif
        }
    }
}

/// Shown when someone started recovering THIS account: cancel it if it was not you.
private struct IncomingRecoveryAlert: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.narrowLayout) private var narrow

    var body: some View {
        if let r = model.incomingRecovery {
            AdaptiveStack(spacing: 12) {
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: "exclamationmark.shield.fill").font(.title).foregroundStyle(.orange)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Your recovery devices started moving your funds").font(.headline)
                        Text("If this was not you, cancel it. It can run after \(Date(timeIntervalSince1970: TimeInterval(r.readyAt)).formatted(date: .abbreviated, time: .shortened)).")
                            .font(.callout).foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                VStack(alignment: narrow ? .leading : .trailing, spacing: 6) {
                    Button("Cancel it") { model.cancelIncomingRecovery() }.buttonStyle(.borderedProminent).tint(.orange).disabled(model.busy)
                    // A recovery you did not start means a recovery key is in other hands.
                    Button("Cancel and remove all recovery keys") { model.removeRecoveryKeys() }.font(.caption).disabled(model.busy)
                }
                .padding(.leading, narrow ? 40 : 0)
            }
            .padding(16)
            .background(Color.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
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
                        Text("Protected by this device").font(.title3.bold())
                        Text("Your key was created inside the Secure Enclave and can never be copied out. Every payment asks for Touch ID or your password. There is no seed phrase to lose.")
                            .font(.callout).foregroundStyle(.secondary)
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
            Text("Recovery words").font(.title3.bold())
            Text("Your key never leaves this device, so there is no seed phrase to back up. Instead, write down 24 recovery words: if you lose every device, they move your funds to a new Mac after a 48-hour safety delay. If someone else finds them, they can only start that delay, and any of your devices can cancel it.")
                .font(.callout).foregroundStyle(.secondary)
            if let words = model.paperWords {
                let list = words.split(separator: " ").map(String.init)
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), alignment: .leading), count: narrow ? 2 : 4), alignment: .leading, spacing: 6) {
                    ForEach(Array(list.enumerated()), id: \.offset) { i, w in
                        Text("\(i + 1). \(w)").font(.callout.monospaced()).lineLimit(1).minimumScaleFactor(0.7)
                    }
                }
                .padding(12)
                .background(.background.tertiary, in: RoundedRectangle(cornerRadius: 10))
                Text("Write them on paper, in order. Do not photograph or store them on this device.").font(.caption).foregroundStyle(.orange)
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
                        Text("Run a node on this Mac").font(.headline)
                        Text("Your Mac checks every block itself and your wallet asks it instead of the network. It stops when you quit Aether.")
                            .font(.callout).foregroundStyle(.secondary)
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
                    Text("Aether \(updates.version)").font(.headline)
                    // Re-read every half minute so "checked 1 hour ago" stays true.
                    TimelineView(.periodic(from: .now, by: 30)) { context in
                        Text(checked(at: context.date)).font(.callout).foregroundStyle(.secondary)
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
            Image(systemName: icon).font(.system(size: 26)).foregroundStyle(node.voting?.voting == true ? Color.green : Color.aether)
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.headline)
                Text(detail).font(.callout).foregroundStyle(.secondary)
                switch model.registration {
                case .working?:
                    HStack(spacing: 7) { OrbitSpinner().frame(width: 14, height: 14); Text("Registering… confirm with Touch ID.") }.font(.callout)
                case .failed(let why)?:
                    Label(why, systemImage: "exclamationmark.triangle.fill").font(.callout).foregroundStyle(.orange)
                        .fixedSize(horizontal: false, vertical: true)
                case nil:
                    EmptyView()
                }
                Text("Mainnet: online Macs share the block rewards every hour, at most 1/\(VotingRules.mainnetIssuanceOperators) per operator. No sale, no founder share.")
                    .font(.caption).foregroundStyle(.secondary)
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
        case .some: "One Mac, one voting node. Your Mac proves it is alive every epoch; the longest-running Macs are picked to sign blocks, and no owner can hold a third."
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
                    Text("Node on this Mac").font(.caption.weight(.semibold))
                    Text(nodeLine).font(.caption2).foregroundStyle(.secondary).lineLimit(2)
                }
            }
            .toggleStyle(.switch)
            .controlSize(.small)
            .help("Verify every block on this Mac and let the wallet use it. Off when the app quits.")
            EarningsSidebarBadge()
            #endif
            HStack(spacing: 8) {
                Circle().fill(model.status == nil ? Color.orange : Color.green).frame(width: 8, height: 8)
                VStack(alignment: .leading, spacing: 1) {
                    Text(model.status == nil ? "Connecting" : "Connected").font(.caption.weight(.semibold))
                    Text(model.status.map { "Block #\($0.height)" } ?? "Searching DHT…").font(.caption2).foregroundStyle(.secondary)
                }
                Spacer()
            }
        }
    }

    #if os(macOS)
    private var nodeLine: String {
        switch node.state {
        case .off: "Off"
        case .starting: node.height > 0 ? "Catching up · block #\(node.height)" : "Starting…"
        case .running: "Verifying · block #\(node.height)"
        case .waitingForPower: "Paused on battery"
        case .failed(let m): m
        }
    }
    #endif
}

// MARK: - Cards

private struct Card<Content: View>: View {
    let content: Content
    init(@ViewBuilder _ content: () -> Content) { self.content = content() }

    var body: some View {
        content
            .padding(18)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.background.secondary, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
    }
}

private struct BalanceCard: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.narrowLayout) private var narrow
    @State private var range: Range = .day

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

    private var points: [BalancePoint] {
        guard let s = range.seconds else { return model.history }
        let from = Date().addingTimeInterval(-s)
        let inRange = model.history.filter { $0.date >= from }
        // Carry the last earlier value in so the line starts at the left edge.
        var pts = inRange
        if let before = model.history.last(where: { $0.date < from }) {
            pts.insert(BalancePoint(date: from, aeth: before.aeth), at: 0)
        }
        // Extend the last known balance to now, so even one observation draws a line.
        if let last = pts.last, Date().timeIntervalSince(last.date) > 1 {
            pts.append(BalancePoint(date: Date(), aeth: last.aeth))
        }
        return pts
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
        if let first = points.first, points.count > 1 {
            let d = balance - first.aeth
            Label("\(d >= 0 ? "+" : "")\(Amount.text(d)) AETH in \(range == .all ? "total" : range.rawValue)",
                  systemImage: d >= 0 ? "arrow.up.right" : "arrow.down.right")
                .font(.callout.weight(.medium))
                .foregroundStyle(d >= 0 ? .green : .red)
        } else {
            Text("Your balance history appears here as it changes.").font(.callout).foregroundStyle(.secondary)
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
            .chartXAxis { AxisMarks(values: .automatic(desiredCount: 4)) }
        } else {
            RoundedRectangle(cornerRadius: 10).fill(.quaternary.opacity(0.5))
                .overlay(Image(systemName: "chart.xyaxis.line").font(.largeTitle).foregroundStyle(.tertiary))
        }
    }
}

private struct VerifiedBadge: View {
    @EnvironmentObject var model: WalletModel

    var body: some View {
        if model.account != nil && model.verifyError == nil {
            Label("Verified", systemImage: "checkmark.shield.fill")
                .font(.caption.weight(.semibold)).foregroundStyle(.green)
                .padding(.horizontal, 10).padding(.vertical, 4)
                .background(.green.opacity(0.12), in: Capsule())
                .help("This device checked the balance itself against the validators' signature. No server was trusted.")
        } else if model.networkOutdated {
            Label("This app is out of date · updating", systemImage: "arrow.down.circle.fill")
                .font(.caption.weight(.semibold)).foregroundStyle(.orange)
                .padding(.horizontal, 10).padding(.vertical, 4)
                .background(.orange.opacity(0.12), in: Capsule())
                .help("The network moved to a new version. The update is being fetched; it applies on the next launch.")
        } else {
            let slow = model.verifyFailingSince.map { Date().timeIntervalSince($0) > 20 } ?? false
            HStack(spacing: 7) {
                OrbitSpinner().frame(width: 13, height: 13)
                Text(model.status == nil ? "Connecting" : slow ? "Still verifying" : "Verifying")
            }
            .font(.caption.weight(.semibold)).foregroundStyle(Color.aether)
            .padding(.horizontal, 11).padding(.vertical, 5)
            .background(Color.aether.opacity(0.10), in: Capsule())
            .help(model.verifyError ?? "Checking the balance against the validators' signature on this device.")
        }
    }
}

private struct NetworkCard: View {
    @EnvironmentObject var model: WalletModel

    private var blocks: [BlockInfo] { model.blocks.sorted { $0.height < $1.height } }

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 12) {
                Text("Activity on the network").font(.headline)
                if !blocks.isEmpty {
                    let busiest = blocks.map(\.txs).max() ?? 0
                    Text("Transactions in the last \(blocks.count) blocks").font(.caption).foregroundStyle(.secondary)
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
                        Text("Quiet: no transactions right now.").font(.caption2).foregroundStyle(.tertiary)
                    }
                }
            }
        }
    }
}

private struct ActivityRow: View {
    let item: ActivityItem

    private var icon: (String, Color) {
        switch item.kind {
        case .sent: ("arrow.up.right.circle.fill", .blue)
        case .received: ("arrow.down.left.circle.fill", .green)
        case .security: ("lock.shield.fill", .purple)
        }
    }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: icon.0).font(.title2).foregroundStyle(icon.1)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.title).font(.callout.weight(.medium)).lineLimit(2)
                Text(item.date, style: .relative).font(.caption).foregroundStyle(.secondary)
                    + Text(" ago").font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 2) {
                if let a = item.amount {
                    Text("\(a >= 0 ? "+" : "")\(Amount.text(a))").font(.callout.weight(.semibold).monospacedDigit())
                        .foregroundStyle(a >= 0 ? .green : .primary)
                }
                switch item.state {
                case .pending: Text("Confirming…").font(.caption).foregroundStyle(.orange)
                case .done: Text("Done").font(.caption).foregroundStyle(.secondary)
                case .failed: Text("Failed").font(.caption).foregroundStyle(.red)
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
                Image(systemName: icon).font(.title3.weight(.semibold))
                    .frame(width: 54, height: 54)
                    .background(Color.aether.opacity(0.14), in: Circle())
                    .foregroundStyle(Color.aether)
                Text(title).font(.caption.weight(.medium)).foregroundStyle(.primary)
            }
            .opacity(enabled ? 1 : 0.4)
        }
        .buttonStyle(.plain)
    }
}

private struct TokenRow: View {
    let symbol: String
    let name: String
    let amount: Double?
    let verified: Bool

    var body: some View {
        HStack(spacing: 12) {
            Text("Æ").font(.headline.bold()).foregroundStyle(.white)
                .frame(width: 40, height: 40)
                .background(LinearGradient(colors: [.aether, .pink], startPoint: .topLeading, endPoint: .bottomTrailing), in: Circle())
            VStack(alignment: .leading, spacing: 2) {
                Text(name).font(.callout.weight(.semibold))
                HStack(spacing: 4) {
                    Text(amount.map { "\(Amount.text($0)) \(symbol)" } ?? "—").font(.caption).foregroundStyle(.secondary)
                    if verified { Image(systemName: "checkmark.seal.fill").font(.caption2).foregroundStyle(.green) }
                }
            }
            Spacer()
            Text(amount.map(Amount.text) ?? "—").font(.callout.weight(.semibold).monospacedDigit())
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
            Text(value).font(.title3.weight(.semibold).monospacedDigit()).lineLimit(1).minimumScaleFactor(0.6)
            Text(label).font(.caption).foregroundStyle(.secondary)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
}

private struct ActivityList: View {
    @EnvironmentObject var model: WalletModel
    let limit: Int

    var body: some View {
        let items = Array(model.activity.prefix(limit))
        if items.isEmpty {
            Text("Nothing yet. Tap Get AETH to receive test tokens.").font(.callout).foregroundStyle(.secondary)
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
            Text("Send AETH").font(.title2.bold())
            if let r = model.paymentRequest {
                Label(r.memo.map { "A page asked for this payment: \($0)" } ?? "A page asked for this payment. Check the address and amount.", systemImage: "link")
                    .font(.callout).foregroundStyle(.orange)
                    .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                    .background(.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
            }
            if let r = model.paymentRequest {
                // A requested payment is shown as asked and cannot be edited here.
                VStack(alignment: .leading, spacing: 6) {
                    Text("To").font(.caption).foregroundStyle(.secondary)
                    Text(r.to).font(.body.monospaced()).textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                    Text("Amount").font(.caption).foregroundStyle(.secondary).padding(.top, 6)
                    Text("\(r.amount) AETH").font(.title3.weight(.semibold).monospacedDigit())
                }
            } else {
                VStack(alignment: .leading, spacing: 6) {
                    Text("To").font(.caption).foregroundStyle(.secondary)
                    TextField("0x… (several: separate with commas)", text: $model.sendTo)
                        .textFieldStyle(.roundedBorder).font(.body.monospaced())
                }
                VStack(alignment: .leading, spacing: 6) {
                    Text("Amount (each)").font(.caption).foregroundStyle(.secondary)
                    HStack {
                        TextField("0", text: $model.sendAmount).textFieldStyle(.roundedBorder).font(.title3.monospacedDigit())
                        Text("AETH").foregroundStyle(.secondary)
                        Button("Max") { model.sendAmount = Amount.text(max(0, balance - 0.001)) }.buttonStyle(.borderless)
                    }
                }
            }
            HStack {
                Text("Available").foregroundStyle(.secondary)
                Spacer()
                Text("\(Amount.text(balance)) AETH").monospacedDigit()
            }.font(.callout)
            if let s = model.status {
                HStack {
                    Text("Network fee").foregroundStyle(.secondary)
                    Spacer()
                    Text("≈ \(Amount.fee(s.transferFeeWei))").monospacedDigit()
                }.font(.callout)
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
            Text("Approve a request").font(.title2.bold())
            if let r = model.callRequest {
                Label("\(r.origin ?? "A page") asks you to sign this. Check it before you approve.", systemImage: "link")
                    .font(.callout).foregroundStyle(.orange)
                    .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                    .background(.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
                row("Action", r.method)
                if !r.to.isEmpty { row("Contract", r.to, mono: true) }
                row("Sends", "\(r.value) AETH")
                if let m = r.memo { row("Note", m) }
                DisclosureGroup("Call data (\((r.data.count - 2) / 2) bytes)") {
                    ScrollView { Text(r.data).font(.caption.monospaced()).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
                        .frame(maxHeight: 120)
                }.font(.caption)
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
        }.font(.callout)
    }
}

/// A page asks for this wallet's address.
private struct ConnectSheet: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Connect").font(.title2.bold())
            Text("\(model.connectRequest?.origin ?? "A page") wants to see your address \(Short.address(model.address)). It cannot move funds: every payment or call still asks you here.")
                .font(.callout).foregroundStyle(.secondary)
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
            Text("Receive AETH").font(.title2.bold())
            QRCode(text: model.address).frame(maxWidth: 200, maxHeight: 200).aspectRatio(1, contentMode: .fit)
            Text(model.address).font(.callout.monospaced()).multilineTextAlignment(.center).textSelection(.enabled)
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
            Text("Recovery device").font(.title3.bold())
            Text("If you lose this device, a second device you trust (your other Mac or iPhone) can move your funds to itself.")
                .font(.callout).foregroundStyle(.secondary)
            step(1, "On the other device, copy its code", "Open Aether there, tap Recovery device, and copy \"This device's code\".")
            HStack {
                TextField("Paste the other device's code", text: $model.guardianInput).textFieldStyle(.roundedBorder).font(.caption.monospaced())
                Button("Trust it") { model.setRecoveryKey() }.buttonStyle(.borderedProminent).disabled(model.busy || model.guardianInput.isEmpty)
            }
            Button("Remove all my recovery devices and words") { model.removeRecoveryKeys() }
                .font(.caption).disabled(model.busy)
                .help("Use this if a recovery device or your recovery words may be in someone else's hands, then add trusted ones again.")
            Divider()
            step(2, "This device's code", "Give it to someone who wants this device as their recovery device.")
            HStack {
                Text(model.recoveryCode.isEmpty ? "…" : "\(model.recoveryCode.prefix(24))…").font(.caption.monospaced())
                    .lineLimit(1).truncationMode(.middle)
                Spacer()
                Button { Clipboard.copy(model.recoveryCode) } label: { Label("Copy", systemImage: "doc.on.doc") }.disabled(model.recoveryCode.isEmpty)
            }
            Divider()
            step(3, "Recover a lost account", "Only works if that account trusted this device. The funds move after its safety delay (48 h by default); its owner can stop it meanwhile.")
            if let p = model.outgoingRecovery {
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Recovering \(Short.address(p.request.lost)): \(Wei.format(p.request.valueWei)) AETH").font(.callout.weight(.medium))
                        Text(p.isReady ? "Ready to finish" : "Can finish \(p.readyAt.formatted(date: .abbreviated, time: .shortened))")
                            .font(.caption).foregroundStyle(p.isReady ? .green : .secondary)
                    }
                    Spacer()
                    Button("Finish recovery") { model.finishRecovery() }.buttonStyle(.borderedProminent).disabled(model.busy || !p.isReady)
                }
            } else {
                HStack {
                    TextField("0x lost account", text: $model.lostInput).textFieldStyle(.roundedBorder).font(.caption.monospaced())
                    Button("Start recovery") { model.recover() }.disabled(model.busy || model.lostInput.isEmpty)
                }
                Text("Lost every device? Use your 24 recovery words instead (with the lost account above).").font(.caption).foregroundStyle(.secondary)
                HStack {
                    SecureField("24 recovery words", text: $model.paperWordsInput).textFieldStyle(.roundedBorder).font(.caption.monospaced())
                    Button("Recover with words") { model.recoverWithWords() }
                        .disabled(model.busy || model.lostInput.isEmpty || model.paperWordsInput.split(separator: " ").count != 24)
                }
            }
        }
    }

    private func step(_ n: Int, _ title: String, _ detail: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Text("\(n)").font(.caption.bold()).frame(width: 22, height: 22).background(Color.aether.opacity(0.15), in: Circle())
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.callout.weight(.semibold))
                Text(detail).font(.caption).foregroundStyle(.secondary)
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
