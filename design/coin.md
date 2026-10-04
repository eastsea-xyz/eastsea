# Doubloon (DBLN) Coin Art Direction

Author: Designer (site-designer agent)  
Date: 2026-10-04  
Status: Art direction only. No image generation capability in this agent.

---

## What the Coin Is

The Doubloon (DBLN) is EastSea's native coin. Its visual identity must work at five scales:

| Context | Size | Format |
|---------|------|--------|
| Website hero | 260–320px | SVG (primary) or raster PNG |
| App icon master | 1024×1024px | Raster PNG (needs image generation) |
| Token row in wallet | 18–24px | SVG or raster PNG @2x |
| Favicon | 32×32px, 16px | SVG embedded in HTML |
| og:image | 1200×630px | Raster composition |

The SVG version (this document) covers: hero, token row, favicon. The raster version (image generation) covers: app icon, og:image.

---

## Shape and Form

**Shape:** Perfect circle. No irregular strike — this is a designed digital coin, not a hand-struck historical artifact. The precision is part of the brand.

**Diameter proportions:**

```
outer ring    ←  full diameter
bezel ring    ←  94% of outer
field ring    ←  88% of outer (inner ring / inner bezel)
relief area   ←  78% of outer (the motif lives here)
```

**Edge:** Reeded. 72 thin radial lines around the outer circumference (visible at 260px+, omitted at 24px and below).

---

## Relief Motif: Dawn Over the East Sea

The DBLN doubloon shows the East Sea at dawn. The motif divides the relief area horizontally:

```
     upper half: the dawn sky
     ─────────────────────────── horizon line (implied)
     lower half: the sea
```

### Upper half — Dawn sky

- A rising sun: a semicircle (half-circle, flat bottom at the horizon) centered at the top of the motif
- The sun has 8 radiating lines extending outward, each line 1.5× the sun radius, alternating long and short (like a traditional compass rose or Japanese mon design)
- The sky area between the sun and the bezel ring is empty (negative space) — no filler marks

### Lower half — Sea

Three rows of stylized waves, each row a series of overlapping arcs:
- Row 1 (nearest, largest): 4 wide arcs, each 25% of the relief width
- Row 2 (mid distance): 5 narrower arcs, slightly above row 1
- Row 3 (distant, smallest): 6 tight arcs, approaching the horizon line

The wave style references Hiroshige wave prints (overlapping arc form) adapted to numismatic engraving. Lines are not equal weight — outer edge of each arc is heavier (2px) than the interior return (1px).

### Text on the coin

- Top arc (following the bezel ring): "동  해" — wide letter-spacing, Pretendard 700, 8% of coin diameter font-size
- Bottom arc (following the bezel ring): "D B L N" — same tracking, same size, follows the lower curve
- Center (below the horizon, above the bottom wave row): "1 DOUBLOON" — small caps, 6% of diameter

On the 18px token icon, all text is dropped. Only the dawn-wave motif reads at small scale.

---

## Metal Treatment

### Lighting model

Light source: 35° from upper-left. This creates:
- Bright highlight on the upper-left rim and the upper-left surface of the relief
- Deep shadow in the lower-right rim and in the valleys between wave arcs

### SVG color strategy

The coin is NOT a gradient of a single gold — it uses layered elements:

1. **Base fill:** `radialGradient` — center `#DFB038` (warm bright gold), outer rim `#8A5C0C` (deep amber-brown)
2. **Specular highlight:** A white-to-transparent `radialGradient` positioned at 35% X / 30% Y (upper left), radius 25% of coin, `opacity: 0.35`
3. **Relief shadows:** The wave arcs and sun rays cast shadows. Each relief element (SVG path) is drawn TWICE:
   - Shadow layer: offset 1.5px down-right, `fill: rgba(80,40,0,0.4)`, `filter: blur(1px)`
   - Highlight layer: offset 0.5px up-left, `fill: rgba(255,220,130,0.25)`
   - The path itself: `fill: var(--coin-fill)` with a slight gradient
4. **Rim:** A `stroke-only` ring at the bezel diameter, `stroke: #A07015`, `stroke-width: 2.5px`
5. **Outer shadow (drop):** `filter: drop-shadow(0 6px 20px rgba(60,30,0,0.55))` on the coin group

### CSS color tokens (defined in coin.md, imported to design/direction.md)

```
--coin-base:    #C8911A   (mid gold)
--coin-bright:  #F0CB5A   (highlight)
--coin-shadow:  #7A4E08   (valley shadow)
--coin-rim:     #9A6810   (outer ring)
--coin-deep:    #4A2804   (deepest recess)
```

---

## SVG Structure Plan

```svg
<svg viewBox="0 0 200 200" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <!-- 1. Base gold gradient -->
    <radialGradient id="coin-base" cx="42%" cy="38%">
      <stop offset="0%"   stop-color="#F0CB5A"/>
      <stop offset="45%"  stop-color="#C8911A"/>
      <stop offset="100%" stop-color="#7A4E08"/>
    </radialGradient>
    <!-- 2. Specular highlight -->
    <radialGradient id="coin-spec" cx="35%" cy="30%">
      <stop offset="0%"   stop-color="white" stop-opacity="0.4"/>
      <stop offset="100%" stop-color="white" stop-opacity="0"/>
    </radialGradient>
    <!-- 3. Inner rim gradient (for the field) -->
    <radialGradient id="field-grad" cx="45%" cy="40%">
      <stop offset="0%"   stop-color="#D4A428"/>
      <stop offset="100%" stop-color="#9A6010"/>
    </radialGradient>
    <!-- 4. Drop shadow filter -->
    <filter id="coin-shadow" x="-15%" y="-15%" width="130%" height="140%">
      <feDropShadow dx="0" dy="6" stdDeviation="8"
                    flood-color="#50300A" flood-opacity="0.55"/>
    </filter>
    <!-- 5. Relief emboss filter -->
    <filter id="relief">
      <feGaussianBlur in="SourceAlpha" stdDeviation="1" result="blur"/>
      <feOffset dx="-1" dy="-1" result="offset-blur"/>
      <feComposite in="SourceGraphic" in2="offset-blur" operator="over"/>
    </filter>
  </defs>

  <!-- COIN GROUP (receives drop shadow) -->
  <g filter="url(#coin-shadow)" class="coin-group">

    <!-- Outer coin fill -->
    <circle cx="100" cy="100" r="96" fill="url(#coin-base)"/>

    <!-- Reeded edge (72 radial lines) — hidden at small scale via <use class="edge"> -->
    <!-- ... generated with a pattern element ... -->

    <!-- Inner field (slightly darker) -->
    <circle cx="100" cy="100" r="88" fill="url(#field-grad)" opacity="0.6"/>

    <!-- Bezel ring -->
    <circle cx="100" cy="100" r="88" fill="none"
            stroke="#9A6810" stroke-width="2"/>
    <circle cx="100" cy="100" r="78" fill="none"
            stroke="#9A6810" stroke-width="1" opacity="0.5"/>

    <!-- RELIEF MOTIF GROUP -->
    <g class="relief" fill="none" stroke="#7A4E08">

      <!-- Rising sun semicircle -->
      <path d="M 72 100 A 28 28 0 0 1 128 100" stroke-width="2.5"
            stroke="#A07015"/>

      <!-- Sun rays (8, alternating length) -->
      <!-- ... 8 line elements radiating from center 100,72 ... -->

      <!-- Wave row 1 (closest, largest arcs) -->
      <path d="M 32 118 Q 44 108 56 118 Q 68 128 80 118 Q 92 108 104 118
               Q 116 128 128 118 Q 140 108 152 118 Q 164 128 168 118"
            stroke-width="2.5" stroke-linecap="round" fill="none"/>

      <!-- Wave row 2 (middle) -->
      <path d="M 36 110 Q 48 102 60 110 Q 72 118 84 110
               Q 96 102 108 110 Q 120 118 132 110 Q 144 102 156 110 Q 162 108 166 110"
            stroke-width="1.8" stroke-linecap="round" fill="none"/>

      <!-- Wave row 3 (distant, smallest) -->
      <path d="M 40 104 Q 50 98 60 104 Q 70 110 80 104
               Q 90 98 100 104 Q 110 110 120 104 Q 130 98 140 104
               Q 150 110 158 104 Q 162 102 165 104"
            stroke-width="1.2" stroke-linecap="round" fill="none"/>

    </g>

    <!-- TEXT ON COIN (hidden at ≤ 48px via SVG class) -->
    <g class="coin-text" font-family="Pretendard, sans-serif" fill="#7A4E08">
      <!-- Top arc: 동해 -->
      <path id="top-arc" d="M 30 100 A 70 70 0 0 1 170 100" fill="none"/>
      <text font-size="10" font-weight="700" letter-spacing="4">
        <textPath href="#top-arc" startOffset="30%">동  해</textPath>
      </text>
      <!-- Bottom arc: DBLN -->
      <path id="bot-arc" d="M 170 100 A 70 70 0 0 1 30 100" fill="none"/>
      <text font-size="10" font-weight="700" letter-spacing="3">
        <textPath href="#bot-arc" startOffset="30%">D B L N</textPath>
      </text>
    </g>

    <!-- Specular highlight overlay -->
    <circle cx="100" cy="100" r="96" fill="url(#coin-spec)"/>

  </g>
</svg>
```

**Notes for implementation:**
- The wave paths need careful hand-tuning. The `Q` (quadratic bezier) control points above are approximate — the actual SVG should be drawn in Inkscape or Figma and exported.
- The sun rays at `filter="url(#relief)"` create the embossed relief illusion.
- At 24px (token row), use a simplified version: gold circle + abbreviated sun arc + single wave arc. No text, no reeded edge.

---

## Scale Variants

### Hero coin (260–320px rendered)
Full version as described above. All detail visible. Animated with CSS `@keyframes` (float).

### Token row (18–24px)
Simplified SVG:
```svg
<svg viewBox="0 0 24 24">
  <circle cx="12" cy="12" r="11" fill="url(#coin-base-sm)"/>
  <circle cx="12" cy="12" r="9" fill="none" stroke="#9A6810" stroke-width="1"/>
  <!-- Simplified dawn: semicircle -->
  <path d="M 7 12 A 5 5 0 0 1 17 12" fill="none" stroke="#7A4E08" stroke-width="1.2"/>
  <!-- Two wave rows -->
  <path d="M 5 14.5 Q 7 12.5 9 14.5 Q 11 16.5 13 14.5 Q 15 12.5 17 14.5 Q 18 15.5 19 14.5"
        fill="none" stroke="#7A4E08" stroke-width="1"/>
  <path d="M 6 12.5 Q 8 11 10 12.5 Q 12 14 14 12.5 Q 16 11 18 12.5"
        fill="none" stroke="#7A4E08" stroke-width="0.8"/>
</svg>
```

### Favicon (32px, 16px)
Use the existing favicon from `site/index.html` as a starting point, but upgrade:
- Add the sun semicircle above the wave paths
- Use the same gold radialGradient
- At 16px: just the gold circle + single wave (anything smaller is indistinguishable)

---

## What is Vector vs. Raster

| Element | Format | Reason |
|---------|--------|--------|
| Hero coin (site) | **SVG** | Scales to any display density, no HTTP request for image, animated via CSS |
| Token row icon | **SVG** | Small and precise; raster would be blurry at 1× |
| Favicon | **SVG in HTML `data:` URL** | Current approach, keep it |
| App icon (1024px master) | **Raster PNG** (image generation needed) | Requires photorealistic metal texture and sub-pixel lighting that SVG cannot achieve convincingly |
| og:image (1200×630) | **Raster PNG** (image generation needed) | Must look great in social card previews; SVG rendering across scrapers is inconsistent |

---

## Image Generation Prompts (for the raster master)

### Prompt Variant A — Photorealistic numismatic

```
A photorealistic gold doubloon coin, perfect circle, studio photography lighting at 35 degrees from upper left, isolated on deep navy blue background (#091522). 

The coin's obverse shows a relief-engraved design: upper half depicts a stylized rising sun — a semicircle with 8 radiating lines, like a Japanese mon or compass rose — rendered as deep-struck engraving. Lower half shows three rows of stylized ocean waves in the style of traditional woodblock prints, rendered as numismatic relief engraving. 

Around the bezel (following the coin edge), the text "동해" arches across the top and "DBLN" arches across the bottom, engraved in the style of coin lettering. Centered: "1 DOUBLOON" in small caps.

Metal: warm amber-gold, proof finish — mirror-polished fields with frosted (matte) relief. Deep relief casting strong shadow in the wave valleys. Bright specular highlight on the upper-left rim and sun relief.

No real currency. No existing coat of arms. Original artistic design. No human faces. High numismatic quality. 4K detail.
```

**Negative prompt:** cartoon, flat, 2D, cheap, plastic, clipart, Bitcoin logo, Ethereum logo, existing coin, real currency, portrait, human face, text outside the coin, background clutter, generic gold texture

---

### Prompt Variant B — Stylized editorial

```
A stylized illustration of a gold coin (doubloon), editorial style, flat-ish but with volumetric lighting. Dark navy background.

The coin is a perfect circle with a warm gold gradient (bright amber-gold at top-left highlight, deep amber-brown at lower-right). 

The coin shows: (1) A sunrise motif in the upper half — half-circle with radiating lines, elegant and minimal. (2) Three rows of stylized ocean waves in the lower half, the waves rendered as clean overlapping arcs in the style of Japanese wave illustration. 

No text on the coin. Clean, modern, slightly geometric interpretation of the historical doubloon. Would look at home on a premium Mac app or a luxury brand website.

Studio lighting. The coin casts a warm amber glow on the dark navy background. High contrast. Premium. Not clipart. Not crypto-hype aesthetic.
```

**Negative prompt:** cartoon, neon, crypto-hype, Bitcoin, Ethereum, flat icon, clipart, text, faces, existing currency, purple gradient

---

### Prompt Variant C — Engraving illustration

```
A detailed pen-and-ink engraving illustration of a gold coin, in the style of historical numismatic engraving plates. Black ink on warm cream paper, then colorized with warm amber-gold wash.

The coin shows: a rising dawn sun (semicircle with rays) in the upper half, three rows of stylized ocean waves in the lower half. Bezel inscription: "DONGHAE" at top, "DBLN" at bottom. 

Engraving style: cross-hatching in the shadows, clean line work in the highlights, visible fine-line relief detail. The style references 18th-century coin documentation engravings. The result should feel handcrafted and historic, not digital or generic.

Print quality. High detail. No real existing currency design.
```

---

## Recommended image generation tool

For the raster master (1024px app icon):
1. **Midjourney v7** — best for photorealistic metal and numismatic detail (Prompt A)
2. **Adobe Firefly** — best for editorial/stylized treatments, good for print-quality illustration (Prompt C), commercially safe licensing
3. **DALL-E 3 via API** — acceptable for Prompt B; less control over fine detail but faster iteration

Recommendation to the lead: **Use Midjourney v7 with Prompt A** for the photorealistic icon master, then commission a clean SVG trace from the result for the web hero and token row. The SVG trace (done by a human designer in Figma/Illustrator) will be more precise and scalable than using the raster directly in the web hero.

---

## Plan for the SVG Coin in the Mockup

The `design/mockup/index.html` will include a hand-authored SVG coin that demonstrates the direction without requiring image generation. It will:

1. Use the `radialGradient` approach for gold fill
2. Show the dawn motif (simplified — sun arc + 3 wave rows)
3. Demonstrate the bezel ring and drop shadow
4. Be clearly labeled as a directional placeholder: `<!-- PLACEHOLDER: Replace with generated master -->`

This SVG is designed to be good enough that the founder can judge the motif direction without seeing the final raster version.
