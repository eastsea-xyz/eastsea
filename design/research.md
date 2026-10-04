# EastSea Design Research

Researcher: Designer (site-designer agent)  
Date: 2026-10-04  
Scope: 12 reference sites, typographic and CSS technique survey, coin visual treatment survey.

---

## Reference Table

Screenshots in `design/research/shots/`.

| # | URL | Category | What makes it premium | Steal | Avoid |
|---|-----|----------|----------------------|-------|-------|
| 1 | **phantom.app** | Consumer wallet | Lavender-ghost identity mark, pill CTAs, high contrast on near-white, minimal nav | Confidence of leaving white space; pill badge for status | Current lavender-on-white treatment feels dated (2023 era); hero has almost no visual content |
| 2 | **rainbow.me** | Consumer wallet | Gradient spectrum identity, friendly humanist type, generous padding, colorful but disciplined | Per-section color accents that feel playful without screaming | Bright spectrum gradient (overdone across crypto); heavy on animations that slow perceived load |
| 3 | **zerion.io** | DeFi wallet | Clean section alternation (dark/light), dashboard screenshot as hero, left-aligned copy | Left-aligned body copy rhythm; clean table typography | Electric-blue gradient is generic; phone-screenshot hero is imitated across 50 sites |
| 4 | **coinbase.com/wallet** | Consumer wallet | Institutional credibility through white space and restrained color | Trust through restraint | Overcrowded "features checklist" layout at scroll depth; feels corporate |
| 5 | **linear.app** | Mac product | Near-black background (#0F0F0F), exceptional kerning, monospace UI shots feel like real software, yellow accent for social proof | Dark-first design; short punchy headline (≤ 8 words); heterogeneous section widths | Requires a very strong brand mark to anchor a nearly-empty hero; risky if brand is weak |
| 6 | **raycast.com** | Mac product | "Your shortcut to everything." — 5-word headline, huge, centered; 3D object as hero; infinite bottom scroll storytelling | Ultra-confident minimal hero; physical-object metaphor for the product | Requires hero 3D asset quality; mediocre 3D looks worse than none |
| 7 | **tailscale.com** | Mac/developer product | Clean grid, map-motif illustrations, serious-but-friendly tone, real copy that explains the product | Explanatory copy that doesn't talk down; network-topology illustration style | Maps as metaphor is too engineer-specific for a consumer product |
| 8 | **craft.do** | Mac product | Editorial whitespace, clean multi-platform screenshots layered on gradients, system-font-bold used at scale | Gallery of product shots tells the story without words | Too product-screenshot-heavy; assumes people recognize the app |
| 9 | **ledger.com** | Crypto hardware | Black editorial, luxury hardware photography, premium badge typography, investor credibility | Hardware-quality photography treatment for the coin art; badge/cert typography in footer | Full-bleed photography requires a real photo budget; text-heavy security section slows pace |
| 10 | **culturedcode.com/things** | Mac app | Warm sand background, elegant serif + rounded-sans pairing, human-scale copy, genuine warmth | Warm background (not white); humanist type; section rhythms that breathe | Lots of screenshot carousels — feels like an app store listing, not editorial |
| 11 | **arc.net** | Mac browser | Colorful gradient orbs as personality marks, short confident copy, dark hero with color pops | Using the brand mark (orb/coin) as the hero graphic anchor | Gradient orb blobs are visually overused in 2026; needs very strong identity to stand apart |
| 12 | **seaofthieves.com** | Maritime/pirate brand | Moody ocean photographic backgrounds, treasure visual language, dark premium feel, illustrated coin/doubloon mark | Doubloon coin visual language; parchment-and-navy palette; premium maritime without kitsch | Full-bleed video backgrounds (bandwidth); overtly pirate/adventure tone would undercut trust for financial product |

---

## What Premium Means for EastSea: Synthesis

Based on studying these 12 sites and the existing `site/index.html`, six patterns separate professional from template:

### 1. Headline restraint
Every premium site has a hero headline of 5–9 words. The current EastSea headline is 22 words across two sentences. Premium sites compress the idea to one claim, then let the subhead explain.

Linear: *"The product development tool for teams and agents."*  
Raycast: *"Your shortcut to everything."*  
→ EastSea needs: *"Your wallet and your node, on your Mac."* or similar — already in the copy but buried under the kicker.

### 2. Typographic identity
System fonts — even SF Pro — signal "I didn't make a typographic decision." Every memorable site on this list made a font choice:
- Linear: Inter at extreme tracking and weight
- Things: Moret (custom) + system rounded
- Ledger: GT America
- Sea of Thieves: hand-lettered custom
The remedy for EastSea is **Pretendard** (best Korean-Latin pairing, SIL OFL) + **EB Garamond** italic for English display headlines. EB Garamond's italic suggests maps, charts, and old navigation — exactly right for the East Sea metaphor.

### 3. The hero visual must have depth
All the forgettable sites (Zerion, Coinbase Wallet) use a flat phone-screenshot mockup. Premium sites use one of:
- A genuine 3D render (Raycast)
- A beautiful illustration or identity mark (Arc, Phantom)
- An abstract composition that suggests rather than shows
EastSea's current coin SVG is two wave strokes inside a circle — it looks like a placeholder. The coin needs to be a real artistic object: engraved relief, layered metal texture, physical presence.

### 4. Section breathing room
Premium sites give each section a minimum of 120px vertical padding. The current EastSea site's sections run together. The eye needs silence between ideas.

### 5. Color with intent
Every color must do one thing. Green = money in. Gold = the coin. Teal = the sea / action. Navy = trust / depth. The current palette is correct in concept but the CSS variable set has redundancy (--hero-gold, --gold, --btn-gold are three slightly different golds). Premium sites have a 3-token accent system: one dominant, one complement, one signal.

### 6. Motion that earns its place
Raycast uses 3D parallax on the hero object. Linear uses almost zero animation. Rainbow uses a gradient animation that is tasteful. EastSea's current coin bob animation is correct in principle (the coin floats on the sea) but needs to be refined — easing must be `cubic-bezier(0.37, 0, 0.63, 1)` (sinusoidal), not linear, and the amplitude should be small (6–8px), with a 3.5–4s period to feel gravitational.

The spark animations (s1, s2) are generic — star-burst sparkles that look like clip-art. Replace with subtle radial light pulses expanding from the coin center.

---

## Typography Research

### Pretendard (chosen)
- Author: Kil Hyung-jin (orioncactus), SIL OFL 1.1
- Character: clean neo-grotesque, designed for Korean-Latin harmony
- Weights: 100–900
- Files (variable): Pretendard-variable.woff2 ≈ 750KB, or 2-weight static subset ≈ 260KB
- Korean: excellent at all sizes; particularly strong at 700–800 weight for headings
- Latin: matches Helvetica's proportions but with better spacing rhythm
- Download: `https://github.com/orioncactus/pretendard`
- License: confirmed SIL OFL 1.1 (free to self-host, no attribution required on page)

### EB Garamond Italic (chosen for English display)
- Author: Georg Mayr-Duffner, SIL OFL 1.1
- Character: revival of Claude Garamond's 16th-century roman/italic; the italic is especially beautiful
- Files: EB Garamond Italic subset (ASCII + punctuation) ≈ 12–18KB woff2
- Use: English H1 only (`font-size: 72px+`, `font-style: italic`, `font-weight: 400`)
- Korean: NOT used for Korean — Pretendard Bold takes all Korean headings
- Download: Google Fonts (but self-host the woff2 file; do NOT use the CDN link)
- Why Garamond for EastSea: the italic suggests nautical charts, hand-lettered maps, and old registers — precisely the visual language of navigation and the East Sea

### Pairing rule
```
English headlines (H1): EB Garamond 400 italic, 72px → 44px mobile
Korean headlines (H1): Pretendard 700, 58px → 38px mobile
H2: Pretendard 700, 36px → 26px mobile
H3: Pretendard 600, 20px
Body (ko + en): Pretendard 400, 16px / 26px
Small: Pretendard 400, 14px / 22px
```

Korean text does NOT use the Garamond — the script mismatch would be jarring. Korean headings at Pretendard 700 already have strong graphic weight and need no serif addition.

---

## CSS Technique Survey

### Depth without imagery
- **Layered radial gradient** on the hero: `radial-gradient(ellipse 60% 40% at 65% 50%, rgba(200,145,34,0.08) 0%, transparent 70%)` — creates a warm light halo behind the coin without a raster image
- **Noise grain overlay** via `url("data:image/svg+xml,<svg ...><filter><feTurbulence>")` — adds tactile depth to flat sections; keeps the parchment-paper feel in light mode
- **CSS `@property` + Houdini paint** for animated gradient (not yet fully supported, use `background-size` animation fallback)

### Coin depth without raster
- SVG `<feDropShadow>` filter on the coin element: `dx="0" dy="4" stdDeviation="8" flood-color="#50300A" flood-opacity="0.5"` — gives lift
- SVG `radialGradient` on the coin fill: center is bright (#E8C070), rim is dark (#8A6010) — mimics oblique lighting
- `<feMerge>` layer for the relief: place a slightly-offset lighter version of the wave path underneath the main path to simulate embossing

### Scroll storytelling
- Use `position: sticky` headers within sections (not the page header)
- `scroll-behavior: smooth` for anchor jumps
- One entry animation per section: `@keyframes fadeInUp` triggered by `IntersectionObserver` (via a small vanilla JS snippet, not a library)

### `prefers-reduced-motion`
- All animation `@keyframes` are wrapped in `@media (prefers-reduced-motion: no-preference)` — matches the existing approach
- The coin float: 4s sinusoidal bob
- Section fade-in: 0.4s ease-out (only runs once, not on scroll)
- No parallax on reduced-motion

---

## Coin Visual Survey

### Historical reference — Spanish 8-escudo (doubloon)
- Irregular, hand-struck appearance; the relief is high but the coin shape varies slightly
- Cross/coat-of-arms on obverse, pillars of Hercules on reverse
- Edge: milled/reeded after 1750

### Numismatic photography technique
- Studio lighting at 25–35° from upper left or upper right
- Dark background (often velvet) or gradient-to-black
- High magnification reveals the relief texture
- The "proof" finish (mirror fields, frosted relief) creates the most dramatic light contrast
- Key lesson: the light source is everything — a flat coin with good lighting looks more 3D than a "3D coin" with flat lighting

### For EastSea's Doubloon SVG
- The relief motif: dawn sun (half-circle + 8 radiating lines) over the East Sea (3 wave rows)
- Not a copy of any real currency
- No human face (numismatic tradition, but also simpler to draw)
- Edge: draw reeded edge as a pattern of thin radial lines around the outer rim
- Color: gold radialGradient (#E0B030 center → #7A4D0A rim)
- Drop shadow: 0 6px 18px rgba(60,30,0,0.55)

### Image generation prompt (for raster master, 1024px)
See `design/coin.md` for the full prompt text and variants.
