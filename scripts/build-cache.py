#!/usr/bin/env python3
"""Family-scoped dev targets with advisory build leases and bounded LRU cleanup."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CACHE = "/Volumes/workspace/build-cache/targets"
MARKER = ".aether-cache.json"


def output(command, cwd=ROOT):
    return subprocess.check_output(command, cwd=cwd, text=True).strip()


def family_target(cache, base):
    base_commit = output(["git", "rev-parse", "--verify", "--end-of-options", base + "^{commit}"])
    family = output(["git", "merge-base", "HEAD", base_commit])
    # Ignore unrelated manifest edits: only dev/test profiles affect this key.
    sections = re.split(r"(?m)(?=^\[)", (ROOT / "Cargo.toml").read_text())
    profiles = "".join(s for s in sections if re.match(r"\[profile\.(dev|test)([.\]])", s))
    identity = [output(["rustc", "-vV"]), sys.platform, profiles]
    identity.extend(os.environ.get(key, "") for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "MACOSX_DEPLOYMENT_TARGET", "CC", "CARGO_BUILD_TARGET"))
    fingerprint = hashlib.sha256("\n".join(identity).encode()).hexdigest()[:16]
    return cache / f"aether-{family[:16]}-{fingerprint}"


def prepare(target, cache):
    target.mkdir(parents=True, exist_ok=True)
    # Only managed, direct children may ever be removed by prune.
    if target.parent.resolve() == cache.resolve():
        marker = target / MARKER
        if not marker.exists():
            marker.write_text(json.dumps({"version": 1, "created": time.time(), "kind": "dev-test"}) + "\n")


def size_bytes(path):
    # du counts allocated blocks, including sparse incremental artifacts.
    return int(output(["du", "-sk", str(path)]).split()[0]) * 1024


def prune(cache, cap, dry_run=False):
    cache.mkdir(parents=True, exist_ok=True)
    with (cache / ".prune-lock").open("a+") as cleanup_lock:
        try:
            fcntl.flock(cleanup_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return
        entries = []
        total = 0
        for target in cache.iterdir():
            if target.is_symlink() or not target.is_dir() or not (target / MARKER).is_file():
                continue
            size = size_bytes(target)
            total += size
            used = target / ".last-used"
            entries.append((used.stat().st_mtime if used.exists() else target.stat().st_mtime, target, size))
        for _, target, size in sorted(entries):
            if total <= cap:
                break
            with (target / ".lease").open("a+") as lease:
                try:
                    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    continue
                print(f"{'would remove' if dry_run else 'remove'} {target} ({size / 2**30:.2f} GiB)", file=sys.stderr)
                if not dry_run:
                    shutil.rmtree(target)
                total -= size
        if total > cap:
            print(f"cache: {total / 2**30:.2f} GiB exceeds cap; active targets retained", file=sys.stderr)
        if not dry_run:
            (cache / ".last-prune").touch()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["path", "run", "prune"])
    parser.add_argument("--cache-root", default=os.environ.get("AETHER_BUILD_CACHE_ROOT", DEFAULT_CACHE))
    parser.add_argument("--base", default=os.environ.get("AETHER_BUILD_BASE", "lead-merge"))
    parser.add_argument("--max-gib", type=float, default=float(os.environ.get("AETHER_BUILD_CACHE_GIB", "64")))
    parser.add_argument("--dry-run", action="store_true")
    args, command = parser.parse_known_args()
    cache = Path(args.cache_root).expanduser().resolve()
    if args.max_gib <= 0:
        parser.error("cache cap must be positive")
    if args.action == "prune":
        if command:
            parser.error("unexpected prune arguments")
        prune(cache, args.max_gib * 2**30, args.dry_run)
        return 0
    target = Path(os.environ["CARGO_TARGET_DIR"]).expanduser().resolve() if os.environ.get("CARGO_TARGET_DIR") else family_target(cache, args.base)
    if args.action == "path":
        print(target)
        return 0
    if command[:1] == ["--"]:
        command = command[1:]
    if not command:
        parser.error("run requires -- COMMAND [ARGS...]")
    cache.mkdir(parents=True, exist_ok=True)
    # Shared pruning lock closes the deletion/lease acquisition race.
    with (cache / ".prune-lock").open("a+") as cleanup_lock:
        fcntl.flock(cleanup_lock, fcntl.LOCK_SH)
        prepare(target, cache)
        lease = (target / ".lease").open("a+")
        fcntl.flock(lease, fcntl.LOCK_SH)
        (target / ".last-used").touch()
        fcntl.flock(cleanup_lock, fcntl.LOCK_UN)
    environment = dict(os.environ, CARGO_TARGET_DIR=str(target), AETHER_CACHE_LEASE_FD=str(lease.fileno()))
    print(f"dev target: {target}", file=sys.stderr)
    child = subprocess.Popen(command, cwd=ROOT, env=environment, start_new_session=True, pass_fds=(lease.fileno(),))
    previous_handlers = {}
    stopping = False

    def stop(signum, _frame):
        nonlocal stopping
        stopping = True
        raise SystemExit(128 + signum)

    for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        previous_handlers[signum] = signal.signal(signum, stop)
    try:
        code = child.wait()
    finally:
        if stopping:
            # Cleanup after wait() unwinds, never from inside its signal handler.
            try:
                os.killpg(child.pid, signal.SIGTERM)
                child.wait(timeout=10)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                pass
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
        (target / ".last-used").touch()
        lease.close()
        for signum, handler in previous_handlers.items():
            signal.signal(signum, handler)
    # Size walks are amortized: explicit prune always enforces the cap, and the
    # wrapper runs it at most hourly so warm tests don't scan a huge target tree.
    last = cache / ".last-prune"
    if not last.exists() or time.time() - last.stat().st_mtime >= 3600:
        prune(cache, args.max_gib * 2**30)
    return code if code >= 0 else 128 - code


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f"build cache: {error}", file=sys.stderr)
        sys.exit(2)
