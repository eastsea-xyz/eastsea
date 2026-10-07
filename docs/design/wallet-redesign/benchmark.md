# Wallet redesign — benchmark

Date: 2026-10-07 · Lane: wallet design (design only, no Swift) · Follows
[docs/research/design-benchmark-2026.md](../../research/design-benchmark-2026.md)
(2026-09, IA and earnings rules), which this does not repeat.

The founder's brief (2026-10-07): "지갑도 통일성있게, 리디자인", and the site
should stop looking "too Claude-made". So this round asks a narrower question
than the September benchmark. That round asked what goes on Home. This one asks
what makes the best wallets and Mac apps look made by a person with taste, and
which of those moves fit a gold-and-navy East Sea wallet.

**Method and honesty note.** The patterns below come from the shipping apps,
their public product pages and release notes, and the September study. Live
search for single Dribbble and Mobbin shots returned nothing stable this
session, so the gallery links are entry points (search or app pages), not
specific shots. Mobbin needs a login.

---

## 1. References

### 1.1 Rainbow (iOS, extension) — <https://rainbow.me>
- **Balance hierarchy:** one total, very large, with the account's own avatar and colour above it. The colour is generated from the address, so each wallet gets its own colour.
- **Activity:** rows grouped by day. Each row has a token icon with a small direction badge (↗ ↙) in the corner, a title written as a sentence ("Sent ETH"), and the amount right-aligned in tabular digits.
- **Send flow:** recipient first, then amount. The review step names the contact and shows the fee as "network fee" in fiat, then hold-to-confirm.
- **Motion:** springy, physical, and every sheet is interruptible. Numbers roll when they change.
- **Take:** the corner direction badge on the token icon (already in `ActivityRow`). Day grouping. A per-account generated colour for the avatar only, never for the chrome.

### 1.2 Phantom (iOS, extension) — <https://phantom.com>
- **Balance hierarchy:** centred balance with a coloured delta under it, then four actions of equal size in a row (Receive, Send, Swap, Buy).
- **Send flow:** before signing, a simulation shows the balance change ("−2 SOL, +1 NFT"). Unknown tokens are hidden behind a "spam" fold.
- **Empty states:** one illustration, one sentence, one button.
- **Take:** the result preview before signing ("Mina gets 2.0 DBLN"). The folded unverified-token section. **Leave:** the centred hero plus round purple buttons. Every wallet built since 2022 looks like this. It is the strongest "AI-generated wallet" tell, and today's `RoundAction` copies it.

### 1.3 Family (iOS, Benji Taylor, wound down 2026) — <https://family.co>, design notes <https://benji.org>
- **Motion:** the reference for continuity. The "dynamic tray" grows and shrinks in place, so a sheet never jumps. Each step of a flow morphs out of the one before.
- **Iconography:** custom, soft, consistent stroke weight, and never a stock emoji.
- **Empty states:** they show what is possible instead of a sad face.
- **Take:** one surface whose size changes as you move through Send (compose → confirm → sent). No stack of separate modals. Progressive disclosure: show the basics, and reveal the rest when it becomes relevant.

### 1.4 Zerion (iOS, web) — <https://zerion.io>
- **Activity:** the best transaction grammar of the set. "Sent 2 ETH to vitalik.eth" is a sentence, with failed and pending shown inline in the row. Rewards and points get their own hub, not Home.
- **Take:** sentence titles; inline failed and pending states with the fix in the row. This is the model for B5 "새 가격으로 다시 보내기".

### 1.5 Apple Wallet and Apple Cash — <https://www.apple.com/apple-cash/>
- **Balance hierarchy:** the card is the brand moment, art with the balance printed on it. Everything under the card is a plain grouped list.
- **Rewards:** Daily Cash becomes one line in each transaction and adds into the balance. There is no rewards hero.
- **Send:** amount first in large numerals, then the recipient, then Face ID or Touch ID with the system sheet. The system prompt carries the app's reason text.
- **Take:** the **balance plate**. A navy card carries the coin and the balance, and it is the only "art" surface on Home. Rewards stay a single line. Amount-first send. Our `localizedReason` names the amount and the person.

### 1.6 Revolut (iOS, web) — <https://www.revolut.com>
- **Balance hierarchy:** account switcher pill above a big balance, with a short row of round actions.
- **Activity:** a merchant logo, or initials on a coloured disc, for every row. Pending rows are greyed with a "Pending" label. Taps open a detail page with a status timeline.
- **Take:** the status timeline (Signed → Sent → Confirmed / Dropped) on the detail. Initials-on-disc avatars for contacts. **Leave:** the density and the promo cards.

### 1.7 Mercury (web, iOS) — <https://mercury.com>
- **Balance hierarchy:** left-aligned, editorial. The balance sits beside a quiet chart, the accounts form a narrow right rail, and there is a lot of air.
- **Activity:** a table on desktop. Clicking a row opens a **side inspector** instead of navigating away. Status shows as small pills.
- **Typography:** one sans for UI, tabular figures everywhere money appears, and muted greys that still pass contrast.
- **Take:** the wide-Mac layout. Main column plus a 268 pt rail (Assets, Node rewards, one notice). Activity with an inspector pane. Pills for state.

### 1.8 Arc (macOS) — <https://arc.net>
- **macOS-native feel:** the sidebar is the app. Tinted translucent material, toolbar folded into the window, the window chrome barely visible.
- **Colour:** each space has its own colour and the chrome follows it.
- **Take:** let the sidebar carry the brand tint (sunken parchment or night) and the node and connection status at its foot, which already exists as `SidebarStatus`. A unified toolbar with only page-relevant actions.

### 1.9 Things 3 (macOS, iOS) — <https://culturedcode.com/things/>
- **Craft:** an obsessive vertical rhythm on a 4 pt grid. Section headers are small and quiet. Blue is used only for meaning (Today), and empty states are drawn with care.
- **Motion:** short and purposeful (≈ 0.25 s). Completing an item gives a single satisfying tick, never confetti.
- **Take:** one accent hue used only for meaning. Small, quiet group headers. One tactile moment per action: the "Sent" check, and the first reward ever.

### 1.10 Linear (macOS) — <https://linear.app>
- **Craft:** a dense list, a keyboard-first design, a command menu (⌘K), and an inspector on the right. Status icons are drawn so they read without colour (shape plus colour).
- **Dark mode:** a designed palette at parity with light, not inverted greys.
- **Take:** status glyphs that differ by shape (pending ring, dropped ⚠, done ✓). ⌘K search in the toolbar. Dark mode designed alongside light: gold primary at night, navy by day.

### 1.11 Cash App (contrast case) — <https://cash.app>
- **Send:** a giant amount keypad is the whole screen. Recipient and note come after.
- **Take:** the amount is the hero of the Send sheet (56 pt). "Max" is a chip, not a button.

### 1.12 Galleries (entry points)
- Dribbble wallet search: <https://dribbble.com/search/crypto-wallet> · macOS apps: <https://dribbble.com/search/macos-app>
- Mobbin (login): <https://mobbin.com> → iOS apps → Rainbow, Phantom, Revolut, Apple Wallet. Flows: "Sending money", "Transaction details", "Empty state".
- Apple HIG, macOS sidebars, toolbars and materials: <https://developer.apple.com/design/human-interface-guidelines/sidebars>

---

## 2. Patterns by topic (what we take)

| Topic | Pattern | From | Where it lands |
|---|---|---|---|
| Balance hierarchy | One art surface (navy plate with coin), balance printed on it; nothing else on Home is larger | Apple Cash, Rainbow | `BalancePlate` (spec §5.1) |
| Wide Mac | Main column + right rail, left-aligned | Mercury | Home ≥ 900 pt (spec §4.1) |
| Actions | Three equal capsules under the plate; Send is the only filled one, with the Touch ID glyph | Apple Cash, Things | `ActionRow` |
| Activity | Sentence titles, day groups, icon + direction badge, inline pending/failed with the fix | Zerion, Rainbow | `ActivityRow` v2 |
| Detail | Inspector pane on Mac, status timeline | Mercury, Linear, Revolut | `ActivityInspector` |
| Send | Amount first; known recipient chip; result preview; Touch ID with our reason text; the sheet morphs between steps | Cash App, Phantom, Family | `SendSheet` v2 |
| Rewards | One line on Home; the big number only on Node | Apple Cash, Zerion | `NodeLine`, `NodePage` |
| Empty | One mark, one sentence, one or two actions | Phantom, Things | `EmptyState` |
| Status | Shape + colour (ring, ⚠, ✓); never colour alone | Linear | `StatusGlyph` |
| macOS feel | Tinted sidebar, status in sidebar foot, unified toolbar, ⌘K | Arc, Linear | `SimpleDashboard.shell` |
| Motion | One continuous surface per flow; ≤ 0.25 s; one tactile moment per action | Family, Things | spec §9 |

## 3. Generic AI-app tells to remove

These are the things that make today's wallet and site read as "Claude-made":

1. **Violet accent, violet-to-pink gradients, aurora blobs, confetti.** `Color.aether`, the `[.aether, .pink]` avatar, `AuroraBackground`, `ConfettiBurst` on every reward.
2. **A centred hero, then three round tinted buttons, then cards stacked to the bottom.** This is the Phantom template every generated wallet copies (`RoundAction`).
3. **A tinted SF Symbol in front of a paragraph, inside a card, repeated.** Security today is four of these (`Image(...).font(.system(size: 34)).foregroundStyle(Color.aether)` + 60-word body).
4. **Everything in one accent colour.** Icons, tiles, spinners and charts are all `.aether`, so colour carries no meaning.
5. **Stat-tile grids for numbers nobody asked for** (`Tile` × 5 on Network: block, validators, block time, fee, mempool).
6. **SF Rounded heavy numerals.** The "friendly fintech" default. The brand system retires rounded for amounts.
7. **Long explanatory paragraphs where a status and one button would do.**

The replacements are a navy-and-gold palette with one meaning per colour, an editorial left-aligned layout, one art surface per screen, and status lists in place of paragraph cards. The serif (New York) appears only where the site uses its display face.
