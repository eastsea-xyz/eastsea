#!/usr/bin/env python3
"""No network or Rust compilation: exercise remote snapshot and quoting boundaries."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


class RemoteTests(unittest.TestCase):
    def setUp(self):
        (ROOT / 'tmp').mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='remote-test-check.', dir=ROOT / 'tmp')
        self.root = Path(self.temp.name)
        (self.root / 'scripts').mkdir()
        (self.root / 'bin').mkdir()
        script = (ROOT / 'scripts/remote-test.sh').read_text()
        # The production binary is fixed; replace it only in this isolated fixture.
        script = script.replace('/usr/bin/rsync', str(self.root / 'bin/rsync'))
        self.script = self.root / 'scripts/remote-test.sh'
        self.script.write_text(script)
        self.log = self.root / 'calls.jsonl'
        for tool in ('ssh', 'rsync'):
            path = self.root / 'bin' / tool
            path.write_text('#!/usr/bin/env python3\nimport json,os,sys\n'
                            'with open(os.environ["CALL_LOG"],"a") as f: f.write(json.dumps({"tool":sys.argv[0].split("/")[-1],"args":sys.argv[1:],"stdin":sys.stdin.read() if sys.argv[0].endswith("ssh") and "bash -s" in sys.argv[-1] else ""})+"\\n")\n'
                            'if sys.argv[0].endswith("ssh") and os.environ.get("SSH_FAIL"): sys.exit(42)\n')
            path.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.root / 'bin') + ':' + os.environ['PATH'], CALL_LOG=str(self.log))
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)
        subprocess.run(['git', '-C', str(self.root), '-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '--allow-empty', '-qm', 'fixture'], check=True)
        subprocess.run(['git', '-C', str(self.root), 'branch', 'lead-merge'], check=True)
        for name in ('scripts/build-cache.py', 'scripts/compile-gate.sh', 'scripts/compile-gate.py', 'scripts/run-rust-tests.sh', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates/types/src/lib.rs', 'crates/types/.env', 'crates/types/secret.key', 'apps/agent/credential.json'):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('fixture')
        (self.root / 'crates/types/src/outside.rs').symlink_to('/etc/passwd')

    def tearDown(self):
        self.temp.cleanup()

    def run_script(self, *args):
        return subprocess.run(['bash', str(self.script), *args], env=self.env, text=True, capture_output=True)

    def test_dry_run_never_connects(self):
        result = self.run_script('--dry-run', '-p', 'aether-types')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('poc-cuda', result.stdout)
        self.assertFalse(self.log.exists())

    def test_disallowed_inputs_never_connect(self):
        for args in (('-p', 'aether-node'), ('-p', 'aether-ffi'), ('--host', 'poc-m3'), ('-p', 'aether-types', '--', '--target=riscv32im'), ('-p',), ()):
            with self.subTest(args=args):
                self.assertNotEqual(self.run_script(*args).returncode, 0)
                self.assertFalse(self.log.exists())

    def test_snapshot_and_shell_quoting(self):
        filter_name = "name with 'quotes'; $(touch SHOULD_NOT_EXIST)"
        result = self.run_script('-p', 'aether-types', '--', filter_name)
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual([call['tool'] for call in calls], ['ssh', 'ssh', 'rsync', 'ssh'])
        self.assertTrue(all('poc-cuda' in call['args'] for call in (calls[0], calls[1], calls[3])))
        for call in (calls[0], calls[1], calls[3]):
            self.assertIn('Hostname=100.121.197.74', call['args'])
        self.assertIn('uname -s', calls[0]['args'][-1])
        self.assertNotIn('mkdir', calls[0]['args'][-1])
        rsync = calls[2]['args']
        self.assertIn('Hostname=100.121.197.74', rsync[rsync.index('-e') + 1])
        self.assertFalse(any(arg.startswith('--delete') for arg in rsync))
        self.assertRegex(rsync[-1], r'^poc-cuda:/mnt/ssd1/aether-dev/lanes/remote-test\.[A-Za-z0-9]+/$')
        listing = Path(next(arg.split('=', 1)[1] for arg in rsync if arg.startswith('--files-from='))).read_bytes().split(b'\0')
        self.assertIn(b'crates/types/src/lib.rs', listing)
        for helper in ('build-cache.py', 'compile-gate.sh', 'compile-gate.py', 'run-rust-tests.sh'):
            self.assertIn(('scripts/' + helper).encode(), listing)
        self.assertNotIn(b'crates/types/.env', listing)
        self.assertNotIn(b'crates/types/secret.key', listing)
        self.assertNotIn(b'crates/types/src/outside.rs', listing)
        self.assertNotIn(b'apps/agent/credential.json', listing)
        import shlex
        self.assertEqual(shlex.split(calls[3]['args'][-1])[-1], filter_name)
        self.assertIn('scripts/run-rust-tests.sh --ram --', calls[3]['stdin'])
        self.assertIn('AETHER_BUILD_CACHE_ROOT=/mnt/ssd1/aether-dev/targets', calls[3]['stdin'])
        self.assertIn('CARGO_TARGET_DIR="$AETHER_BUILD_CACHE_ROOT/aether-${commit:0:16}-${key:0:16}"', calls[3]['stdin'])
        self.assertFalse((self.root / 'SHOULD_NOT_EXIST').exists())

    def test_commits_share_merge_base_family(self):
        first = self.run_script('--dry-run', '-p', 'aether-types')
        subprocess.run(['git', '-C', str(self.root), '-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '--allow-empty', '-qm', 'another commit'], check=True)
        second = self.run_script('--dry-run', '-p', 'aether-types')
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(first.stdout, second.stdout)

    def test_explicit_setup_only(self):
        result = self.run_script('--setup')
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual([call['tool'] for call in calls], ['ssh', 'ssh', 'rsync', 'ssh'])
        self.assertIn('https://get.nexte.st/latest/linux', calls[-1]['stdin'])
        self.assertIn('cargo install sccache --locked', calls[-1]['stdin'])
        self.assertNotIn('sudo', calls[-1]['stdin'])

    def test_ssh_failure_stops_transfer(self):
        self.env['SSH_FAIL'] = '1'
        self.assertEqual(self.run_script('-p', 'aether-types').returncode, 1)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(len(calls), 1)
        self.assertIn('Hostname=100.121.197.74', calls[0]['args'])
        self.assertIn('uname -s', calls[0]['args'][-1])
        self.assertNotIn('mkdir', calls[0]['args'][-1])
        self.assertFalse((self.root / 'tmp').exists())

    def test_private_key_blocks_transfer(self):
        (self.root / 'crates/types/src/key.rs').write_text('-----BEGIN PRIVATE KEY-----')
        result = self.run_script('-p', 'aether-types')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('private-key material', result.stderr)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(len(calls), 1)
        self.assertIn('uname -s', calls[0]['args'][-1])


if __name__ == '__main__':
    unittest.main()
