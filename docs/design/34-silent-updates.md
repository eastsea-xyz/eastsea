# 34. Silent updates for validators and wallets (2026-10-07)

The founder's direction (2026-10-07): "자동으로 조용한 업데이트". Updates install
by themselves and quietly, on wallets and on validators. This document builds
on design 19 (release approval), design 24 (self-healing, #11 update failures),
design 29 (unattended restart) and design 32 (canary channel and phased
rollout). It changes none of their approval rules.

## 0. Summary

- **Approval does not change.** Nothing installs by itself without 2 of 3
  builder signatures (3 of 3 for an emergency) over the manifest, found in
  ReleaseLog, plus the 72 h wait. All of that is checked against the
  `release` pin in `network.json` (design 19, B6). Silent means the user
  is not asked. The rules are not relaxed.
- **Wallets already install silently.** `SUAutomaticallyUpdate` is on, checks
  run hourly, and the install starts right away, not at quit
  (`AetherWalletApp.swift`). Three things are missing:
  - **When** to restart. A seated validator must not restart in the same
    minutes as another seat.
  - **Restarting the node** when a daemon started it (design 29). After the
    update the app re-attaches, and the old binary keeps running.
  - **The legacy 7780 path**, which installs on Sparkle's EdDSA signature
    alone, with no builder signatures.
- **Validators restart one at a time, by a slot the chain decides.** Seat *i*
  of *n* may restart for an update only in its own window, inside every
  *n* × 600 blocks, and only while every other seat is visibly live. No
  coordinator and no messages are needed. Two honest seats can never be in a
  planned restart at the same time.
- **Updates come before protocol activation.** The committee signs an
  activation height only for a release that is already approved, and far
  enough ahead for the canary ring, the phased rollout and one full cycle of
  restart slots. A node still on the old version stops one block before the
  switch (already built). The proof program becomes a scheduled parameter,
  so that a verifier change cannot repeat the 0.7.0 mismatch.
- **The user sees nothing.** One plain sentence appears only when the user
  has to act (§5).

## 1. Goals and non-goals

Goals:

1. A Mac that is on and online ends up on the newest approved release with no
   click. The app and its node move together.
2. A planned update restart never takes the committee below quorum. Planned
   restarts never stall finality.
3. Every Mac that follows a protocol activation has updated before the
   activation height. A Mac that has not stops cleanly and does not fork.
4. Approval rules (design 19) stay in force for every automatic install.
5. The user hears about an update only if something needs them.

Non-goals:

- A remote kill switch, or a push that bypasses the 72 h wait. An emergency
  release is still 3/3 and still visible (design 19).
- Downgrades driven by the feed. Rollback stays local (`aether.prev`, the
  watchdog, design 24).
- A coordinator service. Slots come from chain height, which every node
  already agrees on.

## 2. What exists today

| Piece | Where | State |
|---|---|---|
| Sparkle automatic install, hourly check | `apps/wallet/Info-mac.plist` (`SUEnableAutomaticChecks`, `SUScheduledCheckInterval` 3600, `SUAutomaticallyUpdate`) | on |
| Install immediately, not at quit | `AetherWalletApp.swift` `willInstallUpdateOnQuit` → `immediateInstallationBlock()` | on. No timing policy: it installs whenever the download finishes |
| Release gate (2/3, ReleaseLog proof, 72 h, Sparkle signature bound to the manifest) | `ReleaseUpdateGate.swift`, `ReleaseApproval.swift` (`ReleaseTrust.parse`) | on for networks with a `release` pin. **7777/7780 use the legacy Sparkle-only path** (`trust.legacy`) |
| Update failure tracker (discover → … → health, retry by cause) | `UpdateTracker.swift`, design 24 #11 | on |
| Canary channel, phased rollout (6 h × 7 groups) | `UpdateChannel.swift`, `allowedChannels(for:)`, design 32 §5 | client on. The appcast writer change is open (design 32 §6 O3) |
| Rollback to the previous node | `NodeController.swift` `aether.prev`, `NodeWatchdog.swift` | on |
| Health sentences (L1 program mismatch, L8 upgrade required, L9 rolled back) | `HealthCheck.swift` | on. Program **unknown** raises nothing (prover-070-mismatch §C) |
| Committee-signed protocol upgrades, notices, `newest_scheduled`, `upcoming_upgrades` | `crates/node/src/upgrade.rs`, `chain.rs` `next_schedule`, `aether_status` | on. 604,800-block notice on new genesis, unanimity on 7780 |
| The old node stops before new rules | `main.rs` `watch_upgrades`, `EXIT_UPGRADE_REQUIRED`; `docs/ops/upgrade-drill.md` §3 | on, drilled |
| In-app upgrade notice and deadline | `NetworkUpgrade.swift` (`requiresAppUpdate`, `updateDeadline`) | on |
| Availability "leaving"/"back" (the draw skips a leaving candidate) | `candidate.rs:318`, `NodeController.announceAvailability` | on, registry v3 only. It affects the **draw**, not a seat that is already in the committee |
| Unattended node (LaunchDaemon, re-attach) | `UnattendedDaemon.swift`, `UnattendedDecision.swift`, design 29 | on |
| Shadow replay before an upgrade | `aether shadow` (`crates/node/src/shadow.rs`) | on, for history-v2 sources and archive peers |
| Testnet rolling upgrade | `scripts/testnet-upgrade.sh` | unsafe as written (`docs/ops/testnet-7780-upgrade.md` §8) |

## 3. Design

### 3.1 Approval stays the gate

- **New genesis.** No change. The gate passes, and only then does Sparkle
  download. The install (§3.2) happens only for an item that
  `ReleaseUpdateGate.mayProceed` accepted.
- **7780 and 7777 (legacy).** These chains have no ReleaseLog, and their
  `network.json` bytes cannot change. Today any Sparkle-signed item installs
  silently, which breaks the founder's rule. The fix (W3):
  - The release workflow already produces the manifest and the builder
    signatures (`release-approve.py combine`). Ship them next to the DMG.
  - On the legacy path, the gate requires 2/3 valid signatures (3/3 for an
    emergency) from the **builder keys compiled into the running app**,
    over a manifest whose archive SHA-256 and Sparkle signature match the
    item.
  - There is no chain proof and no 72 h wait on legacy chains: these are
    testnets, so say so in the release notes.
  - Without signatures, the item is not installed automatically. The user
    may still click "Check for updates", and the gate applies there too.
- The canary and the phased rollout (design 32 §5) only **delay** an approved
  item. They never let an unapproved item through.

### 3.2 When a wallet installs: the restart window

Sparkle calls `willInstallUpdateOnQuit` once the archive has been verified.
Today we call `immediateInstallationBlock()` at once. Instead, we keep the
block and call it when `UpdateWindow` says now (W1).

`UpdateWindow` is a pure function in a new file, tested like
`UpdateTracker`. Its inputs are the node role and seat, the chain height,
recent proposers, user activity, power, and the deadline. Its rules:

1. **Not seated** (wallet only, follower, or candidate):
   - Install when the user has been idle for at least 2 min, with no send
     sheet open and no signing in flight.
   - Otherwise wait, at most 6 h, then install at the next idle moment.
   - A candidate in the draw first writes `leaving`
     (`announceAvailability`), installs, and the new version writes `back`
     once it has caught up. That mechanism already exists.
2. **Seated validator** (`NodeController.isValidator`):
   - **Slot.** Let *i* be this Mac's index in the current committee of *n*,
     and W = 600 blocks (about 10 min at 1 s blocks). This Mac may restart
     only while `floor(height / W) mod n == i`, after a random delay of 0–3
     min inside the slot (the jitter keeps it clear of the slot boundary).
   - **Everyone else live.** The last 4 × *n* finalized blocks have at least
     `quorum + 1` distinct proposers, with quorum = *n* − *f*. For *n* = 4
     that means all 4. So a planned restart never meets an unplanned outage
     that is already in progress.
   - **Healthy.** `behind` is 0 and the newest block is less than 10 s old.
   - **Missed slot.** If the Mac sleeps through its slot, it waits for the
     next cycle (*n* × W; 40 min for 4 seats, 160 min for 16).
   - **Deadline override.** If an activation (§3.4) is less than two cycles
     away, the liveness condition gives way to the deadline. The slot still
     holds.
3. **Never in the slot twice in a row.** After an update restart, the node
   records the height. It does not restart for another update until 2 × *n*
   × W blocks later, unless the deadline override applies.

These inputs come from the node, which also decides them for operator-run
validators (§3.3). The wallet reads them over RPC (N1) and does not compute
committee indices itself.

### 3.3 The node helper and operator validators: one restart path

There are three places where a stale binary keeps running after its file has
been replaced:

- a daemon-started node that the app re-attaches to (design 29);
- testnet validators under launchd;
- a future headless validator.

A single node-side mechanism covers all three (N2):

- `aether run` (the supervisor) records the inode and mtime of
  `current_exe()` at start and checks them every minute.
- When they change, it runs the new binary once as `aether protocol` (an
  existing subcommand, read-only). If that fails, it stays on the old binary
  and logs one line.
- If the new binary answers, the supervisor waits for the window in §3.2,
  answers `aether_status.restart = {"pending": true, "slot_at": H}`, and
  exits with a new code, `EXIT_UPDATE_RESTART`.
- launchd (`KeepAlive`), the LaunchDaemon, or the app's watchdog then starts
  it on the new file. The watchdog treats this code as a planned exit: no
  backoff, and it does not count toward a crash loop.
- The app's own child node (`--exit-with-parent`) needs nothing new: the
  app's relaunch restarts it. But the app only relaunches inside the window,
  because the app follows §3.2.
- For operator validators, the install becomes atomic and needs no human:
  write the new pair to a versioned directory, then switch one symlink or
  plist. Each validator restarts in its own slot. The runbook gates (`docs/ops/testnet-7780-upgrade.md`
  §4) become automatic checks in the new binary's first minutes. If a check
  fails, the node exits with `EXIT_UPDATE_UNHEALTHY` and the supervisor
  starts the previous binary (`aether.prev`, as the app does).

### 3.4 Updating before protocol activation

1. **Order.** The release is approved first, then the activation is signed.
   `aether upgrade-sign` refuses an upgrade to protocol *P* unless all of
   these hold (N5):
   - the network's ReleaseLog has an approved entry (2/3) whose manifest
     declares `protocol ≥ P` (a new manifest field, a design 19 format
     change);
   - the entry was published at T0;
   - `activate_at − now ≥ 72 h + 24 h canary + 36 h phased + 2 slot cycles +
     24 h margin`. That is about 6.5 days, and it fits inside the 7-day
     mainnet notice (604,800 blocks).
2. **The wallet fast path.** When `upcoming_upgrades` lists a protocol above
   `node_protocol` and an approved item exists, the wallet skips its phased
   group wait (design 32 §5.2). It still needs 2/3 and 72 h, and the slot
   rule still holds. That is a local decision, so no new key is needed (W4).
3. **Stragglers.** A node that has not updated exits one block before the new
   rules (`EXIT_UPGRADE_REQUIRED`). It never signs or executes a block of the
   other history (drilled, `upgrade-drill.md` §3). The app shows L8 (§5).
4. **The proof program becomes a scheduled parameter (N3).** The 0.7.0
   incident happened because the verifier program is compiled in, not
   scheduled. Every commit moves it, and validators and apps from different
   commits disagree.
   - The protocol schedule carries the accepted program id or ids, as a
     committee-signed upgrade on new genesis, or as a compiled per-chain
     table for 7780 (prover-070-mismatch §B).
   - A proof of block *h* verifies against the program scheduled at *h*.
   - Validators ship the verifiers for both the current and the next
     program, so a switch at the activation height needs no restart at that
     moment.
   - In the same lane (N4), the guest build stops taking the HEAD commit
     time. It takes a pinned guest epoch, so only guest-relevant changes
     move the id.
5. **Non-protocol node releases** (no consensus change, like this 7780
   upgrade) need no activation height. The slot rule alone keeps them
   rolling.

### 3.5 Validators on operator Macs, before N2 exists

Until N2 lands, the operator follows `docs/ops/testnet-7780-upgrade.md`: a
versioned release directory, a per-validator plist, and gates (a)–(f). The
rewrite of `scripts/testnet-upgrade.sh` (O1) automates exactly that runbook
and is the first consumer of the slot rule.

## 4. Exists vs gaps

| Need | Exists | Gap |
|---|---|---|
| Install without asking | Sparkle auto install + immediate install | none |
| Approval before install | Gate on new genesis | **legacy 7780 path is Sparkle-only** (W3) |
| Safe restart timing | none | **slot + liveness window** (W1, N1) |
| Node restarts onto the new binary | app child: yes (relaunch) | **daemon-attached node and launchd validators keep the old binary** (N2, W2) |
| Update before activation | notice period, L8, stop at H−1 | **sign only after approval; deadline fast path** (N5, W4) |
| Proof verifier switch | none (compiled `PROGRAM`) | **scheduled program; stable id** (N3, N4) |
| Honest sentence when action is needed | L1/L8/L9, tracker sentences | **program unknown; "will update tonight" is not needed**: silence is the default (W5) |
| Operator rolling upgrade | runbook (this change) | **script rewrite** (O1); **release gate on the live program** (O2) |
| Proof that it works | upgrade drill (A4) | **silent-update drill**: replace the binary on all 4 drill validators at once and show at most one restarts at a time and finality never stops (T1) |

## 5. What the user sees

Nothing in the normal case: no dialog, no badge, no "updated" toast. The
version is in Settings → About as today. The update card on the Network page
keeps the tracker's existing sentence for failures (design 24 #11).

A sentence appears only when the user has to act. There is one sentence per
case, in English and Korean (W5):

| Case | Sentence |
|---|---|
| Install failed 3 times (existing) | "EastSea could not update itself. Please download the new version from eastsea.xyz." |
| Activation within 24 h and this Mac is not updated (for example, the app is not in /Applications, or the disk is full) | "EastSea must update before tomorrow to keep working. Please open it and allow the update." |
| Stopped at activation (L8, existing) | "EastSea stopped because the network moved to a newer version. Updating the app fixes it." |
| Proof program unknown or mismatched, no update yet (prover-070 §C) | "Proving is resting until the network can check this version's proofs. Nothing is lost." |

A seated validator that waits for its slot says nothing. Waiting is normal.

## 6. Lane tasks

Owners as in design 32: **W** wallet (Swift), **N** node (Rust), **O**
operations, **T** test/drill. Each task ships with a test that fails on the
old code first (lead supervision rule).

| Id | Task | Files |
|---|---|---|
| W1 | `UpdateWindow` (pure) + hold `immediateInstallationBlock` until it says now | new `apps/wallet/Sources/UpdateWindow.swift`; `AetherWalletApp.swift` (`willInstallUpdateOnQuit`); new `apps/wallet/Tests/update-window/main.swift`; `scripts/test-swift-pure.sh` |
| W2 | After an update relaunch, a daemon-attached node on an old binary is asked to restart in its slot (via N2), not left running | `NodeController.swift` (`attachToRunningNode`), `UnattendedDaemon.swift`, `UnattendedDecision.swift` |
| W3 | Legacy 7777/7780 path: no automatic install without 2/3 builder signatures over the manifest (compiled keys); ship `manifest.json` + signatures beside the DMG | `ReleaseApproval.swift` (`ReleaseTrust`), `ReleaseUpdateGate.swift` (`mayProceed`, `inspect`), `scripts/release-mac.sh`, `scripts/release-approve.py`, `apps/wallet/Tests/release-approval` |
| W4 | Deadline fast path: skip the phased group wait when an activation needs this update | `NetworkUpgrade.swift`, `UpdateChannel.swift`, `AetherWalletApp.swift` (`allowedChannels`/phasing), tests |
| W5 | The four sentences of §5; program-unknown raises L1 | `HealthCheck.swift`, `Earnings.swift`, `ProverMenu.swift`, `apps/wallet/Tests/health-check/main.swift` |
| N1 | `aether_status.restart`: seat index, *n*, *f*, distinct proposers over 4*n* blocks, current slot, next slot height, pending flag | `crates/node/src/rpc.rs`, `crates/node/src/rotation.rs` (committee index), `crates/node/src/chain.rs` (recent proposers) |
| N2 | Supervisor watches `current_exe`, checks the new binary with `aether protocol`, waits for the slot, exits `EXIT_UPDATE_RESTART`; watchdog treats it as planned; `EXIT_UPDATE_UNHEALTHY` → previous binary | `crates/node/src/supervisor.rs`, `crates/node/src/main.rs`, `apps/wallet/Sources/NodeWatchdog.swift`, tests in `supervisor.rs` |
| N3 | Scheduled verifier program: per-height accepted program, two verifiers loaded during a switch, 7780 compiled table | `crates/node/src/chain.rs` (`pay_proofs`, verifier), `crates/node/src/prover.rs` (`PROGRAM`), `crates/node/src/upgrade.rs`, `crates/node/src/main.rs` (2693-2703) |
| N4 | Stable program id: pinned guest epoch instead of the HEAD commit time | `apps/prover/build-guest.sh`, `scripts/repro-env.sh`, `scripts/prover-program.sh`, `scripts/test-repro-scripts.sh` |
| N5 | `upgrade-sign` precondition: an approved ReleaseLog entry with `protocol ≥ P` and enough lead time; manifest `protocol` field | `crates/node/src/main.rs` (`upgrade-sign`), `crates/node/src/upgrade.rs`, `scripts/release-approve.py`, `docs/design/19-release-approval.md` |
| O1 | Rewrite `testnet-upgrade.sh` to the 7780 runbook: versioned release dir, per-plist switch, poc-m3 over ssh, gates (a)–(f), no rebuild by default | `scripts/testnet-upgrade.sh`, `scripts/testnet-compare-roots.py`, `docs/ops/testnet-7780-upgrade.md` |
| O2 | The release gate compares the app's program with live `aether_proverProgram`; tag the real source commit | `scripts/build-wallet.sh`, `scripts/release-mac.sh` |
| O3 | Appcast with canary + phased items (already design 32 §6 O3) | `scripts/release-mac.sh` |
| T2 | `aether shadow --ignore-legacy-events`: compare receipts without `events` when the stored row predates `a2cb8cd` (7780 blocks before 2026-09-27 22:02), so a full 7780 replay can reach the proof era | `crates/node/src/shadow.rs`, `crates/node/src/main.rs` (`run_shadow`) |
| T1 | Silent-update drill: binary replaced on all 4 drill validators at once → at most one planned restart at a time, finality never older than 30 s, all four on the new binary within one cycle | `scripts/upgrade-drill.sh`, `docs/ops/upgrade-drill.md` |

Suggested order: O1 and N4 first (they unblock 0.7.1), then N1 + N2 + W1 +
W2 together, then T1 as their merge gate, then W3, N3, N5, W4 and W5.

## 7. Risks

- **Slot math depends on a stable committee.** Across a rotation (a reshare),
  indices change. The rule uses the committee at the current height. A
  restart that straddles a rotation is still a single seat, because every
  node uses the same height.
- **The liveness check reads proposers, not votes.** A seat that proposes but
  cannot vote would look live. That is acceptable for a planned restart,
  because the 30 s finality alert still catches it.
- **The legacy path keeps trust in the compiled keys.** A stolen builder key
  cannot install alone (2/3 are needed), as on new genesis. It is still not
  chain-recorded, so say so in the release notes.
- **Deadline override.** Near an activation, the liveness condition gives way.
  The worst case is one planned restart during an unplanned outage, which is
  better than a Mac that misses the activation.
