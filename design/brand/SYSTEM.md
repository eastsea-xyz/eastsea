# EastSea design system

One system for the site, the Mac and iOS wallet, the browser extension and the explorer.

- **Source of truth:** [`tokens.json`](tokens.json). Change a value there, never in a product.
- **Web:** `node design/scripts/build-tokens.mjs` writes `site/tokens.css`, which holds the CSS custom properties.
  - `--c-*` colours
  - `--font-*` families and `--text-*` sizes
  - `--space-*`, `--radius-*`, `--shadow-*`, `--dur-*` and `--ease-*`
  - `--check` fails if `site/tokens.css` is stale.
- **Apple:** a follow-up lane maps the same roles onto `apps/wallet/Sources/Theme.swift`. It retires the violet `Color.aether` (0.49/0.40/0.95).

## Character

The East Sea at dawn: calm, precise, trustworthy. The product answers the question and keeps the machinery out of sight.

- Paper and navy carry the page. Gold is the coin and the sun, never a second text colour on light backgrounds.
- Dawn coral marks one small thing per view at most, such as the Touch ID glyph.

## Colour roles

| Role | Light | Dark | Use |
|---|---|---|---|
| `bg` | `#F4EFE6` | `#071320` | Window and page background |
| `surface` | `#FBF8F2` | `#0D1C2C` | Cards, sheets |
| `surface-sunken` | `#ECE5D8` | `#0A1826` | Wells, inactive fills |
| `text` / `text-muted` | `#0D2135` / `#4B5B6B` | `#ECE6D9` / `#94A4B5` | Copy |
| `accent` | `#0F5A75` | `#7CC4DC` | Links, focus, selection |
| `accent-fill` | `#0D2135` | `#E8BF59` | Primary button (navy by day, gold by night) |
| `gold` / `gold-text` | `#E8BF59` / `#7E5A0E` | `#E8BF59` | Coin and mark / gold as text |
| `dawn` | `#D9673F` | `#F08F6C` | One accent per view |
| `sea`, `horizon` | `#0D2135`, `#F0DCC0` | `#030B14`, `#132A40` | Art only |
| `success` / `warn` / `danger` | `#1E7A52` / `#AD6100` / `#B42318` | `#5CCB98` / `#FFB857` / `#FF8A7A` | Status. `warn` matches the wallet's existing `Color.warn` |

- All text pairs meet WCAG AA. The contrast ratios are in the `role` strings in `tokens.json`.
- Status is never shown by colour alone. Pair it with a word, a dot or a shape.

## Type

| Family | Web | Apple | For |
|---|---|---|---|
| display | Newsreader | New York (`.serif`) | Headlines, big editorial numbers |
| display-ko | Hahmlet | System Korean serif | Korean headlines |
| text | Geist, then Apple SD Gothic Neo | SF Pro (`.default`) | UI and body |
| mono | Geist Mono | SF Mono (`.monospaced`) | Addresses, block numbers, labels |
| amount | Geist, tabular | SF Pro + `.monospacedDigit()` | Balances. The `.rounded` design is retired for amounts |

- **Scale (px = pt):** caption 12, label 12 (mono, upper case), footnote 13, body 16, body-lg 19, title-3 22, title-2 30, title-1 44, display 72, amount-lg 44.
- **Web display sizes** are fluid with `clamp()` around these values.
- **Korean** gets more line height (1.2 for headlines, 1.75 for body) and `word-break: keep-all`.

## Space, radii, shadows, motion

- **Spacing:** a 4-point scale (4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 96, 128).
- **Radii:**
  - xs 4, sm 8
  - md 12: the inner card, matching the wallet's `Radius.inner`
  - lg 16: the card, matching `Radius.card`
  - xl 24, window 12, pill
- **Shadows:** `sm`, `md` and `window`, each defined for light and dark.
- **Motion:** 120, 240 and 600 ms, plus a 1400 ms scene, all on `cubic-bezier(.2,.7,.2,1)`. All motion is decorative and turns off under reduced motion.

## Token icons

This section restates the security rules in `apps/wallet/Sources/TokenIconSpec.swift`. If the two ever disagree, the Swift file wins.

- **Native coin:** the gold doubloon, `design/brand/dbln-coin-1024.png`. Use the flat `dbln-coin-flat-256.png` below 48 pt.
- **Official:** tokens on the shipped trust list (`TokenGuard.swift` `KnownTokens`) carry bundled art.
  - The list is keyed by **address**, never by symbol. Files: `design/brand/tokens/<SYMBOL>-256.png`.
  - Style: a full circle with a solid fill in system colours (navy, gold, sea, dawn), one soft light and a thin rim.
  - Never put text in the art, never imitate another project's logo, and never fetch an image.
- **Generated (everything else):** a pastel `HSL(hue(seed), 45%, 62%)` disc with the symbol's first letter.
  - It has a **dashed ring** and a **"?" badge**, so its shape alone says "unverified".
  - The seed is FNV-1a of the lowercase address. VoiceOver reads "Unverified token".
- **Sizes:** row 28, large list 40, detail 64.

## Files

- `tokens.json` holds the values. `SYSTEM.md` is this page.
- `design/scripts/build-tokens.mjs` generates the web tokens.
- `design/scripts/subset-ko-font.py` builds the Korean headline font subset.
- `design/og/og-image.html` and `render-og.cjs` produce the social preview image.
