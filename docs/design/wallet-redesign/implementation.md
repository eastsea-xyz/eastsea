# Wallet redesign — implementation plan

For: the SwiftUI coder(s) who pick this up after `claude/wallet-l10n` merges.
Read first: [spec.md](spec.md) (what), [mockups/](mockups/) (how it looks), `design/brand/tokens.json` (values).

Ground rules:
- **Start after the l10n lane merges.** It is rewriting strings in the same files (`SimpleDashboard.swift`, `AgentWalletPanel.swift`, `Onboarding.swift`, and others) and adds `Localizable.xcstrings`, `SettingsView.swift`, `TxStatusText.swift`, `AppLanguage.swift` and the `WALLET_SCREENS` harness. Every new string goes through `String(localized:)` or `LocalizedStringKey`. `scripts/check-wallet-l10n.sh` must stay green.
- **Never launch a built app on real data.** First-launch migration moves the real node folder. Verify only through the screens harness (below) and the pure tests.
- Visual only. Do not change send or verify logic, `TokenIconSpec` classification, or any frozen-intent and dry-run behaviour. When a step needs new model state, it is called out.
- Each step is one commit and leaves the app compiling on macOS 14 and iOS 17.

Two lanes. **Lane A** does steps 1–8, and Lane B starts after A's steps 1–2 are merged.

---

## Lane A: foundation, Home, Activity, Send, Receive, icons

### A1. Tokens: `DesignSystem.swift` (new) + `Theme.swift`
- Add `apps/wallet/Sources/DesignSystem.swift`:
  - A `Color.dynamic(light: UInt32, dark: UInt32)` helper (the NSColor/UIColor dynamic-provider pattern from `Color.warn` in `Theme.swift`).
  - `Color.es*` for every row of spec §7.2, plus the wallet additions `esSubtle`, `esLineControl`, `esPlate`, `esPlate2` and `esPlateSoft`.
  - `Font.esAmountXL` (60 macOS / 48 iOS, `.semibold`, `.monospacedDigit()`), `.esAmountL` (40), `.esAmountM` (34), `.esTitle` (22 semibold), `.esHeadline` (15 semibold), `.esBody`, `.esFootnote`, `.esCaption`, `.esLabel` (SF Mono 11 medium; uppercase and `.tracking(0.9)` are applied by a `LabelText` view) and `.esDisplaySerif` (`.system(.largeTitle, design: .serif)`).
  - `enum Space { 4, 8, 12, 16, 20, 24, 32 }`. `Radius` gains `group = 14`, `plate = 18`, `control = 9`.
- `Theme.swift`: keep `Font.ae*` as aliases for now, and point `Color.warn` at `esWarn`. Delete `Font.display` (rounded) and `Font.heroNumber*` in A9.
- `SimpleDashboard.swift:195`: change `static let aether` to `@available(*, deprecated) static let aether = Color.esAccent`, so the 25 call sites compile until they are migrated.
- **Test:** add `apps/wallet/Tests/design-tokens/` (pure, run by `scripts/test-swift-pure.sh`). It parses `design/brand/tokens.json` and asserts that each `Color.es*` hex in `DesignSystem.swift` matches its tokens.json value. A simple regex over the Swift file is enough. It must fail once against a deliberately wrong hex before it passes.

### A2. Components: `Components.swift` (new)
Build these from spec §5, each with a `#Preview` in light and dark: `ESButtonStyle` (primary/secondary/ghost/danger × regular/large), `ESGroup` + `ESGroupRow`, `SectionHeader`, `LabelText`, `Notice`, `Pill`, `ChipBar`, `StatStrip`, `StatusGlyph`, `StatusTimeline`, `AddressText` (grouped by 4, with optional head and tail highlight), `EmptyState` and `EngravedWaves` (a Canvas of the plate's wave lines).
- Keep `Card` as a thin wrapper over `ESGroup` until A9.

### A3. App chrome: `SimpleDashboard.swift` (`shell`, `body`, `SidebarStatus`)
- `body`: `.tint(.esAccent)`, and the window background becomes `Color.esBg` (`.containerBackground(Color.esBg, for: .window)` on macOS 15+, otherwise `.background`).
- macOS sidebar: `List` with `Section("WALLET")`, `Section("THIS MAC")` and `Section("PROTECTION")` using `LabelText` headers. The sidebar background is `esSunken` (`.scrollContentBackground(.hidden)`). The `Page` enum gains `.node` and `.agents`. **Do not remove `.network` yet.** B1 does that, so Lane B can merge independently.
- `SidebarStatus`: restyle only (switch row plus live-dot row, spec mockup 01).
- Toolbar: add ⌘K search (a `.searchable` placeholder that filters Activity; a no-op on Home in v1) and a QR button that opens Receive.
- Apply `.lineBreakStrategy(.hangulWordPriority)` at the root.

### A4. Home: `SimpleDashboard.swift HomePage`
- Replace `accountButton`, `balanceText`, `VerifiedBadge` and the `RoundAction` row with `BalancePlate` (account line, amount, `VerifiedLine`) and `ActionRow`. Keep every `VerifiedBadge` branch (key error, paused, half-dead, outdated, verifying), rendered as `VerifiedLine` states.
- Wide layout (`!narrow` and width ≥ 900): `HStack(alignment: .top, spacing: 28)` with the main column and a rail of 268 holding `AssetsMini`, `RewardsMini` (node on) and the top `Notice`. Narrow and iOS: a single column with `NodeLine`.
- Remove `HomeEarnings` and `BalanceCard` from Home. `HomeEarnings` stays in `Earnings.swift` until B2. Leave `BalanceCard` in place, since only Home used it, and delete it in A9.
- Banner priority (spec §6): one `HomeBanner` view picks the most severe of `IncomingRecoveryAlert`, `keyError`, `scheduledUpgrades`, offline/paused and `HealthBanner`.
- Copy the coin from `codex/brand-art` once it merges: `design/brand/dbln-coin-1024.png` → `Assets.xcassets/DoubloonCoin.imageset` (1x/2x/3x at 64/128/192 pt, plus a 380 px variant for the plate).

### A5. Activity: `SimpleDashboard.swift` (`ActivityPage`, `ActivityRow`, `ActivityList`) → move to `ActivityViews.swift` (new)
- `ActivityRow` v2 (spec §5): `TxIcon` with a corner badge, a sentence title, a sub line from `TxStatusText` (l10n lane) and an inline B5 button. Its label is `String(localized: "Resend at today's price")`, translated "새 가격으로 다시 보내기". It calls the existing `model.beginResend(item)`, and the amount is struck through when dropped.
- Group by calendar day, and collapse node rewards to one row per day with `RewardDays.group` (already exists).
- `ChipBar` filters on the `ActivityItem.kind` / `isNodeReward` that already exist.
- macOS: `.inspector(isPresented:)` (macOS 14) shows `ActivityInspector` for the selected row. iOS: a `NavigationLink` to the same view.
- Move `LinkedWalletsCard` behind a toolbar `Menu("Show history for…")`, and the breakdown behind a toolbar "Summary" popover.
- **New model state: none.** The inspector's "price then → now" needs the dropped item's original fee cap. If `ActivityItem.Resend` does not carry it, show only "now" and add a TODO. Do not add it to the model in this lane.

### A6. Send: `SimpleDashboard.swift SendSheet` (+ `EnclaveKey.swift`)
- Form → `AmountField` (centred, 56) + asset chip + `RecipientChip`. The recipient's history note uses `model.sentAddresses` (a set), so the copy is "You sent here before" / "보낸 적 있는 주소". A count would need model work, and v1 does not need it.
- Summary group ("Network fee", "<name> gets").
- Look-alike block: `AddressText` with the head and tail highlighted, a `Notice(.warn)`, and the existing `ackPoison` toggle restyled as a checkbox.
- A **sent step** replaces `dismiss()` after a successful native send. It shows a check and a `StatusTimeline` bound to the new item's state, with "View in Activity" and "Done". The sheet stays dismissible.
- Steps morph inside one sheet (`matchedGeometryEffect` on the amount), with height animated over 0.24 s.
- **Touch ID reason text:** in `EnclaveKey.swift` (sign), pass an `LAContext` with `localizedReason` set to the localised "send 2.0 DBLN to Mina (0x41a7…9c03)" through `kSecUseAuthenticationContext`. The caller (`WalletModel.send` / `sendTokenTx` / `approveCall`) supplies the sentence. Today the system prompt shows a generic string.
- The token `confirmCard` keeps its logic and only gets restyled. Leave `CallSheet` and `ConnectSheet` restyled only (`ESButtonStyle`, `Notice`).

### A7. Receive: `SimpleDashboard.swift ReceiveSheet`, `QRCode`
- `BrandedQR` uses CoreImage with `correctionLevel = "H"`. Draw the modules as rounded dots in `esSea`, the finders as rounded squares with `esGold` centres, and the coin in the centre at 22% of the width on a parchment knock-out.
- `AddressText` over 2 lines. The segmented "Address / Ask for an amount" adds amount and memo fields and encodes the existing `paymentRequest` link format, with the URL scheme unchanged.
- Buttons: Share (`ShareLink`) and Copy (primary, showing "Copied ✓" for 1.5 s).

### A8. Token icons: `TokenIconView.swift`, `Assets.xcassets`
- Replace `DoubloonArt` with `DawnMark`, a SwiftUI `Path` port of `design/brand/dawn-flat.svg` (viewBox 32, 3 paths and 3 rays, `#E8BF59` disc and `#0D2135` marks), used below 48 pt.
- `OfficialTokenArt`: use the bundled images `TokenWAETH`, `TokenNEB`, `TokenORB` and `TokenCMT` from `design/brand/tokens/*-256.png`. Keep the drawn fallback for list entries without art.
- `GeneratedGlyphArt`: unchanged.
- `apps/wallet/Tests/token-icon` must pass unchanged, because classification is untouched.

### A9. Cleanup (end of Lane A)
- Remove `RoundAction`, `Tile`, `BalanceCard`, `Font.display` and `Font.heroNumber*`, and the `Card` alias if it is unused.
- **Gate:** `rg -n "Color\.aether|\.aether\b|\.pink|design: \.rounded" apps/wallet/Sources` returns only the deprecated alias (removed in B6).

---

## Lane B: IA, Node, Security, Agents, Onboarding, menu bar

### B1. IA: `SimpleDashboard.swift` (`Page`, `pageView`, iOS `shell`)
- macOS pages: home, activity, explore, node, security, agents. Delete `.network` and its `NetworkPage` struct, and move its content as follows. The tiles and `NetworkCard` go to `DeveloperSection` in `SettingsView.swift` (l10n lane file). `UpgradeNoticeCard` goes to `HomeBanner`. `NodeEarningsCard`, `NodeCard` and `UpdateCard` go to B2 and the Settings ▸ General tab.
- iOS tabs: home, activity, explore, settings. `SettingsPage` (new, `Form`, inset grouped) holds the protection status, Recovery, Connected sites, Updates, Terms and rules, and `DeveloperModeCard` at the bottom.
- Keep the DEBUG `-previewPage` hook, with the new names added (`node`, `agents`, `settings`).

### B2. Node & rewards: `Earnings.swift`, `SimpleDashboard.swift` (`NodeCard`, `VotingNodeRow`)
- `NodePage` (new, in `Earnings.swift` or `NodePage.swift`) has three parts: a header card (switch, live dot and `StatStrip`), a rewards card (`esAmountL` figure in `esGoldText` with `HourBars` from `EarningsModel`'s hourly sums), and a seat group (`VotingNodeRow` restyled, progress bar, and the rules disclosure quoting `VotingRules.mainnetRewardsRule` verbatim).
- `RewardStandingCard` and `EarningsExportCard` become rows in the toolbar Export menu or the seat group.
- Delete `EarningsHero`, `AuroraBackground`, `ConfettiBurst`, `FloatingReward` and `EarnInk`. Keep the `RewardCelebration` model and drive the one-time gold pulse from it. Check `apps/wallet/Tests/earnings` still passes. If it references the deleted views, it is testing the wrong layer, so move the assertion to the model.
- `HomeEarnings` → `NodeLine` / `RewardsMini` (A4 placeholders get wired here).

### B3. Security: `SimpleDashboard.swift` (`SecurityPage`, `PaperKeyPanel`, `RecoveryPanel`)
- A status hero and a 3-row status list. The "way back" row is ✓ when `model` has a recovery device or registered words (an existing flag); otherwise it is ⚠ with [Set up].
- The Recovery group rows open sheets that hold today's `RecoveryPanel` (as numbered steps) and `PaperKeyPanel`.
- `ConnectedSitesSection` (ExploreTab.swift) is restyled into an `ESGroup`.
- Sidebar amber dot when recovery is not set (the same flag).

### B4. AI agents: `AgentWalletPanel.swift` → `AgentsPage`
- Same data and helper commands. Only the layout changes, to spec §4.7: a request card (gold border) per `payee-requests.json` row, a session group, a history group and the empty state.
- "Stop all agents" moves to the toolbar (danger). The session limits shown come from the helper's display files; if the limits are not in them yet, show "—" and leave a TODO for the agent lane.
- Sidebar amber dot while requests exist.

### B5. Onboarding: `Onboarding.swift`, `ContentView.swift`
- `FirstRunFlow` (new) has three steps: Welcome (navy, coin, serif headline, gold capsule), Before you start (`TermsSheet` content cut to 3 lines and a link; the `Terms.version` gating in `ContentView.swift:34` is unchanged), and Ready (address card, recovery CTA). The node switch defaults to **off** (spec Q1).
- `VotingNodeInvite`: three consent rows, with [Not now] and [Touch ID Join].

### B6. Menu bar and final cleanup: `ProverMenu.swift`, `SimpleDashboard.swift`
- `MenuBarPanel`: a mini plate (no coin), one node line, the switch and "Open EastSea". Prover details move under Developer.
- Remove the deprecated `Color.aether` alias. **Gate:** the `rg` above returns nothing.

---

## Verification

1. **Screens harness** (`scripts/wallet-screens.sh`, `WALLET_SCREENS` target from the l10n lane). Renders every screen and sheet off screen with `DesignPreview` data, in ko and en, light and dark, with no node, keychain or data folder.
   - Add renderer entries for every new surface: `home`, `home-narrow`, `activity`, `activity-inspector`, `send-compose`, `send-lookalike`, `send-sent`, `receive`, `receive-amount`, `node`, `security`, `agents`, `agents-empty`, `onboarding-1/2/3`, `menubar`.
   - Add `DesignPreview` variants for the states in spec §6. `empty`, `verifying` and `paused` exist. Add `offline`, `dropped` (one `ActivityItem` with `state: .failed` and a `resend`), `pendingPrice` (reason `state_price_above_cap`), `recoveryUnset` and `agentRequest` (sample `payee-requests.json` rows written to a temp dir).
   - Run `scripts/wallet-screens.sh` for ko and en in light and dark, then `-only home` and similar while iterating. Put each PNG next to its mockup (`docs/design/wallet-redesign/mockups/*.png`) and review them by eye. The match should be close, not pixel-exact: the plate, colours, hierarchy and copy length must match.
2. **Pure tests:** `scripts/test-swift-pure.sh` covers the existing `token-icon`, `resend`, `tx-status-text` and `fee-confirm` tests, plus the new `design-tokens` test (A1).
3. **L10n:** `scripts/check-wallet-l10n.sh`. No hard-coded user-facing strings, and every new key has ko and en.
4. **Gates (grep):** after A9 and B6, there are no `Color.aether`, `.pink`, `design: .rounded`, `AuroraBackground` or `ConfettiBurst` in `apps/wallet/Sources`. No consumer-surface string contains "gas", "nonce", "mempool", "validator" or "state root" (spec P5); check with `rg` over `Localizable.xcstrings`, excluding Developer keys.
5. **Build:** `scripts/build-wallet.sh` for macOS, plus an iOS simulator build. Neither may be launched against real data.
6. **Contrast:** the `design-tokens` test also asserts that each text/background pair in spec §7.2 is ≥ 4.5:1, computed from the hexes.
7. **Accessibility pass** in the harness at the largest Dynamic Type (iOS) and with Reduce Motion on. Rows must wrap rather than truncate amounts, and no animation may run.

## Rough size
- Lane A: about 2–3 days (A6 Send is the biggest piece, about 1 day).
- Lane B: about 2 days.
- Both lanes touch `SimpleDashboard.swift`. Lane B should rebase onto A after A3, so the `Page` enum change lands once.
