# Network and storage resilience

## REL-23: legacy history caches and database reclamation

Durable legacy chains (`history_v2 = false`, including the 7780 history mode) retain finalized summaries, receipts, finality certificates, account activity, and reward records in redb. Their summary/receipt memory copy has a 64 MiB ceiling; `--max-memory` can lower it. Eviction drops the oldest cached rows and keeps the finalized head summary. The cache can exceed an exceptionally small budget by the head summary's own size. Memory-only test chains cannot evict their only copy of history.

The ceiling uses the existing cache size estimate, including receipt output and event data, rather than exact allocator/RSS accounting. A restart loads only a bounded recent summary/receipt cache and rebuilds its compact MMR index using one era of hashes at a time. Archived RPC reads do not repopulate the cache. Legacy history proofs retrieve evicted hashes from redb and check complete eras against the retained MMR root. History v2 retains its existing sealed-era eviction rule.

Free-lane registration confirmations are derived notifications, rather than transaction receipt rows. When a small budget evicts their map entries, the running finalized head's bounded registration IDs still reconstruct its confirmations. Those reads neither grow the cache nor create durable orphan rows. The registry state remains the durable record; older registration notifications retain their existing ephemeral semantics.

Cache eviction performs no pruning. The `pruned_below` marker does not move, archived proof/receipt/block rows remain, and the legacy network still refuses `--history prune` because it has no era files to replace durable history. State and code are authoritative data and remain fully loaded and verified.

Stop the node cleanly before copying its database. Use the offline command on an isolated copy:

```sh
rel23_root="$(git rev-parse --show-toplevel)"
export TMPDIR="$rel23_root/tmp"
mkdir -p "$TMPDIR"
cp "$TMPDIR/isolated-devnet/node-0/state.redb" "$TMPDIR/rel23-copy.redb"
"$TMPDIR/target/debug/aether" db-maintenance --db "$TMPDIR/rel23-copy.redb"
"$TMPDIR/target/debug/aether" db-maintenance --db "$TMPDIR/rel23-copy.redb" --compact
```

The command requires an existing regular file, takes redb's exclusive lock, refuses an active node's open database and incompatible schema, and skips the startup auto-compaction that would hide a before measurement. It does not create a missing database or migrate its schema. Compaction relocates pages and truncates free tails; it deletes no logical rows. Checkpoint/root verification and logical table byte/count comparisons protect the operation before and after compaction.

The JSON report distinguishes:

| Field | Meaning |
| --- | --- |
| `file_bytes` | Logical file length, including preallocation and free pages. |
| `filesystem_allocated_bytes` | Filesystem block allocation (`st_blocks × 512` on Unix; unavailable on other targets). |
| `allocated_pages`, `page_size`, `allocated_bytes` | Pages redb marks allocated and their byte total, distinct from filesystem allocation. |
| `stored`, `metadata`, `fragmented`, `tables` | Logical row/index accounting and per-table breakdown. redb's global fragmentation also includes free-page space. |
| `reclaimable_estimate_bytes` | Upper estimate `file_bytes − allocated_bytes`, saturated at zero; includes header/allocator overhead and is not a promise of physical savings. |

Record actual reclaimed logical bytes and filesystem blocks from the before/after reports. Sparse files, APFS clones, compression, and redb allocator/header overhead can make logical differences, physical allocation changes, and free-page estimates differ. A wallet DB's logical length alone does not establish its disk cost.

Verification fixtures and reports are under worktree `tmp/`. `storage_reclamation.rs` covers legacy budget eviction with archived block/receipt/history-proof lookups and current-head derived registration confirmations, then offline compaction on a copied store with checkpoint/root/summary/receipt/certificate/reward preservation, live-lock refusal, and missing-file noncreation. Both tests were staged before production changes; exact failing/passing commands and the isolated devnet measurements belong in `tmp/net-resilience-report.md`.

Measurement results: pending the parent compile gate and isolated devnet run. No real-data app launch, `~/aether-testnet` access, or push is part of this lane.

## REL-24: corroborated follower heads and independent transports

Normal `follow`, `archive`, and the supervisor's follower child use a guarded upstream. Each polling round verifies a finality certificate at each answering source's claimed height, selects the highest verified candidate, and asks the sources to corroborate that exact block. Two distinct sources must supply valid certificates for the same canonical block. A lower agreeing pair never overrides an observed newer certificate; a replay below the local finalized height cannot roll state back. A bad or unavailable certificate is excluded. Conflicting certified blocks at the same height stop the round. Historical replay remains individually certificate-checked and cannot fetch above the corroborated head.

The iroh pool deduplicates pinned node IDs and retains up to eight connections per transport. Followers have a direct-only endpoint and a relay-only endpoint in addition to their wallet-serving endpoint. Removing the other transport makes successful replies evidence of the transport actually exercised. Cold DHT/NAT handshakes run separately, with at most sixteen warm-up tasks and the existing twenty-second attempt limit. Cached status/certificate calls have a one-second response budget and run concurrently; one decision uses at most three such rounds. Candidate proof buffers are deduplicated by source/path/block. Pool selection rotates every sixty seconds. Failed connections leave the cache and are retried on later polls.

`AETHER_FOLLOW_MIN_PEERS` sets the target number of responsive distinct sources (default 3, range 2–8). It never lowers the two-source certificate requirement. A target is best effort when peers or paths are unavailable; a corroborated head may still be replayed while the diversity alert is active. HTTP inventories are limited to eight distinct authorities, normalize aliases on one host/port to one witness, and rotate request preference every sixty seconds. HTTP URL diversity cannot establish transport or operator independence, so HTTP followers always report unverified path diversity.

An owner can supply a relay inventory through `AETHER_RELAYS`, a JSON object mapping HTTPS relay URLs to operator names. It configures the endpoints' relay map. Different URLs with the same operator name count as one operator; unconfigured relays share an unknown operator domain. Only paths that returned the candidate head count toward that head's diversity. The configuration records owner assertions, not independent operator attestation or proof of distinct ISPs. Independent relay operation must be qualified separately.

`aether_status.follower_network` exposes only aggregate fields: target/current peer counts, candidate-head path/operator counts, corroborated height, `head_confirmed`, `finality_stale`, and `alert`. The alert is active with fewer target peers, fewer than two proven paths, no corroborated head, or a head timestamp older than 120 seconds. A probe failure clears confirmation immediately; stopped probing expires it after thirty seconds. Peer IDs, addresses and relay inventory are not exposed. No native UI text or Swift source changes are included.

The new `rel24_follower_rejects_a_stale_path_and_waits_for_two_current_sources` devnet fixture runs four isolated loopback validators, stops them, and serves their actual certificates through fault-controlled RPC sources. One replays a valid stale head; another serves the current head; a second current witness starts unavailable. The follower must stay at genesis until the second current witness becomes available, then match the current certified block. Losing both current witnesses clears confirmation and cannot roll state back. This fixture validates the stale-source policy through HTTP; real direct/relay traffic, independent operators, NAT failover, and the measured peer target still need transport qualification.

## Large read-RPC responses

Successful HTTP JSON reads negotiate `Content-Encoding: zstd` with `Accept-Encoding`. Identity and compressed responses vary on that header. Compression uses the existing zstd dependency, level 1 and a 1 MiB window, for bodies from 16 KiB through 8 MiB. At most two blocking jobs run or wait; cancellation retains their permits until completion. Busy, oversized, small, or incompressible replies fall back to identity when the caller accepts it. Unsatisfiable encodings receive 406. Errors, writes, era files and encoded snapshot/shard transfers do not consume the compression budget. Canonical block and certificate bytes survive decoding exactly.

The follower's HTTP client opts into zstd and accepts identity from older servers. Decoding enforces the original response-byte limit, a 1 MiB decoder window and two blocking jobs, including through cancellation. Other clients can opt in through `Accept-Encoding: zstd`; QUIC framing is unchanged. Gzip support was not added because this lane adds no dependencies.

## Qualification and copied-devnet measurements

Seven new tests are staged against a frozen copy of the original commit `15816e4` under worktree `tmp/baseline/`. Original-code test execution is required before considering the regressions qualified. Syntax parsing and independent SDK/API review do not substitute for those runs or cargo gates.

The parent verification runner is `tmp/net-resilience-verify.py`. It runs the required compile gate before every cargo command, executes each old-code regression and checks its expected behavioral failure, runs the fixed devnet fixture, then measures read-RPC body bytes and latency before/after compaction of its stopped database copy. The fixture finalizes a padded valid deployment so the read contains a real large canonical transaction. It persists the verified certificates only on the copied DB. Offline maintenance reports logical file bytes, filesystem allocation, redb allocation and the reclaimable upper estimate; compression measurements include 25 alternating identity/zstd samples, median/P95 handler timing and exact decoded-byte equality. The runner then runs tests for `aether-node` and `aether-net` and Clippy. All artifacts stay under this worktree's `tmp/`.

Current qualification: **blocked**. The required `wait-compile.sh` gate is held by the shared release hold for `integrate-073`. The guarded attempt timed out after sixty seconds before cargo started. No new-test old-code failure, cargo pass, copied-devnet allocation figure, reclaimed-byte figure, or RPC saving has been established. The implementation is a branch handoff, not evidence that either checklist item is complete. Exact commands, static checks, remaining work and gate evidence are in `tmp/net-resilience-report.md`.
