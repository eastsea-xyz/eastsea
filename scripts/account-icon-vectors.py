#!/usr/bin/env python3
"""Independent, stdlib-only oracle for the frozen account-icon v3 vectors."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "crates/client/tests/account-icon-vectors.json"
# Frozen drawing tables; intentionally independent of the shipped JS module.
PALETTES = [{'name': 'tidal', 'start': '#209792', 'end': '#1d8781', 'ink': '#0d2135'}, {'name': 'coral', 'start': '#ca2b2b', 'end': '#b92727', 'ink': '#eed7a0'}, {'name': 'cove', 'start': '#2f62da', 'end': '#2558d0', 'ink': '#eed7a0'}, {'name': 'seagrass', 'start': '#429c1c', 'end': '#3b8b18', 'ink': '#0d2135'}, {'name': 'anemone', 'start': '#c7237e', 'end': '#b62073', 'ink': '#eed7a0'}, {'name': 'gold', 'start': '#aa8518', 'end': '#987716', 'ink': '#0d2135'}, {'name': 'azure', 'start': '#298ee0', 'end': '#1f84d6', 'ink': '#0d2135'}, {'name': 'reef', 'start': '#257e52', 'end': '#206f47', 'ink': '#eed7a0'}, {'name': 'rose', 'start': '#d36979', 'end': '#cf596b', 'ink': '#0d2135'}, {'name': 'kelp', 'start': '#6e7722', 'end': '#5f671e', 'ink': '#eed7a0'}, {'name': 'orchid', 'start': '#e444d4', 'end': '#e232d0', 'ink': '#0d2135'}, {'name': 'sea', 'start': '#257793', 'end': '#216a83', 'ink': '#eed7a0'}, {'name': 'dawn', 'start': '#df6320', 'end': '#cd5b1d', 'ink': '#0d2135'}, {'name': 'iris', 'start': '#a029e0', 'end': '#961fd6', 'ink': '#eed7a0'}, {'name': 'copper', 'start': '#96612c', 'end': '#865727', 'ink': '#eed7a0'}, {'name': 'lilac', 'start': '#9579d8', 'end': '#8969d3', 'ink': '#0d2135'}]
SILHOUETTES = [{'name': 'cove', 'path': 'M 52 12 C 36 4 13 10 10 28 C 7 45 25 55 43 48 L 47 38 C 33 44 21 40 22 29 C 23 19 35 15 48 23 Z'}, {'name': 'headland', 'path': 'M 12 47 L 12 34 C 22 32 19 18 29 11 C 37 5 51 12 53 24 C 55 35 44 41 35 38 C 29 36 29 48 22 50 Z'}, {'name': 'sandbar', 'path': 'M 10 40 C 13 29 21 28 29 26 C 35 24 37 10 48 10 L 55 21 C 44 22 46 35 35 38 C 27 41 21 38 17 51 Z'}, {'name': 'twin peaks', 'path': 'M 9 43 L 19 15 C 21 9 25 9 28 17 L 33 29 L 42 12 C 45 7 48 9 50 17 L 56 43 C 42 51 24 51 9 43 Z'}, {'name': 'reef', 'path': 'M 8 32 L 25 9 C 28 6 32 8 32 13 L 29 24 L 50 16 C 56 14 58 20 53 25 L 35 48 C 31 54 26 51 28 45 L 32 34 L 13 42 C 7 45 5 39 8 32 Z'}, {'name': 'breaker', 'path': 'M 8 43 C 16 39 17 18 31 11 C 44 4 56 14 54 27 C 48 18 36 17 34 28 C 40 26 51 32 56 43 C 40 52 22 51 8 43 Z'}, {'name': 'inlet', 'path': 'M 10 48 L 10 26 C 10 6 52 6 52 26 L 52 48 L 40 48 L 40 29 C 40 21 22 21 22 29 L 22 48 Z'}, {'name': 'delta', 'path': 'M 27 50 L 25 32 L 9 20 L 14 9 L 31 22 L 48 9 L 56 18 L 39 34 L 40 50 Z'}, {'name': 'spit', 'path': 'M 11 48 C 9 26 23 9 51 10 C 49 33 33 49 11 48 Z'}, {'name': 'shelf', 'path': 'M 10 15 L 32 10 L 33 25 L 52 20 L 55 40 L 39 50 L 12 45 C 8 34 8 25 10 15 Z'}, {'name': 'hook', 'path': 'M 11 10 L 25 10 L 25 32 C 25 45 43 44 43 32 L 43 22 L 55 22 L 55 35 C 55 59 11 58 11 35 Z'}, {'name': 'crescent', 'path': 'M 50 8 C 23 4 8 19 10 36 C 12 52 33 58 52 45 C 30 43 29 23 50 8 Z'}, {'name': 'ridge', 'path': 'M 8 42 L 14 27 L 25 30 L 31 9 L 43 24 L 51 18 L 57 42 C 39 51 24 50 8 42 Z'}, {'name': 'estuary', 'path': 'M 10 13 L 24 11 L 32 28 L 41 10 L 55 15 L 42 33 L 51 47 L 35 51 L 28 39 L 13 48 L 8 34 L 23 29 Z'}, {'name': 'arch', 'path': 'M 8 44 C 9 27 18 8 32 8 C 46 8 55 27 56 44 L 42 47 C 42 33 37 24 32 24 C 27 24 22 33 22 47 Z'}, {'name': 'tidal pool', 'path': 'M 54 27 C 54 47 38 55 21 48 C 6 41 8 18 23 11 C 39 3 51 13 46 27 C 42 37 30 39 25 29 C 29 32 36 29 35 23 C 34 16 21 21 21 31 C 21 43 43 42 43 29 Z'}]
DOMAIN = b"eastsea-account-icon-v2"
VERSION = 3  # Drawing changed; retain the v2 seed and all small-size identities.
ADDRESSES = [
    "0" * 40, "f" * 40, f"{1:040x}", f"{2:040x}", "deadbeef" * 5,
    "52908400098527886e0f7030069857d2e4169ee7", f"{193:040x}",
    "1234567890abcdef1234567890abcdef12345678",
    "1234567890abcdef0000000000abcdef12345678",
    "a2521982a17474cb2f8741c85de653b5282d72b0",
    "6bc5ded76ccbdc8df35e7cd28b68fed245a74416",
    "961f8add5ae93ff0700be8abd5f9f8ec69ba4347",
    "c91367bac92c6de822de8afd0f34ff19fd8f7670",
    f"{255:040x}", f"{256:040x}", f"{65535:040x}",
]


def derive(address):
    seed = hashlib.sha256(DOMAIN + bytes.fromhex(address)).digest()
    return dict(version=VERSION, palette=seed[0] & 15,
                layout=((seed[1] << 8) | seed[2]) & 0x3fff,
                shape=(seed[0] >> 4) & 3, rotation=(seed[0] >> 6) & 3)


def silhouette(spec):
    return spec["shape"] * 4 + (spec["layout"] & 3)


def svg(spec, size=64):
    palette = PALETTES[spec["palette"]]
    gradient = f'eastsea-island-v3-{spec["palette"]}'
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 64 64" aria-hidden="true">',
             f'<defs><linearGradient id="{gradient}" x1="0" y1="0" x2="64" y2="64" gradientUnits="userSpaceOnUse" color-interpolation="sRGB">',
             f'<stop offset="0" stop-color="{palette["start"]}"/><stop offset="1" stop-color="{palette["end"]}"/></linearGradient></defs>',
             f'<rect width="64" height="64" rx="12" fill="url(#{gradient})"/>',
             f'<g fill="{palette["ink"]}" transform="rotate({spec["rotation"] * 90} 32 32)">',
             f'<path d="{SILHOUETTES[silhouette(spec)]["path"]}" transform="translate(0 {5 if size < 32 else 0}) scale(1 0.8)"/>']
    if size >= 32:
        for index in range(2):
            bits = (spec["layout"] >> (2 + index * 6)) & 63
            x = (8 if index == 0 else 40) + (bits & 3)
            y = (44 if index == 0 else 5) + ((bits >> 2) & 3)
            w = (21 if index == 0 else 10) + ((bits >> 4) & 3)
            if index == 0:
                land = f'M {x} {y + 4} C {x + 2} {y - 3} {x + w - 5} {y + 1} {x + w - 2} {y - 2} L {x + w} {y + 6} C {x + w - 3} {y + 11} {x + 3} {y + 13} {x} {y + 4} Z'
            else:
                land = f'M {x} {y + 2} C {x + 3} {y - 1} {x + w - 4} {y - 2} {x + w} {y + 1} L {x + w - 2} {y + 5} C {x + 3} {y + 7} {x + 1} {y + 5} {x} {y + 2} Z'
            parts.append(f'<path d="{land}"/>')
    return "".join(parts) + "</g></svg>"


def fixture():
    vectors = []
    for address in ADDRESSES:
        spec = derive(address)
        vector = dict(address="0x" + address, features=spec,
                      silhouetteClass=silhouette(spec),
                      seedSha256=hashlib.sha256(DOMAIN + bytes.fromhex(address)).hexdigest())
        for size in [16, 32, 64]:
            vector[f"svg{size}Sha256"] = hashlib.sha256(svg(spec, size).encode()).hexdigest()
        vectors.append(vector)
    return dict(domain=DOMAIN.decode(), version=VERSION, palettes=PALETTES, silhouettes=SILHOUETTES,
                backgrounds=dict(light="#f4efe6", dark="#071320"), vectors=vectors)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify without changing golden data")
    args = parser.parse_args()
    content = json.dumps(fixture(), indent=2) + "\n"
    if args.check:
        if FIXTURE.read_text() != content:
            raise SystemExit("Account-icon v3 vectors differ from the independent oracle")
        print("16 frozen v3 vectors match Python hashlib and canonical SVG")
    else:
        FIXTURE.parent.mkdir(parents=True, exist_ok=True)
        FIXTURE.write_text(content)


if __name__ == "__main__":
    main()
