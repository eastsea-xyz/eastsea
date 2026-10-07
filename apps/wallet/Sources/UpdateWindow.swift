import Foundation

/// When a downloaded, verified update installs (docs/design/34-silent-updates.md
/// §3.2, W1). Sparkle hands the app an `immediateInstallationBlock` once the
/// archive passed EdDSA verification (and, before the download,
/// `ReleaseUpdateGate`). The app keeps that block and calls it only when this
/// says `installNow`: no prompt, no click, and nothing the user is in the
/// middle of gets cut off. Pure, no Sparkle import (Tests/update-window).
///
/// Seated validators: design 34's slot rule (seat i of n restarts only while
/// floor(height / 600) mod n == i) needs the node to publish the slot in
/// `aether_status.restart` (N1), which no node does yet. Until it does,
/// `inOwnSlot` is nil and a seated Mac waits: it installs as soon as
/// `aether_status` no longer has it in the voting set, and otherwise when the
/// app quits (Sparkle always installs a downloaded update at termination).
/// Once N1 lands, the caller passes the node's answer and the slot decides.
enum UpdateWindow {
    struct Moment: Equatable {
        /// Fresh membership of the running node; nil means unknown.
        var seated: Bool? = nil
        /// The chain-assigned restart slot: true inside it, false outside it,
        /// nil while the node does not publish one (N1 not built).
        var inOwnSlot: Bool? = nil
        /// The send sheet is open (the user is composing or confirming a send).
        var sendSheetOpen = false
        /// A Touch ID prompt or a signing is in flight.
        var signing = false
        /// The Aether -> EastSea data migration is running.
        var migrating = false
        /// The node's block data is moving to another disk.
        var storageMoving = false
    }

    enum Reason: String, Equatable {
        case sendSheet = "the send sheet is open"
        case signing = "a Touch ID prompt or signing is in flight"
        case migration = "the data migration is running"
        case storageMove = "the block data is moving"
        case membershipUnknown = "this Mac's current voting membership is unknown"
        case outOfSlot = "this validator is outside its restart slot"
        case seatedNoSlot = "this Mac is in the active committee and the chain publishes no restart slot yet; installing when it leaves the committee, or at quit"

        var logLine: String { "update downloaded; waiting for safe moment: \(rawValue)" }
    }

    enum Decision: Equatable {
        case installNow
        case wait(Reason)
    }

    /// A successful membership read grants permission only briefly. The
    /// monotonic timestamp is the request start, so a delayed response never
    /// looks freshly observed. Node lifecycle changes invalidate its generation.
    struct MembershipSnapshot {
        static let maxAge: TimeInterval = 15
        private(set) var generation: UInt64 = 0
        private var membership: Bool?
        private var requestedAt: TimeInterval?

        init() {}

        mutating func invalidate() {
            generation &+= 1
            membership = nil
            requestedAt = nil
        }

        @discardableResult
        mutating func observe(_ seated: Bool?, requestedAt: TimeInterval, generation: UInt64) -> Bool {
            guard generation == self.generation else { return false }
            membership = seated
            self.requestedAt = seated == nil ? nil : requestedAt
            return true
        }

        func value(at now: TimeInterval) -> Bool? {
            guard let requestedAt, now >= requestedAt,
                  now - requestedAt <= Self.maxAge else { return nil }
            return membership
        }
    }

    /// Only a complete, valid voting set can confirm this key is absent.
    /// Followers forward aether_network to a validator. Transport failures,
    /// null responses and malformed member rows remain unknown.
    static func votingMembership(network: Any?, validatorKey: String) -> Bool? {
        func normalizedKey(_ text: String) -> String? {
            let lower = text.lowercased()
            let key = lower.hasPrefix("0x") ? String(lower.dropFirst(2)) : lower
            guard key.utf8.count == 64,
                  key.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else { return nil }
            return key
        }
        guard let mine = normalizedKey(validatorKey),
              let object = network as? [String: Any],
              let members = object["validators"] as? [[String: Any]],
              !members.isEmpty else { return nil }
        var keys = Set<String>()
        for member in members {
            guard let raw = member["key"] as? String, let key = normalizedKey(raw),
                  keys.insert(key).inserted else { return nil }
        }
        return keys.contains(mine)
    }

    static func decide(_ m: Moment) -> Decision {
        // Whatever the user is doing comes first, validator or not.
        if m.sendSheetOpen { return .wait(.sendSheet) }
        if m.signing { return .wait(.signing) }
        if m.migrating { return .wait(.migration) }
        if m.storageMoving { return .wait(.storageMove) }
        guard let seated = m.seated else { return .wait(.membershipUnknown) }
        guard seated else { return .installNow }
        switch m.inOwnSlot {
        case true?: return .installNow
        case false?: return .wait(.outOfSlot)
        case nil: return .wait(.seatedNoSlot)
        }
    }
}
