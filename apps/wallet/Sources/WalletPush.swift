import CoreFoundation
import Dispatch
import Foundation

/// A local-node hint can schedule a verified read. It never supplies an
/// account proof, transaction receipt, or release approval to the wallet.
struct WalletPushNotice: Equatable {
    let height: UInt64
    let topics: Set<String>
    var balanceChanged: Bool { topics.contains("balance") }
    var transactionsChanged: Bool { topics.contains("tx_status") }
    var releaseChanged: Bool { topics.contains("release") }
}

struct WalletPushFilter: Equatable {
    let address: String?
    let transactions: [String]

    init(address: String, transactions: [String]) {
        let canonical = address.lowercased()
        self.address = Self.hex(canonical, bytes: 20) ? canonical : nil
        var seen = Set<String>()
        self.transactions = Array(transactions.map { $0.lowercased() }.filter {
            Self.hex($0, bytes: 32) && seen.insert($0).inserted
        }.prefix(32))
    }

    private static func hex(_ value: String, bytes: Int) -> Bool {
        value.utf8.count == 2 + bytes * 2 && value.hasPrefix("0x")
            && value.dropFirst(2).utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }
}

enum WalletPushFrame: Equatable {
    case subscribed(String)
    case notice(WalletPushNotice)
    case gap
    case rejected
}

enum WalletPushWire {
    static let maxFrameBytes = 256 * 1024

    static func request(id: Int, filter: WalletPushFilter, after: UInt64?) -> Data {
        var options: [String: Any] = ["transactions": filter.transactions]
        if let address = filter.address { options["address"] = address }
        if let after { options["after"] = after }
        return (try? JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "id": id,
            "method": "aether_subscribe", "params": ["wallet", options]])) ?? Data()
    }

    static func parse(_ data: Data, subscription: String?) -> WalletPushFrame? {
        guard data.count <= maxFrameBytes else { return .rejected }
        guard let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return nil }
        if subscription == nil, integer(object["id"]) == 1 {
            if object["error"] != nil { return .rejected }
            if let id = object["result"] as? String, id.hasPrefix("0x"), id.count <= 66 {
                return .subscribed(id)
            }
            return .rejected
        }
        guard object["method"] as? String == "aether_subscription",
              let params = object["params"] as? [String: Any],
              let subscription, params["subscription"] as? String == subscription,
              let result = params["result"] as? [String: Any] else { return nil }
        if result["kind"] as? String == "gap" { return .gap }
        guard result["kind"] as? String == "wallet", let height = integer(result["height"]),
              let topics = result["topics"] as? [String], topics.count <= 8 else { return nil }
        return .notice(WalletPushNotice(height: height, topics: Set(topics)))
    }

    private static func integer(_ value: Any?) -> UInt64? {
        guard let number = value as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID() else { return nil }
        return UInt64(number.stringValue)
    }
}

/// Monotonic seconds are supplied by the transport; wall-clock changes do
/// not manufacture retries. Healthy sockets have no discovery poll deadline.
struct WalletPushPolicy {
    static let maxBackoff: TimeInterval = 60
    private(set) var isConnected = false
    private(set) var nextRetry: TimeInterval?
    private var failures = 0

    mutating func connecting(at now: TimeInterval) {
        isConnected = false
        nextRetry = nil
    }

    /// True once for the acknowledgement, so reconnect performs one catch-up.
    mutating func connected() -> Bool {
        guard !isConnected else { return false }
        isConnected = true
        nextRetry = nil
        return true
    }

    /// An acknowledgement followed by a close/gap is still a failed stream.
    /// Only a valid wallet delivery demonstrates enough health to reset it.
    mutating func healthy() {
        guard isConnected else { return }
        failures = 0
    }

    mutating func disconnected(at now: TimeInterval, jitter: Double) {
        isConnected = false
        failures = min(failures + 1, 6)
        let base = min(Self.maxBackoff, pow(2, Double(failures)))
        let factor = 0.8 + min(1, max(0, jitter)) * 0.4
        nextRetry = now + min(Self.maxBackoff, base * factor)
    }

    mutating func retryDue(at now: TimeInterval) -> Bool {
        guard !isConnected, let nextRetry, now >= nextRetry else { return false }
        self.nextRetry = nil
        return true
    }

    func shouldReadTransaction(lastRevision: UInt64?, revision: UInt64) -> Bool {
        lastRevision == nil || lastRevision != revision
    }
}

/// Dirty receipt work survives an asynchronous batch and drains after a single
/// hint. Failed reads wait for another delivered head/hint, never a busy loop.
struct WalletPushReconciliation {
    static let maxPending = 500
    static let batchSize = 8
    private var ready: [String] = []
    private var active: [String] = []
    private var redirtied = Set<String>()
    private var deferred = Set<String>()
    private var overflowOffset = 0

    var pendingCount: Int {
        Set(ready).union(active).union(redirtied).union(deferred).count
    }
    var hasReadyWork: Bool { !ready.isEmpty }

    mutating func retain(available: [String]) {
        let allowed = Set(available.prefix(Self.maxPending))
        ready.removeAll { !allowed.contains($0) }
        redirtied.formIntersection(allowed)
        deferred.formIntersection(allowed)
    }

    mutating func invalidate(_ hashes: [String]) {
        for hash in hashes.prefix(Self.maxPending) {
            if active.contains(hash) {
                redirtied.insert(hash)
            } else if !ready.contains(hash) {
                let alreadyRetained = deferred.remove(hash) != nil
                guard alreadyRetained || pendingCount < Self.maxPending else { continue }
                ready.append(hash)
            }
        }
    }

    mutating func nextBatch() -> [String] {
        guard active.isEmpty else { return [] }
        active = Array(ready.prefix(Self.batchSize))
        ready.removeFirst(active.count)
        return active
    }

    mutating func finish(retry: [String]) {
        let completed = active
        active = []
        deferred.formUnion(Set(retry).intersection(completed))
        let repeatReads = completed.filter { redirtied.contains($0) }
        redirtied.removeAll()
        invalidate(repeatReads)
    }

    /// The wire filter has 32 hashes. A finalized head is an opportunity to
    /// reconcile a rotating batch of the remainder, so none wait forever for
    /// a hint about a hash this connection could not subscribe to.
    mutating func headOpportunity(available: [String], subscribed: Set<String>) {
        retain(available: available)
        let bounded = Array(available.prefix(Self.maxPending))
        invalidate(bounded.filter { deferred.contains($0) })
        let overflow = bounded.filter { !subscribed.contains($0) }
        guard !overflow.isEmpty else { overflowOffset = 0; return }
        let start = overflowOffset % overflow.count
        let count = min(Self.batchSize, overflow.count)
        invalidate((0..<count).map { overflow[(start + $0) % overflow.count] })
        overflowOffset = (start + count) % overflow.count
    }
}

enum WalletPushProgress {
    /// A transport pong does not mean the chain advanced. Re-evaluate only
    /// cached dates on the local pulse, with no balance/status discovery RPC.
    static func pauseSince(now: Date, changedAt: Date?, newest: Date?, after: TimeInterval) -> Date? {
        let still = changedAt.map { now.timeIntervalSince($0) } ?? 0
        let oldBlock = newest.map { now.timeIntervalSince($0) > after } ?? false
        guard still > after || (oldBlock && still > 20) else { return nil }
        return min(newest ?? changedAt ?? now, changedAt ?? now)
    }
}

enum WalletPushDelivery {
    case connected
    case notice(WalletPushNotice)
    case fallback
    case gap
}

private final class WalletPushSessionDelegate: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask,
                    willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

/// One bounded WebSocket to this wallet's own loopback node. The timer only
/// checks connection/retry deadlines and sends transport pings; it does not
/// query balance, receipts, logs, or appcasts while the subscription is live.
@MainActor
final class WalletPushClient {
    var onDelivery: ((WalletPushDelivery) -> Void)?
    var onPulse: (() -> Void)?
    private(set) var revision: UInt64 = 0
    private(set) var fallbackReads = 0
    var isConnected: Bool { policy.isConnected }
    private var policy = WalletPushPolicy()
    private var filter = WalletPushFilter(address: "", transactions: [])
    private var port: UInt16 = 18545
    private var cursor: UInt64?
    private var deliveredHeight: UInt64?
    private var subscription: String?
    private var session: URLSession?
    private var socket: URLSessionWebSocketTask?
    private var receiveTask: Task<Void, Never>?
    private var pulse: Timer?
    private var generation: UInt64 = 0
    private var connectDeadline: TimeInterval?
    private var nextPing: TimeInterval = 0
    private var pingDeadline: TimeInterval?
    private var running = false
    private var now: TimeInterval { Double(DispatchTime.now().uptimeNanoseconds) / 1_000_000_000 }

    func configure(port: UInt16, address: String, transactions: [String], resetStream: Bool = false) {
        let next = WalletPushFilter(address: address, transactions: transactions)
        guard resetStream || self.port != port || filter != next else { return }
        if resetStream || self.port != port || filter.address != next.address { resetCursor() }
        self.port = port
        filter = next
        if running { connect() }
    }

    /// A received height alone is never the resume cursor. Only the existing
    /// verifier advances it, and switching account/network discards it.
    func verified(height: UInt64) {
        // The wallet's verifier may already be ahead through remote peers
        // while its own node catches up. Never resume beyond this stream's
        // watermark, which the server correctly diagnoses as a history gap.
        guard let deliveredHeight else { return }
        cursor = max(cursor ?? 0, min(height, deliveredHeight))
    }

    func resetCursor() { cursor = nil; deliveredHeight = nil }

    func shouldReadTransaction(lastRevision: UInt64?) -> Bool {
        policy.shouldReadTransaction(lastRevision: lastRevision, revision: revision)
    }

    func start() {
        guard !running else { return }
        running = true
        pulse = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
        connect()
    }

    func stop() {
        running = false
        pulse?.invalidate()
        pulse = nil
        close()
        policy = WalletPushPolicy()
    }

    private func close() {
        generation &+= 1
        receiveTask?.cancel()
        receiveTask = nil
        socket?.cancel(with: .goingAway, reason: nil)
        socket = nil
        session?.invalidateAndCancel()
        session = nil
        subscription = nil
        connectDeadline = nil
        pingDeadline = nil
    }

    private func connect() {
        close()
        guard running, let url = URL(string: "ws://127.0.0.1:\(port)/") else { return }
        policy.connecting(at: now)
        connectDeadline = now + 10
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.urlCredentialStorage = nil
        configuration.timeoutIntervalForRequest = 10
        let session = URLSession(configuration: configuration, delegate: WalletPushSessionDelegate(), delegateQueue: nil)
        let socket = session.webSocketTask(with: url)
        socket.maximumMessageSize = WalletPushWire.maxFrameBytes
        self.session = session
        self.socket = socket
        let current = generation
        let request = WalletPushWire.request(id: 1, filter: filter, after: cursor)
        socket.resume()
        receiveTask = Task { [weak self] in
            do {
                try await socket.send(.string(String(decoding: request, as: UTF8.self)))
                while !Task.isCancelled {
                    let message = try await socket.receive()
                    guard let self, self.generation == current else { return }
                    let data: Data
                    switch message {
                    case .data(let bytes): data = bytes
                    case .string(let text): data = Data(text.utf8)
                    @unknown default: self.dropped(); return
                    }
                    guard let frame = WalletPushWire.parse(data, subscription: self.subscription) else { continue }
                    if !self.received(frame) { return }
                }
            } catch {
                guard let self, self.generation == current, self.running else { return }
                self.dropped()
            }
        }
    }

    private func received(_ frame: WalletPushFrame) -> Bool {
        switch frame {
        case .subscribed(let id):
            subscription = id
            nextPing = now + 15
            if policy.connected() {
                revision &+= 1
                onDelivery?(.connected)
            }
        case .notice(let notice):
            connectDeadline = nil
            policy.healthy()
            deliveredHeight = max(deliveredHeight ?? 0, notice.height)
            if notice.transactionsChanged { revision &+= 1 }
            onDelivery?(.notice(notice))
        case .gap:
            // The old verified cursor is outside this node's retained window.
            // Current-state reconciliation still uses existing proofs/gates.
            resetCursor()
            revision &+= 1
            dropped()
            onDelivery?(.gap)
            return false
        case .rejected:
            dropped()
            return false
        }
        return true
    }

    private func dropped() {
        close()
        guard running else { return }
        policy.disconnected(at: now, jitter: Double.random(in: 0...1))
    }

    private func tick() {
        guard running else { return }
        onPulse?()
        let time = now
        if let deadline = connectDeadline, time >= deadline { dropped(); return }
        if let deadline = pingDeadline, time >= deadline { dropped(); return }
        if policy.retryDue(at: time) {
            fallbackReads += 1
            revision &+= 1
            onDelivery?(.fallback)
            connect()
            return
        }
        guard policy.isConnected, pingDeadline == nil, time >= nextPing, let socket else { return }
        pingDeadline = time + 10
        nextPing = time + 15
        let current = generation
        socket.sendPing { [weak self] error in
            Task { @MainActor in
                guard let self, self.running, self.generation == current else { return }
                if error != nil { self.dropped() } else { self.pingDeadline = nil }
            }
        }
    }
}
