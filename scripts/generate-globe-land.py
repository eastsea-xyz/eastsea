#!/usr/bin/env python3
"""Generate bundled sphere artwork from Natural Earth 110m land, with no packages.

Place the pinned public-domain source archive in the workspace tmp/ first:
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
SOURCE_SHA256 = "1926c621afd6ac67c3f36639bb1236134a48d82226dc675d3e3df53d02d2a3de"
SAMPLES = 36000
MAX_COASTLINE_ARC = math.radians(2)


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


def sphere_point(lon, lat):
    lon, lat = math.radians(lon), math.radians(lat)
    radius = math.cos(lat)
    return radius * math.sin(lon), math.sin(lat), radius * math.cos(lon)


def coastline(rings):
    """Closed-ring coastline segments, with endpoints on the unit sphere."""
    values = []
    for _, ring in rings:
        for start, end in zip(ring, ring[1:] + ring[:1]):
            # Natural Earth splits polygons at the dateline. Those meridian
            # closures (including Antarctica's pole edges) are not coastline.
            if all(abs(abs(lon) - 180) < 1e-6 for lon, _ in (start, end)):
                continue
            a, b = sphere_point(*start), sphere_point(*end)
            angle = math.acos(max(-1, min(1, sum(x * y for x, y in zip(a, b)))))
            if angle < 1e-10:
                continue
            steps = math.ceil(angle / MAX_COASTLINE_ARC)
            previous = a
            for step in range(1, steps + 1):
                if step == steps:
                    point = b
                else:
                    fraction = step / steps
                    weight_a = math.sin((1 - fraction) * angle) / math.sin(angle)
                    weight_b = math.sin(fraction * angle) / math.sin(angle)
                    point = tuple(weight_a * x + weight_b * y for x, y in zip(a, b))
                values.extend(previous)
                values.extend(point)
                previous = point
    return values


def js_points(name, values):
    rows = [",".join(f"{n:.5f}" for n in values[i:i + 18]) for i in range(0, len(values), 18)]
    return f"export const {name} = [\n" + ",\n".join(rows) + "\n];\n"


def main():
    archive = Path(sys.argv[1])
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    if digest != SOURCE_SHA256:
        raise ValueError("Natural Earth archive does not match the pinned SHA-256")
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
    coast = coastline(rings)
    OUT.write_text(
        "// Generated artwork: Natural Earth 110m land (public domain).\n"
        "// Source: https://www.naturalearthdata.com/downloads/110m-physical-vectors/110m-land/\n"
        f"// Archive SHA-256: {digest}\n"
        "// Regenerate with scripts/generate-globe-land.py; no node location data.\n"
        + js_points("LAND_POINTS", values)
        + "// Consecutive segment endpoints; each segment contains two unit xyz points.\n"
        + js_points("COASTLINE_POINTS", coast)
    )
    print(f"Bundled {len(values) // 3} land dots and {len(coast) // 6} coastline segments "
          f"({OUT.stat().st_size} bytes); source SHA-256 {digest}")


if __name__ == "__main__":
    main()
