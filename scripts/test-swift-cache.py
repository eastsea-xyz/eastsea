#!/usr/bin/env python3
"""Focused cache regressions without a real compiler or compile semaphore."""
import importlib.util
import os
from pathlib import Path
import tempfile
import sys
import subprocess

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
print('OK   Swift cache invalidation, reuse, failed publication, test registration and subsets')
