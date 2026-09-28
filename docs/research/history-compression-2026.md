# Chain history: compress it, verify it piecewise (2026-09-27)

History grows every second (1 s blocks, a BLS certificate each, optional
~95 kB validity proofs). Goal: keep as little as possible on every Mac, and
let anyone check any piece, or the whole, cheaply. Sources: the two research
notes of this date (history storage/distribution; succinct chain proofs).

## What each layer needs

| Layer | Grows as | Compress by | Verify by |
|---|---|---|---|
| Finality | 1 certificate per block | nothing to do: the identity is fixed, so one certificate on the tip covers the chain | 1 BLS check (48-96 B) |
| History (which blocks) | 1 hash per block | a Merkle Mountain Range: 64 peaks on each node, root in every header | ~25-hash inclusion path (~1-1.7 kB) against a certified header |
| Block data | txs + headers | era files of 8192 blocks: columnar headers, zstd, **one aggregated BLS signature per era** | era root in the MMR; the aggregate checks with 2 pairings |
| Execution validity | one proof per block | never archive per-block proofs (3 PB/yr); fold them into a chain proof | one ~100 kB proof, ~0.3 s |
| State | current tree only | checkpoint snapshots as BLAKE3 blobs | bao streaming check + state root in a certified header |

Aggregation works because every certificate is under the same committee key:
σ_era = Σ σ_i verifies as e(σ_era, g) = e(Σ H(m_i), PK). One 96 B signature
replaces 786 kB per era.

## Design

1. **`history_root` in every block**: the BLAKE3 MMR root of all earlier
   block hashes (leaf = H(height ‖ hash), keyed BLAKE3 like the state tree, so
   it is one Jolt inline per hash in proofs). A certificate on block N then
   proves every block before it. Nodes keep only the peaks.
2. **Era files** (8192 blocks ≈ 2 h 17 min): headers stored as columns
   (drop recomputable parent hashes, delta heights and times, dedup unchanged
   state roots), bodies zstd-compressed, the era's aggregated signature, an
   index. The era's MMR sub-root identifies it.
3. **Distribution over iroh-blobs**: content-addressed by BLAKE3 with verified
   range streaming (bao), the transport the nodes already run; a monthly
   torrent bundle as a public mirror (BitTorrent, as with peer discovery).
   Later, Reed-Solomon shards (commonware-coding, e.g. 16 of 32) so each Mac
   keeps a slice of old eras.
4. **Checkpoint sync** (new Macs stop re-executing from genesis): trust only
   the pinned committee identity → latest certificate → header (state root,
   history root, snapshot hash) → state snapshot blob, bao-verified while it
   streams → follow live. Re-execution from genesis becomes an audit mode.
   The trust is the same as the wallet's (the committee), as on Sui or Aptos.
5. **Chain proof** (execution validity of all history in one proof):
   - near term: validators verify each epoch's block proofs natively and the
     committee signs an epoch checkpoint (R_start → R_end), anchored in the
     next header; a wallet checks the tip's block proof itself;
   - mid term: Jolt-in-Jolt recursion with Akita (upstream drafts verify a
     Jolt proof in ~87M guest cycles): per-epoch recursive step, constant
     ~100 kB proof, ~0.3 s to verify, post-quantum and transparent end to end,
     running on the same Metal prover. No Groth16 wrap for the wallet path
     (trusted setup, loses post-quantum security).
   - watch: lattice folding (LatticeFold+, Neo/SuperNeo) could make each step
     far cheaper than full recursion.

## Storage per Mac per year (estimates)

| Load | Normal Mac (7 days + its shards) | Archive (all eras) |
|---|---|---|
| Mostly empty blocks | < 1 GB | 0.5-1 GB |
| 10 tx/s | 2-4 GB | 20-30 GB |
| 100 tx/s | 15-25 GB | 200-300 GB |

## Security notes

- Resharing keeps the identity, so an old committee's shares could sign an
  alternate history: leaving members erase their shares (done, audit round 5);
  handoff records stay on chain.
- The MMR and the snapshot hash are only as good as the certificate on the
  header that carries them: nothing here adds trust beyond the committee.

## Status (2026-09-28, roadmap B1-B3)

**B1: why `state.redb` grew per block.** Measured on a copy of the 7780
testnet store at height 69,651 (`cargo run -p aether-node --example
store_stats -- <copy>`):

| | bytes/block |
|---|---|
| file (logical) | 1,553 |
| of which pages freed by copy-on-write, never returned | ~620 (43 MB) |
| block summaries (JSON) | 587 |
| receipts (JSON, per tx block) | 123 averaged |
| state entries (mostly proof-market records) | 46 |

Besides `state.redb`, marshal's block archive holds ~530 B/block and its
finalization archive ~260 B/block (B4 prunes both once eras exist).

Fixed without consensus changes (7780 included): summaries are written
packed (postcard, links to the previous row elided; JSON rows still load):
539 → 110 B/block; a store more than a quarter free is compacted at
start-up (the testnet copy: 108 → 65 MB). Not fixable on 7780 without a
consensus change: every protocol-2 block writes its statement record into
the state tree (~68 B/block stored, also held in memory) and the record is
pruned only after the 30-day claim window, so the state holds ~2.6M records
at steady state (~180 MB).

**B2: history root.** Already in every 7780 block since genesis (BLAKE3 MMR,
leaf = H('L' ‖ height ‖ hash), checked by every node). Added: nodes keep era
roots plus the open era instead of every leaf (`mmr::EraIndex`), history
proofs read at most two eras (`mmr::prove_by_eras`; before, each
`aether_historyProof` rehashed the whole history), an era's root proves as
an MMR node (`MmrProof::verify_node`), and the light client verifies an old
block's full bytes (`verify_old_block`) and whole eras (`verify_era_root`).

**B3: era files** (`crates/node/src/era.rs`): 8,192 blocks from a multiple of
8,192; heights, parent hashes and history roots recomputed; epochs, views,
timestamps, versions delta-encoded; leaders as a dictionary; state roots and
metadata hashes stored only when they change; bodies JSON; all zstd-19; the
era's MMR sub-root is its identity and every read rebuilds each block and
checks it. Any bit flip is detected (test).

**History v2** (`network.json` `"history": 2`, bound to the genesis hash,
off on 7780): an empty block records no proof-market statement (nothing to
prove), so empty blocks leave state root and metadata unchanged; nodes seal
each era into `eras/era-NNNNNNNN.aera`. Trade-off to confirm before the
mainnet genesis: nobody earns proving issuance for empty blocks.

Measured with `tests/history.rs` (`measure`, release, 8,192 blocks, 4
rotating leaders, ~1 s blocks with jitter), bytes per block:

| run | summary before (JSON) | summary after (packed) | state | era file |
|---|---|---|---|---|
| 7780 rules, empty | 539 | 110 | 68 | 65 |
| 7780 rules, 1 transfer/block | 611 | 144 | 132 | 136 |
| history v2, empty | 539 | 110 | 4 | **1.2** |
| history v2, 1 transfer/block | 611 | 144 | 132 | 136 |

Empty blocks archived under history v2: 1.2 B × 31.5M = **~38 MB/year**
(target < 1 GB). Under 7780 rules they cannot go below ~64 B (two hashes
that change every block): ~2 GB/year. A transfer costs ~136 B archived, close
to its signature and key alone.

Not done yet: the era's aggregated BLS signature (replaces 8,192
certificates). A v2 node also keeps the open era's blocks in `state.redb`
(bounded: one era, reused after sealing).

**B4: pruning** (`crates/node/src/prune.rs`, `archive.rs`, `era_net.rs`).
`--history archive|prune` (default prune on history v2, archive otherwise:
7780 unchanged, and `prune` is refused there since it has no era files),
`--retain-days` (30), `--drop-era-files`. Every minute a node drops whole
eras that are sealed (no kept blocks waiting for their file) and older than
the window: marshal's blocks and certificates (a new node keeps them in
prunable archives with one section per era; a node that already has
immutable archives keeps them and prunes only its store), block summaries,
receipts and a follower's finality proofs, one redb transaction per era. The
roots of dropped eras stay in the store, and at start-up the history index is
rebuilt from them and checked against the committed history MMR. History
proofs of pruned heights read the era file. Old eras travel over the node's
JSON-RPC (`aether_eraInfo`, `aether_eraChunk`, `aether_eraProof`; loopback
HTTP and the public iroh endpoint), not iroh-blobs, which is not a
dependency: the fetcher re-hashes every block and checks the era root against
a history root it already trusts (`aether_light::verify_era_root`), so the
transport carries no trust. A follower whose upstream pruned a height replays
that era from its file, anchored on a later certified block.

Measured (tests/prune.rs, empty v2 blocks): a kept block costs 448 B of
block codec in marshal plus its certificate (~150-260 B) and a 110 B summary,
so 30 days of 1 s blocks is ~2 GB and 7 days ~0.5 GB whatever the history's
length; a pruned block costs 0.2-1.2 B of era file plus 0.004 B of root.

**B5: shards** (`crates/node/src/shards.rs`, library only, no reward weight):
commonware-coding Reed-Solomon over BLAKE3, 32 shards of which any 16 restore
an era file (~2.1x its size in total), assigned by rendezvous hashing on a
committee-signed draw seed. Restores are checked by the era root, so the
shard commitment need not be on chain yet; beacon shard proofs come later.

## Order

1. MMR + `history_root` in blocks (a genesis change: ship with the next
   testnet reset, together with the BLAKE3 state hash).
2. Checkpoint sync with state snapshots (the biggest win for new Macs).
3. Era files + aggregated signatures + iroh-blobs serving, pruning to 7 days.
4. Epoch checkpoints over block proofs; then recursive chain proofs.
5. Reed-Solomon sharding of old eras.
