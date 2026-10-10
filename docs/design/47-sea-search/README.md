# sea://search — lead review

Explore's Home button and new tabs now open a native `sea://search` page. The centered query field searches the real node index, reads exact names from pinned registry contracts, opens chain pages in the bundled Explorer, and offers web search as a separate explicit choice. Settings offers DuckDuckGo (default), Google, Bing, Naver and Brave with query-only URLs.

The placeholder app registry and old start-page inventory are removed. Explorer is labeled **Built-in**. Registry app IDs have their own base32 route; they do not become invented `.sea` names. Verified app opening checks the active release before and after fetching its hashed bundle. Name-only records can open their address in Explorer.

Editing or pressing Enter on plain text never chooses a web engine. Native reads use loopback, with cookies/proxies/cache disabled and redirects refused. Query text is not saved in browser history. Restored tabs retain their saved URL/title until a commit or an explicit Home action. ⌘L focuses the query on Home and the address field on a page.

Round 2 (2026-10-10) changes only globe and result presentation. The search-home globe shows one UN M49 sub-region level. The Mac combines legacy country buckets before applying k≥3, preserves native schema-2 cohorts and their frozen observation window, and leaves broad or unsupported geography unplaced. Public country and continent labels are excluded from Home. The existing host has no marker for this Mac’s opted-in country, so no country marker is added. The regular Network globe retains its previous display policy.

Labels use their actual fractional DOM bounds and at least 8 CSS pixels between them on Home; [render-geometry.json](render-geometry.json) records the checked bounds for all 90 screens, including the 380 pt Home. App results promote a registered name such as `sea://eastsea`, keep the full registry key in a small middle-truncated copyable detail, and navigate through the original registry URL. The registered `search` name displays `sea://search.sea` to keep it distinct from native Home. Regressions cover aggregation before suppression, dense labels, expired accepted cohorts, readable names, reserved names, and unchanged hash/path/query values.

## Screens

There are **90 PNGs and Vision OCR sidecars**: Home, exact name, registry app, address, transaction hash, block, URL, web text, and narrow Home × en/ko/ja/zh-Hans/zh-Hant × light/dark.

Representative views:

- [Home, English/light](sea-search-home-en-light.png)
- [Home, English/dark](sea-search-home-en-dark.png)
- [Home at 380 pt, Korean/dark](sea-search-home-narrow-ko-dark.png)
- [Name, Japanese/light](sea-search-name-ja-light.png)
- [Registry app, Korean/dark](sea-search-app-ko-dark.png)
- [Address, English/light](sea-search-address-en-light.png)
- [Transaction, English/dark](sea-search-tx-en-dark.png)
- [Block, English/light](sea-search-block-en-light.png)
- [URL, English/light](sea-search-url-en-light.png)
- [Explicit web option, Traditional Chinese/dark](sea-search-web-zh-Hant-dark.png)

These images render the **actual SeaSearchPage, design tokens, result labels and LiveGlobeView** in a bounded `DEBUG WALLET_SCREENS` native-view harness. The controller/session/model adapters are isolated screenshot fixtures under `tmp/`, and do not ship. The screenshots cover the page content; the regular WalletScreens target adds the actual browser chrome. Globe snapshots visibly say they are fixtures. This evidence verifies presentation, not a live registry deployment or real app navigation.

The round-2 visual review passes at 94/100 against the redesign's typography, dawn mark, paper/navy palette and layout. The compact globe retains rotation/pause and Reduce Motion behavior. Screenshot verification caught and fixed the initial ES-module readiness race; the native host now waits for its local API before sending the first aggregate/settings. The full renderer also starts outside a held main-queue block, so WebKit can answer its nested run-loop waits.

## Verification

| Check | Result |
| --- | --- |
| Full pure Swift suite on the rebased branch | 76 suites passed |
| Search golden cases | 378 checks; 137 shared extension/explorer name/action cases |
| Registry name/app lookup | 85 checks, including code pins, release validity, stable snapshots and cancellation |
| Explorer JavaScript | 381 tests passed |
| Full extension suite, including packaged WASM | 329 tests passed |
| Localization lint/catalog | Passed; 1,283 keys in every language |
| Screenshot matrix / Vision text | Passed: 90 images, 2,208 primary/multilingual OCR lines; required locale/appearance pairs complete |
| Native source type checking | 127 renderer inputs passed; including the unchanged app entry reproduces the `AppDelegate.isSafeMode` isolation error |
| Xcode project / diff | Project plist and diff whitespace checks passed |

Compiles ran behind the lane semaphore. Swift compiles/type checks used a 2 GiB aggregate RSS stop limit; the final view compile peaked at 302 MiB and the renderer-source typecheck at 285 MiB. No Rust/guest, ad-hoc wallet or release build ran, and EastSea was never launched. All temporary artifacts are in this worktree's `tmp/`; no remote machines or real node processes were used. Native-identity test fixtures cleaned up their own processes. The full extension suite loads an existing real WASM package copied read-only from the sea-names lane into `tmp/`, through a temporary module resolver; no WASM build or source substitution was used.

## Release gates

**Not yet cleared for 0.7.4 shipping.** The lead must supply trusted deployment pins and run the normal integration wallet/renderer gates. Defer to 0.7.5 if those cannot be completed safely.

1. `apps/wallet/Resources/name-sources.json` currently contains an empty chain map. Exact-name and direct registry-app opening fail closed until the network's real contract addresses and runtime SHA-256 pins are published. The node's search index likewise needs its real protocol sources configured. No deployment or guessed pin was introduced here.
2. This worktree has no built FFI library. The cached integrate-074/chain-updates/lead artifacts lack 14 exports required by the current generated bindings (seven dApp function/checksum pairs). A bounded normal WalletScreens link confirmed the mismatch. The lane did not rebuild the FFI/guest or bypass checksums. Full renderer controller assertions (Home/new/restored tabs, explicit web navigation and malformed app keys), whole-browser screenshots, the final wallet build and live pinned registry navigation remain integration gates.
3. The production Debug entry point fails type checking at `AetherWalletApp.swift:209`: the lazy tracker initializer reads the MainActor-isolated `LaunchRecovery.isSafeMode` from a nonisolated context. An isolated copy of the unchanged HEAD wallet sources reproduces the same error with the same SDK/flags. This lane leaves node/migration startup code for its integration owner.

The implementation changes are client-only; protected proving-program crates were untouched.

## Changed files

- Query and routes: `SeaSearch.swift`, `SeaAppLink.swift`, `BrowserInput.swift`, `BrowserController.swift`, `BrowserSession.swift`.
- Registry reads/identity: `SeaNameResolver.swift`, `SeaRegistryReader.swift`, `AppBrowserIdentity.swift`.
- Native UI/settings: `SeaSearchPage.swift`, `BrowserViews.swift`, `BrowserData.swift`, `ExploreTab.swift`, `SimpleDashboard.swift`, `SettingsView.swift`, `Localizable.xcstrings`, and the Xcode source list.
- Globe: `LiveGlobeView.swift`, `LiveGlobe/wallet-host.js`, `LiveGlobe/wallet.css`.
- Verification: `WalletScreens.swift`, Swift search/registry/identity/browser suites, explorer globe/locale tests, `test-swift-pure.sh`, `check-wallet-screens-language.py`, and this review directory.
