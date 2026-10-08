import SwiftUI
#if os(macOS)
import Combine
#endif

// "Your Mac is working, and this is what it earned": a loud hero card on the Network page
// (and on Home, under the balance, once a reward has arrived), a badge in the sidebar and a
// line in the menu-bar panel. Loud in looks only: every amount shown is
// a reward the chain actually paid (from `aether_rewards`), never a projection or a price.
//
// The drawing views take plain values, so they compile on iOS too (for the DEBUG preview
// harness); only the data source (`Earnings`) and the wrappers that read the node are macOS.

// MARK: - Values the views draw

/// What the node on this Mac is doing right now.
struct NodeWork: Equatable {
    enum Phase: Equatable {
        case starting
        /// Verifying every block (a node-only Mac: earns nothing yet on testnet).
        case verifying
        /// Proving blocks on the GPU: the first valid proof of a block is paid.
        case proving
        case paused(String)

        /// The live pill's word.
        var pillText: String {
            switch self {
            case .proving: String(localized: "PROVING")
            case .verifying: String(localized: "WORKING")
            case .starting: String(localized: "STARTING")
            case .paused: String(localized: "PAUSED")
            }
        }
    }

    var phase: Phase
    var height: UInt64 = 0
    /// Blocks this node has followed and checked since it started.
    var blocksVerified: UInt64 = 0
    var runningSince: Date?
    /// Proofs made on the GPU this session.
    var proofs: UInt64 = 0
    var proofsFailing = false
    /// Consecutive hours online as a registered voting node (nil: not registered).
    var streakHours: UInt64?
    var votingNow = false
    /// A wallet address exists, so proving can be switched on from the card.
    var canProve = true

    var isProving: Bool { phase == .proving }
    var isLive: Bool { phase == .proving || phase == .verifying }
}

/// A reward that just arrived (bumps `id` each time; drives the burst).
struct RewardCelebration: Equatable {
    let id: Int
    let amountWei: String
}

enum EarningsText {
    static var unit: String { String(localized: "test \(Brand.networkCoinTicker)") }

    static func aeth(_ wei: String) -> String { Wei.format(wei) }

    /// "3m ago", "2h ago".
    static func ago(_ date: Date, now: Date) -> String {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .abbreviated
        return now.timeIntervalSince(date) < 45 ? String(localized: "just now") : f.localizedString(for: date, relativeTo: now)
    }

    /// "3h 12m".
    static func duration(_ seconds: TimeInterval) -> String {
        let f = DateComponentsFormatter()
        f.allowedUnits = seconds >= 3_600 ? [.day, .hour, .minute] : [.minute, .second]
        f.unitsStyle = .abbreviated
        f.maximumUnitCount = 2
        return f.string(from: max(0, seconds)) ?? "–"
    }

    /// Fraction digits for the count-up: as many as the amount needs, at most 4.
    static func decimals(_ wei: String) -> Int {
        let s = Wei.format(wei)
        guard let dot = s.firstIndex(of: ".") else { return 0 }
        return min(4, s.distance(from: dot, to: s.endIndex) - 1)
    }
}

// MARK: - Hero card

/// The big earnings card while the node is on.
struct EarningsHero: View {
    let summary: EarningsSummary
    let work: NodeWork
    var celebration: RewardCelebration?
    /// Turns GPU proving on (nil hides the call to action).
    var onProve: (() -> Void)?
    @Environment(\.narrowLayout) private var narrow

    /// Money is the headline once the Mac has been paid (never a row of zeros before).
    private var showsEarnings: Bool { summary.count > 0 }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Radius.card, style: .continuous)
        VStack(alignment: .leading, spacing: narrow ? 14 : 18) {
            header
            if showsEarnings { EarnedBlock(summary: summary, celebration: celebration) } else { VerifiedBlock(work: work) }
            tiles
            footer
        }
        .foregroundStyle(DesignTokens.Palette.plateInk.color)
        .padding(narrow ? CardPadding.narrow : CardPadding.wide)
        .frame(maxWidth: .infinity, alignment: .leading)
        .eastSeaNavyPlate(cornerRadius: DesignTokens.Radius.lg)
        .dblnRewardShine(arrival: celebration?.id, cornerRadius: DesignTokens.Radius.lg)
        .clipShape(shape)
        .accessibilityElement(children: .combine)
    }

    /// The live indicator sits on the card's own color, on the padding grid;
    /// the coin (the shipped render, 48 pt and up) anchors the money side.
    private var header: some View {
        HStack(alignment: .center) {
            LivePill(text: pillText, live: work.isLive)
            Spacer(minLength: 0)
            TokenIcon(chainId: Brand.networkChainId, address: nil, symbol: Brand.networkCoinTicker, size: 48)
        }
    }

    private var pillText: String {
        work.phase.pillText
    }

    @ViewBuilder private var tiles: some View {
        HStack(spacing: narrow ? 8 : 12) {
            if showsEarnings {
                if summary.todayWei != "0" {
                    StatTile(label: "Today", value: "+\(EarningsText.aeth(summary.todayWei))", unit: EarningsText.unit)
                }
                StatTile(label: "Received", value: "\(summary.count)", unit: summary.count == 1 ? String(localized: "reward") : String(localized: "rewards"))
                TimelineView(.periodic(from: .now, by: 30)) { tl in
                    StatTile(label: "Last reward", value: summary.lastRewardAt.map { EarningsText.ago($0, now: tl.date) } ?? String(localized: "none yet"),
                             unit: summary.latestHeight.map { String(localized: "in block #\(String($0))") } ?? String(localized: "keep proving"))
                }
            } else {
                TimelineView(.periodic(from: .now, by: 30)) { tl in
                    StatTile(label: "Online", value: work.runningSince.map { EarningsText.duration(tl.date.timeIntervalSince($0)) } ?? "–", unit: String(localized: "this session"))
                }
                StatTile(label: work.votingNow ? "Voting" : "Streak", value: streakValue,
                         unit: work.streakHours == nil ? String(localized: "not registered") : String(localized: "hours online"))
                StatTile(label: "Latest block", value: work.height > 0 ? "#\(String(work.height))" : "–", unit: String(localized: "checked here"))
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private var streakValue: String {
        guard let s = work.streakHours else { return "–" }
        return work.votingNow ? String(localized: "\(s) h") : "\(min(s, VotingRules.minStreakEpochs))/\(VotingRules.minStreakEpochs)"
    }

    @ViewBuilder private var footer: some View {
        VStack(alignment: .leading, spacing: 10) {
            footerLine
            if !work.isProving, work.canProve, let onProve {
                ProveCallToAction(action: onProve)
            }
        }
    }

    private var footerLine: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Image(systemName: footerIcon)
            Text(footerText).fixedSize(horizontal: false, vertical: true)
        }
        .font(.aeBody.weight(.medium))
        .foregroundStyle(DesignTokens.Palette.plateSoft.color)
    }

    /// " · 57 proofs this session" (nothing before the first one).
    private var proofsText: String {
        work.proofs == 0 ? "" : " · " + String(localized: "\(work.proofs) proofs this session")
    }

    private var footerIcon: String {
        switch work.phase {
        case .proving: "cpu"
        case .verifying: "checkmark.shield.fill"
        case .starting: "arrow.down.circle"
        case .paused: "pause.circle"
        }
    }

    private var footerText: String {
        switch work.phase {
        case .proving where summary.count > 0:
            String(localized: "This Mac's GPU is proving blocks\(proofsText). Every amount here was paid on chain.")
        case .proving:
            String(localized: "This Mac's GPU is proving blocks\(proofsText). The first valid proof of a block gets a reward.")
        case .verifying:
            String(localized: "Your Mac checks every block itself. Checking alone earns no reward.")
        case .starting:
            work.height > 0 ? String(localized: "Catching up with the network · block #\(String(work.height))") : String(localized: "Starting the node…")
        case .paused(let why):
            why
        }
    }
}

/// "EARNED SO FAR  12.5 test DBLN  +1.5 in the last hour", counting up.
private struct EarnedBlock: View {
    let summary: EarningsSummary
    let celebration: RewardCelebration?
    @Environment(\.narrowLayout) private var narrow

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Received so far").font(.aeFootnote.weight(.semibold)).foregroundStyle(DesignTokens.Palette.plateSoft.color)
            BigNumber(value: WeiMath.aeth(summary.totalWei), decimals: EarningsText.decimals(summary.totalWei),
                      unit: EarningsText.unit)
            if summary.lastHourWei != "0" { HourDelta(wei: summary.lastHourWei) }
        }
    }
}

/// Node-only: the work this Mac did, counting up block by block.
private struct VerifiedBlock: View {
    let work: NodeWork

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Blocks verified this session").font(.aeFootnote.weight(.semibold)).foregroundStyle(DesignTokens.Palette.plateSoft.color)
            BigNumber(value: Double(work.blocksVerified), decimals: 0, unit: work.blocksVerified == 1 ? String(localized: "block") : String(localized: "blocks"))
        }
    }
}

/// Settled on appearance; later changes use the shared finite count-up.
private struct BigNumber: View {
    let value: Double
    let decimals: Int
    let unit: String
    var compact = false
    @Environment(\.narrowLayout) private var narrow
    @Environment(\.locale) private var locale

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: DesignTokens.Space.s2) { number; unitText }
            VStack(alignment: .leading, spacing: 0) { number; unitText }
        }
    }

    private var number: some View {
        BalanceCountUp(amount: Decimal(value), accessibilityText: "\(formatted(value)) \(unit)") {
            formatted(NSDecimalNumber(decimal: $0).doubleValue)
        }
        .font(narrow || compact ? .heroNumberNarrow : .heroNumber)
        .lineLimit(1)
        .minimumScaleFactor(0.5)
    }

    private var unitText: some View {
        Text(unit).font(.aeBody).foregroundStyle(DesignTokens.Palette.plateSoft.color)
    }

    private func formatted(_ number: Double) -> String {
        number.formatted(.number.precision(.fractionLength(decimals)).grouping(.automatic).locale(locale))
    }
}

private struct HourDelta: View {
    let wei: String

    var body: some View {
        let some = wei != "0"
        HStack(spacing: 6) {
            Image(systemName: some ? "plus.circle.fill" : "clock")
            Text(some ? String(localized: "+\(EarningsText.aeth(wei)) \(EarningsText.unit) in the last hour") : String(localized: "Nothing in the last hour"))
        }
        .font(.aeFootnote.weight(.semibold))
        .foregroundStyle(some ? DesignTokens.Palette.plateSuccess.color : DesignTokens.Palette.plateSoft.color)
        .padding(.horizontal, DesignTokens.Space.s3).padding(.vertical, DesignTokens.Space.s1)
        .background(DesignTokens.Palette.plate2.color, in: Capsule())
    }
}

/// Quiet facts on the navy plate.
private struct StatTile: View {
    let label: LocalizedStringKey
    let value: String
    let unit: String?
    @Environment(\.narrowLayout) private var narrow

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label).font(.aeCaption.weight(.semibold)).foregroundStyle(DesignTokens.Palette.plateSoft.color)
                .lineLimit(1).minimumScaleFactor(0.7)
            Text(value).font(narrow ? .aeHeadline : .aeTitle).monospacedDigit()
                .lineLimit(1).minimumScaleFactor(0.55)
                .contentTransition(.numericText())
            if let unit {
                Text(unit).font(.aeCaption).foregroundStyle(DesignTokens.Palette.plateSoft.color).lineLimit(1).minimumScaleFactor(0.7)
            }
        }
        .padding(.horizontal, narrow ? 10 : 14).padding(.vertical, narrow ? 9 : 12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(DesignTokens.Palette.plate2.color, in: RoundedRectangle(cornerRadius: Radius.inner, style: .continuous))
    }
}

/// A static status word and dot; a live node never animates while idle.
struct LivePill: View {
    let text: String
    let live: Bool

    var body: some View {
        HStack(spacing: DesignTokens.Space.s2) {
            Circle().fill(live ? DesignTokens.Palette.plateSuccess.color : DesignTokens.Palette.plateWarn.color)
                .frame(width: 6, height: 6).accessibilityHidden(true)
            Text(text).font(.aeCaption.weight(.semibold))
        }
        .foregroundStyle(DesignTokens.Palette.plateInk.color)
        .padding(.horizontal, DesignTokens.Space.s3).padding(.vertical, DesignTokens.Space.s1)
        .background(DesignTokens.Palette.plate2.color, in: Capsule())
    }
}

/// The one thing a node-only Mac can do to start earning.
private struct ProveCallToAction: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: "bolt.fill").font(.aeHeadline)
                VStack(alignment: .leading, spacing: 1) {
                    Text("Prove blocks on this Mac's GPU").font(.aeHeadline)
                    Text("for test \(Brand.networkCoinTicker) rewards").font(.aeCaption.weight(.semibold)).opacity(0.8)
                }
                Spacer(minLength: 4)
                Image(systemName: "chevron.right").font(.aeHeadline)
            }
            .foregroundStyle(DesignTokens.Palette.sea.color)
            .padding(.horizontal, 16).padding(.vertical, 12)
            .background(DesignTokens.Palette.gold.color, in: RoundedRectangle(cornerRadius: Radius.inner, style: .continuous))
        }
        .buttonStyle(.plain)
        .help("Uses the GPU and power while on, at your cost. The first valid proof of a block gets a test \(Brand.networkCoinTicker) reward in this wallet.")
    }
}

// MARK: - Compact indicators

/// A quiet status capsule: "● Proving · +1.5 today" or "● Working · 42 blocks".
struct EarningsBadge: View {
    let summary: EarningsSummary
    let work: NodeWork

    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(work.isLive && !work.proofsFailing ? DesignTokens.Palette.success.color : DesignTokens.Palette.warn.color)
                .frame(width: 7, height: 7).accessibilityHidden(true)
            Text(line).lineLimit(1).minimumScaleFactor(0.6)
        }
        .font(.aeCaption.weight(.bold))
        .foregroundStyle(DesignTokens.Palette.text.color)
        .padding(.horizontal, 9).padding(.vertical, 5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(DesignTokens.Palette.surfaceSunken.color, in: Capsule())
    }

    /// Fits the sidebar (170–190 pt) without an ellipsis; the cards keep the full wording.
    private var line: String {
        switch work.phase {
        case .proving: ProvingBadgeText.line(proofsFailing: work.proofsFailing, today: EarningsText.aeth(summary.todayWei))
        case .verifying: String(localized: "Working · \(work.blocksVerified) blocks")
        case .starting: String(localized: "Starting…")
        case .paused: String(localized: "Paused")
        }
    }
}

// MARK: - macOS: the data, and the views that read it

#if os(macOS)
/// Polls this Mac's rewards from its own node, sums them and notices new ones.
@MainActor
final class Earnings: ObservableObject {
    @Published private(set) var summary = EarningsSummary.empty
    /// The reward rows behind `summary`, exactly as the node reported them —
    /// the export writes what this app saw, not a fresh re-read
    /// (docs/research/node-reward-tax-2026.md).
    @Published private(set) var entries: [RewardEntry] = []
    @Published private(set) var celebration: RewardCelebration?
    @Published private(set) var runningSince: Date?
    @Published private(set) var firstHeight: UInt64?
    /// The chain's node-rewards standing (`aether_rewardStatus`), for the Network page.
    @Published private(set) var status: RewardStatus?
    /// How many rewards the node says exist for this address in total (nil:
    /// not reported). Below `entries.count` nothing is missing; above it, the
    /// cards say "N of M" instead of quietly showing a capped list.
    @Published private(set) var totalOnChain: Int?

    static let pollSeconds: TimeInterval = 20
    /// The node returns at most this many (the newest).
    static let limit = 10_000

    private weak var node: NodeController?
    private var operatorAddress: () -> String = { "" }
    private var timer: Timer?
    private var subscriptions = Set<AnyCancellable>()
    /// Rewards were read once for `address`; only later arrivals are celebrated.
    private var loaded = false
    private var address = ""
    private var fetching = false
    private var statusFetching = false

    /// Start following `node` (once; later calls do nothing). `operatorAddress`
    /// is the wallet address rewards would be paid to (it can change).
    func attach(_ node: NodeController, operatorAddress: @escaping () -> String) {
        guard self.node == nil else { return }
        #if DEBUG
        if DesignPreview.on {
            if DesignPreview.variant != "verifying" {
                let sample = EarningsPreviewHarness.sampleEntries(count: 24)
                entries = sample
                summary = EarningsSummary.aggregate(sample, now: Date())
            }
            if DesignPreview.rewardStatus { status = RewardStatus(json: DesignPreview.sampleRewardStatus) }
            runningSince = Date().addingTimeInterval(-11_520)
            firstHeight = 182_926
            return
        }
        #endif
        self.operatorAddress = operatorAddress
        self.node = node
        node.$state.sink { [weak self] in self?.stateChanged($0) }.store(in: &subscriptions)
        node.$height.sink { [weak self] in self?.heightChanged($0) }.store(in: &subscriptions)
        // A new proof or reward on the prover: look now instead of waiting for the timer.
        node.$prover.map { $0?.last_reward }.removeDuplicates().dropFirst()
            .sink { [weak self] _ in self?.refreshSoon() }.store(in: &subscriptions)
        AccountStore.wallet().payoutAddressPublisher.removeDuplicates().dropFirst()
            .sink { [weak self] _ in self?.refresh() }.store(in: &subscriptions)
        timer = Timer.scheduledTimer(withTimeInterval: Self.pollSeconds, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
        refresh()
    }

    /// What the node is doing, for the views.
    func work(_ node: NodeController, canProve: Bool) -> NodeWork {
        var w = NodeWork(phase: phase(node))
        if node.prover?.paused == "program" {
            // Plain words (prover-070-mismatch.md C): no program ids, no
            // "validator proof program" — those stay in developer mode.
            w.phase = .paused(String(localized: "This Mac is resting from proving blocks for now: the network cannot check this version's proofs yet. Nothing is lost."))
        }
        w.height = node.height
        w.blocksVerified = firstHeight.map { node.height > $0 ? node.height - $0 : 0 } ?? 0
        w.runningSince = runningSince
        w.proofs = node.prover?.proofs ?? 0
        if w.isProving {
            w.proofsFailing = ProvingBadgeText.failing(
                reported: node.prover?.proofs_failing ?? false,
                proverRunning: node.prover?.running,
                runningSeconds: runningSince.map { Date().timeIntervalSince($0) }
            )
        }
        if let v = node.voting, v.registered {
            w.streakHours = v.streak
            w.votingNow = v.voting
        }
        w.canProve = canProve
        return w
    }

    private func phase(_ node: NodeController) -> NodeWork.Phase {
        // The one stop reason, the same as the sidebar and the menu.
        if let reason = node.stopReason, node.state != .running, node.state != .starting {
            return .paused(reason.copy().paragraph)
        }
        switch node.state {
        case .running: return node.prove ? .proving : .verifying
        case .starting: return .starting
        case .waitingForPower: return .paused(String(localized: "Paused on battery. It resumes on the power adapter."))
        case .off: return .paused(String(localized: "The node is off."))
        case .failed(let why): return .paused(why)
        }
    }

    private func stateChanged(_ s: NodeController.State) {
        switch s {
        case .running:
            if runningSince == nil { runningSince = Date() }
            refreshSoon()
        case .starting:
            break
        case .off, .waitingForPower, .failed:
            runningSince = nil
            firstHeight = nil
        }
    }

    private func heightChanged(_ h: UInt64) {
        guard h > 0, firstHeight == nil, node?.state == .running else { return }
        firstHeight = h
    }

    private func refreshSoon() {
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(1))
            refresh()
        }
    }

    func refresh() {
        guard let node else { return }
        if node.proveAddress != address {
            address = node.proveAddress
            loaded = false
            summary = .empty
            entries = []
            totalOnChain = nil
            status = nil
            celebration = nil
        }
        guard node.state == .running else { return }
        refreshStatus()
        guard !address.isEmpty, !fetching else { return }
        fetching = true
        let addr = address
        Task { @MainActor in
            defer { fetching = false }
            // Every page, not one flat read: the node caps a single response,
            // and a history longer than that cap would silently lose its tail
            // (the founder's 2,583 rewards ran past the default page size).
            let (rows, total) = await NodeController.allRewards(port: NodeController.port, address: addr)
            guard addr == address, addr == node.proveAddress else { refreshSoon(); return }
            guard !rows.isEmpty || (total ?? 0) == 0 else { return }
            let list = rows.compactMap(RewardEntry.init(json:))
            entries = list
            totalOnChain = total
            apply(EarningsSummary.aggregate(list, now: Date()))
        }
    }

    /// The standing behind the rewards (Network page). Asked without an operator
    /// when this Mac has no address to name: N and the cap are still the chain's.
    private func refreshStatus() {
        guard let node, node.state == .running, !statusFetching else { return }
        let op = node.proveAddress.isEmpty ? operatorAddress() : node.proveAddress
        guard !op.isEmpty else { return }
        statusFetching = true
        Task { @MainActor in
            defer { statusFetching = false }
            if let json = await LocalRPC.call(port: NodeController.port, method: "aether_rewardStatus", params: [op]) as? [String: Any] {
                let current = node.proveAddress.isEmpty ? operatorAddress() : node.proveAddress
                guard op == current else { refreshSoon(); return }
                let fresh = RewardStatus(json: json)
                if fresh != status { status = fresh }
            }
        }
    }

    private func apply(_ new: EarningsSummary) {
        if loaded, let amount = new.arrived(since: summary) {
            celebration = RewardCelebration(id: (celebration?.id ?? 0) + 1, amountWei: amount)
        }
        summary = new
        loaded = true
    }
}

/// The hero card on the Network page, while the node switch is on, with what the
/// chain says about node rewards underneath (`aether_rewardStatus`; nothing on a
/// chain without them, like the testnet).
struct NodeEarningsCard: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var earnings: Earnings
    @EnvironmentObject var model: WalletModel

    var body: some View {
        if node.enabled {
            VStack(spacing: 12) {
                EarningsHero(summary: earnings.summary, work: earnings.work(node, canProve: !model.payoutAddress.isEmpty),
                             celebration: earnings.celebration, onProve: proveOn)
                RewardStandingCard(status: earnings.status)
                EarningsExportCard()
            }
        }
    }

    private func proveOn() {
        node.proveAddress = model.payoutAddress
        node.prove = true
    }
}

/// "Export earnings (CSV)" on the Earnings screen: the rows this app saw
/// (the same ones the hero card sums), written to a file the user picks —
/// plus the one line that keeps EastSea honest about what it is
/// (docs/research/node-reward-tax-2026.md).
private struct EarningsExportCard: View {
    @EnvironmentObject var earnings: Earnings

    var body: some View {
        Card {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "square.and.arrow.down").font(.aeTitle).foregroundStyle(Color.aether)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Export earnings (CSV)").font(.aeHeadline)
                    Text("Every reward this Mac earned, as this app saw it — time, kind and the exact amount — for your own records. This is not tax advice.")
                        .font(.aeBody).foregroundStyle(DesignTokens.Palette.textMuted.color)
                        .fixedSize(horizontal: false, vertical: true)
                    if let total = earnings.totalOnChain, total > earnings.entries.count {
                        Text("The node counts \(total) rewards; \(earnings.entries.count) were read. The numbers here cover only what was read.")
                            .font(.aeFootnote).foregroundStyle(Color.warn)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 8)
                Button("Export CSV…") { export() }
                    .buttonStyle(EastSeaQuietButtonStyle())
                    .disabled(earnings.entries.isEmpty)
                    .help("Saves eastsea-earnings.csv from what this app saw. The records never leave this Mac.")
            }
        }
    }

    private func export() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "eastsea-earnings.csv"
        panel.allowedContentTypes = [.commaSeparatedText]
        if panel.runModal() == .OK, let url = panel.url {
            try? EarningsCSV.document(earnings.entries).write(to: url, atomically: true, encoding: .utf8)
        }
    }
}

/// What the chain itself counts (docs/design/15-node-rewards.md): operators
/// online, the 1/16 cap, this Mac's warm-up and its share of the last hour.
/// Testnet answers `enabled: false`, and then nothing is shown here.
struct RewardStandingCard: View {
    let status: RewardStatus?

    var body: some View {
        if let s = status, s.enabled {
            Card {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Node rewards").font(.aeHeadline)
                    Text("Operators online: \(s.operatorsOnline) · one operator gets at most 1/\(s.maxShare) of the rewards")
                        .font(.aeBody).foregroundStyle(DesignTokens.Palette.textMuted.color)
                        .fixedSize(horizontal: false, vertical: true)
                    if let pct = s.warmupPercent, let days = s.warmupDaysLeft, days > 0 {
                        Text("Warm-up: \(pct)% of a full share · full in \(days) days")
                            .font(.aeBody).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    }
                    if let share = s.expectedShareWei, share != "0" {
                        Text(s.capped
                             ? String(localized: "Last hour: +\(EarningsText.aeth(share)) \(EarningsText.unit) · capped at 1/\(s.maxShare)")
                             : String(localized: "Last hour: +\(EarningsText.aeth(share)) \(EarningsText.unit)"))
                            .font(.aeBody).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    }
                }
            }
        }
    }
}

/// Home, under the balance and the actions: once a reward has arrived, the number
/// in a compact card (160–180 pt — the balance above stays the headline, the
/// reward stays vivid); before that, one quiet line (no zeros).
struct HomeEarnings: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var earnings: Earnings
    @EnvironmentObject var model: WalletModel
    /// Opens the page with the node and the full card.
    let open: () -> Void

    var body: some View {
        if node.enabled {
            let work = earnings.work(node, canProve: !model.payoutAddress.isEmpty)
            if earnings.summary.count > 0, earnings.summary.totalWei != "0" {
                HomeEarningsCard(summary: earnings.summary, work: work, celebration: earnings.celebration, open: open)
            } else {
                NodeStatusLine(work: work, action: open, onProve: node.prove || model.payoutAddress.isEmpty ? nil : proveOn)
            }
        }
    }

    /// Same as the Settings toggle "Prove blocks with Metal".
    private func proveOn() {
        node.proveAddress = model.payoutAddress
        node.prove = true
    }
}

/// The compact earnings card for Home: a navy plate and one reward number at
/// 40 pt (never the 48 pt balance's rival), a delta for the last hour, and a
/// line of facts. Tapping opens the Network page, where the full hero lives.
private struct HomeEarningsCard: View {
    let summary: EarningsSummary
    let work: NodeWork
    var celebration: RewardCelebration?
    let open: () -> Void
    @Environment(\.narrowLayout) private var narrow

    private var pillText: String {
        work.phase.pillText
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Radius.card, style: .continuous)
        Button(action: open) {
            VStack(alignment: .leading, spacing: narrow ? 10 : 12) {
                HStack {
                    Text("Received so far").font(.aeFootnote.weight(.semibold)).foregroundStyle(DesignTokens.Palette.plateSoft.color)
                    Spacer(minLength: 8)
                    LivePill(text: pillText, live: work.isLive)
                }
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    BigNumber(value: WeiMath.aeth(summary.totalWei), decimals: EarningsText.decimals(summary.totalWei),
                              unit: EarningsText.unit, compact: true)
                    Spacer(minLength: 8)
                    if summary.lastHourWei != "0" { HourDelta(wei: summary.lastHourWei).fixedSize() }
                }
                Text(facts)
                    .font(.aeFootnote.weight(.medium)).foregroundStyle(DesignTokens.Palette.plateSoft.color)
                    .lineLimit(1).truncationMode(.tail)
            }
            .foregroundStyle(DesignTokens.Palette.plateInk.color)
            .padding(.horizontal, narrow ? CardPadding.narrow : CardPadding.wide)
            .padding(.vertical, narrow ? 20 : 22)
            .frame(maxWidth: .infinity, alignment: .leading)
            .eastSeaNavyPlate(cornerRadius: DesignTokens.Radius.lg)
            .dblnRewardShine(arrival: celebration?.id, cornerRadius: DesignTokens.Radius.lg)
            .clipShape(shape)
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .help("Open the Network page: the node and the full earnings card")
        .accessibilityElement(children: .combine)
    }

    /// "+12 today · 24 rewards · last one 3m ago" — what the number is made of.
    private var facts: String {
        var parts: [String] = []
        if summary.todayWei != "0" { parts.append(String(localized: "+\(EarningsText.aeth(summary.todayWei)) today")) }
        parts.append(String(localized: "\(summary.count) rewards"))
        if let at = summary.lastRewardAt { parts.append(String(localized: "last one \(EarningsText.ago(at, now: Date()))")) }
        return parts.joined(separator: " · ")
    }
}

/// "● This Mac is verifying blocks  ›"
struct NodeStatusLine: View {
    let work: NodeWork
    let action: () -> Void
    /// Proving is off: say where testnet rewards go, and offer to prove (nil: hidden).
    var onProve: (() -> Void)?

    var body: some View {
        if let onProve, work.phase == .verifying {
            proveOffer(onProve)
        } else {
            line
        }
    }

    /// Testnet rewards go only to the Mac that proves a block first; verifying alone earns nothing.
    private func proveOffer(_ prove: @escaping () -> Void) -> some View {
        HStack(spacing: 12) {
            Circle().fill(Color.aether).frame(width: 8, height: 8)
            Text("This Mac verifies blocks. Rewards go to Macs that prove them.")
                .font(.aeBody).foregroundStyle(DesignTokens.Palette.textMuted.color)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 4)
            Button(action: prove) { Label("Prove blocks", systemImage: "bolt.fill") }
                .buttonStyle(EastSeaPrimaryButtonStyle())
                .help("Uses the GPU and power while on, at your cost. The first valid proof of a block gets a test \(Brand.networkCoinTicker) reward in this wallet.")
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .background(DesignTokens.Palette.surface.color, in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
    }

    private var line: some View {
        Button(action: action) {
            HStack(spacing: 8) {
                Circle().fill(work.isLive ? Color.aether : Color.warn).frame(width: 8, height: 8)
                Text(text).lineLimit(2)
                Spacer(minLength: 4)
                Image(systemName: "chevron.right").foregroundStyle(DesignTokens.Palette.textSubtle.color)
            }
            .font(.aeBody)
            .foregroundStyle(DesignTokens.Palette.textMuted.color)
            .padding(.horizontal, 16).padding(.vertical, 12)
            .background(DesignTokens.Palette.surface.color, in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Open the node on this Mac")
    }

    private var text: String {
        switch work.phase {
        case .verifying: String(localized: "This Mac is verifying blocks")
        case .proving: String(localized: "This Mac is proving blocks · no reward yet")
        case .starting: work.height > 0 ? String(localized: "Node catching up · block #\(String(work.height))") : String(localized: "Node starting…")
        case .paused(let why): why
        }
    }
}

/// Under the node switch in the sidebar.
struct EarningsSidebarBadge: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var earnings: Earnings

    var body: some View {
        if node.enabled {
            EarningsBadge(summary: earnings.summary, work: earnings.work(node, canProve: true))
        }
    }
}

/// One line in the menu-bar panel: "+1.5 test DBLN today".
struct EarningsMenuLine: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var earnings: Earnings

    var body: some View {
        let s = earnings.summary
        if node.enabled, node.prove || s.count > 0 {
            HStack(spacing: 6) {
                Image(systemName: "sparkles")
                Text("+\(EarningsText.aeth(s.todayWei)) \(EarningsText.unit) today").fontWeight(.heavy)
                Spacer(minLength: 4)
                Text("\(EarningsText.aeth(s.totalWei)) total").foregroundStyle(DesignTokens.Palette.textMuted.color)
            }
            .font(.aeBody)
            .foregroundStyle(DesignTokens.Palette.accent.color)
            .contentTransition(.numericText())
        }
    }
}
#endif

// MARK: - DEBUG preview harness

#if DEBUG
/// Sample cards with a reward landing every few seconds (`-earningsPreview` on iOS debug builds).
struct EarningsPreviewHarness: View {
    @State private var summary = EarningsPreviewHarness.sample(count: 24)
    @State private var celebration: RewardCelebration?
    @State private var height: UInt64 = 184_207

    var body: some View {
        ScrollView {
            VStack(spacing: 22) {
                switch UserDefaults.standard.string(forKey: "earningsPreview") {
                case "working":
                    EarningsHero(summary: .empty, work: verifying, onProve: {})
                case "proving":
                    EarningsHero(summary: .empty, work: NodeWork(phase: .proving, height: height, proofs: 3))
                default:
                    EmptyView()
                }
                EarningsHero(summary: summary, work: proving, celebration: celebration)
                EarningsBadge(summary: summary, work: proving)
                EarningsHero(summary: .empty, work: verifying, onProve: {})
                EarningsBadge(summary: .empty, work: verifying)
                EarningsHero(summary: .empty, work: NodeWork(phase: .proving, height: height, proofs: 3))
            }
            .padding(16)
        }
        .background(.background)
        .task { await loop() }
    }

    private var proving: NodeWork {
        NodeWork(phase: .proving, height: height, blocksVerified: 1_284, runningSince: Date().addingTimeInterval(-11_520), proofs: 57)
    }

    private var verifying: NodeWork {
        NodeWork(phase: .verifying, height: height, blocksVerified: 1_284 + height - 184_207,
                 runningSince: Date().addingTimeInterval(-11_520), streakHours: 5)
    }

    /// A block every second; a reward every few seconds.
    private func loop() async {
        var n = 24
        while !Task.isCancelled {
            for _ in 0..<4 {
                try? await Task.sleep(for: .seconds(1))
                height += 1
            }
            n += 1
            summary = Self.sample(count: n)
            celebration = RewardCelebration(id: (celebration?.id ?? 0) + 1, amountWei: "500000000000000000")
        }
    }

    static func sampleEntries(count: Int) -> [RewardEntry] {
        let now = Date()
        return (0..<count).map { i in
            let time = now.addingTimeInterval(Double(i - count + 1) * 600)
            return RewardEntry(proven: UInt64(184_000 + i), amountWei: "500000000000000000", height: UInt64(184_001 + i),
                               time: time, timestampMs: UInt64(time.timeIntervalSince1970 * 1000))
        }
    }

    static func sample(count: Int) -> EarningsSummary {
        EarningsSummary.aggregate(sampleEntries(count: count), now: Date())
    }
}

#Preview("Earnings") { EarningsPreviewHarness() }
#endif
