# Islands v3 review artifacts

The founder selected Islands for 0.7.4 on 2026-10-10. The normative spec and sixteen
shared goldens are in [46-account-icon.md](../46-account-icon.md).

- [Round 3 before/after: the reported orange layout at 16/32/64 px](round3/before-after.png)
- [Selected Islands, with Waves and Navigation retained as static studies](round3/directions.png)
- [Current palette and coastline atlas](round3/atlas.png)
- [Original round 2 directions](round2/directions.png), [atlas](round2/atlas.png) and [comparison](round2/before-after.png)

Only Islands ships. Waves and Navigation remain static geometry in the review
script and have no recognition or parity measurements.

[Face-layout report](round3/pareidolia.json): **0 / 262,144** geometry outcomes flag
the two-small-elements-above-a-larger-form pattern at 32/64 px. A stronger check
finds **0** pairs of secondary centroids above the main centroid, regardless of
area or alignment. Both checks flag the fixed real round 2 orange SVG. Opposite
corners and unequal forms prevent the original paired-eye composition.

[Glance measurement](measurement.json): **36/10,000 = 0.36%** requested weak pairs,
**68/10,000 = 0.68%** conservatively merged coastlines, minimum midpoint ΔE2000
**15.9954**. All three meet the round 2 baseline. V3 keeps the v2 seed domain and
small-size features; it changes larger-size satellite geometry and SVG namespace.

[Resource evidence](remote-resources.json) records poc-m3 execution at nice 15,
4 GiB available-RAM and 30 GiB free-disk floors, a 12 GiB owned-RSS cap, and removal
of the exact owned stage and every owned process. [Earlier palette search](palette-resources.json)
remains the provenance for the unchanged sixteen colors.

Native and browser comparisons:

- [SwiftUI light](swiftui/swiftui-review-light.png) and [dark](swiftui/swiftui-review-dark.png)
- [Browser light](browser/browser-review-light.png) and [dark](browser/browser-review-dark.png)
- [Native source, fixture and 106-image hashes](swiftui/render-info.json)
- [96 native/browser raster comparisons](render-comparison.json)
- [64 small-size baseline comparisons](round3/small-size-stability.json)

The raster comparison averages **1.4007/255** per RGB channel; its worst image is
**2.8372/255**, below the 5/255 limit. At 16 px the browser and native dark images
are exact against round 2; fifteen native light captures vary by at most 1/255.
Canonical small-size geometry and colors are unchanged.

`swiftui/{light,dark}/vector-NN-SIZE.png` and matching `browser/` paths contain all
sixteen addresses at 16, 32 and 64 px. Native renders include neutral placeholders
and the 16 pt MenuBar bridge. `svg/01-64.svg` through `svg/16-64.svg` match the frozen
v3 canonical SVG hashes.

The eight [product surfaces](surfaces/) capture real extension/explorer DOM and
CSS through offline fixtures: extension Home and approval, explorer desktop and
mobile, light and dark. [Checks](surfaces/checks.json) pass full-address, decorative
accessibility, actual icon size, overflow, console/page error and read-only fixture
checks. Approval icons remain 24 px and use the simplified drawing; Home uses 32 px
and account headers 64 px.

[Independent artifact verdict](round3/visual-verdict.json): **92/100**, pass.
[Native verdict](swiftui/visual-verdict.json): **95/100**, pass. These are technical
reviews of the reported composition and artifact quality; human recognition and
other pareidolia remain unmeasured.

Development-Mac reproduction:

```sh
scripts/test-swift-pure.sh account-icon
scripts/render-account-icons.sh
node scripts/render-account-icon-review.mjs
python3 scripts/compare-account-icon-renders.py
python3 scripts/account-icon-vectors.py --check
node scripts/sync-account-icons.mjs --check
node --test scripts/account-icon-layout.test.mjs scripts/account-icon-color.test.mjs
```

Swift compiles only the pure spec or offline icon snapshots behind the Mac's
compile semaphore. Chromium uses an existing installation, keeps its profile in
`./tmp`, and closes it after rendering. No EastSea app, live node or signing input
is used. No wallet, release or Jolt guest build belongs to this lane.

Scale measurements run only in an exact owned directory under
`~/eastsea-lab/account-icon/tmp/` on poc-m3. Stage the development-built icon-only
binary, canonical module/mirrors, and measurement/color/layout scripts. Under the
same resource guard as the round 2 run, with nice 15 and a 1 GiB Node heap:

```sh
/opt/homebrew/bin/node --max-old-space-size=1024 scripts/measure-account-icons.mjs 10000 tmp/measurement.json "$PWD/bin/account-icon-snapshots"
/opt/homebrew/bin/node --max-old-space-size=1024 scripts/account-icon-layout.mjs --exhaustive tmp/pareidolia.json
```

The first command renders all 1,024 actual 16 px identities and checks the same
20,000-address stream as round 2. The second exhausts geometry outcomes, verifies
32/64 emitted-path equivalence and tests the old positive controls. Retrieve the
reports, reap owned processes, and remove only the marked staging directory.
