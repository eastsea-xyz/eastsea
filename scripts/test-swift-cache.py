#!/usr/bin/env python3
"""Focused cache regressions without a real compiler or compile semaphore."""
import importlib.util
import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
import sys
import subprocess
import shutil

sys.dont_write_bytecode = True

root = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('swift_cache', root / 'scripts/swift-test-cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
with tempfile.TemporaryDirectory(prefix='swift-cache-check-', dir=root / 'tmp') as directory:
    work = Path(directory)
    cache.CACHE = work
    source = work / 'main.swift'
    source.write_text('print("first")')
    args = ['-Onone', str(source)]
    original = cache.fingerprint('/compiler/one', 'Swift 1', args)
    assert cache.fingerprint('/compiler/one', 'Swift 1', args) == original
    assert cache.fingerprint('/compiler/two', 'Swift 1', args) != original
    assert cache.fingerprint('/compiler/one', 'Swift 2', args) != original
    assert cache.fingerprint('/compiler/one', 'Swift 1', ['-O', str(source)]) != original
    source.write_text('print("second")')
    assert cache.fingerprint('/compiler/one', 'Swift 1', args) != original
    previous_sdk = os.environ.get('SDKROOT')
    os.environ['SDKROOT'] = str(work / 'different-sdk')
    assert cache.fingerprint('/compiler/one', 'Swift 1', args) != original
    if previous_sdk is None:
        os.environ.pop('SDKROOT', None)
    else:
        os.environ['SDKROOT'] = previous_sdk
    cache.ROOT = work
    (work / 'scripts').mkdir()
    scripts = ['swift-test-cache.py', 'test-swift-pure.sh', 'test-update-daemon.sh', 'test-update-listener.sh']
    for script in scripts:
        (work / 'scripts' / script).write_text('original')
    original = cache.fingerprint('/compiler/one', 'Swift 1', args)
    for script in scripts:
        path = work / 'scripts' / script
        path.write_text('changed')
        assert cache.fingerprint('/compiler/one', 'Swift 1', args) != original
        path.write_text('original')
    compiler = work / 'compiler'
    compiler.write_text('#!/bin/bash\necho build >> "' + str(work / 'calls') + '"\nwhile [ "$1" != -o ]; do shift; done\nshift\nprintf "#!/bin/bash\\nexit 0\\n" > "$1"\nchmod +x "$1"\n')
    compiler.chmod(0o755)
    job = dict(cached=str(work / 'binary'), args=args, err=str(work / 'error'))
    assert cache.build(job, str(compiler))
    assert cache.build(job, str(compiler))
    assert (work / 'calls').read_text().splitlines() == ['build'], 'warm build must not invoke compiler'
    compiler.write_text(compiler.read_text() + 'echo changed >> "' + str(source) + '"\n')
    job['cached'] = str(work / cache.fingerprint(str(compiler), 'Swift 1', args))
    assert not cache.build(job, str(compiler), 'Swift 1')
    assert not Path(job['cached']).exists(), 'changing inputs during build must not publish a cache entry'
    compiler.write_text('#!/bin/bash\necho broken >&2\nexit 1\n')
    job['cached'] = str(work / 'failed-binary')
    assert not cache.build(job, str(compiler))
    assert not Path(job['cached']).exists(), 'failed build must not publish a cache entry'
    assert (work / 'error').read_text().strip() == 'broken'
    # Inspect the actual shell runner's generated manifest without compiling.
    binary = work / 'bin'
    binary.mkdir()
    spy = binary / 'python3'
    spy.write_text('#!/bin/bash\nif [ "$1" = scripts/swift-test-cache.py ]; then cp "$3" "$SWIFT_MANIFEST_CHECK"; else exec "' + sys.executable + '" "$@"; fi\n')
    spy.chmod(0o755)
    manifest = work / 'manifest'
    env = os.environ.copy()
    env.update(PATH=str(binary) + os.pathsep + env['PATH'], SWIFT_MANIFEST_CHECK=str(manifest))
    for selected in [[], ['earnings'], ['update-listener'], ['update-daemon-tree']]:
        if selected and selected[0].startswith('update-') and sys.platform != 'darwin':
            continue
        subprocess.run(['bash', str(root / 'scripts/test-swift-pure.sh'), *selected],
                       env=env, cwd=root, check=True, stdout=subprocess.DEVNULL)
        jobs = [line.rstrip('\t').split('\t') for line in manifest.read_text().splitlines()]
        if not selected:
            full_manifest = manifest.read_text()
            assert len(jobs) == (52 if sys.platform == 'darwin' else 50)
            migration = next(job for job in jobs if job[0] == 'rename-migration')
            assert migration[1:4] == ['-O', '-assert-config', 'Debug']
        else:
            assert len(jobs) == 1
            if selected[0].startswith('update-'):
                assert jobs[0][0] == 'update-listener'
                assert jobs[0][-3:] == ['--command', 'bash', 'scripts/test-update-listener.sh']
            else:
                assert jobs[0][0] == 'earnings'
    manifest.write_text(full_manifest)
    registered = cache.manifest_jobs(manifest)
    names = {job['name'] for job in registered}

    def affected(*paths):
        return {job['name'] for job in cache.affected_jobs(registered, paths)}

    assert affected('apps/wallet/Sources/BalanceHistory.swift') == {'balance-history'}
    assert affected('apps/wallet/Tests/earnings/main.swift') == {'earnings'}
    assert affected('apps/wallet/Tests/earnings/runtime-fixture.json') == {'earnings'}
    assert affected('apps/wallet/Sources/EarningsModel.swift') == {
        'account-removal', 'assets', 'balance-sources', 'earnings', 'earnings-export',
        'fee-confirm', 'reward-status', 'proving-badge', 'token-guard', 'token-icon', 'token-send',
    }, 'a shared source must select every registered consumer'
    assert affected('apps/wallet/Sources/BalanceHistory.swift', 'apps/wallet/Sources/TxTrack.swift') == {'balance-history', 'tx-track'}
    for path in [*cache.GLOBAL_INPUTS, 'apps/wallet/Resources/Localizable.xcstrings']:
        assert affected(path) == names, f'global Swift input must select all jobs: {path}'
    if sys.platform == 'darwin':
        assert affected('apps/wallet/Sources/NodeReleaseIdentity.swift') == {'update-daemon', 'update-listener'}
        assert affected('apps/wallet/Tests/update-daemon-tree/main.swift') == {'update-listener'}
        assert affected('scripts/test-update-listener.sh') == {'update-listener'}
    warning = io.StringIO()
    with contextlib.redirect_stderr(warning):
        assert not affected('apps/wallet/Sources/ContentView.swift')
    assert 'No registered pure Swift test covers' in warning.getvalue()
    assert 'lead wallet gate' in warning.getvalue()
    assert not affected('docs/ops/dev-loop.md')
    for path in ['apps/wallet/Tests/unregistered/main.swift', '/absolute.swift', '../outside.swift']:
        try:
            affected(path)
        except ValueError:
            pass
        else:
            raise AssertionError(f'unsupported affected path must fail clearly: {path}')

    # Run the real shell/cache orchestration in an isolated tree. All compilers,
    # localizations and semaphore scripts below are fixtures; no host slot is used.
    fixture = work / 'runner'
    (fixture / 'scripts').mkdir(parents=True)
    (fixture / 'tmp').mkdir()
    for script in scripts + ['compile-gate.sh', 'compile-gate.py']:
        shutil.copy2(root / 'scripts' / script, fixture / 'scripts' / script)
    for path in ['apps/wallet/Sources/EarningsModel.swift', 'apps/wallet/Sources/AppLanguage.swift',
                 'apps/wallet/Tests/LocalizationTestSupport.swift', 'apps/wallet/Tests/earnings/main.swift']:
        target = fixture / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text('// fixture Swift input\n')
    (fixture / 'scripts/wallet-l10n.py').write_text(
        'import os\nfrom pathlib import Path\n'
        'with Path(os.environ["SWIFT_FIXTURE_PREPARE"]).open("a") as stream: stream.write("prepared\\n")\n')
    fixture_bin = fixture / 'bin'
    fixture_bin.mkdir()
    swiftc = fixture_bin / 'swiftc'
    swiftc.write_text('#!' + sys.executable + '\n' + '''import os, sys, time
from pathlib import Path
with Path(os.environ['SWIFT_FIXTURE_CALLS']).open('a') as stream:
    stream.write('version\\n' if sys.argv[1:] == ['--version'] else 'compile\\n')
if sys.argv[1:] == ['--version']:
    print('fixture Swift compiler')
else:
    time.sleep(0.03)
    destination = Path(sys.argv[sys.argv.index('-o') + 1])
    destination.write_text('#!/bin/bash\\n'
        'if [ "${AETHER_COMPILE_GATE_HELD:-0}" = 1 ]; then echo "test still holds compile slot"; exit 7; fi\\n'
        'if [ -f "$SWIFT_FIXTURE_OWNER" ]; then read -r owner < "$SWIFT_FIXTURE_OWNER"; '
        'if kill -0 "$owner" 2>/dev/null; then echo "compile owner still alive"; exit 8; fi; fi\\n'
        'printf "ran\\\\n" >> "$SWIFT_FIXTURE_RUNS"\\n/bin/sleep 0.03\\nexit 0\\n')
    destination.chmod(0o755)
''')
    swiftc.chmod(0o755)
    calls = fixture / 'tmp/calls'
    runs = fixture / 'tmp/runs'
    prepared = fixture / 'tmp/prepared'
    timing_file = fixture / 'tmp/timing.json'
    gate_timing_file = fixture / 'tmp/gate-timing.json'
    fixture_gate = fixture / 'bin/gate'
    fixture_gate.write_text('#!/bin/bash\nexec /bin/sleep 30\n')
    fixture_gate.chmod(0o755)
    fixture_env = os.environ.copy()
    fixture_env.update(PATH=str(fixture_bin) + os.pathsep + fixture_env['PATH'], TMPDIR=str(fixture / 'tmp'),
                       AETHER_COMPILE_GATE=str(fixture_gate), AETHER_COMPILE_WAIT_SECONDS='0.25',
                       AETHER_DEV_TIMING_FILE=str(timing_file), AETHER_COMPILE_TIMING_FILE=str(gate_timing_file),
                       AETHER_REMOTE_GUARD_ACTIVE='1', AETHER_REMOTE_TEST_ROOT=str(fixture.resolve()),
                       SWIFT_FIXTURE_OWNER=str(fixture / 'tmp/owner'),
                       SWIFT_FIXTURE_CALLS=str(calls), SWIFT_FIXTURE_RUNS=str(runs), SWIFT_FIXTURE_PREPARE=str(prepared))
    fixture_env.pop('AETHER_COMPILE_GATE_HELD', None)

    def run_fixture(*arguments, env=None):
        return subprocess.run(['bash', str(fixture / 'scripts/test-swift-pure.sh'), *arguments],
                              cwd=fixture, env=env or fixture_env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)

    listing = run_fixture('--list')
    assert listing.returncode == 0, listing.stderr
    assert set(listing.stdout.splitlines()) == names
    assert not calls.exists() and not prepared.exists(), '--list must not probe a compiler or prepare localizations'
    changed = fixture / 'tmp/changed'
    changed.write_text('apps/wallet/Sources/EarningsModel.swift\n')
    listing = run_fixture('--list', '--affected-file', str(changed))
    assert listing.returncode == 0, listing.stderr
    assert set(listing.stdout.splitlines()) == affected('apps/wallet/Sources/EarningsModel.swift')
    assert not calls.exists() and not prepared.exists()
    changed.write_text('apps/wallet/Tests/unregistered/main.swift\n')
    rejected = run_fixture('--list', '--affected-file', str(changed))
    assert rejected.returncode == 2 and 'not registered' in rejected.stderr
    changed.write_text('apps/wallet/Sources/ContentView.swift\n')
    skipped = run_fixture('--affected-file', str(changed))
    assert skipped.returncode == 0 and not skipped.stdout.strip()
    assert 'No registered pure Swift test covers' in skipped.stderr
    assert not calls.exists() and not prepared.exists()
    changed.write_text('apps/wallet/Tests/earnings/main.swift\n')
    arguments = ['--affected-file', str(changed)]
    if sys.platform == 'darwin':
        blocked = run_fixture(*arguments)
        assert blocked.returncode == 75, blocked.stderr
        assert 'compile slot wait exceeded 0.25s' in blocked.stderr
        assert calls.read_text().splitlines() == ['version'] and not runs.exists()
        assert not list((fixture / 'tmp/swift-test-cache').iterdir()), 'queue timeout must not publish a binary'
        timing = json.loads(timing_file.read_text())
        assert timing['exit_code'] == 75 and timing['queue_seconds'] > 0
        assert timing['compile_seconds'] == timing['run_seconds'] == timing['builds'] == 0
        # A timing write failure must leave the original queue timeout visible.
        bad_destination = fixture / 'tmp/not-a-directory'
        bad_destination.write_text('fixture')
        bad_env = dict(fixture_env, AETHER_DEV_TIMING_FILE=str(bad_destination / 'timing.json'))
        blocked = run_fixture(*arguments, env=bad_env)
        assert blocked.returncode == 75 and 'cannot write Swift timing file' in blocked.stderr
    fixture_gate.write_text('#!/bin/bash\nprintf "%s\\n" "$PPID" > "$SWIFT_FIXTURE_OWNER"\nexit 0\n')
    cold = run_fixture(*arguments)
    assert cold.returncode == 0, cold.stderr
    assert cold.stdout.strip() == 'OK   earnings'
    timing = json.loads(timing_file.read_text())
    assert timing['builds'] == 1 and timing['cache_hits'] == 0
    assert timing['compile_seconds'] > 0 and timing['run_seconds'] > 0
    assert timing['wall_seconds'] >= timing['queue_seconds'] + timing['compile_seconds'] + timing['run_seconds']
    assert calls.read_text().splitlines().count('compile') == 1
    gate_timing_file.write_text('{"queue_seconds":999}')
    warm = run_fixture(*arguments)
    assert warm.returncode == 0, warm.stderr
    timing = json.loads(timing_file.read_text())
    assert timing['queue_seconds'] == timing['compile_seconds'] == timing['builds'] == 0
    assert timing['cache_hits'] == 1 and not gate_timing_file.exists(), 'warm cache must clear stale queue metrics'
    assert calls.read_text().splitlines().count('compile') == 1
    assert runs.read_text().splitlines() == ['ran', 'ran'], 'cached builds must still execute tests on every run'
    source = fixture / 'apps/wallet/Sources/EarningsModel.swift'
    source.write_text(source.read_text() + '// one-line fixture edit\n')
    edited = run_fixture(*arguments)
    assert edited.returncode == 0, edited.stderr
    assert json.loads(timing_file.read_text())['builds'] == 1
    assert calls.read_text().splitlines().count('compile') == 2
print('OK   Swift cache invalidation, reuse, failed publication, 52 registrations, source selection, queue timeout, build-only slot ownership and timing')
