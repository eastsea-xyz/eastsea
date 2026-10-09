# 46. Account icons: Archipelago v2

2026-10-10. Round 2 replaces the visual layer reviewed at `6cd6fee`.
The implemented direction is **Islands**; **Waves** and **Navigation** are static
alternatives in the [founder directions sheet](46-account-icon/round2/directions.png).

## Direction and boundaries

An address becomes a piece of coastline: a broad cove, headland, sandbar, reef
or inlet on a colored sea. Below 32 logical pixels there is one large silhouette.
At 32 px and above two substantial irregular neighboring islands appear.
There are no black dot grids, circles or small repeated glyphs. Identity colors
and geometry are stable in both appearances; the surrounding surface changes.

The palette extends the [EastSea brand tokens](../../design/brand/tokens.json)
and [wallet redesign](wallet-redesign/spec.md): parchment `#f4efe6`, night sea
`#071320`, navy `#0d2135`, sand `#eed7a0`, sea blue and dawn orange. Sixteen hue
families are used only for account identity, without changing product status colors.
The exact To/From addresses from the lead's review now use sea/sand and dawn/navy,
with different hook orientations. Their measured dominant-color ΔE is **49.26**;
they still share the unrotated hook class. See the
[before/after sheet](46-account-icon/round2/before-after.png).

This is a client-only recognition aid next to authoritative address text.
Derivation reads no names, keys, chain state, storage or network. Drawing uses
no randomness. Malformed input receives the same unseeded neutral placeholder.
Signing, recipient checks, approval controls and wallet account data are unchanged.
No protected guest-compiled crate changed, so no proving-program ID change is needed.

## Version and seed

The redesign publishes v2 rather than silently reinterpreting v1 drawing goldens.
JS, Swift and Rust now default to **2** and reject other explicit versions.
No icon or feature tuple is stored in an account database, so this changes the
recognition art without migrating account state. The v1 implementation and its
100,000-address coarse equality report remain in commit `6cd6fee`; those numbers
must not be presented as v2 perceptual results.

Input is exactly 20 decoded bytes: 40 ASCII hexadecimal digits, optionally
prefixed with `0x` or `0X`. ASCII case and optional prefix do not affect identity.
Whitespace, names, shortened addresses, Unicode digits and other lengths are rejected.

```text
seed = SHA-256(UTF8("eastsea-account-icon-v2") || address_bytes[20])
version  = 2
palette  = seed[0] & 15
layout   = ((seed[1] << 8) | seed[2]) & 0x3fff
shape    = (seed[0] >> 4) & 3
rotation = (seed[0] >> 6) & 3
silhouette_class = shape * 4 + (layout & 3)
```

The feature tuple retains its five-field structure. Palette gets four bits;
shape and rotation move to the next two-bit fields. The layout remains 14 bits.
The first two select a coastline variant; the remaining twelve vary the two
larger-size islands. Swift/JS share all drawing tables; Rust derives the same
features and silhouette class without owning UI geometry.

## Normative drawing

Use a 64×64 view box and an opaque rounded square of radius 12. Its background
is a diagonal gradient from `(0,0)` to `(64,64)`, with two fixed stops, interpolated
in encoded sRGB. Fill the coastline and islands with that palette's `ink`.
Rotate the entire land group clockwise by `rotation * 90` around `(32,32)`;
the background gradient does not rotate.

Sixteen fixed integer `M/L/C/Z` paths are normative in the `silhouettes` table of
[the shared fixture](../../crates/client/tests/account-icon-vectors.json).
The [atlas](46-account-icon/round2/atlas.png) shows all colors and classes independently.
The main path uses `translate(0 5) scale(1 0.8)` below 32 px and
`translate(0 0) scale(1 0.8)` at 32 px and above. Its proportions stay the same;
it moves upward to leave space for the neighboring islands. No secondary
geometry is painted below 32 logical pixels, including the extension's 24 px
approval icons and the menu bar's 16 pt icon at Retina scale.

At 32 px and above, for each island `i = 0, 1`:

```text
bits = (layout >> (2 + 6*i)) & 63
x = 9 + 25*i + (bits & 3)
y = 46 + ((bits >> 2) & 3)
w = 14 + ((bits >> 4) & 3)
M x y+4
C x+2 y-3 x+w-5 y+1 x+w-2 y-2
L x+w y+6
C x+w-3 y+11 x+3 y+13 x y+4
Z
```

These are two large irregular forms, 7–8.5 px wide at 32 px. The previous
occupancy-grid and rotated-mask helpers were removed because they no longer
describe visible art. SVG export and safe SVG DOM construction share one drawing
tree. SwiftUI Canvas paints the same paths; its menu-bar NSImage bridge keeps
its 16 pt logical size and original decorative accessibility behavior.

## Palette and contrast

The final sixteen colors were selected with a deterministic max-min CIEDE2000
search on guarded poc-m3, with HSL saturation capped at 0.75, then reordered
without changing the set of colors. The reordering separates the review pair's
hue families; there is no address-specific exception in derivation or rendering.
The search is a heuristic, not a proof of global optimality.

Minimum ΔE2000 across all 120 background midpoint pairs is
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

The measurement ran on poc-m3 using the icon-only SwiftUI executable built on
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
16 px. The full address remains authoritative, and founder style selection
remains separate from artifact QA.

## Shared goldens and validation

The original sixteen representative addresses now have v2 feature, seed and
**16/32/64 px canonical SVG hashes**. The Python oracle independently implements
SHA-256 and geometry; expected values are frozen and tests do not regenerate them.
Swift and JS check every per-size SVG hash; Rust checks every feature/class tuple.

<!-- ACCOUNT-ICON-VECTORS:START -->
| Address | Tuple `(v,p,l,s,r)` | SVG SHA-256 at 64 px |
| --- | --- | --- |
| `0x0000000000000000000000000000000000000000` | `2,3,12657,1,3` | `9857f39b31d0f708f477deab8a18450d3b20ce5ebaa075341f1a7ccdb4b97aa1` |
| `0xffffffffffffffffffffffffffffffffffffffff` | `2,1,11846,1,0` | `2e0233c2d8a76c1422626fec1a2e8418db9583c28e1d8c6ab7e2144e78ac0b25` |
| `0x0000000000000000000000000000000000000001` | `2,3,5157,2,2` | `cdc24cd0a0b35c897d2b9aea1cf9def7259717eb6230071d8d38cd32b7f75ed3` |
| `0x0000000000000000000000000000000000000002` | `2,7,13120,0,0` | `8fb3a2a6e999ddb9fcb2c4b9b5d2e41b32f7811cde3f521dd6d39c4a673570aa` |
| `0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef` | `2,13,7123,0,2` | `e84e51e1287187d144a954326b69754a4c32bd4e9c80c575af587d589476e6b5` |
| `0x52908400098527886e0f7030069857d2e4169ee7` | `2,3,8027,2,2` | `75d690aaef3ae9ed6b394edf3db9b46b4682db8e81b60d9f0e8cd6b06f032230` |
| `0x00000000000000000000000000000000000000c1` | `2,5,4220,1,2` | `0c027152f3303247258fddad593107697baf48d5267e2981b10449f38e41eba2` |
| `0x1234567890abcdef1234567890abcdef12345678` | `2,12,290,2,2` | `e32efe6dbe768a460b760190e7c895cdb5dc19d470035bd20230e1fdbaa9e3bf` |
| `0x1234567890abcdef0000000000abcdef12345678` | `2,2,12388,3,3` | `66201ac48f016b8bd5e650b9e30c696ec0c2bd5ffd1411a2e334e16e379d35af` |
| `0xa2521982a17474cb2f8741c85de653b5282d72b0` | `2,11,5518,2,3` | `a399b09f5c13d4f765867c8fca69160af8058c1d268964349fb30fc7bdaf500b` |
| `0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416` | `2,14,6726,3,0` | `619c288720e8ea5c35de7d8afac4a15889f902c6f08442022d3068e87c167ea7` |
| `0x961f8add5ae93ff0700be8abd5f9f8ec69ba4347` | `2,15,6548,1,3` | `6110a826a48c21eaac8ea57ea633aa4cf1dc0f250d9c22f4efcdebb103e83f8e` |
| `0xc91367bac92c6de822de8afd0f34ff19fd8f7670` | `2,8,10334,1,3` | `a47d15d127bc2acb60ab4b451617cae3a26b32ec9e6cb45d2509b40eed654300` |
| `0x00000000000000000000000000000000000000ff` | `2,12,1259,3,3` | `3cf44b38cb31f356297f401f8ee940ed54e964779ab79a85b83425a937568534` |
| `0x0000000000000000000000000000000000000100` | `2,5,12298,1,0` | `50468a994f4beecae62473de1ac120c385b61d0f3cfeb2d8f111b256458ffdb5` |
| `0x000000000000000000000000000000000000ffff` | `2,2,3628,0,2` | `8d067d9834b3ed376338f15538d3a13f17d565fb019d7c0184a611500329af06` |
<!-- ACCOUNT-ICON-VECTORS:END -->

Validation passed: Swift pure icon goldens and static Canvas/menu-bar rendering;
Rust client tests **4/4** locally and remotely, rustfmt and clippy with warnings
denied; extension **126/126** and explorer **67/67**; color-science tests **5/5**;
independent Python oracle, byte-identical JS mirrors and syntax/whitespace checks.
The extension suite used exact staged sources and existing WASM under `./tmp`,
without rebuilding it. The client crate has only the existing SHA-256 dependency.

All 106 SwiftUI PNGs, 96 browser icon PNGs, both browser review sheets and eight
extension/explorer light/dark surfaces were regenerated. Their
[96-pair raster comparison](46-account-icon/render-comparison.json) has mean
absolute RGB-channel difference **1.4200/255** and worst image **2.8359/255**,
within the 5/255 review limit; edge antialiasing remains platform-specific.
The [artifact index](46-account-icon/README.md) contains review links and commands.

Wallet/release/guest builds and launching EastSea are reserved for the lead and
were not performed in this lane. No node, signing request or wallet state was
used by the renderers. The new executable's macOS 14 deployment target lets the
same development-Mac build produce the measured rasters on poc-m3's macOS 15.
