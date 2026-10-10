#!/usr/bin/env python3
"""Compare bounded native/browser account-icon screenshots, not PNG byte goldens."""
import json
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

ROOT = Path(__file__).resolve().parent.parent
ARTIFACTS = ROOT / 'docs/design/46-account-icon'
fixture = json.loads((ROOT / 'crates/client/tests/account-icon-vectors.json').read_text())
comparisons = []
for theme in ['light', 'dark']:
    for index in range(len(fixture['vectors'])):
        for size in [16, 32, 64]:
            filename = f'vector-{index:02d}-{size}.png'
            with Image.open(ARTIFACTS / 'swiftui' / theme / filename) as native, Image.open(ARTIFACTS / 'browser' / theme / filename) as browser:
                if native.size != (size, size) or browser.size != (size, size):
                    raise SystemExit(f'Unexpected logical pixel size: {theme}/{filename}')
                difference = ImageChops.difference(native.convert('RGB'), browser.convert('RGB'))
                statistics = ImageStat.Stat(difference)
                comparisons.append(dict(theme=theme, vector=index, size=size,
                    meanAbsoluteChannelDifference=sum(statistics.mean) / 3,
                    maximumChannelDifference=max(high for low, high in statistics.extrema)))
maximum = max(item['meanAbsoluteChannelDifference'] for item in comparisons)
report = dict(version=fixture['version'], comparisons=comparisons, comparisonCount=len(comparisons),
    meanAbsoluteChannelDifference=sum(item['meanAbsoluteChannelDifference'] for item in comparisons) / len(comparisons),
    worstMeanAbsoluteChannelDifference=maximum, thresholdMeanPerImage=5, passes=maximum < 5,
    caveat='PNG rasterizers differ at antialiased edges; canonical per-size SVG hashes are the cross-language goldens.')
(ARTIFACTS / 'render-comparison.json').write_text(json.dumps(report, indent=2) + '\n')
print(f"{len(comparisons)} pairs: mean channel difference {report['meanAbsoluteChannelDifference']:.4f}/255, worst image {maximum:.4f}/255")
if not report['passes']:
    raise SystemExit('Native/browser raster difference exceeds the review threshold')
