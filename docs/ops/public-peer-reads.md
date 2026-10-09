# Public peer reads

The explorer, the site's packaged explorer, and extension reads try
`http://127.0.0.1:18545` first, then a rotating public peer pool. There is no
default HTTP gateway. Settings still accepts an operator's own gateway.
Transaction submission remains on the wallet's existing native/local path;
`eastsea/read/1` never relays writes.

## Operator settings

Wallet Settings exposes public read sharing and its daily data allowance.
The same policy can be set in a follower or validator's data directory:

```json
{"enabled":true,"daily_bytes":268435456}
```

The file is `public-read.json`. Missing means enabled with a 256 MiB daily
allowance; malformed, unreadable, or unknown fields disable sharing. Settings
replaces it atomically, and connected readers see changes on their next
request. `public-read-usage.json` reserves accepted input and outgoing JSON
before serving them, including refusal responses. It survives restart and
resets only when the UTC day moves forward. Failed/interrupted output can
remain conservatively charged. This is an application-byte allowance;
QUIC, TLS, and relay overhead are outside its accounting.

Read streams are bounded to 64 KiB requests, 16 MiB responses, 64 concurrent
requests globally, four per peer, and a 16-request burst refilling at eight
requests/second per authenticated iroh identity. The daily allowance is shared
with public legacy follower RPC so a different ALPN cannot bypass it. A
validator's roster-authenticated native RPC remains available for consensus
and catch-up. Loopback RPC retains its existing behavior.

## Discovery and trust

Each node retains iroh's unsalted pkarr address record. A separate signed
ContactV1 item advertises the read role under the chain/group salt from design
33. The network fingerprint binds chain ID, committee identity, and genesis;
its persisted sequence cannot silently roll back. Contact verification checks
the derived target, key, salt, signature, network, size, expiry, and sequence.

Browser seeds include the bundled validator node IDs and
`public-read-peers.json`. Replaceable pkarr HTTP bridges resolve these known
keys into signed addresses. A bounded `aether_readPeers` exchange introduces
more configured peers and registry-admitted followers. Admitted followers
carry operator hints, with distinct operators sampled before duplicates; the pool prefers
diverse operator/relay hints and keeps at least three verified peers when
three are reachable. Hints select routes and never change network trust.
Salted contact publication does not provide public-topic enumeration: the
BEP-5 torrent/TCP introducer proposed in design 33 remains separate work.

The WASM transport uses iroh 1.2.0 over WebSockets and n0 public relays by
default. The explorer's relay setting and shared reader options replace that
list; Pipln operates no required relay or gateway. The default pkarr bridges
are `pkarr.pubky.app`, `pkarr.pubky.org`, and `relay.pkarr.org`.

The bundled light verifier authenticates finality and returns decoded block
fields. Remote summaries must match those certified fields. Accounts verify
Merkle proofs against the certified child block's parent state root; receipts
verify their committed inclusion proof and requested transaction hash.
Unverifiable peers are dropped. Live head floors survive storage when available
and remain monotonic in memory when it is unavailable. Historical blocks
remain readable without a live-head freshness requirement.

Admission checks each peer's own previously certified head and freshness;
a slightly lagging honest peer remains available for history. Live head and
current account answers must meet the shared verified-height floor before
display. Concurrent head refreshes share one request, and secondary errors
from a closed connection cannot extend a temporary peer quarantine.

Browser admission respects the service's four concurrent streams per peer and
the WASM transport's 32-call limit. A JavaScript timeout keeps its permit until
the underlying transport call settles, and queued reads retain their deadlines.

Execution-only node metrics and unsupported contract calls are unavailable
through peers. Presence returns an explicit unavailable aggregate until the
live-peers implementation is integrated. Ordinary web loading still trusts
the delivered page and its bundled verifier/network pins; a certificate is
not an independent authentication of the website that delivered those pins.

## Packaging and release seeds

`scripts/build-extension.sh` builds one verifier/transport artifact for both
apps. `scripts/package-public-reader.sh` copies the canonical peer reader and
seeds to the extension and packages the explorer under `site/explorer/`.
These WASM/site files are generated artifacts, excluded from git.

Store ZIP and site releases run `node scripts/refresh-read-seeds.mjs` before
packaging. This bounded refresh queries independent pkarr HTTP bridges,
checks requested-key signatures and locator age, and preserves the previous
release if fewer than three signed locators resolve. It changes discovery
hints only, never the pinned committee. Run it manually when preparing a
release; upgrading public providers is still an operator responsibility.

## Local test and measurement

Build the node, its `public_read_network` and `public_read_forged` examples,
the extension WASM, and the pinned upstream iroh-relay test binary through
the lane's compile gate. `node scripts/p2p-read-e2e.mjs` then starts three
actual devnet validators with a local relay, blocks HTTP RPC and non-test
hosts in headless Chrome, measures cold verified-head/block-page times,
rejects an actual iroh peer with a forged header, and stops every process in
`finally`. Logs, screenshots, measurements, and stop evidence live under
`tmp/p2p-read/`. It never launches the wallet app or builds a proving guest.
First certificate timing uses the navigation's monotonic performance clock.
Direct block-page timings launch a separate fresh browser, with empty caches;
warm navigation and full home-page rendering are recorded separately.
The verified head paints before the historical block table finishes; polls
share an outstanding history request and display only certified rows.

Measured on this Mac with three local devnet peers and a local relay,
2026-10-09, three fresh-browser samples: median first verified certificate
3.08 seconds, full home page 17.31 seconds, direct cold block page 15.65 seconds,
warm block navigation 1.36 seconds. The WASM is 3,763,973 bytes (1,508,464 gzip).
These are local-relay observations, not public Internet relay measurements.

For local-only tests, `AETHER_IROH_RELAY_URL=http://127.0.0.1:<port>/` explicitly
replaces native relays and `AETHER_IROH_NO_DHT=1` disables public DHT activity.
The production default remains public relays and signed DHT discovery.
