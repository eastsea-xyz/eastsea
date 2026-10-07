#!/usr/bin/env python3
"""Subset Hahmlet (Korean serif, SIL OFL) to the Hangul used in the site's headlines.

Usage (repo root):
  python3 design/scripts/subset-ko-font.py /path/to/Hahmlet[wght].ttf

Collects every character inside h1/h2/h3/blockquote/td and .brand-ko/.wm-ko in
site/*.html, instances the weight axis to 400-600 and writes
site/fonts/hahmlet-ko-subset.woff2. Re-run whenever headline copy changes:
characters missing from the subset fall back to the system Korean serif.
Needs fontTools + brotli (pip install fonttools brotli).
"""
import sys
from html.parser import HTMLParser
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "site/fonts/hahmlet-ko-subset.woff2"
TAGS = {"h1", "h2", "h3", "blockquote", "td", "summary"}
CLASSES = {"brand-ko", "wm-ko"}


class Collector(HTMLParser):
    def __init__(self):
        super().__init__()
        self.stack = []
        self.chars = set()

    def handle_starttag(self, tag, attrs):
        if tag in {"br", "img", "meta", "link", "input", "path", "circle"}:
            return
        cls = set((dict(attrs).get("class") or "").split())
        self.stack.append(tag in TAGS or bool(cls & CLASSES))

    def handle_endtag(self, tag):
        if self.stack:
            self.stack.pop()

    def handle_data(self, data):
        if any(self.stack):
            self.chars.update(data)


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    c = Collector()
    for page in sorted((ROOT / "site").glob("*.html")):
        c.feed(page.read_text(encoding="utf-8"))
    hangul = {ch for ch in c.chars if "가" <= ch <= "힣"}
    text = "".join(sorted(hangul)) + " .,·!?—–-()0123456789/"
    font = instancer.instantiateVariableFont(TTFont(sys.argv[1]), {"wght": (400, 600)})
    opts = subset.Options()
    opts.flavor = "woff2"
    opts.layout_features = ["kern", "liga"]
    sub = subset.Subsetter(opts)
    sub.populate(text=text)
    sub.subset(font)
    font.flavor = "woff2"
    font.save(OUT)
    print(f"{len(hangul)} Hangul syllables -> {OUT} ({OUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
