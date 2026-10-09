# EastSea: an experiment in a public network on ordinary Macs

**DRAFT for critique — 9 October 2026**
English primary text · [한국어](eastsea.ko.md)

## Abstract

Can ordinary Macs run and prove a public network with no owner? EastSea tests
this question using a drawn Simplex BFT committee, threshold BLS certificates,
and asynchronous execution proofs generated with Jolt and Akita on Metal.
Followers check certificates; new nodes can install certified snapshots.
History is compressed into eras and can be pruned. Issuance rules
divide rewards between node participation and proving, with address-based
operator caps. The experiment is early and unproven. Its current network is
a testnet, proof coverage is incomplete, and voting admission and software updates
retain operating authorities. We describe the implementation, branch proposals,
measurements, fault assumptions, and compromises. We ask readers to run nodes,
review the code, challenge the assumptions, and contribute measurements.
Participation should not be undertaken expecting money. [R1] [R2] [R31]

## 1. The question

A public network can have public source and still depend on a small number of
operators, hosting providers, or software distributors. Our motivating concern
is that the practical cost of running and checking a network can concentrate
its operation. Consumer computers offer another place to do the work. The
repository's research examines both that possibility and the failures of
earlier consumer-device networks; it supplies no census establishing how
concentrated all public networks are. [R3] [R35]

The Mac on a desk is a possible verifier and prover, not merely a terminal
for a remote service. EastSea has generated proofs of its execution on an
M1 Max using Metal. This establishes a capability for the measured workloads;
it does not establish that an idle Mac can prove arbitrary network traffic
in real time. That distinction is part of the experiment. [R9] [R10]

We are asking whether a network operated this way can remain public without
an owner controlling its operation. We have not established that result.
Pipln currently operates admission infrastructure and participates in software
distribution. A committee supplies finality, and a client starts from a
configured committee identity. Here, “with no owner” is the question to test,
not a claim that all these dependencies have disappeared. [R1] [R19] [R21] [R31]

This draft describes source revision
`345c36a4999cb3cb9a28e5f23947ef4c2a040cf8`. It also examines the committee-scale,
storage-defaults, and storage-reward-v2 revisions listed in the source notes.
“Implemented” means present in that source or the explicitly named branch,
not demonstrated on mainnet. The public network described by the checkout is
testnet 7780; its balances do not transfer to mainnet. An old README status
table and several design sketches predate the code. Where they disagree, we
identify the boundary instead of treating the sketch as a result. [R1] [R2] [R31]

Public reading and source access coexist with permissioned voting admission.
Founder reserve keys can hold a bootstrap quorum; independent operation is an
objective, not a present guarantee. This is an infrastructure experiment.
The paper does not yet establish external application demand or show that its
use cases require a new L1 rather than an existing network or a simpler service.
[R17] [R19] [R50] [R65]

## 2. Network and execution

EastSea separates candidate Macs, seated voting nodes, followers, provers,
and history servers. A candidate is not necessarily a voter. A follower
receives certified blocks without holding a voting share. A prover generates
execution proofs after finalization. A history server supplies old data.
These roles can coexist on a Mac, but they have different responsibilities
and trust assumptions. [R4] [R7] [R9] [R14]

Nodes discover addresses through signed records on the BitTorrent Mainline
DHT and connect through iroh QUIC, using hole punching or a relay. The DHT
contains addresses, not block archives. Encryption protects the transport;
it does not hide a node's IP address from its peers. Initial peer identifiers
and the committee identity come from network configuration. Signed address
records authenticate a publisher, not an independent path to the latest
chain. A remote reader can also observe the wallet addresses requested.
Public registration and beacon records can link a node's identities and
participation schedule. [R2] [R19] [R36]

Voting nodes execute a proposed block before accepting it. The executor uses
revm, verifies the block access list and gas, and computes state updates.
Its implemented parallel path speculates against the pre-state and retries
conflicting transactions in block order. This is not the static per-transaction
dependency graph in the earlier design diagram. The state uses an EIP-7864
binary-tree layout with BLAKE3 hashing and redb persistence. Newly configured
reward/history-v2 networks also require the block's receipt commitment to
match reexecution. [R3] [R12] [R37]

The lifecycle is:

```text
transactions -> proposer execution -> committee execution and votes
                                      |
                                      v
                               finality certificate
                                /               \
                    follower replay          Mac prover
                         |                       |
                  certified snapshot       proof submission
                  and state queries              |
                                      later certified proof claim
```

The ordering in this diagram matters: finality comes before the optional
later proof claim. A finality certificate is not a ZK execution proof. [R7] [R9] [R11]

## 3. The voting committee

### 3.1 Finality

The node integrates Commonware Simplex with threshold BLS certificates.
For a roster of `n` seats it uses `f = floor((n − 1)/3)` and quorum
`q = n − f`. The familiar `2f + 1` expression is equivalent only at
`n = 3f + 1`; intermediate growth sizes need the general expression.
The intended fault model is at most `f` Byzantine seats and eventual network
synchrony. Votes must also follow the application rules and preserve their
safety journals across restarts. [R4] [R5] [R6] [R38]

Honest signers must preserve non-rollback voting history as well as their
shares. The journal guards refuse voting when local evidence indicates lost
safety state; they do not establish detection of a complete old backup restored
with its keys and markers. Refusal reduces available seats. Recovery through a
fresh sharing still requires the old quorum; permanent loss of that quorum
has no automatic recovery path under the existing trust anchor. [R6] [R30] [R63]

A dealerless distributed key-generation ceremony creates the initial group
identity. Resharing changes the holders of shares while retaining that identity.
A client can therefore check certificates with one configured group key.
The current engine uses Inline application verification, so execution and
inclusion-list checks determine the vote. Earlier Deferred-adapter descriptions
are obsolete; a testnet stall helped expose why that distinction mattered.
[R1] [R6] [R30]

### 3.2 Admission and the draw

A Mac requests registration with an operator wallet address, voting key,
node identifier, and beacon key. Pipln's registrar checks Apple DeviceCheck
and signs the registration. The on-chain registry checks that signature;
it does not independently contact Apple. Reattestation is also required.
The operating policy seeks one candidate per Mac. DeviceCheck does not prove
one person per operator address, nor cryptographically bind every future
vote to the originally checked physical Mac. Reattestation checks a registered
genuine device and possession of the voting key, not original-device identity.
Its in-memory key/token-hash rate accounting is not a persistent physical-device
identity. [R19] [R20] [R39]

For a draw, the committee signs a domain-separated message containing the
chain identifier and draw number. The eligible pool freezes from prior state
before nodes apply the verified seed. A candidate's ticket hashes the seed and voting
key. This is not the unused uptime-weighted VRF-selection sketch in the
older consensus document. Protocol 3 combines those tickets with on-chain
availability observations: greedy candidate selection improves the predicted
worst-hour probability of retaining a quorum. Ticket order breaks ties.
Thus the implemented selection is not a uniform lottery over people. A unique
threshold signature fixes the seed for that message; it does not guarantee
timely publication or prevent a threshold coalition from evaluating it early.
Succession still requires committee cooperation. [R4] [R5] [R12] [R40]

New-genesis registry-v3 eligibility includes registration age, recent beacon
stability, absence of unannounced low participation, availability at the draw
hour, and no departure announcement. Defaults use 3,600-block epochs and
24-epoch draw intervals. “Hour” and “day” here assume the nominal one-second
cadence; the actual rules count blocks. A candidate's availability profile is
a model fitted to past participation, not a guarantee of future uptime. [R5] [R18] [R19]

### 3.3 Growth and launch target

Below the configured ceiling, protocol 3 adds eligible seats rather than
performing the normal replacement draw. Each draw's addition or replacement
budget is `max(1, floor((n − 1)/3))`. At the ceiling, the quarter-pool target,
roster constraints, and ceiling govern selection. An ordinary operator's
seat cap is expressed per registration address, separately from the issuance
cap. The older “always draw a quarter of the pool” description omits the
protocol-3 growth and availability rules. [R4] [R5] [R12]

This paper adopts **16 active seats as the proposed launch performance target**,
following the committee-scale recommendation. New-genesis configuration
defaults to 16 for genesis and the ordinary draw and accepts configured values
from 4 to 128. The subsequent reserve override does not enforce that inclusive
ceiling in this revision. Before presenting 16 as a maximum for all active
rosters, the implementation must enforce it across reserve seating and handoff,
or publish measurements and a policy for larger reserve-augmented rosters.
The measurement report changes no production rule. [R5] [R12] [R29] [R31] [R32] [R41]

The measured reason is upload demand. In a regional-link simulation with
20 Mbps shared upload per validator and approximately 50 KB payloads, 16
seats had empirical p99 finality of 0.699002 seconds and no one-second misses
among 236 eligible observations. At 32 seats, p99 was 1.023128 seconds and
96 of 238 eligible observations missed that deadline. These are simulation
results under specified conditions, not a proof that 16 home Macs meet the
same envelope. Larger payloads can exceed it even with 16 seats. [R32]

### 3.4 Handoff

The old committee authorizes a handoff after resharing; the switch is scheduled
64 blocks after the handoff block. New-genesis nodes check the state-committed
roster and share-ready partial signatures. Readiness proves possession of a
usable share then, not continued online availability. The old quorum may
omit a bounded number of proposed seats that did not complete readiness,
while retaining at least four. The absence of readiness is not itself proved
on chain, so this discretion can exclude candidates. A handoff still needs
the old quorum; it cannot repair a quorum already lost. [R4] [R12] [R40]

## 4. Execution proofs on Macs

The packaged prover embeds a RISC-V guest ELF. Jolt executes that guest, and
Akita supplies the proving backend, with Metal kernels enabled by default.
The source pins both forks and their archive hashes. CPU Akita is an
alternative backend, not evidence of a hardware-neutral admission policy.
Release nodes compile in the accepted guest ELF's SHA-256 identity and compare
it with the sidecar's reported identity; unpinned development builds are
unsuitable for deployment. This is a release-local pin, not a height-indexed
program commitment. Program changes require coordinated validator acceptance,
and historical program dispatch remains unfinished. Running the packaged
sidecar requires no compiler. The fork derives public setup matrices from
seeds and contains tests comparing embedded verifier prefixes with rederived
values. These checks address setup consistency, not overall soundness. The
deployed backend has no independent audit or published deployment-specific
soundness analysis. [R10] [R13] [R42] [R44] [R45]

The guest reconstructs a state witness, checks its pre-root, executes the
transactions, and commits the block context, transaction hashes, pre/post
roots, and gas. Its public claim also binds the prover's payment address.
The pre-state is **after system writes**. Current block proofs therefore do
not establish the correctness of reward issuance, proof payouts, registrar
rotation, or committee changes. Voting nodes verify those rules by
reexecution. Receipt commitments are also checked by the committee rather
than included in the current guest statement. This is a transaction-execution
argument from an authenticated intermediate state, not a proof of full-chain
validity from genesis. [R11] [R12] [R37] [R43]

Proof work is assigned locally. A prover chooses the oldest eligible unproved
block among 32 recent finalized blocks whose required pre-state is available.
Different provers can select the same work. There is no globally coordinated
allocation in this path. Planned chunk division, resource-weighted assignment,
recursive aggregation, and checkpoint-range proofs remain research and
implementation work. The old design's reference to a chunk-market source
file does not establish a live market; that file is absent from this revision.
[R9] [R11] [R12] [R44]

The implemented whole-block market pays the first valid, unclaimed proof
against a statement recorded in state. A claim expires after the 30-day
height window; a block can include at most two proof claims, each no more
than 128 KiB. Address binding prevents another address from copying a proof
to redirect its payment. An invalid proof makes the proposal unacceptable.
A persistent verifier outage stops the affected validator for supervised
recovery, which can reduce the available quorum. Finality does not require
generating proofs for every earlier block. [R11] [R12] [R43] [R44] [R51]

“Every block is proved” is the intended coverage question, not today's
guarantee. History-v2 blocks with neither transactions nor proof claims create
no proving statement and no proof reward, even when they carry other system
work. Other statements may remain unproved because of capacity, failure,
eviction from the local work window, or expiry. Followers do not independently
reverify these execution proofs: certified replay deliberately trusts the
committee's acceptance of them. Wallet block-proof verification is not
implemented. The current prove-gas meter also omits significant guest costs;
execution admission does not prove that every admitted workload fits the
guest trace bound. [R9] [R12] [R14] [R45]

## 5. Following, checkpoints, and history

Followers verify certified block digests under their configured committee
identity and reexecute the contiguous blocks they follow. For an account
query, the light verifier checks a state proof against a certified root;
the post-state of block `H` is anchored by `parent_state_root` in certified
block `H + 1`. This authenticates the answer under committee trust. It
does not prove that the responding peer supplied the newest possible answer.
[R7] [R46]

A new or sufficiently lagged follower can request a checkpoint. It downloads
snapshot `H`, obtains certified block `H + 1`, reconstructs the state tree,
and checks the parent link, root, history MMR, metadata, code hashes and
completeness, handoff, and draw seed before installation. A manifest hash
protects download integrity; authenticity comes from the certified commitments,
not from a server hashing its own file. This skips replay before `H`; it is
a certified snapshot, not a proof of execution from genesis. Archive nodes
can retain the replay path. Replay from genesis remains possible only while
the required historical bytes are available from local storage or reachable
holders; no published retention result establishes that guarantee. [R7] [R8] [R47]

History v2 seals 8,192-block eras. Delta and dictionary encoding compress
headers, and zstd compresses bodies. Every decoded block is reconstructed
and rehashed. An era's MMR subtree root can be checked against a later
certified history root. Old eras are fetched through chunked RPC, including
the public iroh endpoint. Aggregated per-era BLS certificates and the
earlier iroh-blobs distribution sketch are not implemented. [R14] [R47]

The baseline history-v2 prune mode drops old query tables and certificates
at sealed-era boundaries after a default 30-day window, while retaining
era roots. It retains sealed era files unless told to drop them. The
storage-defaults branch makes the ordinary wallet's policy explicitly
`prune`, 30 days, and `--drop-era-files`; a full-history choice uses archive
mode. These are different disk policies. This paper recommends the branch
default for consumer Macs, without claiming it is already integrated into
the baseline. Old data must then be retrievable from another holder.
Testnet 7780 keeps its historical policy and refuses this pruning path.
[R14] [R15] [R48]

Archive sharing has two distinct forms. The existing Reed-Solomon 16-of-32
shard work has no reward weight. The storage-reward-v2 branch instead
implements a default-off, new-genesis proposal for rewarded whole-era
retrievability. An operator opts in to capacity. The epoch freezes candidates,
the catalogue, and capacities, then assigns eras to up to three distinct
operator addresses. Rewards count canonical retrievable bytes, not the
compressed file's disk size. [R14] [R16]

In that proposal, a fixed-height original-round parent-finalization beacon
after the assignment freeze chooses four distinct 4,096-byte chunks per era
(all chunks if fewer exist). A signed response supplies actual bytes
and Merkle paths before a height deadline. Successful service matures an
assignment's weight over 2,160 epochs; a miss pays zero and resets maturity.
Repeated eligible misses cause quarantine. The proposal reallocates one
eighth of the node half to archive service and leaves seven eighths for
liveness, with the same per-address caps and no extra issuance. Archive guest
source exists, but its ELF build, measured and pinned program identity, and
beacon integration gates remain pending. These rewards are **planned for activation**, not active
mainnet behavior. Sampling proves limited retrievability, not independent
replicas, local physical storage, or genesis replay; outsourcing remains
possible. [R16] [R49]

## 6. Fees, native modules, and agent accounts

The fee model separates execution, proving, and persistent growth. Execution
base fees are burned; proving fees go to escrow; tips split 60% to the
proposer, 20% to proving escrow, and 20% to burning. New-genesis state-growth
rules add a nonzero floor and a refillable growth budget. Zero execution
load therefore does not make a normal transaction free. A separate bounded
free-registration lane lets a Mac enter without a funded account. It requires
both registrar attestation and operator authorization. [R11] [R24] [R25]

The ownerless public sponsor pool is a **design proposal**. It would redirect
a fixed part of otherwise burned fees to an account used only to pay eligible
transaction fees, with device, transaction, block, and epoch limits. It would
have no administrator withdrawal key; parameters would change through protocol
upgrades. Its spend budget would be bounded by balance and previous inflow,
and sponsorship would stop when exhausted. Parameter examples are not
adopted constants. The code does not currently implement this pool. The old
proposal to fund it from unissued rewards was rejected; unissued shares must
remain unissued. [R25]

Native facilities include the delegated account, candidate registry, epoch
randomness view, and release log. “Native modules” also names Solidity
templates adapted to EastSea, including claim campaigns, grant streams, and
escrow. Those templates are contract measurement fixtures, not additional
consensus opcodes or automatically deployed services. Their terms and
participant permissions need individual review. The repository's executor
and contract tests are useful evidence; they do not certify toolbox deployment
on mainnet or an independent audit. [R12] [R26] [R50]

Agent accounts use a device-local Secure Enclave key with an owner-authorized
session. The account contract checks payment caps, allowed recipients, expiry,
and replay state. The owner changes policy with Touch ID or the login password.
Payments start disabled until a payee is approved; token permissions depend
on new-genesis account code. A deceived agent can still spend within its valid permissions.
Revocation takes effect when its transaction is finalized; it cannot undo an
already finalized payment, and a pending payment can win the ordering race.
Hardware key nonexportability does not establish the intent or safety of the software
requesting a signature. Pipln does not custody the user's account keys or
funds. Consensus identity seeds and BLS shares instead live in owner-only
files and are exportable. The Mac-binding guard is a local software control,
not a nonexportable consensus key or an on-chain hardware proof.
[R1] [R2] [R27] [R28] [R41] [R62]

## 7. Issuance and early operating authorities

### 7.1 Published issuance rules

The proposed mainnet starts with no funded genesis accounts, no premine, no
sale, and no founder allocation. Rewards accrue from block 1 under published
rules; sixteen operators are **not** a condition for issuance to begin.
Node distributions occur at epoch boundaries and proof rewards require
accepted claims, so “from block 1” does not mean every reward is transferred
in that block. The public testnet has a different funded genesis and reward
path and is not evidence of the mainnet allocation. These rules describe
genesis allocation, not evidence of equitable later distribution or economic
demand. Early participation and reserve credit can still concentrate rewards,
and no monetary value or liquidity is promised. [R1] [R17] [R29] [R51]

For `h ≥ 1`, let `I(h)` be the maximum scheduled issuance; genesis issues
nothing. In base units
the implementation computes a daily fixed-point decay:

```text
day = floor(h / 86,400)
I(h) = max(1 DBLN × D^day, 0.1 DBLN)
D = 999554841771249391 / 10^18
node(h) = floor(I(h) / 2)
proof(h) = I(h) − node(h)
```

Integer arithmetic rounds down. This approximates a 15% reduction per
365 height-days, with a continuing floor rather than a finite terminal
supply. Calendar timing depends on the block cadence. The two halves are
scheduled budgets, not unconditional minting. Empty history-v2 blocks have
no proof statement; their proof half is not issued. [R12] [R17]

For the node half, twelve unpredictable beacon slots per epoch measure
participation. Each Mac has a warm-up level `l` from 0 to 14. With `a`
answers its weight is `a × (14 + l)`, at most `F = 336`. An operator address
takes the maximum weight of its Macs, rather than summing them. For epoch
node budget `B`, address weight `w_i`, and total `W`, the payout is:

```text
p_i = floor(B × w_i / max(W, 16 × F))
```

Consequently `p_i ≤ B/16`. Missed answers and incomplete warm-up reduce
actual issuance. Sixteen registered addresses alone do not imply full
distribution; sufficient weight is needed. The simplified `min(1/N,1/16)`
description is exact only for full and equal weights. [R17] [R18]

The proof half is paid with the first accepted proof. A registered operator
can receive at most one sixteenth of the proof budget of the **payment
epoch**, even when proving older blocks. Fee escrow is paid separately and
is not subject to that issuance cap. An unregistered prover can receive
escrow but receives no new issuance through this path. Missing work,
expiry, caps, and rounding leave shares permanently unissued. They are not
a treasury balance for Pipln to distribute later. [R11] [R17]

“Operator” means an address, not a verified human or organization. One person
with multiple admitted Macs and addresses can obtain multiple caps and
count as several operators. There is no claim that the one-sixteenth rule
limits a human's total rewards or voting power. The archive-reward branch
would subdivide the node half, as described in Section 5; it does not silently
replace these baseline liveness rules. [R1] [R16] [R17] [R19]

### 7.2 Reserve keys

The founder can configure up to three reserve voting keys on one Mac.
They are not registered candidates and receive no separate proof reward.
When seated for a qualifying full epoch, their service can credit the
founder's registered Mac as having answered its slots, at that Mac's existing
warm-up level. The founder still counts as one operator under the node-half
cap; the credit neither adds an operator nor increases warm-up. It reflects
seating, not proof that each reserve key actually voted. Unnecessary reserve
service credit stops after the documented grace epochs. [R17] [R18]

Older operational notes say reserves fill missing seats below four independent
operators and leave when four qualify. The implementation counts distinct
registration addresses; independent human control is not established.
The current policy is broader: it also permits their return when modeled
worst-hour quorum availability falls
below 0.99, retaining current seating in the 0.99–0.995 band and allowing
departure at 0.995 or above. These are model thresholds, not measurements
of independence. The keys are correlated on one Mac. Three reserves in a
four-seat committee can hold the entire three-share quorum. Reward caps
do not reduce this control. [R5] [R17] [R52]

### 7.3 Registrar, distribution, and updates

The registrar is needed today because consensus cannot directly establish
the intended Mac admission policy from an Apple response. Its signature
authorizes registration and reattestation. It cannot, by that key alone,
create a committee certificate, choose an arbitrary draw result, transfer
an account balance, or change issuance rules. It can deny attestations,
and a compromised service can feed admitted candidates over time. The
registration rate cap limits speed, not eventual control of admission.
Apple or registrar refusal can prevent existing candidates from supplying
required reattestations after their grace, affecting accepted liveness beacons
and future draw eligibility. It does not erase their registrations or directly
stop current committee votes. [R5] [R18] [R19] [R20] [R39]

The committee can rotate or zero the registrar through an authorized upgrade.
Normal notice is 604,800 blocks. The emergency path requires the threshold
certificate plus `n − f` current-member Ed25519 approvals and one epoch's
notice. These mechanisms depend on a functioning quorum. Secure Enclave
registrar signing is an available launch configuration; disk-file signing
also exists for development. Source support is not proof of a particular
production key deployment. The registrar's unattended signer protects key
nonexportability, while attestation policy remains in its caller. Production
controller independence, signer isolation and rotation/recovery drills have
not been established by these source checks. [R20] [R21] [R54] [R61]

Distribution uses Pipln's Developer ID, notarization, and Sparkle, because
users need an installable and maintainable Mac application. The new-genesis
release gate adds a pinned append-only ReleaseLog and three builder keys:
ordinary releases need two signatures and 72 certified hours; emergency
releases need all three and omit the wait. Publishing a log entry is
permissionless and is not approval. The app verifies log state and code,
archive hash, and the release signatures. The pins come from the installed app,
so first installation remains a distribution trust assumption. Without usable
certified network state, the automatic update gate defers installation; an
emergency manual replacement needs independent artifact authentication.
The legacy testnet update path remains Sparkle-only, despite a design proposing
the stronger gate there.
[R21] [R22] [R53]

App approval and consensus-rule approval are distinct. Pipln's distribution
key alone cannot satisfy the new-genesis builder gate. Sufficient release
signers can nevertheless distribute replacement wallet and bundled-node
code, including future changes to client trust rules. That is a software
supply-chain authority, not harmless metadata. Distinct keys do not prove
independent human controllers. Under the current cooperating-client rules, the
registrar alone cannot authorize a protocol upgrade. Declared upgrades require
committee approval and matching software; unsupported clients refuse the
activated protocol. This does not prove software semantics or prevent a buggy
same-version release from causing disagreement. Insufficiently coordinated
installation can also remove the voting quorum at activation. Keeping an older
binary does not guarantee continued compatibility. [R21] [R22] [R54]

The founder's position is **no forced sunset** for registrar or update keys.
They exist for operating functions the current system needs. Their replacement
is technically possible; this paper promises neither their removal nor a
date. “No premine” does not mean the founder cannot receive ordinary work
rewards. “No custody” does not mean that admission and update authority
are absent. [R1] [R2] [R31]

## 8. Security analysis

For `k` compromised seats in a committee of `n`, the following are implications
of the threshold model, not measured attacks. Assume correct honest voting,
durable journals, sound cryptography, and the intended share-holding epoch.
[R4] [R6] [R38]

| Compromised seats | What the model permits or excludes |
|---|---|
| `k ≤ f` | The adversary alone cannot form a quorum certificate. Safety is intended to hold; eventual progress still needs enough honest online seats and delivery. |
| `f < k < q` | Withholding can leave fewer than `q` responsive seats and stop finality. The BFT safety guarantee no longer applies; this is not equivalent to unilateral certificate forgery. |
| `k ≥ q` | The adversary has enough shares to produce threshold signatures without honest participation. Certificate-only clients cannot infer honest execution from them. |

At 16 seats, `f = 5` and `q = 11`. The deterministic fault bound counts
simultaneously faulty seats, not people. Shared power, connectivity, operator
control, Apple service, software, and reserve hardware create correlated
failure. Ordinary-seat quorum probabilities use an independence model;
reserves are modeled as one correlated Mac. There is no measured general
correlation model for ordinary operators. Correlated failures can exceed
the fault bound. [R5] [R32]

Resharing retains the committee identity; it does not cryptographically revoke
retained shares from an earlier sharing. Safety also assumes that an adversary
cannot obtain a usable quorum from any former sharing. Clients checking only
the stable group key cannot identify which sharing produced a signature. The
supervisor replaces active shares and attempts to remove old generation files,
but these operations do not establish destruction of backup or snapshot copies.
Historical-share retirement and recovery require independent review.
[R4] [R6] [R40] [R63]

For a light client, certificates and state proofs prevent an untrusted
read server from inventing a different answer under the trusted root,
assuming the primitives and verifier are correct. Certificates alone cannot
establish the newest available state, discover every fork, make withheld data
available, or prevent eclipse. Clients also enforce timestamp-age and
previously verified height checks; these constrain stale responses without
proving absolute freshness. The public read gateway labels general results unverified; some
balance views have local proof checks. Public peer lists are an availability
aid, not a replacement for a trust anchor. A fresh client offered conflicting
certified histories has no independent first-history oracle. [R21] [R36] [R46] [R55] [R58]

A verified execution proof can add assurance that the committed transaction
transition follows the pinned guest, even if committee execution is suspect.
It does not validate the system writes outside that statement, select the
canonical history, assure timely inclusion, or recover missing data. This
assurance applies only where a proof exists and is independently checked;
Followers and wallets currently trust committee certification of proof
acceptance; they do not cryptographically verify block proofs themselves.
The word “ZK” also does not make public transactions private. We make no
end-to-end post-quantum claim for a network using BLS and conventional
account signatures. [R9] [R11] [R43] [R46]

## 9. Measurements and missing measurements

### 9.1 Committee finality

The committee report is dated 2026-10-09 and pinned to `9e033c8`. It runs
the actual Engine, Simplex, BLS, and execution paths in a simulated network
on poc-m3: Apple M3, 24 GiB, macOS 15.6, Rust 1.98.1, `nice 15`.
Storage is in memory. Disk persistence, physical TCP/iroh transport, and
proving are excluded. Network latency and upload limits are input assumptions.
CPU demand is modeled using physical BLS calibration; simulation CPU
percentages are not observed operating-system utilisation. [R32] [R33]

Below, latency is nominal **one-way** delay: LAN is 1 ms with 0.2 ms jitter;
50 and 150 ms profiles have 5 ms jitter. All use 0.5% loss, 100 Mbps
shared upload per validator, nominally empty payloads, no followers or faults,
current batching, two seeds, and only three virtual seconds with no warm-up.
Values are seconds. Quantiles are nearest-rank, pooled over **completed**
proposals, from CPU-ready proposal to quorum application commits. [R32]

| Seats | Delay | p50 | Empirical p99 | Completed | One-second misses / eligible |
|---:|---|---:|---:|---:|---:|
| 4 | LAN | 0.006461 | 0.009000 | 4 | 0/4 |
| 4 | 50 ms | 0.156440 | 0.163429 | 4 | 0/4 |
| 4 | 150 ms | 0.458415 | 0.463429 | 4 | 0/4 |
| 16 | LAN | 0.010367 | 0.010423 | 3 | 1/4 |
| 16 | 50 ms | 0.166120 | 0.175530 | 3 | 1/4 |
| 16 | 150 ms | 0.465222 | 0.472247 | 3 | 1/4 |
| 32 | LAN | 0.014596 | 0.014838 | 4 | 0/4 |
| 32 | 50 ms | 0.172226 | 0.216809 | 4 | 0/4 |
| 32 | 150 ms | 0.471919 | 0.487718 | 3 | 1/4 |
| 64 | LAN | 0.019382 | 0.019498 | 3 | 1/4 |
| 64 | 50 ms | 0.173842 | 0.175195 | 3 | 1/4 |
| 64 | 150 ms | 0.473708 | 0.474839 | 3 | 1/4 |
| 128 | LAN | 0.031678 | 0.032732 | 4 | 0/4 |
| 128 | 50 ms | 0.188434 | 0.191381 | 3 | 1/4 |
| 128 | 150 ms | 0.488124 | 0.492317 | 3 | 1/4 |

Source for all rows: [R32]. Three or four completions cannot estimate a
population p99. Unfinished observations are censored in the quantiles,
which is why subsecond completed quantiles can coexist with deadline misses.
Here “censored” is statistical, unrelated to transaction censorship.

The stronger boundary run uses the regional-link matrix, 20 Mbps shared
upload, a 50 KB payload target, direct propagation, no followers or faults, current batching,
three seconds' warm-up, 120 virtual seconds, and two seeds. [R32] [R34]

| Seats | p50 s | Empirical p99 s | Maximum s | Completed | One-second misses / eligible |
|---:|---:|---:|---:|---:|---:|
| 16 | 0.671025 | 0.699002 | 0.700559 | 236 | 0/236 |
| 32 | 0.997110 | 1.023128 | 1.023916 | 236 | 96/238 |

Source: [R32] [R34]. These conditional observations justify testing a 16-active-seat
performance target in this envelope. They do not establish long-run failure
rates or physical deployment performance. Two longer 128-seat probes reached
the harness's 1,200-second wall deadline without results; these are measurement
failures, not evidence of a consensus stall. [R32]

### 9.2 Proving

The sidecar report records **2026-09-27, M1 Max, Metal, machine load average
25–75**. The sample workloads are P-256 payments, not arbitrary contract blocks. [R10]

| Workload | Condition | Proving time | Proof bytes as reported |
|---|---|---:|---:|
| 10 transactions, padded trace `2^24` | Cold process | 28.6–31.7 s | 97.9 kB |
| 50 transactions, padded trace `2^26` | Cold process | 71.0–81.2 s | 98.0 kB |
| 50 transactions | Subsequent `serve` request | 47.2 s | 98.0 kB workload report |

Warm verification took 0.23–0.26 seconds for the ten-transaction sample and
0.33 seconds for fifty. These are recorded timings, not today's guaranteed
latency on every Mac. We do not combine the older approximately 270
transactions/hour design note with these later workloads as if they were
one benchmark. [R9] [R10]

Separately, the founder reports version 0.7.1 using approximately 400% CPU
and at most 5% GPU utilisation. The brief supplies no hardware, workload,
sampling interval, raw samples, or run date. We preserve this as a reported
operational observation, dated by its 2026-10-09 report, **not a reproducible
benchmark**. It motivates profiling the still CPU-heavy proving path;
utilisation alone does not measure the fraction of cryptographic work on
the GPU. [R31]

Resource safeguards do not establish a sustained supported-Mac budget. The
resource module records a **2026-09-29** incident on a **64 GB Mac**: the prover
at **14 GB and 350% CPU**, reported system swap **17.5/18.4 GB**, load average
approximately **1000**, and colocated validators slowing to approximately
**0.45 blocks/s**. These are source-recorded incident values, not controlled
telemetry or a current-release benchmark. Candidate-specific memory, swap,
wall-power, thermal and foreground-impact measurements remain missing. [R64]

### 9.3 Disk and checkpoint sync

The history report's **2026-09-28 status section** preserves an 8,192-block
release-codec experiment with four rotating leaders and
approximately one-second timestamps with jitter records 1.2 bytes/block
for an empty history-v2 era and 136 bytes/block for an era with one transfer
per block. Its run date and hardware are not specified in that table.
Those numbers measure era encoding, not total node storage. Annualized
figures elsewhere in the report are extrapolations and are not included
here as measurements. [R14]

The storage-defaults record, committed **2026-10-09**, materializes 2,608,385
synthetic encoded empty blocks and follower proofs using production codecs,
era-batched writes, and pruning. [R15]

| Fixture state | Allocated physical bytes | Logical file length |
|---|---:|---:|
| Complete initial run | 2,137,657,344 | 2,550,317,058 |
| After turnover, prune, and reopen | 2,144,964,608 | 2,550,317,383 |

Source: [R15]. Hardware, OS, and exact run timestamp are not recorded;
the date is the record date. The test does not execute EVM transactions
or sign/verify consensus certificates. It excludes transaction/state growth,
optional archive shares, and existing immutable archives. Physical allocation
and logical length differ. This is not a general two-gigabyte node claim.

A 2026-10-07 upgrade dry run records an empty follower jumping from height
0 to 496,591 and continuing to follow. It gives **no elapsed time to the
first block**. A functional checkpoint test also has a timeout, not a latency
measurement. Checkpoint-to-first-block timing on a named Mac, network path,
snapshot size, and cold/warm condition remains an explicit missing result.
[R7] [R8] [R56]

## 10. Compromises we made, and why

The final column states proposed tests that could justify reconsideration,
or an explicit lack of a removal plan. Those tests are criteria for the
experiment, not measured results or delivery commitments. We think these
are workable trades today; readers should test the stated reasons.

| The ideal | What we chose | Why | Cost or risk | What would change it |
|---|---|---|---|---|
| Hardware neutrality | Apple Silicon Macs and Metal as the supported admission/proving path | One hardware/key platform and working measured proof workloads reduce the initial implementation surface. [R9] [R10] [R19] | Excludes other owners; correlated hardware and vendor failures. | A non-Apple path passes the same execution/proof vectors and publishes comparable memory and proving measurements, plus an admission model. |
| Distribution independent of a vendor | Developer ID/notarization and Sparkle on Mac; Apple distribution on iPhone; ServiceManagement for unattended operation | Installable software, device keys, and approved background restart use existing platform facilities. [R53] [R57] | Apple policy or service changes can affect installation, updates, and admission. | No replacement promised; a tested installation/restart/admission path must preserve the required key and operating guarantees. |
| Anyone can vote immediately | Drawn committee with a proposed 16-active-seat performance target; inclusive reserve ceiling remains unfinished | The 20 Mbps, 50 KB boundary simulation separates sixteen from thirty-two under the one-second goal. [R32] | Few seats, selection barriers, and correlations limit the claim of open validation. | Repeated physical-Mac runs with larger committees meet the same payload/upload conditions and deadline, including loss and unavailable-seat cases. |
| No exceptional voting identities | Up to three founder reserve keys | A small committee can lack sufficient available seats; the implemented policy uses correlated reserves only when the model predicts a useful improvement. [R5] [R52] | One founder Mac can control a bootstrap quorum; modeled need can recur. | Sustained measured availability avoids reseating under the existing model; no permanent disappearance is promised. |
| Permissionless admission and no publisher authority | Pipln registrar plus distribution and builder-release keys | DeviceCheck needs an off-chain signer; client installation and repair need an authenticated release path. [R19] [R20] [R21] | Admission denial, false admission, and harmful approved software. | **No forced sunset or removal plan.** Reviewable alternatives may be proposed; losing these functions without replacement is not a transition. |
| Issuance only after a broad operator set exists | Published budgets from block 1, each address capped at 1/16 | The recorded decision rejects waiting for sixteen operators; unallocated budgets remain unissued. [R17] [R31] | Early operators still receive issuance; addresses do not prove human diversity. | No change planned: expose actual allocations and independence limits rather than claim the cap removes the trade. |
| One immutable network history from first test | Testnet resets and new genesis rehearsals | Testnet and mainnet have different genesis rules; recovery and migration need disposable trials. [R2] [R29] [R30] | Resetting can hide accumulated failures and prevents treating test balances/history as durable. | Mainnet starts only with published genesis and completed gates; resets must remain disclosed, not represented as continuous mainnet operation. |
| Independent minimal clients | Native Swift UI with Rust verification through UniFFI and a bundled Rust node | Reuses one execution/verifier implementation while integrating device-local signing and operation. [R58] | Common implementation defects affect clients and nodes; pure Swift UI is not protocol diversity. | An independently implemented verifier passes shared vectors and adversarial differential tests. |
| Fully independent light-client discovery and freshness | Public read gateway, candidate announcements, peer lists, and a pinned committee identity | Browsers and small clients need reachable readers; some remote transports remain incomplete. [R36] [R46] [R55] | Stale answers, eclipse, metadata exposure, and unverified general views. | Independent sources and client tests establish freshness/fork handling and proof coverage for each displayed fact. |
| Proving dominated by the GPU | Metal-enabled proving with substantial CPU work | Measured proof generation works, but selected kernels are accelerated and the reported utilisation remains low. [R10] [R31] [R42] | CPU heat, contention, low sustained proving capacity. | Published per-stage profiles and whole-block timings show reduced CPU work and better throughput without worse verification or memory use. |
| Every participant retains all history | Thirty-day prune default, era commitments, optional archives | Storage-defaults measures bounded empty-history retention; consumer storage is finite. [R14] [R15] | Old data depends on willing holders; state itself still grows. | Retrieval tests across independently operated archives demonstrate retention and recovery; larger measured budgets may support longer local windows. |
| The user always pays every fee | Free registration and a planned public sponsor pool | A new Mac can register without a funded account; bounded fee sponsorship is intended to reduce onboarding friction. [R25] | Protocol subsidy creates spam and exhaustion surfaces; the pool is unimplemented. | Activate only after fee-accounting, resource-cap, and depletion tests pass with published constants; otherwise retain the present fee path. |

These choices do not cancel one another's costs. A reward cap does not
solve quorum concentration; a hardware key does not make an updater benign;
an execution proof does not create data availability. Proposed alternatives
must be assessed at those boundaries. The registrar and update keys are
designs of necessity in the founder's current position, without an automatic
sunset. [R17] [R21] [R31] [R43]

## 11. Known weaknesses and open problems

The supported validator path is Mac-only. Apple supplies the platform,
attestation service, key APIs, and distribution facilities. DeviceCheck
and wallet addresses do not prove operator independence. The committee's
home-upload limit is established only in simulation, and neither the draw's
availability model nor the reserve exception removes common-mode failure.
These are open limits, not solved decentralization claims. This is a scope
decision, not evidence that Macs are more efficient or decentralized than
other hardware. A matched comparison has not been published. [R5] [R19] [R32] [R53]

The prover remains too slow to cover arbitrary traffic at the block cadence.
Metal support should not be read as GPU dominance. Proof scheduling lacks
a durable complete backfill guarantee, wallets do not independently check
block proofs, and the current cost meter does not bound every accepted
workload's proving cost. Certified snapshots help entry but do not establish
full-history validity or an independently measured first-block latency.
[R9] [R31] [R44] [R45] [R56]

Pruning bounds selected history, not permanent state. The whole state is
reconstructed in memory, while redb allocation and retained code, rewards,
registry records, archives, and snapshots have separate costs. The planned
retrieval rewards still permit outsourcing and shared control; their guest
and activation gates remain unfinished. Storage-envelope calculations must
not be presented as observed disk use under application load. [R8] [R15] [R16] [R24]

Testnet operation has exposed implementation failures. On **2026-09-28**, the
network stopped at height 69,651 when local inclusion-list observations
entered certification decisions and split the four validators' votes. Inline
verification and voting after durable storage addressed that path. On
**2026-09-29**, it stopped at 104,408 after inclusion-list ordering skipped
earlier sender nonces; accumulating vote-journal sections also exhausted
the restart environment's descriptor limit. Ordering, buffering, descriptor
limits, and monitoring were corrected. Finite disks and descriptors still
limit prolonged stalled operation. [R6] [R30]

A proving-input layout error, present from **2026-09-30** and found on
**2026-10-06**, prevented transaction-bearing proving inputs from decoding.
Monitoring failed to expose it for days. The node now encodes the full
layout and round-trips it before submission. **2026-10-07** build-identity
work also addressed guest metadata that could change the program identity
across builds. Pinned inputs, deterministic metadata, and packaging checks
reduce that risk; guest changes still require coordination. The recorded
cross-Mac result concerns guest identity under matching build inputs and stage
configuration. Independent reproduction of the current distributed application
and archive is not established here; signing envelopes, Xcode/SDK selection
and extension compiler inputs retain separate boundaries. These corrections
are source and rehearsal evidence, not proof that all future blocks or
releases work. Testnet recovery and resets are disclosed separately from
the proposed mainnet's irreversible history. [R30] [R59] [R60]

There has been no independent security audit. Repository reviews and tests
do not substitute for one. Unfinished assurance includes the Jolt/Akita/Metal
backend, proof-cost admission, full reshare/share-retirement analysis,
production key independence and ceremony evidence, supply-chain and
ServiceManagement installation, and archive-reward guest bounds. The legal
research is internal analysis, not legal clearance. We invite critique of
these specific boundaries, rather than a general declaration that the
software is safe. [R2] [R9] [R16] [R23] [R54] [R61]

## 12. Conclusion and participation

EastSea has tried committee finality, on-device verification, asynchronous
Mac proving, certified snapshot entry, and compressed history. The reported
measurements support further experiments under bounded conditions. They
do not establish a network without controlling authorities, proof coverage
of every block, or performance on ordinary home deployments. Those are
questions for participants to test. [R7] [R9] [R14] [R31] [R32]

Try the local devnet in a disposable directory and report what breaks.
It resets that directory and starts local validators; use the same directory
for the stop command. It does not register a public-network voter. Public
operator trials should use a specifically qualified release with its
published genesis/reset notice, known issues, resource limits and human
support route; this paper certifies none of those gates. Review the consensus
and proving assumptions, inspect the code and paper, and send measurements
from your own hardware. Include revision, Mac/RAM, OS, workload, network
conditions, timing definition and unsuccessful runs. Report non-sensitive
results through the issue tracker. The experiment is early and unproven.
Participate to test the system, not expecting money. [R1] [R2] [R31] [R65]

## Source files

References to local files describe the baseline revision in Section 1.
Cross-branch references use immutable commit URLs. A source document's estimate
or proposed rule remains an estimate or proposal even when cited. Protocol
constants are source observations at the dated
2026-10-09 baseline, reviewed on 9–10 October; measurement dates and unknown
run conditions are stated separately. The [dated critique](critique-2026-10-09.md)
contains the numerical provenance ledger and proposed evidence work.

- **R1** — [README](../../README.md): participation, current network, planned issuance; its dated status table is not authoritative over newer code.
- **R2** — [DISCLAIMER](../../DISCLAIMER.md): network status, no sale/premine/founder allocation, noncustody, no independent audit.
- **R3** — [Design overview](../design/00-overview.md): choices, goals, and corrections to early proving assumptions.
- **R4** — [Consensus design](../design/07-consensus.md): DKG, committee selection, handoff, admitted limitations.
- **R5** — [Rotation implementation](../../crates/node/src/rotation.rs): eligibility, tickets, growth, quorum model, reserve fallback.
- **R6** — [Consensus engine](../../crates/node/src/engine.rs): Simplex adapter, persistent voting and journal guards.
- **R7** — [Follower implementation](../../crates/node/src/follow.rs): certificate checking, replay, checkpoint entry.
- **R8** — [Snapshot implementation](../../crates/node/src/snapshot.rs): certified snapshot checks.
- **R9** — [Proving design](../design/06-proving.md): asynchronous operation and unimplemented chunk/client paths.
- **R10** — [Prover README](../../apps/prover/README.md): setup and dated M1 Max timings.
- **R11** — [Protocol 2](../design/13-protocol-2.md): statements, proof claims, payout and scope.
- **R12** — [Chain implementation](../../crates/node/src/chain.rs): system writes, proof claims, receipt commitments, roster and statement rules.
- **R13** — [Jolt/Akita fork lock](../../scripts/jolt-fork.lock): exact revisions and archive hashes.
- **R14** — [History-compression research](../research/history-compression-2026.md): implemented eras, pruning, codec results and unimplemented sketches.
- **R15** — [Storage-defaults history report at c5a0d01](https://github.com/eastsea-xyz/eastsea/blob/c5a0d018b0d120dd74ca9cb285a28f1b8405bf61/docs/research/history-compression-2026.md#L160-L224): defaults, synthetic physical/logical disk results and exclusions.
- **R16** — [Archive-reward design at 2c519e6](https://github.com/eastsea-xyz/eastsea/blob/2c519e6bdd1aee09a919a1baf59f65a291c982c4/docs/design/44-archive-rewards.md): proposed activation, funding, canonical content and sampled retrieval.
- **R17** — [Reward implementation](../../crates/rewards/src/lib.rs): issuance, weights, caps, reserve service, and unissued balances.
- **R18** — [Beacon implementation](../../crates/rewards/src/beacons.rs): slots, warm-up inputs, availability profiles.
- **R19** — [Registration design](../design/14-registration.md): admission policy, address/device limits and reattestation.
- **R20** — [Registrar operations](../ops/registrar.md): signer choices, rotation and revocation.
- **R21** — [Release-approval design](../design/19-release-approval.md): app gates, protocol upgrades, trust limits.
- **R22** — [ReleaseApproval.swift](../../apps/wallet/Sources/ReleaseApproval.swift) and [ReleaseUpdateGate.swift](../../apps/wallet/Sources/ReleaseUpdateGate.swift): implemented release and legacy paths.
- **R23** — [Legal research](../research/legal-review-2026.md) and [open legal questions](../ops/legal-open-questions.md): wording limits and unresolved review; not external clearance.
- **R24** — [Persistent-growth fee inventory](../design/27-state-fee.md): state/archive charging and explicitly calculated envelopes.
- **R25** — [Gas-pool design](../design/22-gas-pool.md): implemented free registration, superseded free-transaction promise, unimplemented sponsor pool.
- **R26** — [Native template fixture provenance](../../crates/contracts-onchain/fixtures/native/README.md): claims, grants and escrow templates.
- **R27** — [EastSeaAccount contract](../../contracts/src/EastSeaAccount.sol): owner, session, recipient, token, recovery and replay rules.
- **R28** — [Agent-wallet instructions](../../agents/skills/aether-wallet/SKILL.md): local key and payment-policy boundaries.
- **R29** — [Mainnet launch runbook](../ops/mainnet-launch.md) and [genesis script](../../scripts/mainnet-genesis.sh): proposed ceremony, gates and ceiling checks, not evidence of a completed launch.
- **R30** — [Consensus-recovery runbook](../ops/consensus-recovery.md): dated stalls, fixes, journal and recovery limits.
- **R31** — [Source notes](source-notes.md): exact branch scope and attributed founder decisions/observation, not independently measured evidence.
- **R32** — [Committee-scale report at 9e033c8](https://github.com/eastsea-xyz/eastsea/blob/9e033c8263f43d5cec01bbfad5ba6815974bd620/docs/research/committee-scale-2026-10-09.md): conditions, finality tables, censoring, and launch recommendation.
- **R33** — [Committee-scale host record](https://github.com/eastsea-xyz/eastsea/blob/9e033c8263f43d5cec01bbfad5ba6815974bd620/docs/research/committee-scale-2026-10-09/verification/host.json).
- **R34** — [Boundary-run inputs](https://github.com/eastsea-xyz/eastsea/blob/9e033c8263f43d5cec01bbfad5ba6815974bd620/docs/research/committee-scale-2026-10-09/raw/boundary-long/run-inputs.json) and [boundary summary](https://github.com/eastsea-xyz/eastsea/blob/9e033c8263f43d5cec01bbfad5ba6815974bd620/docs/research/committee-scale-2026-10-09/boundary-summary.json).
- **R35** — [Consumer-device landscape research](../research/device-node-landscape-2026.md): motivation and editorial cautions; its external-project claims are not adopted as measurements here.
- **R36** — [Network design](../design/08-network.md): discovery and remote-reader limitations.
- **R37** — [Execution design](../design/04-execution.md): implemented speculative path, receipt boundary, prove-gas gaps; earlier diagrams are sketches.
- **R38** — [Consensus research](../research/nextgen-consensus.md): intended fault and synchrony model.
- **R39** — [DeviceCheck implementation](../../crates/node/src/devicecheck.rs): what is actually checked and signed.
- **R40** — [Handoff implementation](../../crates/node/src/handoff.rs): seeds, readiness, omission and reshare commitments.
- **R41** — [Roster/genesis implementation](../../crates/node/src/roster.rs): ordinary/genesis committee cap and defaults.
- **R42** — [Prover features](../../apps/prover/Cargo.toml): Metal and CPU Akita backend selection.
- **R43** — [Block-proof statement](../../crates/proving/src/block.rs): public statement and guest transition boundary.
- **R44** — [Prover service](../../crates/node/src/prover.rs): local attempts, retry limits and sidecar verification.
- **R45** — [Precompile/proving review](../research/precompiles-proving-2026-10-06.md): missing cost coverage and trace limits.
- **R46** — [Light verifier](../../crates/light/src/lib.rs): configured identity, certified state roots and account verification.
- **R47** — [Era format](../../crates/node/src/era.rs) and [era RPC transport](../../crates/node/src/era_net.rs).
- **R48** — [Prune implementation](../../crates/node/src/prune.rs): retained window, era-file option and legacy restriction.
- **R49** — [Archive-prover README at 2c519e6](https://github.com/eastsea-xyz/eastsea/blob/2c519e6bdd1aee09a919a1baf59f65a291c982c4/apps/archive-prover/README.md): unbuilt/unpinned guest and unfinished gates.
- **R50** — [Contracts-onchain report](../research/contracts-onchain-2026-10-06.md): real-executor fixture scope and exclusions.
- **R51** — [Node main/genesis allocation](../../crates/node/src/main.rs): `chain_config`, funded test genesis versus empty mainnet allocation.
- **R52** — [Reserve operations](../ops/reserve-keys.md): bootstrap concentration and recovery; its simplified removal description is qualified by current rotation code.
- **R53** — [Apple-platform design](../design/36-apple-platform.md): distribution and platform dependencies; additional integrations are proposals.
- **R54** — [Post-launch change boundaries](../design/30-post-launch-fixability.md) and [feature checklist](../03-analysis/feature-checklist-2026-10-05.md): authority and outstanding production evidence.
- **R55** — [Public read-gateway runbook](../ops/read-gateway.md): read-only service and per-view verification status.
- **R56** — [2026-10-07 upgrade dry run](../ops/testnet-7780-upgrade.md) and [checkpoint functional test](../../crates/node/tests/devnet.rs): successful follow without a recorded first-block elapsed time.
- **R57** — [Unattended daemon](../../apps/wallet/Sources/UnattendedDaemon.swift): ServiceManagement and local opt-in.
- **R58** — [Wallet design](../design/09-wallet.md): Swift UI, UniFFI verification and bundled-node boundaries.
- **R59** — [Proving-input codec](../../crates/node/src/prover_input.rs): dated layout defect and native round-trip check.
- **R60** — [Reproducible-build runbook](../ops/reproducible-builds.md): guest build identity, dated fixes and tested scope.
- **R61** — [Audit 7](../research/audit-7-2026-10-06.md): source-review scope and excluded independent assurance.
- **R62** — [Mac-binding guard](../../crates/node/src/key_binding.rs), [agent keys](../../apps/agent/Sources/Keys.swift), and [owner authorization](../../apps/agent/Sources/Owner.swift): local binding, key-role and user-presence boundaries.
- **R63** — [Supervisor/share retirement](../../crates/node/src/supervisor.rs) and [atomic replacement](../../crates/node/src/atomic.rs): recovery and historical-copy limits.
- **R64** — [Resource safeguards](../../crates/node/src/resources.rs): dated 2026-09-29 incident and monitoring; not a current resource benchmark.
- **R65** — [Community readiness review](../research/community-2026-10-09.md): actual-release/reset/support qualification remains separate from source and demo evidence.

[R1]: ../../README.md
[R2]: ../../DISCLAIMER.md
[R3]: ../design/00-overview.md
[R4]: ../design/07-consensus.md
[R5]: ../../crates/node/src/rotation.rs
[R6]: ../../crates/node/src/engine.rs
[R7]: ../../crates/node/src/follow.rs
[R8]: ../../crates/node/src/snapshot.rs
[R9]: ../design/06-proving.md
[R10]: ../../apps/prover/README.md
[R11]: ../design/13-protocol-2.md
[R12]: ../../crates/node/src/chain.rs
[R13]: ../../scripts/jolt-fork.lock
[R14]: ../research/history-compression-2026.md
[R15]: https://github.com/eastsea-xyz/eastsea/blob/c5a0d018b0d120dd74ca9cb285a28f1b8405bf61/docs/research/history-compression-2026.md#L160-L224
[R16]: https://github.com/eastsea-xyz/eastsea/blob/2c519e6bdd1aee09a919a1baf59f65a291c982c4/docs/design/44-archive-rewards.md
[R17]: ../../crates/rewards/src/lib.rs
[R18]: ../../crates/rewards/src/beacons.rs
[R19]: ../design/14-registration.md
[R20]: ../ops/registrar.md
[R21]: ../design/19-release-approval.md
[R22]: ../../apps/wallet/Sources/ReleaseApproval.swift
[R23]: ../research/legal-review-2026.md
[R24]: ../design/27-state-fee.md
[R25]: ../design/22-gas-pool.md
[R26]: ../../crates/contracts-onchain/fixtures/native/README.md
[R27]: ../../contracts/src/EastSeaAccount.sol
[R28]: ../../agents/skills/aether-wallet/SKILL.md
[R29]: ../ops/mainnet-launch.md
[R30]: ../ops/consensus-recovery.md
[R31]: source-notes.md
[R32]: https://github.com/eastsea-xyz/eastsea/blob/9e033c8263f43d5cec01bbfad5ba6815974bd620/docs/research/committee-scale-2026-10-09.md
[R33]: https://github.com/eastsea-xyz/eastsea/blob/9e033c8263f43d5cec01bbfad5ba6815974bd620/docs/research/committee-scale-2026-10-09/verification/host.json
[R34]: https://github.com/eastsea-xyz/eastsea/blob/9e033c8263f43d5cec01bbfad5ba6815974bd620/docs/research/committee-scale-2026-10-09/raw/boundary-long/run-inputs.json
[R35]: ../research/device-node-landscape-2026.md
[R36]: ../design/08-network.md
[R37]: ../design/04-execution.md
[R38]: ../research/nextgen-consensus.md
[R39]: ../../crates/node/src/devicecheck.rs
[R40]: ../../crates/node/src/handoff.rs
[R41]: ../../crates/node/src/roster.rs
[R42]: ../../apps/prover/Cargo.toml
[R43]: ../../crates/proving/src/block.rs
[R44]: ../../crates/node/src/prover.rs
[R45]: ../research/precompiles-proving-2026-10-06.md
[R46]: ../../crates/light/src/lib.rs
[R47]: ../../crates/node/src/era.rs
[R48]: ../../crates/node/src/prune.rs
[R49]: https://github.com/eastsea-xyz/eastsea/blob/2c519e6bdd1aee09a919a1baf59f65a291c982c4/apps/archive-prover/README.md
[R50]: ../research/contracts-onchain-2026-10-06.md
[R51]: ../../crates/node/src/main.rs
[R52]: ../ops/reserve-keys.md
[R53]: ../design/36-apple-platform.md
[R54]: ../design/30-post-launch-fixability.md
[R55]: ../ops/read-gateway.md
[R56]: ../ops/testnet-7780-upgrade.md
[R57]: ../../apps/wallet/Sources/UnattendedDaemon.swift
[R58]: ../design/09-wallet.md
[R59]: ../../crates/node/src/prover_input.rs
[R60]: ../ops/reproducible-builds.md
[R61]: ../research/audit-7-2026-10-06.md
[R62]: ../../crates/node/src/key_binding.rs
[R63]: ../../crates/node/src/supervisor.rs
[R64]: ../../crates/node/src/resources.rs
[R65]: ../research/community-2026-10-09.md
