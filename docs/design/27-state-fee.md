# Persistent growth fee and inventory (new genesis)

`node_rewards || history_v2` activates these rules from genesis. Chain 7780
has neither flag and retains its old unlimited, unused state-gas dimension and
fee behavior. The new-genesis limit is 100,000 state units per block. One
unit burns `1_000_000_000_000` wei (0.000001 AETH), even when execution and
proving base fees are zero. The signed `header.gas.state` reserves the maximum
before EVM execution; unused funds return to the sender.

## Authoritative persistence inventory

The table counts logical payloads and indexes, excluding redb page and Merkle
tree overhead. “Window” means the default 30-day history-v2 retention, rounded
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
| Canonical signed transaction envelope, calldata, and the transaction copy in staged blocks and era files | Transaction pays one unit/32 canonical bytes, rounded with its receipt. | Combined signed-transaction/receipt meter: 2 MiB/block; block wire cap: 8 MiB. | Staged until seal; era file kept by default, optionally dropped after pruning. |
| `RECEIPTS`: fixed fields, return/revert output, event addresses, topics and data | Transaction pays for 128 fixed bytes/receipt, 64/event, output bytes, 32/topic and data bytes, at one unit/32 bytes. | Same combined 2 MiB/block cap, plus EVM gas limits. | Window; archive indefinitely. |
| `BLOCKS` summary, transaction-hash index, block links and state root | Transaction envelope and receipt-base charges cover transaction-dependent rows; fixed header is protocol overhead. | One summary/block and at most 2,000 transactions; 8 MiB wire cap. | Window; archive indefinitely. |
| `ACCOUNT_HISTORY` sender/recipient/token activity and `ACCOUNT_BLOCK_KEYS` (32-byte reverse key/row) | Transaction base pays the sender row; value recipients require a paid account or transfer; extra token rows require metered events. Reward/registration rows are protocol subsidy. | At most 2,000 transaction base rows, recipient rows and at most two address rows per recognized Transfer event; event count and serialized row data are linear in the 2 MiB metered event/receipt cap. Swap details attach to one sender row. | Window; archive indefinitely. |
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
therefore has a price. The sum may not exceed **2 MiB per block**. Proposer
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
The audit's 400 × 4,096-byte `LOG0` call must therefore pay for over 1.6 MiB
of event bytes, and repeated calls cannot exceed the 2 MiB block cap. The
audit's 714 fresh senders require at least 71,400 units for accounts plus
transaction/receipt units; zero-balance senders cannot reserve that fee and
fail both admission and execution. The full 100,000-unit state budget burns
at most 0.1 AETH/block.

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
