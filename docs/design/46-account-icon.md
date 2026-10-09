# 46 — Address-derived account icons

2026-10-09 · algorithm version **1**, “Archipelago”.

An address has one locally derived icon on the wallet, menu bar, extension,
explorer and site/SDK. No account name, chain, metadata, key, storage, network,
clock or runtime randomness enters the derivation. An icon helps recognize a
changed address; it does not authenticate it. The full address remains visible
at approvals. A determined attacker can search the finite visual space.

## Short survey and decision

Counts are source-derived categorical features, not measured human recognition.
No inspected project establishes a color-blind safety or perceptual-confusion rate.

| Reference | Visual features and collision evidence | Color-blind considerations / license |
| --- | --- | --- |
| [Blockies](https://github.com/ethereum/blockies/blob/master/blockies.js) | 8×8 mirrored grid, 32 independent ternary cells and three generated colors; at most 3³² categorical masks, no measured confusion rate. | Geometry helps; colors are not contrast checked. [README](https://github.com/ethereum/blockies#license) says WTFPL while [package metadata](https://github.com/ethereum/blockies/blob/master/package.json) says MIT; avoid adapting ambiguous code. |
| [Jazzicon](https://github.com/MetaMask/jazzicon/blob/master/index.js) | Background and three transformed rectangles, clipped to a circle, four hue-shifted colors. [MetaMask integration](https://github.com/MetaMask/metamask-extension/blob/develop/ui/helpers/utils/icon-factory.ts) defaults to the first four address bytes: equal prefixes give identical icons. Uniform equal-prefix probability is 2⁻³² per pair, not a visual-collision measurement. | Geometry remains; random color contrast is not guaranteed. [ISC](https://github.com/MetaMask/jazzicon/blob/master/LICENSE). |
| [Boring Avatars](https://github.com/boringdesigners/boring-avatars) | Six variants: 64-cell Pixel; face Beam; geometric Bauhaus; blurred Marble; fixed nine-region Ring; two-band Sunset. [Ring](https://github.com/boringdesigners/boring-avatars/blob/master/src/lib/components/avatar-ring.tsx) and [Sunset](https://github.com/boringdesigners/boring-avatars/blob/master/src/lib/components/avatar-sunset.tsx) have only five outputs with the default cyclic five-color palette. | Some variants depend on hue alone; YIQ text choice is not WCAG validation. [MIT](https://github.com/boringdesigners/boring-avatars/blob/master/LICENSE). |
| [GitHub identicons](https://github.blog/news-insights/company-news/identicons/) | 5×5 binary pattern plus color. The [personal Rust port](https://github.com/dgraham/identicon/blob/master/src/lib.rs) mirrors 15 binary choices (2¹⁵ coarse masks); no established provider perceptual-collision measurement. | Shape survives hue loss. MIT for the personal port; no established reuse license for GitHub’s production generator. |
| [Minidenticons](https://github.com/laurentpayot/minidenticons/blob/main/minidenticons.js) | 15 mirrored binary choices and nine hues: nominal 294,912 SVG states. [Official sample](https://github.com/laurentpayot/minidenticons#collisions): 163 duplicate SVGs in 10,000 random strings (1.63% duplicate fraction, not pair collision or human confusion). | Geometry helps; background/contrast is caller-dependent. [MIT](https://github.com/laurentpayot/minidenticons/blob/main/LICENSE). |
| Solana / Phantom | No canonical Solana account-identicon algorithm established. Phantom documents editable [private account avatars](https://help.phantom.com/articles/manage-your-accounts-in-phantom-28355057809299) and separate [emoji/collectible profile avatars](https://help.phantom.com/articles/manage-your-phantom-profile-12977712693523). Repeated chosen art can be identical; no generated-state count. | Arbitrary artwork has no contrast guarantee; no reusable generator/art license established. |

Choose an original, asymmetric 4×4 archipelago: high-contrast island occupancy,
four large silhouettes and curated sea/kelp/sand/coral colors. Avoid detail,
blur, animation, mirrored layouts and address-prefix seeds. Silhouettes and
layout carry identity independently of hue ([WCAG use of color](https://www.w3.org/WAI/WCAG22/Understanding/use-of-color.html)).

## Normative v1 derivation

Input is exactly 20 bytes. Text accepts exactly 40 ASCII hexadecimal digits,
with optional `0x` or `0X`; casing does not matter. Reject whitespace, truncated
addresses, non-hex characters, ENS names and unsupported versions. Invalid or
missing input has no derived icon; render an unseeded neutral placeholder.

```
seed = SHA-256(UTF8("eastsea-account-icon-v1") || address_bytes[20])
```

There is no terminator or separator, and the 20 bytes are decoded hex, not text.
All byte indexes below are zero-based; bit zero is a byte’s least significant bit.
The tuple fields and order are `(version, palette, layout, shape, rotation)`.

| Field | Source | Range / meaning |
| --- | --- | --- |
| version | explicit API/spec **byte 0x01**; domain includes `v1` | Reject any other version; never silently reinterpret v1. |
| palette | `seed[0] & 7` (bits 0–2) | 0…7, fixed table below. |
| shape | `(seed[0] >> 3) & 3` (bits 3–4) | 0 reef/square, 1 island/disc, 2 cape/triangle, 3 cove/quarter-disc. |
| rotation | `(seed[0] >> 5) & 3` (bits 5–6) | 0…3 clockwise quarter-turns about (32,32). |
| layout | `((seed[1] << 8) \| seed[2]) & 0x3fff` | Fourteen occupancy bits: `seed[2]` bits 0–7 then `seed[1]` bits 0–5. |

`seed[0]` bit 7, `seed[1]` bits 6–7 and `seed[3…31]` are unused in v1.
There is no PRNG and no modulo bias. The public version is part of the feature
tuple, not an extra byte in the prescribed hash input. API defaults explicitly
remain v1; adding v2 requires a new domain/spec/API and published vectors.

Cells are row-major indexes 0…15 before rotation. Cell 0 is always occupied;
cell 15 is always sea. For cell `i` in 1…14, occupancy is layout bit `i-1`.
These anchors prevent empty or solid icons without remapping seeds. The 2²¹
possible tuples are **not** claimed to be 2²¹ distinct images: rotations can
alias layouts. Collision checks therefore compare the actually rotated mask too.

## Normative drawing and colors

Use a 64×64 square view box at every size. Fill an opaque rounded square
`(0,0,64,64)`, radius 12, with the palette color. No strokes, gradients or theme
adaptation. Scale the whole drawing uniformly. Every occupied cell has
`x = 9 + 12*(i%4)`, `y = 9 + 12*floor(i/4)`, ink `#101820`:

| shape | Geometry / canonical SVG |
| --- | --- |
| 0 | `<rect x="x" y="y" width="10" height="10"/>` |
| 1 | `<circle cx="x+5" cy="y+5" r="5"/>` |
| 2 | `<path d="Mx+5 yLx+10 y+10Lx y+10Z"/>` (substitute integer coordinates) |
| 3 | `<path d="Mx yLx+10 yA10 10 0 0 1 x y+10Z"/>` |

Paint occupied cells in ascending index under
`<g fill="#101820" transform="rotate(rotation*90 32 32)">`.
At 16 px each glyph occupies 2.5 px, with a 0.5 px channel between cells and
2.25 px inset. Do not redraw details or choose different features at smaller sizes.
Show icons next to address text; icons are decorative to assistive technology
because the adjacent address is the authoritative identity. No new product copy
is necessary; existing localized account/from/to strings remain in use.

| Index | Palette | Light surface `#f7f5f0` | Dark surface / ink `#101820` |
| --- | --- | ---: | ---: |
| 0 | `#4e8dad` tide | 3.359 | 4.889 |
| 1 | `#368f8b` lagoon | 3.531 | 4.651 |
| 2 | `#73864a` kelp | 3.680 | 4.463 |
| 3 | `#a77a45` sand | 3.496 | 4.698 |
| 4 | `#b36c5c` coral | 3.695 | 4.445 |
| 5 | `#96749e` dusk | 3.644 | 4.507 |
| 6 | `#758694` mist | 3.445 | 4.767 |
| 7 | `#958130` ochre | 3.530 | 4.653 |

Each ink/fill pair exceeds 3:1 internally, and its enclosing fill exceeds 3:1
against both supported light and dark surfaces. Ink never directly touches the
page surface. Different palette fills need not contrast against one another:
identity does not depend on seeing their hues. Contrast uses WCAG sRGB relative
luminance, not YIQ ([WCAG non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html)).

## Frozen vectors and canonical SVG hashes

The shared machine-readable fixture is
[`crates/client/tests/account-icon-vectors.json`](../../crates/client/tests/account-icon-vectors.json).
Swift, JS and Rust must independently check these same vectors. They cover all
eight palettes, all four shapes/rotations, zero/max addresses, single-bit changes,
case normalization and two addresses with matching displayed prefixes/suffixes.
`scripts/account-icon-vectors.py --check` verifies the frozen fixture against an
independent Python hashlib/geometry oracle; tests never regenerate expected data.

The 64 px SVG SHA-256 is over **UTF-8 without a trailing newline**. Opening tag:
`<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64" aria-hidden="true">`;
then `<rect width="64" height="64" rx="12" fill="PALETTE"/>`, the group and
ordered shapes above, and `</g></svg>`. Attribute order, spacing, decimal-free
integers and lower-case colors are normative. PNG artifacts are review snapshots,
not cross-platform byte goldens (OS rasterizers can differ).

<!-- ACCOUNT-ICON-VECTORS:START -->
| Address | Tuple `(v,p,l,s,r)` | SVG SHA-256 at 64 px |
| --- | --- | --- |
| `0x0000000000000000000000000000000000000000` | `1,7,6481,0,2` | `4db767077ac07739ce180680a366b738e734489fa1b4cf7ed1c332ecbdbcd3fb` |
| `0xffffffffffffffffffffffffffffffffffffffff` | `1,5,3096,0,1` | `93c22a2742a1301dbb5256c918104853ea44fbd3e889854f3a688bf3a4c4c234` |
| `0x0000000000000000000000000000000000000001` | `1,0,12226,0,3` | `a438d351bb74bb5288daa7881c07b60e442ce992143323a9980c1923a2d7d62c` |
| `0x0000000000000000000000000000000000000002` | `1,6,1545,0,0` | `679dc7742be8c079dbbf1e746734a10a74d2bf1f2c54547b7126e864cf489775` |
| `0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef` | `1,3,9484,1,3` | `f7da678b06fb47e1221c9ce663ffacf4589e8a0f575cb25ba53894852514cc8a` |
| `0x52908400098527886e0f7030069857d2e4169ee7` | `1,0,14072,3,1` | `e06984cd2d1dab2268624ddd7c45c3d750981ee10019268de872a62b05574c0f` |
| `0x00000000000000000000000000000000000000c1` | `1,0,9025,0,0` | `3abd1892f803a48f253f74787da553820e843f38e5413677030761f10390ade8` |
| `0x1234567890abcdef1234567890abcdef12345678` | `1,1,1473,3,3` | `d0c18373a7cd4b3e056c3b9213f7348ada7e746ad6a23c319b13c9c01ac29b3d` |
| `0x1234567890abcdef0000000000abcdef12345678` | `1,2,10922,3,1` | `6436ade62dba6f71bfe62e6f7af1ba0eeb24405c29e749280a2217209596f96d` |
| `0xa2521982a17474cb2f8741c85de653b5282d72b0` | `1,1,7256,1,3` | `69f51da788a2465a360efbd2c11d438e15c61371055c4431d4277afa968bd9cb` |
| `0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416` | `1,1,13258,2,0` | `032126596d49cfb64922f3ccb53d07901bd36b9301528fbffae3796e01d668cb` |
| `0x961f8add5ae93ff0700be8abd5f9f8ec69ba4347` | `1,7,14725,2,1` | `08608bac48fb619e601fad8c6c01b3a5d9926b26d0030f08a72fadb24ae33885` |
| `0xc91367bac92c6de822de8afd0f34ff19fd8f7670` | `1,3,6700,0,0` | `9841176d191e181e57acbcf8c0a71d1289f484661bbb92c7ea5a7ef88bd83d1c` |
| `0x00000000000000000000000000000000000000ff` | `1,6,7093,0,1` | `7af107b047fb0bfd4335ac0c8192df63e8bb8ee2c368055580c333b12d3e3c26` |
| `0x0000000000000000000000000000000000000100` | `1,4,3593,1,3` | `a91f594378f4df1faa170f3a7e5fa3b4a4d25df8ec6c27eb929fc0fca9cb18db` |
| `0x000000000000000000000000000000000000ffff` | `1,0,12595,2,2` | `9530a37f28adf2af6fccda3a6d65e2f58d77d1f19f35897839ff3f99dca6729e` |
<!-- ACCOUNT-ICON-VECTORS:END -->

## Measured distinguishability (100,000 addresses)

The reproducible stream is
`SHA-256(UTF8("eastsea-account-icon-measure-v1") || UInt32BE(i))[0..20]`,
for `i = 0…99,999`. This measurement-only address generation is independent
of the icon hash domain. The denominator is all **4,999,950,000 unordered pairs**,
not the fraction of addresses that repeat an earlier icon.

| Equality signature | Matching pairs | Pair fraction | Approximately 1 in |
| --- | ---: | ---: | ---: |
| Raw feature tuple | 2,398 | 0.000000479605 | 2,085,050 |
| **All coarse visual features**: palette, rotated mask, glyph style and visible orientation | **3,019** | **0.000000603806** | **1,656,161** |
| Palette + rotated occupancy mask; discard glyph details | 14,335 | 0.000002867029 | 348,793 |
| Grayscale occupancy mask only; discard palette and glyph details | 114,412 | 0.000022882629 | 43,701 |

The requested target is below 0.0001 (1 in 10,000). Even the conservative
mask-only comparison passes. Squares/discs have no visible orientation; their
rotational aliases are counted together. Triangles/quarter-discs retain their
visible orientation. These equality signatures do not measure human confusion,
near matches or deliberate attacker grinding.

The [raw report](46-account-icon/measurement.json) records all contrast pairs,
the stream and runtime. The [resource report](46-account-icon/remote-resources.json)
records the guarded poc-m3 run, nice 15, RAM/disk limits and successful owned
process cleanup. The sub-second process was sampled at 0.5-second intervals;
reported sampled RSS is not a precise peak-memory benchmark. All owned
measurement staging and processes were removed after retrieving the reports.

## Verification and review artifacts

The collision report compares every unordered pair in 100,000 pseudorandom
20-byte addresses generated from a reproducible measurement-only stream. Icons
themselves use no runtime randomness. Report raw tuple collisions, rendered
coarse mask/style/palette collisions, palette+mask collisions, and grayscale
mask-only collisions; count aliases after rotation, not only seed combinations.
These are exact equality rates, never claimed human-recognition rates.

Measurements run under `~/eastsea-lab/account-icon` on poc-m3 at nice 15, with
available RAM ≥4 GiB, disk ≥30 GiB and owned memory ≤12 GiB; the owned processes
are stopped and cleaned up afterward. Builds/tests use the prescribed lane
runners and do not compile guests or launch EastSea.

Snapshots and the measured report live in [46-account-icon/](46-account-icon/).
Static Swift rendering uses only the icon spec/view, never the wallet app,
node, account store or keys. Render all vectors at 16/32/64 px in light/dark,
including a full-size comparison sheet. The same canonical JS module is shipped
inside each static app; its mirrors must be byte-identical before commit.

Passed checks for this lane:

- Swift: `scripts/test-swift-pure.sh account-icon` checks all 16 shared tuples
  and canonical SVG hashes, strict normalization/rejection, anchors, rotations
  and contrast. Static SwiftUI rendering and an isolated MenuBarExtra scene
  typecheck pass; no wallet app was built or launched.
- Extension: **126/126** npm tests; explorer: **67/67** npm tests. This worktree
  lacked generated WASM, so exact source copies and the existing matching
  `/Volumes/workspace/aether-node/target/wasm-pkg` artifacts were staged under
  `./tmp/account-icon-js-check` without rebuilding. All 86 staged source and
  fixture files were compared byte-for-byte with this branch.
- Rust: `scripts/dev-test.sh --remote --changed-file crates/client/src/account_icon.rs`
  compiled the client on poc-m3, but the runner’s offline workspace metadata
  step failed on an uncached, unrelated `aead 0.6.1` dependency before running
  tests. The permitted local final gate, `scripts/dev-test.sh --local` with
  the same selection, passed **4/4** after the compile semaphore. Client-only
  rustfmt and clippy with `-D warnings` pass; no guest was built.
- Python oracle, JS mirror/syntax checks, Swift source syntax, shell syntax,
  whitespace checks and localization lint/catalog checks pass. Existing
  `Account`, `To` and `Contract` strings are complete in all five languages;
  this change adds no product strings.
- Independent code review found no material blockers. Protected crates,
  signing logic, key material and account state were untouched.

The full wallet integration build and live menu-bar behavior are reserved for
the lead. The menu-bar label uses an NSImage rendered from the same icon view
at 16 logical points / 32 Retina pixels and preserves the original prover cube
signal. Its isolated render and typecheck passed, but this is not a claim of a
live wallet runtime check.

Remaining limits: a finite icon space permits deliberate grinding and some
addresses collide. A close visual match is not proof of address equality. Tiny
silhouettes and palette differences need human review; exact equality and WCAG
contrast alone do not prove anti-phishing effectiveness.
