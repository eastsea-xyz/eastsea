#!/usr/bin/env python3
"""Every Korean wallet render reads Korean (founder review of 0.7.0, lead QA 2026-10-07).

  check-wallet-screens-language.py [tmp/screens]

scripts/wallet-screens.sh writes, beside each PNG, the text the screen shows
(read back through accessibility). For every *-ko-*.txt this fails on
- a known English token of the menu and node lines ("block #", "proofs",
  "Last reward", "Verifying", "Proving", "behind", ...), and
- any Latin-only sentence: a line with two or more English words and no
  Hangul, once the words allowed in Korean copy are taken out (DBLN, Mac,
  Aether, EastSea, https, 0x addresses, CSV, GPU, plus product and sample
  names listed below).
English renders are checked the other way: no Hangul in them.
"""
import re
import sys
from pathlib import Path

# English that leaked into Korean menus before (lead QA 2026-10-07).
TOKENS = ["block #", "proofs", "last reward", "verifying", "proving", "behind", "your data", "not moved"]
# Words a Korean screen may carry as they are (the lead's list, then names).
ALLOWED = ["DBLN", "Mac", "Aether", "EastSea", "https", "CSV", "GPU",
           "Touch ID", "Secure Enclave", "Claude Code", "Codex", "Apple DeviceCheck", "DeviceCheck", "Apple",
           "Pipln", "FileVault", "Metal", "Finder", "iPhone", "iCloud", "APFS", "IP", "RPC", "BLS", "EIP-7864", "SHA-256",
           "GB", "kWh", "W", "CPU", "WAETH", "NEB", "ORB", "CMT", "USDX", "VVDBLN", "PATH", "Samsung T7"]
# Sample data the renderer shows (on-chain names, a payee, an RPC address).
SAMPLE = ["Test Nebula", "Test Orbit", "Test Dollar", "Doubloon Cash", "Shop", "Coffee beans", "order",
          "Swap on the EastSea DEX", "posix_spawn failed", "eastsea.xyz", "eastsea-wallet.xyz", "Applications/Aether.app",
          "explorer", "Block explorer"]

HANGUL = re.compile(r"[가-힣]")


def scrub(line):
    t = line
    t = re.sub(r"https?://\S+", " ", t)
    t = re.sub(r"0x[0-9a-fA-F…\.]+", " ", t)
    t = re.sub(r"~?/[\w./-]+", " ", t)
    for w in sorted(SAMPLE + ALLOWED, key=len, reverse=True):
        t = re.sub(r"(?<![A-Za-z])" + re.escape(w) + r"(?![A-Za-z])", " ", t)
    return t


def problems_ko(lines):
    bad = []
    for line in lines:
        low = line.lower()
        rest = scrub(line)
        words = re.findall(r"[A-Za-z][A-Za-z']+", rest)
        token = any(t in low for t in TOKENS)
        latin_only = len(words) >= 2 and not HANGUL.search(line)
        english_run = re.search(r"[A-Za-z']{2,}(?:[ ,]+[A-Za-z']{2,}){2,}", rest) is not None
        if token or latin_only or english_run:
            bad.append(line)
    return bad


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else "tmp/screens")
    files = sorted(root.glob("*-ko-*.txt"))
    if not files:
        print(f"check-wallet-screens-language: no *-ko-*.txt under {root} (render with scripts/wallet-screens.sh)", file=sys.stderr)
        return 1
    failed = 0
    for f in files:
        bad = problems_ko(f.read_text(encoding="utf-8").splitlines())
        for line in bad:
            print(f"error: {f.name}: English in a Korean screen: {line!r}")
        failed += len(bad)
    for f in sorted(root.glob("*-en-*.txt")):
        for line in f.read_text(encoding="utf-8").splitlines():
            if HANGUL.search(line):
                print(f"error: {f.name}: Korean in an English screen: {line!r}")
                failed += 1
    if failed:
        print(f"check-wallet-screens-language: {failed} line(s) in the wrong language", file=sys.stderr)
        return 1
    print(f"check-wallet-screens-language: {len(files)} Korean renders read Korean; English renders have no Hangul")
    return 0


if __name__ == "__main__":
    sys.exit(main())
