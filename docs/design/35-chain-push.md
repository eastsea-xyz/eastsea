# 35. Chain push: EastSea's notification bus

Date: **2026-10-07**. Status: **design only**. This document changes no Rust,
Swift, contract, genesis, or running network. All implementation and fault tests
below are proposed work, not tests run in this lane.

Founder: “푸시역할을 온체인에 넣어 해결할 수 있잖아. 프로토콜을 만들자.”

**EastSea tells you when something happens, using the chain your device already
follows.** The first consumer is the chain-announced silent-update path in
[34's final section](34-silent-updates.md#founder-decision-2026-10-07-the-chain-announces-updates-not-polling).
An online Mac reacts at finality. An active iPhone verifies a subscription from
any reachable follower. **An on-chain bus cannot wake a suspended iPhone.**
Without an optional OS wake service, it catches up when the user opens it.

## 0. Decision and existing boundaries

Use **existing finalized transaction receipts/logs and certified block fields**
as the bus. Add a bounded delivery queue on each serving node, with resumable
`aether_subscribe` over RPC/iroh. There is no push coordinator, recurring
notification-discovery poll, on-chain inbox, subscription registry, or new
consensus ring. A follower serves the history it already verifies; it cannot
authorize a message by serving it.

Read-first basis: [04 execution](04-execution.md), [15 rewards](15-node-rewards.md),
[19 release approval](19-release-approval.md), [22 gas pool](22-gas-pool.md),
[27 B5 storage fees](27-state-fee.md), [29 unattended restart](29-unattended-restart.md),
[32 health signals](32-health-signal.md), [33 public discovery](33-dht-public-addresses.md),
[upgrade.rs](../../crates/node/src/upgrade.rs),
[rpc.rs](../../crates/node/src/rpc.rs), and the `product-philosophy` and
`no-founder-control` memory notes. Consumer screens show the event and any action
needed, without chain vocabulary. Pipln publishes neutral software; there is no
founder key, privileged sender, mandatory Pipln endpoint, notification fee share,
or remotely administered wallet allowlist.

| Current evidence | Boundary for this design |
|---|---|
| New-genesis blocks commit `receipts_root` at the same height; validators re-execute and compare it (04). | Receipt events can be authenticated at finality without waiting for a zk proof. Existing `BlockProof` does **not** prove receipts. |
| `aether_getReceiptProof` returns a receipt, ordered inclusion path, and certified block. | Reuse this proof format. Legacy blocks without a receipt commitment cannot supply this guarantee. |
| `ReleaseLog.publish` is permissionless (19, audit F-07). | A finalized `Published` event is discovery, never release approval. |
| `aether_status` exposes schedule/notices; `aether_releaseEntries` lists at most 64 entries; `eth_getLogs` scans at most 2,000 finalized heights. | These are current reads, not subscriptions or approval proofs. `aether_status.release` and `aether_subscribe` are proposed additions. |
| `aether/rpc/1` currently completes one JSON response per QUIC bidirectional stream. | A persistent framed subscription is new transport work, not an already working method. |
| B5 prices retained transaction/receipt bytes, including zero-value calls. | App publishers pay even when execution/proving base fees are zero. The sponsorship pool in 22 is unimplemented and supplies no free push lane. |

## 1. Message model

### 1.1 Topics and their actual sources

A topic is a client interpretation of proven chain data. A string, event
signature, app name, or RPC's `kind` field grants no authority.

| Class / topic | Source and meaning |
|---|---|
| `system/release` | A `Published` receipt from the pinned ReleaseLog, promoted to an approved announcement only after 19's manifest, builder, code and storage checks. |
| `system/protocol_upgrade` | The existing `SignedUpgrade` in a finalized block's `payload.upgrade`; includes protocol and activation height. |
| `system/security_advisory` | A bounded, separately committee- or builder-signed advisory carried in an ordinary paid log. Warns about an identified issue; cannot change permissions, install software, or move funds. |
| `account/<address>/payment_received` | Successful native transfer proven by the certified transaction **and** its receipt; or a recognized, wallet-approved token's `Transfer` event. An arbitrary token's lookalike event remains app data. |
| `account/<address>/recovery_started` | `RecoveryProposed` from the actual account context running the authenticated EastSeaAccount implementation; recovery nonce, calls hash and ready time come from that receipt. |
| `account/<address>/agent_limit_hit` | A finalized failed direct account transaction with the exact top-level `OverPaymentLimit` or `OverDailyLimit` error in its receipt output. This is a typed refusal, not a successful payment. |
| `app/<publisher>/<appId>/<topic>` | A fee-paid publisher event. Anyone may publish; the wallet decides which publishers/topics it receives and displays. |

Account notices are produced by actual protocol/account execution. There is no
API for a sender to assert “payment received” or “recovery started” on another
account's behalf. Native receipt fields alone omit destination/value: include
the signed transaction from the certified block. Recovery and limit adapters
also require authenticated account implementation/delegation provenance for the
execution, the transaction target and method, and the receipt outcome.

The v1 native adapter covers positive-value plain transfers (empty calldata) to
an EOA or an authenticated account receive handler known not to forward/refund
the value. Success of a complex value-bearing contract call alone does not
prove a net payment received; internal transfers need recognized receipt events
or separately proven account outcomes. Do not manufacture an amount from an
RPC activity label.

EastSeaAccount's limit errors revert: **reverted calls leave no event logs**.
Match the complete top-level error ABI from a direct `sessionExecute` call to the
trusted account; never search nested `CallFailed` bytes for a familiar selector.
A mempool rejection, simulation failure, or agent's local preflight refusal has
no finalized receipt. It stays a local diagnosis under 32, explicitly separate
from a chain notification. A later account implementation cannot gain this
adapter merely by copying its error names.

### 1.2 App event convention

An optional ordinary **stateless** `ChainPushLog` contract provides this ABI:

```solidity
event PushV1(
    bytes32 indexed topic,
    address indexed sender,
    address indexed recipient,
    bytes envelope
);
```

The emitting function sets `sender = msg.sender`, checks envelope bounds, and
only emits; no `SSTORE`, inbox, topic registration, fee receiver, admin, or proxy.
Any deployment with the independently authenticated expected runtime code may
be adopted by a wallet. Adoption uses an authenticated client release or a
user's explicit contract pin, not a DHT claim or a rewrite of ceremony-fixed
`network.json`. Existing dApps may implement the same convention, but an
unverified emitter's claimed sender is not an authenticated publisher.

Use 33's chain fingerprint `C` (authenticated chain ID, initial group identity,
and genesis digest) and group `g`. Define app topic bytes as:

```text
SHA256("eastsea/app-topic/v1\0" || C || u16(g) || publisher20
       || appId32 || SHA256(UTF8(topic_name)))
```

Integers are big-endian; `topic_name` is at most 64 UTF-8 bytes. `recipient=0`
means a public broadcast, otherwise it is an account address. All addresses,
topic bytes and emitter identity are covered by the receipt commitment.
An app cannot acquire a system/account namespace by selecting a similar hash.

The packed envelope has **87 header bytes**, in this order:

| Field | Bytes / rule |
|---|---|
| Magic `ESP1` | 4; the format version |
| `valid_from_height`, `expires_height` | 8 + 8 |
| Publisher-chosen `message_id` | 32; stable across intentional retries |
| `codec` | 1; `0` plain bytes, `1` the encrypted form in §4 |
| `recipient_key_id` | 32; zero for plaintext |
| `body_len` | 2; exact remaining byte length |
| `body` | At most 1,961 bytes; complete envelope **≤2,048 bytes** |

Consumers require `valid_from_height <= inclusion_height < expires_height` and
`0 < expires_height - valid_from_height <= 604,800`. They also require their
current verified head to be below `expires_height`. Seven days means 604,800
finalized heights at the target one-second cadence, not a wall-clock delivery
SLA; no heights advance while finality is stopped. Unknown versions/codecs,
trailing bytes and invalid bounds are rejected before decoding content.

Transport identity is `(C, g, block_digest, tx_index, event_index)`; an index
always refers to the **unfiltered receipt**, not the RPC's list position.
Semantic app deduplication uses `(C, g, emitter, sender, topic, message_id)`.
Keep the first verified payload hash for that key: a retry with different bytes
is a conflict, not a replacement. Unexpired semantic dedup entries are never
evicted to admit new IDs: at capacity, suppress/coalesce new app notices and
report an explicit policy gap until entries expire. A cursor prevents replay
of old inclusions; it alone cannot prevent re-inclusion of the same ID at a
newer height. No notification initiates signing, recovery execution, account
configuration, or an arbitrary URL fetch automatically.

### 1.3 Where bytes live, exact B5 cost, and retention

Notification bodies live in receipts/logs, with their transaction calldata in
ordinary chain history. **Incremental permanent notification state: 0 slots,
0 new accounts, 0 code bytes per message.** Deployment code/account costs are
paid once by the deployer. A publisher using another contract with state writes
pays those separately; the bus does not exempt them.
The ordinary first sender nonce account still costs 100 units when created;
zero incremental notification state means no additional inbox/topic records,
not an exemption for new publishers' ordinary accounts.

The exact B5 logical charge, using 27 and `receipt_persistent_bytes`, is:

```text
T = length of the canonical signed transaction envelope (including calldata)
O = receipt return/revert output bytes
L = T + 128 + O + sum_events(64 + 32*topic_count + event_data_bytes)
U = ceil(L/32) + 100*new_accounts + 100*newly_occupied_slots + new_code_bytes
P(d) = fake_exponential(10^12, max(0, parent_state_debt - 50,000), 12,500)
state_fee_wei = U * P(d)
```

This is one rounding per transaction, not one per notification. Execution and
proving fees still apply. State fees burn; no portion buys a founder service.
At the floor one unit is **0.000001 DBLN**. Signed state budgets reserve the
maximum before execution and refund unused reservation. Expiry/pruning gives
no storage-fee refund. Ciphertext is priced exactly like plaintext.

`PushV1` has four EVM topics (signature plus three indexed fields). With an
envelope of E bytes, ABI event data is `64 + 32*ceil(E/32)` bytes (offset,
length, padded bytes). Thus the event contributes exactly
`256 + 32*ceil(E/32)` logical bytes, or **`8 + ceil(E/32)` additional units**:

| Envelope E | Event logical bytes | Additional units | Floor fee for the event alone |
|---:|---:|---:|---:|
| 128 B | 384 B | 12 | 0.000012 DBLN |
| 1,024 B | 1,280 B | 40 | 0.000040 DBLN |
| 2,048 B | 2,304 B | 72 | 0.000072 DBLN |

These exclude T, receipt base/output, execution/proving fees and new state.
For example, a **fixture** with T=512, O=0, E=128 and no new state pays
`ceil((512+128+384)/32)=32` units, **0.000032 DBLN** at the floor. T is measured
from the actual signed envelope, not estimated from an ABI payload. Content
present in both calldata and the event is retained twice and charged twice.

The 16× B5 allowance bounds paid stored **key/value bytes**, not exact allocated
disk pages: the 384-byte event accounts for at most 6,144 such bytes within the
shared representation allowance. Do not promise a byte-for-byte physical disk
cost; indexes, redb pages, certificates and fragmentation need headroom. Adding
a persistent push index would require its own inventory, payer/subsidy, bound
and retention in 27. V1 adds none.

| Representation | Retention / deletion |
|---|---|
| Receipt, block summary, certificate and existing activity indexes | History-v2 default: 30 configured days expressed in heights, pruned in complete 8,192-block eras. At one second/block: 2,592,000 heights, plus era rounding; unsealed eras/delayed pruning can extend retention. Nodes advertise their actual floor. |
| Staged blocks and sealed era files | Staged until seal. **Era files stay by default even after query-table pruning.** A bounded disk window requires `--history prune --retain-days 30 --drop-era-files`, with successful pruning and disk monitoring. |
| Archive node or third party's copies | May retain forever. Message TTL is not an erasure promise, including for encrypted content. |
| Node delivery queue (local, no consensus state) | At most 1,024 references and 4 MiB, whichever fills first; evict expired items/oldest items. No message is pinned beyond these limits. |
| Wallet notification cache | At most 1,024 messages / 4 MiB; expires with the message. Dedup table ≤4,096 entries; retain unexpired entries even if inbox content is evicted. At capacity refuse new app IDs with a policy gap. Durable cursor prevents older inclusion replay. |

The app/advisory **delivery TTL is at most 604,800 heights**. Native account
adapters use the same seven-day notification window from inclusion. Release
and upgrade announcements remain useful for catch-up while their verified
underlying approval/schedule is relevant; they do not create a new durable
message record. Their local cache still fits the table's cap.

The existing ReleaseLog is a distinct, already permanent **approval record**:
four newly occupied slots per entry (400 units), plus 100 units for the array
length slot when first occupied, and its usual transaction/event fees. Do not
generalize that append-only array to app notifications or create another copy
of manifests in state. Its existing growth remains priced under B5 (19/27).
Replacing its approval evidence with expiring logs would change 19's trust
model and is outside this proposal.

## 2. Who may post and who may authorize

**Relaying is open; authority is constrained.** An unapproved system-looking
log never enters the authenticated system delivery class, even when included
in a finalized block.

| Class | Admission to the authenticated class |
|---|---|
| System release | Pinned builders **2/3**, or **3/3 emergency**, with all of 19's ReleaseLog/manifest checks. Anyone may relay the approved bytes; no unilateral publisher key. |
| System protocol upgrade | Current committee BLS threshold under the chain's BFT quorum (`n-f`, `f=floor((n-1)/3)`): 3/4 seats, 11/16 seats, conventionally “2/3.” Existing normal 604,800-block notice and B4 emergency n-f independent Ed25519 approvals plus one-epoch notice remain. |
| System advisory | Either that committee's separate advisory signature or **2 distinct pinned builder signatures out of 3**. Advisory text grants no installation exception; an emergency app install still needs 3/3. |
| Account topics | Only recognized outcomes of actual finalized transactions under the protocol/account/token provenance checks in §1. No arbitrary poster. |
| App/dApp topics | Anyone paying ordinary execution/proving/B5 fees. Per-sender delivery caps and user filtering apply (§5). |

For advisories, introduce a separate `EastSeaAdvisory/v1` signing domain. The
deterministic signed bytes bind C, group, advisory ID, validity heights,
severity and exact bounded body (≤1,024 bytes). The committee route uses its
authenticated authority at inclusion; the builder route uses the release pins.
Reject duplicate signers, foreign networks/groups, altered text and expired
objects. Never reinterpret an upgrade signature as an advisory signature.
The container log can be paid/relayed by anyone, but a block finality signature
is **not** the advisory's authorization signature.

The only system authorizers are these pre-existing committee/builder sets.
No Pipln-owned “official announcements,” remote preference updates, one-person
emergency channel, notification registrar, or push-service credentials with
on-chain authority are added. System advisories offer information; wallet
policy and existing upgrade/release gates still decide action.

## 3. Delivery without discovery polling

```mermaid
sequenceDiagram
    participant Publisher
    participant Chain
    participant Follower
    participant Wallet
    Publisher->>Chain: Paid event or existing approved upgrade
    Chain->>Follower: Finalized block, execution receipts, certificate
    Follower->>Wallet: aether_subscription frame + proof or proof reference
    Wallet->>Wallet: Verify finality, inclusion, authority, expiry, local filter
    Wallet->>Wallet: Record once; apply the existing action gate
```

### 3.1 Mac full nodes and unattended Macs

After block execution, durable commit and verified finality, the chain/follow
path notifies a local dispatcher. It classifies events and updates bounded
projections such as 34's `aether_status.release`. Queue insertion is nonblocking;
a slow wallet, expensive proof request, or app flood cannot hold the chain lock
or delay a vote. Build/cache proof paths outside the consensus critical path.

An attached Mac wallet uses the local subscription. “Instant at finality” means
there is no timer between observing verified finality and dispatch; verification,
transport and OS scheduling still take time. Same-block receipts may trigger
release discovery immediately, but installation also needs 19's subsequent
certified state anchor and waiting period (§6).

29's unattended daemon can follow, detect and retain bounded update hints before
GUI login. It requires no wallet signing key. The GUI receives a catch-up view
when it attaches. FileVault cold-boot lock, sleeping/offline Macs and an absent
daemon remain real availability limits. A notification grants no new remote
restart power: the silent updater and chain-assigned restart slots remain 34's
separate gate.

### 3.2 Active iPhone: light-client subscription to a follower

There are only two core behaviors: an **active light-client subscription**, or
**nothing while suspended**. A socket, background task label, or the on-chain
event itself does not give an iPhone indefinite background execution.

Proposed request, over an authenticated existing RPC/iroh connection:

```json
{"jsonrpc":"2.0","id":1,"method":"aether_subscribe",
 "params":[{"v":1,"mode":"messages","chain":"<C>","group":0,
            "after":{"height":123,"block_digest":"<verified digest>",
                     "tx_index":4,"event_index":2},
            "filter":{"system":true,"apps":[]}}]}
```

For new clients, this method keeps the response half of its QUIC stream open.
Each frame is a u32 length followed by bounded JSON: initial response,
`aether_subscription` notifications, certified-head watermarks, `gap`, and end
frames. Loopback HTTP offers the same frames as a streaming response; ordinary
one-response methods/clients remain unchanged. Old peers return `-32601`; do
not silently replace the bus with recurring `getLogs`/appcast polls. Choose
another discovered capable peer, or show that live notifications are unavailable.

The initial response reports supported modes, retained floor and head. The
server registers the stream and captures a finalized watermark **atomically**,
then replays after the cursor through that watermark and joins live delivery.
Duplicates during the transition are harmless; losing a block between replay
and live registration is not. Empty blocks advance the head so expiry and
upgrade deadlines progress without a separate timer query.

For an honest reachable peer within the subscribed delivery policy, delivery
is **at least once while the relevant history is available**, ordered by
height/transaction/event within one stream. Commit the verified cursor with
the local inbox update before notifying an action handler; handlers are
idempotent, including after a crash between delivery and action. A filtered
server watermark is a progress hint, not a proof that no omitted event exists.

Proofs may accompany frames or reference a verified/cached certificate and
`aether_getReceiptProof`; a reference alone never permits action. A full receipt
and certified transaction authenticate the requested event. The read-only
gateway currently omits `aether_getReceiptProof`; explicitly add bounded proof
and subscription capabilities where operators opt to serve them. Do not expose
faucets, signing, registration, snapshots or unrestricted node-local methods.

Proposed resource ceilings: two subscriptions/peer, 128/node, eight explicit
app filters/stream, **256 KiB/frame**, 4 MiB queued references/node, and one
in-flight proof construction/peer with eight/node. Certificates and block proof
components are shared immutable cache entries, not copied into every queue.
Large proofs use chunked bounded components: decoded block ≤existing 8 MiB,
receipt data under existing B5 limits, and ancestry ≤existing 64 links. Never
raise the ordinary RPC message limit to admit an unchecked aggregate. Streaming
verification must bound allocations/work and handle an ancestor finalized by a
later certificate; simply reusing `read_to_end` would not implement this API.

On overflow or slow readers, end with `gap` and drop the stream; never drop events
silently and pretend delivery is complete. A server's last-sent cursor is only
a hint: the client always resumes from its own durable **verified** cursor.
Reconnect with backoff/jitter through 33's DHT, known peers and independent provider paths.
One bounded catch-up after reconnect, foregrounding, or detected gap is allowed;
it is not recurring discovery polling. `eth_getLogs` can supply candidate pages
of ≤2,000 heights, but inclusion/retention proofs are separate.

If the cursor predates retained history, return **history unavailable**, not an
empty inbox. An archive may supply old certified data if reachable; current
receipt-proof RPC does not automatically reconstruct pruned receipts from era
files. A future bounded reconstruction path must replay/verify them. Otherwise
reconcile current proven balance, recovery and release state and label the
history gap. Do not show expired account/app notices as new, or let missing
history authorize an update. Current-state reconciliation uses its own certified
state inclusion proofs and existing action gates.

### 3.3 Optional iPhone APNs wake: no message content

Default **off**, post-launch and independently optional. A user may choose any
node operator willing and able to provide wakes. Over encrypted iroh, give it
an APNs device token, app topic/environment, a random lease handle and a lease
of at most 24 hours. Keep leases off chain; erase on revoke/expiry, bound them
to 4,096/node, and never give the node an account address or app subscriptions.

The least-disclosing trigger is a chain-wide activity bucket shared by every
lease: no recipient matching. A finalized transaction or system announcement
marks that bucket dirty; an opted-in node can send a coalesced wake at most
twice/hour/token. This local coalescing timer schedules OS hints; it never polls
for notifications. The APNs JSON is **only**:

```json
{"aps":{"content-available":1}}
```

Use `apns-push-type: background`, priority 5, one fixed collapse identifier and
a short expiry. Include **no address, topic, height, transaction/hash, message
ID, text, amount, URL, ciphertext or per-event collapse identifier**. These are
EastSea minimization rules. Apple documents this background wake mechanism and
does not guarantee delivery; it may throttle it. A wake only invites the app to
reconnect and fetch/verify chain data. It cannot itself display a trusted event
or authorize an update. [Apple background notifications](https://developer.apple.com/documentation/usernotifications/pushing-background-updates-to-your-app)

**Any node is eligible as an operator; arbitrary nodes cannot send APNs using
a device token alone.** Apple requires a provider credential authorized for
the installed app's topic/environment. Topic-specific keys reduce scope but
still have an Apple developer-account issuer and revocation controller. Do not
publish a universal `.p8` key, embed it in nodes, or hide a mandatory Pipln
credential issuer/gateway behind “decentralized push.” If independent operators
cannot legitimately obtain usable credentials for that installed build without
Pipln acting as the gatekeeper, leave wakes unavailable for that build. An
independently distributed build can use its own issuer; its token is not
interchangeable with another build's app topic. These constraints follow from
[Apple provider authentication](https://developer.apple.com/documentation/usernotifications/establishing-a-token-based-connection-to-apns) and
[APNs connection requirements](https://developer.apple.com/documentation/usernotifications/establishing-a-connection-to-apns).

No application message content is sent to Apple or a wake provider. That does
**not** mean no server exists in this optional path: Apple and the selected
node participate. Apple sees the device/app token, provider and wake times; the
node sees token, IP/endpoint, app presence and lease times. Generic wakes reduce
event correlation but cost battery/data and remain correlatable with public
chain activity. Multiple providers can correlate tokens and duplicate wakes.
Expose these costs before opt-in; turning it off stops lease renewal and future
wakes after revocation/expiry. OS refusal, throttling or a lost wake means the
same baseline: catch up on next foreground use. No real-time mobile-delivery SLA.

## 4. Privacy and recipient encryption

Account addresses, payment/recovery events and limit refusals are **public chain
data**. Encrypting a later notification wrapper does not conceal the original
transaction, event, revert reason, or native value. Node-local subscriptions
can keep wallet interests private from an RPC provider; blockchain observers
can still read the events.

For **app messages**, propose single-shot HPKE Base mode with
**P-256 ECDH**: DHKEM(P-256, HKDF-SHA256) `0x0010`, HKDF-SHA256 `0x0001`,
AES-128-GCM `0x0001`. Its encoded ephemeral key is 65 bytes and authentication
tag 16 bytes; `body = enc65 || ciphertext_and_tag`. HPKE derives its nonce;
do not invent a reusable sender nonce or substitute raw ECDH output for the
AEAD key. [RFC 9180, §§5–7](https://www.rfc-editor.org/rfc/rfc9180.html)

Generate a **separate recipient encryption key**, ideally
`SecureEnclave.P256.KeyAgreement.PrivateKey`. The current `EnclaveAccount` is
a signing key; its non-exportable signing handle does not imply ECDH support.
Authenticate a `KeyBindingV1` contact capsule binding C/group, recipient
address, key ID (`SHA256(public_key65)`), public key and validity heights through
the recipient's currently authorized owner, verified with account/owner proofs
or an independently exchanged contact pin. A server-supplied public key is not
enough. No on-chain public-key directory is required. Key-agreement support is
documented separately by [Apple CryptoKit](https://developer.apple.com/documentation/cryptokit/secureenclave/p256/keyagreement).

Use domain `EastSeaPushEncryption/v1`, C/group, emitter/sender/topic/recipient
and the exact 87-byte header as authenticated context/AAD. Reject substitution
of recipient, key ID, expiry or topic. HPKE Base does not authenticate a
publisher: chain provenance and wallet allowlists do that. Decrypt only after
proof, provenance, expiry and allowlist checks. Keep plaintext on the recipient,
out of provider requests, node logs, crash reports and APNs. Treat decoded
content as bounded data, never executable markup or a signing request.

Old encryption keys are kept only within a bounded user policy (at most three
keys during the seven-day delivery window); losing a device/key can make old
messages unreadable. Locked devices may defer decryption until unlock. Static
recipient-key compromise can expose archived ciphertext; this design promises
no forward secrecy, ciphertext deletion or post-quantum secrecy. Key rotation
does not delete already public metadata.

Subscription privacy modes:

- **Local node:** consume all finalized data and filter addresses/topics locally;
  no network subscription list is sent.
- **Remote light client, strongest available mode:** subscribe to complete
  finalized blocks/receipt sets, verify every receipt root, then filter locally.
  Costs bandwidth/work and still exposes the IP/iroh endpoint and timing.
- **Remote filtered mode:** request only selected system/account/app topics.
  Saves mobile bandwidth but tells the provider exactly which topics/addresses
  interest this connection. State this tradeoff; default unknown app topics
  remain muted in either mode.

Full-receipt mode can establish completeness for a covered block by checking
count/order against certified transactions and recomputing its receipt root.
An isolated matching proof in filtered mode establishes inclusion only; it
cannot prove no message was censored. No bloom or “caught up” string fixes that.
iroh encryption protects content in transit from a relay, not from the endpoint
serving public receipts; DHT/relays and providers can observe interests/IP/timing
as in 33. No subscriber address, filter, delivery acknowledgment, or APNs token
is written on chain. Read subscriptions require no wallet-key signature.

## 5. Spam and DoS

The hard resource defense is **B5**, shared with every other transaction:

```text
u <= 100,000 - d
d_next = max(0, d + u - 32)
sum(u over N finalized heights) <= 100,000 + 32*N
```

Restarting or waiting off chain never refills it. Congestion price rises after
50,000 units of debt. Keep the 2 MiB logical transaction/receipt block ceiling,
16× stored-byte bound, and the separate **8 MiB canonical-payload bucket** with
4,096 bytes/height refill. Its existing 256 KiB committee-control reserve is for
approved control fields, **not** any app/advisory log that calls itself “system.”
Consensus remains safe even when execution fees are zero or a rich Sybil pays.

Additional v1 **local policies**, without permanent quota maps:

| Layer | Initial bound / behavior |
|---|---|
| Publisher / ordinary mempool | Retain the existing 64 pending transactions/sender, 10-minute TTL and global 64 MiB budget. A simple ChainPushLog call emits one envelope; batched/dApp events still pay all retained bytes. |
| Node app delivery | At most 8 app envelopes/sender per finalized height and 64/sender per 3,600-height window; count across topics. Attribute fee payer from the certified transaction and publisher from authenticated emitter/caller provenance. Apply both caps so relaying does not hide sender volume. |
| Quota tracking | Local LRU ≤4,096 senders with expiring counters, plus global queue/work/byte budgets. LRU churn or new funded identities can weaken a sender cap but cannot evade the global budgets/B5. |
| Wallet policy | Allowlist keyed by C/group, publisher/emitter and topic; unknown apps muted by default. Following an app does not silently grant notification permission or signing permissions. |
| Wallet interruptions | At most one visible app alert/publisher per 60 seconds and 20/day; coalesce extras. Verified recovery/security notices have a separate bounded class; unverified claims never gain priority. |
| Parsing/proof work | Enforce lengths before allocation, reuse certified anchors/receipt trees, cap concurrent proof and decrypt workers, and stop slow streams. Never do attacker-driven proof/decrypt work while holding chain locks. |

Delivery caps can coalesce/suppress app notices and return an explicit policy
gap. They do **not** make an otherwise valid paid transaction invalid, mutate
fees, or prove a complete filtered range. Funds/account outcomes remain readable
through verified history/state. A chain-wide per-sender quota that rejects
blocks would be a new consensus rule; this proposal deliberately adds no such
rule or unbounded `sender -> quota` storage. Wallet allowlists are user-owned,
local, exportable and editable; no Pipln moderation/curation list is compulsory.

## 6. Verification before any message-driven action

Every accepted message carries a **committee threshold finality certificate
plus an inclusion witness**, attached or fetched before acting. Proof references
are retrieval hints. A local full node can reuse what it has already executed
and verified; a remote node's “verified” flag never substitutes for wallet checks.

1. Authenticate C/group, installed genesis/committee anchor, and valid committee
   transitions. Verify the certificate and certified canonical block digest;
   an ancestor may require the existing bounded descendant `links` proof.
2. For a receipt source, verify `aether/receipt/v1` canonical bytes through the
   indexed/count-bound BLAKE3 path to **that block's** `receipts_root`. Check
   transaction hash/index against its certified transaction, then the exact
   receipt event/output and execution outcome. Do not trust `eth_getLogs`'
   indices, block hash, `removed:false`, or summary strings without this check.
3. For `payload.upgrade`, include the canonical certified block as the inclusion
   witness: recomputing its certified digest authenticates the exact payload
   field. This is a full-block witness, **not a nonexistent payload-field Merkle
   proof**. Verify the separate upgrade signature/notice rules too. No new
   `push_root` is needed. Complete blocks similarly prove signed native tx bytes.
4. Verify class authority/provenance (§2), signed metadata, validity window,
   fresh head and the wallet's persistent height/digest floor. Reject replay
   across chains/groups, unknown code and contradictory same-height history.
   Verify historical catch-up under its historical committee, then assess
   current relevance against a fresh certified head.
5. Apply local filter/deduplication, decrypt if permitted, persist cursor/inbox,
   and only then dispatch the idempotent consumer. Absence of a proof means no
   trusted alert/action. Availability failure does not create permission.

**Finality and authorization are different checks.** A quorum signing a block
containing an attacker's text proves inclusion, not endorsement of that text.
Receipt finality is committee-authenticated, outside the current zk execution
statement (04). “Reorg-free” depends on the committee safety assumption: two
conflicting valid certificates are a consensus safety fault. Freeze
message-driven actions and signing, retain evidence, and show the fault; never
roll back the cursor silently or choose whichever RPC answers last.

### Silent updates keep the entire design-19 gate

An approved release notification schedules discovery; it never calls the
installer directly. Verify the pinned ReleaseLog runtime code and EIP-7864
entry/count storage proofs at one state height under one certified anchor.
Receipt inclusion is at H; the state after H is normally authenticated by the
next certified block's `parent_state_root` (or its supported descendant path).
Do not confuse these roots or the two blocks' timestamps.

Then verify manifest/signature hashes, distinct pinned builders **2/3** and
**72 authenticated hours** since publication (emergency **3/3**, no 72-hour
delay), downloaded archive SHA-256, and Sparkle's pinned EdDSA signature bound
to the same archive as in 19. A manifest hash identifies the manifest; **the
DMG hash must equal the manifest's archive/artifact hash**, not the manifest's
own hash. A derived `install_after_height` is a scheduling hint; height alone
cannot replace the authenticated-time check. Keep 34's canary/deadline decisions,
safe restart windows and local rollback. Missing/forged proof, an early release,
or one builder signature means keep running the current app.

System consumers also retain bounded durable high-water marks: accepted release
build/hash per supported platform and recovery nonce per owned account. An old
or equal release re-published at a newer height cannot trigger another install
or a downgrade. Expiring the notification cache never resets those marks;
34's local health rollback remains a separate decision.

Fetch approved artifacts by hash from any peer/webseed/mirror. Notifications
do not make DHT hints authoritative. New-genesis clients disable periodic
appcast discovery; legacy 7777/7780 retain the explicitly limited path in 34's
final founder decision until migrated. Do not rewrite their pinned bytes or
claim receipt-proof security where the historical block has no receipt root.
This bus also grants no way to install an iOS binary outside its existing
platform distribution rules.

## 7. Genesis impact and changes possible later

Here **genesis-required** means needed for this guarantee from block one;
these rules/pins cannot be silently changed in a client patch. **post-launch**
means this selected feature can be added without a hard fork. A later scheduled
consensus upgrade remains possible for consensus changes; calling that “just
RPC work” would be misleading.

| Item | Mark | Required decision / boundary |
|---|---|---|
| Execution receipt/event canonical fields, ordered receipt root and validator comparison | **genesis-required** | Already implemented for `node_rewards || history_v2` new genesis. Keep address/topics/data/output commitments and empty-root behavior fixed; preserve 7780 bytes. |
| B5 state fees/debt, byte/slot caps and encoded-payload debt/reserve | **genesis-required** | Already implemented. No push subsidy or debt reset. Retuning needs a versioned committee-approved consensus activation (27/G4). |
| Genesis identity/group/trust anchor and release contract/pins | **genesis-required** | Existing ReleaseLog predeployment, runtime hash and three builder keys from 19/B6; existing account implementation rules. No new founder key. |
| Explicit genesis digest / C in authenticated client discovery configuration | **post-launch** | Client trust/config convention from 33, authenticated by installed software or an independent pin; never learned from an untrusted provider or by rewriting the ceremony record. |
| PushV1 ABI, packed app envelope, topic names, advisory signature domain | **post-launch** | Ordinary log/application conventions. They change no EVM event structure, receipt encoding, or root. |
| ChainPushLog stateless emitter | **post-launch** | Ordinary fee-paid contract deployment, immutable/no admin. No reserved genesis address or new precompile; authenticate adopted runtime/emitter. |
| RPC/iroh subscription, bounded replay/queues, release watcher/status projection | **post-launch** | Node/client transport and derived local views; preserve ordinary RPC compatibility and read-gateway separation. |
| Local expiry/dedup/cursors, publisher allowlists, sender delivery limits | **post-launch** | Node/wallet policy only; no chain-wide block-invalid quotas or permanent per-sender records. |
| P-256 recipient encryption/contact capsules and optional APNs wakes | **post-launch** | Client/off-chain operator features. Wakes need independent credential feasibility and explicit opt-in; core delivery cannot depend on them. |
| New push precompile/system contract, synthesized protocol receipts, or on-chain ring/quota rule | **genesis-required** | **Not selected in v1; no allocation or new rule.** If introduced for launch, specify code/format, subsidy/price, exact capacity, overwrite/expiry and proof rules in consensus and 27 before genesis. After launch they require a scheduled consensus upgrade, unlike an ordinary emitter. |

The bounded ring in this proposal is **node-local queue memory**, not a state
tree ring. It is reconstructible from retained chain history and has no
consensus storage cost. A consensus ring would duplicate paid bytes, introduce
new deterministic writes/roots and an expiry migration problem without solving
iPhone suspension. Receipt history already provides inclusion and bounded
ordinary-node retention, so that additional mechanism is unnecessary.

## 8. Ordered lane plan and acceptance gates

Owners: **N** node/network, **L** verifier/FFI, **W** wallet, **C** ordinary
contract/tooling, **T** fault/drill, **D** docs. Paths marked “new” are proposals.
Tasks include meaningful regression/fault tests before claiming implementation.
This design lane creates only this document and runs no compilation.

| Order | Owner / deliverable | Files | Tests and fault gate |
|---:|---|---|---|
| 1 | D/L: freeze formats, sources, approval boundaries and B5 inventory | This document; future additions to `docs/design/27-state-fee.md`, `19-release-approval.md` only if implementation changes their inventory/format | Independent envelope/topic/proof vectors; check E=128/1,024/2,048 costs, calldata duplication and no per-message permanent state. |
| 2 | L: verified notification adapters and provenance; expose existing receipt proofs through FFI | `crates/light/src/lib.rs`, `block.rs`; `crates/ffi/src/lib.rs`; new `crates/light/src/push.rs` | Forged certificate/path/count/index/output/emitter; native to/value substitution; foreign chain/group; fake recovery; nested-error spoof; mempool refusal cannot pass finality verification. |
| 3 | N: dispatch only after execution + durable finalized commit; release/upgrade projections | `crates/node/src/chain.rs`, `follow.rs`, `rpc.rs`, `upgrade.rs`; new `crates/node/src/push.rs` | A finalized but unapproved ReleaseLog event stays unapproved. Receive at finality; state-proof gate waits for its anchor. Queue overflow/dispatcher failure never stalls votes. |
| 4 | N/L: framed subscriptions, atomic replay/live handoff, bounded proof serving and explicit gaps | `crates/net/src/lib.rs`, `crates/node/src/rpc.rs`, `follow.rs`, `store.rs`, `prune.rs`; new `crates/node/tests/chain_push.rs` | Disconnect during handoff, skipped/duplicate/out-of-order frames, slow consumer, invalid lengths, 128-subscription ceiling, missing certificates, ancestor links, pruned receipt, proof-worker cancellation and provider failover. |
| 5 | C/N: stateless app emitter, sender attribution and existing B5 charges | New `contracts/src/ChainPushLog.sol`, `contracts/test/ChainPushLog.t.sol`; `crates/execution/src/fees.rs`, `block.rs`, `receipt.rs` tests (retain the fee rules) | No SSTORE/admin; one envelope/call; forged sender/oversize/malformed envelope refused. LOG4 fee vectors and canonical receipts match; a rich sender/Sybil cannot exceed state or archive bucket, including after restart. |
| 6 | W/L: local inbox/filter/cursor plus P-256 contact/encryption support | New `apps/wallet/Sources/ChainPush.swift`, `PushPolicy.swift`, `PushEncryption.swift`, `apps/wallet/Tests/chain-push/main.swift`; `EnclaveKey.swift`, `NodeController.swift`; FFI bindings | Default mute; per-sender caps/topic churn; cross-network and crash/restart replay; fill >4,096 IDs then retry/change the first ID at a later height; expiry at H−1/H; wrong key/context/tag; locked/lost key; subscriber filters/tokens absent from chain and logs. |
| 7 | W/N: 34 U1–U4 chain-driven silent updates | `ReleaseUpdateGate.swift`, `ReleaseApproval.swift`, `UpdateChannel.swift`, `AetherWalletApp.swift`, `NodeController.swift`; `crates/node/src/rpc.rs`; existing release-approval tests | No recurring appcast requests. Forged system message, 1/3 signatures, early install, wrong archive hash and unavailable state proof each refuse; approved release enters 34's restart window and installs quietly. |
| 8 | T: full-chain and mobile lifecycle drill | New `scripts/chain-push-drill.sh`, `docs/ops/chain-push-drill.md`; node integration + wallet lifecycle tests | Publish payment/recovery/app/advisory/release once; compare full-node and independently verified light-client views; no action before finality, no normal reorg/retraction, contradictory certified history fails closed. Suspend iPhone: no wake promised; foreground catch-up works. |
| 9 | N/W/D: optional content-free wake adapter, only after independent credential feasibility | New `crates/node/src/push_wake.rs`, wallet APNs registration handler/tests, `docs/ops/chain-push-privacy.md` | Captured APNs payload has only `aps.content-available`; no event metadata/ciphertext. Fake/replayed/throttled/lost wake grants no action. Token rotation/revocation/24-hour expiry, lease flood, duplicate providers and battery budget. |
| 10 | T/D: founder-outage and sustained spam acceptance | Extend drill/runbook and 33's independent-provider evidence | Disable every Pipln endpoint/node while independent quorum remains. Discovery, verified release trigger and light subscription work through independent peers; remove quorum/all providers and show unavailable, never fresh invented data. |

Merge gates must cover: **forged system message, replay, expiry, spam and
reorg-free finality**. Record measured finality-to-delivery latency and bounded
memory/disk/work under saturation; do not report targets as measurements.
Use local adversarial fixtures/private drill networks for fault tests. No spam,
forged traffic or destructive fault injection against third-party production
nodes is needed.

## 9. Short threat model and red-team cases

Assets: correct payments/recovery information, update authorization, wallet
keys, user attention/privacy, consumer disk/CPU/battery, and consensus liveness.
Adversaries: app publishers/Sybils, malicious RPC/DHT/wake providers, compromised
builder/committee keys, and passive chain/transport observers. Assumptions:
authenticated initial software/anchors, safe cryptographic primitives and fewer
than the committee fault threshold compromised. If one operator already holds
a committee or builder threshold, this bus does not remove that pre-existing
control; machine/key count alone proves no operator independence. Quorum
compromise and total censorship remain outside an honest delivery guarantee.

| Red-team case | Required result / residual risk |
|---|---|
| Publish “official urgent update” from an app, or a real ReleaseLog with one builder signature | Inclusion verifies but system approval/install fails; no priority from a label. |
| Put an unsigned advisory in a quorum-finalized block | Finality is not endorsement; separate advisory authorization fails. |
| Replay after restart/chain switch, or re-include an ID after filling dedup capacity | C/group/proof/cursor and retained unexpired dedup refuse; durable build/nonce floors protect system actions. |
| Deliver expired encrypted content after reconnect or cursor reset | Verified expiry refuses a new notification; TTL does not erase archive copies. |
| Fake `Transfer`, native value, account emitter or nested limit-error bytes | Proven transaction, receipt outcome and code/provenance checks refuse the account class. |
| Flood logs, topics, sender identities, proof requests or slow subscriptions | B5/global budgets bound growth/work; local caps mute/coalesce, explicit gaps, consensus continues. Sybil delivery pressure remains possible. |
| Omit a recovery alert while advancing filtered watermarks | Inclusion alone cannot expose omission; full receipt verification/independent state reconciliation is the stronger mode. |
| Serve an old valid head or remove every independent peer | Persistent floor/freshness checks stop action; availability loss remains real. |
| Supply two conflicting valid finalized histories | Freeze action/signing and retain evidence; no silent cursor rollback or RPC vote. |
| Substitute the recipient encryption key or corrupt ciphertext | Owner/contact authentication and HPKE context/tag checks fail; static key compromise can expose retained ciphertext. |
| Send fake/duplicate APNs wakes or revoke the issuer credential | No message action from a wake; rate/battery limits and foreground catch-up. Apple/issuer availability is an optional dependency. |
| Correlate subscriptions, wake tokens, IPs and chain activity | Local filtering and generic optional wakes reduce exposure, not anonymity; public account events remain public. |
| Take Pipln offline, then claim success using stale cached data | Acceptance requires live independent quorum, proofs and delivery; cached screens alone cannot pass. |

Proofs establish what happened and who authorized it. They cannot force a node
to deliver, wake a suspended phone, erase another party's archive, or repair a
compromised initial trust anchor. Those limits are part of the protocol's
product promise.
