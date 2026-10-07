import Foundation

/// The unattended-restart decisions (docs/design/29-unattended-restart.md),
/// pure so a standalone test covers every rule: the switch's default (on for
/// Macs in or entering the voting set), attach-vs-start when a daemon node
/// already runs, the pmset/fdesetup readings, and the honest power sentences.
enum UnattendedDecision {
    /// The node the daemon runs and the node the app runs take the same
    /// arguments, so a restart never changes behavior — except the app's own
    /// child also gets `--exit-with-parent` (the daemon has no parent that
    /// dies; the app's child must never outlive the app). This is the single
    /// source for both (Tests/unattended). `storageFlag` is the resolved
    /// history-storage budget (설정 ▸ 역사 보관, `StorageSetting.flag`),
    /// appended last so the flag order stays stable.
    static func nodeArgv(dataDir: String, rpcPort: UInt16, p2pPort: UInt16,
                         networkPath: String?, proverFlags: [String], storageFlag: String? = nil,
                         locationFlags: [String] = []) -> [String] {
        var out = ["run", "--data", dataDir, "--rpc-port", String(rpcPort), "--port", String(p2pPort)]
        if let networkPath { out += ["--network", networkPath] }
        out += proverFlags
        if let storageFlag { out += [storageFlag] }
        // 블록 데이터 위치 / archive (`BlockDataLocation.flags`): last, so the
        // order everything above depends on does not move.
        out += locationFlags
        return out
    }

    /// The switch defaults ON for a Mac in or entering the voting set
    /// (registered candidates beacon and can be picked at any epoch): its
    /// absence after a reboot costs the network a signature. A follower keeps
    /// the default OFF — nothing stops when it reboots. A user who ever moved
    /// the switch by hand owns the choice from then on.
    static func defaultEnabled(registered: Bool) -> Bool { registered }

    /// The default applies only until the user moves the switch once.
    static func effectiveEnabled(userChose: Bool, current: Bool, registered: Bool) -> Bool {
        userChose ? current : defaultEnabled(registered: registered)
    }

    /// Exit code the node uses when the data directory's `run.lock` is held:
    /// "already running" — not a crash, nothing to restart.
    static let lockExitCode: Int32 = 7

    /// What the app does when its own start hit the lock. If the holder answers
    /// on RPC it is the daemon's node: attach to it (one node per data dir —
    /// never a second one). If nothing answers, the holder is dying (or a
    /// leftover): try starting our own again.
    enum AfterLockExit: Equatable {
        case attach
        case retryOwnStart
    }

    static func afterLockExit(rpcAlive: Bool) -> AfterLockExit {
        rpcAlive ? .attach : .retryOwnStart
    }

    /// What the app does before starting a node at all: a daemon node that
    /// survived the reboot answers on the node's RPC port.
    static func shouldAttachOnLaunch(rpcAlive: Bool) -> Bool { rpcAlive }

    /// The power facts behind the honest sentences (docs/design/29):
    /// `pmset -g`'s `autorestart` ("start up automatically after a power
    /// failure", System Settings ▸ Battery/Energy) and `fdesetup status`.
    /// `nil` = the tool could not be read; the app never changes either.
    struct PowerFacts: Equatable {
        var fileVault: Bool?
        var autorestart: Bool?
    }

    /// `autorestart 1` in `pmset -g`'s system-wide section (the value is the
    /// line's last token — pmset prints runs of spaces, some sections tabs).
    static func autorestart(from pmsetOutput: String) -> Bool? {
        for line in pmsetOutput.split(separator: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard trimmed.hasPrefix("autorestart") else { continue }
            let value = trimmed.split(whereSeparator: { $0 == " " || $0 == "\t" }).last
            if value == "1" { return true }
            if value == "0" { return false }
        }
        return nil
    }

    /// "FileVault is On." / "FileVault is Off." — the exact two spellings
    /// `fdesetup status` prints.
    static func fileVault(from fdesetupOutput: String) -> Bool? {
        let out = fdesetupOutput.trimmingCharacters(in: .whitespacesAndNewlines)
        if out.hasSuffix("FileVault is On.") { return true }
        if out.hasSuffix("FileVault is Off.") { return false }
        return nil
    }

    /// The honest sentences about what happens after a power cut, in the app's
    /// language. Nothing here offers to change a setting the app cannot
    /// change; where a person must act, it says where.
    static func powerLines(_ facts: PowerFacts, locale: Locale = .current, bundle: Bundle = .main) -> [String] {
        switch (facts.fileVault, facts.autorestart) {
        case (true?, _):
            return [
                String(localized: "FileVault is on. After a power cut this Mac waits at the unlock screen; unlock it once and the node comes back by itself. macOS update restarts come back unlocked.", bundle: bundle, locale: locale),
            ]
        case (false?, true?):
            return [
                String(localized: "After a power cut this Mac starts up by itself and the node comes back without anyone logging in.", bundle: bundle, locale: locale),
            ]
        case (false?, _):
            return [
                String(localized: "\"Start up automatically after a power failure\" is off. Turn it on in System Settings ▸ Battery (or Energy).", bundle: bundle, locale: locale),
                String(localized: "Until then, turn this Mac on by hand after a power cut; the node then comes back without logging in.", bundle: bundle, locale: locale),
            ]
        default:
            return [
                String(localized: "Power settings cannot be read. Turn this Mac on by hand after a power cut.", bundle: bundle, locale: locale),
            ]
        }
    }
}
