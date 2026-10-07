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
        /// This Mac's node runs and the chain has it in the active committee.
        var seated = false
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
        case outOfSlot = "this validator is outside its restart slot"
        case seatedNoSlot = "this Mac is in the active committee and the chain publishes no restart slot yet; installing when it leaves the committee, or at quit"

        var logLine: String { "update downloaded; waiting for safe moment: \(rawValue)" }
    }

    enum Decision: Equatable {
        case installNow
        case wait(Reason)
    }

    static func decide(_ m: Moment) -> Decision {
        // Whatever the user is doing comes first, validator or not.
        if m.sendSheetOpen { return .wait(.sendSheet) }
        if m.signing { return .wait(.signing) }
        if m.migrating { return .wait(.migration) }
        if m.storageMoving { return .wait(.storageMove) }
        guard m.seated else { return .installNow }
        switch m.inOwnSlot {
        case true?: return .installNow
        case false?: return .wait(.outOfSlot)
        case nil: return .wait(.seatedNoSlot)
        }
    }
}
