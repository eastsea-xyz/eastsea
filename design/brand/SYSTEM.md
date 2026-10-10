# EastSea design system

One system for the site, the Mac and iOS wallet, the browser extension, explorer and contract toolbox.

- **Source of truth:** [`tokens.json`](tokens.json). Change a value there, never in a product.
- **Generation:** `python3 scripts/gen-design-tokens.py` writes all platform outputs. The older `node design/scripts/build-tokens.mjs` command delegates to this script.
- **Web:** `site/tokens.css`, `apps/extension/ui/design-tokens.css`, `apps/explorer/design-tokens.css` and `design/generated/toolbox/design-tokens.css` have identical CSS custom properties.
  - `--c-*` colours
  - `--font-*` families and `--text-*` sizes
  - `--space-*`, `--radius-*`, `--shadow-*`, `--dur-*` and `--ease-*`
  - `--check` fails on any missing or changed platform output, including the shared component copies. It writes nothing.
- **Apple:** `apps/wallet/Sources/DesignTokens.swift` exposes `DesignTokens.Palette`, `TypeScale`, `Space`, `Radius`, `Shadows` and `Motion`. Color pairs resolve Aqua/dark Aqua and accessibility appearances explicitly. Numbers use the default system face and tabular digits. Native system fonts are used through Apple APIs, with no font binaries embedded.
- **Components:** `components.css` is the canonical web control, plate, typography and status grammar, packaged as `design-components.css` by the same generator.
- **Gate:** `scripts/verify.sh apps` checks token drift and generator regression tests. `python3 scripts/test_design_tokens.py` covers units, validation, palette parity, shadow layers, text contrast and read-only drift detection.
- **Phase 1 boundary:** generated values and reusable pieces are ready for the wallet view lane. Existing wallet views and `Theme.swift` are intentionally owned by that lane.

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
| `plate` / `plate-2` | `#0D2135` / `#091A2B` | `#10263C` / `#0A1A2B` | Balance surface; lifted from night background |
| `plate-ink` / `plate-soft` | `#F4EFE6` / `#A9B7C4` | `#ECE6D9` / `#94A4B5` | Primary / secondary text on navy |
| `plate-success` / `plate-warn` | `#5CCB98` / `#FFB857` | same | Status readable on navy in either theme |
| `text-subtle` / `line-control` | `#5F6E7D` / `#C9BFAA` | `#7D8EA0` / `#2C4258` | Supporting text / resting input outline |
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

- **Scale (px = pt):** caption 12, label 12 (mono, upper case), footnote 13, body-ui 14, headline 15, body 16, body-lg 19, title-3 22, title-2 30, title-1 44, display 72; amounts 40, 44 and 60 (48 on iOS for amount-xl).
- **Web display sizes** are fluid with `clamp()` around these values.
- **Korean** gets more line height (1.2 for headlines, 1.75 for body) and `word-break: keep-all`.

## Space, radii, shadows, motion

- **Spacing:** a 4-point scale (4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 96, 128).
- **Radii:**
  - xs 4, sm 8
  - md 12: the inner card, matching the wallet's `Radius.inner`
  - lg 16: the card, matching `Radius.card`
  - xl 24, window 12, pill
- **Shadows:** `sm`, `md`, `window` and `plate`, each defined for light and dark. Generated Apple layers retain offset, blur, spread, color and opacity; SwiftUI consumers select the appropriate surface layer. The balance plate uses its one offset shadow, never a colored halo.
- **Motion:** duration and cubic curves are generated in milliseconds for CSS and seconds for Apple. Panel/sheet spring response and damping are also generated. Use the event language below instead of inventing per-view animations.

## Motion and effect language — the dawn acknowledgement

The balance plate owns the expressive moment. A change has weight, then settles; the surrounding controls and lists stay quiet. A shine means a verified DBLN arrival. A success tick means a confirmed action. No animation establishes that money arrived or that a node is healthy: the caller supplies that truth and a stable event ID.

| Moment | Reusable Apple piece | Timing and behavior | Accessibility |
|---|---|---|---|
| Balance changes | `BalanceCountUp(amount:accessibilityText:format:)` | 240 ms standard cubic. Starts at the previous figure; first appearance is already settled. Exact Decimal target and spoken label remain authoritative; values outside safe interpolation range settle immediately. | Reduce Motion jumps to the final value. Reduce Transparency keeps opaque text. |
| DBLN arrives | `.dblnRewardShine(arrival:)` | One 600 ms narrow warm light crossing, only when a non-nil verified receipt ID changes. No appearance replay, burst, confetti or particle emitter. | Either preference removes the light overlay. |
| Sheet / panel changes | `.eastSeaPresentation(.sheet/.panel,value:)` | Sheet: spring response .36 s, damping .90. Panel: .32 s, damping .86. One continuous surface with small movement and no overshoot spectacle. | Reduce Motion uses identity and disables inherited animations. Reduce Transparency retains an opaque movement with no fade. |
| Navy plate depth | `NavyPlateDepth` / `.eastSeaNavyPlate()` | Fixed engraving, a single light edge, and a tokenized offset shadow. No pointer tracking, shader loop or changing light source. | Reduce Transparency replaces all composited depth with a flat opaque navy plate and solid edge. Reduce Motion preserves static depth. |
| Confirmed success | `.eastSeaSuccessFeedback(event:enabled:)` | One native iOS success feedback or Mac trackpad acknowledgement. No initial feedback; an optional user setting disables it. | Reduce Motion suppresses celebratory feedback. Reduce Transparency leaves nonvisual feedback unchanged. |
| Node event | `NodeStatusPulse(status:event:accessibilityLabel:)` | One 600 ms ring on a newly checked event. Connected dot, checking ring, paused bars, offline glyph: shape plus a sentence. Paused/offline never pulse. | Either preference suppresses the ring. The static glyph and sentence remain. |

**Idle contract:** no `Timer`, `TimelineView`, repeating animation, polling, or display link in these pieces. A receipt/status-ID change can create one cancellable finite task. Completion removes its overlay; disappearance, replacement events and accessibility changes cancel or settle it. A static panel has no scheduled work. Node polling belongs to the model, which must not invent event IDs on render or on a timer.

**Integration:** pass the existing localized amount formatter and exact accessible amount string. Pass receipt IDs only after confirmation, never from an optimistic balance delta. Pass the user's haptic preference. Coordinate a balance/shine on the same plate as one arrival moment; page chrome does not animate alongside it.

The pieces live under `apps/wallet/Sources/Design/`. `scripts/test-swift-pure.sh` compiles them in the `design-effects` target and checks Decimal edge cases, preference composition, finite effect bounds, dynamic appearances and native receive-QR decoding. Real trackpad/iPhone feedback and animation rendering require the later app integration phase.

## Menu-bar component

The four [menu mockups](../../docs/design/wallet-redesign/mockups/) were rendered before implementation: `menubar-{light,dark}-{en,ko}.png`.

`EastSeaDesign.MenuBarPanel` coexists with the existing `MenuBarPanel` in `ProverMenu.swift`. It is a 336 pt opaque popover with 16 pt gutters, a 128 pt mini navy plate, a 40 pt tabular balance, one node-status sentence and native switch, a 104 pt receive QR, and a 40 pt primary action. Labels, stable account identity, account name, verification truth, address, node binding, formatter and actions all come from the wallet lane. Switching account identity resets presentation state, so an existing receipt is not celebrated and the previous account's balance is not interpolated into the new account's amount. The caller can supply any supported language; this component does not infer language or health from a color.

The dawn is a direct Swift vector port of `dawn-flat.svg`. The receive QR encodes the supplied address with error correction M and an opaque four-module quiet zone in both appearances. There is no asset download or signing action in the component.

## Toolbox distribution

The toolbox frontend lives in a separate repository. This lane packages canonical CSS under `design/generated/toolbox/` and an applicable frontend patch at `docs/design/wallet-redesign/toolbox-redesign.patch`. Its source styles were rendered in an isolated checkout under `tmp/`; reference images and test fixtures are never product assets. The patch preserves portable standalone HTML by inlining the shared CSS, dawn mark and licensed font data during generation.

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
- `scripts/gen-design-tokens.py` generates all platform tokens and packaged web components; `design/scripts/build-tokens.mjs` is the compatibility entrypoint.
- `components.css` holds shared web presentation primitives.
- `apps/wallet/Sources/Design/` holds the accessible finite effects and namespaced menu panel.
- `design/scripts/subset-ko-font.py` builds the Korean headline font subset.
- `design/og/og-image.html` and `render-og.cjs` produce the social preview image.
