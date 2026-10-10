# 46. Account icons: Islands (Archipelago v3)

2026-10-10. The founder selected **Islands** for 0.7.4. Round 3 removes the
face-like layouts reviewed at `20e669a`. **Waves** and **Navigation** remain
static studies in the [directions sheet](46-account-icon/round3/directions.png);
the original round 2 studies are retained in the artifact index.

## Direction and boundaries

An address becomes a piece of coastline: a broad cove, headland, sandbar, reef
or inlet on a colored sea. Below 32 logical pixels there is one large silhouette.
At 32 px and above an elongated island and a smaller reef occupy opposite
corners around the main coastline. Their sizes, outlines and positions differ.
There are no black dot grids, circles or small repeated glyphs. Identity colors
and geometry are stable in both appearances; the surrounding surface changes.

The palette extends the [EastSea brand tokens](../../design/brand/tokens.json)
and [wallet redesign](wallet-redesign/spec.md): parchment `#f4efe6`, night sea
`#071320`, navy `#0d2135`, sand `#eed7a0`, sea blue and dawn orange. Sixteen hue
families are used only for account identity, without changing product status colors.
The exact To/From addresses from the lead's review now use sea/sand and dawn/navy,
with different hook orientations. Their measured dominant-color ΔE is **49.26**;
they still share the unrotated hook class. See the
[before/after sheet](46-account-icon/round3/before-after.png).

This is a client-only recognition aid next to authoritative address text.
Derivation reads no names, keys, chain state, storage or network. Drawing uses
no randomness. Malformed input receives the same unseeded neutral placeholder.
Signing, recipient checks, approval controls and wallet account data are unchanged.
No protected guest-compiled crate changed, so no proving-program ID change is needed.

## Version and seed

The drawing publishes **v3** and rejects every other explicit version in JS,
Swift and Rust. The seed domain remains **eastsea-account-icon-v2** deliberately:
colors, silhouette class, orientation and below-32-px geometry stay stable for
existing addresses. Only larger-size satellite geometry changes. The v3 SVG
namespace (`eastsea-island-v3-<palette>`) changes the canonical hashes at all sizes;
it does not change the gradient colors.

No icon or feature tuple is stored in an account database, so this changes the
recognition art without migrating account state. V1 and v2 remain in commits
`6cd6fee` and `20e669a`. The v1 coarse equality report is not a v3 perceptual result.

Input is exactly 20 decoded bytes: 40 ASCII hexadecimal digits, optionally
prefixed with `0x` or `0X`. ASCII case and optional prefix do not affect identity.
Whitespace, names, shortened addresses, Unicode digits and other lengths are rejected.

```text
seed = SHA-256(UTF8("eastsea-account-icon-v2") || address_bytes[20])
version  = 3
palette  = seed[0] & 15
layout   = ((seed[1] << 8) | seed[2]) & 0x3fff
shape    = (seed[0] >> 4) & 3
rotation = (seed[0] >> 6) & 3
silhouette_class = shape * 4 + (layout & 3)
```

The feature tuple retains its five-field structure. Palette gets four bits;
shape and rotation move to the next two-bit fields. The layout remains 14 bits.
The first two select a coastline variant; the remaining twelve vary the two
opposite-corner islands. Swift/JS share all drawing tables; Rust derives the same
features and silhouette class without owning UI geometry.

## Normative drawing

Use a 64×64 view box and an opaque rounded square of radius 12. Its background
is a diagonal gradient from `(0,0)` to `(64,64)`, with two fixed stops, interpolated
in encoded sRGB. Fill the coastline and islands with that palette's `ink`.
Rotate the entire land group clockwise by `rotation * 90` around `(32,32)`;
the background gradient does not rotate.

Sixteen fixed integer `M/L/C/Z` paths are normative in the `silhouettes` table of
[the shared fixture](../../crates/client/tests/account-icon-vectors.json).
The [atlas](46-account-icon/round3/atlas.png) shows all colors and classes independently.
The main path uses `translate(0 5) scale(1 0.8)` below 32 px and
`translate(0 0) scale(1 0.8)` at 32 px and above. Its proportions stay the same;
it moves upward to leave space for the neighboring islands. No secondary
geometry is painted below 32 logical pixels, including the extension's 24 px
approval icons and the menu bar's 16 pt icon at Retina scale.

At 32 px and above, read six bits per secondary island:

```text
bits = (layout >> (2 + 6*i)) & 63

Long island (i = 0):
x = 8 + (bits & 3)
y = 44 + ((bits >> 2) & 3)
w = 21 + ((bits >> 4) & 3)
M x y+4
C x+2 y-3 x+w-5 y+1 x+w-2 y-2
L x+w y+6
C x+w-3 y+11 x+3 y+13 x y+4
Z

Smaller reef (i = 1):
x = 40 + (bits & 3)
y = 5 + ((bits >> 2) & 3)
w = 10 + ((bits >> 4) & 3)
M x y+2
C x+3 y-1 x+w-4 y-2 x+w y+1
L x+w-2 y+5
C x+3 y+7 x+1 y+5 x y+2
Z
```

The forms occupy southwest and northeast corners before rotation. A main arch
cannot acquire two eyes above it: in every quarter-turn one secondary centroid
is above the main centroid and the other below. Unequal outlines and areas provide
additional asymmetry. Widths at 32 px are 10.5–12 px and 5–6.5 px respectively.
The previous
occupancy-grid and rotated-mask helpers were removed because they no longer
describe visible art. SVG export and safe SVG DOM construction share one drawing
tree. SwiftUI Canvas paints the same paths; its menu-bar NSImage bridge keeps
its 16 pt logical size and original decorative accessibility behavior.

## Face-layout regression

The [geometry report](46-account-icon/round3/pareidolia.json) exhausts all
**262,144** geometry outcomes (`4 shapes × 16,384 layouts × 4 rotations`).
It verifies that actual emitted paths and transforms are identical at 32 and
64 px. Palette does not enter geometry or detection, so these outcomes cover
all sixteen colors. The check ran on guarded poc-m3.

The independent detector samples each emitted cubic in 64 steps, applies SVG
transforms, and measures filled-polygon area, centroid and bounds. A face pattern
requires two elements with area ratio >=0.65, each <=0.65 of a larger element,
centroid Y separation <=0.5 mean heights and X separation >=1 mean width,
with both above the larger element. **0 outcomes are flagged.** The fixed real
round 2 orange SVG flags once, proving the detector reproduces the reported bug.

A stronger placement gate ignores size and alignment: **0 outcomes put both
secondary centroids above the main centroid**. Every unrotated main centroid lies
between the secondary centroids in both X and Y; minimum bracketing margins are
**4.0173** and **11.2807** view-box units. This remains true under all quarter-turns.
Maximum secondary area ratio is **0.3461**, well below the similarity threshold.
Unit tests cover positive controls, threshold boundaries, SVG transforms and all
sixteen golden addresses in every rotation at 32/64 px.

The numeric checks cover the specified composition; human pareidolia remains
unmeasured. Direct artifact review additionally confirmed that the original orange
example no longer presents two eyes over an arch.

## Palette and contrast

The final sixteen colors were selected with a deterministic max-min CIEDE2000
search on guarded poc-m3, with HSL saturation capped at 0.75, then reordered
without changing the set of colors. The reordering separates the review pair's
hue families; there is no address-specific exception in derivation or rendering.
The search is a heuristic, not a proof of global optimality.

Re-measured minimum ΔE2000 across all 120 background midpoint pairs is
**15.9954**. Midpoints are encoded-sRGB averages converted
to D65 Lab, with `kL = kC = kH = 1`. The closest pair is iris/lilac.
The color implementation passes 40 ordered comparisons against the published
Sharma/Wu/Dalal reference data, including achromatic and hue-wrap cases.

| Index | Family | Background gradient | Shape color |
| --- | --- | --- | --- |
| 0 | tidal | `#209792` → `#1d8781` | `#0d2135` |
| 1 | coral | `#ca2b2b` → `#b92727` | `#eed7a0` |
| 2 | cove | `#2f62da` → `#2558d0` | `#eed7a0` |
| 3 | seagrass | `#429c1c` → `#3b8b18` | `#0d2135` |
| 4 | anemone | `#c7237e` → `#b62073` | `#eed7a0` |
| 5 | gold | `#aa8518` → `#987716` | `#0d2135` |
| 6 | azure | `#298ee0` → `#1f84d6` | `#0d2135` |
| 7 | reef | `#257e52` → `#206f47` | `#eed7a0` |
| 8 | rose | `#d36979` → `#cf596b` | `#0d2135` |
| 9 | kelp | `#6e7722` → `#5f671e` | `#eed7a0` |
| 10 | orchid | `#e444d4` → `#e232d0` | `#0d2135` |
| 11 | sea | `#257793` → `#216a83` | `#eed7a0` |
| 12 | dawn | `#df6320` → `#cd5b1d` | `#0d2135` |
| 13 | iris | `#a029e0` → `#961fd6` | `#eed7a0` |
| 14 | copper | `#96612c` → `#865727` | `#eed7a0` |
| 15 | lilac | `#9579d8` → `#8969d3` | `#0d2135` |

All **96 endpoint contrast pairs** pass 3:1: each gradient endpoint against its
shape ink, parchment and night sea. Minimum internal contrast is **3.4367:1**;
minimum enclosing contrast is **3.0171:1** on light and **3.0118:1** on dark.
Maximum-brightness brand gold and aqua cannot be outer fills unchanged under
both enclosing constraints, so identity hues use darker tonal relatives.

The midpoint ΔE minimum does not promise every raster's dominant color is at
least 15 apart. Antialiasing mixes some land into the largest pixel cluster;
the actual-raster measurement below counts those near-color matches.

## Measured glance distinction

The reproducible stream produces **20,000 pseudorandom addresses**, paired
consecutively into **10,000 independent pairs**:

```text
SHA-256(UTF8("eastsea-account-icon-glance-v2") || UInt32BE(i))[0..20]
i = 0..19999
```

The v3 measurement repeated the same round 2 stream on poc-m3 using the icon-only SwiftUI executable built on
the development Mac. It rendered all **1,024** visible 16 px combinations
(`palette × coastline class × rotation`) and reused only exact visual signatures.
Each raster is truly 16×16 sRGB RGBA, without a surface backdrop. Premultiplied
channels are unpremultiplied; alpha below 0.95 is excluded. Twelve iterations of
two-cluster sRGB k-means identify the largest pixel cluster, whose centroid is
converted to D65 Lab. The sea cluster dominates every sampled signature,
with minimum share **74.18%**.

A pair counts as weak if its dominant-color ΔE2000 is below 15 **and** it has
the same unrotated silhouette class. Rotation is deliberately ignored.

| Proxy | Matching pairs / 10,000 | Fraction | Target |
| --- | ---: | ---: | ---: |
| Requested exact silhouette class | **36** | **0.36%** | <1%: pass |
| Conservative related-outline groups | **68** | **0.68%** | <1%: pass |

The conservative check merges cove/crescent, inlet/hook/arch and twin peaks/ridge,
leaving twelve groups. It avoids counting closely related outlines as automatic
successes. The requested proxy's 95% Wilson interval is **0.26–0.50%**.
Of the 36 weak pairs, 28 have different rotations; seven pairs have identical
16 px palette/class/orientation signatures.

[Raw measurement](46-account-icon/measurement.json) records the stream, denominator,
clustering, class groups, palette distances, examples and source/binary hashes.
[Resource evidence](46-account-icon/remote-resources.json) records nice 15, the
4 GiB available-RAM floor, 30 GiB free-disk floor, 12 GiB owned-RSS cap and cleanup.
The exact owned measurement staging and processes were removed after retrieval.

These are the requested visual proxies, not a human-recognition experiment.
There are only 1,024 small-size visual identities, so repeated identities and
address grinding remain possible. Large-size detail does not add entropy at
16 px. The full address remains authoritative.

## Shared goldens and validation

The original sixteen representative addresses now have v3 feature, stable v2 seed and
**16/32/64 px canonical SVG hashes**. The Python oracle independently implements
SHA-256 and geometry; expected values are frozen and tests do not regenerate them.
Swift and JS check every per-size SVG hash; Rust checks every feature/class tuple.

<!-- ACCOUNT-ICON-VECTORS:START -->
| Address | Tuple `(v,p,l,s,r)` | SVG SHA-256 at 64 px |
| --- | --- | --- |
| `0x0000000000000000000000000000000000000000` | `3,3,12657,1,3` | `0750b6233e646fe10ec6dea09c2d0ea97df9573492740e1936dff4a550a719da` |
| `0xffffffffffffffffffffffffffffffffffffffff` | `3,1,11846,1,0` | `26de1ae1ca6aa19954408875d993888bcee2d9add1b7cb65c786a6c44008c609` |
| `0x0000000000000000000000000000000000000001` | `3,3,5157,2,2` | `89613ca6a5a81c926f04cc52f75413bce6165a04b3dd51356d1744a0ef4f54d9` |
| `0x0000000000000000000000000000000000000002` | `3,7,13120,0,0` | `8015632864296e24420ec5f8a7d72e7aef6bcf43e3d3e7c31d6ef471f235be31` |
| `0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef` | `3,13,7123,0,2` | `7b8cf48f2946a0e3c6cd64470e133ad6c8b1a9373acb5d8d0f4a2085a49f3390` |
| `0x52908400098527886e0f7030069857d2e4169ee7` | `3,3,8027,2,2` | `b066734a74ff428dab2650adbf466d2cb1563923f824ad0cc79e1cc231a5c3af` |
| `0x00000000000000000000000000000000000000c1` | `3,5,4220,1,2` | `a9b5d649adfea7961e3aee19a9f9168c0ccf7b69f89f2969bb95bf98899fe9bb` |
| `0x1234567890abcdef1234567890abcdef12345678` | `3,12,290,2,2` | `238ec74f446b189761764de304dbd1efb09f7dd9dedba8145f5295fd5deea944` |
| `0x1234567890abcdef0000000000abcdef12345678` | `3,2,12388,3,3` | `60856d88f05a12149c96337fd4cd86e1eb64d0730a823bd273d1a414ef72751c` |
| `0xa2521982a17474cb2f8741c85de653b5282d72b0` | `3,11,5518,2,3` | `9f68122d9a665d79d5e9185aa2de884aa0b9416f5a7b1a475a30c04b82f088a8` |
| `0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416` | `3,14,6726,3,0` | `c770a76b91fe8c9326e53244412b9fd09b1a7943314d27f01030efacbf7f7af0` |
| `0x961f8add5ae93ff0700be8abd5f9f8ec69ba4347` | `3,15,6548,1,3` | `935c741817c6427cd138cdf46cccf176ef64fb9dba82d5a1a8fc143c1d7a7c24` |
| `0xc91367bac92c6de822de8afd0f34ff19fd8f7670` | `3,8,10334,1,3` | `a294099c04cd1baedd4bb9b6e6ac4635a028143f3f554f36cb24a1f8c0f690fc` |
| `0x00000000000000000000000000000000000000ff` | `3,12,1259,3,3` | `d2e72e7ed7ee6a2480e89d2a49295c01ac561666582298a75007bd46aeadea4b` |
| `0x0000000000000000000000000000000000000100` | `3,5,12298,1,0` | `4bce7d04f95cc580f1351c41cb64396368d0991d065cf2f55dccceac046c9a74` |
| `0x000000000000000000000000000000000000ffff` | `3,2,3628,0,2` | `6fd370a8e04dcb00731d965671f434b666fdbfe5166b6ce676aaaa664e36c694` |
<!-- ACCOUNT-ICON-VECTORS:END -->

Validation passed: Swift pure icon goldens and static Canvas/menu-bar rendering;
Rust client tests **4/4** locally, rustfmt and clippy with warnings
denied; extension **329/329** and explorer **377/377**; color-science tests **5/5** and layout tests **14/14**;
independent Python oracle, byte-identical JS mirrors and syntax/whitespace checks.
The extension suite used exact staged sources and existing WASM under `./tmp`,
without rebuilding it. The reused WASM exports the account verifier and matches
the captured proof fixture; the unrelated older root WASM lacks that export. The client crate has only the existing SHA-256 dependency.

All 106 SwiftUI PNGs, 96 browser icon PNGs, both browser review sheets and eight
extension/explorer light/dark surfaces were regenerated. Their
[96-pair raster comparison](46-account-icon/render-comparison.json) has mean
absolute RGB-channel difference **1.4007/255** and worst image **2.8372/255**,
within the 5/255 review limit; edge antialiasing remains platform-specific.
The [artifact index](46-account-icon/README.md) contains review links and commands. The
[64-image stability report](46-account-icon/round3/small-size-stability.json)
finds all browser and native dark 16 px captures exact against round 2. Fifteen
native light captures differ by at most one channel level out of 255; their
canonical geometry and colors are unchanged. The measured glance result remains
**36/10,000 = 0.36%**, with the same **0.68%** conservative result and **15.9954**
minimum midpoint ΔE. The measurement script enforces all three round 2 limits.

Wallet/release/guest builds and launching EastSea are reserved for the lead and
were not performed in this lane. No node, signing request or wallet state was
used by the renderers. The new executable's macOS 14 deployment target lets the
same development-Mac build produce the measured rasters on poc-m3's macOS 15.

## Integration handoff

Rebased onto `codex/integrate-074` at `bcd7273`. Seven round 1 conflicts were
resolved in explorer documentation/CSS/pages, the extension popup, and wallet
account-switcher/receive/dashboard views. Integration design tokens, navy plates,
QR rendering, branding, verified reads and transaction-preview/signing behavior
were preserved while restoring the account-icon wiring. Gated Swift syntax parsing
of the three resolved views passed; the lead still owns the full wallet build.

The integration's newer DOM and approval lifecycle required namespace/storage
mocks and reviewed-preview fixtures. Its existing locale test was updated to
assert the current five translated HTTPS refusals rather than requiring the English
`.com` example in every language. Product translations were not edited. Expanded
extension/explorer suites pass after these adaptations.

The fresh mobile capture shows an existing integration header issue: the Live
network label can overlap Settings at 360 px. It is recorded for the lead; this
lane preserved that header's styling. Account icons, full addresses and the
specified face-layout gates pass. See [verification](46-account-icon/round3/verification.json).
