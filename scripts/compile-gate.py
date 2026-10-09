#!/usr/bin/env python3
"""Run a build after the Mac's counting semaphore, bounding only queue time."""
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time


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


def main():
    if len(sys.argv) < 2:
        print("usage: scripts/compile-gate.sh COMMAND [ARGS...]", file=sys.stderr)
        return 2
    command = sys.argv[1:]
    if sys.platform != "darwin" or os.environ.get("AETHER_COMPILE_GATE_HELD") == "1":
        os.execvp(command[0], command)
    root = Path(__file__).resolve().parent.parent
    scratch = root / "tmp"
    scratch.mkdir(exist_ok=True)
    gate = Path(os.environ.get("AETHER_COMPILE_GATE", str(Path.home() / ".claude/playbooks/aether-team/wait-compile.sh")))
    if not gate.is_file():
        print(f"compile gate missing: {gate}", file=sys.stderr)
        return 2
    timeout = float(os.environ.get("AETHER_COMPILE_WAIT_SECONDS", "1200"))
    with tempfile.TemporaryDirectory(prefix="compile-gate-", dir=scratch) as directory:
        ready = Path(directory) / "ready"
        # wait-compile records this shell's PID. exec replaces it with the build,
        # so the semaphore stays owned until that build exits.
        script = '"$1" && : > "$2" && shift 2 && export AETHER_COMPILE_GATE_HELD=1 && exec "$@"'
        lease_fd = os.environ.get("AETHER_CACHE_LEASE_FD")
        inherited = (int(lease_fd),) if lease_fd is not None else ()
        child = subprocess.Popen(["bash", "-c", script, "compile-gate", str(gate), str(ready), *command], cwd=root, start_new_session=True, pass_fds=inherited)

        def stop(signum, _frame):
            terminate(child)
            raise SystemExit(128 + signum)

        for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            signal.signal(signum, stop)
        started = time.monotonic()
        while child.poll() is None and not ready.exists():
            if time.monotonic() - started >= timeout:
                terminate(child)
                print(f"compile slot wait exceeded {timeout:g}s; remaining build gates must run on the lead", file=sys.stderr)
                return 75
            time.sleep(0.2)
        if ready.exists():
            print(f"compile gate: waited {time.monotonic() - started:.2f}s", file=sys.stderr)
        code = child.wait()
        return code if code >= 0 else 128 - code


if __name__ == "__main__":
    sys.exit(main())
