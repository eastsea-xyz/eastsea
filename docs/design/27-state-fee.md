# Persistent growth fee and inventory (new genesis)

`node_rewards || history_v2` activates these rules from genesis. Chain 7780
has neither flag and retains its old unlimited, unused state-gas dimension and
fee behavior. New genesis has a **100,000-unit burst bucket**, refilled by **32 units per
finalized block** (one second). The actual block limit is the remaining bucket,
not a fresh 100,000 units every second. One unit burns at least
`1_000_000_000_000` wei (0.000001 DBLN), even when execution and proving
base fees are zero; sustained demand raises that price. The signed `header.gas.state` reserves the maximum
before EVM execution; unused funds return to the sender.

## Authoritative persistence inventory

The table inventories logical payloads and indexes. The paid archive rows also
have the stored key/value bound below; redb page and Merkle tree overhead are
excluded. “Window” means the default 30-day history-v2 retention, rounded
to complete 8,192-block eras; an archive node keeps the row indefinitely.
The operator can configure another window or drop old era files. A wire limit
alone bounds size, but does not price long-lived archive growth.
The marshal archive's freezer/journal is physical backing for its listed
block/certificate rows. RPC snapshots are memory-only; a follower's bounded
download file is temporary and installs into the same state/code tables.

| Finalized bytes | Payer or subsidy | Per-block bound | Retention |
|---|---|---|---|
| State tree basic account record, **including the sender's first nonce account**, recipient and contract accounts | Transaction pays 100 units/new account. Issuance funds protocol prover/operator credits. | Transaction portion: 100,000 units, hence at most 1,000 otherwise empty accounts. Protocol: at most two proof claims/block and at most 100,000 visible registered operators at an epoch payout. | Until removed; no routine state pruning. |
| State tree storage keys and 32-byte values | Transaction pays 100 units/newly occupied slot. System registry, beacon, proof and reward words are protocol subsidy (below). | At most 512 transaction-created slots/block; system bounds below. | Until cleared; no routine state pruning. |
| State tree code hash/chunks and `CODE` blobs | Transaction pays one unit/new code byte, plus a new-account charge. | 100,000 priced code bytes/block, further limited by EVM gas/code deposit. | Tree chunks until replacement/deletion; historical `CODE` blobs remain indefinitely. |
| Canonical signed transaction envelope, calldata, and the transaction copy in staged blocks and era files | Transaction pays one unit/32 canonical bytes, rounded with its receipt. | Combined logical signed-transaction/receipt meter: 2 MiB/block; its 16× stored-byte allowance bounds paid key/value rows to 32 MiB/block. Block wire cap: 8 MiB. | Staged until seal; era file kept by default, optionally dropped after pruning. |
| `RECEIPTS`: fixed fields, return/revert output, event addresses, topics and data | Transaction pays for 128 fixed bytes/receipt, 64/event, output bytes, 32/topic and data bytes, at one unit/32 bytes. | Same combined 2 MiB logical and 32 MiB stored-byte bound, plus EVM gas limits. A 480 × 4,096-byte LOG0 block measured **3,973,691 stored / 1,997,287 metered = 1.990×** across receipt, activity/index, summary and staged block rows; the bound is 16×. | Window; archive indefinitely. |
| `BLOCKS` summary, transaction-hash index, block links and state root | Transaction envelope and receipt-base charges cover transaction-dependent rows; fixed header is protocol overhead. | One summary/block and at most 2,000 transactions; 8 MiB wire cap. | Window; archive indefinitely. |
| `ACCOUNT_HISTORY` sender/recipient/token activity and `ACCOUNT_BLOCK_KEYS` (32-byte reverse key/row) | Transaction base pays the sender row; value recipients require a paid account or transfer; extra token rows require metered events. Reward/registration rows are protocol subsidy. | At most 2,000 transaction base rows, recipient rows and at most two address rows per recognized Transfer event; event count and serialized row data are within the shared 16× allowance on the 2 MiB logical meter. Swap details attach to one sender row. | Window; archive indefinitely. |
| `ERA_BLOCKS` staged encoded block, sealed `.aera` file, permanent `ERA_ROOTS` | Signed transaction bytes priced above; bounded proof/beacon/registration/handoff/seed payload is protocol subsidy. | One encoded block of at most 8 MiB/height; one 32-byte root/8,192 blocks. | Staged until seal; era file default indefinitely or dropped after pruning; root permanently. |
| Marshal finalized block/certificate and follower `PROOFS` finality certificate | Consensus protocol subsidy, independent of a transaction. | One block/certificate per height, block at most 8 MiB; committee size at most 128. | Window; archive indefinitely. |
| `REWARDS` prover tax rows and reward account-history rows | Issuance for valid proof claims, not a transaction sender. | At most two claims/block, each decoded proof at most 128 KiB; at most two prover rows/block. Epoch operator payout sees at most 100,000 registered candidates. | Tax rows indefinitely; account history window. |
| `META`: head/root/digest, history MMR peaks, schedule, handoff, seed, statement, notices and era-start record | Consensus/committee protocol subsidy. | One head update/block; MMR peaks logarithmic in height; at most one committee-approved activation/block, bounded notices and at most 128 roster seats; one era-start record/era. | Current values overwritten; era-start record until seal. |
| Free registration payload, registry candidate/index/lane-nonce words and in-memory pseudo-receipt | Explicit onboarding subsidy signed by operator and registrar. | Four free items/block; **16 total free plus contract registrations/epoch**. Free lane has a 100,000-candidate lifetime cap. Each free item adds at most five candidate words, one index, one lane nonce and bounded shared counters. | Registry words permanent; payload in era file; pseudo-receipt only in memory; history row window. |
| Beacon answer payload, per-candidate liveness, beacon and availability words | Registered-operator protocol subsidy, checked by voting-key signature. | 1,024 answers/block; twelve ordinary answer slots/candidate/epoch. Availability signals are bound to a signed block height. Existing candidate words are updated; v3 adds at most two availability words/candidate. | State until overwrite; payload in era file. |
| Proof commitment/escrow records, expiry deletions, epoch randomness, roster/pool and reward state | Proof issuance and committee protocol subsidy. | At most two claims/block; proof records expire after the claim window; one randomness word/epoch; roster/pool bounded by the registry and 128-seat committee. | Proof records until expiry; randomness and bounded registry/reward state persist. |

Any new persistent table or retained block field **must be added here with its
payer, price or explicit protocol subsidy, per-block bound and retention rule
before activation**. Paid registry contract calls beyond the free lane's
100,000-entry limit still pay transaction state units; reward/beacon system
processing sees at most the first 100,000 candidates.

The paid-row expansion allowance is 16 stored key/value bytes per logical
metered byte (`MAX_STORED_BYTES_PER_METERED_BYTE`). The bound covers the
`RECEIPTS`, `ACCOUNT_HISTORY`, `ACCOUNT_BLOCK_KEYS`, `BLOCKS` transaction hash,
and staged transaction copy: receipt JSON hex needs two characters per binary
byte and at most 68 per 32-byte topic; its fixed fields fit within 16× the
128-byte receipt floor. A history base row and reverse key fit within 16× a
signed envelope. Each decoded batch recipient occupies at least one 96-byte
ABI tuple and adds at most one row. A recognized Transfer event has 192
logical bytes and adds at most two address rows and token movements; a Swap
event's 288 logical bytes add detail only to the sender row. Those bounded
JSON rows and the signed transaction's staged copy fit inside the remaining
16× allowance. Protocol entries listed separately above do not consume this
transaction budget, but every canonical block payload byte consumes the independent
archive budget below, including proof blobs, beacons, registrations and BAL. The redb regression test measures key and value bytes,
not allocated pages or fragmentation.

## Transaction price and common cap

The executor counts the final revm state difference against the transaction's
pre-state. A reverted or set-then-cleared slot adds no storage units. A new
sender account costs 100 units even if it only stores nonce one. A new fee
recipient created solely by revm's tip credit is protocol overhead; a user
transaction that uses it otherwise pays normally.

For each transaction, *persistent bytes* are its canonical signed envelope
length plus 128 receipt bytes, its return/revert output length, and for each
event 64 framing/address bytes + 32 bytes/topic + data length. The charge is
`ceil(persistent_bytes / 32)` state units. A zero-topic, zero-data `LOG0`
therefore has a price. The logical sum may not exceed **2 MiB per block**:
the 16× expansion bound then caps paid redb key/value payloads at
**32 MiB per block**. Proposer
selection, validator execution, FOCIL append checks, admission of a single
transaction and proving replay use the same executor rule. A transaction
over the byte cap or its signed state budget is invalid.
Admission reports the same cost as execution for a transaction at its current
nonce or after an executable pending prefix. A future-nonce pool entry without
that prefix is provisional: the proposer re-executes it in block order and
cannot finalize it or persist its bytes unless the prefix and all fees pass.

The receipt remains stored for wallet queries, including zero-value and
zero-tip transactions, because its bytes are priced. A normal ERC-20 Transfer
event has three topics and 32 data bytes: 64+96+32 = 192 metered bytes,
costing six units (0.000006 AETH) beyond the transaction and receipt base.
The audit's 400 × 4,096-byte `LOG0` call pays for over 1.6 MiB of event bytes.
A 480-event call fits near the logical cap; repeated calls cannot exceed it.
The
audit's 714 fresh senders require at least 71,400 units for accounts plus
transaction/receipt units; zero-balance senders cannot reserve that fee and
fail both admission and execution. An unused full 100,000-unit burst budget costs 0.1 DBLN at the floor.
It cannot be consumed again until it refills, and congested blocks pay more.

No product flow needs an account-less sender's zero-value plain transfer.
Mac onboarding uses the bounded free registration lane and leaves its wallet
nonce at zero. A token-only wallet without native balance needs fee funding
before a token transfer; the proposed sponsorship pool is not implemented.
The old free first-transaction promise is withdrawn for new-genesis chains.
Chain 7780 keeps its legacy path. Wallets reserve enough state budget for
possible fresh sender/recipient accounts and ordinary contract logs; receipts
report the actual units and burned fee.

The name service still needs its separate commitment bond and committer
binding: this generic growth fee does not compensate the service for abandoned
commitments.


## B5: consumer disk envelope and congestion price (2026-10-05)

A per-block cap alone was insufficient: 2 MiB/second permits 181.19 GB/day
(168.75 GiB) of logical transaction/receipt data and, at 16×, 2.899 TB/day
of paid stored key/value rows. Receipt-only spam could buy that rate for
about 5,662 DBLN/day at the floor. A beta has no reliable market price to make
that cost a disk defense. The hard rolling budget now holds even for a funded
attacker willing to pay any fee.

Let `B = 100,000`, `R = 32`, `d = parent.excess.state`, and `u` be the block's
actual state units. A block requires `u <= B - d` and records
`d' = max(0, d + u - R)`. Debt is certified in existing parent metadata and
persisted in block summaries/snapshots; an empty block replenishes R, restarting
never replenishes anything, and wall-clock gaps do not mint budget. In any N
consecutive blocks, `sum(u) <= B + R*N`. All code, accounts, slots and archived
bytes share that inequality. Rounding the archive charge up only tightens it.

| Payload envelope at one block/second | Per block/burst | Per day | Per 30 days | Per 365 days |
|---|---:|---:|---:|---:|
| Paid logical archive, if all units buy 32 bytes | Existing 2 MiB block cap; sustained 1,024 B/s | 88.474 MB steady + at most 3.20 MB initial burst | 2.657 GB including burst | 32.296 GB including burst |
| Paid stored archive key/value rows at 16× | Existing 32 MiB block cap; sustained 16,384 B/s | 1.416 GB steady; **1.467 GB including burst** | **42.519 GB including burst** | 516.737 GB including burst |
| Permanent new code, if all units buy code bytes | Up to 100,000 priced bytes in initial burst; sustained 32 B/s | 2.765 MB steady | 83.044 MB including burst | **1.009 GB including burst** |
| Permanent state: conservative 16× storage planning allowance | 1.6 MB initial burst; sustained 512 B/s | 44.237 MB steady | 1.329 GB including burst | **16.148 GB including burst** |

MB/GB/TB here are decimal. The archive and permanent maxima are alternatives,
not additive: both spend the same bucket. Slots/accounts cost 100 units rather
than one; at most about 27,648 new slots or accounts/day are sustainable, with
the existing 512-slot block cap still enforced. The node stores 32-byte tree
keys and 32-byte values, plus code by hash (31 code bytes per tree chunk);
16× for permanent state is a conservative planning allowance, not a new
assertion about filesystem allocation. The paid archive 16× bound remains the
existing representation bound and redb regression measurement.

An independent **8 MiB encoded-payload bucket**, refilled by **4,096 bytes
per finalized height**, closes the unpaid system-payload gap. It measures the
entire canonical payload: signed transactions, BAL, proofs, beacons,
registrations, committee handoffs, seeds, upgrade notices and fixed headers.
Validator execution checks its available budget **before system pre-state
writes**. The proposer selects/defer extras before applying those writes,
selects transactions with an incremental executor callback over each tentative
block outcome so rejected candidates commit no state and BAL, gas and receipts
remain correct. No full-block replay is needed per rejected candidate. The paid state fee/context/proof
statement are unchanged by this independent envelope.

With archive debt `a`, a block must fit `8 MiB - a` and records
`a' = max(0, a + canonical_payload_bytes - 4096)`. Genesis starts at zero.
Nonzero debt is authenticated by the child's `parent_meta`, persisted in
versioned packed summaries and snapshot envelopes, and restored on restart.
Zero debt retains the legacy packed summary/postcard and metadata encodings;
chain 7780 never charges archive debt. Neither restarts nor wall time refill it.
In N consecutive heights total payload bytes are at most `8 MiB + 4096*N`.
The original 8 MiB wire-sized burst preserves log-heavy ordinary transactions.

**256 KiB of burst capacity is reserved for committee control fields**
(handoff, seed or upgrade). Without any such field, the effective cap is the
larger of the canonical empty payload size and `available - 256 KiB`, never
more than available. Below that reserve, only empty blocks fit, allowing the
bucket to refill; optional proof/beacon traffic and small transactions cannot
starve an urgent upgrade. Control fields take priority over optional entries.
Each individual control item, including its empty header, must fit the 256 KiB
reserve; validator execution enforces this regardless of committee signatures.
Combined controls may use the whole available bucket. If the combination does
not fit, the proposer first carries a fitting upgrade alone, then a fitting
handoff/seed pair, or the seed alone so the handoff can follow. Deferral never
waits for a control larger than the protected reserve: such an item is invalid
and must be signed in a smaller form or changed by a version-gated upgrade.
The upgrade's 16 releases × four 512-byte fields × worst-case six-character
JSON escapes, plus notes, 128 emergency approval pairs and fixed framing, fit
below 256 KiB; normally encoded handoffs/seeds are much smaller, and the
individual bound also constrains unusual DKG output encodings.
FOCIL append checks include the prospective transaction's exact BAL and payload
bytes and the same reserve rule. They execute only the prospective appended
transaction, restoring the prior exact fee settlement before merging BAL, gas
and receipts; the full inclusion-list obligation is retained. Proof claims have a 32 KiB decoded minimum,
128 KiB maximum; their hexadecimal payload encoding means roughly 16–64 seconds
of refill per claim before header overhead, even though the burst can hold more.

| Encoded archive planning envelope (one block/second) | Burst | Per day | Per 30 days | Per 365 days |
|---|---:|---:|---:|---:|
| Complete canonical payload | 8.389 MB; 4,096 B/s steady | 353.894 MB steady | 10.625 GB including burst | 129.180 GB including burst |
| Four physical payload copies (staged/era plus consensus archive allowance) | 33.554 MB; 16,384 B/s steady | 1.416 GB steady; 1.449 GB including burst | 42.501 GB including burst | 516.719 GB including burst |
| Paid rows plus four payload copies, conservatively double-counting paid transaction bytes | 84.754 MB combined burst | **2.831 GB steady; 2.916 GB including burst** | **85.019 GB including bursts** | **1.033 TB including bursts** |

The four-copy allowance is a conservative bound on simultaneously retained
payload representations, not a promise about allocated filesystem pages.
Fixed certificates, permanent system records from the inventory, redb page
fragmentation and transient snapshots still require operator headroom. The
100,000-entry candidate registry, epoch registration cap and protocol-specific
system-state bounds remain necessary; encoded archive metering does not price
those state writes. A roughly 85 GB 30-day payload envelope can fit a consumer
Mac with 100–500 GB free while leaving space for permanent state and overhead.
The lower end requires monitoring/free-space stops and actual era deletion:
the default history policy retains sealed eras after pruning query tables.
Keeping all eras permanently instead admits the annual 1.033 TB archive
figure; the paid permanent-state envelope remains the table above. Operators
must configure era-file deletion to enforce a 30-day retained payload window.
A full 8 MiB archive burst needs 2,048 heights (34 minutes 8 seconds) to refill
before subtracting the required empty header bytes; real empty blocks recover
slightly slower because their bytes are counted too.

State price uses the existing fee vector and proof context; the independent
archive budget only adds authenticated metadata in versioned store/snapshot
envelopes:

`base_state = fake_exponential(10^12, max(0, d - 50,000), 12,500)`.

It stays at the old floor through 50,000 units of burst debt. At debt 62,500,
75,000 and near 100,000 it is approximately 2.72×, 7.39× and 54.46× the
floor. The proposer/admission context and stateless proof statement carry both
the remaining capacity and fee vector. All execution paths reserve at the
current price, reject a lower signed cap, refund unused reservation and burn
actual units times price. The state floor and surcharge apply even when a
new-genesis devnet disables execution/proving fees.

Transfers, ERC-20 calls, swaps, names and the audited 12,588-unit vault deploy
retain their accounting and floor costs during ordinary use. A full bucket
admits several such deployments; three consecutive 12,588-unit samples leave debt below the 50,000-unit
price-free burst. A fourth still pays the floor, then raises the next price
slightly as debt passes that threshold. A 12,588-unit burst recovers
in at most 394 empty heights, and the whole bucket in at most 3,125 heights
(52 minutes 5 seconds). Signed budgets may still be 100,000: consensus checks
actual committed units against remaining capacity, so a conservative wallet
estimate does not reject a small transfer. Congestion deliberately delays large
bursts until capacity returns or a smaller transaction is selected.

### Retuning after launch (G4)

Parameters live together in `crates/execution/src/fees.rs`: burst capacity,
refill, price-free burst, exponential denominator, floor price and the existing
byte/slot/expansion caps, encoded archive burst/refill, control reserve and
four-copy planning allowance. The launch checklist also checks the paid-row envelope
including burst stays below **1.5 GB/day** and the combined paid-plus-archive
planning envelope stays below **3 GB/day** and verifies that state price rises.
We keep constants for this pre-genesis change: moving parameters into registry
storage would require new authenticated update payloads, migration defaults,
witness/context plumbing and replay rules. It is not a cheap local parameter
write, and an unauthenticated registry setter must not acquire consensus power.

Retune with a committee-signed protocol upgrade and matching binaries, using
the normal **604,800-block notice**, or the B4 emergency **n−f approval plus
one-epoch notice** for an urgent disk defense. Update the constants, disk math,
launch policy and differential/replay tests together; increase protocol version
and activate the changed rule at the scheduled height, never silently replace
the running protocol's rules. The encoded archive constants and control reserve
require the same version gate; a binary-only constant edit is not an activation. If a future bounded registry parameter mechanism
is introduced, migrate existing debt conservatively (never reset/replenish it),
bind values to certified state and proof contexts, and retain hard ceilings on
refill, burst, byte expansion and minimum price. **Chain 7780 keeps zero state
debt/price, unused unlimited state gas, and its pinned genesis/receipt bytes.**
