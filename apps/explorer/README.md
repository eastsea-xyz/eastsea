# EastSea Explorer

> 한국어 요약: 정적 파일 블록 익스플로러. 이 Mac의 노드
> (`127.0.0.1:18545`)를 먼저 읽고, 연결되지 않으면 공개 노드 배열에서
> iroh WASM과 WebSocket 릴레이로 읽는다. 공개 노드의 응답은 번들에 고정된
> 네트워크 신원, BLS 최종성 인증서와 Merkle 증명을 검증한 뒤 표시한다.
> 체인 읽기에는 기본 HTTP 게이트웨이가 없다. Settings에 개인 게이트웨이와 릴레이 주소를 설정할 수 있다.

Static files, no framework or server-side API. Reads try your own node first,
then a rotating pool of public nodes over `eastsea/read/1`. The browser keeps
at least three certified peers when three are available, rotates peers, and
closes slow peers or peers that serve invalid certificates or display fields.
Pipln runs no gateway or relay for this path. No analytics or prices.

The pool starts from node IDs in bundled `network.json` and release hints in
`public-read-peers.json`. The WASM transport resolves known IDs through signed
pkarr HTTP packets. A certified peer can return more address hints with
`aether_readPeers`; every discovered peer must supply its own valid certificate.
Configured operator and relay hints guide diversity and carry no chain trust.

## Run

```bash
# Package the wallet/light verifier and browser iroh transport first:
scripts/build-extension.sh
cd apps/explorer
python3 -m http.server 8090
# open http://localhost:8090
```

Any static server can host the explorer. Include `wasm/`, `network.json`,
`public-read-peers.json`, `token-sources.json`, the JS modules and styles.
A deployment without the verifier cannot read public peers.

Settings starts with `http://127.0.0.1:18545` and an empty gateway field.
Chrome may ask for local network access; Safari may block an HTTP loopback
request from an HTTPS page. Verified peers continue the read when loopback
is unavailable. An explicitly configured personal HTTP gateway remains a
fallback and is labeled unverified. The WebSocket relay list can be replaced;
an empty list selects iroh's n0 public relays.

## Pages

| Page | What it shows |
|---|---|
| Home | certified finalized height paints before coalesced verified history; local-node reads also show committee, scheduled protocol, mempool, base fees, prover status and anonymous Live network observations; unsupported public-peer fields stay unavailable |
| Live network (`#/network`) | privacy-safe continent totals on a draggable WebGL globe, opted-in countries only at k ≥ 3, accessible list, static reduced-motion map; public gateway presence only, separate from account/peer reads |
| Block | every header field the RPC serves, neighbor links, this node's prover view of the block's proof, the transactions with their receipts (a pruned block shows what the era record still carries) |
| Transaction | receipt (status, gas, contract creation, output), events decoded as ERC-20 `Transfer`/`Approval` with symbol and amount, raw logs for anything else |
| Account | balance/nonce/code with a committee-certificate badge (`verified by committee certificate` only when the wallet's own wasm check passed — `js/verify.js`), token detection, latest rewards (`aether_rewards`), ERC-20 transfers to/from the address in the node's log window |
| Token | name/symbol/decimals/total supply, the origin badge and impersonation warning exactly as the wallet shows them, recent transfers |
| Search | apps via `aether_search`; canonical `.sea` name pages keep indexed results beside the wallet handoff; height, `0x`-address, or tx hash keeps its direct lookup, and a hash with no receipt is matched against the newest block hashes |

Enter an app title, description, or `.sea` name in the header search bar, or open
`#/search/harbor.sea`; existing search bookmarks remain valid. Name and action
links offer a wallet handoff, with payments approved in the wallet. The index
reads `aether_search` and `aether_searchInfo` from a configured HTTP node or
personal gateway and preserves the node's neutral order. Public peers have no
supported index proof and do not supply these records. The page shows
lookalike warnings and incomplete index or usage coverage. A content-hash label
means only that a nonzero hash was recorded, not safety or certificate verification.
Search UI strings are in English, Korean, Japanese, Simplified Chinese, and Spanish.
The node builds its index from chain records without a central search server or
runtime manifest downloads; see [the search design](../../docs/design/app-search.md).

## Verified data

Public reads construct displayed header fields from certified block bytes,
compare RPC summaries with those bytes, and verify receipt and account Merkle
proofs. A wrong answer closes that peer and another peer is tried. The head
uses a persistent verified-height floor and a freshness check; the session
retains its floor even when browser storage is denied. Historical block and
receipt checks verify inclusion without requiring the old block to be fresh.

Verification badges belong to the exact object that passed verification.
A JSON field named `verified` and a concurrent change of source cannot grant
a badge to an HTTP answer.

The certified header commits to its **parent state root**. An unproved current
state root, base fees, prover escrow and code size are omitted from public
reads. Mempool, prover activity and presence are uncommitted and unavailable.
Token contract calls, log scans, rewards and registry summaries need your own
node: the public client does not display responses without a supported proof.
Receipt proofs for legacy blocks without a receipt commitment are unavailable.
The explorer signs nothing and relays no transactions.

The wasm and `network.json` (the pinned committees) are build products here;
without them the account page says `not verified`, never pretends.

The **Live network** route polls `aether_presence` at the configured public
presence source while visible. This separate presence-only transport retains
`https://rpc.eastsea.xyz` as its default and uses a personal gateway override
when configured. That host is never a default source for certified chain reads
or transaction submission. Presence is an unverified, frozen cohort observation,
separate from consensus. The privacy producer serves thresholded schema-2
counts; the integration adapts that aggregate contract to the globe. Missing
geography and quality evidence must remain unavailable, and withheld counts
must not become zero. Fixture views are explicitly labeled and are never an
automatic fallback for failed live reads. Shared globe assets are bundled
locally, without map, tracker or geolocation requests.

## CORS and the node's endpoint

`crates/node/src/rpc.rs` serves JSON-RPC on `POST /` with
`Access-Control-Allow-Origin: *` (methods POST/OPTIONS, header `Content-Type`),
bound to loopback. In practice:

- A page served from **any origin** — `localhost:8090`, another port, a hosted
  copy — may read a node on the user's own machine, which is exactly what this
  explorer does.
- The node **binds loopback only**. To explore a node on another machine you
  need a tunnel or proxy (e.g. `ssh -L 18545:127.0.0.1:18545`) and to point
  Settings at it.
- The explorer only ever sends reads (`aether_*` queries, `eth_call`,
  `eth_getLogs`, `eth_blockNumber`). It never sends a transaction; the one
  write-ish RPC a node has (`aether_sendTransaction`) is never called.

## RPC methods used

`aether_status`, `aether_recentBlocks`, `aether_getBlock`, `aether_getReceipt`,
`aether_getAccount`, `aether_candidates`, `aether_proverStatus`, `aether_presence`,
`aether_rewards`, `aether_history`, `aether_search`, `aether_searchInfo`, `eth_call`, `eth_getLogs`,
`eth_blockNumber`, and `aether_getFinalized` (the account page's certificate
check, `js/verify.js`). The public reader also uses `aether_getBlockProof`,
`aether_getReceiptProof` and `aether_readPeers`. Node-side notes are in
`crates/node/src/rpc.rs` and `docs/ops/public-peer-reads.md`.

Two windows to know about: `eth_getLogs` scans at most the newest 2,000
finalized blocks (token/account transfer lists are labeled with that), and
block summaries are served for heights this node kept — older ones come back
as era records, and heights below what it ever kept simply don't. The public
gateway caps these the same way and refuses everything else — its allowlist
and caps are in `docs/ops/read-gateway.md`.

## Tests and measurements

```bash
cd apps/explorer
npm test
node test/live.mjs http://127.0.0.1:18545  # optional local-node page smoke
```

The unit tests cover the pure helpers: ABI word parsing and `Transfer`/
`Approval` decoding, revert-reason decoding, amount/time formatting, the token
metadata and origin scans (against a mock reader), the badge rules, the search
classifier and resolver, the live presence shape and unavailable states,
continent breakdowns and ten-second polling, and the JSON-RPC client (injected
`fetch`, endpoint persistence, error and timeout paths). `test/live.mjs` is a manual smoke test
in a minimal DOM stub — it is deliberately not part of `npm test`.
The offline suite also covers peer-only reads, three-peer maintenance, discovery,
diversity hints, wrong-header and slow-peer removal, field sanitization,
verified-height replay protection, optional gateway persistence, and existing
decoding/formatting/search behavior. The lane's devnet/headless browser runner
adds real certificate and WebSocket transport checks.

`window.aetherReadDiagnostics()` reports the current source, peer IDs and
protocol-level head/block read timing. It sends no telemetry. Full cold-page
latencies and WASM sizes are recorded by the browser/devnet measurement runner.

Globe tests cover country folding/duplicate buckets, sanitized output, stable
session jitter, safe request envelopes, bundle drift, local land geometry and
the 300 KB gzipped JS budget. Real-browser offline smoke (from the repo root):

```bash
mkdir -p tmp
# Uses existing Playwright tooling and installed Chrome; never a live node.
TMPDIR="$(git rev-parse --show-toplevel)/tmp" PLAYWRIGHT_MODULE=/path/to/playwright node scripts/test-live-globe.mjs
```

The runner intercepts the public RPC with fixtures, verifies both deployments
in light/dark and mobile, reduced motion, language switching,
unavailable/empty/stale states, drag/keyboard/pause, 10-second polling and
hidden-tab idle. Artifacts stay under `tmp/live-globe/`.

## Layout

```
index.html          the shell (header, notice, view, footer)
design-tokens.css   generated EastSea tokens (scripts/gen-design-tokens.py)
design-components.css generated shared plate, control and status primitives
explorer.css        paper/navy reading surface, quiet rows and responsive layout
token-sources.json  copy of the wallet's token sources (keep in sync)
network.json        copy of the extension's pinned committees (verify.js)
wasm/               the wallet wasm (build product; scripts/build-extension.sh)
public-read-peers.json release peer discovery hints (not chain trust)
js/dom.js           DOM builder — text only, never HTML from chain data
js/rpc.js           local HTTP, verified peer and optional gateway failover
js/peers.js         shared certified peer reader, discovery, floors and quarantine
js/verify.js        the account page's committee-certificate check (wasm)
js/format.js        amounts, numbers, times (BigInt-exact)
js/abi.js           ABI words, selectors, ERC-20 event decoding, revert reasons
js/erc20.js         token metadata, origin scan, badges, impersonation check
js/search.js        search classification and hash resolution
js/sea-url.mjs      canonical name/action parsing, copied from apps/shared
js/presence.js      versioned live presence reads, roles and relay regions
js/polling.js       visible home and pending-transaction polling (10 seconds)
js/app-search.js    apps/names RPC results, warnings and index coverage
js/search-catalog.js search UI strings in five languages
js/pages.js         the five views
js/app.js           router, header (source badge), settings, theme, polling
test/*.test.mjs     units (npm test)
test/live.mjs       live smoke (manual)
```

Mobile-friendly (tables scroll, detail values wrap and Settings stays within
the viewport), dark/light (follows the system; the theme button cycles
auto → dark → light). Every route uses the shared dawn mark and generated
tokens. Account balances use the shared navy plate; metadata uses semantic
label/value lists. A read gets one finite loading acknowledgement, and Reduce
Motion leaves a static indicator. The existing RPC reads and polling remain
owned by the model; the presentation adds no reads or network sources.

`js/peers.js` is the shared public reader, copied into the extension and hosted
site by packaging. `js/rpc.js` handles local HTTP reads, optional gateway
settings and source selection. `js/verify.js` loads the pinned network and
verifier; `js/pages.js` renders proof-backed results and explicit unavailable
fields. `js/app.js` provides routing, settings and polling.

Account headers and address links show decorative Archipelago v1 icons derived
locally from the full 20-byte address, using the same implementation as the
wallet extension. The address text remains authoritative. The frozen algorithm
and shared vectors are in `docs/design/46-account-icon.md`; regenerate the
byte-identical static mirrors with `node scripts/sync-account-icons.mjs`, and
verify them with `node scripts/sync-account-icons.mjs --check`.
