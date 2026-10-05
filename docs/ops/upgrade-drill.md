# Upgrade drill (mainnet beta gate A4)

`scripts/upgrade-drill.sh` rehearses a protocol upgrade end to end on a
throwaway local chain. It is the answer to checklist A4 — "재실행 가능한
업그레이드 리허설" (`docs/03-analysis/feature-checklist-2026-10-05.md`):
anyone with the repo can run the same script, on any Mac, and watch every
upgrade emergency behave the same way. A green run is printed at the end
(`PASS: …` per claim); any failed check makes the script exit 1.

```bash
scripts/upgrade-drill.sh          # ~4 min incremental builds + ~10 min chain
```

Needs: rustup cargo (the repo toolchain), foundry (`cast`/`forge`), swiftc,
python3 with `cryptography`. Set `AETHER_DRILL_PROVER=/path/to/aether-prover`
to reuse an existing proving sidecar (the first run otherwise builds one from
`apps/prover`; `scripts/prover-program.sh` documents that path). Everything
the run creates lives under `tmp/` (throwaway); the run log is
`tmp/upgrade-drill.log`.

## What the drill proves

The chain is built the way a real launch is (`scripts/mainnet-rehearsal.sh`):
four validators on loopback ports, DKG over TCP, 1 s blocks, history v2, node
rewards, dev registrar — chain id 7795 (not 7780), protocol 3 at genesis.
One difference from a real launch: the genesis also funds a **faucet** so a
dev account can pay to publish releases on chain. The real mainnet has no
faucet and no dev accounts; publishing there is a paid transaction from an
operator account (design 19, `docs/design/19-release-approval.md`).

**1. Builder release approval (design 19).** A fake app bundle and DMG are
prepared (`scripts/release-approve.py prepare`), signed by three software
builder keys (python `cryptography`, the same P-256 x963 key / raw low-s
r‖s form `scripts/builder-sign` verifies), and combined:

- 2-of-3 signatures approve an ordinary release; 1-of-3 is refused
  ("need 2 to 3").
- An emergency release needs 3-of-3; 2-of-3 is refused.
- Signatures over a tampered manifest are refused ("signs a different
  manifest").
- The wallet's approval policy (`apps/wallet` `ReleaseApproval.swift`) passes
  its unit tests — the Mac-side rules agree with the builder-side script.
- Both releases are published to an on-chain `ReleaseLog`
  (`contracts/src/ReleaseLog.sol`, deployed with `aether deploy`, paid by the
  faucet-funded dev account). `aether_releaseEntries` returns the exact
  approved hashes, and `aether storage` verifies the slots **against a
  committee-certified state root** (`--identity` = the DKG group key): the
  log is not just readable, it is provably the finalized state.

**2. Scheduled committee upgrade.** The committee signs an upgrade to
protocol 4 with 3-of-4 BLS partials (`aether upgrade-sign` ×3,
`upgrade-combine`, `upgrade-verify`, drop the signed file into each node's
`upgrades/`). 2-of-4 partials do not combine. Validators 1–3 are restarted
on the new release; the upgrade rides a block and activates at exactly the
signed height: the drill polls the chain live and must observe protocol 3
below the height and 4 from it, with the parent-link intact (H−1 → H) and
blocks finalizing through the switch — plus state-root agreement across the
upgraded validators.

**3. The node left behind stops cleanly.** Validator 4 keeps running the old
release. One block before the new rules it exits by itself with code 3
(`EXIT_UPGRADE_REQUIRED`), after logging `UPGRADE REQUIRED`. Its last
finalized block is **the chain's own block** (`aether head` digest ==
`aether_getBlock(h).hash` at the same height): no fork, no conflicting
signature. Once its binary is updated it rejoins, catches up past the
switch, and agrees on the post-switch state root.

**4. Emergency upgrade (B4).** An upgrade marked `emergency: true` — which
`aether upgrade-sign` countersigns with each member's ed25519 key — needs
**committee n−f independent approvals** (B4, `upgrade::verify_emergency`):
3-of-4 activates, 2-of-4 is refused *even with a valid committee BLS
signature* (the drill strips the countersignature from one partial). It
activates after the epoch notice (60 blocks on this chain), not the
604,800-block mainnet notice, with all four validators restarted mid-flight.
The legacy 7780 chain keeps its every-member rule; this drill chain (7795)
exercises the new-genesis rule.

**5. Bad release, rollback.** A protocol-6 upgrade is signed and on chain
before anything is installed. Validator 1 then "installs" a release that
refuses to start (a stub exiting 1): it dies immediately and visibly while
the other three keep quorum. The operator rolls validator 1 back to the
previous release **before the switch height**; it rejoins and follows the
chain with no damage. Once the good build is installed everywhere, protocol
6 activates cleanly and all four validators agree on the final state root.

## The two dev-only hooks (and how they are gated)

This tree implements protocol 3; protocols 4–6 exist only in the drill. Two
hooks make that rehearable, both compiled **only** behind the cargo feature
`dev-drill` in `crates/node` (`crates/node/Cargo.toml`):

| Hook | Effect | Without the feature |
|------|--------|---------------------|
| `AETHER_DEV_PROTOCOL=<n>` | the binary *claims* protocol n (raise-only: n ≤ 3 is ignored) | env ignored, claims `PROTOCOL` (3) |
| `AETHER_DEV_UPGRADE_NOTICE=<blocks>` | shortens the 604,800-block mainnet notice (the drill uses 30) | env ignored, full notice |

Neither changes consensus rules: forks beyond protocol 3 are no-ops
(`forks::activate`), so a claimed protocol 4+ only exercises the *upgrade
machinery* (notices, stopping, rejoining) — exactly what A4 asks. The gate
is the same pattern as the existing `test-seam` feature: no other crate
enables it, no shipped build passes it, and the drill's step 0 proves both
sides at runtime — a plain binary ignores the env vars and has no `dev-b3`
subcommand, the drill binary honors them. `upgrade.rs` unit tests
(`shipped_builds_have_no_drill_overrides`,
`dev_overrides_only_accept_a_later_protocol_and_a_positive_notice`) pin the
same at test time.

Why every drill node gets the notice override (not just upgraded ones): a
build without it enforces the 604,800-block notice and would reject the
short-notice block that carries the upgrade — the "unupgraded" node in the
drill is unupgraded in its *claimed protocol* only (no
`AETHER_DEV_PROTOCOL`), not in its notice handling.

## Reading a failure

Each check prints `PASS`/`FAIL` with the evidence (heights, hashes, grep
hits); the run log keeps everything. What the important failures mean:

- **Builder approval failures** — the approval script or the wallet policy
  diverged from design 19 (signature counts, pinned keys, manifest
  canonicalization). Release publication is broken; treat as launch-blocking.
- **The upgrade never reached the chain / activated late** — committee
  signing or the notice window is wrong. On mainnet this is a stuck or
  missed upgrade; treat as launch-blocking.
- **The old node forked or exited dirty** — the stop-before-new-rules guard
  (`watch_upgrades`, `pre_state_with`) regressed. This is the check that
  prevents a split-brain; treat as launch-blocking.
- **A rollback corrupted state** — the worst class: data-dir damage from
  running an older binary. Launch-blocking.
- Chain-layer flakiness under loopback (a rejoin that needed its retry)
  is visible in the log; rerun before concluding anything.

A drill run never touches anything outside `tmp/`: not the real node folder,
not the 7780 testnet in `~/aether-testnet`, not any GUI app.
