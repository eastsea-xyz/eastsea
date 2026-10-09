import Foundation

/// Layer 1 of the health signal (docs/design/32-health-signal.md §4.2): the
/// app already knows its own node's state — `aether_status`,
/// `aether_proverStatus`, the wallet's connection, the watchdog's and the
/// update tracker's decisions. This turns that state into one sentence a
/// person can act on, and makes sure nobody misses it: one alert per
/// incident, one "resolved" when it ends, nothing sent anywhere.
///
/// Pure logic like `NodeWatchdog` and `UpdateTracker` — `HealthMonitor` feeds
/// it an `Observation` every few seconds and carries out what it returns —
/// so every table row is a plain unit test (Tests/health-check).
///
/// Rules:
/// - an issue is *raised* once its condition has held for its hold time (the
///   stuck connection waits three minutes, an unanswering node 30 s, the
///   rest are immediate) and *resolved* once the condition has been gone for
///   `resolveAfter` — a flapping condition is one incident, not ten;
/// - a raised issue is never raised again until it resolved (no repeated
///   notifications; design §7.4 alert fatigue);
/// - every duration is a `MonotonicInstant` (red team #6): a manual clock
///   change or an NTP step can neither fabricate an incident nor hide one;
///   a sleep or a wake (`invalidate`) and a gap between observations longer
///   than `maxGap` (the app was suspended — nothing was observed meanwhile)
///   restart every hold that was still counting, rather than counting the
///   unobserved time as "held".
struct HealthCheck {
    /// The table rows of design §4.2. Declaration order is priority: the
    /// banner shows the first active one.
    enum Issue: String, CaseIterable, Equatable {
        /// L8: the chain runs a protocol this app's node cannot.
        case upgradeRequired
        /// L7: three deaths in ten minutes stopped the restarts.
        case crashLoop
        /// L10: the switch is on and no node runs, for a reason worth telling
        /// (`NodeStopReason.isIncident`) that L4/L7/L8 do not already cover —
        /// the founder's 0.7.0 report ("왜 멈췄는지 왜 설명을 안해줌?").
        case nodeStopped
        /// L3 (paused) and L4: below the disk floor, or a write hit ENOSPC.
        /// The node may still answer RPC — "half dead" — which is exactly
        /// why no healthy badge may show meanwhile (the 2026-10-06 incident).
        case diskPaused
        /// L1: the prover's guest program differs from the validators'.
        case programMismatch
        /// L1: proofs are refused (acceptance below 1/4), or proving earns
        /// nothing for six hours while other proofs are paid.
        case proofsRejected
        /// L1: proving is stuck on the node's own error while the chain
        /// moves on (the stalled sidecar of the 2026-10-06 incident: a
        /// broken pipe, rewards stopped for two hours, only the lag grew).
        case proverStalled
        /// L9: the update did not get healthy and the node went back to the
        /// previous binary.
        case updateRolledBack
        /// L5: the watchdog restarted a node that stopped following.
        case followerStuck
        /// L6: the node process lives but its RPC has not answered for 30 s.
        case nodeUnresponsive
        /// L2: "connecting…" for three minutes.
        case connectionStuck
        /// L3 (warning): within 3 GB of the disk floor.
        case diskAlmostFull

        /// The design table's row, for tests and the diagnostics text.
        var row: String {
            switch self {
            case .programMismatch, .proofsRejected, .proverStalled: return "L1"
            case .connectionStuck: return "L2"
            case .diskAlmostFull: return "L3"
            case .diskPaused: return "L4"
            case .followerStuck: return "L5"
            case .nodeUnresponsive: return "L6"
            case .crashLoop: return "L7"
            case .upgradeRequired: return "L8"
            case .updateRolledBack: return "L9"
            case .nodeStopped: return "L10"
            }
        }

        /// The closed failure kind of design §2.4 this issue counts as (the
        /// diagnostics text lists them); a warning counts as none.
        var failureKind: String? {
            switch self {
            case .programMismatch: return "prover_program_mismatch"
            case .proofsRejected: return "proof_rejected"
            case .proverStalled: return "prover_stalled"
            case .connectionStuck, .nodeUnresponsive: return "rpc_unreachable"
            case .diskAlmostFull: return nil
            case .diskPaused: return "disk_floor_pause"
            case .followerStuck: return "follower_stuck"
            case .crashLoop: return "crash_loop"
            case .upgradeRequired: return "upgrade_required"
            case .updateRolledBack: return "update_unhealthy"
            case .nodeStopped: return "node_stopped"
            }
        }
    }

    /// The one button a banner carries (design §4.2 "버튼·자동 동작").
    enum Action: Equatable {
        case checkForUpdates
        case retryConnection
        case openStorage
        case copyDiagnostics
        /// The stop reason's own button (`NodeStopReason.copy(locale:bundle:).action`).
        case fixNode
    }

    /// What the monitor should do after an observation.
    enum Event: Equatable {
        /// Notify once: the issue began.
        case raised(Issue)
        /// Notify once: the issue ended.
        case resolved(Issue)
        /// L2 at 90 s: look the network up again (fresh discovery, new remote
        /// node list). Once per incident; the [다시 시도] button asks again.
        case rediscover
        /// L6: restart the unanswering node. Once per incident — the
        /// watchdog's own backoff governs everything after it.
        case restartNode
    }

    /// Everything the checks read, as the app already has it. Defaults are a
    /// healthy wallet without a node.
    struct Observation: Equatable {
        // aether_proverStatus (only while this Mac proves)
        var proving = false
        var programMismatch = false
        var proofsFailing = false
        /// The node paused proving for the program mismatch (`paused == "program"`).
        var proverPausedForProgram = false
        /// `last_reward` as the node reports it: only its changes matter.
        var lastReward: String?
        /// `aether_proverStatus.error` is set: the node itself says proving
        /// fails (the 2026-10-06 incident sat on "sidecar: Broken pipe").
        var proverError = false
        /// Blocks the chain is ahead of the last proof made here (`lag`).
        var proverLag: UInt64 = 0
        /// Layer 0 (§4.1): other provers' proofs are being paid. Nil when the
        /// app cannot tell; the six-hour cross-check then stays off.
        var networkProofsRewarded: Bool?
        /// A newer approved release is known (the L1 sentence says "update").
        var updateAvailable = false
        // connection
        /// The wallet has a chain status from the network (`WalletModel.status`).
        var walletConnected = true
        /// `NWPathMonitor`: false only when the Mac has no internet at all.
        var internetReachable: Bool?
        // node
        var nodeRunning = false
        /// The node's RPC answered the latest poll.
        var nodeResponsive = true
        /// The node's RPC has answered at least once since it started: a node
        /// still starting up is its own state, not an unanswering one.
        var nodeAnsweredSinceStart = false
        /// `aether_status.resources`: within 3 GB of the floor / below it.
        var diskAlmostFull = false
        var diskPaused = false
        /// This Mac is in the voting set (L4's extra sentence).
        var voting = false
        /// The watchdog stopped restarting, and why (L7; disk → L4,
        /// upgrade → L8).
        var stopped: NodeWatchdog.Failure?
        /// The node needs a newer binary (exit 3/5, or a scheduled protocol
        /// above its own).
        var upgradeRequired = false
        /// The node runs the previous binary after an update failed its
        /// health check, or the update tracker recorded the health failure.
        var rolledBack = false
        /// Why the node is not running (nil while it runs): the one reason
        /// the sidebar, the Node page and the menu show too.
        var nodeStop: NodeStopReason?
    }

    /// A banner: the issue, its sentence and its one button.
    struct Alert: Equatable {
        let issue: Issue
        let sentence: String
        let action: Action?
        /// The condition still holds now; a banner may remain for the quiet
        /// period after recovery, but diagnostics disappear as soon as it clears.
        let diagnosticsVisible: Bool
        /// Includes .fixNode when the stop reason's own action copies diagnostics.
        let primaryActionCopiesDiagnostics: Bool

        var showsSeparateDiagnostics: Bool {
            diagnosticsVisible && !primaryActionCopiesDiagnostics
        }
    }

    /// Hold times (design §4.2).
    static let rediscoverAfter: TimeInterval = 90
    static let connectionAlertAfter: TimeInterval = 180
    static let unresponsiveAfter: TimeInterval = 30
    /// L1's cross-check: proving this long without a new reward while other
    /// proofs are paid (the layer-0 threshold too).
    static let rewardSilenceAfter: TimeInterval = 6 * 3600
    /// L1: the node's own proving error, with the lag growing, this long is
    /// a stalled prover — long enough that a retrying node recovers first,
    /// short enough that the two-hour silence of the 2026-10-06 incident
    /// cannot repeat unnoticed.
    static let proverStallAfter: TimeInterval = 600
    /// L5: a stall restart this recent keeps the issue up; two within
    /// `stallWindow` is "keeps stalling".
    static let stallQuiet: TimeInterval = 600
    static let stallWindow: TimeInterval = 3600
    /// A condition gone this long ends the incident.
    static let resolveAfter: TimeInterval = 20
    /// Observations further apart than this saw nothing in between: holds
    /// restart instead of counting the gap.
    static let maxGap: TimeInterval = 120
    /// The diagnostics text counts failures over this window.
    static let failureWindow: TimeInterval = 24 * 3600

    static func hold(_ issue: Issue) -> TimeInterval {
        switch issue {
        case .connectionStuck: return connectionAlertAfter
        case .nodeUnresponsive: return unresponsiveAfter
        case .proverStalled: return proverStallAfter
        default: return 0
        }
    }

    /// The issues raised and not yet resolved.
    private(set) var active: Set<Issue> = []
    /// The latest observation (the sentences read it).
    private(set) var last = Observation()
    /// Since when each condition has held without a break.
    private var heldSince: [Issue: MonotonicInstant] = [:]
    /// Since when an active issue's condition has been gone.
    private var clearSince: [Issue: MonotonicInstant] = [:]
    private var lastObserved: MonotonicInstant?
    /// This connection incident already asked for a fresh discovery.
    private var rediscovered = false
    /// The watchdog's stall restarts (L5), newest last.
    private var stallRestarts: [MonotonicInstant] = []
    /// L1 cross-check: since when this Mac has proved without a new reward.
    private var rewardSilentSince: MonotonicInstant?
    private var lastRewardSeen: String?
    /// L1: the lag when the node's proving error began (nil: no error). The
    /// stall is "the error holds while the chain moves on", so a proof that
    /// landed (the lag fell) starts a fresh count.
    private var proverLagWhenErrored: UInt64?
    /// Every raise, for the diagnostics' 24-hour failure counts.
    private var raised: [(kind: String, at: MonotonicInstant)] = []

    // MARK: events

    /// One observation. Returns the notifications to post and the actions to
    /// carry out, in priority order.
    mutating func observe(_ o: Observation, at: MonotonicInstant) -> [Event] {
        if let previous = lastObserved, at.elapsed(since: previous) > Self.maxGap {
            restartHolds()
        }
        lastObserved = at
        last = o
        trackReward(o, at: at)
        trackProver(o)
        stallRestarts.removeAll { at.elapsed(since: $0) > Self.stallWindow }
        raised.removeAll { at.elapsed(since: $0.at) > Self.failureWindow }

        var events: [Event] = []
        for issue in Issue.allCases {
            if holds(issue, o, at: at) {
                clearSince[issue] = nil
                let since = heldSince[issue] ?? at
                heldSince[issue] = since
                if !active.contains(issue), at.elapsed(since: since) >= Self.hold(issue) {
                    active.insert(issue)
                    // The disk warning grows into the pause: the same
                    // incident, escalated — the warning leaves quietly
                    // instead of announcing a "resolved" that is not one.
                    if issue == .diskPaused { active.remove(.diskAlmostFull) }
                    if let kind = issue.failureKind { raised.append((kind, at)) }
                    events.append(.raised(issue))
                    if issue == .nodeUnresponsive { events.append(.restartNode) }
                }
            } else {
                heldSince[issue] = nil
                if active.contains(issue) {
                    let gone = clearSince[issue] ?? at
                    clearSince[issue] = gone
                    if at.elapsed(since: gone) >= Self.resolveAfter {
                        active.remove(issue)
                        clearSince[issue] = nil
                        if issue == .diskPaused, holds(.diskAlmostFull, o, at: at) {
                            // Back above the floor but still short of space:
                            // the incident steps down to the warning, quietly.
                            active.insert(.diskAlmostFull)
                            heldSince[.diskAlmostFull] = at
                        } else {
                            events.append(.resolved(issue))
                        }
                    }
                }
            }
        }
        // L2 at 90 s: a fresh discovery, once per incident — never while the
        // Mac has no internet at all (nothing to discover; it reconnects by
        // itself), and the incident ends only when the wallet connects.
        if o.walletConnected {
            rediscovered = false
        } else if !rediscovered, o.internetReachable != false,
                  let since = heldSince[.connectionStuck], at.elapsed(since: since) >= Self.rediscoverAfter {
            rediscovered = true
            events.append(.rediscover)
        }
        return events
    }

    /// The watchdog restarted a node that stopped following (L5).
    mutating func stallRestarted(at: MonotonicInstant) {
        stallRestarts.append(at)
    }

    /// The person pressed [다시 시도] (L2): look again now.
    mutating func retry() -> [Event] {
        rediscovered = true
        return [.rediscover]
    }

    /// A sleep or a wake (red team #9): every hold still counting is stale.
    /// Raised issues stay raised — their resolution needs a fresh observation.
    mutating func invalidate() {
        restartHolds()
        lastObserved = nil
    }

    private mutating func restartHolds() {
        heldSince = heldSince.filter { active.contains($0.key) }
        clearSince = [:]
        rediscovered = false
        rewardSilentSince = nil
    }

    /// Proving without any new reward: the silence clock runs only while this
    /// Mac proves, and any change of `last_reward` restarts it.
    private mutating func trackReward(_ o: Observation, at: MonotonicInstant) {
        guard o.proving else {
            rewardSilentSince = nil
            lastRewardSeen = o.lastReward
            return
        }
        if o.lastReward != lastRewardSeen || rewardSilentSince == nil {
            lastRewardSeen = o.lastReward
            rewardSilentSince = at
        }
    }

    /// The node's own proving error: remembered with the lag it began at, so
    /// the stall is "the error holds while the chain moves on" — an error
    /// that clears, or a lag that falls (a proof landed), starts over.
    private mutating func trackProver(_ o: Observation) {
        if o.proving, o.proverError {
            if proverLagWhenErrored == nil || o.proverLag < proverLagWhenErrored! {
                proverLagWhenErrored = o.proverLag
            }
        } else {
            proverLagWhenErrored = nil
        }
    }

    // MARK: conditions

    private func holds(_ issue: Issue, _ o: Observation, at: MonotonicInstant) -> Bool {
        switch issue {
        case .upgradeRequired:
            return o.upgradeRequired || o.stopped == .upgradeNeeded
        case .crashLoop:
            guard let stopped = o.stopped else { return false }
            return stopped != .diskFull && stopped != .upgradeNeeded
        case .diskPaused:
            return Self.halfDead(o)
        case .programMismatch:
            // A mismatch, or a pause because the node cannot confirm the
            // validators' program at all (0.7.0 on validators without
            // aether_proverProgram): either way proving rests.
            return o.proving && (o.programMismatch || o.proverPausedForProgram)
        case .proofsRejected:
            guard o.proving, !o.programMismatch else { return false }
            if o.proofsFailing { return true }
            // The cross-check: hours of proving, no reward, while the chain
            // pays other provers — rejected without the node knowing why.
            guard o.networkProofsRewarded == true, let since = rewardSilentSince else { return false }
            return at.elapsed(since: since) >= Self.rewardSilenceAfter
        case .proverStalled:
            // The node says proving fails, and the chain keeps moving ahead
            // of it — the stall of the 2026-10-06 incident, where the lag
            // was the only thing growing. An error without a growing lag is
            // not a stall (the chain may simply be quiet, or a retry is
            // under way and about to clear it).
            guard o.proving, o.proverError, let since = proverLagWhenErrored else { return false }
            return o.proverLag > since
        case .updateRolledBack:
            return o.rolledBack
        case .nodeStopped:
            guard let r = o.nodeStop, r.isIncident, o.stopped == nil, !o.upgradeRequired else { return false }
            if case .diskFull = r { return false }   // L4 says it, with the same numbers
            if case .upgradeNeeded = r { return false }   // L8
            return true
        case .followerStuck:
            return stallRestarts.last.map { at.elapsed(since: $0) < Self.stallQuiet } ?? false
        case .nodeUnresponsive:
            return o.nodeRunning && o.nodeAnsweredSinceStart && !o.nodeResponsive
        case .connectionStuck:
            return !o.walletConnected
        case .diskAlmostFull:
            // While the pause is up (or ending), the warning is part of it.
            return o.diskAlmostFull && !Self.halfDead(o) && !active.contains(.diskPaused)
        }
    }

    /// L4: the disk stopped the node's part in the network, whether or not
    /// its RPC still answers.
    static func halfDead(_ o: Observation) -> Bool {
        o.diskPaused || o.stopped == .diskFull
    }

    /// Whether any "Verified"/"정상" badge may show. Never while L4 holds —
    /// read straight from the observation, with no hold: a half-dead node
    /// looking healthy is the incident of 2026-10-06 itself.
    static func healthyBadgeAllowed(_ o: Observation) -> Bool {
        !halfDead(o)
    }

    var healthyBadgeAllowed: Bool { Self.healthyBadgeAllowed(last) }

    // MARK: words

    /// The banner for the most urgent active issue, nil when all is well.
    func alert(locale: Locale = .current, bundle: Bundle = .main) -> Alert? {
        guard let top = Issue.allCases.first(where: { active.contains($0) }) else { return nil }
        let diagnosticsVisible = lastObserved.map { holds(top, last, at: $0) } ?? false
        let primary = action(top)
        let copiesDiagnostics = primary == .copyDiagnostics
            || (primary == .fixNode && last.nodeStop?.copy().action == .copyDiagnostics)
        return Alert(issue: top, sentence: sentence(top, locale: locale, bundle: bundle),
                     action: !diagnosticsVisible && (copiesDiagnostics || primary == .fixNode) ? nil : primary,
                     diagnosticsVisible: diagnosticsVisible, primaryActionCopiesDiagnostics: copiesDiagnostics)
    }

    /// The failure kinds raised over the last 24 hours, with their counts
    /// (the diagnostics text buckets them).
    func failureCounts() -> [String: Int] {
        raised.reduce(into: [:]) { $0[$1.kind, default: 0] += 1 }
    }

    func action(_ issue: Issue) -> Action? {
        switch issue {
        case .programMismatch, .proofsRejected, .upgradeRequired: return .checkForUpdates
        case .connectionStuck: return last.internetReachable == false ? nil : .retryConnection
        case .diskAlmostFull, .diskPaused: return .openStorage
        case .crashLoop: return last.nodeStop?.copy().action == nil ? .copyDiagnostics : .fixNode
        case .nodeStopped: return last.nodeStop?.copy().action == nil ? nil : .fixNode
        case .proverStalled: return .copyDiagnostics
        case .followerStuck, .nodeUnresponsive, .updateRolledBack: return nil
        }
    }

    /// What happened + what to do + whether the money is safe (design §4.2),
    /// with no jargon: guest, ENOSPC and RPC stay in developer mode.
    func sentence(_ issue: Issue, locale: Locale = .current, bundle: Bundle = .main) -> String {
        let o = last
        switch issue {
        case .programMismatch, .proofsRejected:
            if o.updateAvailable {
                return String(localized: "Your reward proofs are being rejected. Updating the app fixes it.", bundle: bundle, locale: locale)
            }
            if o.proverPausedForProgram {
                return String(localized: "This Mac is resting from proving blocks for now: the network cannot check this version's proofs yet. Nothing is lost.", bundle: bundle, locale: locale)
            }
            return String(localized: "Your reward proofs are being rejected. You will be told as soon as a fixed version is out.", bundle: bundle, locale: locale)
        case .proverStalled:
            return String(localized: "Reward proofs have stopped. This Mac is retrying by itself; if it keeps up, please reopen the app.", bundle: bundle, locale: locale)
        case .connectionStuck:
            if o.internetReachable == false {
                return String(localized: "This Mac is not connected to the internet. It reconnects by itself once it is.", bundle: bundle, locale: locale)
            }
            return String(localized: "The network cannot be found right now. Trying again. Your balance is safe.", bundle: bundle, locale: locale)
        case .diskAlmostFull:
            return String(localized: "This Mac is almost out of storage. Please free about 3 GB.", bundle: bundle, locale: locale)
        case .diskPaused:
            var base = String(localized: "Storage is low, so this Mac paused its part in the network. It restarts by itself once 7 GB is free.", bundle: bundle, locale: locale)
            if let r = o.nodeStop, case .diskFull = r { base = r.copy(locale: locale, bundle: bundle).paragraph }
            guard o.voting else { return base }
            return base + String(localized: " This Mac is a voting node, so the other validators are waiting for it.", bundle: bundle, locale: locale)
        case .followerStuck:
            if stallRestarts.count >= 2 {
                return String(localized: "The node keeps stalling. Update the app or restart the Mac. Meanwhile your balance is checked through other nodes.", bundle: bundle, locale: locale)
            }
            return String(localized: "This Mac fell behind the network, so its node was restarted.", bundle: bundle, locale: locale)
        case .nodeUnresponsive:
            return String(localized: "This Mac's node stopped answering, so it is being restarted.", bundle: bundle, locale: locale)
        case .crashLoop:
            // The stop reason says when it retries; else layer 4's sentence.
            if let r = o.nodeStop, r.isIncident { return r.copy(locale: locale, bundle: bundle).paragraph }
            return (o.stopped ?? .other).sentence(locale: locale, bundle: bundle)
        case .nodeStopped:
            return o.nodeStop?.copy(locale: locale, bundle: bundle).paragraph
                ?? String(localized: "This Mac's node is not running. Your balance is still checked through other nodes.", bundle: bundle, locale: locale)
        case .upgradeRequired:
            return String(localized: "The network's rules changed, so this app needs an update. Until then your balance is checked through other nodes.", bundle: bundle, locale: locale)
        case .updateRolledBack:
            return String(localized: "The node did not run well on the new version, so it went back to the previous one. The fixed version installs by itself when it arrives.", bundle: bundle, locale: locale)
        }
    }

    /// The one "해결됐어요" each incident earns when it ends.
    static func resolvedSentence(_ issue: Issue, locale: Locale = .current, bundle: Bundle = .main) -> String {
        switch issue {
        case .programMismatch, .proofsRejected:
            return String(localized: "Resolved: reward proofs are being accepted again.", bundle: bundle, locale: locale)
        case .proverStalled:
            return String(localized: "Resolved: reward proofs are being made again.", bundle: bundle, locale: locale)
        case .connectionStuck:
            return String(localized: "Resolved: connected to the network again.", bundle: bundle, locale: locale)
        case .diskAlmostFull, .diskPaused:
            return String(localized: "Resolved: there is enough storage again.", bundle: bundle, locale: locale)
        case .followerStuck:
            return String(localized: "Resolved: this Mac is keeping up with the network again.", bundle: bundle, locale: locale)
        case .nodeUnresponsive:
            return String(localized: "Resolved: this Mac's node is answering again.", bundle: bundle, locale: locale)
        case .crashLoop, .nodeStopped:
            return String(localized: "Resolved: the node is running again.", bundle: bundle, locale: locale)
        case .upgradeRequired:
            return String(localized: "Resolved: the app is updated to the network's rules.", bundle: bundle, locale: locale)
        case .updateRolledBack:
            return String(localized: "Resolved: the fixed version is installed.", bundle: bundle, locale: locale)
        }
    }
}
