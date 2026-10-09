#!/bin/bash
# Exercise the shipped DMG in disposable macOS guests, never on this Mac.
# One-time founder setup: docs/ops/release-smoke.md.
# All Tart storage is deliberately fixed to the external workspace volume.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
VM_HOME=/Volumes/workspace/build-cache/vm
BASE=eastsea-smoke-base
PROFILE_TIMEOUT=1080
TART_BIN=""

find_tart() {
  TART_BIN="$(command -v tart 2>/dev/null || true)"
  if [ -z "$TART_BIN" ] && [ -x "$VM_HOME/tools/tart.app/Contents/MacOS/tart" ]; then
    TART_BIN="$VM_HOME/tools/tart.app/Contents/MacOS/tart"
  fi
  [ -n "$TART_BIN" ] && [ -x "$TART_BIN" ]
}

# Presence only: a configured but unsafe cache must not qualify for a skip.
# This does not invoke Tart or change files, including when setup is absent.
if [ "${1:-}" = --check-setup ]; then
  [ $# -eq 1 ] || { echo "usage: $0 --check-setup" >&2; exit 2; }
  if find_tart && [ -f "$VM_HOME/vms/$BASE/config.json" ] && [ -f "$VM_HOME/vms/$BASE/disk.img" ]; then
    exit 0
  fi
  exit 78
fi

DRY_RUN=0
if [ "${1:-}" = --dry-run ]; then DRY_RUN=1; shift; fi
if [ $# -ne 1 ] || [ "${1:-}" = --help ] || [[ "${1:-}" = --* ]]; then
  echo "usage: $0 [--dry-run] <dmg> | --check-setup" >&2
  exit 2
fi
DMG=$1

# An absent dist/ is normal during release planning. No setup, artifact, or
# filesystem probes precede this path, and DRY-RUN must never claim PASS.
if [ "$DRY_RUN" -eq 1 ]; then
  printf 'DRY-RUN release-vm-smoke: DMG=%q\n' "$DMG"
  echo "DRY-RUN fixed Tart storage: $VM_HOME; local base: $BASE"
  echo "DRY-RUN clone two disposable VMs sequentially: empty profile, then large-log (64 MiB node.log)."
  echo "DRY-RUN guest changes: install /Applications/EastSea.app; set fresh smoke-user node preferences; launch shipped app."
  echo "DRY-RUN observe each first launch for 600 seconds against testnet 7780 / guest RPC 18545; quit, relaunch, observe 60 seconds."
  echo "DRY-RUN require stable app PID, bundled child node, no new/changed EastSea or aether crash reports, advancing head within 12 blocks of https://rpc.eastsea.xyz."
  echo "DRY-RUN retain evidence under $ROOT/tmp/vm-smoke.<id>; stop and delete only disposable clones."
  exit 0
fi

fail() { echo "FAIL release-vm-smoke: $*" >&2; exit 1; }
[ "$(uname -s)" = Darwin ] || fail "macOS is required"
[ "$(/usr/bin/sw_vers -productVersion | cut -d. -f1)" -ge 14 ] || fail "Tart smoke requires macOS 14 or newer"
[ -z "${TART_HOME:-}" ] || [ "$TART_HOME" = "$VM_HOME" ] || fail "TART_HOME cannot override $VM_HOME"
find_tart || fail "Tart is not installed; use the exact founder setup commands in docs/ops/release-smoke.md"
[ -x /usr/bin/python3 ] || fail "Python 3 is required; see docs/ops/release-smoke.md"
[ -f "$DMG" ] || fail "DMG not found: $DMG"
[ -f "$VM_HOME/vms/$BASE/config.json" ] && [ -f "$VM_HOME/vms/$BASE/disk.img" ] \
  || fail "local base $BASE is absent; setup must create it without an automatic image pull"

# diskutil also rejects a plain /Volumes/workspace directory on the internal
# disk. Check every existing component, including the base's storage files.
/usr/bin/python3 - "$ROOT" "$VM_HOME" "$BASE" <<'PY_STORAGE' || fail "unsafe workspace or VM storage"
import os, pathlib, plistlib, subprocess, sys
root, cache, base = map(pathlib.Path, sys.argv[1:])
volume = pathlib.Path('/Volumes/workspace')
def no_links(path):
    for component in [path, *path.parents]:
        if component.is_symlink():
            raise RuntimeError('symlink is forbidden in storage path: ' + str(component))
no_links(volume)
info = plistlib.loads(subprocess.check_output(['/usr/sbin/diskutil', 'info', '-plist', str(volume)]))
internal = info.get('Internal', info.get('DeviceInternal'))
if info.get('MountPoint') != str(volume) or internal is not False or not os.path.ismount(volume):
    raise RuntimeError('/Volumes/workspace must be a mounted external volume (Internal=false)')
for path in [root, root / 'tmp', cache, cache / 'vms', cache / 'cache', cache / 'tmp',
             cache / 'vms' / base, cache / 'vms' / base / 'config.json', cache / 'vms' / base / 'disk.img']:
    no_links(path)
    path.resolve().relative_to(volume)
    existing = path
    while not existing.exists():
        existing = existing.parent
    if existing.stat().st_dev != volume.stat().st_dev:
        raise RuntimeError('storage leaves the mounted external volume: ' + str(path))
for path in (cache / 'vms' / base).rglob('*'):
    no_links(path)
    if path.stat().st_dev != volume.stat().st_dev:
        raise RuntimeError('base storage leaves the external volume: ' + str(path))
PY_STORAGE

export TART_HOME="$VM_HOME"
export TART_NO_AUTO_PRUNE=1
mkdir -p "$ROOT/tmp"
export TMPDIR="$ROOT/tmp"
RUN_DIR="$(mktemp -d "$TMPDIR/vm-smoke.XXXXXXXX")"
RUN_ID="$(basename "$RUN_DIR")-$$"
mkdir "$RUN_DIR/dmg"
chmod 755 "$RUN_DIR/dmg"
cp "$DMG" "$RUN_DIR/dmg/EastSea.dmg"
chmod 644 "$RUN_DIR/dmg/EastSea.dmg"
DMG_HASH="$(/usr/bin/shasum -a 256 "$RUN_DIR/dmg/EastSea.dmg" | awk '{print $1}')"
echo "release-vm-smoke artifact: sha256=$DMG_HASH"
echo "release-vm-smoke evidence: $RUN_DIR"

CLONES=()
RUN_PID=""
ACTIVE_VM=""
CLEANING=0
owned_runner() {
  local parent command_line
  parent="$(/bin/ps -p "$RUN_PID" -o ppid= 2>/dev/null | tr -d '[:space:]')"
  command_line="$(/bin/ps -ww -p "$RUN_PID" -o command= 2>/dev/null || true)"
  [ "$parent" = "$$" ] && [[ "$command_line" = *" run "*" $ACTIVE_VM" ]]
}
cleanup() {
  [ "$CLEANING" -eq 0 ] || return 1
  local deferred_signal=0
  # Complete teardown even if another signal arrives while the stop command
  # is running. The requested interruption still makes the gate fail.
  trap 'deferred_signal=130' INT
  trap 'deferred_signal=143' TERM
  CLEANING=1
  local bad=0 vm
  if [ -n "$RUN_PID" ]; then
    echo "release-vm-smoke change: stop disposable VM $ACTIVE_VM"
    "$TART_BIN" stop --timeout 10 "$ACTIVE_VM" >> "$RUN_DIR/cleanup.log" 2>&1 || true
    # Stop.swift falls back to SIGKILL. If the CLI cannot stop it, terminate
    # only our verified child runner; its Virtualization VM dies with it.
    local deadline=$((SECONDS + 20))
    while kill -0 "$RUN_PID" 2>/dev/null && [ "$SECONDS" -lt "$deadline" ]; do sleep 1; done
    if kill -0 "$RUN_PID" 2>/dev/null && owned_runner; then
      echo "release-vm-smoke change: stop fallback SIGTERM for owned Tart runner pid=$RUN_PID VM=$ACTIVE_VM"
      kill -TERM "$RUN_PID" 2>/dev/null || true
      deadline=$((SECONDS + 5))
      while kill -0 "$RUN_PID" 2>/dev/null && [ "$SECONDS" -lt "$deadline" ]; do sleep 1; done
      if kill -0 "$RUN_PID" 2>/dev/null && owned_runner; then
        echo "release-vm-smoke change: stop fallback SIGKILL for owned Tart runner pid=$RUN_PID VM=$ACTIVE_VM"
        kill -KILL "$RUN_PID" 2>/dev/null || true
        deadline=$((SECONDS + 10))
        while kill -0 "$RUN_PID" 2>/dev/null && [ "$SECONDS" -lt "$deadline" ]; do sleep 1; done
      fi
    fi
    if kill -0 "$RUN_PID" 2>/dev/null; then
      echo "FAIL release-vm-smoke: Tart runner still alive; retaining $ACTIVE_VM disk" >&2
      bad=1
    else
      wait "$RUN_PID" 2>/dev/null || true
      RUN_PID=""
      ACTIVE_VM=""
    fi
  fi
  for vm in "${CLONES[@]:-}"; do
    [ -n "$vm" ] || continue
    if [ -n "$RUN_PID" ] && [ "$vm" = "$ACTIVE_VM" ]; then continue; fi
    if [ -d "$VM_HOME/vms/$vm" ] && [ ! -L "$VM_HOME/vms/$vm" ]; then
      echo "release-vm-smoke change: delete disposable VM $vm"
      "$TART_BIN" delete "$vm" >> "$RUN_DIR/cleanup.log" 2>&1 || bad=1
    fi
  done
  CLEANING=0
  trap 'exit 130' INT
  trap 'exit 143' TERM
  [ "$deferred_signal" -eq 0 ] || bad=1
  return "$bad"
}
on_exit() {
  local status=$?
  trap - EXIT
  trap '' INT TERM
  cleanup || status=1
  [ "$status" -eq 0 ] || echo "FAIL release-vm-smoke: evidence retained at $RUN_DIR" >&2
  exit "$status"
}
trap on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

write_guest() {
  local profile=$1 directory=$2
  mkdir -p "$directory/output"
  chmod 755 "$directory"
  # VirtioFS preserves UIDs. Only this bounded output directory is writable
  # by smoke, which need not have the same UID as the host release operator.
  chmod 777 "$directory/output"
  /usr/bin/python3 - "$directory/config.json" "$RUN_ID" "$profile" "$DMG_HASH" <<'PY_CONFIG'
import json, sys
with open(sys.argv[1], 'w') as out:
    json.dump({'run_id': sys.argv[2], 'profile': sys.argv[3], 'dmg_sha256': sys.argv[4]}, out)
PY_CONFIG
  cat > "$directory/guest-runner.sh" <<'GUEST_RUNNER'
#!/bin/bash
set -euo pipefail
RUN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
OUTPUT=/Volumes/'My Shared Files'/release-smoke-output
umask 022
share_deadline=$((SECONDS + 90))
while [ ! -d "$OUTPUT" ]; do
  [ "$SECONDS" -lt "$share_deadline" ] || { echo 'FAIL: guest output share missing'; exit 1; }
  sleep 1
done
mkdir "$OUTPUT/guest-started" 2>/dev/null || exit 0
exec >> "$OUTPUT/guest.log" 2>&1
finish_runner() {
  local guest_exit=$?
  trap - EXIT INT TERM
  printf '{"schema":1,"exit_status":%d}\n' "$guest_exit" > "$OUTPUT/exit-status.json.part"
  mv "$OUTPUT/exit-status.json.part" "$OUTPUT/exit-status.json"
  exit "$guest_exit"
}
trap finish_runner EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
if [ ! -x /usr/bin/python3 ] || ! /usr/bin/xcode-select -p >/dev/null 2>&1; then
  echo 'FAIL: guest Python/Command Line Tools missing; see founder setup.'
  printf '%s\n' '{"schema":1,"result":"FAIL","reason":"guest Python/Command Line Tools missing"}' > "$OUTPUT/result.json.part"
  mv "$OUTPUT/result.json.part" "$OUTPUT/result.json"
  exit 1
fi
/usr/bin/python3 "$RUN_DIR/guest-smoke.py" "$RUN_DIR/config.json"
GUEST_RUNNER
  cat > "$directory/guest-smoke.py" <<'GUEST_PYTHON'
import ctypes
import datetime
import hashlib
import json
import os
import pathlib
import plistlib
import re
import shlex
import shutil
import subprocess
import sys
import time
import traceback

FIRST_SECONDS = 600
RELAUNCH_SECONDS = 60
SAMPLE_SECONDS = 5
HEAD_TOLERANCE = 12
CHAIN_ID = 7780
SEEDED_BYTES = 64 * 1024 * 1024
LOCAL_RPC = 'http://127.0.0.1:18545'
REFERENCE_RPC = 'https://rpc.eastsea.xyz'
BUNDLE_ID = 'com.pipln.eastsea'
TEAM_ID = '45WU468FZE'
APP = pathlib.Path('/Applications/EastSea.app')
RUN = pathlib.Path(sys.argv[1]).parent
OUTPUT = pathlib.Path('/Volumes/My Shared Files/release-smoke-output')
CONFIG = json.loads(pathlib.Path(sys.argv[1]).read_text())
HOME = pathlib.Path.home()
NODE = HOME / 'Library/Application Support/EastSea/node'
report = {'schema': 1, 'result': 'FAIL', **CONFIG, 'chain_id': CHAIN_ID,
          'reference_rpc': REFERENCE_RPC, 'head_tolerance': HEAD_TOLERANCE,
          'started_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
          'crash_reports': [], 'phases': [], 'seeded_log_bytes': 0}
mount = None
caffeinate = None
crash_baseline = {}
app_executable = None
proc = None
rpc_id = 0

def command(argv, timeout=30, check=True):
    print('guest command: ' + shlex.join(str(item) for item in argv), flush=True)
    done = subprocess.run([str(item) for item in argv], capture_output=True, text=True, timeout=timeout)
    if done.stdout:
        print(done.stdout.rstrip(), flush=True)
    if done.stderr:
        print(done.stderr.rstrip(), flush=True)
    if check and done.returncode != 0:
        raise RuntimeError('command failed (%s): %s' % (done.returncode, argv[0]))
    return done

def atomic_result():
    report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    partial = OUTPUT / 'result.json.part'
    with partial.open('w') as out:
        json.dump(report, out, indent=2)
        out.write('\n')
        out.flush()
        os.fsync(out.fileno())
    os.replace(partial, OUTPUT / 'result.json')

def no_links(path):
    for component in [path, *path.parents]:
        if component.is_symlink():
            raise RuntimeError('symlink refused in guest path: ' + str(component))

def process_table():
    rows = subprocess.check_output(['/bin/ps', '-axo', 'pid=,ppid=,uid='], text=True)
    table = {}
    for line in rows.splitlines():
        fields = line.split()
        if len(fields) != 3:
            continue
        pid, parent, uid = map(int, fields)
        if uid != os.getuid():
            continue
        buffer = ctypes.create_string_buffer(4096)
        if proc.proc_pidpath(pid, buffer, len(buffer)) > 0:
            table[pid] = (parent, os.fsdecode(buffer.value))
    return table

def app_pid(table):
    pids = [pid for pid, (_, binary) in table.items() if binary == str(app_executable)]
    if len(pids) != 1:
        raise RuntimeError('expected one installed EastSea process, found ' + str(pids))
    return pids[0]

def belongs_to(pid, parent, table):
    visited = set()
    while pid in table and pid not in visited:
        if pid == parent:
            return True
        visited.add(pid)
        pid = table[pid][0]
    return False

def listener_pid(parent, table):
    done = subprocess.run(['/usr/sbin/lsof', '-nP', '-iTCP:18545', '-sTCP:LISTEN', '-Fp'],
                          capture_output=True, text=True, timeout=5)
    listeners = {int(line[1:]) for line in done.stdout.splitlines() if re.fullmatch(r'p\d+', line)}
    if not listeners:
        return None
    expected = str(APP / 'Contents/Helpers/aether')
    if len(listeners) != 1:
        raise RuntimeError('RPC 18545 has ambiguous listeners: ' + str(listeners))
    pid = listeners.pop()
    if pid not in table or table[pid][1] != expected or not belongs_to(pid, parent, table):
        raise RuntimeError('RPC 18545 is not owned by this app\'s bundled node: pid=' + str(pid))
    return pid

def crash_state():
    state = {}
    for directory in [HOME / 'Library/Logs/DiagnosticReports', pathlib.Path('/Library/Logs/DiagnosticReports')]:
        if not directory.exists():
            continue
        for path in directory.iterdir():
            if path.name.startswith(('EastSea-', 'EastSea_', 'aether-', 'aether_')):
                metadata = path.stat()
                state[str(path)] = (metadata.st_ino, metadata.st_size, metadata.st_mtime_ns)
    return state

def check_crashes():
    changed = [name for name, metadata in crash_state().items() if crash_baseline.get(name) != metadata]
    if changed:
        report['crash_reports'] = changed
        destination = OUTPUT / 'crash-reports'
        destination.mkdir(exist_ok=True)
        for index, name in enumerate(changed):
            path = pathlib.Path(name)
            if path.is_file():
                shutil.copy2(path, destination / ('%d-%s' % (index, path.name)))
        raise RuntimeError('new or changed app/node crash reports: ' + ', '.join(changed))

def rpc(url, method):
    global rpc_id
    rpc_id += 1
    payload = json.dumps({'jsonrpc': '2.0', 'id': rpc_id, 'method': method, 'params': []})
    done = subprocess.run(['/usr/bin/curl', '--silent', '--show-error', '--fail', '--max-time', '4',
                           '--header', 'Content-Type: application/json', '--data-binary', payload, url],
                          capture_output=True, text=True, timeout=6)
    raw = {'at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'url': url, 'method': method,
           'id': rpc_id, 'exit': done.returncode, 'body': done.stdout, 'error': done.stderr}
    with (OUTPUT / 'rpc.jsonl').open('a') as out:
        out.write(json.dumps(raw) + '\n')
    if done.returncode:
        raise RuntimeError('RPC unavailable: %s %s: %s' % (url, method, done.stderr.strip()))
    reply = json.loads(done.stdout)
    if reply.get('id') != rpc_id or reply.get('error') is not None:
        raise RuntimeError('RPC error or mismatched id: ' + done.stdout[:300])
    value = reply.get('result')
    if not isinstance(value, str) or not re.fullmatch(r'0x[0-9a-fA-F]+', value):
        raise RuntimeError('RPC did not return a hex quantity: ' + done.stdout[:300])
    return int(value, 16)

def launch():
    command(['/usr/bin/open', str(APP)])
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        check_crashes()
        table = process_table()
        matches = [pid for pid, (_, binary) in table.items() if binary == str(app_executable)]
        if matches:
            return app_pid(table)
        time.sleep(1)
    raise RuntimeError('installed app did not launch within 30 seconds')

def observe(stage, pid, duration, startup_grace):
    start = time.monotonic()
    phase = {'stage': stage, 'app_pid': pid, 'required_seconds': duration, 'samples': []}
    report['phases'].append(phase)
    last_local = None
    last_reference = None
    node_answered = False
    while True:
        table = process_table()
        if app_pid(table) != pid:
            raise RuntimeError(stage + ': app PID changed unexpectedly')
        check_crashes()
        elapsed = time.monotonic() - start
        sample = {'elapsed_seconds': round(elapsed, 3), 'app_pid': pid}
        node = listener_pid(pid, table)
        sample['node_pid'] = node
        if node is None and (node_answered or elapsed > startup_grace):
            raise RuntimeError(stage + ': app node is not listening on RPC 18545')
        if node is not None:
            try:
                chain = rpc(LOCAL_RPC, 'eth_chainId')
                if chain != CHAIN_ID:
                    raise RuntimeError('local RPC chain_id=%d, required %d' % (chain, CHAIN_ID))
                local = rpc(LOCAL_RPC, 'eth_blockNumber')
            except (RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
                if node_answered or elapsed > startup_grace or 'chain_id=' in str(error):
                    raise
                sample['startup_rpc_error'] = str(error)
            else:
                node_answered = True
                if last_local is not None and local < last_local:
                    raise RuntimeError(stage + ': local chain height regressed')
                last_local = local
                sample.update({'chain_id': chain, 'local_height': local})
                try:
                    reference_chain = rpc(REFERENCE_RPC, 'eth_chainId')
                    if reference_chain != CHAIN_ID:
                        raise RuntimeError('reference RPC chain_id=%d, required %d' % (reference_chain, CHAIN_ID))
                    reference = rpc(REFERENCE_RPC, 'eth_blockNumber')
                except (RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
                    if 'chain_id=' in str(error):
                        raise
                    sample['reference_rpc_error'] = str(error)
                else:
                    if last_reference is not None and reference < last_reference:
                        raise RuntimeError(stage + ': reference chain height regressed')
                    last_reference = reference
                    sample.update({'reference_chain_id': reference_chain, 'reference_height': reference,
                                   'lag_blocks': reference - local})
        phase['samples'].append(sample)
        with (OUTPUT / 'samples.jsonl').open('a') as out:
            out.write(json.dumps({'stage': stage, **sample}) + '\n')
        # Recheck after the bounded RPC calls: a crash during a call must not
        # turn the final successful response into a passing observation.
        if app_pid(process_table()) != pid:
            raise RuntimeError(stage + ': app exited or restarted during RPC sample')
        check_crashes()
        if elapsed >= duration:
            break
        time.sleep(min(SAMPLE_SECONDS, duration - elapsed))
    phase['observed_seconds'] = round(time.monotonic() - start, 3)
    samples = phase['samples']
    final = samples[-1]
    if 'lag_blocks' not in final or abs(final['lag_blocks']) > HEAD_TOLERANCE:
        raise RuntimeError(stage + ': final sample is not within 12 blocks of the reference head')
    caught = [row for row in samples if 'lag_blocks' in row and abs(row['lag_blocks']) <= HEAD_TOLERANCE]
    if len(caught) < 2 or final['local_height'] <= caught[0]['local_height']:
        raise RuntimeError(stage + ': local node did not advance after catching the chain head')
    if final['reference_height'] <= caught[0]['reference_height']:
        raise RuntimeError(stage + ': reference chain did not advance after catch-up')
    # A single successful early sync does not prove ongoing following. The
    # final 30 seconds must contain a second local and reference advance.
    recent = [row for row in caught if row['elapsed_seconds'] >= duration - 30]
    if len(recent) < 2 or final['local_height'] <= recent[0]['local_height'] \
            or final['reference_height'] <= recent[0]['reference_height']:
        raise RuntimeError(stage + ': node/reference stalled at the end of observation')
    phase['first_caught_height'] = caught[0]['local_height']
    phase['final_local_height'] = final['local_height']
    phase['final_reference_height'] = final['reference_height']
    phase['final_lag_blocks'] = final['lag_blocks']
    print('PASS guest phase: %s pid=%d seconds=%.3f local=%d reference=%d lag=%d' %
          (stage, pid, phase['observed_seconds'], final['local_height'], final['reference_height'], final['lag_blocks']), flush=True)

def main():
    global mount, caffeinate, crash_baseline, app_executable, proc
    # These checks run before preferences, data, or Applications are changed.
    model = command(['/usr/sbin/sysctl', '-n', 'hw.model']).stdout.strip()
    if not model.startswith('VirtualMac'):
        raise RuntimeError('guest helper refuses a physical Mac: hw.model=' + model)
    if command(['/usr/bin/id', '-un']).stdout.strip() != 'smoke' or HOME != pathlib.Path('/Users/smoke'):
        raise RuntimeError('guest must run as the fresh smoke user at /Users/smoke')
    if command(['/usr/bin/stat', '-f', '%Su', '/dev/console']).stdout.strip() != 'smoke':
        raise RuntimeError('smoke must own the live GUI console session')
    marker = HOME / '.eastsea-release-smoke-vm'
    if not marker.is_file() or marker.is_symlink():
        raise RuntimeError('isolated VM marker missing: ' + str(marker))
    if CONFIG['profile'] not in ('empty', 'large-log') or not re.fullmatch(r'[A-Za-z0-9.-]+', CONFIG['run_id']):
        raise RuntimeError('invalid host run configuration')
    for relative in ['Library/Application Support/EastSea', 'Library/Application Support/Aether',
                     'Library/Application Support/AetherWallet', 'Library/Saved Application State/com.pipln.eastsea.savedState',
                     'Library/Preferences/com.pipln.eastsea.plist', 'Library/Preferences/com.pipln.aether.plist', 'aether-testnet']:
        path = HOME / relative
        no_links(path)
        if path.exists():
            raise RuntimeError('base user is not fresh: ' + str(path))
    for domain in [BUNDLE_ID, 'com.pipln.aether']:
        if command(['/usr/bin/defaults', 'read', domain], check=False).returncode == 0:
            raise RuntimeError('base user has existing preferences: ' + domain)
    for path in [APP, pathlib.Path('/Applications/Aether.app')]:
        no_links(path)
        if path.exists():
            raise RuntimeError('base already contains an EastSea/Aether app: ' + str(path))
    proc = ctypes.CDLL('/usr/lib/libproc.dylib')
    proc.proc_pidpath.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
    proc.proc_pidpath.restype = ctypes.c_int
    if any('EastSea.app/' in binary or 'Aether.app/' in binary for _, binary in process_table().values()):
        raise RuntimeError('base already has an EastSea/Aether process')
    command(['/usr/bin/sudo', '-n', '/usr/bin/true'])
    guest_tmp = HOME / 'tmp'
    no_links(guest_tmp)
    guest_tmp.mkdir(exist_ok=True)
    os.environ['TMPDIR'] = str(guest_tmp)
    work = guest_tmp / CONFIG['run_id']
    no_links(work)
    work.mkdir()
    mount = work / 'dmg'
    mount.mkdir()
    caffeinate = subprocess.Popen(['/usr/bin/caffeinate', '-dimsu', '-w', str(os.getpid())])
    dmg = pathlib.Path('/Volumes/My Shared Files/release-dmg/EastSea.dmg')
    digest = hashlib.sha256()
    with dmg.open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(chunk)
    if digest.hexdigest() != CONFIG['dmg_sha256']:
        raise RuntimeError('shared DMG checksum differs from host artifact')
    command(['/usr/bin/hdiutil', 'attach', '-readonly', '-nobrowse', '-mountpoint', mount, dmg], timeout=120)
    source_app = mount / 'EastSea.app'
    with (source_app / 'Contents/Info.plist').open('rb') as source:
        info = plistlib.load(source)
    if info.get('CFBundleIdentifier') != BUNDLE_ID:
        raise RuntimeError('DMG bundle identity differs from ' + BUNDLE_ID)
    executable = info.get('CFBundleExecutable', '')
    if not executable or pathlib.Path(executable).name != executable:
        raise RuntimeError('invalid app executable in bundle metadata')
    with (source_app / 'Contents/Resources/network.json').open() as source:
        network = json.load(source)
    if network.get('chain_id') != CHAIN_ID:
        raise RuntimeError('shipped network.json must be testnet 7780')
    command(['/usr/bin/codesign', '--verify', '--deep', '--strict', source_app], timeout=120)
    signing = command(['/usr/bin/codesign', '--display', '--verbose=4', source_app])
    if 'TeamIdentifier=' + TEAM_ID not in (signing.stdout + signing.stderr).splitlines():
        raise RuntimeError('DMG is not signed by the release Developer ID team')
    print('guest change: install ' + str(APP), flush=True)
    command(['/usr/bin/sudo', '-n', '/usr/bin/ditto', source_app, APP], timeout=120)
    command(['/usr/bin/codesign', '--verify', '--deep', '--strict', APP], timeout=120)
    report['bundle_id'] = BUNDLE_ID
    report['team_id'] = TEAM_ID
    report['version'] = info.get('CFBundleShortVersionString')
    report['build'] = info.get('CFBundleVersion')
    app_executable = APP / 'Contents/MacOS' / executable
    command(['/usr/bin/hdiutil', 'detach', mount], timeout=30)
    mount = None
    print('guest change: configure only the fresh smoke-user preferences', flush=True)
    for key, value in [('nodeEnabled', True), ('nodeOnlyOnPower', False), ('proveBlocks', False),
                       ('nodeUnattended', False), ('unattendedUserChose', True), ('loginItemDefaultApplied', True),
                       ('SUEnableAutomaticChecks', False), ('SUAutomaticallyUpdate', False), ('SUHasLaunchedBefore', True)]:
        command(['/usr/bin/defaults', 'write', BUNDLE_ID, key, '-bool', 'true' if value else 'false'])
    if CONFIG['profile'] == 'large-log':
        no_links(NODE)
        NODE.mkdir(parents=True)
        log = NODE / 'node.log'
        print('guest change: seed %d bytes into %s' % (SEEDED_BYTES, log), flush=True)
        block = (b'release smoke historical node log\n' * 32768)[:1024 * 1024]
        with log.open('wb') as out:
            remaining = SEEDED_BYTES
            while remaining:
                chunk = block[:min(remaining, len(block))]
                out.write(chunk)
                remaining -= len(chunk)
            out.flush()
            os.fsync(out.fileno())
        report['seeded_log_bytes'] = log.stat().st_size
        if report['seeded_log_bytes'] != SEEDED_BYTES:
            raise RuntimeError('large-log fixture was not fully seeded')
    crash_baseline = crash_state()
    report['crash_baseline'] = crash_baseline
    if crash_baseline:
        raise RuntimeError('base contains app/node crash reports and is not a never-used profile')
    report['reference_chain_id'] = rpc(REFERENCE_RPC, 'eth_chainId')
    if report['reference_chain_id'] != CHAIN_ID:
        raise RuntimeError('reference RPC is not testnet 7780')
    print('guest change: launch shipped /Applications/EastSea.app', flush=True)
    first_pid = launch()
    observe('first-launch', first_pid, FIRST_SECONDS, startup_grace=120)
    print('guest change: stop installed app pid=%d using SIGTERM before relaunch' % first_pid, flush=True)
    # Avoid Apple Events consent/UI automation. The shipped child uses
    # --exit-with-parent; require both processes to exit before relaunch.
    os.kill(first_pid, 15)
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        check_crashes()
        table = process_table()
        app_alive = any(binary == str(app_executable) for _, binary in table.values())
        node_alive = any(binary == str(APP / 'Contents/Helpers/aether') for _, binary in table.values())
        if not app_alive and not node_alive:
            break
        time.sleep(1)
    else:
        raise RuntimeError('app or bundled node did not exit after ordinary quit')
    time.sleep(3)
    check_crashes()
    print('guest change: relaunch shipped /Applications/EastSea.app', flush=True)
    second_pid = launch()
    if second_pid == first_pid:
        raise RuntimeError('relaunch did not create a new app process')
    observe('relaunch', second_pid, RELAUNCH_SECONDS, startup_grace=30)
    check_crashes()
    report['result'] = 'PASS'
    print('PASS guest profile: ' + CONFIG['profile'], flush=True)

try:
    main()
except BaseException as error:
    report['result'] = 'FAIL'
    report['reason'] = str(error)
    traceback.print_exc()
finally:
    if mount is not None:
        try:
            command(['/usr/bin/hdiutil', 'detach', mount], check=False)
        except BaseException as error:
            print('guest cleanup mount: ' + str(error), flush=True)
    if caffeinate is not None:
        caffeinate.terminate()
        try:
            caffeinate.wait(timeout=5)
        except subprocess.TimeoutExpired:
            caffeinate.kill()
            caffeinate.wait()
    atomic_result()
sys.exit(0 if report['result'] == 'PASS' else 1)
GUEST_PYTHON
  chmod 444 "$directory/config.json" "$directory/guest-smoke.py"
  chmod 555 "$directory/guest-runner.sh"
}

validate_result() {
  /usr/bin/python3 - "$1" "$2" "$RUN_ID" "$3" "$DMG_HASH" <<'PY_RESULT'
import json, pathlib, sys
path, status_path, run_id, profile, digest = sys.argv[1:]
file = pathlib.Path(path)
status_file = pathlib.Path(status_path)
if file.is_symlink() or status_file.is_symlink():
    raise RuntimeError('guest result/completion status cannot be a symlink')
status = json.loads(status_file.read_text())
if status.get('schema') != 1 or type(status.get('exit_status')) is not int or status['exit_status'] != 0:
    raise RuntimeError('guest helper did not exit successfully: ' + str(status))
data = json.loads(file.read_text())
if data.get('result') != 'PASS':
    raise RuntimeError('guest FAIL: ' + data.get('reason', 'incomplete result'))
required = {'schema': 1, 'run_id': run_id, 'profile': profile, 'dmg_sha256': digest,
            'chain_id': 7780, 'bundle_id': 'com.pipln.eastsea', 'team_id': '45WU468FZE',
            'reference_rpc': 'https://rpc.eastsea.xyz', 'reference_chain_id': 7780, 'head_tolerance': 12,
            'seeded_log_bytes': 64 * 1024 * 1024 if profile == 'large-log' else 0}
for key, value in required.items():
    if data.get(key) != value:
        raise RuntimeError('guest result mismatch: ' + key)
if data.get('crash_reports') != []:
    raise RuntimeError('guest reported new/changed crash reports')
phases = data.get('phases', [])
if len(phases) != 2:
    raise RuntimeError('both launch observations are required')
for phase, name, seconds in zip(phases, ['first-launch', 'relaunch'], [600, 60]):
    samples = phase.get('samples', [])
    if phase.get('stage') != name or phase.get('required_seconds') != seconds \
            or phase.get('observed_seconds', 0) < seconds or len(samples) < 2 \
            or samples[-1].get('elapsed_seconds', 0) < seconds:
        raise RuntimeError('incomplete ' + name + ' monitoring window')
    if len(samples) < (30 if seconds == 600 else 6):
        raise RuntimeError('too few app observations during ' + name)
    previous = None
    for sample in samples:
        elapsed = sample.get('elapsed_seconds', -1)
        if elapsed < 0 or (previous is not None and (elapsed <= previous or elapsed - previous > 30)):
            raise RuntimeError('missing or non-monotonic app observations during ' + name)
        previous = elapsed
    pid = phase.get('app_pid')
    if not isinstance(pid, int) or pid <= 0 or any(row.get('app_pid') != pid for row in samples):
        raise RuntimeError('unstable installed app process')
    final = samples[-1]
    if final.get('chain_id') != 7780 or final.get('reference_chain_id') != 7780 or not final.get('node_pid') \
            or abs(final.get('lag_blocks', 999999)) > 12:
        raise RuntimeError('missing final app-node/chain-head evidence')
    caught = [row for row in samples if 'lag_blocks' in row and abs(row['lag_blocks']) <= 12]
    recent = [row for row in caught if row['elapsed_seconds'] >= seconds - 30]
    for group in [caught, recent]:
        if len(group) < 2 or final['local_height'] <= group[0]['local_height'] \
                or final['reference_height'] <= group[0]['reference_height']:
            raise RuntimeError('chain did not advance after catch-up and at end of ' + name)
    print('PASS release-vm-smoke evidence: profile=%s stage=%s pid=%d seconds=%.3f local=%d reference=%d lag=%d' %
          (profile, name, pid, phase['observed_seconds'], final['local_height'], final['reference_height'], final['lag_blocks']))
if phases[0]['app_pid'] == phases[1]['app_pid']:
    raise RuntimeError('relaunch must have a new app PID')
print('PASS release-vm-smoke profile=%s seeded_node_log_bytes=%d new_crashes=0 artifact_sha256=%s result=%s' %
      (profile, data['seeded_log_bytes'], digest, path))
PY_RESULT
}

for profile in empty large-log; do
  directory="$RUN_DIR/$profile"
  write_guest "$profile" "$directory"
  vm="eastsea-release-smoke-$RUN_ID-$profile"
  echo "release-vm-smoke change: clone local base $BASE to $vm (disk: $VM_HOME/vms/$vm)"
  CLONES+=("$vm")
  "$TART_BIN" clone "$BASE" "$vm" > "$directory/clone.log" 2>&1 || fail "clone failed; see $directory/clone.log"
  /usr/bin/python3 - "$VM_HOME/vms/$vm" <<'PY_CLONE' || fail "clone storage is not isolated on the workspace volume"
import pathlib, sys
path = pathlib.Path(sys.argv[1])
for item in [path, path / 'config.json', path / 'disk.img', *path.rglob('*')]:
    if not item.exists() or item.is_symlink() or item.resolve() != item:
        raise RuntimeError('unsafe disposable clone storage: ' + str(item))
    if item.stat().st_dev != pathlib.Path('/Volumes/workspace').stat().st_dev:
        raise RuntimeError('clone storage is not on workspace volume')
PY_CLONE
  ACTIVE_VM=$vm
  echo "release-vm-smoke change: boot $vm; read-only helpers=$directory; writable evidence=$directory/output; read-only DMG=$RUN_DIR/dmg"
  launch_signal=0
  # Register the runner PID before allowing a signal to enter cleanup.
  trap 'launch_signal=130' INT
  trap 'launch_signal=143' TERM
  "$TART_BIN" run --no-graphics "--dir=release-smoke:$directory:ro" \
    "--dir=release-smoke-output:$directory/output" "--dir=release-dmg:$RUN_DIR/dmg:ro" "$vm" \
    > "$directory/tart.log" 2>&1 &
  RUN_PID=$!
  trap 'exit 130' INT
  trap 'exit 143' TERM
  [ "$launch_signal" -eq 0 ] || exit "$launch_signal"
  deadline=$((SECONDS + PROFILE_TIMEOUT))
  while [ ! -f "$directory/output/exit-status.json" ]; do
    kill -0 "$RUN_PID" 2>/dev/null || fail "VM runner exited before guest completion; see $directory/tart.log"
    [ "$SECONDS" -lt "$deadline" ] || fail "$profile exceeded ${PROFILE_TIMEOUT}s; check the base auto-login/LaunchAgent and $directory/output/guest.log"
    sleep 2
  done
  validate_result "$directory/output/result.json" "$directory/output/exit-status.json" "$profile" \
    || fail "$profile did not pass; see $directory/output"
  # Each profile starts from the pristine base, never from the prior run.
  cleanup || fail "could not safely stop/delete disposable VM; see $RUN_DIR/cleanup.log"
  CLONES=()
done

trap - EXIT INT TERM
echo "PASS release-vm-smoke: empty and 64 MiB log profiles; 600s launch + 60s relaunch each; testnet 7780; evidence=$RUN_DIR"
