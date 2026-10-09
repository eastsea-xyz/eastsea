#!/usr/bin/env python3
"""No network or compilation: verify remote scope, snapshots, and resource cleanup."""
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parent.parent
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('remote_guard', ROOT / 'scripts/remote-resource-guard.py')
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class RemoteTests(unittest.TestCase):
    def setUp(self):
        (ROOT / 'tmp').mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='remote-test-check.', dir=ROOT / 'tmp')
        self.root = Path(self.temp.name)
        (self.root / 'scripts').mkdir()
        (self.root / 'bin').mkdir()
        script = (ROOT / 'scripts/remote-test.sh').read_text()
        # Replace only the local production binary in this isolated fixture.
        script = script.replace("['/usr/bin/rsync', '-a'", '[' + repr(str(self.root / 'bin/rsync')) + ", '-a'", 1)
        self.script = self.root / 'scripts/remote-test.sh'
        self.script.write_text(script)
        self.log = self.root / 'calls.jsonl'
        for tool in ('ssh', 'rsync'):
            path = self.root / 'bin' / tool
            path.write_text('''#!/usr/bin/env python3
import json, os
from pathlib import Path
import subprocess, sys, time
tool = Path(sys.argv[0]).name
record = {'tool': tool, 'args': sys.argv[1:], 'stdin': sys.stdin.read() if tool == 'ssh' else ''}
if tool == 'rsync':
    source = Path(sys.argv[-2])
    record['files'] = sorted(str(p.relative_to(source)) for p in source.rglob('*') if p.is_file())
with open(os.environ['CALL_LOG'], 'a') as out: out.write(json.dumps(record) + '\\n')
if tool == 'ssh' and os.environ.get('SSH_FAIL'): sys.exit(42)
if tool == 'ssh' and '--preflight' in sys.argv[-1] and os.environ.get('LOW_RAM'): sys.exit(75)
if tool == 'ssh' and 'bash -s' in sys.argv[-1]:
    if os.environ.get('SSH_HANG'):
        Path(os.environ['SSH_STARTED']).write_text(str(os.getpid()))
        time.sleep(30)
    status = int(os.environ.get('TEST_EXIT', '0'))
    print('dev-test timing: '+json.dumps({'compile_seconds':.2,'run_seconds':.01,'exit_code':status,
                                        'resources':{'peak_owned_rss_bytes':12345}}))
    sys.exit(status)
if tool == 'rsync':
    if os.environ.get('RSYNC_FAIL'): sys.exit(43)
    destination = Path(os.environ['MOCK_REMOTE_SOURCE'])
    destination.mkdir(exist_ok=True)
    # Use actual system rsync locally to exercise production deletion rules.
    subprocess.run(['/usr/bin/rsync', '-a', '--delete', '--exclude=/tmp/', '--exclude=/.git/',
                    sys.argv[-2], str(destination) + '/'], check=True,
                   env=dict(os.environ, PATH='/usr/bin:/bin:/usr/sbin:/sbin'))
''')
            path.chmod(0o755)
        self.remote_source = self.root / 'remote-source'
        self.env = dict(os.environ, PATH=str(self.root / 'bin') + ':' + os.environ['PATH'],
                        CALL_LOG=str(self.log), MOCK_REMOTE_SOURCE=str(self.remote_source),
                        SSH_STARTED=str(self.root / 'ssh-started'))
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)
        subprocess.run(['git', '-C', str(self.root), '-c', 'user.name=Test', '-c',
                        'user.email=test@example.invalid', 'commit', '--allow-empty', '-qm', 'fixture'], check=True)
        subprocess.run(['git', '-C', str(self.root), 'branch', 'lead-merge'], check=True)
        for package in ('types', 'node', 'ffi'):
            path = self.root / 'crates' / package / 'Cargo.toml'
            path.parent.mkdir(parents=True)
            path.write_text('[package]\nname = "aether-' + package + '"\nversion = "0.1.0"\n')
        for name in ('build-cache.py', 'compile-gate.sh', 'compile-gate.py', 'run-rust-tests.sh',
                     'test-swift-pure.sh', 'swift-test-cache.py', 'wallet-l10n.py',
                     'test-update-daemon.sh', 'test-update-listener.sh', 'remote-resource-guard.py'):
            shutil.copy2(ROOT / 'scripts' / name, self.root / 'scripts' / name)
        for name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates/types/src/lib.rs',
                     'crates/types/.env', 'crates/types/secret.key', 'apps/agent/credential.json',
                     'apps/wallet/Sources/EarningsModel.swift', 'apps/wallet/Tests/earnings/main.swift',
                     'apps/wallet/Resources/Localizable.xcstrings', 'apps/agent/Tests/history/main.swift',
                     'apps/bridge/Tests/bridge-plan/main.swift', 'apps/prover/guest/src/main.rs'):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('fixture')
        (self.root / 'crates/types/src/outside.rs').symlink_to('/etc/passwd')

    def tearDown(self):
        self.temp.cleanup()

    def run_script(self, *args):
        return subprocess.run(['bash', str(self.script), *args], env=self.env,
                              text=True, capture_output=True, timeout=10)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_dry_run_never_connects(self):
        result = self.run_script('--dry-run', '-p', 'aether-node', '--test', 'wake_signal')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('poc-m3 (user kjaylee, macOS)', result.stdout)
        self.assertIn('~/eastsea-lab/dev-speed/source', result.stdout)
        self.assertIn('nice=15', result.stdout)
        self.assertFalse(self.log.exists())

    def test_disallowed_inputs_never_connect(self):
        cases = (('-p', 'aether-ffi'), ('-p', 'aether-prover'), ('-p', 'unknown'),
                 ('--host', 'example.invalid'), ('-p', 'aether-types', '--', '--target=riscv32im'),
                 ('-p', 'aether-types', '--', '--release'), ('-p', 'aether-types', '--', '--profile=release'),
                 ('-p', 'aether-types', '--', '--manifest-path=apps/prover/Cargo.toml'),
                 ('-p', 'aether-types', '--', '-E'), ('--swift', 'missing'), ('--swift',),
                 ('--test', '../outside'), ('-p', 'aether-node', '--test', '../outside'), ('-p',), ())
        for args in cases:
            with self.subTest(args=args):
                self.assertNotEqual(self.run_script(*args).returncode, 0)
                self.assertFalse(self.log.exists())

    def test_snapshot_and_shell_quoting(self):
        expression = "name with 'quotes'; $(touch SHOULD_NOT_EXIST)"
        result = self.run_script('-p', 'aether-node', '--test', 'wake_signal', '--', '-E', expression)
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertEqual([call['tool'] for call in calls], ['ssh', 'ssh', 'rsync', 'ssh', 'ssh'])
        for call in (calls[0], calls[1], calls[3], calls[4]):
            self.assertEqual(call['args'][-2], 'poc-m3')
            self.assertEqual(call['args'][call['args'].index('-l') + 1], 'kjaylee')
            self.assertFalse(any(arg.startswith('Hostname=') for arg in call['args']))
        self.assertIn('--preflight', calls[0]['args'][-1])
        self.assertIn('vm_stat', calls[0]['stdin'])
        self.assertNotIn('.mkdir(', calls[0]['stdin'])
        rsync = calls[2]['args']
        self.assertIn('--delete', rsync)
        self.assertIn('--exclude=/tmp/', rsync)
        self.assertIn('--rsync-path=/usr/bin/rsync', rsync)
        self.assertEqual(rsync[-1], 'poc-m3:eastsea-lab/dev-speed/source/')
        listing = calls[2]['files']
        self.assertIn('crates/types/src/lib.rs', listing)
        for helper in ('build-cache.py', 'compile-gate.sh', 'compile-gate.py', 'run-rust-tests.sh',
                       'test-swift-pure.sh', 'swift-test-cache.py', 'remote-resource-guard.py'):
            self.assertIn('scripts/' + helper, listing)
        self.assertIn('apps/wallet/Resources/Localizable.xcstrings', listing)
        for excluded in ('crates/types/.env', 'crates/types/secret.key', 'crates/types/src/outside.rs',
                         'apps/agent/credential.json', 'apps/prover/guest/src/main.rs'):
            self.assertNotIn(excluded, listing)
        self.assertEqual(shlex.split(calls[3]['args'][-1])[-2:], ['-E', expression])
        self.assertIn('--test wake_signal', calls[3]['args'][-1])
        self.assertIn('CARGO_BUILD_JOBS=4', calls[3]['stdin'])
        self.assertIn('CARGO_HOME="$base/tmp/cargo-home"', calls[3]['stdin'])
        self.assertIn('CARGO_TARGET_DIR="$AETHER_BUILD_CACHE_ROOT/aether-${commit:0:16}-${key:0:16}"', calls[3]['stdin'])
        self.assertIn('remote-resource-guard.py --root', calls[3]['stdin'])
        self.assertNotIn('AETHER_COMPILE_GATE_HELD', calls[3]['stdin'])
        self.assertFalse((self.root / 'SHOULD_NOT_EXIST').exists())

    def test_swift_subset_and_timing_transport(self):
        result = self.run_script('--swift', 'earnings', '--swift', 'token-send')
        self.assertEqual(result.returncode, 0, result.stderr)
        call = next(call for call in self.calls() if 'bash -s' in call['args'][-1])
        self.assertIn('--swift earnings --swift token-send --', call['args'][-1])
        self.assertIn('bash scripts/test-swift-pure.sh "${swift[@]}"', call['stdin'])
        self.assertIn('AETHER_DEV_TIMING_FILE=', call['stdin'])
        self.assertIn('dev-test timing: ', call['stdin'])
        timing = json.loads(next(line.removeprefix('dev-test timing: ')
                                 for line in result.stdout.splitlines() if line.startswith('dev-test timing: ')))
        self.assertGreater(timing['rsync_seconds'], 0)
        self.assertEqual(timing['rsync_exit_code'], 0)
        self.assertEqual(timing['resources']['peak_owned_rss_bytes'], 12345)
        self.assertIn('--report-file', call['stdin'])

    def test_failed_rsync_starts_no_tests_and_releases_lease(self):
        self.env['RSYNC_FAIL'] = '1'
        result = self.run_script('-p', 'aether-node', '--test', 'rpc_alias')
        self.assertEqual(result.returncode, 43, result.stderr)
        calls = self.calls()
        self.assertFalse(any('bash -s' in call['args'][-1] for call in calls))
        self.assertIn('lock.rmdir()', calls[-1]['stdin'])
        self.assertNotIn('dev-test timing: ', result.stdout)

    def test_startup_failure_transports_resources_without_workload_timing(self):
        token = 'startup-test'
        resources = {'exit_code': 75, 'cleanup_complete': True, 'peak_owned_rss_bytes': 12345}
        (self.root / ('remote-' + token + '-resources.json')).write_text(json.dumps(resources))
        body = self.script.read_text().split("<<'TIMING'\n", 1)[1].split('\nTIMING\n', 1)[0]
        result = subprocess.run([sys.executable, '-', str(self.root), token, '75'], input=body,
                                text=True, capture_output=True, check=True)
        timing = json.loads(result.stdout.removeprefix('dev-test timing: '))
        self.assertEqual(timing['resources'], resources)
        self.assertEqual(timing['exit_code'], 75)
        self.assertEqual(timing['workloads'], [])
        self.assertEqual(timing['compile_seconds'], 0)

    def test_snapshot_deletes_stale_sources_and_keeps_warm_artifacts(self):
        self.assertEqual(self.run_script('-p', 'aether-types').returncode, 0)
        warm = self.remote_source / 'tmp/swift-test-cache/binary'
        warm.parent.mkdir(parents=True)
        warm.write_text('warm')
        (self.root / 'crates/types/src/lib.rs').unlink()
        self.assertEqual(self.run_script('-p', 'aether-types').returncode, 0)
        self.assertFalse((self.remote_source / 'crates/types/src/lib.rs').exists())
        self.assertEqual(warm.read_text(), 'warm')
        transfers = [call for call in self.calls() if call['tool'] == 'rsync']
        self.assertEqual(transfers[0]['args'][-1], transfers[1]['args'][-1])

    def test_commits_share_merge_base_family(self):
        first = self.run_script('--dry-run', '-p', 'aether-types')
        subprocess.run(['git', '-C', str(self.root), '-c', 'user.name=Test', '-c',
                        'user.email=test@example.invalid', 'commit', '--allow-empty', '-qm', 'another commit'], check=True)
        second = self.run_script('--dry-run', '-p', 'aether-types')
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(first.stdout, second.stdout)

    def test_setup_only_downloads_lane_local_prebuilt_tools(self):
        result = self.run_script('--setup')
        self.assertEqual(result.returncode, 0, result.stderr)
        call = next(call for call in self.calls() if 'bash -s' in call['args'][-1])
        self.assertIn('https://get.nexte.st/0.9/mac', call['stdin'])
        self.assertIn('sccache-v0.18.0-aarch64-apple-darwin.tar.gz', call['stdin'])
        self.assertIn('hashlib.sha256', call['stdin'])
        self.assertIn('"$base/tmp/tools"', call['stdin'])
        self.assertNotIn('cargo install', call['stdin'])
        self.assertNotIn('rustup-init', call['stdin'])
        self.assertNotIn('sudo', call['stdin'])
        self.assertIn('! command -v cargo-nextest', call['stdin'])
        self.assertIn('! command -v sccache', call['stdin'])

    def test_preflight_failure_stops_transfer_and_local_scratch(self):
        for key, status in [('SSH_FAIL', 42), ('LOW_RAM', 75)]:
            with self.subTest(key=key):
                self.env[key] = '1'
                self.assertEqual(self.run_script('-p', 'aether-types').returncode, status)
                self.assertFalse((self.root / 'tmp').exists())
                self.env.pop(key)
        self.assertEqual(len(self.calls()), 2)

    def test_private_key_blocks_transfer(self):
        (self.root / 'crates/types/src/key.rs').write_text('-----BEGIN PRIVATE KEY-----')
        result = self.run_script('-p', 'aether-types')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('private-key material', result.stderr)
        self.assertEqual(len(self.calls()), 1)

    def test_test_failure_still_releases_owned_lease(self):
        self.env['TEST_EXIT'] = '7'
        result = self.run_script('-p', 'aether-types')
        self.assertEqual(result.returncode, 7)
        self.assertIn('lock.rmdir()', self.calls()[-1]['stdin'])

    def test_cleanup_failure_retains_snapshot_lease(self):
        base = self.root / 'lease-fixture'
        lock = base / 'tmp/remote-test.lock'
        lock.mkdir(parents=True)
        token = 'cleanup-test'
        owner = lock / 'owner'
        owner.write_text(token)
        resources = base / ('source/tmp/remote-' + token + '-resources.json')
        resources.parent.mkdir(parents=True)
        body = self.script.read_text().split("<<'RELEASE'", 1)[1].split('\n', 1)[1].split('\nRELEASE\n', 1)[0]
        body = body.replace("base = Path.home() / 'eastsea-lab/dev-speed'", 'base = Path(' + repr(str(base)) + ')')
        resources.write_text(json.dumps({'cleanup_complete': False}))
        result = subprocess.run([sys.executable, '-', token], input=body, text=True, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('snapshot lease retained', result.stderr)
        self.assertTrue(owner.exists())
        resources.write_text(json.dumps({'cleanup_complete': True}))
        result = subprocess.run([sys.executable, '-', token], input=body, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(lock.exists())

    def test_signal_stops_ssh_and_runs_matching_cleanup(self):
        self.env['SSH_HANG'] = '1'
        child = subprocess.Popen(['bash', str(self.script), '-p', 'aether-types'], env=self.env,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            deadline = time.monotonic() + 5
            while not Path(self.env['SSH_STARTED']).exists() and time.monotonic() < deadline:
                time.sleep(0.02)
            self.assertTrue(Path(self.env['SSH_STARTED']).exists())
            child.send_signal(signal.SIGTERM)
            child.communicate(timeout=5)
            self.assertEqual(child.returncode, 143)
            self.assertIn('os.kill(value[\'pid\'], signal.SIGTERM)', self.calls()[-1]['stdin'])
        finally:
            if child.poll() is None:
                child.kill()
            child.communicate()


class ResourceGuardTests(unittest.TestCase):
    def setUp(self):
        (ROOT / 'tmp').mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='remote-guard-check.', dir=ROOT / 'tmp')
        self.root = Path(self.temp.name)
        (self.root / 'tmp').mkdir()

    def tearDown(self):
        self.temp.cleanup()

    def test_mac_memory_counts_only_free_and_speculative_pages(self):
        text = ('Mach Virtual Memory Statistics: (page size of 16384 bytes)\n'
                'Pages free: 200000.\nPages speculative: 62144.\nPages inactive: 999999.\n')
        with mock.patch.object(guard.subprocess, 'check_output', return_value=text):
            self.assertEqual(guard.free_ram(), 4 * guard.GIB)
        with mock.patch.object(guard.subprocess, 'check_output', return_value='unknown statistics'):
            self.assertRaises(RuntimeError, guard.free_ram)

    def test_limits_refuse_low_ram_disk_or_excess_owned_memory(self):
        for ram, disk, rss in [(guard.MIN_RAM - 1, 40 * guard.GIB, 0),
                               (8 * guard.GIB, guard.MIN_DISK - 1, 0),
                               (8 * guard.GIB, 40 * guard.GIB, guard.MAX_RSS + 1)]:
            with self.subTest(ram=ram, disk=disk, rss=rss), \
                 mock.patch.object(guard, 'free_ram', return_value=ram), \
                 mock.patch.object(guard.shutil, 'disk_usage', return_value=SimpleNamespace(free=disk)):
                self.assertRaises(RuntimeError, guard.check_resources, self.root, rss)
        with mock.patch.object(guard, 'free_ram', return_value=guard.MIN_RAM), \
             mock.patch.object(guard.shutil, 'disk_usage', return_value=SimpleNamespace(free=guard.MIN_DISK)):
            self.assertEqual(guard.check_resources(self.root, guard.MAX_RSS), (guard.MIN_RAM, guard.MIN_DISK))

    def test_root_rejects_wrong_host_account_and_symlinks(self):
        with mock.patch.object(guard.sys, 'platform', 'linux'):
            self.assertRaises(RuntimeError, guard.authorized_base)
        with mock.patch.object(guard.sys, 'platform', 'darwin'), \
             mock.patch.object(guard.pwd, 'getpwuid', return_value=SimpleNamespace(pw_name='other', pw_dir=str(self.root))):
            self.assertRaises(RuntimeError, guard.authorized_base)
        base = self.root / 'eastsea-lab/dev-speed'
        with mock.patch.object(guard, 'authorized_base', return_value=base):
            self.assertRaises(RuntimeError, guard.validate_root, self.root / 'outside')
            base.parent.mkdir()
            base.symlink_to(self.root / 'tmp', target_is_directory=True)
            self.assertRaises(RuntimeError, guard.validate_root, base / 'source')

    def test_process_tree_includes_detached_descendants_and_excludes_unrelated(self):
        rows = {100: (1, 100, 1, 'parent'), 101: (100, 101, 2, 'child'),
                102: (101, 101, 3, 'grandchild'), 300: (1, 300, 20, 'unrelated')}
        owned = guard.OwnedProcesses([100])
        with mock.patch.object(guard, 'process_table', return_value=rows):
            self.assertEqual(set(owned.sample()), {100, 101, 102})
        rows = {101: (1, 101, 2, 'child'), 102: (1, 101, 3, 'grandchild'), 300: (1, 300, 20, 'unrelated')}
        with mock.patch.object(guard, 'process_table', return_value=rows):
            self.assertEqual(set(owned.sample()), {101, 102})

    def test_process_table_keeps_birth_identity_and_observed_nice(self):
        text = '100 1 100 8 15 S Fri Oct 9 12:34:56 2026\n101 100 100 4 15 Z Fri Oct 9 12:34:57 2026\n'
        with mock.patch.object(guard.subprocess, 'check_output', return_value=text):
            self.assertEqual(guard.process_table(), {100: (1, 100, 8192, 'Fri Oct 9 12:34:56 2026', 15)})

    def test_resource_report_measures_owned_processes_and_cleanup(self):
        report = self.root / 'tmp/resources.json'
        unrelated = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'], start_new_session=True)
        try:
            with mock.patch.object(guard, 'validate_root'), \
                 mock.patch.object(guard, 'free_ram', return_value=8 * guard.GIB), \
                 mock.patch.object(guard.shutil, 'disk_usage', return_value=SimpleNamespace(free=40 * guard.GIB)):
                self.assertEqual(guard.run(self.root, [sys.executable, '-c', 'import time; time.sleep(.6)'], report_file=report), 0)
            value = json.loads(report.read_text())
            self.assertGreater(value['peak_owned_rss_bytes'], 0)
            self.assertGreater(value['samples'], 0)
            expected_nice = min(20 if sys.platform == 'darwin' else 19,
                                os.getpriority(os.PRIO_PROCESS, 0) + 15)
            self.assertEqual(value['nice_max'], expected_nice)
            self.assertGreaterEqual(value['nice_min'], os.getpriority(os.PRIO_PROCESS, 0))
            self.assertEqual(value['min_free_ram_bytes'], 8 * guard.GIB)
            self.assertTrue(value['cleanup_complete'])
            self.assertEqual(value['exit_code'], 0)
            self.assertIsNone(unrelated.poll())
        finally:
            unrelated.terminate()
            unrelated.wait()

    def test_resource_report_preserves_violating_ram_sample(self):
        report = self.root / 'tmp/resources.json'
        with mock.patch.object(guard, 'validate_root'), \
             mock.patch.object(guard, 'free_ram', side_effect=[8 * guard.GIB, guard.MIN_RAM - 1]), \
             mock.patch.object(guard.shutil, 'disk_usage', return_value=SimpleNamespace(free=40 * guard.GIB)):
            with self.assertRaisesRegex(RuntimeError, 'resource stop'):
                guard.run(self.root, [sys.executable, '-c', 'import time; time.sleep(30)'], report_file=report)
        value = json.loads(report.read_text())
        self.assertEqual(value['min_free_ram_bytes'], guard.MIN_RAM - 1)
        self.assertEqual(value['samples'], 1)
        self.assertTrue(value['cleanup_complete'])
        self.assertEqual(value['exit_code'], 75)

    def test_resource_report_records_cleanup_failure_after_success(self):
        report = self.root / 'tmp/resources.json'
        with mock.patch.object(guard, 'validate_root'), \
             mock.patch.object(guard, 'free_ram', return_value=8 * guard.GIB), \
             mock.patch.object(guard.shutil, 'disk_usage', return_value=SimpleNamespace(free=40 * guard.GIB)), \
             mock.patch.object(guard.OwnedProcesses, 'terminate', side_effect=RuntimeError('cleanup failure')):
            with self.assertRaisesRegex(RuntimeError, 'cleanup failure'):
                guard.run(self.root, [sys.executable, '-c', 'pass'], report_file=report)
        value = json.loads(report.read_text())
        self.assertFalse(value['cleanup_complete'])
        self.assertEqual(value['exit_code'], 75)
        self.assertEqual(value['error'], 'cleanup failure')

    def test_reused_private_group_is_excluded_from_discovery_and_fallback(self):
        owned = guard.OwnedProcesses([100])
        original = {100: (1, 100, 1, 'parent'), 101: (100, 101, 2, 'original child')}
        with mock.patch.object(guard, 'process_table', return_value=original):
            self.assertEqual(set(owned.sample()), {100, 101})
        reused = {100: original[100], 101: (1, 101, 5, 'unrelated reused PID')}
        with mock.patch.object(guard, 'process_table', return_value=reused):
            self.assertEqual(set(owned.sample()), {100})
        self.assertNotIn(101, owned.groups)
        with mock.patch.object(guard, 'process_table', side_effect=subprocess.TimeoutExpired('/bin/ps', 0.01)), \
             mock.patch.object(guard.os, 'killpg') as groups, mock.patch.object(guard.os, 'kill') as pids:
            owned.terminate()
        self.assertEqual([call.args for call in groups.call_args_list],
                         [(100, signal.SIGTERM), (100, signal.SIGKILL)])
        pids.assert_not_called()

    def test_initial_resource_refusal_starts_nothing(self):
        owner = self.root / 'tmp/owner.json'
        with mock.patch.object(guard, 'validate_root'), \
             mock.patch.object(guard, 'check_resources', side_effect=RuntimeError('low RAM')), \
             mock.patch.object(guard.subprocess, 'Popen') as launch:
            self.assertRaises(RuntimeError, guard.run, self.root, ['unused'], owner_file=owner)
            launch.assert_not_called()
            self.assertFalse(owner.exists())

    def test_private_sccache_foreground_server_has_stable_socket_and_owned_cleanup(self):
        calls = []
        socket = self.root / 'tmp/remote-sccache.sock'
        def launch(command, **options):
            calls.append((command, options))
            if len(calls) == 1:
                socket.touch()
                return SimpleNamespace(pid=101, poll=lambda: None, wait=lambda: 0)
            return SimpleNamespace(pid=102, poll=lambda: 0, wait=lambda: 0)
        with mock.patch.object(guard, 'validate_root'), \
             mock.patch.object(guard, 'check_resources', return_value=(8 * guard.GIB, 40 * guard.GIB)), \
             mock.patch.dict(guard.os.environ, RUSTC_WRAPPER='lane-sccache', AETHER_COMPILE_GATE_HELD='1'), \
             mock.patch.object(guard.subprocess, 'Popen', side_effect=launch), \
             mock.patch.object(guard, 'reap_children') as reaping, \
             mock.patch.object(guard, 'OwnedProcesses') as ownership:
            ownership.return_value.sample.return_value = {}
            self.assertEqual(guard.run(self.root, ['test-command'], sccache=True), 0)
            ownership.return_value.terminate.assert_called_once()
            reaping.assert_called_once()
        self.assertEqual(calls[0][0], ['/usr/bin/nice', '-n', '15', 'lane-sccache'])
        self.assertEqual(calls[0][1]['env']['SCCACHE_START_SERVER'], '1')
        self.assertEqual(calls[0][1]['env']['SCCACHE_NO_DAEMON'], '1')
        self.assertEqual(calls[1][1]['env']['SCCACHE_SERVER_UDS'], str(socket))
        self.assertNotIn('AETHER_COMPILE_GATE_HELD', calls[1][1]['env'])
        self.assertFalse(socket.exists())

    def test_guard_terminates_owned_tree_on_resource_stop_and_preserves_other_process(self):
        unrelated = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'], start_new_session=True)
        pidfile = self.root / 'tmp/descendant'
        code = ('import subprocess,sys,time\n'
                'p=subprocess.Popen([sys.executable,"-c","import time; time.sleep(30)"],start_new_session=True)\n'
                'open(sys.argv[1],"w").write(str(p.pid))\n'
                'time.sleep(30)\n')
        calls = []
        original_popen = subprocess.Popen
        def popen(command, **options):
            calls.append((command, options))
            return original_popen(command, **options)
        def resources(_root, _rss=0, _report=None):
            if pidfile.exists():
                raise RuntimeError('simulated RAM drop')
            return 8 * guard.GIB, 40 * guard.GIB
        try:
            with mock.patch.object(guard, 'validate_root'), mock.patch.object(guard, 'check_resources', side_effect=resources), \
                 mock.patch.object(guard.subprocess, 'Popen', side_effect=popen):
                with self.assertRaisesRegex(RuntimeError, 'simulated RAM drop'):
                    guard.run(self.root, [sys.executable, '-c', code, str(pidfile)])
            self.assertIsNone(unrelated.poll())
            self.assertEqual(calls[0][0][:3], ['/usr/bin/nice', '-n', '15'])
            self.assertEqual(calls[0][1]['env']['AETHER_REMOTE_GUARD_ACTIVE'], '1')
            self.assertNotIn(int(pidfile.read_text()), guard.process_table())
        finally:
            unrelated.terminate()
            unrelated.wait()

    def test_guard_signal_cleans_process_and_owner_record(self):
        ready = self.root / 'tmp/ready'
        owner = self.root / 'tmp/owner.json'
        code = 'import pathlib,sys,time; pathlib.Path(sys.argv[1]).touch(); time.sleep(30)'
        def signal_when_ready():
            deadline = time.monotonic() + 5
            while not ready.exists() and time.monotonic() < deadline:
                time.sleep(0.02)
            if ready.exists():
                os.kill(os.getpid(), signal.SIGTERM)
        thread = threading.Thread(target=signal_when_ready)
        thread.start()
        try:
            with mock.patch.object(guard, 'validate_root'), \
                 mock.patch.object(guard, 'check_resources', return_value=(8 * guard.GIB, 40 * guard.GIB)):
                with self.assertRaises(SystemExit) as caught:
                    guard.run(self.root, [sys.executable, '-c', code, str(ready)], owner_file=owner)
            self.assertEqual(caught.exception.code, 143)
            self.assertFalse(owner.exists())
        finally:
            thread.join(timeout=5)

    def test_ps_timeout_still_kills_and_reaps_child_and_removes_owner(self):
        owner = self.root / 'tmp/owner.json'
        ready = self.root / 'tmp/ready'
        code = ('import pathlib,signal,subprocess,sys,time; '
                'signal.signal(signal.SIGTERM,signal.SIG_IGN); '
                'p=subprocess.Popen([sys.executable,"-c",'
                '"import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(30)"]); '
                'pathlib.Path(sys.argv[1]).write_text(str(p.pid)); time.sleep(30)')
        unrelated = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'], start_new_session=True)
        children = []
        original_popen = subprocess.Popen
        original_table = guard.process_table
        previous = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
        def launch(command, **options):
            child = original_popen(command, **options)
            if command[:3] == ['/usr/bin/nice', '-n', '15']:
                children.append(child)
            return child
        def table():
            if children:
                deadline = time.monotonic() + 5
                while not ready.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                raise subprocess.TimeoutExpired('/bin/ps', 0.01)
            return original_table()
        try:
            with mock.patch.object(guard, 'validate_root'), \
                 mock.patch.object(guard, 'check_resources', return_value=(8 * guard.GIB, 40 * guard.GIB)), \
                 mock.patch.object(guard.subprocess, 'Popen', side_effect=launch), \
                 mock.patch.object(guard, 'process_table', side_effect=table):
                with self.assertRaises(subprocess.TimeoutExpired):
                    guard.run(self.root, [sys.executable, '-c', code, str(ready)], owner_file=owner)
            self.assertTrue(ready.exists())
            self.assertEqual(len(children), 1)
            self.assertIsNotNone(children[0].returncode, 'owned child was not killed and reaped')
            with self.assertRaises(ChildProcessError):
                os.waitpid(children[0].pid, os.WNOHANG)
            self.assertFalse(owner.exists())
            self.assertIsNone(unrelated.poll())
            self.assertNotIn(int(ready.read_text()), guard.process_table())
            for sig, handler in previous.items():
                self.assertEqual(signal.getsignal(sig), handler)
        finally:
            for sig, handler in previous.items():
                signal.signal(sig, handler)
            for child in children:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGKILL)
                child.wait()
            unrelated.terminate()
            unrelated.wait()

    def test_ps_failure_cleans_private_server_socket_and_all_direct_children(self):
        owner = self.root / 'tmp/owner.json'
        ready = self.root / 'tmp/ready'
        socket = self.root / 'tmp/remote-sccache.sock'
        server = self.root / 'tmp/dummy-sccache'
        server.write_text('#!/usr/bin/env python3\nimport os,signal,socket,time\n'
                          'signal.signal(signal.SIGTERM,signal.SIG_IGN)\n'
                          'sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)\n'
                          'sock.bind(os.path.relpath(os.environ["SCCACHE_SERVER_UDS"]))\n'
                          'time.sleep(30)\n')
        server.chmod(0o755)
        children = []
        original_popen = subprocess.Popen
        original_table = guard.process_table
        previous = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
        def launch(command, **options):
            child = original_popen(command, **options)
            if command[:3] == ['/usr/bin/nice', '-n', '15']:
                children.append(child)
            return child
        def table():
            if len(children) == 2:
                deadline = time.monotonic() + 5
                while not ready.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                raise subprocess.TimeoutExpired('/bin/ps', 0.01)
            return original_table()
        code = 'import pathlib,sys,time; pathlib.Path(sys.argv[1]).touch(); time.sleep(30)'
        try:
            with mock.patch.object(guard, 'validate_root'), \
                 mock.patch.object(guard, 'check_resources', return_value=(8 * guard.GIB, 40 * guard.GIB)), \
                 mock.patch.dict(guard.os.environ, RUSTC_WRAPPER=str(server)), \
                 mock.patch.object(guard.subprocess, 'Popen', side_effect=launch), \
                 mock.patch.object(guard, 'process_table', side_effect=table):
                with self.assertRaises(subprocess.TimeoutExpired):
                    guard.run(self.root, [sys.executable, '-c', code, str(ready)], sccache=True, owner_file=owner)
            self.assertTrue(ready.exists())
            self.assertEqual(len(children), 2)
            for child in children:
                self.assertIsNotNone(child.returncode)
                with self.assertRaises(ChildProcessError):
                    os.waitpid(child.pid, os.WNOHANG)
            self.assertFalse(owner.exists())
            self.assertFalse(socket.exists())
            for sig, handler in previous.items():
                self.assertEqual(signal.getsignal(sig), handler)
        finally:
            for sig, handler in previous.items():
                signal.signal(sig, handler)
            for child in children:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGKILL)
                child.wait()


if __name__ == '__main__':
    unittest.main()
