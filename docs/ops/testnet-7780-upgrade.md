# 7780 testnet: moving all four validators to the 0.7.0 node (runbook)

Written 2026-10-07 for the founder's request "검증자들 버전을 다 올려야겠는데?". The
dry run that backs every claim here is in `.claude/team/upgrade-dryrun/`
(commands, logs and outputs). Nothing in this runbook has been run against
the live validators yet. The lead approves it first (see the end).

## 1. What we install, and what we must not install

| File | Source | SHA-256 (unsigned) | Proof program |
|---|---|---|---|
| `aether` | `target/release/aether`, built 2026-10-07 09:29 | `b1b75090566…1ce9` | embeds `745b6736…` |
| `aether-prover` | `apps/prover/target/release/aether-prover`, built 09:15 | (record at install) | `aether-prover info` → `745b6736b97da286…9ab8d4` |
| running now | `~/aether-testnet/bin/aether`, 2026-09-29 10:49, about `ad84a35` | `fb97df95c9ba…56f8` | `3e9c8976…` |

- **Use the 09:29/09:15 artifacts, not a fresh build of `lead-merge`.** Their
  verifier program is `745b6736…`, the same as EastSea 0.7.0 (build 13), so
  0.7.0 provers start verifying again once the validators run them.
- The proof program id moves with every commit, because the guest build uses
  the HEAD commit time and pulls in `aether-execution`, `aether-state` and
  `aether-types` (`.claude/team/prover-070-mismatch.md` §1). A build of
  `lead-merge` HEAD (`d8fd469`) would get a fourth id and break 0.7.0 again.
  `scripts/testnet-upgrade.sh` rebuilds from HEAD, so do **not** run it for
  this upgrade (§8 lists its other problems).
- Commit: the binary was built from about `e187cbf` (09:15). The last crate
  change before it was `37d67a6` (06:12), so its Rust source equals `37d67a6`.
  The tag `app-v0.7.0` points at a notes-only commit, so no record pins the
  exact source of build 13. Two node commits came after the binary
  (`b8a5ba2`, a pending-nonce index plus a CLI fee clamp, and `0962953`, the
  wallet). Neither changes block validity (commit message of `b8a5ba2`), and
  both wait for 0.7.1.
- **The sidecar must match.** The node refuses a sidecar whose guest is not
  its compiled `PROGRAM`. Without a verifier it refuses every block that
  carries proofs (`chain.rs:584`). The node finds the sidecar next to its own
  executable (`prover.rs:26-36`), so both files go in the same directory.

## 2. Is this upgrade consensus-relevant?

**Yes, but only in one narrow way: which proofs a block can carry. Mixed
versions do not fork state.**

1. **Finalized blocks never depend on the local verifier.** A node applies a
   finalized block through `execute_certified`. That path pays its proofs
   with a verifier that always says yes, because the quorum that certified
   the block already checked them (`chain.rs:1186-1195`, `1427-1437`). The
   state transition is a pure function of the block bytes. An old node and
   a new node that apply the same certified block get the same state root,
   proof rewards included.
2. **Voting does depend on it.** A proposal is executed with
   `certified=false`. `pay_proofs` (`chain.rs:3735-3770`) asks the local
   verifier, and a proof it rejects makes the whole proposal invalid
   ("proof of block N does not verify"). Proof rewards are state
   (`aether_execution::proofs::pay`). So a block carrying a `745b6736` proof
   needs 3 of the 4 validators to run `745b6736`, and a block carrying a
   `3e9c8976` proof needs 3 on the old program.
3. **What happens when versions are mixed.** A proposer whose proof block
   loses stops putting proofs in its proposals for 100 blocks (`PROOF_BACKOFF`,
   `chain.rs:2948-2953`), and the chain carries on without proofs. The cost is
   a few slower views and unpaid proofs, never two histories. With 2 old and
   2 new validators, no proof of either program can be included.
4. **Today that cost is close to zero.** Since height 155,910 (2026-09-29
   15:08Z) no shipped prover has made a `3e9c8976` proof. 0.7.0 pauses, and
   Aether 0.6.6 makes `03b7c50a` proofs that both programs reject.
5. **Other rules.** The protocol stays 3 (`newest_scheduled` 3 on all four).
   Nothing in this tree defines protocol 4, so there is no activation height
   to schedule: the committee-signed upgrade path (`testnet-activate.sh`)
   does not apply. 7780 keeps its legacy flags (`node_rewards`, `history_v2`
   off), and its `network.json` (sha256 `26faa6bc…`, the same on all four) is
   the shipped legacy file. That file skips the ceremony bind
   (`mainnet.rs:653`, `main.rs:3232`), so the new `aether run` starts these
   data dirs without a `ceremony-check.json`. The commonware version is
   unchanged since 9-29 (`Cargo.lock`), so the vote journal and the block
   archive keep the same format.
6. **The open risk.** We showed that the new binary accepts and re-executes
   blocks built by old proposers, with identical roots (§5). We could not show
   offline that **old** validators accept proposals **built** by a new one:
   the canonical-bytes check (`chain.rs:1208`) would refuse a payload that
   encodes differently. The gate after the first restart checks this live
   (§4, gate c). It is also why we upgrade one validator at a time.

**Method:** a rolling restart, one validator at a time, keeping 3 of 4 voting.
A stop-all/swap/start-all switch would stop the chain and run four
`AETHER_RECOVER_CONSENSUS` restarts at once, and it buys nothing because
mixed versions cannot fork. Keep the mixed window short (about 30 minutes for
all four), because proofs stop while it is open.

## 3. Pre-flight (read-only; abort if any check fails)

```bash
R=$HOME/aether-testnet/releases/0.7.0-745b6736
df -h / ; ssh poc-m3 'df -h /'            # at least 10 GB free on each Mac
for p in 8601 8602 8603; do curl -s localhost:$p -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"aether_status","params":[]}'; echo; done
ssh poc-m3 "curl -s localhost:8604 -H 'content-type: application/json' -d '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"aether_status\",\"params\":[]}'"
# all four: same height ±2, same hash at a common height, protocol 3, behind 0
tail -5 ~/aether-soak/alerts.log           # no open finality or hash alert
strings target/release/aether | grep -c 745b6736         # 1
apps/prover/target/release/aether-prover info            # guest_elf_sha256 745b6736…
pgrep -f "cargo (build|test)|bin/rustc "                 # empty: no build is rewriting target/
```

Then tell the soak (§6) and write the start time into
`~/aether-soak/alerts.log` by hand: `echo "$(date -u +%FT%TZ) NOTE upgrade start" >> ~/aether-soak/alerts.log`.

## 4. The upgrade

**Install beside the old binaries.** `~/aether-testnet/bin` stays untouched as
the rollback target. Each plist then points at its own binary, so a validator
that launchd restarts on its own cannot switch versions by surprise. (With a
shared `bin/aether`, a crash would quietly upgrade an old validator.) Keep the
files on the internal disk: launchd agents cannot run binaries from
`/Volumes/workspace` (memory note `testnet-validator-layout`).

```bash
mkdir -p "$R"
cp target/release/aether apps/prover/target/release/aether-prover "$R/"
codesign --force --options runtime --timestamp \
  --sign "Developer ID Application: Pipln (45WU468FZE)" "$R/aether" "$R/aether-prover"
"$R/aether-prover" info                   # still 745b6736… after signing
shasum -a 256 "$R"/* | tee "$R/SHA256SUMS"
rsync -a "$R/" poc-m3:aether-testnet/releases/0.7.0-745b6736/
ssh poc-m3 'cd ~/aether-testnet/releases/0.7.0-745b6736 && shasum -a 256 -c SHA256SUMS'
```

**Order: v3, v2, v4 (poc-m3), v1.** v3 is the validator the dry run cloned.
v1 goes last because it holds the faucet, DeviceCheck and registrar
arguments, and because scripts and apps read 8601.

**One validator (local, N = 3, 2, then 1):**

```bash
N=3; L=com.pipln.aether.testnet.v$N; P=~/Library/LaunchAgents/$L.plist
cp "$P" ~/aether-testnet/releases/$L.plist.0929       # once per plist: the rollback copy
/usr/libexec/PlistBuddy -c "Set :ProgramArguments:0 $R/aether" "$P"
launchctl bootout gui/$(id -u)/$L                     # SIGTERM; wait until pgrep shows it gone
while pgrep -f "aether-testnet/$N/network.json" >/dev/null; do sleep 1; done
launchctl bootstrap gui/$(id -u) "$P"
```

**v4 on poc-m3:** the same commands over `ssh poc-m3`, with `N=4`, port 8604
and log `~/aether-testnet/node4.log`. The plist on poc-m3 has no `--node-arg`
lines. That is expected.

**Gates before the next validator.** Run all of them. Any failure means
rollback (§7) for this validator and stop.

- (a) **It serves and keeps up.** Within 3 minutes the RPC answers,
  `catching_up` is false, and its height is within 2 of the others. In the dry
  run the new binary opened a 2 GB validator data dir in about 10 s on first
  start and in 3 s on later starts (`11-first-open.log`, `10-follow-new2.log`).
- (b) **It runs the new program.** `aether_proverProgram` answers on its port
  (the old binary says "method not found") with `745b6736…`. Its log shows
  `proof verifier ready program=745b6736…`.
- (c) **Old validators accept its blocks.** Within 10 minutes, the last 100
  finalized blocks have 4 distinct proposers (the soak monitor's
  `proposers_last100` column, or `aether_getBlock` → `proposer`). This
  validator must be among them. This gate covers the open risk in §2.6.
- (d) **Same state.** Compare the hash and state root block by block against
  an old validator for at least 200 blocks after its restart:
  `python3 scripts/testnet-compare-roots.py <new_port> <old_port> <restart_height>`
  must print `mismatched=0`.
- (e) **Clean log.** Since the restart, the log has none of `ParentRootMismatch`,
  `BadPayload`, `HistoryMismatch`, `UPGRADE REQUIRED`, `TooNew`, `vote journal`,
  or `panicked`. In the mixed window, `does not verify` for proofs is expected
  and is not a failure.
- (f) **Finality.** The newest finalized block is less than 30 s old (no
  open soak finality alert).

After v1: all four answer `aether_proverProgram` with `745b6736…`. Within an
hour, 0.7.0 provers leave `paused="program"` (`aether_proverStatus` on a 0.7.0
Mac) and the first `proof` reward records reappear (`aether_rewards`). Aether
0.6.6 provers (poc-m3's `/Applications/Aether.app`) keep failing until they
update. Ask the founder to update or quit that app. It wastes CPU every hour.

## 5. What the dry run proved (2026-10-07, 13:24–13:52 KST)

Full record: `.claude/team/upgrade-dryrun/`. The live validators were only
read: their RPC, and an APFS clone of v3's data dir. Nothing was restarted,
and no file under `~/aether-testnet` or in poc-m3's `~/aether-testnet` was
changed.

1. **Clone.** `cp -c -R ~/aether-testnet/3 ~/aether-upgrade-dryrun/v3` at
   height 495,420, while v3 was running (a crash-consistent copy). From the
   clone we removed `validator.key`, `threshold.json`, `node-account.key`,
   `faucet.key`, `registrar.key` and `dkg-runtime`, so the clone could never
   vote or sign. `/` had 16–17 GB free throughout, and the clone grew by
   about 0 GB.
2. **The new binary opens the old data.** `aether head` on the clone gives
   the same height and hash with both binaries
   (`495420 7c5b3bc3…`). The new store logs
   `no schema version recorded: adopting this database as version 1 in place`
   (schema versioning arrived in `6d3f37c`, 10-03).
3. **It stays in sync with identical roots.** We ran `aether follow` (new
   binary) on the clone with HTTP upstreams only (`--from-rpc` 8601–8603: no
   iroh endpoint, no announcement), `--prover-max-memory 0`, on port 18783.
   It replayed from 495,421 and followed the live chain for 25 minutes. Every
   block from 495,421 to 496,840 (1,420 blocks, checked against v1, v2 and
   v3) has the same block hash and state root (`05-…`, `12-…`, `13-…`,
   `16-compare-final.txt`: `mismatched=0`). No WARN or ERROR lines appeared.
4. **Rollback works on the data the new binary wrote.** After the new binary
   ran, the 9-29 binary read the clone (`head` → `495992 a3b0437b…`). It then
   followed the chain on that clone, and all 63 blocks matched
   (`09-compare-rollback.txt`). The new binary then reopened the same data
   (roll forward again) and stayed in sync.
5. **History replay, partial.** `aether shadow` (new binary) replayed from
   genesis against v2's RPC into a scratch store at about 1,000 blocks a
   minute.
   - Heights 1 to 15,734 match: state roots and receipts.
   - At 15,735 the state root also matches. The replay then stops on the
     **receipts digest** only. The stored receipt has no `events` (`logs: 1`,
     `14-receipt-15735-v2.json`) because a binary older than `a2cb8cd`
     executed that block (2026-09-27 20:35 KST, 87 minutes before
     `Receipt.events` existed). The replay fills `events` in.
   - 7780 commits no receipts root (`chain.rs`: `expected_receipts_root` only
     with `node_rewards || history_v2`), so this is a display difference in
     old stored rows, not a rule change.
   - `shadow` stops at the first mismatch, so the replay never reached the
     proof era (26,477 to 155,910) or protocol 3 (79,875). Lane task: a
     `shadow` option that ignores `events` on receipts stored without them.
   - A second follower with an empty data dir took a certified snapshot from
     an old validator (jumping 0 to 496,591) and followed from there. That
     shows snapshot sync across versions works. It is not a history replay.
6. **Not covered.** We did not run `aether node`, meaning voting, the vote
   journal or block archive writes, on the clone. That would mean running a
   second copy of v3's committee identity and share, which could double-sign.
   The live gates (a)–(f) cover that part, one validator at a time.

## 6. The soak (A5)

A5 runs on the 9-29 binary until 10/9 10:00. That binary is 26 commits to the guest's
crates and about 70,000 changed lines (`git diff --stat ad84a35 e187cbf --
crates`) away from what we would launch: no schema versioning, none of the
10-03 self-healing exits, no B5 fee rules, no `aether_proverProgram`, and no
working proof market since 9-29. A pass would show that the four Macs,
Tailscale and launchd stay up. It would show little about the mainnet code.

**Recommendation: upgrade now and restart the A5 clock** once v1 passes its
gates. A5 then ends 72 h after that (about 10/10 15:00 if the upgrade
finishes around 15:00 today).

- **Cost.** The beta gate moves about 29 h later, from 10/9 10:00 to about
  10/10 15:00. The roughly 27 h already accrued count only as evidence for
  the infrastructure. A second restart is possible: if 0.7.1 ships during
  the soak and the validators must follow it for the proof program, the
  clock restarts again. So cut 0.7.1 before the new A5 starts, or after it
  ends, never in the middle.
- **Gain.** The soak then also covers the proof market, which has been dead
  on 7780 since 9-29 (0.7.0 provers resume), plus the store schema, the
  self-healing paths and the B5 fee rules, all on the build the users have.
- **Monitor.** `~/aether-soak` needs no change. Each restart triggers a short
  "stops answering"/"falls behind" alert pair. The NOTE lines from §3 mark
  them as planned.

## 7. Rollback

One validator, any time. This is also the path if a gate fails.

```bash
cp ~/aether-testnet/releases/$L.plist.0929 "$P"       # ProgramArguments:0 back to ~/aether-testnet/bin/aether
launchctl bootout gui/$(id -u)/$L; while pgrep -f "aether-testnet/$N/network.json" >/dev/null; do sleep 1; done
launchctl bootstrap gui/$(id -u) "$P"
```

- The data dir needs nothing: the 9-29 binary reads what the new one wrote
  (§5.4). The schema stamp is a row the old binary ignores.
- Roll back in the reverse order (v1, v4, v2, v3) if more than one validator
  must go back.
- If the old binary ever answers with a store error on a data dir, do not
  delete anything. `follow::move_aside` keeps the old files under
  `corrupt-<time>/`. Follow `docs/ops/consensus-recovery.md`.
- Two validators down at the same time stops the chain (3 of 4 needed).
  Never start the next validator while one is unhealthy.

## 8. Follow-ups (not needed for this upgrade)

- `scripts/testnet-upgrade.sh` is unsafe as written:
  - It rebuilds from HEAD, so it gets a new proof program.
  - It overwrites the shared `bin/aether`, so it cannot roll back one
    validator.
  - It loops over the local `~/aether-testnet/[0-9]*`, which still contains a
    stale `4/` from before v4 moved to poc-m3 on 10-06. It then runs
    `kickstart` on an unloaded `v4` label and aborts with v4 never upgraded.
  - Its only gate is height. Rewrite it around §4: versioned release dirs,
    plist pointing, gates (a)–(f), and poc-m3 over ssh. Lane task V1 in
    `docs/design/34-silent-updates.md`.
- Tag the exact source commit of every app release (`app-vX` on the source,
  not on a notes commit). Make `build-wallet.sh` refuse to package for 7780
  when its program differs from the live `aether_proverProgram`
  (prover-070-mismatch §3).
- For 0.7.1: build the validators from the same commit as the app, then
  repeat this runbook with that release dir.

## 9. What the lead approves before anyone touches the live validators

1. **The artifacts.** Install the 09:29 `aether` and the 09:15
   `aether-prover` (program `745b6736…`, the same as 0.7.0), not a HEAD
   build. Record their source as about `e187cbf`, with crates equal to
   `37d67a6`.
2. **The method and order.** A rolling restart in the order v3, v2, v4, v1,
   with gates (a)–(f), and the plist change that points each validator at
   its own versioned release dir.
3. **The soak.** Restart A5 when v1 passes its gates (about +29 h to the beta
   gate), and hold 0.7.1 until the new A5 finishes.
4. **The time window.** About 45 minutes with someone watching. Do not start
   while a build is rewriting `target/`, during the 6-hourly 1,000-transfer
   burst, or while another lane is restarting anything.
5. **The open items.** Accept that §2.6 (old validators accepting blocks
   built by the new binary) and the proof-era history replay (§5.5) are
   checked live by gate (c), not offline.
