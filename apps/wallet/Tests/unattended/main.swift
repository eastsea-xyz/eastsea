// The unattended-restart decisions (docs/design/29-unattended-restart.md),
// pure so this standalone test covers every rule: the switch's default (on
// for Macs in or entering the voting set), attach-vs-start when a daemon node
// already holds the data directory, the pmset/fdesetup readings the honest
// power sentences stand on, and the one argv both the app's node and the
// daemon's node are built from.
//   swiftc -o ./tmp/unattended-check apps/wallet/Sources/UnattendedDecision.swift apps/wallet/Tests/unattended/main.swift && ./tmp/unattended-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

// The exit the node uses when the data directory's run.lock is held — the
// number `NodeController.exited` branches on, so it cannot drift from the
// Rust side's EXIT_LOCKED (crates/node/src/supervisor.rs).
check(UnattendedDecision.lockExitCode == 7, "lock exit code is 7 (EXIT_LOCKED)")

// One argv for both nodes (the single source, `UnattendedDecision.nodeArgv`):
// the app's own child adds --exit-with-parent on top of it, the daemon's
// marker carries it verbatim — so a restart changes nothing about behavior.
let flags = ["--prover-memory", "auto", "--prover-cores", "half"]
let argv = UnattendedDecision.nodeArgv(dataDir: "/tmp/n", rpcPort: 18545, p2pPort: 19101,
                                       networkPath: "/tmp/network.json", proverFlags: flags)
check(argv == ["run", "--data", "/tmp/n", "--rpc-port", "18545", "--port", "19101",
               "--network", "/tmp/network.json"] + flags,
      "argv order is stable: run, data, rpc, p2p, network, then prover flags")
check(!argv.contains("--exit-with-parent"), "the shared argv never carries the app-child-only --exit-with-parent")
check(UnattendedDecision.nodeArgv(dataDir: "/tmp/n", rpcPort: 1, p2pPort: 2, networkPath: nil, proverFlags: [])
    == ["run", "--data", "/tmp/n", "--rpc-port", "1", "--port", "2"],
      "a nil network path leaves --network out")

// The history-storage budget rides the same argv (설정 ▸ 역사 보관): last, so
// the order everything else depends on does not move.
check(UnattendedDecision.nodeArgv(dataDir: "/tmp/n", rpcPort: 1, p2pPort: 2, networkPath: nil, proverFlags: [],
                                  storageFlag: "--max-shards=128")
    == ["run", "--data", "/tmp/n", "--rpc-port", "1", "--port", "2", "--max-shards=128"],
      "a storage flag rides last")
check(UnattendedDecision.nodeArgv(dataDir: "/tmp/n", rpcPort: 1, p2pPort: 2, networkPath: nil, proverFlags: ["--prover-threads=8"],
                                  storageFlag: "--max-shards=0").last == "--max-shards=0",
      "the off flag rides last too, after the prover flags")
check(!UnattendedDecision.nodeArgv(dataDir: "/tmp/n", rpcPort: 1, p2pPort: 2, networkPath: nil, proverFlags: [])
    .contains { $0.hasPrefix("--max-shards") },
      "no storage flag passed means the node's own default holds")

// The switch's default: a Mac in or entering the voting set keeps running
// through restarts by default; a follower does not; a user who ever moved
// the switch owns the choice from then on.
check(UnattendedDecision.defaultEnabled(registered: true), "registered Macs default ON")
check(!UnattendedDecision.defaultEnabled(registered: false), "followers default OFF")
check(UnattendedDecision.effectiveEnabled(userChose: true, current: false, registered: true) == false,
      "the user's OFF beats the registry's default ON")
check(UnattendedDecision.effectiveEnabled(userChose: false, current: false, registered: true) == true,
      "without a user choice the default applies")
check(UnattendedDecision.effectiveEnabled(userChose: true, current: true, registered: false) == true,
      "the user's ON beats the default OFF too")

// What the app does when its start hit the lock: a holder that answers on
// RPC is the daemon's node — attach, never a second node; a silent holder is
// dying, so start our own again.
check(UnattendedDecision.afterLockExit(rpcAlive: true, releaseMatches: true) == .attach, "an answering holder is attached to")
check(UnattendedDecision.afterLockExit(rpcAlive: false) == .retryOwnStart, "a silent holder means retry our own start")
check(UnattendedDecision.shouldAttachOnLaunch(rpcAlive: true, releaseMatches: true), "a node already answering at launch is attached to")
check(!UnattendedDecision.shouldAttachOnLaunch(rpcAlive: false), "nothing answering at launch means a normal start")

// R11: RPC success does not identify the installed release.
check(UnattendedDecision.afterLockExit(rpcAlive: true) == .retryOwnStart,
      "R11 answering daemon with unknown release identity cannot attach")
check(UnattendedDecision.afterLockExit(rpcAlive: true, releaseMatches: false) == .retryOwnStart,
      "R11 mismatched release cannot attach")
check(UnattendedDecision.afterLockExit(rpcAlive: false, releaseMatches: true) == .retryOwnStart,
      "R11 verified identity does not override absent RPC")
let sourceRoot = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
let appSource = try String(contentsOf: sourceRoot.appendingPathComponent("Sources/AetherWalletApp.swift"), encoding: .utf8)
check(appSource.contains("node.prepareForUpdate()") && appSource.contains("applicationShouldTerminate("),
      "R11 automatic and quit-time installation coordinate node shutdown")
check(!UnattendedDecision.mayStopForUpdate(ownProcess: false, attached: false, daemonPresent: true,
          releaseVerified: false, unclaimedRuntimeAbsent: true),
      "R11 unattached daemon cannot be stopped without its live writer lease")
check(!UnattendedDecision.mayStopForUpdate(ownProcess: false, attached: false, daemonPresent: false,
          releaseVerified: false, unclaimedRuntimeAbsent: false),
      "R11 unknown lock or listener cannot become quiescence permission")
check(UnattendedDecision.mayStopForUpdate(ownProcess: false, attached: false, daemonPresent: true,
          releaseVerified: true, unclaimedRuntimeAbsent: false),
      "R11 an attested unattached daemon can be quiesced")
check(UnattendedDecision.mayStopForUpdate(ownProcess: false, attached: false, daemonPresent: false,
          releaseVerified: false, unclaimedRuntimeAbsent: true),
      "R11 positively absent runtime permits installation while holding its lock")
let nodeSource = try String(contentsOf: sourceRoot.appendingPathComponent("Sources/NodeController.swift"), encoding: .utf8)
check(nodeSource.contains("NodeReleaseIdentity.matches(binding: attestedBinding"),
      "R11 shutdown revalidates its exact attested process and listener after actor handoff")

// pmset's actual spelling (`pmset -g` on this Mac, spaces not tabs):
// " autorestart          1" inside the system-wide section.
check(UnattendedDecision.autorestart(from: "System-wide power settings:\n autorestart          1\n sleep                0\n") == true,
      "autorestart 1 reads as on")
check(UnattendedDecision.autorestart(from: " autorestart          0\n") == false,
      "autorestart 0 reads as off")
check(UnattendedDecision.autorestart(from: " sleep                0\n") == nil,
      "no autorestart line reads as unknown")
check(UnattendedDecision.autorestart(from: " Sleep On Power Button 1\n autorestart\t1\n") == true,
      "a similarly named line does not confuse it, and a tab-separated value reads too")

// fdesetup's exact two spellings.
check(UnattendedDecision.fileVault(from: "FileVault is On.\n") == true, "FileVault On reads as on")
check(UnattendedDecision.fileVault(from: "FileVault is Off.\n") == false, "FileVault Off reads as off")
check(UnattendedDecision.fileVault(from: "") == nil, "anything else reads as unknown")

// The honest power sentences: one of these four rows, never a promise macOS
// cannot keep. FileVault on — the unlock screen waits; FileVault off with
// autorestart — nothing to do; FileVault off without — where to turn it on;
// unreadable — say so instead of guessing.
// Both languages, explicitly: the app picks one from its bundle localization
// (never from the Mac's language list), and each must stand on its own.
for ko in [false, true] {
    let fv = UnattendedDecision.powerLines(UnattendedDecision.PowerFacts(fileVault: true, autorestart: true), ko: ko)
    check(fv.count == 1 && fv[0].contains("FileVault"), "FileVault on says so in one sentence (ko: \(ko))")
    check(UnattendedDecision.powerLines(UnattendedDecision.PowerFacts(fileVault: true, autorestart: false), ko: ko) == fv,
          "FileVault on decides the sentence regardless of autorestart (ko: \(ko))")
    let bothOff = UnattendedDecision.powerLines(UnattendedDecision.PowerFacts(fileVault: false, autorestart: false), ko: ko)
    check(bothOff.count == 2 && bothOff[0].contains(ko ? "시스템 설정" : "System Settings"),
          "autorestart off says where to turn it on (ko: \(ko))")
    check(bothOff[1].contains(ko ? "직접" : "by hand"), "and what to do until then (ko: \(ko))")
    // One language per line: no Hangul in English, and Korean lines are Korean.
    let hangul = { (s: String) in s.unicodeScalars.contains { (0xAC00...0xD7A3).contains($0.value) } }
    for line in fv + bothOff { check(hangul(line) == ko, "each line is in the asked language only (ko: \(ko)): \(line)") }
    check(UnattendedDecision.powerLines(UnattendedDecision.PowerFacts(fileVault: false, autorestart: nil), ko: ko).count == 2,
          "an unreadable autorestart is treated as off, never guessed as on")
    check(UnattendedDecision.powerLines(UnattendedDecision.PowerFacts(fileVault: false, autorestart: true), ko: ko).count == 1,
          "FileVault off with autorestart on is the one nothing-to-do row")
    check(UnattendedDecision.powerLines(UnattendedDecision.PowerFacts(fileVault: nil, autorestart: true), ko: ko).count == 1,
          "unreadable power facts say just that")
}
// The default follows the bundle's localization: a bare test binary has none
// but its development language, so it reads English whatever this Mac prefers.
check(UnattendedDecision.powerLines(UnattendedDecision.PowerFacts(fileVault: false, autorestart: false))
      == UnattendedDecision.powerLines(UnattendedDecision.PowerFacts(fileVault: false, autorestart: false),
                                       ko: Bundle.main.preferredLocalizations.first?.hasPrefix("ko") ?? false),
      "the default language is the bundle's, not Locale.preferredLanguages")
