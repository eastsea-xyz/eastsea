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
        return o
    }

    private func handle(_ events: [HealthCheck.Event]) {
        let ko = HealthCheck.korean
        let title = ko ? Brand.projectKo : Brand.project
        for event in events {
            switch event {
            case .raised(let issue):
                LocalNotice.post(title: title, body: check.sentence(issue, ko: ko))
            case .resolved(let issue):
                LocalNotice.post(title: title, body: HealthCheck.resolvedSentence(issue, ko: ko))
            case .rediscover:
                model?.rediscover()
                node?.refreshWalletRoute()
            case .restartNode:
                node?.restartUnresponsive()
            }
        }
        let banner = check.alert(ko: ko)
        if alert != banner { alert = banner }
        if healthyBadgeAllowed != check.healthyBadgeAllowed { healthyBadgeAllowed = check.healthyBadgeAllowed }
    }

    // MARK: buttons

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
        // The text drops these (Tests/diagnostic-report proves it).
        s.address = model.address
        s.balanceWei = model.account?.balanceWei ?? ""
        s.nodeId = node.candidate?.nodeId ?? ""
        Clipboard.copy(DiagnosticReport.text(s))
    }
}

extension HealthCheck.Action {
    /// The banner button's words, in the app's language.
    func label(ko: Bool = HealthCheck.korean) -> String {
        switch self {
        case .checkForUpdates: return ko ? "업데이트 확인" : "Check for Updates"
        case .retryConnection: return ko ? "다시 시도" : "Try Again"
        case .openStorage: return ko ? "저장 공간 관리 열기" : "Open Storage Settings"
        case .copyDiagnostics: return ko ? "진단 정보 복사" : "Copy Diagnostics"
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
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "exclamationmark.triangle.fill").font(.title2).foregroundStyle(Color.warn)
                VStack(alignment: .leading, spacing: 8) {
                    Text(alert.sentence).font(.aeBody)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 12) {
                        if let action = alert.action {
                            Button(action.label()) { health.perform(action) }
                                .buttonStyle(.borderedProminent).tint(Color.warn)
                        }
                        if alert.action != .copyDiagnostics {
                            Button(copied ? (HealthCheck.korean ? "복사했어요" : "Copied")
                                          : HealthCheck.Action.copyDiagnostics.label()) {
                                health.copyDiagnostics()
                                copied = true
                            }
                            .buttonStyle(.borderless).font(.aeFootnote)
                            .help(HealthCheck.korean ? "주소·잔액·노드 ID 없이, 이 Mac 밖으로 보내지 않고 클립보드에만 복사해요."
                                  : "Copies to the clipboard only — no address, balance or node ID, nothing sent from this Mac.")
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(16)
            .background(Color.warn.opacity(0.12), in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
            .onChange(of: alert.issue) { _, _ in copied = false }
        }
    }
}
#endif
