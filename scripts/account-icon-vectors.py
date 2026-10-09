#!/usr/bin/env python3
"""Independent, stdlib-only oracle for the frozen account-icon v1 vectors."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "crates/client/tests/account-icon-vectors.json"
PALETTES = ["#4e8dad", "#368f8b", "#73864a", "#a77a45", "#b36c5c", "#96749e", "#758694", "#958130"]
INK = "#101820"
DOMAIN = b"eastsea-account-icon-v1"
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
    return dict(version=1, palette=seed[0] & 7,
                layout=((seed[1] << 8) | seed[2]) & 0x3fff,
                shape=(seed[0] >> 3) & 3, rotation=(seed[0] >> 5) & 3)


def svg(spec, size=64):
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 64 64" aria-hidden="true">',
             f'<rect width="64" height="64" rx="12" fill="{PALETTES[spec["palette"]]}"/>',
             f'<g fill="{INK}" transform="rotate({spec["rotation"] * 90} 32 32)">']
    for cell in range(16):
        if cell == 15 or (cell != 0 and not (spec["layout"] >> (cell - 1)) & 1):
            continue
        x, y = 9 + 12 * (cell % 4), 9 + 12 * (cell // 4)
        shape = spec["shape"]
        if shape == 0:
            parts.append(f'<rect x="{x}" y="{y}" width="10" height="10"/>')
        elif shape == 1:
            parts.append(f'<circle cx="{x + 5}" cy="{y + 5}" r="5"/>')
        elif shape == 2:
            parts.append(f'<path d="M{x + 5} {y}L{x + 10} {y + 10}L{x} {y + 10}Z"/>')
        else:
            parts.append(f'<path d="M{x} {y}L{x + 10} {y}A10 10 0 0 1 {x} {y + 10}Z"/>')
    return "".join(parts) + "</g></svg>"


def fixture():
    return dict(domain=DOMAIN.decode(), version=1, palettes=PALETTES, ink=INK,
                backgrounds=dict(light="#f7f5f0", dark="#101820"),
                vectors=[dict(address="0x" + address, features=derive(address),
                              seedSha256=hashlib.sha256(DOMAIN + bytes.fromhex(address)).hexdigest(),
                              svg64Sha256=hashlib.sha256(svg(derive(address)).encode()).hexdigest())
                         for address in ADDRESSES])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify without changing golden data")
    args = parser.parse_args()
    content = json.dumps(fixture(), indent=2) + "\n"
    if args.check:
        if FIXTURE.read_text() != content:
            raise SystemExit("Account-icon v1 vectors differ from the independent oracle")
        print("16 frozen v1 vectors match Python hashlib and canonical SVG")
    else:
        FIXTURE.parent.mkdir(parents=True, exist_ok=True)
        FIXTURE.write_text(content)


if __name__ == "__main__":
    main()
