#!/usr/bin/env python3
"""Routing tests use fixture commands; no network or compiler is started."""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]


class DevTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dev-loop-fixture-', dir=ROOT / 'tmp')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        scripts = self.root / 'scripts'
        scripts.mkdir()
        (self.root / 'tmp').mkdir()
        shutil.copy2(ROOT / 'scripts/dev-test.py', scripts)
        self.log = self.root / 'calls.jsonl'
        self.env = dict(os.environ, FIXTURE_LOG=str(self.log), FIXTURE_PACKAGES='aether-node',
                        FIXTURE_SWIFT='', PYTHONDONTWRITEBYTECODE='1')
        selector = scripts / 'affected-crates.py'
        selector.write_text("import json,os\ndef changed_paths(root, base):\n    assert base == 'HEAD'\n    return {'crates/node/src/lib.rs'}\nif __name__ == '__main__':\n    print(json.dumps({'packages':os.environ['FIXTURE_PACKAGES'].splitlines(),'tests':json.loads(os.environ.get('FIXTURE_TARGETS','{}'))}))\n")
        swift = scripts / 'test-swift-pure.sh'
        swift.write_text('''#!/bin/bash
if [[ "$1" == --list ]]; then
  [ -z "$FIXTURE_SWIFT" ] || printf '%s\\n' "$FIXTURE_SWIFT"
  exit 0
fi
exec python3 "$(dirname "$0")/fixture.py" swift "$@"
''')
        fixture = scripts / 'fixture.py'
        fixture.write_text('''import json,os,pathlib,sys
kind = sys.argv[1]
with open(os.environ['FIXTURE_LOG'],'a') as log:
    log.write(json.dumps({'kind':kind,'args':sys.argv[2:],
                          'queue_limit':os.environ.get('AETHER_COMPILE_WAIT_SECONDS')})+'\\n')
if kind != 'remote':
    code = int(os.environ.get('FIXTURE_LOCAL_EXIT','0'))
    if os.environ.get('FIXTURE_QUEUE_KIND') == kind:
        pathlib.Path(os.environ['AETHER_COMPILE_TIMING_FILE']).write_text(json.dumps({'status':'queue_timeout','queue_seconds':60}))
        code = 75
    pathlib.Path(os.environ['AETHER_DEV_TIMING_FILE']).write_text(json.dumps({'kind':kind,'exit_code':code,'compile_seconds':0,'run_seconds':.01,'queue_seconds':60 if code==75 else 0}))
else:
    code = int(os.environ.get('FIXTURE_REMOTE_EXIT','0'))
    print('dev-test timing: '+json.dumps({'kind':'remote','queue_seconds':0,'compile_seconds':.2,'run_seconds':.01,'exit_code':code}))
sys.exit(code)
''')
        for name, kind in [('run-rust-tests.sh', 'rust'), ('remote-test.sh', 'remote')]:
            path = scripts / name
            path.write_text('#!/bin/bash\nexec python3 "$(dirname "$0")/fixture.py" ' + kind + ' "$@"\n')
            path.chmod(0o755)

    def run_loop(self, *args):
        return subprocess.run(['python3', str(self.root / 'scripts/dev-test.py'), *args],
                              env=self.env, text=True, capture_output=True, timeout=20)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def test_remote_never_invokes_local_worker_and_preserves_target(self):
        result = self.run_loop('--remote', '--changed-file', 'crates/node/src/lib.rs', '--rust-test', 'wake_signal')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([c['kind'] for c in self.calls()], ['remote'])
        self.assertEqual(self.calls()[0]['args'], ['-p', 'aether-node', '--test', 'wake_signal'])

    def test_only_verified_queue_timeout_offloads_and_remaining_swift_stays_remote(self):
        self.env.update(FIXTURE_QUEUE_KIND='rust', FIXTURE_SWIFT='earnings')
        result = self.run_loop('--changed-file', 'crates/node/src/lib.rs')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([c['kind'] for c in self.calls()], ['rust', 'remote', 'remote'])
        self.assertEqual(self.calls()[0]['queue_limit'], '60')
        report = next((self.root / 'tmp').glob('dev-test-*/timing.json'))
        attempts = json.loads(report.read_text())['attempts']
        self.assertEqual(attempts[0]['timing']['queue_seconds'], 60)
        self.assertEqual(attempts[1]['location'], 'poc-m3')

    def test_runtime_exit75_is_not_offload_evidence(self):
        self.env['FIXTURE_LOCAL_EXIT'] = '75'
        result = self.run_loop('--changed-file', 'crates/node/src/lib.rs')
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertEqual([c['kind'] for c in self.calls()], ['rust'])

    def test_explicit_local_waits_twenty_minutes_and_never_offloads(self):
        self.env['FIXTURE_QUEUE_KIND'] = 'rust'
        result = self.run_loop('--local', '--changed-file', 'crates/node/tests/rpc_alias.rs')
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertEqual([c['kind'] for c in self.calls()], ['rust'])
        self.assertEqual(self.calls()[0]['queue_limit'], '1200')
        self.assertNotEqual(self.run_loop('--local', '--remote').returncode, 0)

    def test_local_failure_and_remote_failure_propagate(self):
        self.env['FIXTURE_LOCAL_EXIT'] = '1'
        self.assertEqual(self.run_loop('--changed-file', 'crates/node/src/lib.rs').returncode, 1)
        self.env['FIXTURE_REMOTE_EXIT'] = '78'
        self.assertEqual(self.run_loop('--remote', '--changed-file', 'crates/node/src/lib.rs').returncode, 78)

    def test_default_selects_uncommitted_changes_since_head(self):
        result = self.run_loop()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([c['kind'] for c in self.calls()], ['rust'])

    def test_integration_edit_runs_only_affected_target(self):
        self.env['FIXTURE_TARGETS'] = '{"aether-node":["rpc_alias"]}'
        result = self.run_loop('--changed-file', 'crates/node/tests/rpc_alias.rs')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls()[0]['args'], ['--', '-p', 'aether-node', '--test', 'rpc_alias'])

    def test_swift_only_and_empty_selection(self):
        self.env.update(FIXTURE_PACKAGES='', FIXTURE_SWIFT='tx-status-text')
        result = self.run_loop('--changed-file', 'apps/wallet/Sources/TxStatusText.swift')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([c['kind'] for c in self.calls()], ['swift'])
        self.log.unlink()
        self.env['FIXTURE_SWIFT'] = ''
        result = self.run_loop('--changed-file', 'docs/ops/dev-loop.md')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls(), [])

    def test_dry_run_and_bad_paths_start_no_worker(self):
        self.assertEqual(self.run_loop('--dry-run', '--changed-file', 'crates/node/src/lib.rs').returncode, 0)
        for path in ('/etc/passwd', '../outside', 'file\nsecond'):
            self.assertEqual(self.run_loop('--changed-file', path).returncode, 2)
        self.assertEqual(self.calls(), [])

    def test_signal_and_malformed_timing_reap_owned_descendants(self):
        for malformed in (False, True):
            with self.subTest(malformed=malformed):
                pidfile = self.root / 'owned-child'
                driver = self.root / 'driver.py'
                worker = self.root / 'owned.py'
                child_code = 'import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(60)'
                worker.write_text('import pathlib,subprocess,sys,time\n'
                                  f'child=subprocess.Popen([sys.executable,"-c",{child_code!r}])\n'
                                  f'pathlib.Path({str(pidfile)!r}).write_text(str(child.pid))\n'
                                  'time.sleep(.2)\n'
                                  + ('print("dev-test timing: malformed",flush=True)\n' if malformed else '')
                                  + 'time.sleep(60)\n')
                driver.write_text('import importlib.util,os\n'
                                  f'spec=importlib.util.spec_from_file_location("loop",{str(self.root/"scripts/dev-test.py")!r})\n'
                                  'loop=importlib.util.module_from_spec(spec);spec.loader.exec_module(loop)\n'
                                  f'loop.execute(["python3",{str(worker)!r}],dict(os.environ))\n')
                process = subprocess.Popen(['python3', str(driver)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                owned_pid = None
                try:
                    deadline = time.monotonic() + 10
                    while not pidfile.exists() and time.monotonic() < deadline:
                        time.sleep(.02)
                    self.assertTrue(pidfile.exists())
                    owned_pid = int(pidfile.read_text())
                    if not malformed:
                        time.sleep(.3)
                        process.send_signal(signal.SIGTERM)
                    process.communicate(timeout=15)
                    self.assertNotEqual(process.returncode, 0)
                    # A reaped group may leave a briefly unreaped grandchild zombie.
                    state = subprocess.run(['ps', '-p', str(owned_pid), '-o', 'stat='], text=True, capture_output=True).stdout.strip()
                    self.assertTrue(not state or state.startswith('Z'), state)
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.communicate()
                    if owned_pid:
                        try:
                            os.kill(owned_pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                    pidfile.unlink(missing_ok=True)


if __name__ == '__main__':
    unittest.main()
