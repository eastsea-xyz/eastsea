#!/usr/bin/env python3
"""Aggregate pc_counts.txt by function and by crate using the guest ELF's symbols."""
import bisect, collections, re, subprocess, sys, os
elf = sys.argv[1]
nm = os.path.expanduser("~/.rustup/toolchains/1.95-aarch64-apple-darwin/lib/rustlib/aarch64-apple-darwin/bin/llvm-nm")
syms = []
for l in subprocess.run([nm, "-n", "-C", "--defined-only", elf], capture_output=True, text=True).stdout.splitlines():
    p = l.split(" ", 2)
    if len(p) == 3 and p[1] in "tTwW" and not p[2].startswith(".L"):
        syms.append((int(p[0], 16), p[2].strip()))
syms.sort()
keys = [a for a, _ in syms]
by_fn, by_crate = collections.Counter(), collections.Counter()
total = 0
for l in open("pc_counts.txt"):
    a, c = l.split()
    a, c = int(a, 16), int(c)
    total += c
    i = bisect.bisect_right(keys, a) - 1
    name = syms[i][1] if i >= 0 else "?"
    by_fn[name] += c
    m = re.match(r"^[<&*\s]*(?:impl\s+)?([A-Za-z_][A-Za-z0-9_]*)", name)
    crate = m.group(1) if m else "?"
    by_crate[crate] += c
print(f"total {total:,} cycles")
print("\n== by crate")
for k, v in by_crate.most_common(15):
    print(f"{100*v/total:5.1f}%  {v:>12,}  {k}")
print("\n== top functions")
for k, v in by_fn.most_common(25):
    print(f"{100*v/total:5.1f}%  {v:>12,}  {k[:140]}")
