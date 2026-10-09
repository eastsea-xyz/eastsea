#!/usr/bin/env python3
"""Content-addressed Swift test builds; manifests compile at most four at once."""
import argparse
import concurrent.futures
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import signal
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
CACHE = ROOT / 'tmp/swift-test-cache'

# These inputs define the table or the localization environment for the suite.
GLOBAL_INPUTS = {
    'scripts/test-swift-pure.sh', 'scripts/swift-test-cache.py',
    'scripts/test-swift-cache.py', 'scripts/wallet-l10n.py',
    'scripts/check-wallet-l10n.sh', 'scripts/check-wallet-screens-language.py',
    'scripts/wallet-l10n-allow.txt', 'apps/wallet/Sources/AppLanguage.swift',
    'apps/wallet/Tests/LocalizationTestSupport.swift',
}


def manifest_jobs(path):
    jobs = []
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        fields = line.rstrip('\t').split('\t')
        name, args = fields[0], fields[1:]
        job = dict(name=name, args=args, err=str(ROOT / ('tmp/sw-' + name + '.err')),
                   out=str(ROOT / ('tmp/sw-' + name + '.out')))
        if '--command' in args:
            split = args.index('--command')
            job.update(args=args[:split], command=args[split + 1:])
        jobs.append(job)
    return jobs


def affected_jobs(jobs, paths):
    selected = set()
    for value in paths:
        path = PurePosixPath(value)
        if path.is_absolute() or '..' in path.parts:
            raise ValueError(f'affected Swift path must be repository-relative: {value}')
        name = str(path)
        if name in GLOBAL_INPUTS or (name.startswith('apps/wallet/Resources/') and name.endswith('.xcstrings')):
            selected.update(job['name'] for job in jobs)
            continue
        matches = []
        for job in jobs:
            # Test directories may also hold runtime fixtures used by main.swift.
            test_dirs = [str(PurePosixPath(arg).parent) + '/' for arg in job['args']
                         if '/Tests/' in arg and arg.endswith('/main.swift')]
            if name in job['args'] or name in job.get('command', []) or any(name.startswith(directory) for directory in test_dirs):
                matches.append(job['name'])
        if matches:
            selected.update(matches)
        elif name.startswith('apps/wallet/Tests/') and name.endswith('.swift'):
            raise ValueError(f'wallet Swift test is not registered in scripts/test-swift-pure.sh: {name}')
        elif name.startswith('apps/wallet/Sources/'):
            print(f'No registered pure Swift test covers {name}; wallet UI/app integration requires the lead wallet gate.', file=sys.stderr)
    return [job for job in jobs if job['name'] in selected]


def read_timing(path):
    try:
        value = json.loads(Path(path).read_text()) if path else {}
        return value if isinstance(value, dict) else {}
    except (OSError, ValueError):
        return {}


def clear_timing(path):
    if path:
        try:
            Path(path).unlink(missing_ok=True)
        except OSError as error:
            print(f'cannot clear Swift timing file {path}: {error}', file=sys.stderr)


def write_timing(record):
    destination = os.environ.get('AETHER_DEV_TIMING_FILE')
    if not destination:
        return
    temporary = None
    try:
        Path(destination).parent.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(mode='w', prefix='swift-timing-', dir=ROOT / 'tmp', delete=False) as stream:
            temporary = Path(stream.name)
            json.dump(record, stream)
            stream.write('\n')
        os.replace(temporary, destination)
    except OSError as error:
        # Optional instrumentation must never turn a failed build into success.
        print(f'cannot write Swift timing file {destination}: {error}', file=sys.stderr)
    finally:
        if temporary is not None:
            try:
                temporary.unlink(missing_ok=True)
            except OSError:
                pass


def remote_guarded():
    # Keep the bypass identical to the gate's root/user/guard contract.
    spec = importlib.util.spec_from_file_location('aether_compile_gate', ROOT / 'scripts/compile-gate.py')
    gate = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gate)
    return gate.remote_guarded(ROOT)


def gated_run(command, env):
    child = subprocess.Popen(command, env=env)
    previous = {}
    signal_code = None

    def stop(signum, _frame):
        nonlocal signal_code
        signal_code = 128 + signum
        if child.poll() is None:
            child.send_signal(signum)

    try:
        for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            previous[signum] = signal.signal(signum, stop)
        code = child.wait()
        return signal_code or (code if code >= 0 else 128 - code)
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)


def fingerprint(compiler, version, args):
    digest = hashlib.sha256()
    for value in [str(ROOT), compiler, version, *args]:
        digest.update(value.encode() + b'\0')
    # Changes to orchestration or test flags must never leave stale binaries.
    for name in ['scripts/swift-test-cache.py', 'scripts/test-swift-pure.sh',
                 'scripts/test-update-daemon.sh', 'scripts/test-update-listener.sh']:
        digest.update(name.encode() + b'\0' + (ROOT / name).read_bytes())
    for arg in args:
        if arg.endswith('.swift'):
            digest.update(arg.encode() + b'\0' + (ROOT / arg).read_bytes())
    for name in ['SDKROOT', 'DEVELOPER_DIR', 'TOOLCHAINS', 'MACOSX_DEPLOYMENT_TARGET']:
        digest.update((name + '=' + os.environ.get(name, '')).encode() + b'\0')
    return digest.hexdigest()


def build(job, compiler, version=None):
    target = Path(job['cached'])
    if target.is_file() and os.access(target, os.X_OK):
        return True
    fd, temporary = tempfile.mkstemp(prefix='building-', dir=CACHE)
    os.close(fd)
    try:
        with open(job['err'], 'w') as err:
            result = subprocess.run([compiler, *job['args'], '-o', temporary], stderr=err)
        if result.returncode:
            return False
        if version is not None and fingerprint(compiler, version, job['args']) != target.name:
            with open(job['err'], 'a') as err:
                err.write('Swift test inputs changed during compilation; rerun the test.\n')
            return False
        Path(temporary).chmod(0o755)
        os.replace(temporary, target)
        return True
    finally:
        Path(temporary).unlink(missing_ok=True)


def main():
    started = time.monotonic()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest')
    parser.add_argument('--output')
    parser.add_argument('--affected-file', help='newline-separated repository-relative changed paths')
    parser.add_argument('--list', action='store_true', help='print selected job names without preparation or compilation')
    parser.add_argument('--build-only', action='store_true', help=argparse.SUPPRESS)
    parser.add_argument('args', nargs=argparse.REMAINDER)
    options = parser.parse_args()
    os.chdir(ROOT)
    if options.manifest:
        jobs = manifest_jobs(options.manifest)
        if options.affected_file:
            try:
                paths = [line.strip() for line in Path(options.affected_file).read_text().splitlines() if line.strip()]
                jobs = affected_jobs(jobs, paths)
            except (OSError, ValueError) as error:
                parser.error(str(error))
    else:
        if options.affected_file or options.list:
            parser.error('--affected-file and --list require --manifest')
        if not options.output:
            parser.error('--output or --manifest is required')
        args = options.args[1:] if options.args[:1] == ['--'] else options.args
        jobs = [dict(name='fixture', args=args, output=options.output, err=options.output + '.err')]
    if options.list:
        for job in jobs:
            print(job['name'])
        return 0
    timing = dict(schema_version=1, kind='swift', wall_seconds=0, queue_seconds=0,
                  compile_seconds=0, run_seconds=0, selected_tests=[job['name'] for job in jobs],
                  builds=0, cache_hits=0, exit_code=0)
    outer = os.environ.get('AETHER_COMPILE_GATE_HELD') != '1'
    if outer:
        clear_timing(os.environ.get('AETHER_DEV_TIMING_FILE'))
        clear_timing(os.environ.get('AETHER_COMPILE_TIMING_FILE'))

    def finish(code):
        timing.update(wall_seconds=time.monotonic() - started, exit_code=code)
        write_timing(timing)
        return code

    if not jobs:
        print('No affected pure Swift tests.', file=sys.stderr)
        return finish(0)
    CACHE.mkdir(parents=True, exist_ok=True)
    compiler = shutil.which('swiftc')
    if not compiler:
        parser.error('swiftc is unavailable')
    version = subprocess.check_output([compiler, '--version'], text=True, stderr=subprocess.STDOUT)
    for job in jobs:
        Path(job['err']).write_text('')
        if options.manifest:
            Path(job['out']).write_text('')
    for job in jobs:
        job['cached'] = str(CACHE / fingerprint(compiler, version, job['args']))
    missing = [job for job in jobs if not Path(job['cached']).is_file() or not os.access(job['cached'], os.X_OK)]
    gated_build = False
    if missing and sys.platform == 'darwin' and outer and not remote_guarded():
        # The inner stage owns the compile slot only through the bounded batch.
        # Tests run below after the gate has released it. The inner runner
        # rechecks source fingerprints after waiting for the semaphore.
        with tempfile.TemporaryDirectory(prefix='swift-build-stage-', dir=ROOT / 'tmp') as directory:
            stage_env = os.environ.copy()
            stage_env.setdefault('AETHER_DEV_TIMING_FILE', str(Path(directory) / 'build.json'))
            stage_env.setdefault('AETHER_COMPILE_TIMING_FILE', str(Path(directory) / 'gate.json'))
            code = gated_run(['/bin/bash', str(ROOT / 'scripts/compile-gate.sh'),
                              sys.executable, str(Path(__file__).resolve()), '--build-only', *sys.argv[1:]], stage_env)
            inner = read_timing(stage_env['AETHER_DEV_TIMING_FILE'])
            if inner.get('kind') == 'swift' and inner.get('selected_tests') == timing['selected_tests']:
                timing.update(inner)
            gate = read_timing(stage_env['AETHER_COMPILE_TIMING_FILE'])
            timing['queue_seconds'] = gate.get('queue_seconds', 0)
        if code:
            return finish(code)
        version = subprocess.check_output([compiler, '--version'], text=True, stderr=subprocess.STDOUT)
        for job in jobs:
            job['cached'] = str(CACHE / fingerprint(compiler, version, job['args']))
        missing = [job for job in jobs if not Path(job['cached']).is_file() or not os.access(job['cached'], os.X_OK)]
        if missing:
            print('Swift test inputs changed after compilation; rerun the test.', file=sys.stderr)
            return finish(1)
        gated_build = True
    if options.manifest:
        (ROOT / 'tmp/swift-module-cache').mkdir(parents=True, exist_ok=True)
    workers = min(4, max(1, int(os.environ.get('AETHER_SWIFT_JOBS', '4'))))
    compiling = time.monotonic()
    if missing:
        with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
            compiled = list(pool.map(lambda job: build(job, compiler, version), jobs))
        timing['compile_seconds'] = time.monotonic() - compiling
    else:
        compiled = [True] * len(jobs)
    if not gated_build:
        timing.update(builds=len(missing), cache_hits=len(jobs) - len(missing))
    if options.build_only:
        for job, success in zip(jobs, compiled):
            if not success:
                print('FAIL ' + job['name'] + ' :: ' + Path(job['err']).read_text()[:160].replace('\n', ' '), file=sys.stderr)
        return finish(min(125, sum(not success for success in compiled)))
    if options.manifest:
        bundle = ROOT / 'tmp/wallet-languages/WalletLocalizations.bundle'
        code = subprocess.run(['/usr/bin/python3', str(ROOT / 'scripts/wallet-l10n.py'), 'prepare-tests', '--out', str(bundle)]).returncode
        if code:
            return finish(code)
        print(f"Swift tests: {timing['builds']} builds, {timing['cache_hits']} cached; compile jobs={workers}", file=sys.stderr, flush=True)
    failed = 0
    running = time.monotonic()
    for job, success in zip(jobs, compiled):
        if success and options.output:
            shutil.copy2(job['cached'], options.output)
        elif success:
            env = os.environ.copy()
            env.pop('AETHER_DEV_TIMING_FILE', None)
            env.pop('AETHER_COMPILE_TIMING_FILE', None)
            env.update(AETHER_AGENT_TEST_TMP=str(ROOT / 'tmp'),
                       WALLET_TEST_BUNDLE=str(ROOT / 'tmp/wallet-languages/WalletLocalizations.bundle'))
            with open(job['out'], 'w') as out:
                success = subprocess.run(job.get('command', [job['cached']]), env=env, stdout=out, stderr=subprocess.STDOUT).returncode == 0
        if options.manifest:
            if success:
                print('OK   ' + job['name'], flush=True)
            else:
                errors = Path(job['err']).read_text()[:160].replace('\n', ' ')
                output = ' '.join(Path(job['out']).read_text().splitlines()[-2:])
                print('FAIL ' + job['name'] + ' :: ' + errors + ' ' + output, flush=True)
        if options.output and not success:
            sys.stderr.write(Path(job['err']).read_text())
        failed += not success
    timing['run_seconds'] = time.monotonic() - running
    print(f"Swift timing: compile={timing['compile_seconds']:.2f}s run={timing['run_seconds']:.2f}s", file=sys.stderr, flush=True)
    return finish(min(125, failed))


if __name__ == '__main__':
    sys.exit(main())
