# EastSea 0.7.4 design system — phase 1

The shared paper/navy/gold system is implemented as generated platform values and reusable effects. The extension and explorer use the site identity, and the menu panel has four language/theme mockups plus a model-independent SwiftUI component. The toolbox redesign is a tested applicable patch for its separate repository.

## Verification

| Gate | Result |
|---|---|
| Platform token drift | PASS — 9 generated outputs, read-only `--check` |
| Generator validation / contrast / drift regressions | PASS — 6 tests |
| Explorer | PASS — 62 tests |
| Extension | PASS — 118 tests, including the packaged WASM verifier |
| Toolbox patch | PASS — 17 app script checks, deterministic regeneration, embedded asset identity, unchanged app JS and external `git apply --check` |
| Browser | PASS — 20 themed full-page/board captures plus 2 scrolled-ledger crops; no console/page errors or document overflow |
| Independent source / visual review | APPROVE — prior appearance, mobile-table and unverified-icon findings resolved; 96/100 |
| Pure Swift target | QUEUED — `scripts/test-swift-pure.sh` waits on the external release reservation before every compiler call |

Native compilation has not started. The gate at `~/.claude/playbooks/aether-team/wait-compile.sh` blocks this lane while `~/.claude/playbooks/aether-team/release-hold` reserves `lead` and `no-fork-spawn`. Its hold file remains present; it is not this lane’s reservation to clear. The queued run writes `tmp/redesign-system/swift-pure.log`. The new design target compiles all new SwiftUI pieces and tests Decimal precision, effect-policy composition, dynamic appearances and native receive-QR decoding when the slot opens.

Extension prerequisites were exercised in `tmp/redesign-system/extension-tests`: 193 Rust source/manifests matched byte-for-byte with the lead worktree before its existing JS/WASM artifacts were reused. No Rust build or product artifact replacement was needed. No EastSea app, installed application, user data directory or validator was launched or modified.

## Design and captures

- [System and effect language](../../../design/brand/SYSTEM.md)
- [Three-reference comparison and five closed deltas](compare.md)
- [Every rendered image path, dimensions and SHA-256](render-manifest.json)
- [Menu light English](mockups/menubar-light-en.png), [dark English](mockups/menubar-dark-en.png), [light Korean](mockups/menubar-light-ko.png), [dark Korean](mockups/menubar-dark-ko.png)
- [Extension light](mockups/extension-light.png), [dark](mockups/extension-dark.png), [unverified light](mockups/extension-unverified-light.png), [unverified dark](mockups/extension-unverified-dark.png)
- [Explorer light](mockups/explorer-light.png), [dark](mockups/explorer-dark.png), [mobile light](mockups/explorer-mobile-light.png), [mobile dark](mockups/explorer-mobile-dark.png)
- [Toolbox light](mockups/toolbox-light.png), [dark](mockups/toolbox-dark.png), [token light](mockups/toolbox-token-light.png), [token dark](mockups/toolbox-token-dark.png)

The installed gstack binary lacked its headless Chromium executable. Captures used the already-installed Playwright with Chrome; no browser or dependency was installed. `scripts/render-design-previews.mjs` reproduces menu/web captures from a repository-root loopback server and toolbox captures from the isolated patched frontend.

## Simplifications and integration

- One generator replaces the independent site generator and packages the same tokens/components for every web surface.
- Violet/pink chrome, gradient placeholder logos and competing metric cards become navy balance surfaces, the dawn mark and quiet rows.
- Four inline popup style attributes become named CSS classes, preserving their behavior under the strict preview CSP.
- Official art remains keyed by chain/address; unknown art now matches the wallet’s deterministic pastel algorithm with a dashed ring and question-mark badge.
- Effects run for finite event changes and settle on appearance, account change, cancellation and accessibility changes. No static timer, polling or repeating animation was added.
- `EastSeaDesign.MenuBarPanel` coexists with the current wallet component. The later wallet lane supplies localized labels, stable account identity, verified event IDs, bindings and actions.

The toolbox frontend is outside this worktree. [toolbox-redesign.patch](toolbox-redesign.patch) applies to `/Volumes/workspace/eastsea-toolbox` and preserves portable single-file deployment by embedding shared styles, fonts and their OFL licenses. The external checkout stays unchanged. Apply with `git -C /Volumes/workspace/eastsea-toolbox apply <absolute-path-to-toolbox-redesign.patch>`, then regenerate with its `scripts/gen-apps.py` when canonical CSS changes.

Remaining limits: the mandatory native compile gate is pending the release reservation; wallet view wiring belongs to the other lane; actual spring/shine rendering and hardware haptics require later native integration. All font embedding licenses are recorded in [brand README](../../../design/brand/README.md).

## Changed files

- `apps/explorer/assets/dawn.svg`
- `apps/explorer/design-components.css`
- `apps/explorer/design-tokens.css`
- `apps/explorer/explorer.css`
- `apps/explorer/fonts/OFL-Geist.txt`
- `apps/explorer/fonts/OFL-Newsreader.txt`
- `apps/explorer/fonts/geist-latin.woff2`
- `apps/explorer/fonts/geist-mono-latin.woff2`
- `apps/explorer/fonts/newsreader-latin.woff2`
- `apps/explorer/index.html`
- `apps/explorer/js/app.js`
- `apps/explorer/js/dom.js`
- `apps/explorer/js/pages.js`
- `apps/extension/test/token-art.test.mjs`
- `apps/extension/ui/assets/CMT-256.png`
- `apps/extension/ui/assets/NEB-256.png`
- `apps/extension/ui/assets/ORB-256.png`
- `apps/extension/ui/assets/WAETH-256.png`
- `apps/extension/ui/assets/dawn-flat.svg`
- `apps/extension/ui/design-components.css`
- `apps/extension/ui/design-tokens.css`
- `apps/extension/ui/fonts/OFL-Geist.txt`
- `apps/extension/ui/fonts/OFL-Newsreader.txt`
- `apps/extension/ui/fonts/geist-latin.woff2`
- `apps/extension/ui/fonts/geist-mono-latin.woff2`
- `apps/extension/ui/fonts/newsreader-latin.woff2`
- `apps/extension/ui/popup.css`
- `apps/extension/ui/popup.html`
- `apps/extension/ui/popup.js`
- `apps/extension/ui/token-art.js`
- `apps/wallet/Sources/Design/BalanceCountUp.swift`
- `apps/wallet/Sources/Design/DesignEffects.swift`
- `apps/wallet/Sources/Design/DesignEventEffect.swift`
- `apps/wallet/Sources/Design/DesignSurface.swift`
- `apps/wallet/Sources/Design/MenuBarPanel.swift`
- `apps/wallet/Sources/Design/NavyPlateDepth.swift`
- `apps/wallet/Sources/Design/NodeStatusPulse.swift`
- `apps/wallet/Sources/Design/PresentationMotion.swift`
- `apps/wallet/Sources/Design/RewardShine.swift`
- `apps/wallet/Sources/Design/SuccessFeedback.swift`
- `apps/wallet/Sources/DesignTokens.swift`
- `apps/wallet/Tests/design-effects/main.swift`
- `design/brand/README.md`
- `design/brand/SYSTEM.md`
- `design/brand/components.css`
- `design/brand/tokens.json`
- `design/generated/toolbox/design-components.css`
- `design/generated/toolbox/design-tokens.css`
- `design/scripts/build-tokens.mjs`
- `docs/design/wallet-redesign/.gitattributes`
- `docs/design/wallet-redesign/compare.md`
- `docs/design/wallet-redesign/mockups/explorer-dark.png`
- `docs/design/wallet-redesign/mockups/explorer-light.png`
- `docs/design/wallet-redesign/mockups/explorer-mobile-dark.png`
- `docs/design/wallet-redesign/mockups/explorer-mobile-ledger-dark.png`
- `docs/design/wallet-redesign/mockups/explorer-mobile-ledger-light.png`
- `docs/design/wallet-redesign/mockups/explorer-mobile-light.png`
- `docs/design/wallet-redesign/mockups/extension-assets-dark.png`
- `docs/design/wallet-redesign/mockups/extension-assets-light.png`
- `docs/design/wallet-redesign/mockups/extension-dark.png`
- `docs/design/wallet-redesign/mockups/extension-light.png`
- `docs/design/wallet-redesign/mockups/extension-unverified-dark.png`
- `docs/design/wallet-redesign/mockups/extension-unverified-light.png`
- `docs/design/wallet-redesign/mockups/html/explorer-preview.html`
- `docs/design/wallet-redesign/mockups/html/explorer-preview.js`
- `docs/design/wallet-redesign/mockups/html/extension-preview.css`
- `docs/design/wallet-redesign/mockups/html/extension-preview.html`
- `docs/design/wallet-redesign/mockups/html/extension-preview.js`
- `docs/design/wallet-redesign/mockups/html/menubar-dark-en.html`
- `docs/design/wallet-redesign/mockups/html/menubar-dark-ko.html`
- `docs/design/wallet-redesign/mockups/html/menubar-light-en.html`
- `docs/design/wallet-redesign/mockups/html/menubar-light-ko.html`
- `docs/design/wallet-redesign/mockups/html/menubar-panel.css`
- `docs/design/wallet-redesign/mockups/html/menubar-panel.js`
- `docs/design/wallet-redesign/mockups/html/menubar-receive-qr.svg`
- `docs/design/wallet-redesign/mockups/menubar-dark-en.png`
- `docs/design/wallet-redesign/mockups/menubar-dark-ko.png`
- `docs/design/wallet-redesign/mockups/menubar-light-en.png`
- `docs/design/wallet-redesign/mockups/menubar-light-ko.png`
- `docs/design/wallet-redesign/mockups/reference-apple-cash-wallet.jpg`
- `docs/design/wallet-redesign/mockups/reference-mercury-accounts.jpg`
- `docs/design/wallet-redesign/mockups/reference-provenance.json`
- `docs/design/wallet-redesign/mockups/reference-things-today.png`
- `docs/design/wallet-redesign/mockups/toolbox-dark.png`
- `docs/design/wallet-redesign/mockups/toolbox-light.png`
- `docs/design/wallet-redesign/mockups/toolbox-mobile-dark.png`
- `docs/design/wallet-redesign/mockups/toolbox-mobile-light.png`
- `docs/design/wallet-redesign/mockups/toolbox-token-dark.png`
- `docs/design/wallet-redesign/mockups/toolbox-token-light.png`
- `docs/design/wallet-redesign/phase1-report.md`
- `docs/design/wallet-redesign/render-manifest.json`
- `docs/design/wallet-redesign/toolbox-redesign.patch`
- `scripts/gen-design-tokens.py`
- `scripts/render-design-previews.mjs`
- `scripts/test-swift-pure.sh`
- `scripts/test_design_tokens.py`
- `scripts/verify.sh`
- `site/design-components.css`
- `site/tokens.css`
