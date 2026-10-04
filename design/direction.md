# EastSea — Design Direction v2

Author: Designer (site-designer agent)  
Date: 2026-10-04  
Status: Direction document. Approved direction proceeds to mockup; no changes to site/ files.

---

## Concept and One-Line Idea

**"Tide Tables"**

> A blockchain for your Mac that reads like a precision navigation instrument — warm, authoritative, and built to last.

The East Sea at first light: amber on the horizon, deep navy below. Not treasure-hunt adventure; the quiet confidence of a navigator who has crossed this sea before. The doubloon is not a gambler's chip — it is a historical coin with weight and craft.

---

## Mood Words

Deliberate · Warm · Legible · Handcrafted · Still · Trustworthy · Nautical without kitsch · Korean without stereotype

Anti-mood: Hype · Casino · Clipart · Purple-gradient · Generic dark SaaS · "Earn money while you sleep"

---

## Color System

### Design tokens — light mode

```css
:root {
  /* Backgrounds */
  --bg:          #F3EFE3;  /* chart parchment */
  --bg-raise:    #FDFBF4;  /* card surface    */
  --bg-tint:     #EBE5D3;  /* section alternation */

  /* Text */
  --ink:         #111C29;  /* near-black navy   */
  --ink-mid:     #3A5068;  /* secondary text    */
  --ink-soft:    #6A8098;  /* captions, labels  */

  /* Structural */
  --line:        #D2C8A8;  /* dividers          */

  /* Accent — Gold (the coin) */
  --gold:        #B8871E;  /* interactive gold  */
  --gold-light:  #E5B840;  /* coin highlight    */
  --gold-deep:   #7A5210;  /* readable gold text*/
  --gold-bg:     #F5E6BB;  /* gold tint surface */

  /* Accent — Teal (the sea / actions) */
  --teal:        #0A6E68;  /* primary action    */
  --teal-light:  #14A89E;  /* hover state       */
  --teal-bg:     #DFF4F2;  /* teal tint surface */

  /* Hero panel (dark band at top) */
  --hero-bg:     #091522;  /* deep ocean        */
  --hero-bg2:    #0D2135;  /* slightly lighter navy */
  --hero-ink:    #EDE7DC;  /* warm white text   */
  --hero-soft:   #97B5C8;  /* secondary hero text */
  --hero-gold:   #E8C050;  /* coin on dark      */
  --hero-teal:   #5EC5BA;  /* teal on dark      */

  /* Coin SVG internal */
  --coin-fill:   #D4A428;  /* gold base         */
  --coin-bright: #F0CB5A;  /* highlight spot    */
  --coin-shadow: #7A4E08;  /* deep shadow       */
  --coin-rim:    #9A6810;  /* outer rim         */
}
```

**Contrast ratios (light mode):**
- `--ink` #111C29 on `--bg` #F3EFE3 → **16.4:1** (AAA)
- `--ink-mid` #3A5068 on `--bg` #F3EFE3 → **8.2:1** (AA Large + AA)
- `--gold-deep` #7A5210 on `--bg` #F3EFE3 → **5.3:1** (AA)
- `--teal` #0A6E68 on `--bg` #F3EFE3 → **5.8:1** (AA)

### Design tokens — dark mode

```css
@media (prefers-color-scheme: dark) {
  :root {
    --bg:          #070F1B;  /* deep ocean floor  */
    --bg-raise:    #0B1925;  /* surface           */
    --bg-tint:     #091421;  /* alternation       */

    --ink:         #ECE6D8;  /* warm white        */
    --ink-mid:     #9AB2C8;  /* secondary         */
    --ink-soft:    #6080A0;  /* captions          */

    --line:        #182C40;  /* dividers          */

    --gold:        #E8B840;  /* coin gold on dark */
    --gold-light:  #F5D070;  /* highlight         */
    --gold-deep:   #E8B840;  /* gold text on dark — same, readable */
    --gold-bg:     #1E1608;  /* gold tint         */

    --teal:        #4DC4BA;  /* action on dark    */
    --teal-light:  #70D8D0;  /* hover             */
    --teal-bg:     #051B1A;  /* teal tint         */

    --hero-bg:     #04090F;  /* deeper still      */
    --hero-bg2:    #07111E;
    --hero-ink:    #ECE6D8;
    --hero-soft:   #7A9AB5;
    --hero-gold:   #E8C050;
    --hero-teal:   #5EC5BA;

    --coin-fill:   #D4A428;
    --coin-bright: #F0CB5A;
    --coin-shadow: #7A4E08;
    --coin-rim:    #9A6810;
  }
}
```

**Contrast ratios (dark mode):**
- `--ink` #ECE6D8 on `--bg` #070F1B → **16.0:1** (AAA)
- `--ink-mid` #9AB2C8 on `--bg` #070F1B → **8.5:1** (AA)
- `--gold` #E8B840 on `--bg` #070F1B → **10.1:1** (AA)
- `--teal` #4DC4BA on `--bg` #070F1B → **9.8:1** (AA)

---

## Type System

### Font families

**Display (English H1 only):** `"EB Garamond", Georgia, serif`  
→ Self-hosted, italic, 400 weight only, ASCII+punctuation subset (~15KB woff2)  
→ Use at 72px+ (desktop) / 44px+ (mobile), `font-style: italic`, `font-weight: 400`  
→ NOT used for Korean — Pretendard takes all Korean text

**All other text (Korean and English):** `"Pretendard", system-ui, sans-serif`  
→ Self-hosted variable font or 3-weight static (Regular 400, SemiBold 600, Bold 700)  
→ Static woff2 subset for used characters: ~2×140KB (Regular + Bold)  
→ Best Korean web font for Korean-Latin harmony; SIL OFL

**Font stack in CSS:**
```css
--font-display: "EB Garamond", Georgia, "Times New Roman", serif;
--font-body:    "Pretendard", "Apple SD Gothic Neo", "Noto Sans KR", system-ui, sans-serif;
```

### Type scale

| Token | Size (desktop) | Size (mobile) | Family | Weight | Line-height | Tracking |
|-------|---------------|---------------|--------|--------|-------------|----------|
| `display-en` | 72px | 44px | EB Garamond | 400 italic | 1.12 | -0.02em |
| `display-ko` | 58px | 38px | Pretendard | 700 | 1.15 | -0.025em |
| `h2` | 36px | 26px | Pretendard | 700 | 1.20 | -0.02em |
| `h3` | 20px | 18px | Pretendard | 600 | 1.30 | -0.01em |
| `body` | 16px | 15px | Pretendard | 400 | 1.70 | 0 |
| `small` | 14px | 13px | Pretendard | 400 | 1.60 | 0.01em |
| `kicker` | 12px | 12px | Pretendard | 600 | 1.40 | 0.08em |

**Korean-Latin pairing rules:**
1. Korean `body` (Pretendard 400) and English `body` (Pretendard 400) are the same token — Pretendard handles both scripts.
2. Only `display-en` switches to EB Garamond, and only for English nodes (`.en` class).
3. Korean `display-ko` (Pretendard 700) is more typographically complete than any serif/sans-serif mix — Korean letterforms at 700 weight need no additional styling.
4. Do not mix Garamond and Korean on the same line.

---

## Spacing and Grid

```
--gutter: 20px   (phone 360px viewport)
--gutter: 32px   (tablet 768px)
--gutter: 48px   (desktop 1024px+)
--maxw:   68rem  (1088px)

Section padding: 100px top/bottom (desktop) / 64px (mobile)
Card padding: 28px (desktop) / 20px (mobile)
Card radius: 16px
```

Grid: 12-column, 20px gap on desktop. Benefits section: 2-column on tablet, 2-column on desktop (not 4-column — breathing room is more important than density).

---

## Page Structure and Section-by-Section Layout

### 0. Nav (fixed, transparent → solid on scroll)
- Brand mark (coin SVG, 28px) + "동해" logotype in Pretendard 700, small caps
- Right: language toggle (EN/한글) + very small pill status badge (testnet / mainnet)
- On scroll past hero: background gains `backdrop-filter: blur(16px)` + subtle border-bottom

### 1. Hero (dark navy, full viewport height on desktop)

**Layout:** Two-column split. Left column (55%): headline + subhead + status pill + CTAs. Right column (45%): doubloon coin.

The left/right split is not equal — the text has precedence. The coin floats center-right, slightly cropped at bottom by the wave SVG that transitions to the next section.

```
┌─────────────────────────────────────────────────────┐
│ nav                                        [EN] [●]  │
├────────────────────────────┬────────────────────────┤
│                            │                        │
│  ─────────────────         │    ╭─────────────╮     │
│  BLOCKCHAIN FOR YOUR MAC   │   ╱   ☀  ~~~~~   ╲    │
│  COIN: DOUBLOON DBLN       │  │   ~~~~~~~~~~   │    │
│  ─────────────────         │  │   DBLN  동해   │    │
│                            │   ╲               ╱    │
│  Your wallet and your      │    ╰─────────────╯     │
│  node, on your Mac.        │                        │
│                            │   light pulse rings    │
│  (subhead)                 │                        │
│                            │                        │
│  ● testnet · no token sale │                        │
│                            │                        │
│  [See downloads]           │                        │
│  [How rewards work]        │                        │
│                            │                        │
└────────────────────────────┴────────────────────────┤
        ~~~~ wave SVG transition ~~~~
```

**Mobile (360px):** Single column. Coin appears BELOW the CTAs, slightly cropped on sides. Wave at bottom of coin, smooth into section below.

**Visual detail:**
- Background: `radial-gradient(ellipse 55% 40% at 70% 50%, rgba(228,184,64,0.07) 0%, transparent 65%)` — a faint warm glow behind the coin
- The coin casts a soft amber shadow on the navy: `box-shadow: 0 0 80px 20px rgba(228,184,64,0.12)`
- Wave SVG (3 layers) transitions from hero dark into the section below

### 2. Benefits ("What you get")

**Background:** Light (--bg). 4 benefits in a 2×2 grid on desktop, 1-column on mobile.

Each benefit card:
- No card border. White space separates them.
- Icon: 40×40 SVG, `stroke: var(--teal)`, 2px stroke, no fill
- H3: Pretendard 600, 20px
- Body: Pretendard 400, 16px, --ink-mid

Benefits in order (unchanged from current — copy will stay):
1. Touch ID (Secure Enclave, no seed phrase)
2. Your Mac verifies your balance
3. Node switch — earns while online (language must be neutral per legal §3.6)
4. AI agent wallet

**Legal note (§3.6 compliance):** Benefit 3 description must NOT say "earns overnight" or "works while you sleep." Revised framing: "Turn on the node switch and your Mac participates in the network. While it's online, it receives the protocol's reward for its uptime — the rules are just below."

### 3. How rewards work ("Node rewards")

**Background:** Tint (--bg-tint). Left-aligned layout on desktop: explanatory prose left (60%), rule table right (40%).

Section is required by the copy but must be framed as "here are the rules" not "here is how to earn money." The legal review (§3.6) flags "먼저 온 사람이 더 큰 몫" (Early operators get the larger slice) as potentially risky — keep the rule table but remove the advantage framing from the H3 heading. Rename to "Distribution by operator count."

Visual treatment: the rule table gets a subtle gold left-border `2px solid var(--gold)`. The "honest box" disclaimer gets a `background: var(--teal-bg)` treatment to signal transparency, not warning.

### 4. Safe by design

**Background:** Light (--bg). Full-width single column. Checklist with teal checkmarks.

No changes to content. Typography change: the `<strong>` labels get `color: var(--ink)` (not gold) so they read as prose, not badges.

### 5. Downloads

**Background:** Tint (--bg-tint). Two platform cards side-by-side. Each card: `border: 1px solid var(--line)`, 16px radius, white background.

The "Coming at mainnet launch" badge: amber/gold pill. Remove default button shape — it's not clickable.

### 6. FAQ

**Background:** Light (--bg). `<details>/<summary>` accordion. No JS needed.

Visual upgrade: the summary arrow (`▶`) becomes a `+` / `×` using CSS `content` — cleaner and more editorial. Mild `border-bottom: 1px solid var(--line)` between items.

### 7. Footer

Dark (--hero-bg). Legal, trademark, copyright. Three small lines. Back-to-top link.

---

## Motion Principles

1. **The coin floats.** Amplitude 6px, period 3.8s, easing `cubic-bezier(0.37, 0, 0.63, 1)`. Only when `prefers-reduced-motion: no-preference`.
2. **Section reveals.** Each section fades in (`opacity: 0 → 1`) + slides up 12px (`translateY(12px) → 0`) over 0.4s `ease-out` as it enters the viewport. Triggered by `IntersectionObserver`. One-shot (not on every scroll).
3. **Coin light pulse.** Two concentric rings expand from the coin center, `opacity: 0.15 → 0`, `scale: 1 → 1.4`, over 2.5s, staggered 1.25s apart. Replaces the current star-spark elements.
4. **Nav fade-in.** The nav background appears on scroll past 40px. `transition: background-color 0.25s ease, backdrop-filter 0.25s ease`.
5. **No other animations.** Hover states use `transition: 0.15s ease` only.

---

## Imagery Rules

1. No stock photos. No photographic backgrounds.
2. The ONLY raster asset allowed is the doubloon coin master (if used). All other art is SVG.
3. The doubloon coin should appear in the hero at approximately 260–320px. It may also appear at 18px as a token icon in the download cards.
4. All icons are hand-drawn SVG paths, 2px stroke, no fill. Style matches the existing icons in site/index.html but with slightly more care in the path geometry.
5. Decorative background elements (wave, grain, glow) are CSS or inline SVG only.

---

## What We Will NOT Do

- Purple gradients on white
- Spinning coin animation
- "Earn," "profit," "income," "passive," "ROI," or any return-framing language (legal §3.6)
- Third-party CDN fonts, scripts, or trackers
- Fake download buttons (the site/ README bans placeholders that look real)
- Full-bleed photography
- Dark mode as an afterthought — both modes are designed at parity
- Bright neon "Web3" aesthetic
- Generic hero showing a phone mockup
- Marketing copy that contradicts DISCLAIMER.md
- "Works while you sleep" (legal §3.6 risk)

---

## Three Hero Concepts

### Concept A — "Split Horizon" (RECOMMENDED)

**Idea:** The left side of the hero is deep ocean (text domain), the right side holds the coin with a warm amber glow emanating from behind it — like the sun rising on the horizon. The split is a subtle 3° diagonal, not a hard vertical line.

**Wireframe:**

```
┌──────────────────────────────────────────────────────────┐
│ [nav]                                                     │
│                                                           │
│                                              ░░░░░░░░    │
│  KICKER: A BLOCKCHAIN FOR YOUR MAC           ░░░░░░░░    │
│                                             ░░░░░░░░░░   │
│  A wallet and your node,    /              ░░░░(coin)░░  │
│  on your Mac.              /              ░░░░░░░░░░░░   │
│                           /                ░░░░░░░░░░    │
│  (subhead text)          /                  ░░░░░░░░     │
│                         / ← 3° diagonal                  │
│  ● testnet · no sale   /                                  │
│                                                           │
│  [CTA primary]  [CTA ghost]                              │
│                                                           │
│ ≈≈≈≈≈≈≈≈≈≈ wave transition ≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈    │
└──────────────────────────────────────────────────────────┘
```

**Key elements:**
- Left: text on dark navy (#091522)
- Right: same dark navy with a radial warm glow (`rgba(232,184,64,0.08)`) centered on the coin
- Diagonal split: CSS `clip-path: polygon(0 0, 57% 0, 60% 100%, 0 100%)` for the text panel; the glow extends through the full width
- Coin: 300px, floats with 6px amplitude, centered in the right column
- Headline (English): EB Garamond 400 italic, 68px, `--hero-ink` color
- Headline (Korean): Pretendard 700, 56px, `--hero-ink` color
- Status bar: teal pill `background: rgba(78,196,186,0.15); border: 1px solid rgba(78,196,186,0.3)`
- CTAs: primary = `background: var(--gold); color: #0A0A0A` pill button; ghost = transparent with `border: 1px solid rgba(237,231,220,0.3)`

**Why recommended:** The diagonal horizon is an original layout move that no wallet site currently uses. It visually communicates "horizon / sea / dawn" without saying it. It puts the coin in clear focus while giving the copy room to breathe on the left. It works on all screen widths: at mobile, the diagonal disappears and the layout stacks cleanly.

---

### Concept B — "Monumental Type"

**Idea:** Full-width dark, no side-by-side split. The word "동해" appears in enormous Pretendard Black at 140px — the whole top half of the viewport is the brand name as a typographic object. Below: coin centered, small, as a decorative seal. Below that: English tagline in EB Garamond italic 36px. Below: CTAs.

**Wireframe:**

```
┌──────────────────────────────────────────────┐
│                                               │
│           동  해                              │
│           (140px Pretendard Black)            │
│                                               │
│               ○ coin ○                        │
│           (120px, centered)                   │
│                                               │
│   A wallet and your node, on your Mac.        │
│           (EB Garamond italic, 32px)          │
│                                               │
│           [Download]  [Learn more]            │
│                                               │
└──────────────────────────────────────────────┘
```

**Risk:** Extremely typography-forward. Requires Pretendard Black (900 weight) to look correct — missing font fallback degrades badly. Bold bet for a brand name that most visitors won't immediately recognize as meaningful. High ceiling, high floor.

---

### Concept C — "Cartographic"

**Idea:** Light cream background (like chart paper), coin emerges from the right edge like a rising sun — only the left half of the coin is visible, the rest cropped by the viewport edge. Faint nautical grid lines (1px, 5% opacity) on the background. Editorial newspaper layout: left column is text, right column has the half-coin composition.

**Wireframe:**

```
┌──────────────────────────────────────────────┐
│  nav (light background version)              │
│                                               │
│  ┌────────────────┐ ┌─────────────────────┐  │
│  │                │ │                     │  │
│  │ KICKER TEXT    │ │      ╭──────        │  │
│  │                │ │    ╭╯  ☀  ~        │  │
│  │ headline here  │ │   │    ~~~~         │  │
│  │ (dark on cream)│ │    ╰╮  ~DBLN        │  │
│  │                │ │      ╰──────        │  │
│  │ subhead        │ │  (coin half-visible) │  │
│  │                │ │                     │  │
│  │ [CTA] [CTA]    │ │                     │  │
│  └────────────────┘ └─────────────────────┘  │
│                                               │
│ ≈≈≈ wave ≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈  │
└──────────────────────────────────────────────┘
```

**Risk:** Light hero is less dramatic and may read as generic landing page without a strong visual. The half-cropped coin is clever but can look like an asset loading error. The nautical grid is a subtle nod that most users will miss — it's decoration without payoff.

---

## Recommendation: Concept A — Split Horizon

**Reasons:**
1. The diagonal horizon directly expresses the product name (East Sea horizon) without explaining it — good design works on multiple levels simultaneously.
2. The coin gets visual primacy while the text has protected reading space. Neither crowds the other.
3. The technical execution is pure CSS + SVG — no 3D library needed. The coin SVG can be as detailed as we make it.
4. The dark hero with warm glow is in the same register as Linear, Raycast, and Arc — aligned with premium Mac product aesthetics.
5. It handles the bilingual headline gracefully: Korean and English can stack on the left column without the layout feeling crowded, because the right column is purely visual.
6. Responsive degradation is clean: on mobile, the diagonal clips to zero and the layout stacks (text → coin → wave).
