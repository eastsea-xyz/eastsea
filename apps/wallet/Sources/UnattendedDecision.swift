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
                         locationFlags: [String] = [], presenceFlags: [String] = []) -> [String] {
        var out = ["run", "--data", dataDir, "--rpc-port", String(rpcPort), "--port", String(p2pPort)]
        if let networkPath { out += ["--network", networkPath] }
        out += proverFlags
        if let storageFlag { out += [storageFlag] }
        // 블록 데이터 위치 / archive (`BlockDataLocation.flags`): last, so the
        // order everything above depends on does not move.
        out += locationFlags
        out += presenceFlags
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

    static func afterLockExit(rpcAlive: Bool, releaseMatches: Bool = false) -> AfterLockExit {
        rpcAlive && releaseMatches ? .attach : .retryOwnStart
    }

    /// What the app does before starting a node at all: a daemon node that
    /// survived the reboot answers on the node's RPC port.
    static func shouldAttachOnLaunch(rpcAlive: Bool, releaseMatches: Bool = false) -> Bool { rpcAlive && releaseMatches }

    /// Whether shutdown can prove that every writer retains ownership.
    static func mayStopForUpdate(ownProcess: Bool, attached: Bool, daemonPresent: Bool,
                                 releaseVerified: Bool, unclaimedRuntimeAbsent: Bool) -> Bool {
        if ownProcess || attached || daemonPresent { return releaseVerified }
        return unclaimedRuntimeAbsent
    }

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
    /// change; where a person must act, it says where. `ko` defaults to the
    /// language the app bundle is shown in, so these lines never differ from
    /// the screen around them.
    static func powerLines(_ facts: PowerFacts,
                           ko: Bool = Bundle.main.preferredLocalizations.first?.hasPrefix("ko") ?? false) -> [String] {
        switch (facts.fileVault, facts.autorestart) {
        case (true?, _):
            return [ko
                ? "FileVault가 켜져 있어요. 정전이 나면 이 Mac은 잠금 화면에서 기다려요 — 한 번 잠금을 풀면 노드가 저절로 돌아와요. macOS 업데이트로 재시동하면 잠금 없이 돌아와요."
                : "FileVault is on. After a power cut this Mac waits at the unlock screen; unlock it once and the node comes back by itself. macOS update restarts come back unlocked."]
        case (false?, true?):
            return [ko
                ? "정전이 나도 이 Mac은 저절로 켜지고, 로그인 없이 노드가 돌아와요."
                : "After a power cut this Mac starts up by itself and the node comes back without anyone logging in."]
        case (false?, _):
            return [ko
                ? "정전 후 자동 켜기가 꺼져 있어요. 시스템 설정 ▸ 배터리(또는 에너지)에서 '정전 후 자동으로 켜기'를 켜 주세요."
                : "\"Start up automatically after a power failure\" is off. Turn it on in System Settings ▸ Battery (or Energy).",
                ko
                ? "그 전까지는 정전 후 이 Mac을 직접 켜 주세요. 켜면 로그인 없이 노드가 돌아와요."
                : "Until then, turn this Mac on by hand after a power cut; the node then comes back without logging in."]
        default:
            return [ko
                ? "전원 설정을 읽을 수 없어요. 정전 뒤에는 이 Mac을 직접 켜 주세요."
                : "Power settings cannot be read. Turn this Mac on by hand after a power cut."]
        }
    }
}
