#!/usr/bin/env python3
"""Cache and gate regressions using isolated fixtures, never real compilers."""
import fcntl
import importlib.util
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
os.environ["PYTHONDONTWRITEBYTECODE"] = "1"
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('build_cache', ROOT / 'scripts/build-cache.py')
cache_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache_module)

class DevCache(unittest.TestCase):
    def setUp(self):
        (ROOT / 'tmp').mkdir(exist_ok=True)
        self.fixture = tempfile.TemporaryDirectory(prefix='test-dev-cache-', dir=ROOT / 'tmp')
        self.directory = Path(self.fixture.name)
        self.cache = self.directory / 'cache'
        self.cache.mkdir()
    def tearDown(self):
        self.doCleanups()
        self.fixture.cleanup()
    def target(self, name, age):
        path = self.cache / name
        cache_module.prepare(path, self.cache)
        used = path / '.last-used'
        used.touch()
        os.utime(used, (age, age))
        return path
    def wait_file(self, path, process):
        deadline = time.monotonic() + 60
        while not path.exists():
            if process.poll() is not None or time.monotonic() > deadline:
                self.fail('fixture process failed to become ready')
            time.sleep(0.02)
    def test_lru_removes_idle_only_and_ignores_unmanaged_and_symlink(self):
        oldest = self.target('oldest', 1)
        active = self.target('active', 2)
        newest = self.target('newest', 3)
        unmanaged = self.cache / 'unmanaged'
        unmanaged.mkdir()
        linked = self.directory / 'outside'
        linked.mkdir()
        (linked / cache_module.MARKER).touch()
        (self.cache / 'symlink').symlink_to(linked, target_is_directory=True)
        with (active / '.lease').open('a+') as lease:
            fcntl.flock(lease, fcntl.LOCK_SH)
            with patch.object(cache_module, 'size_bytes', return_value=10):
                cache_module.prune(self.cache, 20)
        self.assertFalse(oldest.exists())
        for retained in (active, newest, unmanaged, linked, self.cache / 'symlink'):
            self.assertTrue(retained.exists())
    def test_inherited_lease_survives_cache_parent_sigkill(self):
        target = self.cache / 'leased'
        pidfile = self.directory / 'child.pid'
        code = 'import os,pathlib,time; pathlib.Path(os.environ["PIDFILE"]).write_text(str(os.getpid())); time.sleep(60)'
        env = dict(os.environ, CARGO_TARGET_DIR=str(target), PIDFILE=str(pidfile))
        process = subprocess.Popen([sys.executable, str(ROOT / 'scripts/build-cache.py'), 'run', '--cache-root', str(self.cache), '--', sys.executable, '-c', code], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        child_pid = None
        try:
            self.wait_file(pidfile, process)
            child_pid = int(pidfile.read_text())
            process.kill()
            process.wait(timeout=3)
            with patch.object(cache_module, 'size_bytes', return_value=10):
                cache_module.prune(self.cache, 1)
            self.assertTrue(target.exists(), 'live descendant must retain inherited lease')
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            if child_pid is None and pidfile.exists():
                child_pid = int(pidfile.read_text())
            if child_pid:
                try:
                    os.killpg(child_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
    def test_wrapper_refuses_release_staticlib_and_workspace_without_build(self):
        env = dict(os.environ, RUSTC_WRAPPER='/usr/bin/true', AETHER_BUILD_CACHE_ROOT=str(self.cache))
        cases = [['build', '-p', 'aether-node', '--profile', 'release'], ['build', '-p', 'aether-node', '--profile=release'], ['build', '-p', 'aether-node', '-r'], ['build', '--package=aether-ffi'], ['build', '--workspace'], ['build']]
        for args in cases:
            with self.subTest(args=args):
                result = subprocess.run([str(ROOT / 'scripts/dev-cargo.sh'), *args], env=env, capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn('dev-cargo', result.stderr)
                self.assertEqual(list(self.cache.iterdir()), [])
    def gate_driver(self, gate, command, timeout, ready, ignore_term=False):
        env = dict(os.environ, AETHER_COMPILE_GATE=str(gate), AETHER_COMPILE_WAIT_SECONDS=str(timeout),
                   AETHER_COMPILE_TIMING_FILE=str(self.directory / 'gate-timing.json'))
        env.pop('AETHER_COMPILE_GATE_HELD', None)
        env.pop('AETHER_CACHE_LEASE_FD', None)
        groups = self.directory / 'owned-groups'
        started = self.directory / 'queue-started'
        driver = self.directory / 'driver.py'
        driver.write_text("""import os, pathlib, runpy, signal, subprocess, sys, time
sys.platform = 'darwin'
original = subprocess.Popen
def fixture_popen(*args, **kwargs):
    if IGNORE_TERM:
        kwargs['preexec_fn'] = lambda: signal.signal(signal.SIGTERM, signal.SIG_IGN)
    child = original(*args, **kwargs)
    pathlib.Path(GROUPS).write_text(str(child.pid))
    deadline = time.monotonic() + 60
    while not pathlib.Path(READY).exists():
        if child.poll() is not None or time.monotonic() >= deadline:
            raise RuntimeError('fake gate startup failed')
        time.sleep(.02)
    pathlib.Path(STARTED).write_text(str(time.monotonic()))
    return child
subprocess.Popen = fixture_popen
runpy.run_path(sys.argv.pop(1), run_name='__main__')
""".replace('IGNORE_TERM', repr(ignore_term)).replace('GROUPS', repr(str(groups))).replace('READY', repr(str(ready))).replace('STARTED', repr(str(started))))
        process = subprocess.Popen([sys.executable, str(driver), str(ROOT / 'scripts/compile-gate.py'), *command], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        def cleanup():
            if groups.exists():
                try:
                    os.killpg(int(groups.read_text()), signal.SIGKILL)
                except ProcessLookupError:
                    pass
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
            try:
                process.communicate(timeout=5)
            finally:
                process.stdout.close()
                process.stderr.close()
        self.addCleanup(cleanup)
        return process, started
    def test_gate_owner_pid_is_exec_build_pid(self):
        owner = self.directory / 'owner'
        built = self.directory / 'built'
        gate = self.directory / 'gate.sh'
        gate.write_text('#!/bin/bash\nprintf "%s" "$PPID" > "' + str(owner) + '"\n')
        gate.chmod(0o755)
        code = 'import os,pathlib; pathlib.Path(' + repr(str(built)) + ').write_text(str(os.getpid()))'
        process, started = self.gate_driver(gate, [sys.executable, '-c', code], 3, ready=owner)
        self.wait_file(started, process)
        out, err = process.communicate(timeout=60)
        self.assertEqual(process.returncode, 0, err)
        self.assertEqual(owner.read_text(), built.read_text())
    def test_timeout_cleans_term_ignoring_descendant_after_owner_exits(self):
        gate = self.directory / 'gate.sh'
        group = self.directory / 'gate-group'
        gate.write_text('#!/bin/bash\ntrap "" TERM\nprintf "%s" "$PPID" > "' + str(group) + '"\nwhile true; do sleep 1; done\n')
        gate.chmod(0o755)
        process, started = self.gate_driver(gate, ['/usr/bin/true'], 3, ready=group)
        self.wait_file(started, process)
        out, err = process.communicate(timeout=12)
        self.assertEqual(process.returncode, 75, err)
        with self.assertRaises(ProcessLookupError):
            os.killpg(int(group.read_text()), 0)
    def test_term_ignoring_gate_timeout_is_bounded(self):
        gate = self.directory / 'gate.sh'
        ready = self.directory / 'ready'
        gate.write_text('#!/bin/bash\ntrap "" TERM\nprintf ready > "' + str(ready) + '"\nwhile true; do sleep 1; done\n')
        gate.chmod(0o755)
        process, started = self.gate_driver(gate, ['/usr/bin/true'], 3, ready=ready, ignore_term=True)
        self.wait_file(started, process)
        out, err = process.communicate(timeout=12)
        # Queue timing starts before Popen. Under load the fixture's startup wait
        # can consume part of the queue; its observer marker is not the gate clock.
        import json
        timing = json.loads((self.directory / 'gate-timing.json').read_text())
        elapsed = timing['wall_seconds']
        self.assertEqual(process.returncode, 75, err)
        self.assertGreaterEqual(elapsed, 8)
        self.assertLess(elapsed, 11.5)
        self.assertIn('wait exceeded', err)

if __name__ == '__main__':
    unittest.main()
