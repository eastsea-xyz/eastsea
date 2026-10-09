#!/usr/bin/env python3
"""Run a build after the Mac's counting semaphore, bounding only queue time."""
import getpass
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import shutil
import tempfile
import time


def remote_guarded(root):
    """Only the guarded, owned poc-m3 snapshot may bypass this Mac's semaphore."""
    approved = Path.home() / "eastsea-lab/dev-speed/source"
    return (sys.platform == "darwin" and getpass.getuser() == "kjaylee"
            and root.resolve() == approved.resolve()
            and os.environ.get("AETHER_REMOTE_TEST_ROOT") == str(root.resolve())
            and os.environ.get("AETHER_REMOTE_GUARD_ACTIVE") == "1")


def save_timing(started, queue, compile_time, cleanup, status, code):
    filename = os.environ.get("AETHER_COMPILE_TIMING_FILE")
    if not filename:
        return
    path = Path(filename).resolve()
    root = Path(__file__).resolve().parent.parent
    path.relative_to((root / "tmp").resolve())
    path.parent.mkdir(parents=True, exist_ok=True)
    staged = path.with_name(path.name + f".{os.getpid()}.new")
    staged.write_text(json.dumps(dict(schema_version=1, queue_seconds=queue,
                                    compile_seconds=compile_time, cleanup_seconds=cleanup,
                                    wall_seconds=time.monotonic() - started,
                                    status=status, exit_code=code)) + "\n")
    os.replace(staged, path)


def release_slot(owner, root):
    # The gate's two-minute orphan grace protects a launch gap. A completed,
    # known owner has no launch gap and must not block the next edit for two minutes.
    directory = Path.home() / '.claude/playbooks/aether-team/compile-sem'
    for slot in directory.glob('slot-*'):
        if slot.is_symlink():
            continue
        try:
            if ((slot / 'pid').read_text().strip() == str(owner)
                    and (slot / 'worktree').read_text().strip() == str(root.resolve())):
                shutil.rmtree(slot)
        except FileNotFoundError:
            pass


def terminate(child):
    try:
        os.killpg(child.pid, signal.SIGTERM)
        child.wait(timeout=5)
    except (ProcessLookupError, subprocess.TimeoutExpired):
        pass
    # The shell can exit on TERM while its gate/build descendants ignore it.
    # Always reap the entire group, including after the direct owner exits.
    try:
        os.killpg(child.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    child.wait()
    release_slot(child.pid, Path(__file__).resolve().parent.parent)


def main():
    if len(sys.argv) < 2:
        print("usage: scripts/compile-gate.sh COMMAND [ARGS...]", file=sys.stderr)
        return 2
    command = sys.argv[1:]
    root = Path(__file__).resolve().parent.parent
    if (sys.platform != "darwin" or os.environ.get("AETHER_COMPILE_GATE_HELD") == "1"
            or remote_guarded(root)):
        os.execvp(command[0], command)
    scratch = root / "tmp"
    scratch.mkdir(exist_ok=True)
    gate = Path(os.environ.get("AETHER_COMPILE_GATE", str(Path.home() / ".claude/playbooks/aether-team/wait-compile.sh")))
    if not gate.is_file():
        print(f"compile gate missing: {gate}", file=sys.stderr)
        return 2
    timeout = float(os.environ.get("AETHER_COMPILE_WAIT_SECONDS", "1200"))
    if not 0 < timeout <= 1200:
        print("compile queue timeout must be between 0 and 1200 seconds", file=sys.stderr)
        return 2
    with tempfile.TemporaryDirectory(prefix="compile-gate-", dir=scratch) as directory:
        ready = Path(directory) / "ready"
        # wait-compile records this shell's PID. exec replaces it with the build,
        # so the semaphore stays owned until that build exits.
        script = '"$1" && : > "$2" && shift 2 && export AETHER_COMPILE_GATE_HELD=1 && exec "$@"'
        lease_fd = os.environ.get("AETHER_CACHE_LEASE_FD")
        inherited = (int(lease_fd),) if lease_fd is not None else ()
        started = time.monotonic()
        started_wall = time.time()
        child = subprocess.Popen(["bash", "-c", script, "compile-gate", str(gate), str(ready), *command], cwd=root, start_new_session=True, pass_fds=inherited)

        def stop(signum, _frame):
            raise SystemExit(128 + signum)

        for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            signal.signal(signum, stop)
        try:
            while child.poll() is None and not ready.exists():
                if time.monotonic() - started >= timeout:
                    queue = time.monotonic() - started
                    cleanup_started = time.monotonic()
                    terminate(child)
                    save_timing(started, queue, 0, time.monotonic() - cleanup_started, "queue_timeout", 75)
                    print(f"compile slot wait exceeded {timeout:g}s; remaining build gates must run on the lead", file=sys.stderr)
                    return 75
                time.sleep(0.2)
            queue = max(0, ready.stat().st_mtime - started_wall) if ready.exists() else time.monotonic() - started
            if ready.exists():
                print(f"compile gate: waited {queue:.2f}s", file=sys.stderr)
            code = child.wait()
            release_slot(child.pid, root)
            code = code if code >= 0 else 128 - code
            compile_time = max(0, time.monotonic() - started - queue) if ready.exists() else 0
            save_timing(started, queue, compile_time, 0,
                        ("success" if code == 0 else "build_failed") if ready.exists() else "gate_failed", code)
            return code
        except SystemExit as stopped:
            # Unwind wait()'s non-reentrant waitpid lock before cleanup waits.
            queue = max(0, ready.stat().st_mtime - started_wall) if ready.exists() else time.monotonic() - started
            compile_time = max(0, time.monotonic() - started - queue) if ready.exists() else 0
            cleanup_started = time.monotonic()
            terminate(child)
            save_timing(started, queue, compile_time, time.monotonic() - cleanup_started, "signal", stopped.code)
            return stopped.code


if __name__ == "__main__":
    sys.exit(main())
