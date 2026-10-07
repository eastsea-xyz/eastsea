#!/usr/bin/env python3
"""The proving guest's inputs, as one digest.

The proving program id (the guest ELF's SHA-256) must follow the guest's real
inputs and nothing else: not the commit it was built at, not its time, not the
checkout's path. This names those inputs and hashes them, so a change that
does not touch them (docs, the wallet, the node) provably leaves them alone and
one that does (aether-execution, a pinned dependency, the fork, the toolchain)
provably moves them.

    scripts/guest-inputs.py [ROOT]            print the digest
    scripts/guest-inputs.py --list [ROOT]     print every input and its hash

The inputs:

- every Aether path crate in the guest's dependency closure (walked through
  apps/prover/Cargo.lock from aether-prover-guest): a tree hash of the files
  cargo compiles from its directory (tests/, benches/, examples/ and
  proptest-regressions/ are never part of the guest build);
- the Cargo.lock entries of that closure (name, version, source, checksum,
  dependencies);
- the manifests the build reads: apps/prover/Cargo.toml (profile, patches) and
  the [workspace.package] / [workspace.dependencies] tables of the root
  Cargo.toml the Aether crates inherit from (not its member list);
- apps/prover/build-guest.sh (target, memory layout, flags) and
  apps/prover/rust-toolchain.toml;
- scripts/jolt-fork.lock (the fork packages are pinned there by commit and
  content hash; the jolt CLI is checked against it);
- the toolchain: `rustc -vV` as rustup resolves it in apps/prover
  (AETHER_GUEST_RUSTC_VV overrides, for tests);
- the canonical stage path (scripts/guest-stage.sh), which cargo hashes.

Only the files' contents and their paths relative to ROOT go in, so the digest
is the same in any checkout directory and on any machine.
"""

import hashlib
import os
import re
import subprocess
import sys

DEFAULT_STAGE = "/tmp/aether-guest-stage"
SKIP_DIRS = {"target", "tests", "benches", "examples", "proptest-regressions", ".git"}
GUEST = "aether-prover-guest"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def parse_lock(text):
    """[[package]] tables of a Cargo.lock as dicts (the format is fixed)."""
    packages, cur, in_deps = [], None, False
    for line in text.splitlines():
        if line == "[[package]]":
            cur, in_deps = {"dependencies": [], "raw": []}, False
            packages.append(cur)
            continue
        if cur is None:
            continue
        if line.startswith("[") and line != "[[package]]":
            cur = None
            continue
        if line.strip():
            cur["raw"].append(line)
        if in_deps:
            if line.strip() == "]":
                in_deps = False
            else:
                cur["dependencies"].append(line.strip().strip(",").strip('"'))
            continue
        m = re.match(r'^(\w+) = "(.*)"$', line)
        if m:
            cur[m.group(1)] = m.group(2)
        elif line.startswith("dependencies = ["):
            in_deps = not line.endswith("]")
    return packages


def closure(packages, root_name):
    """Packages reachable from root_name through the lock's dependency lists."""
    by_name = {}
    for p in packages:
        by_name.setdefault(p["name"], []).append(p)

    def resolve(spec):
        parts = spec.split(" ")
        cands = by_name.get(parts[0], [])
        if len(parts) >= 2:
            cands = [c for c in cands if c.get("version") == parts[1]]
        if len(parts) >= 3:
            src = parts[2].strip("()")
            cands = [c for c in cands if c.get("source") == src]
        if len(cands) != 1:
            sys.exit(f"guest-inputs: Cargo.lock dependency {spec!r} is ambiguous or missing")
        return cands[0]

    seen, todo = {}, [resolve(root_name)]
    while todo:
        p = todo.pop()
        key = (p["name"], p.get("version"), p.get("source"))
        if key in seen:
            continue
        seen[key] = p
        todo.extend(resolve(d) for d in p["dependencies"])
    return [seen[k] for k in sorted(seen, key=lambda k: tuple(x or "" for x in k))]


def local_crates(root):
    """name -> directory of every Aether crate the prover build can see."""
    found = {}
    for base in ("crates", "apps/prover", "apps/prover/patches", "vendor"):
        top = os.path.join(root, base)
        if not os.path.isdir(top):
            continue
        dirs = [top] + [os.path.join(top, d) for d in sorted(os.listdir(top))]
        for d in dirs:
            manifest = os.path.join(d, "Cargo.toml")
            if not os.path.isfile(manifest):
                continue
            with open(manifest, encoding="utf-8") as f:
                text = f.read()
            start = text.find("[package]")
            m = re.search(r'^name\s*=\s*"([^"]+)"', text[start:], re.M) if start >= 0 else None
            if m:
                found.setdefault(m.group(1), d)
    return found


def tree(root, directory):
    """(relative path, sha256) of every file cargo may compile from directory."""
    out = []
    for dirpath, dirnames, filenames in os.walk(directory, followlinks=True):
        dirnames[:] = sorted(
            d for d in dirnames
            if d not in SKIP_DIRS and not d.startswith("target-")
        )
        for name in sorted(filenames):
            if name in (".DS_Store",):
                continue
            path = os.path.join(dirpath, name)
            with open(path, "rb") as f:
                out.append((os.path.relpath(path, root), sha(f.read())))
    return out


def workspace_tables(text):
    """The root manifest's tables a member crate inherits from, verbatim."""
    keep, out = False, []
    for line in text.splitlines():
        m = re.match(r"^\[([^\]]+)\]\s*$", line)
        if m:
            keep = m.group(1) in ("workspace.package", "workspace.dependencies", "workspace.lints", "workspace.lints.rust")
        if keep:
            out.append(line)
    return "\n".join(out) + "\n"


def rustc_vv(root):
    if "AETHER_GUEST_RUSTC_VV" in os.environ:
        return os.environ["AETHER_GUEST_RUSTC_VV"]
    # The rustup proxy first, as the build scripts put it (PATH="$HOME/.cargo/bin:$PATH"):
    # a system rustc (Homebrew) earlier on PATH is not what builds the guest.
    proxy = os.path.join(os.environ.get("CARGO_HOME", os.path.expanduser("~/.cargo")), "bin", "rustc")
    rustc = proxy if os.access(proxy, os.X_OK) else "rustc"
    try:
        return subprocess.run(
            [rustc, "-vV"], cwd=os.path.join(root, "apps/prover"), check=True, capture_output=True, text=True
        ).stdout
    except (OSError, subprocess.CalledProcessError) as e:
        sys.exit(f"guest-inputs: cannot read the guest toolchain (rustc -vV in apps/prover): {e}")


def read(root, rel):
    path = os.path.join(root, rel)
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError as e:
        sys.exit(f"guest-inputs: {rel} is a guest input and cannot be read: {e}")


def inputs(root):
    """Ordered (label, sha256) pairs; the digest is over their listing."""
    entries = []
    lock = read(root, "apps/prover/Cargo.lock").decode()
    pkgs = closure(parse_lock(lock), GUEST)
    crates = local_crates(root)
    for p in pkgs:
        entries.append((f"lock {p['name']} {p.get('version', '')}", sha("\n".join(p["raw"]).encode())))
        if "source" in p:
            continue  # registry or git: the lock entry's checksum / commit pins it
        d = crates.get(p["name"])
        if d is None:
            # A path package outside this tree is the Jolt fork's, pinned by
            # commit and content in scripts/jolt-fork.lock.
            entries.append((f"fork {p['name']}", "pinned by scripts/jolt-fork.lock"))
            continue
        for rel, digest in tree(root, d):
            entries.append((f"src {rel}", digest))
    entries.append(("apps/prover/Cargo.toml", sha(read(root, "apps/prover/Cargo.toml"))))
    entries.append(("Cargo.toml [workspace.*]", sha(workspace_tables(read(root, "Cargo.toml").decode()).encode())))
    for rel in ("apps/prover/build-guest.sh", "apps/prover/rust-toolchain.toml"):
        entries.append((rel, sha(read(root, rel))))
    # The pin the stage actually enforces (AETHER_JOLT_LOCK overrides it, as there).
    fork_lock = os.environ.get("AETHER_JOLT_LOCK") or os.path.join(root, "scripts/jolt-fork.lock")
    entries.append(("scripts/jolt-fork.lock", sha(read(root, fork_lock))))
    entries.append(("rustc -vV", sha(rustc_vv(root).encode())))
    entries.append(("stage", sha(os.environ.get("AETHER_GUEST_STAGE", DEFAULT_STAGE).encode())))
    return entries


def main(argv):
    listing = "--list" in argv
    args = [a for a in argv if a != "--list"]
    root = os.path.realpath(args[0] if args else os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
    entries = inputs(root)
    body = "".join(f"{digest}  {label}\n" for label, digest in entries)
    if listing:
        sys.stdout.write(body)
    print(sha(body.encode()))


if __name__ == "__main__":
    main(sys.argv[1:])
