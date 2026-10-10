"""Timing and remote bypass checks use fixtures, never a real compiler slot."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import signal
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('compile_gate', ROOT / 'scripts/compile-gate.py')
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class CompileTiming(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='compile-timing-', dir=ROOT / 'tmp')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def test_release_only_matching_owner_and_worktree(self):
        directory = self.root / '.claude/playbooks/aether-team/compile-sem'
        for index, pid, worktree in [(1, '123', str(ROOT)), (2, '456', str(ROOT)),
                                     (3, '123', str(self.root))]:
            slot = directory / f'slot-{index}'
            slot.mkdir(parents=True)
            (slot / 'pid').write_text(pid)
            (slot / 'worktree').write_text(worktree)
        with patch.object(gate.Path, 'home', return_value=self.root):
            gate.release_slot(123, ROOT)
        self.assertFalse((directory / 'slot-1').exists())
        self.assertTrue((directory / 'slot-2').is_dir())
        self.assertTrue((directory / 'slot-3').is_dir())

    def test_remote_bypass_requires_entire_owned_guard_contract(self):
        approved = self.root / 'eastsea-lab/dev-speed/source'
        env = dict(AETHER_REMOTE_TEST_ROOT=str(approved), AETHER_REMOTE_GUARD_ACTIVE='1')
        with patch.object(gate.Path, 'home', return_value=self.root), patch.object(gate.sys, 'platform', 'darwin'), patch.object(gate.getpass, 'getuser', return_value='kjaylee'), patch.dict(os.environ, env):
            self.assertTrue(gate.remote_guarded(approved))
            self.assertFalse(gate.remote_guarded(ROOT))
            with patch.dict(os.environ, {'AETHER_REMOTE_GUARD_ACTIVE': '0'}):
                self.assertFalse(gate.remote_guarded(approved))

    def test_shell_and_command_callers_reuse_the_same_live_ancestor_slot(self):
        scripts = self.root / 'scripts'
        scripts.mkdir()
        directory = self.root / 'compile-sem/slot-1'
        directory.mkdir(parents=True)
        wrapper = scripts / 'compile-gate.sh'
        source = (ROOT / 'scripts/compile-gate.sh').read_text()
        wrapper.write_text(source.replace('gate_dir="$HOME/.claude/playbooks/aether-team"',
                                          'gate_dir="$root"'))
        fixture_gate = self.root / 'wait-compile.sh'
        fixture_gate.write_text('#!/bin/bash\necho unexpected second slot >&2\nexit 99\n')
        fixture_gate.chmod(0o755)
        command = '''printf '%s\\n' "$$" > compile-sem/slot-1/pid
pwd -P > compile-sem/slot-1/worktree
bash scripts/compile-gate.sh && bash scripts/compile-gate.sh /bin/bash -c 'test "$AETHER_COMPILE_GATE_HELD" = 1'
'''
        result = subprocess.run(['/bin/bash', '-c', command], cwd=self.root,
                                text=True, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)

    def run_gate(self, script, timeout):
        fixture = self.root / 'gate'
        fixture.write_text('#!/bin/bash\n' + script)
        fixture.chmod(0o755)
        timing = self.root / 'timing.json'
        env = dict(os.environ, AETHER_COMPILE_GATE=str(fixture),
                   AETHER_COMPILE_WAIT_SECONDS=str(timeout), AETHER_COMPILE_TIMING_FILE=str(timing))
        env.pop('AETHER_COMPILE_GATE_HELD', None)
        result = subprocess.run(['python3', str(ROOT / 'scripts/compile-gate.py'), '/usr/bin/true'],
                                env=env, text=True, capture_output=True, timeout=15)
        return result, json.loads(timing.read_text())

    def test_queue_timeout_has_no_compile_time(self):
        result, timing = self.run_gate('sleep 10\n', .25)
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertEqual(timing['status'], 'queue_timeout')
        self.assertEqual(timing['compile_seconds'], 0)
        self.assertGreaterEqual(timing['queue_seconds'], .25)
        self.assertLess(timing['wall_seconds'], 3)

    def test_success_reports_separate_queue_and_command(self):
        result, timing = self.run_gate('sleep .25\n', 2)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(timing['status'], 'success')
        self.assertGreaterEqual(timing['queue_seconds'], .25)
        self.assertGreaterEqual(timing['compile_seconds'], 0)

    def test_signal_unwinds_active_wait_before_cleanup(self):
        for kind in ('gate', 'cache'):
            with self.subTest(kind=kind):
                pidfile = self.root / (kind + '-pid')
                fixture = self.root / 'fast-gate'
                fixture.write_text('#!/bin/bash\nexit 0\n')
                fixture.chmod(0o755)
                env = dict(os.environ, AETHER_COMPILE_GATE=str(fixture),
                           CARGO_TARGET_DIR=str(self.root / 'target'),
                           AETHER_COMPILE_TIMING_FILE=str(self.root / 'signal-timing.json'))
                env.pop('AETHER_COMPILE_GATE_HELD', None)
                command = ['python3', '-c', f'import os,pathlib,time; pathlib.Path({str(pidfile)!r}).write_text(str(os.getpid())); time.sleep(60)']
                wrapper = (['python3', str(ROOT / 'scripts/compile-gate.py')] if kind == 'gate'
                           else ['python3', str(ROOT / 'scripts/build-cache.py'), 'run', '--cache-root', str(self.root / 'cache'), '--'])
                process = subprocess.Popen([*wrapper, *command], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                pid = None
                try:
                    deadline = time.monotonic() + 10
                    while not pidfile.exists() and time.monotonic() < deadline:
                        time.sleep(.02)
                    self.assertTrue(pidfile.exists())
                    pid = int(pidfile.read_text())
                    time.sleep(.3)
                    process.send_signal(signal.SIGTERM)
                    out, err = process.communicate(timeout=15)
                    self.assertEqual(process.returncode, 143, err.decode())
                    with self.assertRaises(ProcessLookupError):
                        os.kill(pid, 0)
                    if kind == 'gate':
                        record = json.loads((self.root / 'signal-timing.json').read_text())
                        self.assertEqual(record['status'], 'signal')
                        self.assertEqual(record['exit_code'], 143)
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.communicate()
                    if pid:
                        try:
                            os.killpg(pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass


if __name__ == '__main__':
    unittest.main()
