# Voting-set security: Sybils, sampling, outages (2026-09-26)

Research for the limits left after the handoff redesign (07-consensus.md, "open
voting nodes"). Three questions: one person with many Macs and wallets; a
patient attacker climbing a deterministic ranking; outages and unreachable
players.

## 1. One person, many Macs: make it irrelevant instead of forbidden

- Apple offers no per-person signal. Sign in with Apple (supported for
  Developer ID apps with a Developer ID provisioning profile) gives one stable,
  team-scoped id per Apple Account; accounts need a phone number and are limited
  per device and year, so it raises cost but does not bind a person. Private
  Access Tokens are unlinkable (no lasting uniqueness); the rate-limited variant
  was abandoned (draft expired 2024). App Attest is not available on macOS
  (`isSupported` is false; a Developer ID profile cannot authorize the
  entitlement: the app is killed at launch, as observed).
- Per-person proofs exist outside Apple (World ID, zkPassport; ZK, per-app
  nullifier) but add a dependency and a trust anchor.
- **Decision direction (Bitcoin-like):** security from physical hardware, not
  identity. DeviceCheck gives one seat per real Mac (no VM farms). Select the
  voting set by **uniform random sampling** from all eligible Macs with an
  unbiasable seed. Then wallet addresses do not matter; only the attacker's
  share `p` of eligible Macs does, and each Mac costs real money (and, unlike
  PoW, stays resellable: weaker than slashed stake).

## 2. Sampling instead of ranking

Top-N by streak lets a patient attacker with a small share hold all seats.
Eligibility threshold, then a uniform draw, caps every Mac at one ticket.

Probability that a committee of `n` has at least `n/3` adversarial seats when
the adversary holds `p` of the eligible pool (binomial; hypergeometric N=1000
in brackets):

| p \ n | 16 | 32 | 64 | 128 |
|---|---|---|---|---|
| 0.20 | 8.2e-2 | 4.1e-2 | 5.1e-3 (3.8e-3) | 2.2e-4 (7.2e-5) |
| 0.25 | 0.19 | 0.15 | 6.0e-2 | 1.8e-2 (1.2e-2) |
| 0.30 | 0.34 | 0.36 | 0.26 | 0.21 (0.20) |

At p = 0.10, n = 128: 3.7e-13 per draw; p = 0.15: 1.3e-7. Above p ≈ 0.25 no
feasible `n` helps: the defence is a large honest pool.

Rule (implemented in `rotation.rs`):
- **Eligible:** registered, alive in the previous epoch, streak ≥ `S`.
- **Frozen pool:** the eligible set is fixed from the state the epoch's first
  block builds on, before the seed exists.
- **Seed:** the running committee's BLS threshold signature on
  `("aether-committee-seed", chain id, epoch)`, posted in a block. Unique (no
  grinding), unpredictable without 2/3 of the committee, and f faulty members
  cannot withhold it.
- **Draw:** order the pool by `H(seed ‖ key)`, take `n` = the pool / 4 rounded
  to `3f+1`, between 4 and 128. Fewer than a third of the seats change per draw.
- **Cadence:** one draw per day (resharing costs O(n²) messages).

## 3. Unreachable players

Done: `feldman_desmedt` dealers reveal the share of a player that never acks
(at most f per dealer), and the round stays valid. Our agreement step waited
for every player; after the dealing window it now needs a quorum of players
announcing the same identity (`dkg.rs`, test
`reshare_completes_without_an_unreachable_new_player`). The unreachable player
keeps a seat whose share is public: it counts as one of the f faults until the
next draw replaces it.

Later: Commonware 2026.9.0 also ships `dkg::golden` (ePrint 2025/1924), a
non-interactive DKG whose dealings can go in blocks and whose players decrypt
their shares from the chain later (no revealed shares). It produces a
`Sharing<MinPk>`; our consensus uses MinSig, so adopting it means moving the
committee key to MinPk (a new identity format for wallets): a genesis change.

Eligibility also requires 95% uptime during the streak: the registry counts
epochs missed within the grace period (`missed * 20 <= streak`).

## 4. Mass outages (≥ 1/3 of the set offline at once)

No handoff without 2/3 of the old committee is safe in the Byzantine model
(a recovery quorum cannot be guaranteed to overlap every old quorum). Options:
- Keep safety absolute (chosen): eligibility requires measured uptime, the set
  grows with the pool, and diversity lowers correlated sleep.
- Research track: an available-but-not-final ledger while finality waits
  (ebb-and-flow; Neu–Tas–Tse), finalized when 2f+1 return.
- Rejected unless the owner decides otherwise: a last-resort recovery handoff by
  a larger set, safe only if absent members were asleep and not voting.

## Sources

Apple: developer.apple.com (supported-capabilities-macos, applesignin
entitlement, TN3125, verifying-a-user, DCAppAttestService isSupported,
assessing-fraud-risk), WWDC22 10077; IETF draft-ietf-privacypass-rate-limit-tokens,
RFC 9577; docs.world.org; zkpassport.id. Consensus: docs.rs commonware-consensus
simplex (seed properties), commonware-cryptography 2026.9.0 `dkg::golden` and
`feldman_desmedt` (source), ePrint 2025/1924, 2021/339 (Groth21), 2021/005;
arXiv 2009.04987, 2209.03255, 2302.11326; eth2book (inactivity leak).
