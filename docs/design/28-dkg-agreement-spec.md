# DKG / reshare agreement specification (as implemented at `consensus-freeze-6`)

Five audit rounds (A1, R2-1, A3-1, A3-2, A4-1, A4-2, A5-2) each found a
Critical/High defect in this layer. This document specifies the protocol **as
the code implements it** — not as we wish it were — states the adversary model
and the numbered invariants, and maps every past finding to the invariant it
violated. The scenario matrix that checks these invariants is
`crates/node/tests/dkg_matrix.rs`.

Sources: `crates/node/src/dkg.rs`, `dkg_agreement.rs`, `handoff.rs`,
`supervisor.rs`, and the drivers in `crates/node/src/main.rs` (reshare staging
child ≈ lines 1380–1520, genesis driver ≈ 2660–2785).

## 1. The protocol as implemented

### 1.1 Participants, rounds, attempts

- A ceremony has **dealers** (holders of the previous output's shares; for
  genesis every player is a dealer with no share) and **players** (the proposed
  new committee). Joint-Feldman DKG (`commonware feldman_desmedt`, MinSig,
  `Mode::NonZeroCounter`, `Reveal::V1`, threshold `n = 3f+1`): every dealer is
  also a player.
- A `Round` (`dkg.rs:108`) is `dkg(participants, round)` or
  `reshare(previous_output, players, round)`, optionally `.with_chain_id(id)`
  (strict, new-genesis) or `.legacy_agreement()` (chain 7780 only).
- **Chain binding.** `namespace()` (`dkg.rs:146`) is
  `_AETHER_CHAIN_V1_DKG_<chain_id be-bytes>` for strict rounds and the frozen
  `_AETHER_DEVNET_V1_DKG` for 7780. `context_digest()` (`dkg.rs:158`) hashes
  namespace, chain binding, round number, the previous output's bytes, dealers,
  and players. Every signature in the ceremony and the agreement layer is made
  over material that includes this context, so a message from another chain,
  another round, another roster, or an earlier attempt of the same round never
  verifies.
- **Attempts from chain height.** The supervisor derives the DKG round from the
  finalized rotation proposal, not the sampled head: `reshare_round(old,
  attempt) = old + attempt + 1` (`supervisor.rs:160`), where `attempt` is the
  proposal's draw (×2 for strict, `reshare_proposal_attempt`). The attempt
  window is `reshare_attempt_blocks` (`supervisor.rs:146`):
  `2 × default_reshare_timeout(players_bound) + 60 s` in ≥1 s blocks. All
  staggered supervisors that see the same finalized proposal therefore run the
  same signed round.
- A fresh runtime directory and per-round journal names are used per attempt
  (`main.rs` reshare child); a retried round never reads an older attempt's
  Commonware state.

### Background transport

The supervisor keeps consensus and reshare on separate Commonware listeners:
`aether run` defaults the reshare listener to `--port + 10000`, or accepts an
explicit `--reshare-port` (required if the default exceeds 65535). The port
must be nonzero and distinct from the local consensus and RPC ports. A
plus-one default would collide with the next validator when several local
validators use consecutive consensus ports, as the 7780 launch scripts do.

The supervisor passes the resolved port to both the validator child and the
staged reshare child. The validator's public iroh endpoint forwards
`aether/reshare/1` to that configured loopback listener; consensus continues
on `aether/p2p/1`. Reshare still uses the `_DKG` namespace and the union of
old and new rosters. A validator's reshare child dials from an unpublished
endpoint (`--via-node`); a candidate serves reshare under its own node id.
TCP devnets discover the separate listeners through `--dev-peer-dir`.
These transport settings do not change DKG messages, committee rounds,
legacy 7780 agreement, history replay or the proving program.

### 1.2 Messages (`Msg`, `dkg.rs:68`)

| Message | Carries | Addressing |
|---|---|---|
| `Deal { commitment, dealing }` | dealer → one player's private dealing | `To::One` only |
| `Ack(dealer)` | player confirms a persisted dealing | `To::One` |
| `Log { dealer, log }` | dealer's signed log (commitments + reveals for players that never acked) | `To::All` |
| `Done(digest)` | legacy (7780) identity announcement; strict: recorded, never fatal | `To::All` |
| `Transcript(entries)` | the exact signed dealer/log bundle (bounded per sender at `2×dealers`, min 4) | `To::All` |
| `Agreement(AgreementMsg)` | NewView / Proposal / Vote / PrecommitCertificate / Decision | `To::All` |
| `Ready { player, proof }` | post-stage readiness proof, consumed by the relay | `To::All` |

Agreement messages (`dkg_agreement.rs:85`): `NewView` (view, highest
precommit QC), `Proposal` (view, digest, quorum of NewViews), `Vote`
(SignedVote: view, phase, digest, signer, signature), `PrecommitCertificate`,
`Decision`. A `Phase` is `Precommit` then `Commit`. Certificates
(`dkg_agreement.rs:59`) require **exactly** `2f+1` votes, all same view/phase/
digest, all distinct verified signers.

### 1.3 Phases and timers

Tick interval 500 ms (`TICK_INTERVAL`). Defaults `Timeouts { dealing: 30 s,
total: 300 s }`.

| Timer | Value | Meaning |
|---|---|---|
| dealing window | 30 s | deals re-sent via `pending_deals()` until all players ack or the window closes |
| `QUORUM_LOG_DELAY` | 10 s | after dealing closes, wait this long before accepting a quorum (vs all) of logs |
| `LOG_SETTLE_TIME` | 2 s | logs must be stable this long before proposing |
| view timers | `BASE_VIEW_TICKS(8) × 2^min(view, MAX_VIEW_BACKOFF(4))` ticks | view 0 = 4 s, view ≥ 4 = 64 s; NewView re-sent, everything rebroadcast every `REBROADCAST_TICKS(4)` ticks; votes with `view > current + MAX_FUTURE_VIEWS(32)` are dropped |
| `STRICT_DECISION_GRACE` | 5 s | after a decision, wait for stragglers' Done before returning |
| `CERTIFIED_TRANSCRIPT_RELAY` | 30 s | in-process relay of the decision before return |
| `POST_STAGE_RELAY` | 60 s | post-stage relay child serves Decision + Transcript |
| `SHARE_READY_WINDOW` | 120 s | readiness collection window for the relay |

**Agreement** is a two-phase Tendermint-style one-shot instance over the
transcript digest with locks and transferable quorum certificates: proposer is
`players[view % n]`; precommit is gated by the lock
(`lock.digest == proposal.digest || proof_view > lock.view`); the first Commit
quorum decides and broadcasts `Decision`. `Agreement::new_with_context`
(`dkg_agreement.rs:169`) binds the roster hash and DKG context digest, so
certificates from an earlier attempt of the same round (different dealers or
previous output) do not verify. A sender's **first** authenticated vote per
`(view, phase)` is the only one stored — equivocation cannot inflate either
certificate. Non-players may only relay `Decision`.

**Transcript digest.** `transcript_digest()` (`dkg.rs:665`) = blake3 over
`"aether DKG signed transcript agreement v2"`, round, output encoding, and the
ordered dealer/log bytes. Agreement is on this digest — not the (constant,
reshare-preserving) committee identity.

**Bundle validity.** `checked_logs()` (`dkg.rs:645`) accepts a log set only
when every dealer is in the roster, no dealer appears twice, every signature
verifies, the dealer count meets quorum, `observe()` yields an output, the
expected identity matches (reshare), and — the A4-1 gate —
`output.revealed()` contains **no seated player** in strict mode.

**Finalization.** Strict mode has exactly one path: `finish_decided(rng,
digest)` (`dkg.rs:744`) revalidates `revealed()` a second time, replays the
deal journal's accepted dealings into a fresh player, finalizes against the
exact certified logs, persists `DecidedTranscript`, and sets the staged
identity/output digest. `finish()` (local logs only) exists for 7780 only;
strict calls to it error.

### 1.4 Certification, staging, return

`run_inner` (`dkg.rs:1089`) selects on receiver + 500 ms tick: close dealing
when all acked or window elapsed; `enough = have_all_logs() || (elapsed >
dealing + QUORUM_LOG_DELAY && have_quorum_logs())`; propose when a player with
stable logs; on a certified transcript persist + `finish_decided`; **return
`Ok` once `STRICT_DECISION_GRACE` elapses after the decision** — the process
does not wait for every player's Done. If no decision by `strict_deadline
(players) = max(total, dealing + 64 s × players + CERTIFIED_TRANSCRIPT_RELAY)`
the child returns a timeout error. `strict_return_bound` (`dkg.rs:899`) adds
the grace and two ticks:

| players | strict deadline | return bound | supervisor timeout |
|---|---:|---:|---:|
| 4 | 316 s | 322 s | 457 s |
| 7 | 508 s | 514 s | 649 s |
| 8 (window bound) | 572 s | 578 s | 713 s |

`default_reshare_timeout` (`supervisor.rs:121`) = `strict_return_bound +
SHARE_READY_WINDOW + 15 s` margin, and `effective_reshare_timeout` takes the
max with the configured value — an operator cannot configure a supervisor
deadline that kills a healthy strict child (A4-2).

The reshare child then writes the staged `threshold-next.json` (0600) and
`network-next.json` **before** the relay starts (`main.rs:1451-1473`), and a
genesis child writes `threshold.json` + `network.json` after checking
`KeyFile::reveals_seated_share`. Both re-check the revealed-share gate and
fail (exit / "retry with a higher --round") rather than stage a disclosed
output.

### 1.5 Post-stage relay and readiness proofs

`run_relay_with_journal` (`dkg.rs:971`) reopens both journals, restores the
available bundle set, and serves `Decision` + `Transcript` rebroadcast for
`POST_STAGE_RELAY` (60 s), or `SHARE_READY_WINDOW` (120 s) when a
`ReadinessPlan` is present. It collects `Msg::Ready` proofs into a
`handoff::Readiness` record and writes `reshare-ready.json` (`READY_FILE`)
when complete, returning early. A **departing dealer** runs the same relay and
verifies readiness without holding a new share.

A readiness proof (`handoff.rs:83`) is a threshold partial signature over
`READY_NAMESPACE` on the message `(chain_id, round, blake3(output),
blake3(members), "share-ready")`; `check_ready` requires the partial's index
to match the member's seat in the ordered player set — a proof from another
seat, another round, another output, or another chain is rejected.

### 1.6 Handoff and install

The old committee threshold-signs a `Handoff` (NAMESPACE partials over
`chain_id, round, output, members(with node ids)`) — `sign_staged` is only
called once the child exited successfully and `READY_FILE` exists
(`supervisor.rs` watch loop). `verify_output` (`handoff.rs:197`) on
new-genesis chains requires: n ≥ 4; no revealed seated player;
`players == members`; `ready.len() == members.len()` with every proof passing
`check_ready` — a seat without a usable-share proof cannot be certified
(A5-2). `roster_allowed` (non-7780): members ⊆ proposed, ≤ 1/3 removed, ≥ 4,
same node ids. Installation switches at `DELAY = 64` blocks after
finalization, and only after `check_joining_share`: the staged KeyFile's
round/output match the handoff, the share's index maps to this validator,
`partial_public(index) == share.public()`, and no revealed seated share.
Generation directories `gen/<round>/.installed` make activation
idempotent across restarts.

If readiness does not complete, `Readiness::retry_members` (`handoff.rs:61`)
proposes the ready subset when it has ≥ 4 members and removes ≤ 1/3 of the
proposed seats, and the supervisor reruns a **fresh adjacent round** with that
reduced roster (fresh dealer randomness — a published reveal is never reused).

### 1.7 Supervisor lifecycle

`aether run` spawns the reshare child once per attempt; on failure it removes
`STAGED_THRESHOLD`, `STAGED_NETWORK`, and `READY_FILE`; on success +
strict + not-yet-retried it may run the reduced-roster retry above. Restart
backoff 1 s → 60 s doubling; more than 3 exits in 10 minutes stops the
supervisor with `EXIT_UPGRADE_REQUIRED(3)` / `EXIT_NO_VERIFIER(5)` /
`EXIT_LOCKED(7)`. An untrusted vote journal writes a no-vote marker file and
keeps the node from signing. `finish_incomplete` resumes a partially installed
generation idempotently; a leaving member drops its `threshold.json`.

## 2. Adversary model

- **n players, f Byzantine, n ≥ 3f+1** (4 → f=1, 7 → f=2). The adversary
  controls f member keys and can sign **valid** messages with them.
- Any number of players may be **silent** (never send), **late** (deliver just
  before or after any timer in §1.3), or **restarting** at any point (lose all
  in-memory state; durable journals survive).
- The network delays and reorders messages up to the timers, drops messages,
  and **replays recorded messages across attempts, rounds, and chain ids**.
- Byzantine players may **equivocate** (different valid messages to different
  peers, including two different signed logs and two different votes in the
  same view/phase), send **arbitrary or malformed `Done`/votes/certificates**,
  publish dealer logs that **reveal** other players' shares, and **withhold
  readiness proofs**.
- The adversary **cannot**: break Ed25519/BLS, read a `To::One` private
  dealing of an honest player, forge a quorum certificate (needs 2f+1 distinct
  signers), or control more than f identities.

## 3. Invariants

### Safety

- **S1 — one certified output per round.** For any (chain_id, round), any two
  certificates honest players accept commit the same transcript digest, and
  every honest player that stages stages the bytes of that certified digest.
  *(Two-phase QC quorum intersection; agreement on `transcript_digest`, not
  identity; `finish_decided` only on the certified bundle.)*
- **S2 — no revealed seated share.** No output that any honest player votes
  for, certifies, stages, or hands off has a seated player in
  `output.revealed()`. *(checked_logs gate, finish_decided re-check,
  `KeyFile::reveals_seated_share` at every process gate,
  `handoff::verify_output`, `check_joining_share`.)*
- **S3 — usable shares.** Every honest seated player that stages holds a
  share that signs under the staged output's polynomial, and a committee is
  only handed off when every seat has proved a usable share. *(deal journal
  replay in finish_decided; per-seat readiness proofs.)*
- **S4 — no share material in broadcast.** Private dealings travel only
  `To::One`; the only share material ever published is a reveal of a player
  that did not ack — and such a bundle is rejected by S2.
- **S5 — no cross-context acceptance.** A message signed for another chain
  id, another round, another roster, or an earlier attempt of the same round
  never changes ceremony or agreement state. *(namespace + context_digest +
  roster hash in every signature.)*
- **S6 — equivocation cannot inflate.** A player's first authenticated vote
  per (view, phase) is the only one counted; certificates need exactly 2f+1
  distinct verified signers on one digest; a dealer with two different valid
  logs is detected, both versions relayed, and it cannot widen any bundle.
- **S7 — handoff binding.** The old committee's threshold signature covers
  (chain_id, round, output, members + node ids, readiness proofs);
  installation additionally requires `roster_allowed` and
  `check_joining_share`. A seat with a revealed share or without a matching
  readiness proof is never installed.

### Liveness

- **L1 — attempt progress.** With at most f silent or Byzantine players and
  honest players eventually connected, every honest ceremony either returns a
  staged share by `strict_return_bound` or fails cleanly by that bound
  without staging; the supervisor then starts the next attempt in the window.
  An arbitrary, absent, or malformed `Done` never changes the outcome.
- **L2 — restart never sticks.** An honest player that restarts mid-ceremony
  (same key, same round, durable journals) re-derives the same dealer
  randomness, never signs a second vote in a (view, phase) it already signed,
  catches up from peers' rebroadcast/relay, and neither blocks the quorum nor
  stays stuck. A Byzantine peer's certified claims cannot keep it out
  (verified-claim catch-up gate + quarantine).
- **L3 — late recovery.** An honest player that durably accepted its dealings
  but missed the decision recovers the exact certified bundle and certificate
  from any one honest relayer within the bounded relay window and stages the
  same output.
- **L4 — readiness-gated handoff.** If a proposed seat cannot prove a usable
  share within `SHARE_READY_WINDOW`, no handoff names it: the supervisor
  retries with a reduced roster (≥ 4 members, ≤ 1/3 removed, fresh adjacent
  round) or keeps the current committee signing.

## 4. Past findings → violated invariants

| Finding | What happened | Invariant violated then | Enforced now by |
|---|---|---|---|
| **A1** — one Byzantine roster member stops the committee after restart | a restarting validator could not safely rejoin (certified-claim gate) | **L2** | verified-certificate catch-up gate + quarantine (`follow.rs`), journal-bound restart, restart backoff in the supervisor |
| **R2-1** — agree on the identity with incompatible keys | quorums of different dealer-log subsets both announced the same constant identity and staged different polynomials | **S1** | agreement on `transcript_digest` with context binding; bare `Done` cannot stage |
| **A3-1** — stage before the fourth player reveals a different output | players finalized local log subsets and staged different outputs | **S1, S3** | `finish_decided` finalizes only the certified bundle, rechecking `revealed()`; exact signed-log replay |
| **A3-2** — arbitrary `Done` aborts every attempt | one Byzantine 32-byte `Done` failed the ceremony for everyone | **L1** | strict `Done` is recorded but never fatal (32-byte shape gate); decision-grace return; timeout is the only failure |
| **A4-1** — certified output with a publicly reconstructible share | a 3-of-4 bundle whose logs revealed the fourth player's share was certified and staged | **S2, S4** | `revealed()` gates at proposal, finalization, staging, genesis/handoff/joining checks |
| **A4-2** — one silent new player prevents staging | all-players-finished return (316 s) exceeded the supervisor kill (300 s) | **L1** | 5 s decision grace; supervisor timeout derived from `strict_return_bound + SHARE_READY_WINDOW + 15 s` (457 s @ 4 players) |
| **A5-2** — finite relay certifies a committee with an unusable seat | a seat that missed the bounded relay was counted in a signed handoff | **L4, S3** | per-seat readiness partials, `verify_output` requires all members proved, reduced-roster retry with a fresh round |

## 5. Scenario matrix

`crates/node/tests/dkg_matrix.rs` runs the in-memory ceremony harness (the
same `Ceremony` API the production child drives) over the cross product:

- **n ∈ {4, 7}** (f = 1, f = 2), Byzantine count 1 or f, several seeds;
- **behaviours**: silent from start; silent after publishing a log; ack late
  just before / past the dealing window; log late after others proposed;
  decision missed past the grace (recovered from one bounded relay); honest
  restart during dealing and during agreement; equivocating logs; equivocating
  votes (same key, two ceremonies, two digests, same view); forged/arbitrary
  `Done` and malformed agreement messages; revealed-share bundle; withheld
  readiness proof; replay of a prior round's log/decision; a valid message for
  another chain id;
- **assertions after every run**: S1 (one certified digest, byte-identical
  staged outputs), S2 (no revealed seated share in any staged output), S3
  (every staged share signs under the output; shares distinct), S5 (replayed /
  cross-chain messages leave state untouched), plus the per-case liveness
  claim (clean failure + fresh-round recovery when the attempt must fail).

The durable-journal layer itself (fsync-before-send, torn-record truncation,
seed reuse, no-double-vote restore) is covered by the unit tests inside
`dkg.rs` / `dkg_agreement.rs`; the matrix exercises the ceremony state machine
above them.
