# Archipelago v1 review artifacts

The normative algorithm, survey and 16 published vectors are in
[46-account-icon.md](../46-account-icon.md).

Start with the actual SwiftUI comparison sheets:

- [Light, native 16/32/64 px](swiftui/swiftui-review-light.png)
- [Dark, native 16/32/64 px](swiftui/swiftui-review-dark.png)
- [Browser reference, light](browser/browser-review-light.png)
- [Browser reference, dark](browser/browser-review-dark.png)

`swiftui/{light,dark}/vector-NN-SIZE.png` and the matching `browser/` paths
contain all 16 individual icons at each requested size. `swiftui/` also includes
neutral placeholders and the 16 px menu-bar NSImage bridge. `svg/01-64.svg`
through `svg/16-64.svg` are the canonical SVGs whose hashes are frozen in the
shared fixture.

The eight [surface screenshots](surfaces/) use the real extension/explorer
DOM and CSS with offline mocked data. They show Home and transaction approval
at 360 px, plus mobile/desktop explorer address pages in both themes.
[Surface checks](surfaces/checks.json) confirm full identity text, decorative
icons, no horizontal overflow and no browser errors. No live node, wallet key
or signing request was used.

[Measurement](measurement.json) covers 100,000 addresses and all unordered
pairs. [Resource evidence](remote-resources.json) confirms the guarded poc-m3
measurement and process cleanup. [Render comparison](render-comparison.json)
records 96 SwiftUI/Chromium pairs: mean channel difference 1.0617 out of 255,
mostly edge antialiasing; geometry, palette and orientation agree.
[Visual verdict](visual-verdict.json): 98/100, pass. Exact equality and contrast
do not establish a human-recognition or anti-phishing success rate.

To reproduce the required Swift snapshots on the development Mac:

```sh
scripts/render-account-icons.sh
```

This compiles only the icon spec/view and static renderer behind the compile
gate. It never builds or launches EastSea. Temporary files stay under `./tmp`.
The original browser-reference capture used existing Chrome at
DPR 1; [render metadata](browser/render-info.json) records the actual version.
PNG bytes can vary by OS/browser rasterizer; canonical SVG hashes do not.

The measurement implementation is `scripts/measure-account-icons.mjs`.
Run it on guarded poc-m3 under `~/eastsea-lab/account-icon`, with nice 15,
12 GiB owned RSS cap, available RAM ≥4 GiB and disk ≥30 GiB; remove owned
processes/staging afterward. `node scripts/measure-account-icons.mjs 100000
OUTPUT.json` writes the report. Do not run scale measurements on the development
Mac. The v1 oracle and mirror checker are safe read-only local checks:

```sh
python3 scripts/account-icon-vectors.py --check
node scripts/sync-account-icons.mjs --check
```
