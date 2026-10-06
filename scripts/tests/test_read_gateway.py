"""scripts/run-read-gateway.sh: the runner owns the follower's restarts (audit 7 A7-2).

`aether follow` exits on purpose sometimes: the stall watchdog (exit 11) exits
so a supervisor can rebuild its transport, and the disk floor (12) waits for
space. `aether run` has that supervisor (crates/node/src/supervisor.rs); the
gateway runner started `follow` under bare `nohup`, so a stall exit left
cloudflared alive against a dead origin forever. The runner now supervises the
follower with the supervisor's own policy: restart with bounded backoff that
resets after five minutes of life, stop for codes no restart fixes (3 upgrade,
4 storage, 5 verifier, 6 identity, 7 locked), a 30 s wait on the disk floor,
and a stop after four stall exits in an hour — reusing FOLLOW_ARGS verbatim, so
the read-only allowlist and the loopback bind ride every restart.

Each test fakes the two binaries (an exit-code script for `aether`, a
list/route/run stub for `cloudflared`) and runs the real runner with --apply
against a throwaway HOME and data dir. Point AETHER_GATEWAY_RUNNER at another
copy of the script to test it there (the pre-A7-2 script fails these).

    python3 -m unittest scripts/tests/test_read_gateway.py
"""

import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import time
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
RUNNER = pathlib.Path(os.environ.get("AETHER_GATEWAY_RUNNER")
                      or ROOT / "scripts/run-read-gateway.sh")

# Every `aether follow` this fake runs appends its argv and takes the next exit
# code from the list (0 past the list's end); a 0 means "a healthy follower
# that served, then was stopped deliberately" and leaves the origin's recovery
# evidence behind.
FAKE_AETHER = r"""#!/usr/bin/env python3
import json, os, sys
if len(sys.argv) < 2 or sys.argv[1] != "follow":
    sys.exit(7)  # the runner must start `aether follow`, never another mode
d = os.environ["FAKE_DIR"]
count = 0
if os.path.exists(d + "/count"):
    count = int(open(d + "/count").read() or 0)
count += 1
open(d + "/count", "w").write(str(count))
with open(d + "/argv", "a") as f:
    f.write(json.dumps(sys.argv[1:]) + "\n")
codes = [int(x) for x in open(d + "/codes").read().split()] if os.path.exists(d + "/codes") else []
code = codes[count - 1] if count <= len(codes) else 0
if code == 0:
    open(d + "/served", "a").write("origin up\n")
sys.exit(code)
"""

# `tunnel list` reports the one named tunnel (no create needed), `route dns`
# succeeds, and `tunnel run` stays up until the runner stops it.
FAKE_CLOUDFLARED = r"""#!/usr/bin/env python3
import os, sys, time
a = sys.argv[1:]
if "list" in a:
    print('[{"name": "%s", "id": "test-tunnel-id"}]' % os.environ["FAKE_TUNNEL_NAME"])
elif "create" in a or "route" in a:
    pass
else:  # tunnel ... run: the tunnel stays up until the runner stops it
    time.sleep(100000)
"""


class GatewayRunnerTest(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix="read-gateway-test-"))
        self.home = self.tmp / "home"
        self.data = self.tmp / "data"
        self.fake_dir = self.tmp / "fake"
        for d in (self.home, self.data, self.fake_dir, self.home / ".cloudflared"):
            d.mkdir(parents=True)
        (self.home / ".cloudflared" / "test-tunnel-id.json").write_text("{}")
        for name, body in (("aether", FAKE_AETHER), ("cloudflared", FAKE_CLOUDFLARED)):
            p = self.fake_dir / name
            p.write_text(body)
            p.chmod(0o755)

    def tearDown(self):
        # The pre-A7-2 runner left cloudflared behind; never leak it from a test.
        pid_file = self.data / "logs" / "cloudflared.pid"
        if pid_file.exists():
            try:
                os.kill(int(pid_file.read_text()), 15)
            except (ProcessLookupError, ValueError):
                pass
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_runner(self, codes, timeout=120):
        """--apply with the fake binaries; each `aether follow` takes codes[i]."""
        for f in ("count", "argv", "served"):
            (self.fake_dir / f).unlink(missing_ok=True)
        (self.fake_dir / "codes").write_text(" ".join(str(c) for c in codes))
        env = dict(os.environ)
        env.update({
            "HOME": str(self.home),
            "AETHER": str(self.fake_dir / "aether"),
            "CLOUDFLARED": str(self.fake_dir / "cloudflared"),
            "FAKE_DIR": str(self.fake_dir),
            "FAKE_TUNNEL_NAME": "eastsea-read",
        })
        try:
            return subprocess.run(
                [str(RUNNER), "--apply", "--data", str(self.data),
                 "--port", "18551", "--hostname", "rpc.test", "--tunnel", "eastsea-read"],
                env=env, capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            self.fail(f"the runner did not finish within {timeout}s (codes={codes})")

    def runs(self):
        f = self.fake_dir / "count"
        return int(f.read_text()) if f.exists() else 0

    def argvs(self):
        f = self.fake_dir / "argv"
        if not f.exists():
            return []
        return [json.loads(line) for line in f.read_text().splitlines()]

    def kill_gone(self, pid, what):
        deadline = time.time() + 5
        while time.time() < deadline:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return
            time.sleep(0.1)
        self.fail(f"{what} (pid {pid}) survived the runner")

    def test_a_stall_exit_is_restarted_and_the_origin_recovers(self):
        r = self.run_runner([11, 0])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.runs(), 2, "the stall exit must be restarted, not left dead")
        argvs = self.argvs()
        self.assertEqual(argvs[0], argvs[1], "the restarted follower gets the very same args")
        for a in argvs:
            self.assertIn("--public-read-only", a, "the read-only flag rides every restart")
            self.assertEqual(a[a.index("--rpc-port") + 1], "18551",
                             "the loopback port rides every restart")
        self.assertTrue((self.fake_dir / "served").exists(),
                        "the restarted follower really served — the origin recovered")
        self.kill_gone(int((self.data / "logs" / "cloudflared.pid").read_text()),
                       "cloudflared")

    def test_codes_no_restart_fixes_stop_the_runner(self):
        for code in (3, 4, 5, 6, 7):
            with self.subTest(code=code):
                r = self.run_runner([code])
                self.assertEqual(r.returncode, code, r.stdout + r.stderr)
                self.assertEqual(self.runs(), 1, "no restart was attempted")
                self.assertFalse((self.fake_dir / "served").exists())

    def test_a_disk_floor_exit_waits_and_restarts(self):
        r = self.run_runner([12, 0])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.runs(), 2, "the disk floor is waited out, not fatal")
        self.assertTrue((self.fake_dir / "served").exists())

    def test_four_stall_exits_within_an_hour_stop_the_runner(self):
        r = self.run_runner([11, 11, 11, 11])
        self.assertEqual(r.returncode, 11, r.stdout + r.stderr)
        self.assertEqual(self.runs(), 4)
        self.assertIn("stall", (r.stdout + r.stderr).lower())

    def test_a_fatal_task_exit_is_restarted(self):
        r = self.run_runner([9, 0])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.runs(), 2, "a dead critical task is restartable")
        self.assertTrue((self.fake_dir / "served").exists())


if __name__ == "__main__":
    unittest.main()
