# eastsea.xyz redesign: benchmark (2026-10-07)

Founder's brief: "eastsea.xyz 디자인이 너무 클로드로 만든거 같아. 드리블같은데서 벤치마킹해서 새로 만들어줘."
The old page looked machine-made. This note records what we studied, what we took from each reference, and what we avoided.
Every reference was opened in a browser and screenshotted on 2026-10-07 at 1440 px.

## What made the old page read as "AI-made"

- A centred, symmetric hero, with a glowing coin floating next to the headline.
- Four identical icon cards in a grid, each with a thin-line icon, a title and a paragraph.
- A "kicker, H2, cards" rhythm repeated in every section, so nothing had its own shape.
- A system font for everything, plus one decorative italic used only in the hero.
- Status and legal copy written as one dense paragraph, which reads as fine print.
- No product on the page: the reader never saw what the app looks like.

## References

| # | Reference | URL | What we took |
|---|---|---|---|
| 1 | Mercury | https://mercury.com | **Art direction as a mood.** A real landscape at dawn, not an illustration. The regulatory notice sits in a pill on the hero and is not hidden in the footer. We took the dawn-landscape idea and the honest notice that is always on screen. |
| 2 | Linear | https://linear.app | **Left-aligned display headline and a product UI as the hero image.** The app window is cropped off the edge and sits close to the type. We draw our wallet window in HTML/CSS the same way, so it stays sharp and themeable and loads no image. |
| 3 | Arc / The Browser Company | https://arc.net | **Serif display type in a consumer tech product** (ABC Oracle with a serif headline), warm paper backgrounds and wavy dividers. It shows that a serif can feel modern and calm, not old. |
| 4 | Teenage Engineering | https://teenage.engineering | **Data as typography.** Mono labels, numbers set large, everything on a strict grid. Our node-reward table is set like a printed tide table, with large numerals and mono column labels, not as a pricing-card grid. |
| 5 | Rainbow | https://rainbow.me | **The wallet as the hero.** A credible balance screen with tabular figures, real-looking rows and token marks. We copied the information density, not the neon. |
| 6 | Family | https://family.co | **Restraint in a crypto wallet:** short sentences, one primary action and a lot of whitespace. Its illustration style is not ours, but the consumer tone is ("give consumers an answer"). |
| 7 | Apple, MacBook Air | https://www.apple.com/macbook-air/ | **The product is the art; the headline sits left, under it.** One product shot fills the hero, with no decoration around it. A short headline with a single product name follows. Native details (SF type, traffic lights, a 12 px window radius) are why our window copies macOS chrome exactly. |
| 8 | Stripe Press | https://press.stripe.com | **Editorial craft:** a serif wordmark with an italic tagline, mono small caps on the book spines, and warm dark paper. Objects (books) carry the colour, and the page stays quiet. Our pull-quote, the roman-numeral rules (i., ii., iii.) and the mono labels come from here. |
| 9 | Phantom | https://phantom.com | **Brand colour carried by one strong illustration** inside a rounded frame, with everything else neutral. Our equivalent is the sun-on-the-horizon scene in the hero. |

### Anti-references, studied so we would not repeat them

- **Dribbble, "crypto wallet landing page"** (https://dribbble.com/search/crypto-wallet-landing-page).
  - Nearly every shot is the same: a dark UI, neon green or purple, a phone mockup on a laptop, and a glowing centred hero.
  - This is exactly the "AI landing page" look the founder rejected. We used it as a checklist of what to avoid.
- **Awwwards, fintech category** (https://www.awwwards.com/websites/fintech/).
  - The winners rely on heavy WebGL and scroll-jacking. That cuts against our Lighthouse ≥ 95 target and against a calm, trustworthy voice.
- **Godly** (https://godly.website).
  - The 2026 trend is brutalist type and experimental motion.
  - We took only the confidence of big type and left the noise.

## Direction chosen: "Dawn almanac"

1. **East Sea at dawn as the one piece of art.** A gold sun rests on a crisp horizon above a navy sea engraved with fine wave lines: the coin's motif, drawn at page scale. The wallet window sits on the water.
2. **An almanac or tide-table voice.** We use a serif display face (Newsreader) for headlines, Geist for UI text, and Geist Mono for labels, numbers and addresses. Korean headlines use Hahmlet, a Korean serif that pairs with Newsreader. Hairline rules replace cards wherever we can.
3. **Honest by layout.** There is a testnet bar on every page. A "Today / Planned for mainnet" ledger gives the true-today vs planned split its own section. The no-sale sentence is set as the largest type on the page after the hero.

### What we deliberately avoided

- A centred hero, gradient blobs and glassmorphism cards.
- Emoji or line-icon grids, and three or four identical feature cards.
- Inter-only type, and "Built for X" filler copy.
- Fake screenshots. The window is labelled as an illustration of the testnet app. It shows only screens and strings that exist in `apps/wallet/Sources` ("Account 1", "Recent activity", "Node on this Mac", "Protected by this device", "Confirm the send"). The amounts are marked as examples.

## Type, colour and assets

- **Type.** All fonts are SIL OFL 1.1 and self-hosted in `site/fonts/` as subsets, about 132 KB in total. Hahmlet loads only when Korean is shown.
- **Colour.** The palette comes from the coin and the sea: paper `#F4EFE6`, navy `#0D2135`, gold `#E8BF59`, sea accent `#0F5A75`, and dawn coral `#D9673F` used once per view.
  - Dark mode is "night sea" (`#071320`), with the gold button.
  - All roles live in `design/brand/tokens.json` (see `design/brand/SYSTEM.md`).
- **Brand art.** The coin, favicon and app icon come from the Codex brand-art lane (worktree `brand-art`, `design/brand/*`, uncommitted at the time of writing). The OG image is rendered from `design/og/og-image.html`.
