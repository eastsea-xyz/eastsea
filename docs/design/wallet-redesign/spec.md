# Wallet redesign — spec

Date: 2026-10-07 · Status: design spec, ready for implementation after `claude/wallet-l10n` merges
Inputs: [benchmark.md](benchmark.md), [design/brand/tokens.json](../../../design/brand/tokens.json) (site lane, v1.0.0),
[docs/research/design-benchmark-2026.md](../../research/design-benchmark-2026.md), product rules in the team memory
(consumers get an answer, the tech stays hidden, every payment needs Touch ID, there is no seed phrase).
Mockups: [mockups/](mockups/) (PNG), sources in [mockups/html/](mockups/html/).

## 1. Direction

1. **The chart room at dawn.** The wallet uses the site's palette: parchment by day and night sea after dark, navy ink, and gold that is spent only on the coin, rewards and the night-time primary button. Violet, pink, the aurora and confetti are retired.
2. **One art surface, then calm lists.** A navy balance plate carries the doubloon and the number. Everything under it is quiet grouped rows that speak in sentences. A row says what happened, whether any money moved, and the one thing to do.
3. **A real Mac app.** The sidebar holds the status, the toolbar holds the page actions, the inspector holds the details, and Settings is its own window. The iPhone gets the same parts in four tabs.

## 2. Principles (each one a test a screen can fail)

- **P1. Nothing on a screen is larger than the balance.** The one exception is the Node page's "Received so far" figure, which is 40 pt, and Node is the only page where it appears.
- **P2. One meaning per colour.** Accent (sea blue) is for links, selection and tint. Success (green) is for money in, verified and confirmed. Gold is for the coin and rewards. Warn (amber) means it needs you soon. Danger (red) means money is at risk or an action failed. Everything else is ink.
- **P3. Status by shape, then colour.** A ring means pending, ⚠ means dropped or needs you, ✓ means done or verified, and a dashed ring means unverified. None of these depends on colour alone.
- **P4. A sentence, not a paragraph.** A card body is at most 2 lines (ko: about 40 characters a line, en: about 70), with "Details" behind a disclosure.
- **P5. No chain words on consumer surfaces.** "gas", "nonce", "mempool", "validator", "committee", "state root", "block #" and "DHT" may only appear in Developer mode or in "Details for experts". The B5 copy says "순서 번호", not nonce.
- **P6. Every payment shows Touch ID before it happens.** The primary Send button carries the Touch ID glyph (Face ID on an iPhone that has it, chosen by `LAContext.biometryType`). The system prompt's reason text names the amount and the recipient.
- **P7. One thing moves at a time.** No continuous animation except the live dot. Nothing moves under Reduce Motion.

## 3. Information architecture

### 3.1 Mac window (NavigationSplitView)

```
Sidebar (224 pt, sunken surface)          Detail
─────────────────────────────────         ───────────────────────────────
WALLET                                    Toolbar: page title · page actions · ⌘K search
  Home                                    Content: max 760 pt (Home: 960 with rail)
  Activity            [pending count]
  Explore
THIS MAC
  Node & rewards
PROTECTION
  Security            [amber dot = recovery not set]
  AI agent payments   [amber dot = payee request]
───────────────
Node on this Mac   [switch]   Verifying blocks
● Connected · Checked just now
```

| Today | New home | Why |
|---|---|---|
| Network page: status card, 5 stat tiles, network chart | **Removed.** Connection status → sidebar foot. Tiles and chart → Developer mode (Settings ▸ Developer, or ⌘⇧D shows them as a section at the bottom of Node) | Consumers don't need block time, fee or mempool (P5) |
| Network page: `NodeEarningsCard`, `NodeCard`, `RewardStandingCard` | **Node & rewards** page | One place for the node |
| Network page: `UpdateCard` | Settings window ▸ General, plus the "Check for Updates…" menu item (already exists) | Updates install by themselves |
| Security: 3 paragraph cards + `PaperKeyPanel` + `RecoveryPanel` + `AgentWalletPanel` + `ConnectedSitesSection` | **Security** = status list (3 protections) + Recovery group + Connected sites. **AI agent payments** is its own page | Status first, actions one click away |
| Activity: `LinkedWalletsCard`, `BalanceBreakdownCard`, `RewardDaysCard` above/below the list | Activity = filter chips + day-grouped list + inspector. Linked wallets → toolbar menu "Show history for…". Breakdown → Home rail "Where it came from" disclosure, or Activity toolbar ▸ Summary. Reward days → collapsed into one "Node rewards" row per day | The list is the page |
| Home: `HomeEarnings` hero card (Mac) | Home rail "Node rewards" mini card (Mac wide) or one `NodeLine` (narrow, iPhone) | Rewards explain the balance (Apple Cash), they never compete with it |
| Home: `BalanceCard` chart | Removed from Home v1. It returns later as a sparkline inside the plate (spec §11 Q3) | A flat 170 pt chart was a fake signal |

### 3.2 iPhone (TabView, 4 tabs)

`Home · Activity · Explore · Settings`. Settings is an inset grouped `Form` with the protection status, Recovery, Connected sites, Updates, Terms and rules, and Developer mode at the bottom. There is no Node tab, because rewards from a Mac appear as one `NodeLine` on Home.

### 3.3 Other Mac surfaces

- **Settings window** (`SettingsView`, being extracted by the l10n lane): General (updates, language, launch at login), Node (resources, storage, battery, unattended), Developer (developer mode, network stats, local devnet).
- **Menu bar panel** (`MenuBarPanel`): balance on a small plate (no coin), one node line, an "Open EastSea" button and the node switch. Prover detail moves to Developer.

## 4. Screens

Measurements are in pt, on a 4 pt grid. Mac content gutter: 32 (wide) / 16 (compact). iPhone gutter: 18.

### 4.1 Home — mockups `01-dashboard-en.png`, `02-dashboard-ko.png`, `10-states.png`
- **Mac wide (detail ≥ 900):** two columns, main `1fr` and rail 268, gap 28.
  - Main: `BalancePlate` (h 214) → `ActionRow` (3 equal capsules, h 46, gap 10, top 16) → "Recent activity" header + `ActivityList(limit: 3)` grouped.
  - Rail: `AssetsMini` (native + verified + first unverified, tap → Assets sheet) → `RewardsMini` (gold today figure + 12 hourly bars; only when the node is on) → at most one `Notice` (recovery not set, etc.).
- **Mac compact (< 900) and iPhone:** a single column. Plate (h 186), actions, then `NodeLine` (only when rewards exist or the node is on), then recent activity. The iPhone account row sits above the plate: an avatar (conic gradient from address hue, accent/navy/gold) with "Account 1" on the left and a QR button on the right that opens Receive.
- **Toolbar:** title "Home", then ⌘K search and a QR button (Receive). Send is not in the toolbar, because the primary action sits under the plate.
- **Banners** (one at a time, above the plate): recovery alert > network update needed > offline/paused > health. See §6.

### 4.2 Send — `03-send.png`
One sheet (440 wide on Mac, a `.large` detent on iPhone) that **morphs** between steps (`matchedGeometryEffect` on the amount and recipient):
1. **Compose.** Asset chip (icon + name, ⌄ menu) on the left and "Available 12.5" on the right. The **amount** is centred in 56 pt with tabular digits and the ticker at 22 pt in `text-subtle`, with a "Max" chip under it. Below that comes the "To" field: an empty field shows "Name, address or paste". A recognised address becomes a recipient chip (initial avatar, name or short address, "You sent here 3 times" in success). Then a summary group with "Network fee: up to X" and "Mina gets 2.0 DBLN". The primary button reads `[Touch ID] Send with Touch ID`.
2. **Look-alike block.** The full address is shown in mono, with the matching head and tail highlighted in warn tint. A warn notice compares it with the known address, and a checkbox ("I compared the whole address…") must be ticked before Send is enabled. First-time sends get one info line, not a block.
3. **Touch ID.** The system sheet. `localizedReason` = "send 2.0 DBLN to Mina (0x41a7…9c03)" / "미나(0x41a7…9c03)에게 2.0 DBLN 보내기".
4. **Sent.** A success check (64), "2.0 DBLN on its way to Mina", and the timeline Signed ✓ · Sent ✓ · Confirming ◌ "usually under 10 s". Buttons: "View in Activity" and "Done". The sheet can be closed at any time and the row keeps the state.
- Token send keeps today's frozen-intent confirm step (`confirmCard`), restyled as step 1.5 "Check the exact amount". Unverified units keep the explicit unit-count checkbox.
- A payment link fills the sheet with its fields read-only, and a gold-tint note says "A page asked for this payment".
- Refusals (dry-run revert, fee rose) show as a danger/warn notice above the button, and the sheet stays open (today's behaviour).

### 4.3 Receive — `04-receive.png`
A segmented control ("Address" / "Ask for an amount"), then a branded QR: navy dot modules, finder eyes with gold centres, the coin at the centre (52) on parchment 236. The address is printed in 4-character mono groups that alternate ink and subtle, over two lines. Hint: "Send only DBLN and EastSea tokens to this address." Buttons: Share… and **Copy address** (primary, which turns to "Copied ✓" for 1.5 s). "Ask for an amount" adds an amount and memo and encodes `eastsea:pay?to=…&amount=…&memo=…` (today's `paymentRequest` link).

### 4.4 Activity — `05-activity.png`
- Toolbar: title, ⌘K search, an Export menu (CSV, which is today's `EarningsCSV.activityDocument`), and "Show history for…" (linked wallets).
- Filter chips: All · Money in · Money out · Rewards · Security.
- Day groups ("TODAY", "YESTERDAY", then dates) in mono uppercase labels.
- `ActivityRow` v2 (§5.3). Node rewards collapse to **one row per day**: "Node rewards ▮▮▮ · 24 rewards from this Mac · +1.5". Expanding it in the inspector lists each reward.
- **Inspector** (Mac, 320 wide, sunken surface): a status pill, the amount at 34 pt, a one-sentence explanation, the status timeline, the one action, then "Details for experts" (hash, block, nonce, fee paid) behind a disclosure. On iPhone, tapping a row pushes the same content as a detail page.

### 4.5 Node & rewards (Mac) — `07-node.png`
1. **Node header card:** a success-tint Mac icon, the title "This Mac is part of the network" (when off: "Run the network on this Mac"), the live dot plus one line, and a large switch. Under it, a 3-cell stat strip: online this session · latest block checked · power.
2. **Rewards card** (only once the node has run): on the left, a mono label "RECEIVED SO FAR", the figure at 40 pt in gold-text, "+1.5 today" in success, and the rule sentence (max 2 lines). On the right, a 24-hour per-hour bar chart: gold bars, zero hours as 3 pt stubs, and the current hour hatched.
3. **Voting seat** group: state icon, "Candidate · 18 of 24 hours online", a progress bar in accent, then "Network reward rules" as a disclosure that quotes `VotingRules.mainnetRewardsRule` verbatim.
- `EarningsHero`, `AuroraBackground` and `ConfettiBurst` are retired. The only celebration is the first reward ever: a single gold ring pulse on the figure (0.6 s) and a haptic or sound on iPhone. Never confetti.
- GPU proving, when available, is one row in the seat group ("Prove blocks on this Mac's GPU", switch).

### 4.6 Security — `06-security.png`
- Hero: a navy seal tile (64) with a shield, the title "**2 of 3 protections are on**" (or "Your wallet is fully protected"), and a one-sentence explanation of Secure Enclave with no seed phrase.
- A status list of three rows with ✓ or ⚠ glyphs: Touch ID per payment · Checks before you send · A way back if this Mac is lost [Set up]. This replaces the three paragraph cards.
- RECOVERY group: Recovery device [Add…], Recovery words [Create…], Recover a lost account ›. Each opens a sheet holding today's `RecoveryPanel` and `PaperKeyPanel` content, restyled as numbered steps.
- CONNECTED SITES group (`ConnectedSitesSection`).
- Footnote: who sees your addresses (one sentence, `text-subtle`).
- `KeyExposureNotice` texts move into the sheets where they apply (recovery explains it saves a lost key, not a stolen one).

### 4.7 AI agent payments (Mac) — `08-agents.png`
- Page title: "AI agent payments" / "AI 에이전트 결제" (the l10n lane's name; it replaces "AI 비서" everywhere, including body copy: 비서 → 에이전트). Toolbar: "Stop all agents" / "모든 에이전트 멈추기" (danger text, needs Touch ID).
- **Payee request card** (gold 1 pt border and 4 pt gold halo, at the top only while requests exist): agent tile, "Claude Code wants to add a new recipient", the line "Until you allow it, not a single coin can go there", a key/value grid (recipient, requested amount, purpose), and the buttons [Decline] [Touch ID Allow]. The name field for the payee is prefilled from the request and stays editable.
- **Active session** group: agent, expiry, "key in this Mac's Secure Enclave", [Change limits], and a 3-cell strip: per payment · spent today with meter · approved recipients.
- **Agent history** group: rows written as sentences ("Paid Shop · Claude Code"), with the purpose in quotes.
- Footnote: limits are enforced by the account contract on chain, so even a tricked agent cannot go over them.
- Empty state: an agent tile, "Let an AI agent pay for you, within limits", the copyable `aether-agent setup all --apply` (to be renamed with the CLI), and [Connect an agent…].
- Korean is the source copy for this page today. `AgentWalletPanel` is hard-coded Korean, and the l10n lane is adding English.

### 4.8 Onboarding — `09-onboarding.png`
A 560 × 640 window with no sidebar and 3 steps, with dots at the bottom:
1. **Welcome** (always navy, both themes): the engraved coin at 200, the serif italic display "Your money, on your Mac." / ko "내 돈을, 내 Mac에서." (Hahmlet-style serif is not native, so use `Font.system(.largeTitle, design: .serif)` bold for ko), a sub line, a **gold capsule** "[Touch ID] Create my wallet", and fine print saying there is no seed phrase.
2. **Before you start:** `TermsSheet` reduced to **3 lines + link** (testnet, no sale or promise, your responsibility) and [Quit] [I understand and agree]. The mainnet rules move to Node ▸ rules (§3.1).
3. **Ready:** a success check, "Your wallet is ready.", the address card, and [Set up recovery now] (primary) and [Later — open my wallet]. A node switch row offers to also run a node, **off by default** (decision for lead/legal, §11 Q1).

`VotingNodeInvite` is restyled the same way: 3 consent rows + [Not now] [Touch ID Join].

## 5. Component library

New shared views go in a new file `apps/wallet/Sources/DesignSystem.swift` (tokens) and `apps/wallet/Sources/Components.swift` (views), so `SimpleDashboard.swift` (1,946 lines) shrinks rather than grows.

| Component | SwiftUI | Replaces | File(s) |
|---|---|---|---|
| Tokens | `extension Color { static let esBg, esSurface, esSunken, esLine, esLineStrong, esText, esMuted, esSubtle, esAccent, esAccentFill, esOnAccentFill, esGold, esGoldText, esSuccess, esWarn, esDanger, esSea, esDawn }` built from one `dynamic(light:dark:)` helper (the `Color.warn` pattern in Theme.swift) | `Color.aether`, ad-hoc `.green/.red/.orange` | `Theme.swift` → becomes `DesignSystem.swift`; delete `Color.aether` in `SimpleDashboard.swift:195` |
| Type | `Font.esDisplay(serif)`, `.esAmountXL (60/48 semibold, .monospacedDigit())`, `.esAmountL (40)`, `.esTitle (22)`, `.esHeadline (15 semibold)`, `.esBody`, `.esFootnote`, `.esLabel (11 mono, uppercase, tracking .08)` | `Font.display` (rounded), `heroNumber` (heavy rounded) | `Theme.swift` |
| `BalancePlate` | ZStack: navy `LinearGradient(sea → sea2)` + soft gold radial + `EngravedWaves` (Canvas, gold 20%) + the `DoubloonCoin` image (190, offset right, cropped) + label, amount, `VerifiedLine` | `HomePage.balanceText`, `VerifiedBadge`, `accountButton` | `Components.swift`, used in `SimpleDashboard.swift HomePage` |
| `VerifiedLine` | The states of `VerifiedBadge` as one line on the plate: ✓ Verified on this Mac · just now / ◌ Checking… / ⏸ Last verified 21:10 / 🔒 key locked | `VerifiedBadge` (keep its logic) | `SimpleDashboard.swift` |
| `ActionRow` / `ActionButton` | 3 equal capsules; `.primary` = accent-fill + Touch ID glyph | `RoundAction` | `Components.swift` |
| `ESButtonStyle(.primary/.secondary/.ghost/.danger, size: .regular/.large)` | Capsule, 34/44 high; primary fill = `esAccentFill` (navy by day, gold by night) | `.borderedProminent` + `.tint(.aether)` | `Components.swift`; apply in sheets |
| `Group` (inset list) | Surface fill, 14 radius, 0.5 hairline, `shadow.sm`; `GroupRow` with leading 34 icon, 2-line text, trailing value/chevron/button; hairline inset 60 | `Card`, `Tile` | `Components.swift`; `Card` stays as an alias during migration |
| `SectionHeader` | Title 15 semibold + trailing link; or `.label` mono uppercase | ad-hoc `Text(...).font(.aeHeadline)` | `Components.swift` |
| `ActivityRow` v2 | Leading `TxIcon` (token icon + corner badge: ↙ success, ↗ muted, ⚠ warn, ✕ danger), sentence title, sub line (time · state sentence from `TxStatusText`), inline action (B5 resend), trailing amount (+ success / − ink / dropped struck-through subtle) and unit | `ActivityRow` | `SimpleDashboard.swift` (or `ActivityViews.swift`) |
| `ActivityInspector` | Pill, amount, explanation, `StatusTimeline`, action, expert disclosure | — (new) | `ActivityViews.swift` |
| `StatusTimeline` | Vertical steps: ✓ success · ◌ warn ring · ✕ warn | — | `Components.swift` |
| `StatusGlyph` | `.pending` ring (12, 2 pt, warn), `.done` ✓, `.dropped` ⚠, `.failed` ✕ | `OrbitSpinner` in rows | `Spinners.swift` (keep `OrbitSpinner` for the plate only) |
| `Notice(kind: .warn/.danger/.info/.plain)` | 12 radius tint strip, glyph, **bold lead** + one sentence + optional link/button | ad-hoc `.background(Color.warn.opacity(0.14))` blocks (×9) | `Components.swift` |
| `NodeLine` | One row: dot (gold/subtle/warn) + sentence + trailing figure + chevron | `NodeRewardsLine`, `HomeEarnings` | `Earnings.swift`, `SimpleDashboard.swift` |
| `Pill` | 22 high capsule, tint bg + meaning colour text | `LivePill`, badge capsules | `Components.swift` |
| `Chip` / `ChipBar` | Filter chips, the selected one in ink fill | — | `Components.swift` |
| `StatStrip` | 3 cells divided by hairlines; value 20 / label 11.5 | `Tile` grid, `StatTile` | `Components.swift` |
| `HourBars` | Swift Charts `BarMark`, gold, zero stubs, hatched current | `NetworkCard` chart, `BalanceCard` | `Earnings.swift` |
| `RecipientChip` | Initial avatar + name / short address + history note | `TextField` only | `SimpleDashboard.swift SendSheet` |
| `AmountField` | Centred 56 pt amount, ticker, Max chip | `TextField("0")` | `SimpleDashboard.swift SendSheet` |
| `AddressText` | Mono 4-character groups, alternating ink/subtle, optional head/tail highlight | `Text(address).monospaced()` | `Components.swift` (Receive, Send look-alike, inspector) |
| `BrandedQR` | Rounded modules navy, gold-centre finders, coin knock-out (error correction **H**) | `QRCode` (M, square) | `SimpleDashboard.swift QRCode` |
| `EmptyState` | Flat coin 44, title 15 semibold, one sentence, 1–2 buttons | `ActivityList` empty text | `Components.swift` |
| `SidebarStatus` | Same content, restyled: node switch row + live dot row | `SidebarStatus` | `SimpleDashboard.swift` |

### 5.6 Icons
SF Symbols, monochrome, `.regular` weight in the sidebar (tinted accent), `.medium` in buttons. The mockups' stroke icons stand for these:
home `house`, activity `clock`, explore `safari`, node `desktopcomputer`, security `checkmark.shield`, agents `apple.terminal`, send `arrow.up.right`, receive `arrow.down.left`, assets `square.stack.3d.up`, Touch ID `touchid` (Face ID `faceid`), copy `doc.on.doc`, QR `qrcode`, recovery device `laptopcomputer.and.iphone`, words `doc.text`, key `key`, warn `exclamationmark.triangle`, info `info.circle`, dropped `exclamationmark.triangle.fill` (badge), resend `arrow.clockwise`, offline `wifi.slash`, paused `pause.circle`, stop `stop.circle`.
Remove the per-card 34 pt tinted symbol pattern (benchmark §3 tell 3).

## 6. States

| State | Home | Row / detail | Copy (en / ko) |
|---|---|---|---|
| **Empty** (balance 0, no history) | Plate shows `0 DBLN` ✓; Send dimmed 40%; `EmptyState`: "Your first DBLN starts here" + [Show my address] [Turn on node] (Mac) | — | "Share your address to get paid, or turn on the node." / "주소를 보내 받거나, 노드를 켜 보세요." |
| **Loading** (first verify) | Plate skeleton (170×44 shimmer, the existing `ShimmerBar`), coin at 50%, `◌ Checking on this Mac…`; after 20 s "Still checking — the network is slow. Your funds are safe." | Two skeleton rows | "확인 중…" / "아직 확인 중이에요. 네트워크가 느려요. 자금은 안전해요." |
| **Pending tx** | Row: ring + "Confirming · usually under 10 seconds"; sidebar Activity badge count | Timeline: Signed ✓ Sent ✓ Confirming ◌ | from `TxStatusText` (l10n lane) |
| **Waiting on price** (`state_price_above_cap`) | Row: ring (warn) + "Waiting — the network is busy. It goes through when the price comes down." | Timeline step "Waiting for a lower price" + countdown to the 10-min cancel | `TxStatusText` |
| **Dropped (B5)** | Row: ⚠ badge, title "Didn't go through · to Mina", warn line "The network price rose before it was processed. Nothing left your wallet.", inline secondary button **"Resend at today's price" / "새 가격으로 다시 보내기"**, amount struck through in subtle | Inspector: pill "Didn't go through", the explanation, timeline with ✕ Dropped and the price then vs now, primary button, note "Uses the same slot as the dropped one, so at most one of the two can ever go through." / "처음 송금과 같은 순서 번호를 써요. 둘 중 하나만 처리되니 두 번 나갈 일은 없어요." | Button opens Send prefilled (`beginResend`), with a **price then → now** comparison and `Touch ID 다시 보내기` |
| **Failed** (included, reverted) | ✕ badge, danger line "It reached the network but didn't run. Your fee was used; the amount did not move." | Expert details show the revert reason | |
| **Error: send refused** | Danger notice in the sheet: "Not sent. <reason>. Nothing moved." Sheet stays open | — | |
| **Offline** (no route) | Warn banner: "Can't reach the network. Showing what this Mac last verified, 4 min ago. Sending waits until it's back."; plate label "BALANCE · 4 MIN AGO"; verified line ⏸ in warn; list at 55% opacity; Send disabled | Sidebar dot warn: "Reconnecting · Last checked 4 min ago" | ko: "네트워크에 연결할 수 없어요. 이 Mac이 4분 전에 확인한 잔액이에요." |
| **Chain paused** | Same pattern: "The network is paused, not your wallet. Nothing is lost." (`NetworkPausedBadge` logic) | | |
| **Update needed** | Warn banner with deadline; "installs by itself" | | `UpgradeNoticeCard` text |
| **Incoming recovery** | Highest-priority banner (warn), [Cancel it] primary + "Cancel and remove all recovery keys" in the detail | | existing copy |
| **Storage low / node half-dead** | `HealthBanner` (keep logic) as a Notice; plate verified line shows warn | | existing |
| **Key locked** (Secure Enclave unavailable) | Verified line 🔒 warn: today's `keyError` | | existing |

Banner rule: at most **one** banner on Home. Priority: incoming recovery > key locked > update deadline < 24 h > offline/paused > storage > update scheduled. The rest stay reachable on their own page.

## 7. Tokens

### 7.1 Source
Colours, spacing, radius, shadow and motion come from **`design/brand/tokens.json`** (site lane, 2026-10-07). The wallet is a consumer of that file. When it changes, `DesignSystem.swift` changes with it. A generator, `design/scripts/build-tokens.mjs → DesignSystem.generated.swift`, is a natural follow-up; for now, a hand-written mapping is fine.

### 7.2 Colour mapping

| tokens.json | Light | Dark | Wallet use |
|---|---|---|---|
| `bg` | #F4EFE6 | #071320 | Window and sheet background |
| `surface` | #FBF8F2 | #0D1C2C | Groups, inputs, secondary buttons |
| `surface-sunken` | #ECE5D8 | #0A1826 | Sidebar, inspector, wells, segmented track |
| `line` | #D8CEBB | #1F3347 | Hairlines |
| `line-strong` | #A99F8C | #4A6178 | Focused/hovered control border |
| `text` | #0D2135 | #ECE6D9 | Primary text |
| `text-muted` | #4B5B6B | #94A4B5 | Secondary text |
| `accent` | #0F5A75 | #7CC4DC | Tint: links, selection, sidebar icons, switches, progress |
| `accent-fill` / `on-accent-fill` | #0D2135 / #F4EFE6 | **#E8BF59 / #0D2135** | The one primary button (navy by day, gold by night) |
| `gold` | #E8BF59 | #E8BF59 | Coin, reward bars, reward dot, Touch ID glyph on the navy button |
| `gold-text` | #7E5A0E | #E8BF59 | Reward figures |
| `success` | #1E7A52 | #5CCB98 | Money in, verified, confirmed, live dot |
| `warn` | #AD6100 | #FFB857 | Pending ring, needs-you, dropped |
| `danger` | #B42318 | #FF8A7A | Failed, refused, money at risk, destructive |
| `sea` | #0D2135 | #030B14 | Balance plate base (dark-theme plate is lifted to #10263C so it separates from bg) |
| `sea-line` | #E8BF59 @ 20% | same | Engraved waves on the plate |
| `dawn` | #D9673F | #F08F6C | Not used in the wallet v1 (reserved; at most one accent per view) |

**Wallet additions: please fold these into tokens.json (flagged to the site lane):**
- `text-subtle` #5F6E7D / #7D8EA0. This is a third text level for timestamps and units (4.6:1 and 5.6:1). Without it, `text-muted` has to carry both levels.
- `line-control` #C9BFAA / #2C4258. This is the resting border of secondary buttons and inputs, quieter than `line-strong`.
- `plate` / `plate-2` (the balance plate's gradient stops) and `plate-soft` #A9B7C4 (secondary text on the plate).
- Tints: `<meaning>-tint` = 12–14% of the meaning colour over `surface` (SwiftUI `.opacity(0.12)` over the surface).

### 7.3 Type (tokens.json `font.family.apple`)
- Text: SF Pro (`.default`). Korean falls to Apple SD Gothic Neo. Set `.lineBreakStrategy(.hangulWordPriority)` app-wide (iOS 17 / macOS 14) so Korean breaks at word boundaries, which is what the mockups' `word-break: keep-all` shows.
- **Amounts: SF Pro `.default` + `.monospacedDigit()`, semibold.** Rounded is retired (tokens.json `amount`). Sizes: plate 60 (Mac) / 48 (iPhone); Send 56; inspector 34; Node figure 40; rows 14.
- Display: New York (`design: .serif`), italic only for English onboarding headlines. Korean display uses `.serif` bold (no italic in Korean).
- Labels: SF Mono 11, uppercase, tracking 0.08 for group headers, day headers and the plate label (tokens.json `label`). For Korean labels, use SF Pro 11 semibold without uppercase, since mono Hangul falls back anyway.
- Addresses: SF Mono, grouped by 4.
- Scale: caption 12 · footnote 13 · body 13.5 (Mac) / 15 (iOS) · headline 15 semibold · title 22 · amount-L 40 · amount-XL 56–60.

### 7.4 Space, radius, shadow
- Space is on a 4 grid (tokens.json `space`): 4 · 8 · 12 · 16 · 20 · 24 · 32. Section gap 22–24, group inner padding 14–16, row 11 × 16.
- Radius: groups 14 (between md 12 and lg 16; use **lg 16** if the site wants exact parity, as the mockups are 14), plate 18, sheets 14 (Mac system) / 22 (iOS sheet), buttons pill, inputs 9 (sm 8), window 12.
- Shadow: groups `shadow.sm`; sheets and windows `shadow.window`; the plate has its own (0 14 30 rgba(sea,.22)).

## 8. Token icon family — `11-token-icons.png`

The rules are unchanged (`TokenIconSpec.swift` is authoritative, restated in tokens.json `tokenIcon`). The new art is from `codex/brand-art`:
- **Native coin:** at ≥ 48 pt, the engraved `dbln-coin-1024.png` (asset `DoubloonCoin`, replace the image). Below 48 pt, the **flat dawn mark** (`dawn-flat.svg` / `dbln-coin-flat-256.png`): a gold disc with the navy sun and sea. Today's vector `DoubloonArt` (gold gradient with a diamond punch) is replaced by a SwiftUI port of `dawn-flat.svg` (3 paths and 3 rays, viewBox 32). The flat mark is the same as the site favicon.
- **Official:** a saturated disc with a cream glyph (`design/brand/tokens/<SYMBOL>-256.png` for WAETH, NEB, ORB and CMT), bundled as images, replacing `OfficialTokenArt`'s drawn shapes. They are chosen by address only.
- **Generated:** unchanged. A pastel HSL(seed, 45%, 62%) fill, a dark letter, a **dashed** ring and a "?" badge.
- **Badges on top** (activity): a 16 pt circle cut out of the icon at the bottom-right, with a 12 pt meaning-coloured disc and a white glyph inside.
- Sizes: row 34 (activity) / 28 (lists) / 22 (chips) / 20 (menus) / 40 (Assets list) / 64 (detail).

## 9. Motion

| Moment | Motion | Duration / curve |
|---|---|---|
| Balance changes | `.contentTransition(.numericText())` (kept) | 0.24 s, standard (`cubic-bezier(.2,.7,.2,1)` ≈ `.snappy`) |
| Send steps | One sheet; amount and recipient `matchedGeometryEffect`; the height animates | 0.24 s |
| Sent | Check scales 0.6 → 1 with a spring, then the timeline rows fade in, staggered 60 ms | 0.4 s total |
| Pending ring | Rotation 1 s linear (the only continuous motion on a row) | — |
| Live dot | 2.5 s opacity breathe (kept from `LivePill`), only on the sidebar foot and the Node header | — |
| New reward | `NodeLine` / rail figure flashes gold-tint once | 0.6 s |
| First reward ever | One gold ring pulse on the Node figure + `.sensoryFeedback(.success)` (iOS) | 0.6 s |
| Row inserted | `.transition(.move(edge: .top).combined(with: .opacity))` | 0.24 s |
| Reduce Motion | All of the above become cross-fades or nothing, and the ring becomes a static ◌ | — |

Retired: `AuroraBackground`, `ConfettiBurst`, `FloatingReward`, `EarningsHero` keyframe scale.

## 10. Copy: ko/en length

- Write Korean in 해요체, and write English in sentence case without exclamation marks.
- **Budgets** (the mockups fit both):
  - Button: ko ≤ 10 characters, en ≤ 22 ("새 가격으로 다시 보내기" = 11 is the agreed exception and needs its own row line).
  - Row title: ko ≤ 18, en ≤ 32, then truncated with the address in the middle.
  - Row sub line: ko ≤ 34, en ≤ 60, two lines max for warn/danger.
  - Banner: bold lead ≤ ko 14 / en 28, then one sentence.
  - Sidebar items: ko ≤ 9, en ≤ 18 (the longest is "AI 에이전트 결제" / "AI agent payments").
- English runs about 1.6–1.9× longer than Korean in characters, but at the same pt size it sets about 1.1–1.3× as wide. Layouts are checked in **English** for width and **Korean** for line breaks (keep-all).
- Numbers are never translated. The ticker stays `DBLN`. Korean puts the time phrase after the value ("오늘 +1.5").
- Key strings:

| Key | en | ko |
|---|---|---|
| verified | Verified on this Mac · just now | 이 Mac에서 확인함 · 방금 |
| send.primary | Send with Touch ID | Touch ID로 보내기 |
| send.known | You sent here 3 times | 3번 보낸 적 있는 주소 |
| send.lookalike | This only looks like Mina's address. | 미나의 주소와 비슷하지만 다른 주소예요. |
| tx.dropped.title | Didn't go through · to Mina | 미나에게 보내지 못함 |
| tx.dropped.line | The network price rose before it was processed. Nothing left your wallet. | 네트워크 가격이 올라 처리되지 않았어요. 돈은 그대로 있어요. |
| tx.resend | Resend at today's price | 새 가격으로 다시 보내기 |
| node.line | Node rewards · +1.5 today | 노드 보상 · 오늘 +1.5 |
| recovery.notice | Set up recovery. If this Mac is lost, nobody can bring the funds back. | 복구 방법을 정해 두세요. 이 Mac을 잃어버리면 누구도 자금을 되찾아 줄 수 없어요. |
| onboarding.hero | Your money, on your Mac. | 내 돈을, 내 Mac에서. |

These go into the l10n lane's `Localizable.xcstrings`. English strings here are the source; for AI agent payments, Korean is the source until that page is localised.

## 11. Light and dark

Both themes are designed together, and neither is an afterthought:
- **Light:** parchment window, sunken sidebar, white-warm groups, a navy plate, a navy primary button with a gold Touch ID glyph.
- **Dark:** night-sea window, a plate lifted to #10263C with a 0.5 pt light edge so it reads against the background, a **gold primary button** with a navy label and glyph, and success and warn in their bright variants.
- Onboarding Welcome is navy in both themes. It is the brand moment.
- Contrast: all text pairs ≥ 4.5:1 (tokens.json lists its ratios; the wallet additions are ≥ 4.6:1). Gold is never body text on light.

## 12. Open questions and flags

- **Q1 (lead/legal):** Should onboarding offer the node switch, and with which default? The mockup shows it on; the spec says **off by default** until legal confirms the reward wording ("can receive node rewards") is OK there.
- **Q2 (site lane):** Add `text-subtle`, `line-control`, `plate*` and the tint rule to tokens.json (§7.2), and settle the group radius at 14 or 16.
- **Q3:** The balance history sparkline inside the plate (1W) needs real history first. It is not in v1.
- **Q4:** The CLI is still named `aether-agent`. The agents empty state shows it verbatim until the rename lands.
- **Q5:** Hahmlet (the site's Korean serif) is not bundled in the app. The ko onboarding headline uses the system serif. Bundling Hahmlet (OFL) for that one line is optional.
