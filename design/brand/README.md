# EastSea brand art

Final artwork for EastSea (동해), the Mac-first wallet and node, and its native Doubloon (DBLN, 더블룬). Created with Codex's built-in `image_gen.imagegen` tool on 2026-10-07. The original gold dawn-over-waves doubloon established the motif; these are new artwork and platform exports.

The family uses a warm gold dawn over deep sea blue: a substantial semicircular sun, short rays and broad flowing waves. The app mark omits stars and fine ornament; the large native coin retains eight stars and a restrained satin-gold rim. There is no text in any icon or token image. The social preview contains only “EastSea”.

## Files

Paths in this table are relative to the repository root.

| File | Pixels / format | Alpha | Use |
| --- | --- | --- | --- |
| `design/brand/app-icon-1024.png` | 1024 × 1024 PNG | Transparent corners and outer margin | macOS master. An exact 824 × 824 rounded-square body at (100, 100), radius 185, with a subtle external shadow. |
| `design/brand/app-icon-ios-1024.png` | 1024 × 1024 PNG | Fully opaque RGB | iOS master. Full bleed with square outside corners; iOS supplies the mask. |
| `design/brand/app-icon-32.png` | 32 × 32 PNG | Transparent corners | Dedicated small app icon for 16–32 px. Hand-simplified, enlarged dawn motif, one broad wave, three rays, no rim, shading or shadow. |
| `design/brand/dbln-coin-1024.png` | 1024 × 1024 PNG | Transparent exterior | Large DBLN coin for 48 pt and above. Centered 944 px circular face, satin-gold relief, eight stars. |
| `design/brand/dbln-coin-flat-256.png` | 256 × 256 PNG | Transparent exterior | Native coin for rows under 48 pt. Solid gold/navy dawn and two waves; no metal texture or stars. Its 232 px disc matches the token grid. |
| `design/brand/dawn-flat.svg` | 256 × 256, 32-unit viewBox | Transparent exterior | Editable vector source for the hand-reduced flat native coin. Paths and two solid fills only. |
| `design/brand/tokens/WAETH-256.png` | 256 × 256 PNG | Transparent exterior | Original wrapped-sunrise mark: cream cradle arcs, gold sunrise and cream sea on blue. |
| `design/brand/tokens/NEB-256.png` | 256 × 256 PNG | Transparent exterior | Original three-star constellation on plum. |
| `design/brand/tokens/ORB-256.png` | 256 × 256 PNG | Transparent exterior | Original gold planet and cream orbit on teal. |
| `design/brand/tokens/CMT-256.png` | 256 × 256 PNG | Transparent exterior | Original cream comet head and two gold tail ribbons on night indigo. |
| `design/brand/org-avatar-1024.png` | 1024 × 1024 PNG | Fully opaque RGB | Square avatar for the `eastsea-xyz` organization; ample navy around the simpler medallion also accommodates circular cropping. |
| `site/assets/og-1200x630.png` | 1200 × 630 PNG | Fully opaque RGB | Social-sharing card: the coin beside “EastSea” in clean navy sans-serif type on parchment. |
| `site/assets/favicon-32.png` | 32 × 32 PNG | Transparent exterior | Small browser favicon. Flat dawn with one wave, drawn from the SVG below. |
| `site/assets/favicon-180.png` | 180 × 180 PNG | Fully opaque RGB | Touch/bookmark icon. Square full-bleed navy, with the flat dawn centered inside; the platform can apply its own mask. |
| `site/assets/favicon.svg` | 32 × 32, 32-unit viewBox | Transparent exterior | Clean vector favicon, also the source geometry for the hand-simplified small app icon. No fonts, embedded raster, script or external resource. |

There are **13 final PNGs and two SVG sources**. The existing `site/assets/favicon-32.png` is replaced; the placeholder assets under `design/assets/` remain available as references.

## Shipped token list

Read from `apps/wallet/Sources/TokenGuard.swift` (`KnownTokens`), with classification and visual conventions checked against `TokenIconSpec.swift` and `TokenIconView.swift`. The list is **not empty**: it ships these four ERC-20 entries on legacy test chain **7780**.

| Symbol | Shipped name | Address on chain 7780 |
| --- | --- | --- |
| WAETH | Wrapped AETH | `0xa2521982a17474cb2f8741c85de653b5282d72b0` |
| NEB | Test Nebula | `0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416` |
| ORB | Test Orbit | `0x961f8add5ae93ff0700be8abd5f9f8ec69ba4347` |
| CMT | Test Comet | `0xc91367bac92c6de822de8afd0f34ff19fd8f7670` |

Each token has a centered 232 px solid circular disc inside its 256 px canvas. The colour and original motif differ; visual weight, cream/gold accents and circular edge treatment are shared. WAETH uses the wallet's own coastal sunrise and wrapping arcs, with no Ethereum diamond. No USDC, ETH or other project's logo was used. File symbols label the assets; the shipped address/chain classification remains the authority for their use.

## Export and small-size treatment

The app, large coin, token motifs, organization avatar and social card were generated with the built-in image tool. Selected square sources were 1254 × 1254; the social source was 1730 × 909. Pillow 12.1.0 was used for final PNG sizing, exact platform geometry and verification.

The macOS body is the generated full-bleed app artwork resized to 824 px, placed at (100, 100), and clipped to the rounded-square grid. Its shadow has a 14 px blur, a 10 px downward offset and a restrained 20% maximum alpha. iOS retains the same generated artwork, scaled to 1024 px, entirely opaque and square.

Circular exports were centered from the disc's bounds and sampled four source pixels inside the perimeter to exclude extraction flecks and mixed edge matte. Exact antialiased circular apertures give the final PNGs fully opaque interiors and genuinely transparent exteriors. The ORB transparent-generation attempts produced holes and mottled patches; they were rejected. The selected ORB is a fresh clean opaque source with its cream production backdrop excluded by the same circular export aperture.

Small marks are deliberately **hand-reduced from the generated dawn**, rather than miniature photographs: generous navy cutouts, three short rays, and one wave for the small app/favicon; two waves for the 256 px flat coin. The metallic flat-coin attempts retained shading and were rejected in favour of the true two-fill vector reduction in `dawn-flat.svg`. PNGs from the small vector paths were rasterized with 8× supersampling. The editable favicon SVG contains the one-wave geometry.

## Verification

Pillow checks passed for all 13 final PNGs: exact pixel dimensions, valid PNG format, expected alpha range, and the expected four-corner alpha values. The macOS body mask was measured at exactly `(100, 100, 924, 924)`, an 824 px body.

The large coin and four official tokens were additionally checked for centered circular bounds, fully opaque interiors, and no stray alpha outside their antialiased edges. Flat-coin fill checks confirmed the solid gold `#E8BF59` and navy `#0D2135` colours. Both SVGs parse correctly and contain only local vector geometry and solid fills.

Every final result was inspected, including full masters, coin/token edges on white and navy, the small assets, and 16/24/32/48 px previews. An independent visual review passed at **94/100**, with no blocking defects. Temporary generation sources, rejected variants, measurements and review sheets live under `tmp/brand-art/` and are excluded from the commit.

This is the design-asset handoff. Swift/Rust source, app asset catalogs and site HTML were unchanged; no app compilation was run.

## Image-generation prompts

These are the exact prompts used, including rejected iterations. “Transparent” below means the built-in tool's `transparent_background: true`; “opaque” means `false`. References describe the supplied generated image, not another project's logo.

### 1. Full-bleed app source — selected

Opaque; no input image.

```text
Use case: logo-brand.
Asset type: final full-bleed iOS app icon master for EastSea, a calm trustworthy Mac-first consumer finance wallet. Create one square 1024 x 1024 image, opaque edge to edge, absolutely no rounded outside corners.
Design: a single warm gold circular doubloon medallion centered on a deep East Sea navy field (#0D2135). The medallion is about 640 px diameter and is front facing, perfectly circular, visually centered. Inside the gold disc is an original simplified dark navy sunrise over the sea: one bold semicircular sun above one broad flowing wave and one shorter wave below. Only three short substantial sun rays, spaced generously. Strong simple solid geometry, thoughtful negative space; the main silhouette must survive a 16 px app icon.
Material: very restrained satin gold with a single subtle edge, no microscopic detail; elegant smooth sea-blue enamel backdrop with very gentle light from upper left. Flat-first geometry with a little crafted depth, consumer finance dignity. No rings of stars in this small icon, no thin ornamental lines. Gold #E8BF59 and amber #C99838, ocean #0D2135, no neon.
Composition: medallion centered at (512,512), no tilt, no perspective, no props. Square navy background fills all four corners. Do not render a device mockup.
Constraints: absolutely no text, letters, digits, watermark, badges, initials, existing brand logos, Ethereum diamonds, Bitcoin marks, photographic noisy coin texture, stock clipart, casino look, lens flare or shiny plastic.
```

### 2. Large coin — first attempt, rejected for edge flecks

Transparent; no input image. The existing coin supplied motif context only.

```text
Use case: logo-brand / stylized-concept.
Asset type: EastSea native Doubloon coin, 1024 x 1024 PNG with TRUE transparent background.
Supporting reference: the existing gold sunrise-over-waves doubloon is a motif reference only. Make a fresh original modern coin retaining sunrise, ocean waves and restrained stars around the rim.
Subject: one centered front-facing perfectly circular warm-gold doubloon, 912 px diameter with clean continuous circular perimeter, ample transparent margin, no shadows outside its edge, no perspective, no tilt, no ellipse. Fine satin metal with restrained bevel and soft top-left studio light. Quiet confident crafted object, no excessive shine or aged scratches.
Face: a large clean simplified sunrise above two flowing sea waves, engraved in deep amber/navy relief so it is readable. Semicircular sun with just five substantial rays. A calm balanced wave motif with broad curves and generous spacing, not a detailed storm. Around the rim only eight small simple five-point stars, evenly spaced with an elegant thin inner rim; no tiny beading or ornate filigree. Warm gold #DDB052 / #E8BF59; deep etched shadows #785519 / #132B3C.
Constraints: TRUE alpha transparency outside the coin including corners. Crisp antialiased edge with no white/black matte, fringe, glow, cast shadow, loose pixels. No text anywhere, no letters, denominations, digits, symbols from existing currencies, people, emblems from other projects, watermark. Single coin only; object is centered and entirely visible.
```

### 3. Coin perimeter revision — selected for circular export

Transparent; reference: generated first coin.

```text
Use case: precise-object-edit. Asset type: final transparent EastSea Doubloon coin.
Input image: the generated coin is the EDIT TARGET; preserve its warm gold sunrise, two navy flowing waves and eight rim stars, centered front view and restrained satin-gold material.
Change only the perimeter and framing: make the outer edge a perfectly smooth machined circle, remove ALL white flecks, colored sparks, loose alpha pixels and ragged fringe outside the rim. Remove rough texture on the outermost circumference. No cast shadow or halo outside the circle. Make the coin perfectly centered within the square, with ample equal transparent margins, fully visible.
Keep the face motif and eight stars without any changes. Keep the modest top-left light. TRUE alpha transparency outside the smooth circle. The silhouette must be absolutely clean on both white and dark backgrounds. No text anywhere, no watermark, no extra decorative elements. Produce a single final 1024 x 1024 PNG.
```

### 4. Flat coin — first attempt, rejected for shading

Transparent; reference: generated full-bleed app.

```text
Use case: style-transfer / logo-brand. Asset type: flat Doubloon coin for token rows smaller than 48 pt. Input image is the generated EastSea app icon as IDENTITY REFERENCE. Make one flat token icon with the same native coin sunrise and waves, no app background.
1024 x 1024 PNG, TRUE alpha outside a perfect centered warm-gold circle of 896 px diameter. Circle fill is solid warm gold #E8BF59. The symbol is deep sea navy #0D2135: large semicircular rising sun, exactly three substantial short rays and two broad flowing waves, same silhouette as the reference. Remove all metallic detail, gradients, outlines, decorative rim, shadow, stars and texture. Large bold shapes, generous negative space, exact visual centering, no tilt.
Absolutely no text, letters, digits, watermark, or existing cryptocurrency logo. Clean smooth circular outer edge, no fringe or stray alpha.
```

### 5. WAETH — selected

Transparent; no input image.

```text
Use case: logo-brand. Asset type: official token art in EastSea, a calm trustworthy consumer finance wallet. Single original emblem, square 1024 x 1024 PNG for eventual 256 px export. TRUE transparent background outside a centered perfect circular disc. Disc centered at (512,512), 896 px diameter. Flat solid ink colours, thick smooth geometric shapes, no texture or gradients, no shadows or bevel, clean antialiasing. Use the whole canvas with even transparent margin. A quiet clear mark legible at 16–32 px, visual weight and proportions shared with a warm gold sunrise-over-waves doubloon. Only one mark per image. NO letters, text, ticker, digits, watermark, badges, checkmarks or question marks. No existing project's logo, no Ethereum diamond or faceted rhombus, no Bitcoin B, no USDC or currency logo. Continuous smooth circle edge, no fringe, glow or stray particles. Colours should be saturated enough to be distinct from the wallet's pastel dashed unverified glyphs, yet restrained and dignified. Subject: Wrapped AETH, the wallet's own wrapped legacy chain asset, not Ethereum. Disc deep sea blue #175B83. Cream #F5EAD1 original mark consisting of TWO broad open circular cradle arcs around a small central warm-gold sunrise and one cream wave. The arcs are thick smooth cupped arcs, separated at the upper right and lower left, wrapping the sunrise as a sheltered sea. Make the core motif boldly legible and well centered. Do NOT use any diamond, rhombus, hexagon, letter, currency glyph or Ethereum resemblance. Only the sunrise and protective arcs.
```

### 6. NEB — first attempt, rejected for imperfect interior alpha

Transparent; no input image.

```text
Use case: logo-brand. Asset type: official token art in EastSea, a calm trustworthy consumer finance wallet. Single original emblem, square 1024 x 1024 PNG for eventual 256 px export. TRUE transparent background outside a centered perfect circular disc. Disc centered at (512,512), 896 px diameter. Flat solid ink colours, thick smooth geometric shapes, no texture or gradients, no shadows or bevel, clean antialiasing. Use the whole canvas with even transparent margin. A quiet clear mark legible at 16–32 px, visual weight and proportions shared with a warm gold sunrise-over-waves doubloon. Only one mark per image. NO letters, text, ticker, digits, watermark, badges, checkmarks or question marks. No existing project's logo, no Ethereum diamond or faceted rhombus, no Bitcoin B, no USDC or currency logo. Continuous smooth circle edge, no fringe, glow or stray particles. Colours should be saturated enough to be distinct from the wallet's pastel dashed unverified glyphs, yet restrained and dignified. Subject: Test Nebula. Disc muted saturated plum #604A7D. Three cream #F5EAD1 four-point stars, two larger and one slightly smaller, arranged as a balanced triangular constellation around the centre. Their arms have gently rounded inward curves; generous spacing. The three stars form a composed central emblem, not a random particle field. Only three stars, no dots, orbit rings or decorations.
```

### 7. ORB — first attempt, rejected for background artifacts

Transparent; no input image.

```text
Use case: logo-brand. Asset type: official token art in EastSea, a calm trustworthy consumer finance wallet. Single original emblem, square 1024 x 1024 PNG for eventual 256 px export. TRUE transparent background outside a centered perfect circular disc. Disc centered at (512,512), 896 px diameter. Flat solid ink colours, thick smooth geometric shapes, no texture or gradients, no shadows or bevel, clean antialiasing. Use the whole canvas with even transparent margin. A quiet clear mark legible at 16–32 px, visual weight and proportions shared with a warm gold sunrise-over-waves doubloon. Only one mark per image. NO letters, text, ticker, digits, watermark, badges, checkmarks or question marks. No existing project's logo, no Ethereum diamond or faceted rhombus, no Bitcoin B, no USDC or currency logo. Continuous smooth circle edge, no fringe, glow or stray particles. Colours should be saturated enough to be distinct from the wallet's pastel dashed unverified glyphs, yet restrained and dignified. Subject: Test Orbit. Disc deep sea teal #207568. Centered warm-gold #E8BF59 circular planet with a single broad cream #F5EAD1 orbit ellipse tilted 24 degrees from horizontal, passing around the planet. Clear front/back interruption gives a readable orbit. All shapes are flat. The central circular planet and encircling orbit fill roughly 60% of the disc and are perfectly balanced. No crescent moons, extra stars or dots.
```

### 8. CMT — selected

Transparent; no input image.

```text
Use case: logo-brand. Asset type: official token art in EastSea, a calm trustworthy consumer finance wallet. Single original emblem, square 1024 x 1024 PNG for eventual 256 px export. TRUE transparent background outside a centered perfect circular disc. Disc centered at (512,512), 896 px diameter. Flat solid ink colours, thick smooth geometric shapes, no texture or gradients, no shadows or bevel, clean antialiasing. Use the whole canvas with even transparent margin. A quiet clear mark legible at 16–32 px, visual weight and proportions shared with a warm gold sunrise-over-waves doubloon. Only one mark per image. NO letters, text, ticker, digits, watermark, badges, checkmarks or question marks. No existing project's logo, no Ethereum diamond or faceted rhombus, no Bitcoin B, no USDC or currency logo. Continuous smooth circle edge, no fringe, glow or stray particles. Colours should be saturated enough to be distinct from the wallet's pastel dashed unverified glyphs, yet restrained and dignified. Subject: Test Comet. Disc dark night-indigo #303F69. One centered diagonal comet travelling from lower-left to upper-right: a substantial cream #F5EAD1 circular head at upper right connected to TWO broad gently tapering warm-gold #E8BF59 tail ribbons trailing toward lower left. The complete head-and-tail silhouette must be optically centered within the disc with generous padding. Flat colours and rounded gentle curves. No extra stars, particles, text, or rocket shape.
```

### 9. Flat-coin revision — rejected for retained shading

Transparent; reference: first generated flat coin. Final flat asset is the hand-reduced SVG rendition.

```text
Use case: style-transfer. Asset type: final flat EastSea Doubloon token-row icon.
EDIT TARGET: the supplied gold disc with dark sunrise and waves. Preserve the silhouette exactly: gold circular coin, navy semicircle sun, three short rays, two flowing waves.
Change only the rendering to strictly FLAT SOLID FILLS. Gold must be one uniform colour #E8BF59 over the entire circular disc. Navy shapes must be one uniform colour #0D2135. No gradients, lighting, material, embossing, rim, texture, shadow, haze, mottling, colour variation, tiny dots or decorative detail anywhere. Treat it as a two-ink vector illustration, not a physical object.
Disc itself completely opaque: no transparency or holes inside it. Only OUTSIDE the perfect circle is truly transparent. Keep the circle perfectly centered with identical padding on each side. Smooth pristine continuous circular edge, no fringe or loose pixels. No text or watermark. Single 1024 x 1024 image for export at 256 px.
```

### 10. ORB field revision — rejected for retained artifacts

Transparent; reference: first generated ORB.

```text
Use case: precise-object-edit. Asset type: final official Test Orbit token icon.
EDIT TARGET: the supplied ORB icon. Preserve the centered gold circle and cream ellipse orbit and their current balanced geometry.
Change only the backdrop: replace ALL mottled, dark, scratched, wispy, translucent or textured patches with one completely smooth, UNIFORM SOLID teal #207568 disc. Every pixel INSIDE the disc must be completely opaque. Do not add any stars, particles, clouds, scratches or ring shapes. The gold planet is solid #E8BF59 and the orbit is solid cream #F5EAD1. Three flat ink colours only, no gradients or lighting or texture.
Keep the perfect circular disc centered with equal transparent margins. Only outside the smooth outer circle has TRUE alpha transparency; no holes inside. Original emblem, no text, digits, watermark or existing logo. Single 1024 x 1024 transparent PNG.
```

### 11. Organization avatar — selected

Opaque; reference: generated full-bleed app.

```text
Use case: logo-brand. Asset type: square organization avatar for EastSea, a calm trustworthy consumer finance product. Input image is the approved EastSea app icon identity reference.
Create one 1024 x 1024 square, opaque full bleed navy #0D2135 in all corners. Reuse the SAME gold doubloon sunrise-over-two-waves identity from the reference: warm gold circular medallion, navy semicircle sun, three short rays and two broad smooth flowing navy waves, all centered.
Make an elegant flatter avatar interpretation, with smooth simple shapes and very restrained satin-gold depth, no ornate rim. Gold circle about 700 px diameter, enough navy margin to survive circular organization-avatar cropping. Sunrise and waves bold, optically centered, balanced and quiet. Front-on, no tilt or perspective.
No letters, initials, text, symbol ticker, watermark, badges, neon, existing currency logo, photographs, particles, or extra decorations. Original EastSea dawn-on-the-sea medallion only. Filled square outside; no rounded square border and no transparent outside corners.
```

### 12. Social preview — selected

Opaque; reference: generated perimeter-revised coin.

```text
Use case: ads-marketing. Asset type: final EastSea social-sharing image, WIDE LANDSCAPE 1200 x 630 aspect ratio (approximately 1.9:1), not square.
Input image: the generated gold EastSea Doubloon is the coin identity reference. Compose one quiet sophisticated social card on a completely opaque warm parchment #F3EFE3 backdrop. On the LEFT, the supplied gold sunrise-over-waves doubloon with eight rim stars, front facing, about 370 px diameter, centered at approximately (315,315), full coin visible. Restrained satin gold, soft natural light, no glow or shadow outside a very subtle contact shadow.
On the RIGHT, only the word "EastSea" in very clean precise dark navy #0D2135 modern humanist sans-serif type, like Avenir Next Medium, about 96 px tall. Exact spelling and casing: capital E, lowercase ast, capital S, lowercase ea. Text (verbatim): "EastSea". One line, no subtitle, punctuation, slogan or ticker.
Align the coin's visual middle and the word's visual middle at the card's middle. Broad generous margins. Composition extends horizontally with ample quiet negative space and all elements kept inside the central 1100 x 520 safe area. One clean coin and one word only. No other text, letters on the coin, trademarked logos, watermark, UI mockup, decoration, gradients, lines, waves in background, or crypto-casino styling.
```

### 13. ORB fresh transparent source — rejected for field holes

Transparent; no input image.

```text
Use case: logo-brand. Make an ORIGINAL 2D VECTOR-STYLE FLAT ICON, not a photograph, not a metal coin, not a textured illustration. A Test Orbit token emblem for EastSea, a calm finance wallet.
Only three completely UNIFORM SOLID ink colours: deep teal #207568 for a PERFECT circular disc, gold #E8BF59 for a centered circular planet, cream #F5EAD1 for ONE diagonal ellipse orbit around it. Circle and orbit make a simple balanced Saturn-like celestial mark. Circle planet roughly 38% of disc diameter, orbit 66% of disc diameter with substantial stroke, tilt minus 24 degrees. No other shapes. Disc fills 88% of square canvas and is perfectly centered with equal padding.
All pixels INSIDE the teal disc must be solid, smooth and completely opaque. The only transparent pixels are OUTSIDE the outer disc. TRUE alpha outside. No gradients or shading at all. No texture, vignette, scratches, clouds, haze, dark patches, surface lighting, edge lighting, hole or shadow. NO TEXT, numbers, logos from another project, stars, dots, badges or watermark. Clean flat SVG illustration appearance, high contrast, legible at 16–32 px. Output one 1024 x 1024 transparent PNG.
```

### 14. NEB fresh source — selected

Transparent; no input image.

```text
Use case: logo-brand. Make an ORIGINAL 2D VECTOR-STYLE FLAT ICON, not a photograph, coin or textured illustration. A Test Nebula token emblem for EastSea, a calm finance wallet.
Only two UNIFORM SOLID ink colours: plum #604A7D for a PERFECT centered circular disc filling 88% of square canvas; cream #F5EAD1 for THREE four-point stars in a balanced triangular arrangement within the central 62% of the disc. Two substantial stars and one slightly smaller star, elegantly rounded concave sides, generous negative space. Stars are geometric shape glyphs, never glowing lights.
All pixels INSIDE the plum disc must be solid, smooth and completely opaque. The only transparency is OUTSIDE the perfect circle, TRUE alpha with equal margins. No gradients, texture, scratches, dithering, shading, mottling, dark or light patches, transparent holes or shadow. NO TEXT, numbers, existing-project logos, extra dots, rings, halos, particles, badges or watermark. High contrast, plain flat SVG illustration appearance, legible at 16–32 px. Single 1024 x 1024 transparent PNG.
```

### 15. ORB clean production source — selected for circular export

Opaque; no input image. The final delivered token PNG has a transparent exterior.

```text
Use case: logo-brand. Asset type: opaque production source for a Test Orbit token icon. Square 1024 x 1024. Background is a completely SOLID warm cream #F3EFE3, full bleed, entirely OPAQUE; do not remove the background, do not make anything transparent.
Draw one perfect flat teal #207568 circular disc, centered, 896 px diameter. On the disc, one gold #E8BF59 circle planet at the precise centre, about 38% of the disc diameter, and ONE cream #F5EAD1 elliptical orbital ring tilted minus 24 degrees around the planet. Orbit spans 66% of the disc diameter, broad smooth stroke with a natural front/back interruption around the planet. Original quiet restrained emblem for EastSea, no existing brand logo.
This is a CLEAN TWO-DIMENSIONAL FLAT GRAPHIC: all fills UNIFORM SOLID COLOURS, like an exact SVG or printed ink. No gradients, shading, light, patina, vignette, surface noise, scratches, blurring, haze, glow, holes, shadow, marks or mottled patches anywhere. Teal field must be perfectly uniform from edge to edge. Only the circle planet and one orbital ellipse on the teal disc. All parts entirely opaque. No text, letters, digits, extra stars, extra orbital lines, dots, watermark, border or mockup. Smooth exact geometric shapes.
```
