# B5 stuck-transaction red-team review — 2026-10-07

**Verdict: request changes.** Findings: **0 Critical, 2 High, 3 Medium,
1 Low**. No consensus-rule change or double-spend through the same-nonce wallet
resend was found. Fee confirmation and recovery are not yet reliable on all
supported paths.

Scope: `git diff fa78208 fb4527b`, against
`.claude/team/b5-stuck-tx.md` and the **Bug #5 / Stress** sections of
`docs/research/contracts-live-2026-10-06.md`. All source locations below refer
to **fb4527b**. Review was read-only apart from this report: no source edits,
compilation, tests, app launches, or live-network requests. Findings are based
on source tracing and snapshot comparisons, with independent node and wallet
reviews. The reported stress results were inspected, not rerun.

## Findings

### 1. High — the displayed maximum still does not bound the signed fee

**Locations:** `crates/ffi/src/lib.rs:822`, `crates/ffi/src/lib.rs:825`,
`crates/ffi/src/lib.rs:767`, `crates/ffi/src/lib.rs:1735`,
`crates/ffi/src/lib.rs:1758`.

The changed quote correctly prices state units at the doubled state cap, but
still prices execution at `base + 1 gwei`. The envelope signs an execution cap
of `2 × base + 1 gwei`. The shown-fee check compares against the same incomplete
quote, so it cannot detect this difference. Prove-fee exposure is absent from
the quote. The state quantities also differ: a positive plain transfer signs
216 units (`crates/execution/src/tx.rs:56`), while the quote prices 232 units
for an unproven recipient or 132 for a proven existing recipient.

**Concrete failure:** At a paid-state snapshot with execution base
`10^12 wei`, state price `10^12 wei`, prove base zero, and an unproven ordinary
recipient, a 21,000-gas send displays:

`21,000 × (10^12 + 10^9) + 232 × 2 × 10^12 = 21,485 × 10^12 wei`.

Its signed budget/cap maximum is instead
`21,000 × (2 × 10^12 + 10^9) + 216 × 2 × 10^12 = 42,453 × 10^12 wei`.
Preparation passes the shown-fee check. If the execution base rises just 3%
before inclusion, execution alone charges `21,651 × 10^12 wei`, exceeding
the entire displayed maximum while remaining below the signed cap. The state
charge is additional. The normal resend sheet uses this same quote and check.

**Remedy:** Derive the displayed maximum and its confirmation check from the
actual envelope's gas budgets, fee caps, and tip. Account for all charged
dimensions. Verify the displayed value against a prepared envelope, including
nonzero execution/prove prices and an inclusion-price rise.

**Delta qualification:** The execution underquote and separate budget formulas
predate this diff. B5 changes this function and explicitly claims M1 compliance;
its requested shown-maximum gate remains unsatisfied. This is not a newly
introduced consensus pricing defect.

### 2. High — normal follower reads cannot see the accepting validator's drop

**Locations:** `crates/ffi/src/tx_status.rs:43`,
`apps/wallet/Sources/WalletModel.swift:1023`,
`apps/wallet/Sources/WalletModel.swift:1288`.
Supporting route: `crates/ffi/src/lib.rs:941`,
`crates/ffi/src/lib.rs:968`, `crates/ffi/src/lib.rs:640`,
`crates/node/src/main.rs:2877`, `crates/node/src/rpc.rs:1165`.

With no local node, submissions go to a validator, but `aether_getReceipt` is
classified as an ordinary read and goes to follower Macs first. A healthy
follower's `null` is returned immediately, without validator fallback.
Followers replay finalized blocks and forward their own submissions; they do
not receive the validators' pending-transaction gossip. The receipt handler
consults only that node's receipts, mempool, and tombstones.

**Concrete failure:** A remote wallet submits to validator A. A burst makes
its transfer unincludable, while a healthy follower F keeps answering `null`
because F never admitted it. After approximately 60 seconds the wallet marks
the row failed/unknown and clears its resend intent. A records the B5 price
tombstone at its 10-minute TTL, but the wallet has stopped tracking and F still
has no record. The promised reason and same-nonce recovery button never appear.
The agent's receipt/status tools have the same route. Queue selection also
uses this read: a genuinely pending hash can appear nonpending and cause nonce
reuse or spurious refusals.

**Remedy:** Preserve the admitting endpoint for pending-status queries, or
perform bounded validator fallback when a follower cannot resolve a submitted
hash. Preserve unresolved transaction context until inclusion or nonce
reconciliation; a follower's ignorance must not become a terminal failure.
Keep the public read gateway's existing prohibition on upstream forwarding.

**Delta qualification:** Follower routing is pre-existing; the new B5 status
and queue paths rely on it without accounting for node-local mempool history.
The documented CLI stress run queries the submission node and does not cover
this normal remote-wallet path.

### 3. Medium — the newest pending hash does not prove an unbroken nonce queue

**Locations:** `crates/ffi/src/tx_status.rs:171`,
`crates/ffi/src/tx_status.rs:193`, `crates/ffi/src/tx_status.rs:198`,
`crates/ffi/src/tx_status.rs:210`.

`QUEUED` remembers only the sender's newest `(nonce, hash)`. Any `pending`
answer for that hash makes `next_nonce` return its nonce plus one; earlier
pending transactions and the reason for waiting are not checked.

**Concrete failure:** On a consistent single node, submit N, then N+1 later.
Both wait under the rising state price. N reaches its TTL first, while the
younger N+1 remains pending behind missing N. The FFI sees N+1 as pending,
prepares N+2, and permits its submission. With a fresh sufficient cap and
balance, admission accepts N+2 by simulating its future nonce
(`crates/execution/src/block.rs:310`). It now waits behind the gap the guard
claims to prevent. Individual expiry at `crates/node/src/chain.rs:2769` makes
this possible without concurrent FFI submissions or inconsistent RPC answers.

**Remedy:** Track/check a contiguous sequence beginning at the chain nonce,
with bounded per-sender state. Checking only `waiting.nonce_gap` is insufficient:
`pending_reason` prioritizes the state-price reason over a simultaneous gap.
Cover an older predecessor dropping while its younger successor stays pending.

An immediate refusal of N in an otherwise contiguous queue does not advance
the cursor and does block N+1. The defect concerns an earlier queued nonce
subsequently dropping; serialization alone does not establish continuity.

### 4. Medium — pending receipt reads amplify work under the chain mutex

**Locations:** `crates/node/src/chain.rs:3219`,
`crates/node/src/rpc.rs:1167`.

For a pending transaction whose state cap meets the current price,
`pending_reason` scans the entire mempool to collect one sender's nonces.
The receipt handler holds the shared chain mutex throughout the scan.
`MAX_MEMPOOL` is 50,000, although one sender is limited to 64 transactions
(`crates/node/src/chain.rs:25`). Before B5 this branch performed a hash lookup.

**Concrete failure:** During heavy traffic, repeated reads of one known,
fee-eligible pending hash force up to 50,000 sender comparisons per request,
contending with admission, proposal work, and finalization. A future-nonce
transaction can stay pending behind a gap while meeting the current fee caps.
On a public-mode node with such pending state, an allowed eight-call batch
can force up to 400,000 comparisons synchronously. The public gateway allows
this method (`crates/node/src/rpc.rs:70`); its batch limit does not bound
receipt work or contention. An ordinary follower that never saw the hash
takes the cheap unknown path, so this is conditional on an actual pending hit.

**Remedy:** Maintain a bounded sender-to-pending-nonces index or cached gap
information, keeping lookup work proportional to at most that sender's 64
entries. Move avoidable computation outside the shared mutex and meter receipt
execution where needed. This is a static availability regression; no request
rate, latency, or block-stall threshold was measured.

### 5. Medium — a node-local drop permanently finalizes failed agent history

**Locations:** `apps/agent/Sources/Tools.swift:349`,
`apps/agent/Sources/Tools.swift:379`.
Supporting state transition: `apps/agent/Sources/History.swift:49`.

A dropped answer calls `History.finalize(hash:success:false)` and tells the
caller that nothing was paid. That removes the pending context. Later
finalization can update history only if the hash still has a pending entry.
A tombstone is evidence that one node removed a transaction, not proof that
the transaction can never be included elsewhere.

**Concrete failure:** A expires an original transaction while B still holds
it, for example after later propagation/arrival. The agent reads A's drop,
records failed history, and removes the pending entry. B later includes the
original successfully. Even an explicit later receipt read that reports
success cannot repair `aether_history`: `History.finalize` finds no pending
entry and returns. The user has paid, while agent history remains failed.

The wallet also terminates `track` on a local drop
(`apps/wallet/Sources/WalletModel.swift:1006`), and a payment-link callback can
already have received `status=failed` (`WalletModel.swift:885`). Its chain
history refresh can repair the activity row (`WalletModel.swift:1201`), so
permanently wrong wallet history is not claimed. A same-nonce resend still
cannot make both transactions spend.

**Remedy:** Represent local drops separately from finalized on-chain failure;
retain reconciliation context and allow a later receipt to supersede a drop.
Qualify the message and callback instead of asserting cancellation or permanent
nonpayment from a node-local observation.

### 6. Low — the CLI's zero-price exception is not tied to a stateless chain

**Locations:** `crates/node/src/main.rs:3450`,
`crates/node/src/main.rs:3453`, `crates/execution/src/fees.rs:124`.

The CLI parses missing/malformed state prices as zero, and the shared cap helper
returns zero for a zero input. Unlike the app's `state_price_for`
(`crates/ffi/src/lib.rs:731`), it does not establish that this is the known
legacy stateless chain before taking the exception.

**Concrete failure:** On a paid-state chain, a stale or faulty RPC reports
state price `"0"`, or omits the field. The CLI signs `max_fee.state = 0` and
skips setting its state budget (`crates/node/src/main.rs:3480`). The required
paid-state floor/headroom is absent, and an honest paid-state node refuses the
transfer. The app correctly clamps a parsed paid-chain zero to the floor and
refuses an absent/malformed price. No on-chain overspend is implied.

**Remedy:** Permit zero only for an established stateless chain; validate a
paid-chain report and apply `2 × max(price, floor)` to parsed prices.
The permissive CLI parser predates B5; it remains a gap in the requested
CLI/app headroom parity. Keeping a zero cap on the actual legacy chain is
intentional compatibility, not a finding.

## Results against the six requested checks

| Check | Source-review result |
|---|---|
| 1. No consensus change | **Pass.** Existing validity, pricing, state-root, and builder inclusion code is unchanged; retention still uses the same predicates. Added work has the availability concern in finding 4. |
| 2. Tombstone bound, LRU, provenance, DoS | **Bound/LRU/provenance pass.** No remote hash/reason injection or unbounded tombstone growth found. Pending status calculation has the separate CPU/lock issue in finding 4. |
| 3. Receipt compatibility and public gateway | **Fields pass.** Included `{height, receipt}`, pending `pending:true`, and genuinely unknown `null` remain. Dropped answers contain no fabricated receipt. No sensitive gateway disclosure found. Status visibility across nodes fails in finding 2. |
| 4. Same-nonce resend, confirmation, silent signing | **Nonce/no-silent-signing pass; M1 fails.** The explicit resend preserves nonce, recipient, and exact wei. Normal user action and an enclave signature remain required. Findings 1 and 5 concern confirmation/outcome truthfulness. |
| 5. Submit serialization, deadlock, gaps | **Partial.** No new lock cycle found; direct refusal is handled. The mutex serializes all senders in one process, rather than independently per sender. Later predecessor drops defeat the gap check (finding 3). |
| 6. CLI/app cap and shown maximum | **Valid paid prices pass the shared cap formula, with u128 saturation.** App/CLI call the same helper; actual legacy zero pricing stays zero. Shown maximum fails finding 1, and CLI paid-price validation fails finding 6. |

## Evidence and limits

- Snapshot comparison found identical function text for 22 chain functions,
  including `execute_as`, `block_context`, `next_base_fee`, `mempool_candidates`,
  inclusion checks, `affordable`, `admissible`, `build_payload`, and archive
  inclusion limits. Six existing fee functions are also identical:
  `state_block_limit`, `next_state_excess`, `state_base_fee`,
  `fake_exponential`, `next_excess`, and `base_fee`.
- The two revisions have identical Git blobs for
  `crates/execution/src/block.rs`, `crates/execution/src/tx.rs`,
  `crates/light/src/block.rs`, `crates/node/src/application.rs`,
  `crates/node/src/inclusion.rs`, and `crates/node/src/store.rs`.
  Execution, nonce validation, scheduler invalidation, and state commitments
  were traced in source; compiled-binary equivalence was not tested.
- Comparing the original `keep_in_pool` with `drop_reason(...).is_none()`
  preserves removal for consumed nonce, TTL, exec/prove fee-wait expiry, and
  affordability, in the same order. The gap argument changes only the reason.
- Tombstones store at most 4,096 entries after each operation. Each touch
  removes the old order entry; recording evicts the least recently used one;
  successful lookups refresh order, and unknown lookups insert nothing
  (`crates/node/src/tombstone.rs:63`, `:73`, `:82`, `:89`). Production string
  fields are decimal u128 values, at most 39 characters each. Neither index
  accumulates stale order rows during normal operation.
- Production reasons are written only for actual local eviction/finalization
  removals (`crates/node/src/chain.rs:2774`, `:3102`). RPC and gossip validate
  signatures, sender, chain, and payload before admission
  (`crates/node/src/rpc.rs:1111`, `crates/node/src/main.rs:2446`). The hash is
  computed from the canonical envelope (`crates/execution/src/tx.rs:130`).
  There is no network method accepting arbitrary tombstone hashes or reasons.
- Receipt responses expose hash-scoped kind, cap/price, expected nonce, refill
  estimate, and resendability. They expose no envelope, payload, signature,
  key, filesystem path, or recipient. The gateway gate and write/upstream
  restrictions remain unchanged. Inspected FFI, CLI, extension, and explorer
  readers distinguish receipt presence from pending/no receipt.
- `prepare_transfer_at` keeps the explicit original nonce
  (`crates/ffi/src/lib.rs:1738`); the wallet checks recipient and exact wei
  (`apps/wallet/Sources/WalletModel.swift:859`). If the original lands after
  replacement signing, the second envelope has an already consumed nonce.
  Execution uses the envelope nonce (`crates/execution/src/block.rs:339`),
  and the unchanged scheduler invalidates speculative sender-nonce reads
  (`crates/execution/src/parallel.rs:94`). Both cannot execute successfully on
  the same canonical chain, whether proposed together or in different blocks.
- Submit acquires `SUBMIT` and releases the `QUEUED` guard before network
  reads; no reverse acquisition or callback reacquiring `SUBMIT` was found.
  Network calls occur under the process-wide serialization lock, so unrelated
  senders share any wait. Runtime deadlock/latency testing was not performed.
- The documented rerun covers 148 dropped transfers, far below the tombstone
  cap. A wave exceeding 4,096 drops, churn of valid transactions, or a node
  restart can still erase reasons before polling. This follows the requested
  bounded, nonpersistent design and limits any claim of universal zero silent
  losses; it is not unbounded memory growth or spoofed-reason injection.
- Added tests were inspected, not executed. Pure quote and nonce-helper tests
  do not exercise the follower route, predecessor-expiry sequence, full signed
  maximum, or late inclusion after a local drop. Those scenarios need coverage
  when implementation work is authorized.

Only this report was added. No implementation simplifications or fixes were
made; the findings remain open.

## Round 2 resolution (branch `claude/b5-stuck-tx-2`)

All six findings were fixed as `.claude/team/b5-stuck-tx-2.md` specifies.
No consensus rule changed: block validity, `fees.rs` pricing, the state root
and the builder's inclusion rule are untouched. The new helpers in
`crates/execution/src/tx.rs` (`signed_fee_maximum`, `wallet_state_price`)
are wallet policy that no node calls. Each new test was first run against
the pre-fix code and failed; the commit messages quote those failures.

1. **High, the shown maximum is the signed maximum: fixed.** The send sheet's
   quote and the prepared envelope now come from one function
   (`crates/ffi` `draft_fees`). Exec gas, state budget and prove budget, each
   with its cap, and the tip are built there once. The quote is that
   envelope's `signed_fee_maximum`: exec cap × gas, plus state cap × state
   budget, plus prove cap × prove budget. The M1 check in `prepare_at`
   compares what was shown with the maximum of the envelope it is about to
   return, never a separate formula. A plain transfer, the resend sheet
   (`prepare_transfer_at`) and a batch all take this path. Batches now get
   their own quote (`batch_quote`), and the sheet shows it.
   Test: `the_shown_maximum_is_the_signed_envelope_maximum`, with nonzero exec
   and prove prices, a 3 % exec rise, a refused larger rise and a batch.
   Before the fix it showed 21,485 × 10^12 wei against a signed maximum of
   42,495 × 10^12. The CLI does not display a maximum, so the CLI is out of
   scope for this finding.
2. **High, pending status follows the node that admitted it: fixed.**
   `submit_signed` records which validator accepted each transaction
   (`RpcClient::call_tracked`). `aether_getReceipt` reads (`receipt_answer`,
   used by `tx_status`, `receipt` and the nonce queue) go to that validator
   first, by id only (`RpcClient::call_node`). After it come the ordinary
   follower-first read, then a validator if a follower returned null. All of
   this runs under the one read deadline. A follower's null becomes
   `unknown`, which is never final. Each send's context (sender, nonce,
   admitter) is kept in a bounded book (1,024 sends, 16 senders × 64 nonces)
   until a receipt arrives or the chain nonce passes that nonce. The agent
   tools use the same FFI route. The public read gateway is unchanged and
   still never forwards upstream. Test: `crates/ffi/tests/wallet_pending_route.rs`
   uses a real QUIC follower and validator. The follower answers null and the
   validator holds the send: before the fix the result was `unknown`; now it
   is `pending`, and the admitter is asked first, without the follower. The
   devnet stress run adds a follower and reads every burst hash through it
   (counts below).
3. **Medium, contiguous queue: fixed.** `QUEUED` now keeps, for each sender,
   nonce → hash from the chain nonce up (≤ 64), not only the newest pair.
   The next nonce is the end of the unbroken run of still-pending sends
   (`next_in_sequence`). If N dropped while N+1 is still pending, the next send
   gets N, which fills the gap, and N+2 is refused. Test:
   `an_older_drop_under_a_pending_successor_holds_the_next_send`. Before the
   fix the next nonce was 7, and nonce 7 was sent behind the missing 5.
4. **Medium, receipt cost: fixed.** The node keeps a sender → pending-nonces
   index (`nonces_by_sender`), updated wherever the pool changes: admission,
   eviction, and the rebuild at finalization. Under the chain lock, the
   receipt handler now only does lookups and copies one sender's ≤ 64
   nonces (`pending_facts`). It computes the reason, including the refill
   binary search, after releasing the lock. Tests:
   `a_pending_lookup_reads_only_its_senders_entries` (before the fix, one
   read examined all 2,001 pool entries) and
   `the_sender_nonce_index_follows_the_pool`.
5. **Medium, a drop is not final: fixed.** `TxStatus` gains `is_final`. Only
   `included` and the new `replaced` are final. `replaced` means the chain
   nonce passed ours and no path has our receipt; the nonce is read before
   the receipt, and every path is asked once more first. A node's `dropped`
   and an `unknown` stay open. Their words are "… 처리되지 않았어요 (아직 체인에
   기록되지 않음)", with no "cancelled" and no "no money left". The wallet has a
   separate `notIncluded` row state, labelled "Not on chain yet". The row
   keeps polling, and a later receipt overrides the drop (`TxTrack`, chain
   history merge, `reconcileUnresolved` with the stored nonce). A payment-link
   callback gets `status=not_included` with that note; `failed` is sent only
   on a chain fact. The agent's history no longer finalizes a drop.
   `History.markNotIncluded` keeps the pending entry with its sender and
   nonce, and `aether_history` / `aether_receipt` finalize it only on a
   receipt or `replaced`. The explorer shows a dropped hash as "not recorded
   on chain yet" and keeps re-checking it. Tests: agent `Tests/history` (a
   later receipt now overrides a drop; before the fix the entry stayed
   `failed`), wallet `Tests/tx-track` (before the fix a drop mapped to
   `failed`), `a_drop_is_not_final_and_says_not_recorded_yet`, and the
   explorer `droppedText` test.
6. **Low, CLI zero price: fixed.** The CLI's `fee_caps` now uses the app's
   rule (`wallet_state_price`, shared). A zero state cap is allowed only on
   the legacy stateless chain 7780. On any other chain a reported 0 or a
   missing price clamps to `2 × STATE_UNIT_PRICE` (the spec's clamp branch:
   nodes omit the field where state is unpriced, so local test devnets keep
   sending, and a cap there is harmless). A malformed price, or a status
   without a chain id, is refused. Test:
   `cli_state_cap_has_headroom_and_zero_only_on_the_legacy_chain`. Before the
   fix it signed 0 on paid chain 7796.

**End to end (2026-10-07, `scripts/contracts-live.sh reset bin chain stress
stop`, with a follower):** 220 submissions, 205 with a hash.
- 55 included (50 transfers, 5 deploys).
- 15 deploys refused at admission with text.
- 150 transfers dropped with `state_price_above_cap`.
- 0 silent losses, and the mempool was empty after the window.
- Blocks never stopped (max stall 6.1 s, height 5 → 272).

For the 150 hashes the validator held, the follower answered `null` every
time, after submission and after the drain. The wallet's routed order
answered all of them both times (`routedNull` 0). The devnet was stopped
afterwards.

Remaining limits, as before: tombstones are bounded and kept in memory only.
`replaced` relies on receipts that the nodes still hold; a node keeps at least
one sealed era of them. The book of sends lives only for one process: after
a restart the wallet reconciles through the nonce it stored on the row, and
the agent through the sender and nonce in its pending history.
