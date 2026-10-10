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
/// `aether_restartSlot` for this Mac's voting key. Its signed local listener
/// supplies a fresh slot observation alongside voting membership. Missing or
/// expired observations keep a seated Mac waiting, including at quit.
enum UpdateWindow {
    struct Moment: Equatable {
        /// Fresh membership of the running node; nil means unknown.
        var seated: Bool? = nil
        /// The chain-assigned restart slot: true inside it, false outside it,
        /// nil when the node's slot observation is unavailable or expired.
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
        case seatedNoSlot = "this Mac is in the active committee and no verified restart slot is available"

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
        votingMembership(network: network, identity: validatorKey, field: "key")
    }

    /// Recovery can attest the running node's identity without reading its
    /// candidate key. Node IDs and validator keys are separate identities;
    /// every member's node ID must be valid before absence is confirmed.
    static func votingMembership(network: Any?, nodeID: String) -> Bool? {
        votingMembership(network: network, identity: nodeID, field: "node")
    }

    private static func normalizedIdentity(_ text: String) -> String? {
        let lower = text.lowercased()
        let identity = lower.hasPrefix("0x") ? String(lower.dropFirst(2)) : lower
        guard identity.utf8.count == 64,
              identity.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else { return nil }
        return identity
    }

    private static func votingMembership(network: Any?, identity: String, field: String) -> Bool? {
        guard let mine = normalizedIdentity(identity),
              let object = network as? [String: Any],
              let members = object["validators"] as? [[String: Any]],
              !members.isEmpty else { return nil }
        var identities = Set<String>()
        for member in members {
            guard let raw = member[field] as? String, let identity = normalizedIdentity(raw),
                  identities.insert(identity).inserted else { return nil }
        }
        return identities.contains(mine)
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

    /// Rotation can occur between the roster read and the slot read. Either
    /// seated observation keeps the validator gate on; both complete replies
    /// must agree before declaring this Mac unseated.
    static func reconciledMembership(network: Any?, validatorKey: String, restartSlot: Any?) -> Bool? {
        let roster = votingMembership(network: network, validatorKey: validatorKey)
        guard let slot = restartSlot as? [String: Any],
              let count = slot["committee_size"] as? Int, count > 0,
              let seat = slot["seat_index"] else { return roster == true ? true : nil }
        let slotSeated: Bool
        if let index = seat as? Int, (0..<count).contains(index) { slotSeated = true }
        else if seat is NSNull { slotSeated = false }
        else { return roster == true ? true : nil }
        if roster == true || slotSeated { return true }
        return roster == false ? false : nil
    }
}
