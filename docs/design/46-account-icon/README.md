# Archipelago v2 review artifacts

The normative spec and sixteen shared goldens are in [46-account-icon.md](../46-account-icon.md).

Start with the founder review sheets:

- [Before / after, actual 16 px and the original approval pair](round2/before-after.png)
- [Islands, Waves and Navigation directions](round2/directions.png)
- [Sixteen palette families and silhouette classes](round2/atlas.png)

Islands is fully implemented. Waves and Navigation are static alternatives in
the review script; neither alternative ships or has recognition/parity measurements.

Native and browser comparisons:

- [SwiftUI light, 16/32/64 px](swiftui/swiftui-review-light.png)
- [SwiftUI dark, 16/32/64 px](swiftui/swiftui-review-dark.png)
- [Browser light](browser/browser-review-light.png)
- [Browser dark](browser/browser-review-dark.png)

`swiftui/{light,dark}/vector-NN-SIZE.png` and matching `browser/` paths contain
all sixteen addresses at 16, 32 and 64 px. SwiftUI additionally includes unseeded
placeholders and the 16 pt menu-bar image bridge. `svg/01-64.svg` through
`svg/16-64.svg` are the frozen canonical 64 px SVG exports.

The eight [product surfaces](surfaces/) capture real extension/explorer DOM and
CSS with bounded offline mocks: extension Home and approval, explorer desktop
and mobile, light and dark. [Checks](surfaces/checks.json) verify full addresses,
decorative accessibility, actual resolved icon sizes, no overflow/errors and no
external or signing operations. Approval icons are actually 24 px, so they use
the same simplified drawing as 16 px. Home uses 32 px and account headers 64 px.

[Glance measurement](measurement.json): **36/10,000 = 0.36%** requested weak pairs;
**68/10,000 = 0.68%** with similar coastlines conservatively merged. Minimum
background-midpoint ΔE2000 is **15.9954**. [Resource report](remote-resources.json)
confirms guarded poc-m3 execution and cleanup; [palette-search resources](palette-resources.json)
record the bounded optimization runs. [Render comparison](render-comparison.json)
covers 96 native/browser pairs (mean RGB channel difference 1.4200/255).
[Artifact verdict](round2/visual-verdict.json) is 92/100 and
[surface verdict](surfaces/visual-verdict.json) is 93/100. These are technical visual
QA results, not founder approval or human anti-phishing effectiveness.

To reproduce on the development Mac:

```sh
scripts/render-account-icons.sh
node scripts/render-account-icon-review.mjs
python3 scripts/compare-account-icon-renders.py
python3 scripts/account-icon-vectors.py --check
node scripts/sync-account-icons.mjs --check
node --test scripts/account-icon-color.test.mjs
```

Swift rendering compiles only the icon spec/view and offline snapshot executable
behind the compile semaphore; it never builds or launches EastSea. Browser
rendering uses an existing Playwright/Chrome installation (override `PLAYWRIGHT_MODULE`
and `CHROME_BINARY` if needed), keeps its profile in `./tmp` and cleans it afterward.
The PNGs have native logical pixels; their bytes can vary by OS rasterizer.

Scale measurement runs only on guarded poc-m3, staged in an exact owned subdirectory
under `~/eastsea-lab/account-icon`. Copy the development-built icon-only executable,
canonical JS and color/measurement scripts; do not compile a wallet or guest there.
From the guarded root, with nice 15, available RAM >=4 GiB, free disk >=30 GiB,
owned RSS <=12 GiB and a 1 GiB Node heap limit:

```sh
/opt/homebrew/bin/node --max-old-space-size=1024 scripts/measure-account-icons.mjs 10000 tmp/measurement.json "$PWD/bin/account-icon-snapshots"
```

The script renders at most 1,024 unique 16 px identities and counts 10,000 independent
pairs. Its `--dominants` subprocess exports only RGBA pixels. Retrieve the reports,
stop all owned processes and remove the exact owned staging directory afterward.
Runtime keys, a live node, the installed EastSea app and the testnet are never inputs.
