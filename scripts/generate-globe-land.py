#!/usr/bin/env python3
"""Generate bundled sphere artwork from Natural Earth 110m land, with no packages.

Download the public-domain source archive into the workspace tmp/ first:
  curl -fsSL https://naciscdn.org/naturalearth/110m/physical/ne_110m_land.zip -o tmp/ne_110m_land.zip
  python3 scripts/generate-globe-land.py tmp/ne_110m_land.zip
The browser never downloads this archive or receives node locations.
"""

import hashlib
import math
from pathlib import Path
import struct
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "apps/explorer/live-globe/land.js"
SAMPLES = 9000


def polygons(raw):
    """Read Polygon records from a shapefile; each part is a closed ring."""
    offset = 100
    while offset < len(raw):
        _, length = struct.unpack_from(">ii", raw, offset)
        start = offset + 8
        offset = start + length * 2
        kind = struct.unpack_from("<i", raw, start)[0]
        if kind != 5:
            raise ValueError("Expected Natural Earth Polygon records")
        parts, points = struct.unpack_from("<ii", raw, start + 36)
        indices = list(struct.unpack_from(f"<{parts}i", raw, start + 44)) + [points]
        xy = list(struct.iter_unpack("<dd", raw[start + 44 + parts * 4:offset]))
        for a, b in zip(indices, indices[1:]):
            ring = xy[a:b]
            xs, ys = zip(*ring)
            yield (min(xs), min(ys), max(xs), max(ys)), ring


def contains(x, y, ring):
    inside = False
    for (ax, ay), (bx, by) in zip(ring, ring[1:] + ring[:1]):
        if (ay > y) != (by > y) and x < (bx - ax) * (y - ay) / (by - ay) + ax:
            inside = not inside
    return inside


def main():
    archive = Path(sys.argv[1])
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    with zipfile.ZipFile(archive) as source:
        rings = list(polygons(source.read("ne_110m_land.shp")))
    values = []
    golden_angle = math.pi * (3 - math.sqrt(5))
    for i in range(SAMPLES):
        y = 1 - 2 * (i + .5) / SAMPLES
        lat = math.degrees(math.asin(y))
        lon = math.degrees((i * golden_angle) % (2 * math.pi)) - 180
        # Even/odd rings preserve holes. Longitude/latitude exist only here at
        # artwork generation time, never in the presence API or model.
        inside = False
        for (xmin, ymin, xmax, ymax), ring in rings:
            if xmin <= lon <= xmax and ymin <= lat <= ymax and contains(lon, lat, ring):
                inside = not inside
        if inside:
            r = math.sqrt(1 - y * y)
            angle = math.radians(lon)
            values.extend((r * math.sin(angle), y, r * math.cos(angle)))
    rows = [",".join(f"{n:.5f}" for n in values[i:i + 18]) for i in range(0, len(values), 18)]
    OUT.write_text(
        "// Generated artwork: Natural Earth 110m land (public domain).\n"
        "// Source: https://www.naturalearthdata.com/downloads/110m-physical-vectors/110m-land/\n"
        f"// Archive SHA-256: {digest}\n"
        "// Regenerate with scripts/generate-globe-land.py; no node location data.\n"
        "export const LAND_POINTS = [\n" + ",\n".join(rows) + "\n];\n"
    )
    print(f"Bundled {len(values) // 3} land dots ({OUT.stat().st_size} bytes); source SHA-256 {digest}")


if __name__ == "__main__":
    main()
