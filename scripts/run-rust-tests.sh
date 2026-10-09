#!/usr/bin/env bash
# Compile under the shared gate, then execute prebuilt tests outside it.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
export PATH="$HOME/.cargo/bin:$PATH"
export AETHER_TEST_ROOT="$ROOT"
exec python3 - "$@" <<'PY'
import os
import fcntl
import hashlib
import json
import signal
from contextlib import contextmanager
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

root = Path(os.environ['AETHER_TEST_ROOT'])
started = time.monotonic()
build_elapsed = run_elapsed = 0.0
exit_code = 0
cache_hit = False
gate_file = os.environ.get('AETHER_COMPILE_TIMING_FILE')
if gate_file:
    Path(gate_file).resolve().relative_to((root / 'tmp').resolve())
    Path(gate_file).unlink(missing_ok=True)


def timing():
    filename = os.environ.get('AETHER_DEV_TIMING_FILE')
    if not filename:
        return
    path = Path(filename).resolve()
    path.relative_to((root / 'tmp').resolve())
    path.parent.mkdir(parents=True, exist_ok=True)
    gate = json.loads(Path(gate_file).read_text()) if gate_file and Path(gate_file).exists() else {}
    queue = gate.get('queue_seconds', 0.0)
    cleanup = gate.get('cleanup_seconds', 0.0)
    compilation = gate.get('compile_seconds', build_elapsed)
    wall = time.monotonic() - started
    record = dict(schema_version=1, kind='rust', wall_seconds=wall,
                  queue_seconds=queue, compile_seconds=compilation,
                  run_seconds=run_elapsed, cleanup_seconds=cleanup,
                  overhead_seconds=max(0, wall - queue - compilation - run_elapsed - cleanup),
                  cache_hit=cache_hit, exit_code=exit_code, gate_status=gate.get('status'))
    staged = path.with_name(path.name + f'.{os.getpid()}.new')
    staged.write_text(json.dumps(record) + '\n')
    os.replace(staged, path)

class Uncacheable(ValueError):
    pass


def input_key(build, env):
    """Content evidence, never timestamps or Git state, determines build reuse."""
    digest = hashlib.sha256()
    digest.update(json.dumps(['rust-test-cache-v1', build, str(root), sys.platform]).encode())
    for command in ([env.get('RUSTC', 'rustc'), '-vV'], ['cargo', '-V'], ['cargo-nextest', '--version']):
        digest.update(subprocess.check_output(command, cwd=root, env=env))
    prefixes = ('CARGO_', 'RUST', 'SCCACHE_', 'CC_', 'CXX_', 'CFLAGS_', 'CXXFLAGS_', 'AR_', 'AETHER_BUILD_')
    names = {'CC', 'CXX', 'CFLAGS', 'CXXFLAGS', 'AR', 'SDKROOT', 'MACOSX_DEPLOYMENT_TARGET', 'PATH'}
    digest.update(json.dumps(sorted((key, value) for key, value in env.items()
                                   if key in names or key.startswith(prefixes))).encode())
    cargo_home = Path(env.get('CARGO_HOME', str(Path.home() / '.cargo')))
    for name in ('config', 'config.toml'):
        config = cargo_home / name
        if config.is_symlink():
            raise Uncacheable(f'symlink Cargo configuration: {config}')
        if config.is_file():
            digest.update(('user-cargo/' + name).encode() + b'\0' + config.read_bytes() + b'\0')
    paths = []
    for name in ('crates', 'legacy', 'vendor', '.cargo'):
        directory = root / name
        if directory.is_symlink():
            raise Uncacheable(f'symlink input: {directory}')
        if not directory.exists():
            continue
        for current, directories, files in os.walk(directory, followlinks=False):
            directories[:] = sorted(d for d in directories if d not in ('target', 'tmp', '.git'))
            for entry in directories + files:
                if (Path(current) / entry).is_symlink():
                    raise Uncacheable(f'symlink input: {Path(current) / entry}')
            paths.extend(Path(current) / name for name in sorted(files))
    paths.extend(root / name for name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain', 'rust-toolchain.toml',
                                         'build.rs', 'scripts/run-rust-tests.sh', 'scripts/dev-cargo.sh',
                                         'scripts/build-cache.py') if (root / name).exists())
    for path in sorted(paths):
        if path.is_symlink() or not path.is_file():
            raise Uncacheable(f'nonregular input: {path}')
        before = path.stat()
        digest.update(str(path.relative_to(root)).encode() + b'\0')
        with path.open('rb') as source:
            while data := source.read(1024 * 1024):
                digest.update(data)
        after = path.stat()
        if (before.st_mtime_ns, before.st_size, before.st_ino) != (after.st_mtime_ns, after.st_size, after.st_ino):
            raise Uncacheable(f'input changed while hashing: {path}')
        digest.update(b'\0')
    return digest.hexdigest()


def artifact_stamps(binaries):
    listing = json.loads(binaries.read_text())
    target = Path(listing['rust-build-meta']['target-directory'])
    if not target.is_absolute() or not target.is_dir():
        raise ValueError('test target no longer exists')
    rows = listing['rust-binaries']
    if not isinstance(rows, dict):
        raise ValueError('unsupported nextest binary metadata')
    paths = [Path(row['binary-path']) for row in rows.values()]
    # Non-test executables may be invoked by integration tests via CARGO_BIN_EXE.
    for rows in listing['rust-build-meta'].get('non-test-binaries', {}).values():
        paths.extend(Path(row['path']) for row in rows)
    stamps = {}
    for path in paths:
        if not path.is_absolute():
            path = target / path
        path.resolve().relative_to(target.resolve())
        if not path.is_file():
            raise ValueError(f'test artifact no longer exists: {path}')
        stat = path.stat()
        stamps[str(path)] = [stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns]
    return target, stamps


@contextmanager
def target_lease(target, env):
    cache = Path(env.get('AETHER_BUILD_CACHE_ROOT', '/Volumes/workspace/build-cache/targets')).expanduser()
    cache.mkdir(parents=True, exist_ok=True)
    with (cache / '.prune-lock').open('a+') as pruning:
        fcntl.flock(pruning, fcntl.LOCK_SH)
        # Do not resurrect targets deleted by pruning.
        if not target.is_dir():
            raise ValueError('test target was pruned')
        lease = (target / '.lease').open('a+')
        fcntl.flock(lease, fcntl.LOCK_SH)
        (target / '.last-used').touch()
        fcntl.flock(pruning, fcntl.LOCK_UN)
    try:
        yield lease.fileno()
    finally:
        (target / '.last-used').touch()
        lease.close()


def run_runtime(command, env, lease_fd):
    child = subprocess.Popen(command, cwd=root, env=env, pass_fds=(lease_fd,), start_new_session=True)
    stopping = False
    def stop(signum, frame):
        nonlocal stopping
        global exit_code
        stopping = True
        exit_code = 128 + signum
        raise SystemExit(128 + signum)
    previous = {sig: signal.signal(sig, stop) for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
    try:
        code = child.wait()
        if code:
            raise subprocess.CalledProcessError(code if code > 0 else 128 - code, command)
    finally:
        if stopping:
            # Signal handlers must unwind wait() before waiting again: Popen's
            # waitpid lock is not reentrant.
            try:
                os.killpg(child.pid, signal.SIGTERM)
                child.wait(timeout=5)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                pass
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
        for sig, handler in previous.items():
            signal.signal(sig, handler)

args = sys.argv[1:]
ram = False
if args and args[0] == '--ram':
    ram = True
    args.pop(0)
if args and args[0] == '--':
    args.pop(0)
if args == ['--help']:
    print('Usage: scripts/run-rust-tests.sh [--ram] -- -p CRATE [NEXTEST_ARGS...]')
    sys.exit(0)

# Build options cannot be combined with --binaries-metadata at execution time.
build_values = {'-p', '--package', '--bin', '--example', '--test', '--bench', '-F', '--features', '--target'}
build_flags = {'--lib', '--bins', '--examples', '--tests', '--benches', '--all-targets',
               '--all-features', '--no-default-features', '--locked', '--offline', '--frozen'}
run_values = {'-E', '--filterset', '--test-threads', '--jobs', '-j', '--run-ignored', '--partition',
              '--failure-output', '--success-output', '--status-level', '--final-status-level',
              '--no-tests', '--max-fail', '--color'}
run_flags = {'--no-capture', '--fail-fast', '--no-fail-fast', '--no-pager', '--ignore-default-filter',
             '--hide-progress-bar', '-v', '--verbose'}
build, runtime, packages = [], [], []
i = 0
try:
    while i < len(args):
        arg = args[i]
        if arg == '--':
            runtime.extend(args[i:])
            break
        key, equals, value = arg.partition('=')
        if key in build_values | run_values:
            if not equals:
                i += 1
                if i >= len(args):
                    raise ValueError(f'{key} requires a value')
                value = args[i]
            destination = build if key in build_values else runtime
            destination.extend((key, value))
            if key in ('-p', '--package'):
                if not value.startswith('aether-') or value in ('aether-ffi', 'aether-prover') or '*' in value or '?' in value:
                    raise ValueError('only explicit safe aether workspace packages are supported; guest/staticlib builds require the lead')
                packages.append(value)
        elif arg in build_flags:
            if arg != '--locked':
                build.append(arg)
        elif arg in run_flags:
            runtime.append(arg)
        elif arg.startswith('-'):
            raise ValueError(f'unsupported option: {arg}; release, guest, build reuse and alternate profiles are disabled')
        else:
            runtime.append(arg)
        i += 1
    if not packages:
        raise ValueError('supply at least one explicit -p CRATE; unrestricted workspace builds are disabled')
    if not shutil.which('cargo-nextest'):
        raise ValueError('cargo-nextest is required; follow docs/ops/dev-loop.md setup steps')
    (root / 'tmp').mkdir(exist_ok=True)
    env = dict(os.environ, TMPDIR=str(root / 'tmp'))
    cache_root = root / 'tmp/rust-test-cache'
    cache_root.mkdir(exist_ok=True)
    try:
        key = input_key(build, env)
    except Uncacheable as error:
        print(f'build metadata cache disabled: {error}', file=sys.stderr)
        key = None
    with tempfile.TemporaryDirectory(prefix='rust-tests-', dir=root / 'tmp') as temporary:
        directory = Path(temporary)
        with (cache_root / ((key or 'uncached') + '.lock')).open('a+') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            cached = cache_root / key if key else directory
            binaries = cached / 'binaries.json'
            metadata = cached / 'cargo.json'
            valid = False
            if key and binaries.is_file() and metadata.is_file():
                try:
                    target, stamps = artifact_stamps(binaries)
                    valid = (stamps == json.loads((cached / 'artifacts.json').read_text())
                             and input_key(build, env) == key)
                except (OSError, ValueError, KeyError, TypeError):
                    pass
            if not valid:
                binaries = directory / 'binaries.json'
                metadata = directory / 'cargo.json'
                build_started = time.monotonic()
                with binaries.open('w') as output:
                    try:
                        subprocess.run([str(root / 'scripts/dev-cargo.sh'), 'nextest', 'list',
                                        '--list-type', 'binaries-only', '--message-format', 'json', '--locked', *build],
                                       cwd=root, env=env, stdout=output, check=True)
                    finally:
                        build_elapsed = time.monotonic() - build_started
                # Graph retrieval only; every compilation goes through dev-cargo.
                with metadata.open('w') as output:
                    subprocess.run(['cargo', 'metadata', '--format-version', '1', '--locked'],
                                   cwd=root, env=env, stdout=output, check=True)
                target, stamps = artifact_stamps(binaries)
                try:
                    publish = key and input_key(build, env) == key
                except Uncacheable:
                    publish = False
                if publish:
                    staged = Path(tempfile.mkdtemp(prefix='publish-', dir=cache_root))
                    try:
                        shutil.copy2(binaries, staged / 'binaries.json')
                        shutil.copy2(metadata, staged / 'cargo.json')
                        (staged / 'artifacts.json').write_text(json.dumps(stamps))
                        if cached.exists():
                            shutil.rmtree(cached)
                        os.replace(staged, cached)
                    finally:
                        if staged.exists():
                            shutil.rmtree(staged)
                    binaries, metadata = cached / 'binaries.json', cached / 'cargo.json'
            else:
                cache_hit = True
                print('reuse verified test binaries (no compile gate)', file=sys.stderr)
            # Hold the cache lock and target lease throughout execution. Validation
            # inside the pruning lock closes the lookup/deletion race.
            with target_lease(target, env) as lease_fd:
                if artifact_stamps(binaries)[1] != stamps:
                    raise ValueError('test artifacts changed before execution; rerun to rebuild')
                command = ['cargo', 'nextest', 'run', '--binaries-metadata', str(binaries),
                           '--cargo-metadata', str(metadata), *runtime]
                run_started = time.monotonic()
                if ram:
                    env.pop('TMPDIR', None)
                    env.pop('TMP', None)
                    env.pop('TEMP', None)
                    if os.environ.get('AETHER_TEST_TMPDIR'):
                        env['TMPDIR'] = os.environ['AETHER_TEST_TMPDIR']
                    command.insert(0, str(root / 'scripts/test-tmpdir.sh'))
                    try:
                        run_runtime(command, env, lease_fd)
                    finally:
                        run_elapsed = time.monotonic() - run_started
                else:
                    with tempfile.TemporaryDirectory(prefix='runtime-', dir=directory) as work:
                        env.update(TMPDIR=work, TMP=work, TEMP=work)
                        try:
                            run_runtime(command, env, lease_fd)
                        finally:
                            run_elapsed = time.monotonic() - run_started
except (ValueError, OSError, subprocess.CalledProcessError) as error:
    print(f'run-rust-tests: {error}', file=sys.stderr)
    exit_code = getattr(error, 'returncode', 2)
    sys.exit(exit_code)
finally:
    timing()
PY
