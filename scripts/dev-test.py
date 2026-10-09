#!/usr/bin/env python3
"""Run affected Rust/Swift tests locally, offloading only a verified queue timeout."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent


def changed_paths(base):
    spec = importlib.util.spec_from_file_location('affected_crates', ROOT / 'scripts/affected-crates.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.changed_paths(ROOT, base)


def capture(command, env):
    return subprocess.check_output(command, cwd=ROOT, env=env, text=True).splitlines()


def execute(command, env):
    """Stream output and retain remote timing records without a local gate."""
    records = []
    child = subprocess.Popen(command, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, text=True, start_new_session=True)
    completed = False

    def terminate(signum=signal.SIGTERM):
        try:
            os.killpg(child.pid, signum)
        except ProcessLookupError:
            pass
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        # Reap descendants even when the direct wrapper exited promptly on TERM.
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        child.wait()

    def stop(signum, _frame):
        # Unwind a possible Popen.wait() lock before the finally block reaps.
        raise SystemExit(128 + signum)

    previous = {sig: signal.signal(sig, stop) for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
    try:
        for line in child.stdout:
            print(line, end='', flush=True)
            if line.startswith('dev-test timing: '):
                records.append(json.loads(line.removeprefix('dev-test timing: ')))
        code = child.wait()
        completed = True
        return (code if code >= 0 else 128 - code), records
    finally:
        if not completed:
            terminate()
        child.stdout.close()
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def main():
    started = time.monotonic()
    parser = argparse.ArgumentParser(description=__doc__)
    location = parser.add_mutually_exclusive_group()
    location.add_argument('--remote', action='store_true', help='use guarded poc-m3; no local slot')
    location.add_argument('--local', action='store_true', help='wait up to 20 minutes locally; never offload')
    parser.add_argument('--base', default='HEAD', help='include committed changes since this ref (default: HEAD)')
    parser.add_argument('--changed-file', action='append', help='scope to an explicit changed path; repeatable')
    parser.add_argument('--rust-test', action='append', default=[], help='restrict Rust integration test binaries')
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    for name in args.rust_test:
        if not re.fullmatch(r'[A-Za-z0-9_-]+', name):
            parser.error('Rust test names must be simple target names')
    paths = sorted(set(args.changed_file) if args.changed_file else changed_paths(args.base))
    for name in paths:
        if Path(name).is_absolute() or '..' in Path(name).parts or '\n' in name:
            parser.error('changed paths must be repository-relative without traversal or newlines')
    scratch = ROOT / 'tmp'
    scratch.mkdir(exist_ok=True)
    # Records remain reviewable, while test caches live in their existing warm directories.
    directory = Path(tempfile.mkdtemp(prefix='dev-test-', dir=scratch))
    pathlist = directory / 'changed-paths'
    pathlist.write_text(''.join(path + '\n' for path in paths))
    env = dict(os.environ, TMPDIR=str(scratch))
    selection = json.loads('\n'.join(capture([sys.executable, str(ROOT / 'scripts/affected-crates.py'),
                                              '--base', args.base, '--paths-file', str(pathlist), '--json'], env))) if paths else dict(packages=[], tests={})
    packages = selection['packages']
    swift = capture(['bash', str(ROOT / 'scripts/test-swift-pure.sh'), '--list',
                     '--affected-file', str(pathlist)], env) if paths else []
    if not packages and not swift:
        print('No affected Rust packages or registered pure Swift tests.')
        return 0
    print('Affected Rust: ' + (', '.join(packages) or 'none'))
    print('Affected Swift: ' + (', '.join(swift) or 'none'))
    work = []
    if packages:
        # Cargo --test applies across all selected packages, so group targets only
        # when their selections match; untouched consumers keep the full crate gate.
        groups = {}
        for package in packages:
            targets = tuple(args.rust_test or selection['tests'].get(package, []))
            groups.setdefault(targets, []).append(package)
        for targets, group in groups.items():
            options = [arg for package in group for arg in ('-p', package)]
            options.extend(arg for name in targets for arg in ('--test', name))
            work.append(('rust', [str(ROOT / 'scripts/run-rust-tests.sh'), '--', *options],
                         [str(ROOT / 'scripts/remote-test.sh'), *options]))
    if swift:
        work.append(('swift', ['bash', str(ROOT / 'scripts/test-swift-pure.sh'), *swift],
                     [str(ROOT / 'scripts/remote-test.sh'),
                      *[arg for name in swift for arg in ('--swift', name)]]))
    if args.dry_run:
        for kind, local, remote in work:
            print(json.dumps(dict(kind=kind, command=remote if args.remote else local,
                                  offload_after_seconds=None if args.remote or args.local else 60)))
        return 0
    selection_seconds = time.monotonic() - started
    results = []
    offloaded = args.remote
    code = 0
    for index, (kind, local, remote) in enumerate(work):
        gate_file = directory / f'{index}-{kind}-gate.json'
        timing_file = directory / f'{index}-{kind}.json'
        worker_env = dict(env, AETHER_COMPILE_WAIT_SECONDS='1200' if args.local else '60',
                          AETHER_COMPILE_TIMING_FILE=str(gate_file),
                          AETHER_DEV_TIMING_FILE=str(timing_file))
        command_started = time.monotonic()
        code, streamed = execute(remote if offloaded else local, worker_env)
        gate = json.loads(gate_file.read_text()) if gate_file.exists() else {}
        worker = json.loads(timing_file.read_text()) if timing_file.exists() else None
        attempt = dict(kind=kind, location='poc-m3' if offloaded else 'local',
                       wall_seconds=time.monotonic() - command_started, exit_code=code,
                       timing=streamed if offloaded else worker)
        results.append(attempt)
        # A test failure returning 75 is not evidence of a queue timeout.
        if not args.local and not offloaded and code == 75 and gate.get('status') == 'queue_timeout':
            print('Local compile queue reached 60 seconds; offloading to poc-m3.', flush=True)
            offloaded = True
            remote_started = time.monotonic()
            code, streamed = execute(remote, worker_env)
            results.append(dict(kind=kind, location='poc-m3',
                                wall_seconds=time.monotonic() - remote_started,
                                exit_code=code, timing=streamed))
        if code:
            break
    record = dict(schema_version=1, wall_seconds=time.monotonic() - started,
                  selection_seconds=selection_seconds,
                  exit_code=code, changed_paths=paths, packages=packages,
                  swift_tests=swift, attempts=results)
    report = directory / 'timing.json'
    report.write_text(json.dumps(record, indent=2) + '\n')
    print(f'dev-test report: {report}')
    for result in results:
        print(f"{result['kind']} {result['location']}: {result['wall_seconds']:.2f}s wall, exit {result['exit_code']}")
    return code


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f'dev-test: {error}', file=sys.stderr)
        sys.exit(getattr(error, 'returncode', 2))
