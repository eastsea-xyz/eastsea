import Foundation

/// The node watchdog (docs/design/24-self-healing.md layer 2): what the app
/// does when its node stops, stalls, or dies over and over. Pure decisions —
/// `NodeController` feeds it events and carries them out — so every rule is a
/// plain unit test (Tests/watchdog).
///
/// Rules:
/// - the node exited on its own → restart it, backing off (1 s doubling to
///   60 s) so a tight crash loop cannot spin the Mac;
/// - three exits within 10 minutes → stop restarting and say one plain
///   sentence about the last failure (layer 4: what to do, no jargon);
/// - the network's finalized height moves but ours has not for 60 s while the
///   node runs → restart it (the incident of 2026-09-29 looked exactly like
///   this, and "끊김" told the user nothing).
struct NodeWatchdog {
    /// What the app should do about its node right now.
    enum Decision: Equatable {
        /// Nothing: keep watching.
        case none
        /// Relaunch the node after `after` seconds (crash backoff, or a
        /// stall): `after` is 0 for a stall, which only fires once per freeze.
        case restart(after: TimeInterval)
        /// Stop restarting and show this failure's sentence (three deaths in
        /// ten minutes).
        case stop(Failure)
        /// The updated binary just died three times within a minute of
        /// starting: run the previous binary instead (layer 2, upgrade
        /// rollback), and say so.
        case rollback
    }

    /// Why the node kept dying, in words a person can act on (layer 4).
    enum Failure: Equatable {
        case diskFull
        case database
        case memory
        case network
        case other

        /// One sentence, in the app's language: what happened and what to do.
        var sentence: String {
            let ko = Locale.preferredLanguages.first?.hasPrefix("ko") ?? false
            switch self {
            case .diskFull:
                return ko ? "디스크 공간이 부족해요. 10GB 비우면 노드가 저절로 다시 시작합니다."
                    : "The disk is full. Free 10 GB and the node restarts by itself."
            case .database:
                return ko ? "노드 데이터가 계속 손상됩니다. 앱을 지웠다가 다시 설치해 주세요."
                    : "The node's data keeps getting damaged. Please delete and reinstall the app."
            case .memory:
                return ko ? "메모리가 부족해요. 다른 앱을 몇 개 닫아 주세요."
                    : "The Mac is low on memory. Close a few other apps."
            case .network:
                return ko ? "네트워크에 연결할 수 없어요. 인터넷 연결을 확인해 주세요."
                    : "No network connection. Please check the internet."
            case .other:
                return ko ? "노드가 계속 종료됩니다. 앱을 다시 실행해 주세요."
                    : "The node keeps stopping. Please restart the app."
            }
        }
    }

    /// Three deaths inside this window stop the restarts (design layer 2).
    static let crashWindow: TimeInterval = 600
    static let maxCrashes = 3
    /// This long frozen at one height while the network moves = a stall.
    static let stallAfter: TimeInterval = 60
    /// A quick death: an update whose binary cannot run dies this fast.
    static let quickExit: TimeInterval = 60
    static let maxQuickExits = 3
    /// Restart backoff bounds.
    static let firstBackoff: TimeInterval = 1
    static let maxBackoff: TimeInterval = 60

    /// The node's recent deaths (sliding `crashWindow`), oldest first.
    private(set) var exits: [Date] = []
    /// When the node last started (quick-exit streaks measure from this).
    private(set) var startedAt: Date?
    /// Deaths in a row that each came within `quickExit` of a start.
    private(set) var quickExits = 0
    /// The last failure the node reported, for the sentence if it keeps dying.
    private(set) var lastFailure: Failure = .other
    private var lastHeight: UInt64?
    private var frozenSince: Date?
    private var stalled = false

    /// The node process is running as of `at`.
    mutating func started(_ at: Date) {
        startedAt = at
        lastHeight = nil
        frozenSince = nil
        stalled = false
    }

    /// The node exited on its own with `code` (a signal-death is reported with
    /// `signaled: true`). `log` is the tail of node.log, when there is one:
    /// the exit code says *that* storage failed, the log tail says which kind.
    mutating func exited(_ at: Date, code: Int32, signaled: Bool = false, log: String = "") -> Decision {
        exits.append(at)
        exits.removeAll { at.timeIntervalSince($0) > Self.crashWindow }
        lastFailure = Self.classify(code: code, signaled: signaled, log: log)
        if let startedAt, at.timeIntervalSince(startedAt) < Self.quickExit {
            quickExits += 1
        } else {
            quickExits = 0
        }
        if quickExits >= Self.maxQuickExits {
            return .rollback
        }
        // Three restarts is the limit: the fourth death in ten minutes stops
        // the loop and tells the user what to do instead.
        guard exits.count <= Self.maxCrashes else {
            return .stop(lastFailure)
        }
        return .restart(after: min(Self.firstBackoff * pow(2, Double(exits.count - 1)), Self.maxBackoff))
    }

    /// One 2-second poll (the same one NodeController already runs): `local`
    /// is this node's height, `network` the highest height any node reports.
    mutating func polled(_ at: Date, local: UInt64?, network: UInt64?) -> Decision {
        guard let local else {
            frozenSince = nil
            return .none
        }
        if local != lastHeight {
            lastHeight = local
            frozenSince = nil
            stalled = false
            return .none
        }
        // Our height is standing still; that is only a stall while the network
        // moves ahead (a paused network is not the node's fault, and it has
        // its own notice).
        guard let network, network > local + 2, startedAt != nil else {
            frozenSince = nil
            return .none
        }
        let since = frozenSince ?? at
        frozenSince = since
        guard at.timeIntervalSince(since) >= Self.stallAfter, !stalled else { return .none }
        stalled = true
        lastFailure = .network
        return .restart(after: 0)
    }

    /// A restart was carried out (the watchdog's counts live on across them:
    /// three restarts in ten minutes is three, however they were decided).
    mutating func restarting() {
        stalled = false
        frozenSince = nil
    }

    /// Which sentence this death earns. The storage exit code (4) is the node
    /// saying "my disk or my database"; the log tail tells them apart, because
    /// one asks the user to free space and the other to reinstall.
    static func classify(code: Int32, signaled: Bool, log: String) -> Failure {
        if signaled || code == 137 { return .memory }
        let tail = log.suffix(8_192).lowercased()
        if tail.contains("enospc") || tail.contains("no space left") {
            return .diskFull
        }
        if code == 4 {
            return tail.contains("does not verify") || tail.contains("corrupt") ? .database : .diskFull
        }
        if tail.contains("cannot allocate memory") || tail.contains("out of memory") {
            return .memory
        }
        if tail.contains("no upstream") || tail.contains("partitioned") || tail.contains("connection refused") {
            return .network
        }
        return .other
    }
}
