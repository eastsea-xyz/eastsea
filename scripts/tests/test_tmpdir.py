#!/usr/bin/env python3
"""Bounded lifecycle tests; no real disks or processes are touched."""
import os
import pathlib
import subprocess
import tempfile
import types
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[2]
source = (ROOT / 'scripts/test-tmpdir.sh').read_text().split("<<'PY'\n", 1)[1].rsplit('\nPY', 1)[0]
module = types.ModuleType('test_tmpdir')
with patch.dict(os.environ, AETHER_TEST_ROOT=str(ROOT)):
    exec(compile(source, 'test-tmpdir.sh', 'exec'), module.__dict__)

class Lifecycle(unittest.TestCase):
    def setUp(self):
        (ROOT / 'tmp').mkdir(exist_ok=True)
        self.base = tempfile.TemporaryDirectory(dir=ROOT / 'tmp')
        self.env = patch.dict(os.environ, TMPDIR=self.base.name)
        self.env.start()
    def tearDown(self):
        self.env.stop()
        self.base.cleanup()
    def test_disk_tmpdir_refused(self):
        with patch.object(module, 'ram_backed', return_value=False), patch.object(module.subprocess, 'Popen') as start:
            with self.assertRaisesRegex(RuntimeError, 'verified RAM'):
                module.main(['fake'])
            start.assert_not_called()
    def test_explicit_ram_directory_survives_and_descendants_are_stopped(self):
        with patch.object(module, 'ram_backed', return_value=True), patch.object(module.subprocess, 'Popen') as start, patch.object(module.os, 'killpg') as kill, patch.object(module.time, 'sleep'):
            start.return_value.pid = 12345
            start.return_value.wait.return_value = 7
            self.assertEqual(module.main(['fake']), 7)
            self.assertTrue(start.call_args.kwargs['start_new_session'])
            self.assertEqual(start.call_args.kwargs['env']['AETHER_TEST_TMP_ACTIVE'], '1')
            self.assertEqual(kill.call_count, 2)
            self.assertEqual(list(pathlib.Path(self.base.name).iterdir()), [])
    def test_signal_exit_still_stops_descendants_and_removes_work(self):
        with patch.object(module, 'ram_backed', return_value=True), patch.object(module.subprocess, 'Popen') as start, patch.object(module.os, 'killpg') as kill, patch.object(module.time, 'sleep'):
            start.return_value.pid = 12345
            start.return_value.wait.side_effect = [SystemExit(143), 0]
            with self.assertRaises(SystemExit) as error:
                module.main(['fake'])
            self.assertEqual(error.exception.code, 143)
            self.assertEqual(kill.call_count, 2)
            self.assertEqual(list(pathlib.Path(self.base.name).iterdir()), [])
    def test_mount_failure_detaches_only_created_disk(self):
        calls = []
        def run(*args):
            calls.append(args)
            if 'attach' in args:
                return b'/dev/disk999\n'
            if 'mount' in args:
                raise subprocess.CalledProcessError(1, args)
            return b''
        with patch.dict(os.environ):
            os.environ.pop('TMPDIR', None)
            with patch.object(module.sys, 'platform', 'darwin'), patch.object(module, 'mac_size', return_value=128), patch.object(module, 'run', side_effect=run):
                before = set((ROOT / 'tmp').iterdir())
                with self.assertRaises(subprocess.CalledProcessError):
                    module.main(['fake'])
                self.assertEqual(calls[-1], ('/usr/bin/hdiutil', 'detach', '/dev/disk999'))
                self.assertEqual(set((ROOT / 'tmp').iterdir()), before)
    def test_linux_uses_longest_mount_and_rejects_disk(self):
        mounts = '1 0 0:1 / / rw - ext4 /dev/sda rw\n2 1 0:2 / /dev/shm rw - tmpfs shm rw\n3 2 0:3 / /dev/shm/disk rw - ext4 /dev/sdb rw\n'
        with patch.object(module.sys, 'platform', 'linux'), patch.object(module.pathlib.Path, 'read_text', return_value=mounts):
            self.assertTrue(module.ram_backed('/dev/shm/aether'))
            self.assertFalse(module.ram_backed('/dev/shm/disk/aether'))
            self.assertFalse(module.ram_backed('/dev/shm-lookalike'))
    def test_command_start_failure_removes_work(self):
        with patch.object(module, 'ram_backed', return_value=True), patch.object(module.subprocess, 'Popen', side_effect=FileNotFoundError('fake')):
            with self.assertRaises(FileNotFoundError):
                module.main(['fake'])
            self.assertEqual(list(pathlib.Path(self.base.name).iterdir()), [])
    def test_utility_timeout_kills_group_and_reports_timeout(self):
        with patch.object(module.subprocess, 'Popen') as start, patch.object(module.os, 'killpg') as kill:
            start.return_value.pid = 12345
            start.return_value.communicate.side_effect = subprocess.TimeoutExpired(['fake'], 30)
            with self.assertRaises(subprocess.TimeoutExpired):
                module.run('fake')
            self.assertEqual(start.return_value.communicate.call_args.kwargs['timeout'], 30)
            self.assertEqual(kill.call_count, 2)
    def test_mount_timeout_detaches_known_device(self):
        calls = []
        def run(*args):
            calls.append(args)
            if 'attach' in args:
                return b'/dev/disk999\n'
            if 'mount' in args:
                raise subprocess.TimeoutExpired(args, 30)
            return b''
        with patch.dict(os.environ):
            os.environ.pop('TMPDIR', None)
            with patch.object(module.sys, 'platform', 'darwin'), patch.object(module, 'mac_size', return_value=128), patch.object(module, 'run', side_effect=run):
                with self.assertRaises(subprocess.TimeoutExpired):
                    module.main(['fake'])
                self.assertEqual(calls[-1], ('/usr/bin/hdiutil', 'detach', '/dev/disk999'))
    def test_attach_timeout_partial_device_is_detached(self):
        calls = []
        def run(*args):
            calls.append(args)
            if 'attach' in args:
                raise subprocess.TimeoutExpired(args, 30, output=b'/dev/disk999\n')
            return b''
        with patch.dict(os.environ):
            os.environ.pop('TMPDIR', None)
            with patch.object(module.sys, 'platform', 'darwin'), patch.object(module, 'mac_size', return_value=128), patch.object(module, 'run', side_effect=run):
                with self.assertRaises(subprocess.TimeoutExpired):
                    module.main(['fake'])
                self.assertEqual(calls[-1], ('/usr/bin/hdiutil', 'detach', '/dev/disk999'))
    def test_busy_detach_forces_only_owned_device(self):
        calls = []
        def run(*args):
            calls.append(args)
            if 'attach' in args:
                return b'/dev/disk999\n'
            if 'mount' in args:
                raise RuntimeError('mount failed')
            if 'detach' in args and '-force' not in args:
                raise subprocess.CalledProcessError(1, args)
            return b''
        with patch.dict(os.environ):
            os.environ.pop('TMPDIR', None)
            with patch.object(module.sys, 'platform', 'darwin'), patch.object(module, 'mac_size', return_value=128), patch.object(module, 'run', side_effect=run):
                with self.assertRaisesRegex(RuntimeError, 'mount failed'):
                    module.main(['fake'])
                self.assertEqual(calls[-1], ('/usr/bin/hdiutil', 'detach', '-force', '/dev/disk999'))
    def test_cli_timeout_returns_error_without_disk_fallback(self):
        injection = '\ndef run(*args):\n    raise subprocess.TimeoutExpired(args, 30)\n'
        mocked_source = source.replace("if __name__ == '__main__':", injection + "\nif __name__ == '__main__':")
        env = dict(os.environ, AETHER_TEST_ROOT=str(ROOT))
        env.pop('TMPDIR', None)
        code = 'import sys; sys.platform="darwin"; exec(' + repr(mocked_source) + ')'
        result = subprocess.run([os.sys.executable, '-c', code, 'fake'], env=env, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 1)
        self.assertIn('test-tmpdir:', result.stderr)
        self.assertIn('timed out after 30 seconds', result.stderr)
    def test_low_memory_refused_before_attach(self):
        with patch.dict(os.environ):
            os.environ.pop('TMPDIR', None)
            with patch.object(module.sys, 'platform', 'darwin'), patch.object(module, 'mac_size', side_effect=RuntimeError('insufficient')), patch.object(module, 'run') as run:
                with self.assertRaisesRegex(RuntimeError, 'insufficient'):
                    module.main(['fake'])
                run.assert_not_called()

if __name__ == '__main__':
    unittest.main()
