# EastSea Explorer

> 한국어 요약: EastSea 노드의 JSON-RPC를 읽는 정적 파일 블록 익스플로러. 빌드 단계
> 없이 `python3 -m http.server`로 띄운다. 읽기는 순서대로 시도한다 — 이 Mac의 노드
> (기본 `127.0.0.1:18545`) 먼저, 브라우저가 막으면 공개 읽기 전용 게이트웨이(기본
> `https://rpc.eastsea.xyz`, Settings에서 변경)로. 분석·가격 정보 없음. 계정 잔액은
> 지갑과 같은 wasm으로 위원회 인증서를 검증하고 통과할 때만 "verified"로 표시하며,
> 나머지 데이터는 "노드 제공, 검증 안 됨"으로 표시한다.
> `npm test`로 단위 테스트, `node test/live.mjs`로 실노드 스모크.

A read-only block explorer for an EastSea chain, served as **static files** — no
build step, no framework, no server-side code. The page in your browser reads
through an ordered list of sources: your own node first (default
`http://127.0.0.1:18545`, while the EastSea app runs), then — when this browser
cannot reach loopback, or the node is down — the public read-only gateway
(default `https://rpc.eastsea.xyz`, changeable in Settings; see
`docs/ops/read-gateway.md`). The header badge always says which source answered.
No analytics, no prices.

## Run

```bash
cd apps/explorer
python3 -m http.server 8090
# open http://localhost:8090
```

Any static file server works; `npx serve` or a GitHub Pages deployment behave
the same. Opening `index.html` straight from the filesystem also works in most
browsers (the node's CORS allows it — see below), but a served origin is the
supported path.

The node must be reachable from the browser: the EastSea app's node listens on
`127.0.0.1:18545` on this Mac while it runs. Point Settings at another URL to
read a different node. When the browser blocks the loopback read (Chrome asks
for local-network access and was denied, or Safari blocks http from an https
page), the page says so plainly and reads the gateway instead — unverified, and
never a write. Clear the gateway field in Settings to read your node only.

## Pages

| Page | What it shows |
|---|---|
| Home | finalized height (hero), tx rate over the newest 30 blocks, committee (registry candidates + epoch), protocol (with node/scheduled versions and an update pill), mempool, base fee, prover status, latest blocks, chain facts |
| Block | every header field the RPC serves, neighbor links, this node's prover view of the block's proof, the transactions with their receipts (a pruned block shows what the era record still carries) |
| Transaction | receipt (status, gas, contract creation, output), events decoded as ERC-20 `Transfer`/`Approval` with symbol and amount, raw logs for anything else |
| Account | balance/nonce/code with a committee-certificate badge (`verified by committee certificate` only when the wallet's own wasm check passed — `js/verify.js`), token detection, latest rewards (`aether_rewards`), ERC-20 transfers to/from the address in the node's log window |
| Token | name/symbol/decimals/total supply, the origin badge and impersonation warning exactly as the wallet shows them, recent transfers |
| Search | height, `0x`-address, or tx hash; a hash with no receipt is matched against the newest block hashes |

## Honest labels

The account balance is the one thing this explorer verifies in the browser: the
same wasm the wallet extension ships (copied into `wasm/` by
`scripts/build-extension.sh`) runs the committee-certificate check — pinned
identity, chain, finalized height, ten-minute freshness, and the state proof
behind the balance — and the badge says `verified by committee certificate`
only when it passed. Everything else is honest about being unverified: pages
say "Data read from the node at …, not light-client verified", and when the
source is the public gateway the header badge says so too ("Public gateway ·
not verified"). Proof status on a block is that node's prover's view, not an
on-chain record. Token badges follow the wallet's zero-trust policy:
*Launchpad · unverified* is the wallet's label for launchpad tokens, not a
judgement of fraud, and an "official list" badge only means the address is in
the bundled `token-sources.json` (kept in sync with `apps/wallet` and
`apps/extension`).

The wasm and `network.json` (the pinned committees) are build products here;
without them the account page says `not verified`, never pretends.

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
`aether_getAccount`, `aether_candidates`, `aether_proverStatus`,
`aether_rewards`, `aether_history`, `eth_call`, `eth_getLogs`,
`eth_blockNumber`, and `aether_getFinalized` (the account page's certificate
check, `js/verify.js`). Node-side notes are in `crates/node/src/rpc.rs`; the
explorer adds no node RPCs.

Two windows to know about: `eth_getLogs` scans at most the newest 2,000
finalized blocks (token/account transfer lists are labeled with that), and
block summaries are served for heights this node kept — older ones come back
as era records, and heights below what it ever kept simply don't. The public
gateway caps these the same way and refuses everything else — its allowlist
and caps are in `docs/ops/read-gateway.md`.

## Tests

```bash
cd apps/explorer
npm test              # decoding/formatting/search/RPC units (node --test, offline)
node test/live.mjs    # renders every page against a real node (default 127.0.0.1:18545)
```

The unit tests cover the pure helpers: ABI word parsing and `Transfer`/
`Approval` decoding, revert-reason decoding, amount/time formatting, the token
metadata and origin scans (against a mock reader), the badge rules, the search
classifier and resolver, and the JSON-RPC client (injected `fetch`, endpoint
persistence, error and timeout paths). `test/live.mjs` is a manual smoke test
in a minimal DOM stub — it is deliberately not part of `npm test`.

## Layout

```
index.html          the shell (header, notice, view, footer)
explorer.css        the extension's palette, stretched over a page
token-sources.json  copy of the wallet's token sources (keep in sync)
network.json        copy of the extension's pinned committees (verify.js)
wasm/               the wallet wasm (build product; scripts/build-extension.sh)
js/dom.js           DOM builder — text only, never HTML from chain data
js/rpc.js           JSON-RPC client, endpoint+gateway persistence, failover
js/verify.js        the account page's committee-certificate check (wasm)
js/format.js        amounts, numbers, times (BigInt-exact)
js/abi.js           ABI words, selectors, ERC-20 event decoding, revert reasons
js/erc20.js         token metadata, origin scan, badges, impersonation check
js/search.js        search classification and hash resolution
js/pages.js         the five views
js/app.js           router, header (source badge), settings, theme, polling
test/*.test.mjs     units (npm test)
test/live.mjs       live smoke (manual)
```

Mobile-friendly (tables scroll, tiles wrap), dark/light (follows the system;
the ◐ button cycles auto → dark → light).
