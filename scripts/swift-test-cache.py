#!/usr/bin/env python3
"""Content-addressed Swift test builds; manifests compile at most four at once."""
import argparse
import concurrent.futures
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
CACHE = ROOT / 'tmp/swift-test-cache'


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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest')
    parser.add_argument('--output')
    parser.add_argument('args', nargs=argparse.REMAINDER)
    options = parser.parse_args()
    os.chdir(ROOT)
    CACHE.mkdir(parents=True, exist_ok=True)
    compiler = shutil.which('swiftc')
    if not compiler:
        parser.error('swiftc is unavailable')
    version = subprocess.check_output([compiler, '--version'], text=True, stderr=subprocess.STDOUT)
    if options.manifest:
        jobs = []
        for line in Path(options.manifest).read_text().splitlines():
            fields = line.rstrip('\t').split('\t')
            name, args = fields[0], fields[1:]
            job = dict(name=name, args=args, err=str(ROOT / ('tmp/sw-' + name + '.err')),
                       out=str(ROOT / ('tmp/sw-' + name + '.out')))
            if '--command' in args:
                split = args.index('--command')
                job.update(args=args[:split], command=args[split + 1:])
            Path(job['err']).write_text('')
            Path(job['out']).write_text('')
            jobs.append(job)
    else:
        if not options.output:
            parser.error('--output or --manifest is required')
        args = options.args[1:] if options.args[:1] == ['--'] else options.args
        jobs = [dict(name='fixture', args=args, output=options.output, err=options.output + '.err')]
    for job in jobs:
        job['cached'] = str(CACHE / fingerprint(compiler, version, job['args']))
    missing = [job for job in jobs if not Path(job['cached']).is_file() or not os.access(job['cached'], os.X_OK)]
    if missing and sys.platform == 'darwin' and os.environ.get('AETHER_COMPILE_GATE_HELD') != '1':
        # Gate shell execs this process and lives through all bounded child builds.
        os.execv('/bin/bash', ['bash', str(ROOT / 'scripts/compile-gate.sh'),
                              sys.executable, str(Path(__file__).resolve()), *sys.argv[1:]])
    workers = min(4, max(1, int(os.environ.get('AETHER_SWIFT_JOBS', '4'))))
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        compiled = list(pool.map(lambda job: build(job, compiler, version), jobs))
    if options.manifest:
        print(f'Swift tests: {len(missing)} builds, {len(jobs) - len(missing)} cached; compile jobs={workers}', file=sys.stderr, flush=True)
    failed = 0
    for job, success in zip(jobs, compiled):
        if success and options.output:
            shutil.copy2(job['cached'], options.output)
        elif success:
            env = os.environ.copy()
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
    return min(125, failed)


if __name__ == '__main__':
    sys.exit(main())
