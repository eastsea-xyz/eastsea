#!/usr/bin/env bash
# Runtime-only RAM-backed temporary storage; never falls back to a disk.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$ROOT/tmp"
export AETHER_TEST_ROOT="$ROOT"
exec python3 - "$@" <<'PY'
import os, pathlib, plistlib, re, shutil, signal, subprocess, sys, tempfile, time

ROOT = pathlib.Path(os.environ['AETHER_TEST_ROOT'])

def run(*args):
    child = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             start_new_session=True)
    try:
        output, _ = child.communicate(timeout=30)
        if child.returncode:
            raise subprocess.CalledProcessError(child.returncode, args, output=output)
        return output
    except BaseException:
        # Utilities have their own group so timeout/signals cannot leave a mount
        # process alive after the helper starts detaching its owned device.
        try:
            os.killpg(child.pid, signal.SIGTERM)
            child.wait(timeout=1)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            pass
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        child.wait()
        raise
    finally:
        if child.stdout:
            child.stdout.close()

def ram_backed(path):
    if sys.platform == 'darwin':
        info = plistlib.loads(run('/usr/sbin/diskutil', 'info', '-plist', str(path)))
        device = info.get('DeviceIdentifier')
        images = plistlib.loads(run('/usr/bin/hdiutil', 'info', '-plist')).get('images', [])
        return any(str(image.get('image-path', '')).startswith('ram://') and
                   any(entity.get('dev-entry') == '/dev/' + str(device)
                       for entity in image.get('system-entities', [])) for image in images)
    if sys.platform.startswith('linux'):
        # Resolve symlinks and select the longest enclosing mount from mountinfo.
        target = str(pathlib.Path(path).resolve())
        mounts = []
        for line in pathlib.Path('/proc/self/mountinfo').read_text().splitlines():
            left, right = line.split(' - ', 1)
            mount = left.split()[4].replace('\\040', ' ')
            if target == mount or target.startswith(mount.rstrip('/') + '/'):
                mounts.append((len(mount), right.split()[0]))
        return bool(mounts) and max(mounts)[1] == 'tmpfs'
    return False

def mac_size():
    stats = run('/usr/bin/vm_stat').decode()
    page = int(re.search(r'page size of (\d+)', stats)[1])
    pages = sum(int(re.search(r'Pages ' + key + r':\s+(\d+)', stats)[1])
                for key in ('free', 'speculative'))
    # Only a quarter of immediately available memory; no inactive-page assumptions.
    size = min(512, pages * page // (4 * 1024 * 1024))
    if size < 128:
        raise RuntimeError('insufficient free memory for a 128 MiB test RAM disk')
    return size

def main(args):
    if not args:
        raise RuntimeError('usage: scripts/test-tmpdir.sh COMMAND [ARGS...]')
    owned = None
    device = None
    child = None
    def interrupted(signum, frame):
        raise SystemExit(128 + signum)
    for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(sig, interrupted)
    try:
        explicit = os.environ.get('TMPDIR')
        if explicit:
            if not pathlib.Path(explicit).is_dir() or not ram_backed(explicit):
                raise RuntimeError('explicit TMPDIR must exist on a verified RAM-backed filesystem')
            base = explicit
        elif sys.platform == 'darwin':
            owned = pathlib.Path(tempfile.mkdtemp(prefix='test-ram-', dir=ROOT / 'tmp'))
            size = mac_size()
            try:
                output = run('/usr/bin/hdiutil', 'attach', '-nomount', 'ram://' + str(size * 2048)).decode()
            except subprocess.TimeoutExpired as error:
                # hdiutil may publish its device before attachment stalls.
                partial = (error.output or b'').decode(errors='replace')
                match = re.search(r'/dev/disk\d+', partial)
                if match:
                    device = match[0]
                raise
            device = re.search(r'/dev/disk\d+', output)[0]
            run('/sbin/newfs_hfs', '-v', 'AetherTests', device)
            run('/usr/sbin/diskutil', 'mount', '-mountPoint', str(owned), device)
            if not ram_backed(owned):
                raise RuntimeError('mounted filesystem could not be verified as RAM-backed')
            base = str(owned)
            print(f'test RAM disk: {size} MiB at {base}', file=sys.stderr)
        elif sys.platform.startswith('linux') and ram_backed('/dev/shm'):
            base = '/dev/shm'
        else:
            raise RuntimeError('RAM-backed test storage unavailable; disk fallback is disabled')
        work = pathlib.Path(tempfile.mkdtemp(prefix='aether-tests-', dir=base))
        env = dict(os.environ, TMPDIR=str(work), TMP=str(work), TEMP=str(work),
                   AETHER_TEST_TMP_ACTIVE="1")
        try:
            child = subprocess.Popen(args, env=env, start_new_session=True)
            return child.wait()
        finally:
            if child:
                # Clean descendants even if their direct parent already exited.
                for sig in (signal.SIGTERM, signal.SIGKILL):
                    try:
                        os.killpg(child.pid, sig)
                    except ProcessLookupError:
                        break
                    if sig == signal.SIGTERM:
                        time.sleep(0.3)
                child.wait()
            shutil.rmtree(work)
    finally:
        # A second terminal signal must not interrupt disk cleanup.
        for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            signal.signal(sig, signal.SIG_IGN)
        if device:
            # Only detach the device returned by this invocation's ram:// attach.
            try:
                run('/usr/bin/hdiutil', 'detach', device)
            except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
                run('/usr/bin/hdiutil', 'detach', '-force', device)
        if owned:
            owned.rmdir()

if __name__ == '__main__':
    try:
        sys.exit(main(sys.argv[1:]))
    except (RuntimeError, subprocess.SubprocessError, OSError) as error:
        print('test-tmpdir: ' + str(error), file=sys.stderr)
        sys.exit(1)
PY
