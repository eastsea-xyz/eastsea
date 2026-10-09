#!/usr/bin/env python3
"""Verify build reuse and runtime storage without compiling or mounting disks."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class RunnerTests(unittest.TestCase):
    def setUp(self):
        (ROOT / 'tmp').mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='rust fixture ', dir=ROOT / 'tmp')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        scripts = self.root / 'scripts'
        scripts.mkdir()
        shutil.copy2(ROOT / 'scripts/run-rust-tests.sh', scripts)
        self.bin = self.root / 'home/.cargo/bin'
        self.bin.mkdir(parents=True)
        self.log = self.root / 'calls.jsonl'
        self.env = dict(os.environ, HOME=str(self.root / 'home'), TEST_CALL_LOG=str(self.log),
                        AETHER_BUILD_CACHE_ROOT=str(self.root / 'tmp/targets'),
                        TMPDIR='/an/inherited/macos/temp')
        (self.root / 'crates/types').mkdir(parents=True)
        (self.root / 'crates/types/lib.rs').write_text('// initial source\n')
        (self.root / 'Cargo.toml').write_text('[workspace]\n')
        fake = '''#!/usr/bin/env python3
import json, os, pathlib, sys, time
if sys.argv[1:] in (['-V'], ['--version']):
    print('fixture version 1')
    sys.exit(0)
with open(os.environ['TEST_CALL_LOG'], 'a') as f:
    f.write(json.dumps({'exe': os.path.basename(sys.argv[0]), 'args': sys.argv[1:], 'tmp': os.environ.get('TMPDIR'), 'active': os.environ.get('AETHER_TEST_TMP_ACTIVE')}) + '\\n')
if os.path.basename(sys.argv[0]) == 'dev-cargo.sh':
    time.sleep(float(os.environ.get('FIXTURE_BUILD_DELAY', '0')))
    root = pathlib.Path(os.environ['AETHER_TEST_ROOT'])
    target = root / 'tmp/mock-target'
    target.mkdir(parents=True, exist_ok=True)
    binary = target / 'types-test'
    binary.write_text('fixture executable')
    if os.environ.get('FIXTURE_MUTATE_SOURCE'):
        (root / 'crates/types/lib.rs').write_text('// edited while waiting for compile gate\\n')
    print(json.dumps({'rust-build-meta': {'target-directory': str(target), 'non-test-binaries': {}},
                      'rust-binaries': {'types': {'binary-path': str(binary)}}}))
else:
    print('{}')
'''
        for path in (scripts / 'dev-cargo.sh', self.bin / 'cargo', self.bin / 'cargo-nextest'):
            path.write_text(fake)
            path.chmod(0o755)
        rustc = self.bin / 'rustc'
        rustc.write_text('#!/bin/sh\necho fixture-rustc-1\n')
        rustc.chmod(0o755)
        ram = scripts / 'test-tmpdir.sh'
        ram.write_text('''#!/usr/bin/env python3
import json, os, subprocess, sys
with open(os.environ['TEST_CALL_LOG'], 'a') as f:
    f.write(json.dumps({'exe':'ram', 'tmp':os.environ.get('TMPDIR')}) + '\\n')
env = dict(os.environ, TMPDIR='verified-ram', AETHER_TEST_TMP_ACTIVE='1')
sys.exit(subprocess.call(sys.argv[1:], env=env))
''')
        ram.chmod(0o755)

    def run_helper(self, *args):
        return subprocess.run(['bash', str(self.root / 'scripts/run-rust-tests.sh'), *args],
                              env=self.env, capture_output=True, text=True)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_build_and_runtime_are_separate(self):
        result = self.run_helper('--', '-p', 'aether-types', '--lib', '-E', 'test(name with space)', '--test-threads', '1')
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertEqual([c['exe'] for c in calls], ['dev-cargo.sh', 'cargo', 'cargo'])
        self.assertEqual(calls[0]['args'][:7], ['nextest', 'list', '--list-type', 'binaries-only', '--message-format', 'json', '--locked'])
        self.assertNotIn('--test-threads', calls[0]['args'])
        self.assertEqual(calls[1]['args'][0], 'metadata')
        runtime = calls[2]
        self.assertEqual(runtime['args'][:2], ['nextest', 'run'])
        self.assertIn('--binaries-metadata', runtime['args'])
        self.assertIn('--cargo-metadata', runtime['args'])
        self.assertNotIn('-p', runtime['args'])
        self.assertNotIn('--locked', runtime['args'])
        self.assertNotIn('--lib', runtime['args'])
        self.assertIn('test(name with space)', runtime['args'])
        self.assertTrue(runtime['tmp'].startswith(str(self.root / 'tmp')))
        self.assertFalse(any(p.name.startswith('rust-tests-') for p in (self.root / 'tmp').iterdir()))

    def test_ram_only_wraps_runtime_and_discards_inherited_tmpdir(self):
        result = self.run_helper('--ram', '--', '-p', 'aether-node')
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertEqual([c['exe'] for c in calls], ['dev-cargo.sh', 'cargo', 'ram', 'cargo'])
        self.assertEqual(calls[0]['tmp'], str(self.root / 'tmp'))
        self.assertIsNone(calls[2]['tmp'])
        self.assertEqual(calls[3]['tmp'], 'verified-ram')
        self.assertEqual(calls[3]['active'], '1')

    def test_ram_override_is_preserved(self):
        self.env['AETHER_TEST_TMPDIR'] = '/explicit ram path'
        result = self.run_helper('--ram', '--', '-p', 'aether-node')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls()[2]['tmp'], '/explicit ram path')

    def test_unsafe_options_fail_before_compilation(self):
        for args in (('-p', 'aether-ffi'), ('-p', 'aether-prover'), ('--workspace',),
                     ('-p', 'aether-types', '--release'), ('-p', 'aether-types', '--cargo-profile', 'release'),
                     ('-p', 'aether-types', '--manifest-path', 'apps/prover/Cargo.toml')):
            with self.subTest(args=args):
                result = self.run_helper('--', *args)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertFalse(self.log.exists())

    def test_unchanged_inputs_skip_gate_even_when_runtime_filter_changes(self):
        first = self.run_helper('--', '-p', 'aether-types', '-E', 'test(first)')
        self.assertEqual(first.returncode, 0, first.stderr)
        second = self.run_helper('--', '-p', 'aether-types', '-E', 'test(second)')
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertIn('no compile gate', second.stderr)
        self.assertEqual(sum(c['exe'] == 'dev-cargo.sh' for c in self.calls()), 1)
        self.assertIn('test(second)', self.calls()[-1]['args'])

    def test_source_options_compiler_and_flags_invalidate(self):
        changes = ('source', 'manifest', 'compiler', 'flags', 'options', 'vendor')
        for change in changes:
            with self.subTest(change=change):
                args = ['--', '-p', 'aether-types']
                first = self.run_helper(*args)
                self.assertEqual(first.returncode, 0, first.stderr)
                before = sum(c['exe'] == 'dev-cargo.sh' for c in self.calls())
                if change == 'source':
                    (self.root / 'crates/types/lib.rs').write_text('// source changed\n')
                elif change == 'manifest':
                    (self.root / 'Cargo.toml').write_text('[workspace]\n# changed\n')
                elif change == 'compiler':
                    (self.bin / 'rustc').write_text('#!/bin/sh\necho fixture-rustc-2\n')
                elif change == 'flags':
                    self.env['RUSTFLAGS'] = '-C opt-level=1'
                elif change == 'options':
                    args.append('--lib')
                else:
                    (self.root / 'vendor').mkdir()
                    (self.root / 'vendor/input.rs').write_text('// vendor input\n')
                second = self.run_helper(*args)
                self.assertEqual(second.returncode, 0, second.stderr)
                self.assertEqual(sum(c['exe'] == 'dev-cargo.sh' for c in self.calls()), before + 1)

    def test_missing_or_replaced_binary_rebuilds(self):
        for action in ('delete', 'replace'):
            with self.subTest(action=action):
                result = self.run_helper('--', '-p', 'aether-types')
                self.assertEqual(result.returncode, 0, result.stderr)
                before = sum(c['exe'] == 'dev-cargo.sh' for c in self.calls())
                binary = self.root / 'tmp/mock-target/types-test'
                if action == 'delete':
                    binary.unlink()
                else:
                    binary.write_text('another build replaced this artifact')
                result = self.run_helper('--', '-p', 'aether-types')
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(sum(c['exe'] == 'dev-cargo.sh' for c in self.calls()), before + 1)

    def test_concurrent_invocations_publish_one_complete_build(self):
        self.env['FIXTURE_BUILD_DELAY'] = '0.2'
        command = ['bash', str(self.root / 'scripts/run-rust-tests.sh'), '--', '-p', 'aether-types']
        children = [subprocess.Popen(command, env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE) for _ in range(2)]
        for child in children:
            stdout, stderr = child.communicate(timeout=30)
            self.assertEqual(child.returncode, 0, stderr.decode())
        self.assertEqual(sum(c['exe'] == 'dev-cargo.sh' for c in self.calls()), 1)

    def test_symlink_inputs_disable_reuse(self):
        (self.root / 'crates/types/escape.rs').symlink_to(self.root / 'Cargo.toml')
        for _ in range(2):
            result = self.run_helper('--', '-p', 'aether-types')
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('cache disabled', result.stderr)
        self.assertEqual(sum(c['exe'] == 'dev-cargo.sh' for c in self.calls()), 2)

    def test_source_edit_during_build_prevents_stale_publication(self):
        self.env['FIXTURE_MUTATE_SOURCE'] = '1'
        result = self.run_helper('--', '-p', 'aether-types')
        self.assertEqual(result.returncode, 0, result.stderr)
        cache = self.root / 'tmp/rust-test-cache'
        self.assertFalse(any(path.is_dir() for path in cache.iterdir()))
        del self.env['FIXTURE_MUTATE_SOURCE']
        result = self.run_helper('--', '-p', 'aether-types')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(sum(c['exe'] == 'dev-cargo.sh' for c in self.calls()), 2)


if __name__ == '__main__':
    unittest.main()
