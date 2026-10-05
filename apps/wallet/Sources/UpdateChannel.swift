import Foundation

/// The canary channel (docs/design/32-health-signal.md §5.2, G2): an appcast
/// item marked `<sparkle:channel>canary</sparkle:channel>` is offered only to
/// a Mac that allows that channel. The channel changes *when* an approved
/// release is offered, never *what* is approved — the same ReleaseLog entry,
/// the same 2/3 signatures and the same 72 hours gate it either way
/// (`ReleaseUpdateGate`).
///
/// Beta has no settings switch: a team Mac turns it on with
/// `defaults write com.pipln.eastsea updateChannel canary`. Anything else —
/// unset, another word, another type — is the default empty set, which is
/// exactly what an installed app without this code does with channel items:
/// ignore them. No Sparkle import here: the rule is a plain unit test
/// (Tests/update-channel).
enum UpdateChannel {
    static let defaultsKey = "updateChannel"
    static let canary = "canary"

    /// What `SPUUpdaterDelegate.allowedChannels(for:)` answers.
    static func allowedChannels(_ defaults: UserDefaults = .standard) -> Set<String> {
        defaults.string(forKey: defaultsKey) == canary ? [canary] : []
    }
}
