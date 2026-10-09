#!/usr/bin/env python3
"""Select changed workspace packages and their transitive reverse dependents."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args])


def changed_paths(root, base):
    ancestor = git(root, "merge-base", base, "HEAD").decode().strip()
    paths = set()
    for args in (("diff", "--name-only", "--no-renames", "-z", ancestor, "HEAD"),
                 ("diff", "--name-only", "--no-renames", "-z", "--cached"),
                 ("diff", "--name-only", "--no-renames", "-z"),
                 ("ls-files", "--others", "--exclude-standard", "-z")):
        paths.update(os.fsdecode(p) for p in git(root, *args).split(b"\0") if p)
    return paths


def select(root, metadata, paths):
    members = set(metadata["workspace_members"])
    packages = {p["id"]: p for p in metadata["packages"] if p["id"] in members}
    directories = {pid: Path(p["manifest_path"]).parent.relative_to(root).as_posix()
                   for pid, p in packages.items()}
    # Manifest changes include removed members which no longer appear in metadata.
    global_change = any(
        Path(path).name in {"Cargo.toml", "Cargo.lock", "rust-toolchain", "rust-toolchain.toml"}
        or path.startswith((".cargo/", ".config/nextest", "vendor/"))
        or ("/" not in path and path.endswith(".rs"))
        for path in paths
    )
    if global_change:
        return sorted(p["name"] for p in packages.values())
    selected = {pid for pid, directory in directories.items()
                if any(path == directory or path.startswith(directory + "/") for path in paths)}
    by_dir = {str(Path(p["manifest_path"]).parent.resolve()): pid for pid, p in packages.items()}
    reverse = {pid: set() for pid in packages}
    for pid, package in packages.items():
        for dep in package["dependencies"]:
            if dep.get("path"):
                dependency = by_dir.get(str(Path(dep["path"]).resolve()))
                if dependency:
                    reverse[dependency].add(pid)
    pending = list(selected)
    while pending:
        for dependent in reverse[pending.pop()] - selected:
            selected.add(dependent)
            pending.append(dependent)
    return sorted(packages[pid]["name"] for pid in selected)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="base ref (default: lead-merge or origin/lead-merge)")
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--metadata-file", type=Path, help="read precomputed cargo metadata (for tests)")
    args = parser.parse_args()
    root = args.root.resolve()
    base = args.base
    if base is None:
        for candidate in ("refs/heads/lead-merge", "refs/remotes/origin/lead-merge"):
            if subprocess.run(["git", "-C", str(root), "rev-parse", "--verify", candidate],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
                base = candidate
                break
        if base is None:
            parser.error("lead-merge ref not found; supply --base <ref>")
    paths = changed_paths(root, base)
    if args.metadata_file:
        metadata = json.loads(args.metadata_file.read_text())
    else:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked", "--offline"], cwd=root))
    names = select(root, metadata, paths)
    if "aether-ffi" in names:
        print("affected-crates: skipping aether-ffi; staticlib verification requires the lead's release gate", file=sys.stderr)
    for name in names:
        if name == "aether-ffi":
            continue
        print(name)


if __name__ == "__main__":
    try:
        main()
    except (subprocess.CalledProcessError, ValueError, OSError) as error:
        print(f"affected-crates: {error}", file=sys.stderr)
        sys.exit(1)
