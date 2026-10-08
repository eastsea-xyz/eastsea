# Readable live network globe — 0.7.4

Owner: `codex/live-globe`. Presence producer: sibling `codex/live-peers`.
This is the public aggregate contract. The producer must adopt v2 before the
lead enables live data. This lane changes only web code, fixtures, tests and
documentation. It performs no deployment or installed-app/testnet changes.

## Readability and disclosure

The opening hemisphere centers on the continent with the most observed Macs,
including session jitter; today's snapshot opens on Asia. Thin Natural Earth
coastlines, 10,385 high-contrast land dots, a faint 30-degree graticule and an
atmosphere rim add geographic context to the silhouette. The globe rotates slowly at
0.025 radians/second. Dragging, keyboard input, pulse selection and list
interaction hold rotation until 10 seconds of inactivity. A retained selection
does not prevent the idle timeout; manual pause persists until resumed.

Each populated known continent gets one pulse at its bundled centroid, with
bounded session jitter. Pulse diameter uses a square-root count scale. A gold
core's area represents the founder-operated share; the cyan outer arc represents
the independent share. A count label, such as **아시아 4 · 창업자 4**, and its
accessible name state the counts explicitly. There are no per-node markers.
Behind-the-globe pulses are hidden and their list rows remain highlighted with
**지구본 뒷면 · 목록에서 확인** / **Far side of globe · highlighted here**.
Unknown regions stay in the list and never receive an invented position.
Hovering/tapping either a pulse or its full list row highlights both. Keyboard
users can focus continent buttons, move among populated rows and rotate the
canvas. Labels are measured outside the animation loop, clamped to the stage
and spaced to avoid collisions. Crowded static maps use a taller stage.

The bilingual legend is:

- **점 크기 = 연결된 Mac 수, 위치는 대륙 단위** /
  **Dot size = Macs connected; placed per continent**
- **금색 = 창업자 운영 노드** / **Gold = founder-operated nodes**;
  the accompanying ring key identifies independent nodes.

The supplied snapshot is dated **2026-10-08** and labeled
**오늘 기준 실제 구성 (실시간 아님)** / **Today’s actual setup (not live)**:
Asia **4 Macs**, Korea **3**; validators **4 (founder-run 4)**;
wallet nodes **3 (founder-run 2)**; reserve keys **3 standby, 0 seated**.
There are no fabricated recent block events. The former 24-Mac example lives
only in `apps/explorer/test/fixtures/presence-example.json` for regression
coverage; historical screenshots below show the earlier implementation.
A failed live request never substitutes fixture counts.

## RPC contract (v2)

Request: `{"jsonrpc":"2.0","id":1,"method":"aether_presence","params":[]}`.
The configured public read gateway must allowlist this read-only method.
Today's aggregate `result` illustrates the exact shape:

```json
{
  "schema_version": 2,
  "scope": "node",
  "total": 4,
  "founder_operated": 4,
  "roles": {
    "validator": { "count": 4, "founder_operated": 4 },
    "wallet": { "count": 3, "founder_operated": 2 },
    "candidate": { "count": 0, "founder_operated": 0 },
    "follower": { "count": 0, "founder_operated": 0 }
  },
  "versions": { "0.7.4": 4 },
  "reserve_keys": { "standby": 3, "seated": 0 },
  "regions": [
    { "continent": "asia", "country": "KR", "count": 3, "founder_operated": 3 },
    { "continent": "asia", "count": 1, "founder_operated": 1 }
  ],
  "recent_blocks": []
}
```

- All counts and heights are nonnegative safe JSON integers. Region buckets
  are disjoint. Their `count` sum equals `total`; their `founder_operated` sum
  equals the top-level founder count. Every founder count is at most its
  corresponding count. Versions sum to `total`, use semver release strings
  of at most 64 characters, and contain no arbitrary server messages.
- Role counts describe participation on connected Macs. Validator and wallet
  roles may overlap on the same Mac, so roles are **not summed to get total**.
  Each role's count is at most `total`; each role's founder count is at most
  its own count. Founder attribution is independent for each hosted role;
  wallet participation can use an independently operated wallet on a
  founder-operated Mac. Role founder counts are not summed against the Mac
  founder total. `validator`, `wallet`, `candidate`, `follower` are required.
- `reserve_keys.standby` and `.seated` count configured reserve validator keys
  in those states, not connected Macs or extra wallet nodes. Their sum must
  also be a safe integer. Standby keys do not inflate active validators. A
  seated reserve key's host is counted through the normal active Mac/role
  aggregation, never by adding reserve keys to the Mac total.
- **Producer attribution only:** match internal genesis/reserve keys and the
  configured founder operator address against the explicit founder-owned set.
  Assign `founder_operated` separately per region and role during aggregation.
  Never infer another operator's identity or infer founder status from geography,
  version, counts or behavior. The browser consumes explicit counts and never
  receives keys, addresses or a founder identity lookup table. The founder
  consented to this disclosure; no extra founder privacy step is required.
- `continent` is one of `africa`, `asia`, `europe`, `north_america`,
  `south_america`, `oceania`, `antarctica`, `unknown`. Live continents come
  from a node's **home iroh relay**, never IP geolocation. They approximate
  relay placement, not the person's physical location. Unmapped/no relay is
  `unknown`. The dated manual snapshot is supplied configuration, not a claim
  that a live relay was queried.
- `country` is optional uppercase ISO 3166-1 alpha-2, only by explicit opt-in.
  Merge a country's opted-in entries within its continent before testing
  **k=3**. Below 3 Macs, omit the country code and fold **both counts** into
  its continent-only bucket before transmission. The browser repeats this
  folding defensively. Do not cross-tabulate countries by role/version.
  Other operators remain anonymous regional aggregates; no precise locations
  or individual records are added by founder disclosure.
- `recent_blocks` is optional (default `[]`), at most 8 entries, oldest first.
  Each entry is `{ "height": safeInteger, "continent": continentCode }`.
  Omit unavailable regions; never synthesize arcs for a live or manual snapshot.
  Arcs join consecutive known validator continents and show block-region
  sequence, not measured node-to-node traffic.
- Browser limits: 1,024 raw region buckets, 128 versions, 8 recent blocks.
  No IPs, relay URLs, cities, coordinates, peer/node identifiers, wallet
  addresses, timestamps, hashes, arbitrary strings or individual presence
  records are part of the public model. JSON-RPC errors use the usual envelope;
  the page never echoes their contents.

### Producer integration gate

The interim presence response previously returned `nodes`, `observer`
identifiers and singleton `by_country` entries. It must not be served to this
page. The public producer must emit v2 aggregates and enforce country k=3
before exposure. Internal signed presence/gossip records remain internal.
The consumer rejects both interim individual schemas and v1 aggregates
without founder disclosure; missing attribution is not silently interpreted
as independent. This is a producer handoff, not a claim that browser filtering
can retract identifiers already transmitted.

## Shared module and runtime

Canonical ES modules live in `apps/explorer/live-globe/`; byte-identical site
copies are generated by `scripts/sync-live-globe.mjs`. Both surfaces serve
committed local assets without a build or CDN. Explorer uses `#/network`,
caption **Macs this node can see**, and the Settings public read gateway.
The site's Korean/English section and `site/live-network.json` remain intact.
Only `aether_presence` is polled, every 10 seconds, without credentials or
referrers; no peer/committee/individual method is called.

Centroids, coastlines and graticules are internal bundled artwork, never
positions supplied by the presence API. Session jitter remains bounded to
0.035 radians per axis, in memory only and stable across mounts in one page.
Opted-in countries appear only in text. No new dependency or external asset
request is introduced.

Reduced motion and WebGL failure use the static map with the same labeled
pulses and text equivalent. GPU context loss switches to the map; restoration
rebuilds buffers without losing counts. Animation, CSS pulses and polling stop
in hidden tabs or outside the viewport. Resuming visibility requests fresh
presence. Loading, unavailable, stale and empty counts remain explicit.

Artwork is generated from the bundled public-domain
[Natural Earth 110m land](https://www.naturalearthdata.com/downloads/110m-physical-vectors/110m-land/)
archive, SHA-256
`1926c621afd6ac67c3f36639bb1236134a48d82226dc675d3e3df53d02d2a3de`.
`scripts/generate-globe-land.py` uses the standard library, verifies this digest,
samples 36,000 equal-area sphere positions and retains 10,385 land dots.
It exports 5,057 coastline segments, subdivided below 2 degrees, omitting
artificial dateline closure edges. The archive stays in `tmp/`; committed
`land.js` contains only finite unit-sphere artwork.

## Verification

The offline unit suite passes **95/95 tests** and includes privacy/schema-v2 validation, today's exact
counts, overlapping roles, reserve keys, founder bounds, identifier stripping,
k=3 folding, local request rules, synchronized copies and the gzip budget.
`test/globe-renderer.test.mjs` drives the real renderer and component against a
deterministic browser host. It verifies centering on multiple hemispheres;
every populated known continent's marker or explicit off-globe list state;
unknown handling; founder core area/ring share and square-root sizing; crowded
mobile map label bounds/collisions; linked pulse/full-row selection;
10-second idle resumption including retained selection; manual pause;
hidden/reduced-motion scheduling; and GPU context loss/restoration.
These are behavior/geometry tests, not browser raster or GPU performance proof.

The canonical JavaScript, including artwork, is **173,668 bytes gzipped**,
within the **300,000-byte** budget. The Impeccable mechanical detector returned
no findings. No dependencies were added. Run from the repository root:

```sh
node scripts/sync-live-globe.mjs --check
npm --prefix apps/explorer test
```

### Browser verification is pending

This session's sandbox rejected a local HTTP listener with `listen EPERM` and
aborted installed headless Chrome with `SIGABRT`; browser-skill also had no
reachable daemon. The updated browser runner fulfills workspace files directly
through Playwright interception, avoiding the listener and all external calls,
but Chrome still cannot launch here. **The new renderer has no fresh screenshots,
visual-verdict pass, accessibility raster review or measured 60-fps result yet.**
The previous renderer's 60-fps result from commit `1332e22` does not verify this
change. The 60-fps target is retained; the runner's portable smoke floor is
30 fps, and its recorded frame rate must be reviewed against the 60-fps target
on the reference M1 Max. The mock host does not compile or rasterize shaders.

The prepared offline runner covers 11 screenshots plus multi-continent marker
coverage, mobile label bounds/collisions, linked hover/tap, timed idle rotation,
local-only assets, language switching, WebGL/reduced motion, polling cadence,
hidden-tab lifecycle, stale/empty/unavailable responses, and session jitter.
Run it in a host that permits headless Chrome, then copy the successful output
into `docs/design/live-globe/` and replace the pending cells below:

```sh
mkdir -p tmp
TMPDIR="$(git -C . rev-parse --show-toplevel)/tmp" \
PLAYWRIGHT_MODULE=/path/to/existing/playwright \
node scripts/test-live-globe.mjs
```

The existing `docs/design/live-globe/` images are still the **previous renderer**,
not evidence for the new implementation. Preserved baseline copies are in
`docs/design/live-globe-before/`. Never substitute synthetic mock-host renders
for browser screenshots.

## Before / after review matrix

The baseline is the 24-Mac illustrative setup at `1332e22`; the after render
will use the dated four-Mac actual setup. All eleven after images remain pending
because of the browser restriction above.

| Surface/state | Before (`1332e22`) | After (new renderer) |
| --- | --- | --- |
| Site Korean, light | ![Before, site light](live-globe-before/site-light.png) | Pending browser render |
| Site Korean, dark | ![Before, site dark](live-globe-before/site-dark.png) | Pending browser render |
| Explorer, light | ![Before, explorer light](live-globe-before/explorer-light.png) | Pending browser render |
| Explorer, dark | ![Before, explorer dark](live-globe-before/explorer-dark.png) | Pending browser render |
| Explorer mobile, light | ![Before, explorer mobile light](live-globe-before/explorer-mobile-light.png) | Pending browser render |
| Explorer mobile, dark | ![Before, explorer mobile dark](live-globe-before/explorer-mobile-dark.png) | Pending browser render |
| Site English mobile, light | ![Before, site mobile light](live-globe-before/site-mobile-light.png) | Pending browser render |
| Site English mobile, dark | ![Before, site mobile dark](live-globe-before/site-mobile-dark.png) | Pending browser render |
| Reduced-motion map | ![Before, reduced motion](live-globe-before/explorer-reduced-motion.png) | Pending browser render |
| Unavailable | ![Before, unavailable](live-globe-before/explorer-unavailable.png) | Pending browser render |
| Stale snapshot | ![Before, stale](live-globe-before/explorer-stale.png) | Pending browser render |
