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

## Order

1. MMR + `history_root` in blocks (a genesis change: ship with the next
   testnet reset, together with the BLAKE3 state hash).
2. Checkpoint sync with state snapshots (the biggest win for new Macs).
3. Era files + aggregated signatures + iroh-blobs serving, pruning to 7 days.
4. Epoch checkpoints over block proofs; then recursive chain proofs.
5. Reed-Solomon sharding of old eras.
