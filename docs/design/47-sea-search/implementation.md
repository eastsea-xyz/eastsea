# Explore Home: sea://search

Target: 0.7.4. Home and new tabs use a native search page at `sea://search`.

Implementation plan:

1. Protect the existing browser, app-search, name-resolution and globe policies with their pure Swift suites. Baseline: all five suites pass.
2. Add a pure classifier and explicit web-choice policy. Reuse the node's real `aether_search` index and pinned, snapshot-consistent name registry reads. Keep bundled tools separate from registry records; delete the placeholder inventory.
3. Route Home, new tabs, restored Home URLs and browser history to the native page. One centered query field shows actionable names, apps, chain lookups, URLs and an explicit web option. Enter opens the first native result; web search requires its own button.
4. Reuse the bundled live globe below the field, preserve motion/privacy controls, and provide all five languages, theme support and accessibility labels. Settings selects DuckDuckGo, Google, Bing, Naver or Brave.
5. Run bounded pure Swift and explorer/extension parity tests, localization checks and the WALLET_SCREENS renderer. Save the rendered evidence here and commit using Lore trailers.

Build constraints: compile semaphore before every build; no guest, release or ad-hoc build; no EastSea launch; no push; forbidden Rust crates untouched. Temporary files stay in this worktree's `tmp/`. A large build requires at least 15 GB internal free disk and 12 GB available RAM; stop for disk below 12 GB or swap growth above 3 GB.

Implementation and bounded verification are finished. See [README.md](README.md) for the 90 screenshots, test evidence, changed files and release gates. Shipping remains gated on trusted registry pins, matching FFI artifacts and the inherited production app-entry isolation failure.
