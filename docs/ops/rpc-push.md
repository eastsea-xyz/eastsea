# Node and wallet RPC push

The node serves JSON-RPC WebSockets on `GET /` and `GET /ws` on the existing
RPC listener. Ordinary `POST /` calls keep their existing behavior. Finalized
commits publish a bounded height reference; subscription filtering, encoding
and socket writes run outside consensus execution.

## Ethereum subscriptions

```json
{"jsonrpc":"2.0","id":1,"method":"eth_subscribe","params":["newHeads"]}
{"jsonrpc":"2.0","id":2,"method":"eth_subscribe","params":["logs",{"address":["0x0000000000000000000000000000000000007705"],"topics":[null]}]}
{"jsonrpc":"2.0","id":3,"method":"eth_unsubscribe","params":["0x2"]}
```

Subscription IDs belong to one connection. Unsubscribe returns `true` once
and `false` for unknown IDs, including another connection's ID. Notifications
use `eth_subscription`; logs retain the unfiltered block's transaction and
log indexes and have `removed: false`. Only finalized blocks are delivered.
Address filters support one address or up to 64 addresses; up to four
positional topic filters support exact matches, OR arrays of at most 64
alternatives, and `null` wildcards. Historical range keys are refused on a
live log subscription.

## Wallet topics

On its own node, the wallet opens one private subscription:

```json
{"jsonrpc":"2.0","id":1,"method":"aether_subscribe","params":["wallet",{"address":"0x0000000000000000000000000000000000000001","transactions":[],"after":123}]}
```

The response's hex ID identifies subsequent `aether_subscription` frames:

```json
{"jsonrpc":"2.0","method":"aether_subscription","params":{"subscription":"0x2","result":{"kind":"wallet","height":124,"observed_height":124,"topics":["head","balance","tx_status","release"]}}}
```

Every finalized height carries a head watermark, including empty blocks.
Balance/nonce changes and candidate token transfers, changes to up to 32 explicitly tracked transaction
statuses, finalized upgrade notices/schedules and `Published` logs from the
ReleaseLog address mark the corresponding topic dirty. An initial snapshot
and bounded replay mark all topics for reconciliation. These frames schedule
the wallet's existing verified reads and release checks. A topic does not
provide a balance proof, receipt proof or release approval.

With `after`, retained heights are replayed in order through the captured
watermark before live frames. Registration and finality publication share the
chain lock, and live delivery skips heights covered by that replay. A cursor
in the future, below the node's retained cache/history floor, or more than
1,024 heights behind produces `kind: "gap", reason: "history_unavailable"`
after the subscription acknowledgement, then closes the stream. Missing
individual retained heights also produce a gap. The wallet resumes only from
its verifier's height bounded by its received watermark, resets context on
account/network changes, and reconciles on a gap or reconnect.

The wallet disables recurring balance/receipt/appcast discovery while the
stream is healthy. A dropped, rejected or unresponsive socket enables fallback
with exponential backoff and jitter capped at 60 seconds. Transport pings and
cached-state pause observation continue; existing node health/watchdog reads
also retain their safety role. A known release waiting for its approval time
is revisited on head delivery through the existing approval and install gates.

## Resource policy and qualification

- Two subscriptions per connection, 128 per node; 128 connections, including
  closing writers. Slots release when the connection task and writer finish.
- 256 KiB per incoming/outgoing frame; 32 queued frames per client plus one
  in flight. Node references, including the 64-height finality ring, total at
  most 1,024; queued/in-flight payload leases plus ring heights fit 4 MiB.
- Overflow evicts the connection retaining the largest backlog. Frames keep
  their memory charge until destruction, including a socket write already
  in flight. Socket writes have a two-second deadline. A gap and close signal
  the need to reconcile; an unreadable socket may observe only the close.
- The public read-only gateway permits bounded Ethereum subscriptions and
  retains its ordinary method gates. Wallet subscriptions require the private
  node listener.

`crates/node/tests/devnet.rs` adds isolated four-validator qualification for
heads/unsubscribe/limits, filtered receipt logs, and wallet replay/gaps plus a
threshold-signed future upgrade announcement. The first test also exercises
deterministic queue eviction, exact in-flight accounting, connection lifetime
and broadcast lag. Pure Swift suites cover the wire contract, retry policy
and wallet wiring. Per-run commands, red/green evidence, poll measurements and
remaining qualification items live in the requested `tmp/rpc-push-report.md`.

This selected own-node wallet transport implements the local push slice of
design 35 §3. General `PushV1` app/advisory delivery, proof-bearing framed QUIC
streams, encrypted notifications and optional suspended-iPhone APNs wakes
remain separate work.
