#if os(macOS)
import AppKit
import Network
import SwiftUI

/// Carries out `HealthCheck` (docs/design/32-health-signal.md §4.2): every
/// few seconds it reads what the app already knows into an observation, posts
/// the macOS notification for each new incident and each resolution (once),
/// and does what the check asks — a fresh network lookup at 90 s of
/// "connecting", a restart of a node whose RPC went silent. The banner, the
/// menu-bar line and the healthy badge read its published state. Nothing
/// here sends anything off this Mac.
@MainActor
final class HealthMonitor: ObservableObject {
    /// The most urgent incident's banner, nil while all is well.
    @Published private(set) var alert: HealthCheck.Alert?
    /// False while L4 holds: no "Verified"/"정상" badge may show.
    @Published private(set) var healthyBadgeAllowed = true

    /// Every check runs on this never-jumping clock (red team #6).
    private let clock: Clock
    private var check = HealthCheck()
    private weak var node: NodeController?
    private weak var model: WalletModel?
    /// An approved release is on its way (the L1 sentence says "update").
    private var updateComing: @MainActor () -> Bool = { false }
    /// The update tracker recorded that an update did not get healthy (L9).
    private var updateUnhealthy: @MainActor () -> Bool = { false }
    private var openUpdates: @MainActor () -> Void = {}
    private var timer: Timer?
    private var started = false
    /// Whether this Mac has internet at all (`NWPathMonitor`), nil until known.
    private var internet: Bool?
    private let path = NWPathMonitor()
    private var stallRestartsSeen = 0
    private var wakeObservers: [NSObjectProtocol] = []

    /// The check runs every 5 s: L6's 30 s and the 20 s resolution need no
    /// finer grain, and the node's own poll stays the 2 s one.
    static let interval: TimeInterval = 5

    init(clock: Clock = UptimeClock()) {
        self.clock = clock
    }

    #if DEBUG
    /// Design preview: the banner for `issue`, as the check would raise it.
    func loadPreview(issue: HealthCheck.Issue) {
        let c = HealthCheck()
        alert = HealthCheck.Alert(issue: issue, sentence: c.sentence(issue), action: c.action(issue))
        healthyBadgeAllowed = issue != .diskPaused
    }
    #endif

    /// Once per launch, beside the node and the update tracker.
    func start(node: NodeController, model: WalletModel, updateComing: @escaping @MainActor () -> Bool,
               updateUnhealthy: @escaping @MainActor () -> Bool, openUpdates: @escaping @MainActor () -> Void) {
        guard !started else { return }
        started = true
        self.node = node
        self.model = model
        self.updateComing = updateComing
        self.updateUnhealthy = updateUnhealthy
        self.openUpdates = openUpdates
        stallRestartsSeen = node.stallRestarts
        path.pathUpdateHandler = { [weak self] p in
            let online = p.status == .satisfied
            Task { @MainActor in self?.internet = online }
        }
        path.start(queue: DispatchQueue(label: "\(Brand.project).health.path"))
        // Sleep/wake (red team #9): every hold that was counting is stale.
        let center = NSWorkspace.shared.notificationCenter
        wakeObservers = [NSWorkspace.willSleepNotification, NSWorkspace.didWakeNotification].map { name in
            center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.check.invalidate() }
            }
        }
        timer = Timer.scheduledTimer(withTimeInterval: Self.interval, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
    }

    private func tick() {
        guard let node, let model else { return }
        let now = clock.now
        if node.stallRestarts > stallRestartsSeen {
            stallRestartsSeen = node.stallRestarts
            check.stallRestarted(at: now)
        }
        handle(check.observe(observation(node: node, model: model), at: now))
    }

    /// What the app knows right now, in the check's terms.
    private func observation(node: NodeController, model: WalletModel) -> HealthCheck.Observation {
        var o = HealthCheck.Observation()
        let prover = node.prove ? node.prover : nil
        o.proving = prover?.running == true
        o.programMismatch = prover?.program_mismatch == true
        o.proofsFailing = prover?.proofs_failing == true
        o.proverPausedForProgram = prover?.paused == "program"
        o.lastReward = prover?.last_reward
        // The stalled prover (L1): the node's own error, with the lag the
        // chain is ahead of it.
        o.proverError = prover?.error != nil
        o.proverLag = prover?.lag ?? 0
        // The app has no layer-0 view of other provers' rewards yet: the
        // six-hour cross-check stays off until it does (health-watch.py
        // watches that from outside meanwhile).
        o.networkProofsRewarded = nil
        o.updateAvailable = updateComing()
        o.walletConnected = model.status != nil
        o.internetReachable = internet
        o.nodeRunning = node.process != nil || node.attached
        o.nodeResponsive = node.rpcAnswering
        o.nodeAnsweredSinceStart = node.answeredSinceStart
        o.diskAlmostFull = node.diskAlmostFull
        o.diskPaused = node.diskPaused
        o.voting = node.isValidator
        o.stopped = node.stoppedFailure
        o.upgradeRequired = node.upgradeRequired || model.networkOutdated
        o.rolledBack = node.usePreviousBinary || updateUnhealthy()
        o.nodeStop = node.stopReason
        return o
    }

    private func handle(_ events: [HealthCheck.Event]) {
        let title = Brand.name
        for event in events {
            switch event {
            case .raised(let issue):
                LocalNotice.post(title: title, body: check.sentence(issue))
            case .resolved(let issue):
                LocalNotice.post(title: title, body: HealthCheck.resolvedSentence(issue))
            case .rediscover:
                model?.rediscover()
                node?.refreshWalletRoute()
            case .restartNode:
                node?.restartUnresponsive()
            }
        }
        let banner = check.alert()
        if alert != banner { alert = banner }
        if healthyBadgeAllowed != check.healthyBadgeAllowed { healthyBadgeAllowed = check.healthyBadgeAllowed }
    }

    // MARK: buttons

    /// The "not healthy" badge's words (dashboard, menu): the node's own
    /// stop reason, else the disk pause.
    var pausedBadgeTitle: String {
        node?.stopReason.map { $0.copy().title }
            ?? (String(localized: "Storage low · node resting"))
    }

    /// The stop reason's own button label (L10's banner shows it).
    var nodeActionLabel: String? { node?.stopReason?.copy().actionLabel }

    func perform(_ action: HealthCheck.Action) {
        switch action {
        case .checkForUpdates:
            openUpdates()
        case .retryConnection:
            handle(check.retry())
        case .openStorage:
            if let url = URL(string: "x-apple.systempreferences:com.apple.settings.Storage") {
                NSWorkspace.shared.open(url)
            }
        case .copyDiagnostics:
            copyDiagnostics()
        case .fixNode:
            if let action = node?.stopReason?.copy().action { node?.perform(action) }
        }
    }

    /// "진단 정보 복사": local only — onto the clipboard, nowhere else.
    func copyDiagnostics() {
        guard let node, let model else { return }
        var s = DiagnosticReport.Snapshot()
        let info = Bundle.main.infoDictionary
        s.appVersion = info?["CFBundleShortVersionString"] as? String ?? ""
        #if DEBUG
        s.publicBuild = false
        #endif
        s.nodeProtocol = node.nodeProtocol
        s.newestScheduled = node.newestScheduledProtocol
        s.networkProgram = node.prover?.network_program
        s.programMatches = node.prover.map { $0.program_mismatch != true }
        s.nodeRunning = node.process != nil || node.attached
        s.voting = node.isValidator
        s.proving = node.prove && node.prover?.running == true
        s.localHeight = node.height > 0 ? node.height : nil
        s.networkHeight = model.status?.height
        s.finalizedAge = model.blocks.map(\.timestampMs).max().map { Date().timeIntervalSince1970 - TimeInterval($0) / 1000 }
        s.osMajor = ProcessInfo.processInfo.operatingSystemVersion.majorVersion
        s.failures = check.failureCounts()
        s.nodeStop = node.stopReason?.code
        // "<time> stopped <code> …" → the code only (no exact time, no paths).
        s.lastStop = node.lastStopLine.flatMap { $0.split(separator: " ").dropFirst(2).first.map(String.init) }
        // The text drops these (Tests/diagnostic-report proves it).
        s.address = model.address
        s.balanceWei = model.account?.balanceWei ?? ""
        s.nodeId = node.candidate?.nodeId ?? ""
        Clipboard.copy(DiagnosticReport.text(s))
    }
}

extension HealthCheck.Action {
    /// The banner button's words, in the app's language.
    func label() -> String {
        switch self {
        case .checkForUpdates: return String(localized: "Check for Updates")
        case .retryConnection: return String(localized: "Try Again")
        case .openStorage: return String(localized: "Open Storage Settings")
        case .copyDiagnostics: return String(localized: "Copy Diagnostics")
        case .fixNode: return String(localized: "Fix")
        }
    }
}

/// Simple mode's health banner (design §4.2): one sentence and one button,
/// above the balance, plus the local diagnostics copy.
struct HealthBanner: View {
    @EnvironmentObject var health: HealthMonitor
    @State private var copied = false

    var body: some View {
        if let alert = health.alert {
            HStack(alignment: .top, spacing: DesignTokens.Space.s3) {
                Image(systemName: "exclamationmark.triangle.fill").font(.aeHeadline).foregroundStyle(Color.warn)
                    .frame(width: 24, height: 24)
                VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
                    Text(alert.sentence).font(.aeBody)
                        .foregroundStyle(DesignTokens.Palette.text.color)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: DesignTokens.Space.s3) {
                        if let action = alert.action {
                            Button(action == .fixNode ? (health.nodeActionLabel ?? action.label()) : action.label()) { health.perform(action) }
                                .buttonStyle(EastSeaPrimaryButtonStyle())
                        }
                        if alert.action != .copyDiagnostics {
                            Button(copied ? (String(localized: "Copied"))
                                          : HealthCheck.Action.copyDiagnostics.label()) {
                                health.copyDiagnostics()
                                copied = true
                            }
                            .buttonStyle(.borderless).font(.aeFootnote).foregroundStyle(DesignTokens.Palette.accent.color)
                            .help("Copies to the clipboard only — no address, balance or node ID, nothing sent from this Mac.")
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(DesignTokens.Space.s4)
            .background(DesignTokens.Palette.surface.color, in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: Radius.card, style: .continuous)
                    .stroke(DesignTokens.Palette.line.color, lineWidth: 1)
            }
            .onChange(of: alert.issue) { _, _ in copied = false }
        }
    }
}
#endif
