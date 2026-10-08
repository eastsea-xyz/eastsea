# EastSea Explorer

> 한국어 요약: 정적 파일 블록 익스플로러. 이 Mac의 노드
> (`127.0.0.1:18545`)를 먼저 읽고, 연결되지 않으면 공개 노드 배열에서
> iroh WASM과 WebSocket 릴레이로 읽는다. 공개 노드의 응답은 번들에 고정된
> 네트워크 신원, BLS 최종성 인증서와 Merkle 증명을 검증한 뒤 표시한다.
> 기본 게이트웨이는 없다. Settings에 개인 게이트웨이와 릴레이 주소를 설정할 수 있다.

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

## Tests and measurements

```bash
cd apps/explorer
npm test
node test/live.mjs http://127.0.0.1:18545  # optional local-node page smoke
```

The offline suite covers peer-only reads, three-peer maintenance, discovery,
diversity hints, wrong-header and slow-peer removal, field sanitization,
verified-height replay protection, optional gateway persistence, and existing
decoding/formatting/search behavior. The lane's devnet/headless browser runner
adds real certificate and WebSocket transport checks.

`window.aetherReadDiagnostics()` reports the current source, peer IDs and
protocol-level head/block read timing. It sends no telemetry. Full cold-page
latencies and WASM sizes are recorded by the browser/devnet measurement runner.

## Layout

`js/peers.js` is the shared public reader, copied into the extension and hosted
site by packaging. `js/rpc.js` handles local HTTP reads, optional gateway
settings and source selection. `js/verify.js` loads the pinned network and
verifier; `js/pages.js` renders proof-backed results and explicit unavailable
fields. `js/app.js` provides routing, settings and polling.

Tables scroll on small screens. Theme follows the system; ◐ cycles auto,
dark and light.
