#!/usr/bin/env bash
# Source-only, guarded offload to the lane's authorized Mac; never takes a local slot.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd -P)
HOST=poc-m3
BASE=eastsea-lab/dev-speed
ssh_options=(-o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=10 -o ServerAliveCountMax=2 -l kjaylee)
setup=0 dry=0
packages=() targets=() swift=() test_args=()
usage() { echo 'Usage: scripts/remote-test.sh [--setup] [--dry-run] [-p PACKAGE ...] [--test NAME ...] [--swift NAME ...] [-- NEXTEST_ARGS...]'; }
while (($#)); do
  case "$1" in
    --setup) setup=1; shift ;;
    --dry-run) dry=1; shift ;;
    -p|--package) (($# >= 2)) || { usage >&2; exit 2; }; packages+=("$2"); shift 2 ;;
    --test) (($# >= 2)) || { usage >&2; exit 2; }; targets+=("$2"); shift 2 ;;
    --swift) (($# >= 2)) || { usage >&2; exit 2; }; swift+=("$2"); shift 2 ;;
    --) shift; test_args=("$@"); break ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unsupported option: $1" >&2; usage >&2; exit 2 ;;
  esac
done
if ((${#packages[@]} == 0 && ${#swift[@]} == 0 && setup == 0)); then usage >&2; exit 2; fi
if ((${#packages[@]} == 0 && (${#targets[@]} > 0 || ${#test_args[@]} > 0))); then
  echo '--test and nextest arguments require an explicit Rust package.' >&2; exit 2
fi
# Validate before SSH. Names come from normal workspace manifests; alternate
# manifests/profiles, staticlibs and guest projects are never accepted.
python3 - "$ROOT" "${#packages[@]}" "${#targets[@]}" "${#swift[@]}" \
  ${packages[@]+"${packages[@]}"} ${targets[@]+"${targets[@]}"} ${swift[@]+"${swift[@]}"} ${test_args[@]+"${test_args[@]}"} <<'VALIDATE'
from pathlib import Path
import re, sys
root = Path(sys.argv[1]); counts = list(map(int, sys.argv[2:5])); args = sys.argv[5:]
packages, targets, swift = [], [], []
for count, selection in zip(counts, (packages, targets, swift)):
    selection.extend(args[:count]); del args[:count]
known = set()
for manifest in [root / 'legacy/Cargo.toml', *root.glob('crates/*/Cargo.toml')]:
    if manifest.is_file() and not manifest.is_symlink():
        section = re.search(r'(?ms)^\[package\]\s*\n(.*?)(?=^\[|\Z)', manifest.read_text())
        match = re.search(r'(?m)^name\s*=\s*"([a-z0-9-]+)"', section[1]) if section else None
        if match and match[1] not in ('aether-ffi', 'aether-prover'):
            known.add(match[1])
for package in packages:
    if package not in known or not re.fullmatch(r'aether-[a-z0-9-]+', package):
        raise SystemExit('Package is not approved for remote tests: ' + package)
for name in targets:
    if not re.fullmatch(r'[a-zA-Z0-9_][a-zA-Z0-9_-]*', name):
        raise SystemExit('Invalid Rust integration test name: ' + name)
known_swift = set(re.findall(r'^run ([a-z0-9-]+) ', (root / 'scripts/test-swift-pure.sh').read_text(), re.M))
known_swift.add('update-listener')
for name in swift:
    if name not in known_swift:
        raise SystemExit('Unknown pure Swift test: ' + name)
values = {'-E', '--filterset', '--test-threads', '--jobs', '-j', '--run-ignored', '--partition',
          '--failure-output', '--success-output', '--status-level', '--final-status-level',
          '--no-tests', '--max-fail', '--color'}
flags = {'--no-capture', '--fail-fast', '--no-fail-fast', '--no-pager', '--ignore-default-filter',
         '--hide-progress-bar', '-v', '--verbose'}
while args:
    arg = args.pop(0); key, equals, value = arg.partition('=')
    if key in values:
        if not equals:
            if not args: raise SystemExit('Missing nextest option value: ' + arg)
            value = args.pop(0)
        if not value: raise SystemExit('Empty nextest option value: ' + arg)
    elif arg.startswith('-') and arg not in flags:
        raise SystemExit('Unsupported nextest runtime option: ' + arg)
VALIDATE
[[ -x /usr/bin/rsync ]] || { echo 'The macOS system /usr/bin/rsync is required.' >&2; exit 1; }
base=${AETHER_BUILD_BASE:-lead-merge}
commit=$(git -C "$ROOT" merge-base HEAD "$base")
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || exit 1
if ((dry)); then
  printf 'Host: %s (user kjaylee, macOS)\nSnapshot: ~/%s/source\nWarm target: ~/%s/targets/aether-%s-<compiler-config-hash>\n' "$HOST" "$BASE" "$BASE" "${commit:0:16}"
  printf 'Packages:'; printf ' %s' ${packages[@]+"${packages[@]}"}; printf '\n'
  printf 'Rust targets:'; printf ' %s' ${targets[@]+"${targets[@]}"}; printf '\n'
  printf 'Swift tests:'; printf ' %s' ${swift[@]+"${swift[@]}"}; printf '\n'
  printf 'Setup: %s; nice=15, owned RSS <=12 GiB, free RAM >=4 GiB, disk >=30 GiB; no local compile slot or SSH.\n' "$setup"
  exit 0
fi
# Run the same guard read-only over stdin before creating remote files.
ssh "${ssh_options[@]}" "$HOST" 'export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"; exec python3 - --preflight' \
  < "$ROOT/scripts/remote-resource-guard.py"
mkdir -p "$ROOT/tmp"
export TMPDIR="$ROOT/tmp"
lane=$(mktemp -d "$ROOT/tmp/remote-test.XXXXXXXX")
token=${lane##*/}
remote="$BASE/source"
held=0 remote_pid=
# shellcheck disable=SC2329
cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP
  if [[ -n "$remote_pid" ]]; then kill "$remote_pid" 2>/dev/null || true; wait "$remote_pid" 2>/dev/null || true; fi
  if ((held)); then
    # Stop only the matching guarded run before releasing the snapshot lease.
    # Retain a stale lease when cleanup cannot be established.
    # token is an internally generated, alphanumeric mktemp basename.
    # shellcheck disable=SC2029
    ssh "${ssh_options[@]}" "$HOST" "python3 - '$token'" <<'RELEASE' || true
import json, os
from pathlib import Path
import signal, subprocess, sys, time
base = Path.home() / 'eastsea-lab/dev-speed'; token = sys.argv[1]
lock = base / 'tmp/remote-test.lock'; owner = lock / 'owner'
if owner.is_file() and owner.read_text() == token:
    record = base / 'source/tmp' / ('remote-owner-' + token + '.json')
    if record.is_file():
        value = json.loads(record.read_text())
        result = subprocess.run(['/bin/ps', '-p', str(value['pid']), '-o', 'lstart='], text=True, capture_output=True)
        if result.returncode == 0 and result.stdout.strip() == value['start']:
            os.kill(value['pid'], signal.SIGTERM)
            deadline = time.monotonic() + 10
            while record.exists() and time.monotonic() < deadline: time.sleep(0.1)
            if record.exists(): raise SystemExit('Remote cleanup incomplete; snapshot lease retained.')
        record.unlink(missing_ok=True)
    resources = base / 'source/tmp' / ('remote-' + token + '-resources.json')
    if resources.is_file() and not json.loads(resources.read_text()).get('cleanup_complete', False):
        raise SystemExit('Remote cleanup incomplete; snapshot lease retained.')
    owner.unlink(); lock.rmdir()
RELEASE
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
# A complete sanitized staging tree lets --delete remove stale deleted sources.
# Source/tmp is protected; sibling targets and tools never enter rsync.
python3 - "$ROOT" "$lane/source" <<'SNAPSHOT'
from pathlib import Path
import re, shutil, subprocess, sys
root = Path(sys.argv[1]); output = Path(sys.argv[2]); output.mkdir()
files = subprocess.check_output(['git', '-C', str(root), 'ls-files', '-z', '--cached', '--others', '--exclude-standard']).split(b'\0')
helpers = {'dev-cargo.sh', 'build-cache.py', 'compile-gate.sh', 'compile-gate.py', 'run-rust-tests.sh',
           'test-tmpdir.sh', 'test-swift-pure.sh', 'swift-test-cache.py', 'wallet-l10n.py',
           'test-update-daemon.sh', 'test-update-listener.sh', 'remote-resource-guard.py'}
allowed = {'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.cargo/config', '.cargo/config.toml', '.config/nextest.toml'}
allowed.update('scripts/' + name for name in helpers)
prefixes = ('crates/', 'legacy/', 'fuzz/corpus/', 'vendor/n0-mainline/', 'apps/wallet/Sources/', 'apps/wallet/Tests/',
            'apps/agent/Sources/', 'apps/agent/Tests/', 'apps/bridge/Sources/', 'apps/bridge/Tests/')
allowed.add('apps/wallet/Resources/Localizable.xcstrings')
allowed.add('apps/wallet/Resources/network.json')
allowed.add('tests/fixtures/sea-urls.json')
deny = re.compile(r'(^|/)(?:\.git|target|tmp|\.cache|\.env[^/]*|secrets?(?:[._-][^/]*)?|credentials?(?:[._-][^/]*)?|id_rsa|id_ed25519|\.ssh|\.aws|\.claude|\.omx)(/|$)|\.(?:pem|key|p12|pfx|keystore)$', re.I)
for raw in sorted(set(files)):
    if not raw: continue
    name = raw.decode('utf-8'); path = root / name
    if name not in allowed and not name.startswith(prefixes): continue
    if deny.search(name) or path.is_symlink() or not path.is_file(): continue
    if any(parent.is_symlink() for parent in path.parents if parent != root): continue
    if any(part in ('guest', 'guests', 'aether-jolt') for part in Path(name).parts): continue
    before = path.stat(); data = path.read_bytes()
    if re.search(rb'-----BEGIN (?:[A-Z ]*PRIVATE KEY|OPENSSH PRIVATE KEY)-----', data):
        raise SystemExit('Refusing private-key material in source snapshot: ' + name)
    after = path.stat()
    if (before.st_size, before.st_mtime_ns, before.st_ino) != (after.st_size, after.st_mtime_ns, after.st_ino):
        raise SystemExit('Source changed during snapshot: ' + name)
    destination = output / name; destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(data); shutil.copystat(path, destination)
SNAPSHOT
# token is an internally generated, alphanumeric mktemp basename.
# shellcheck disable=SC2029
ssh "${ssh_options[@]}" "$HOST" "python3 - '$token'" <<'RESERVE'
from pathlib import Path
import sys
base = Path.home() / 'eastsea-lab/dev-speed'
for path in (base.parent, base, base / 'tmp', base / 'source', base / 'source/tmp'):
    if path.is_symlink() or (path.exists() and not path.is_dir()):
        raise SystemExit('Refusing non-owned remote lane directory: ' + str(path))
(base / 'tmp').mkdir(parents=True, exist_ok=True)
try: (base / 'tmp/remote-test.lock').mkdir()
except FileExistsError: raise SystemExit('Another remote run owns the dev-speed snapshot; retry after it finishes.')
(base / 'tmp/remote-test.lock/owner').write_text(sys.argv[1])
(base / 'source').mkdir(exist_ok=True)
RESERVE
held=1
python3 - "$lane/source/" "$HOST:$remote/" "$lane/rsync.json" <<'SYNC_TIMING'
import json, subprocess, sys, time
from pathlib import Path
started = time.monotonic()
status = subprocess.call(['/usr/bin/rsync', '-a', '--delete', '--exclude=/tmp/', '--exclude=/.git/',
                          '--rsync-path=/usr/bin/rsync', '-e',
                          'ssh -o BatchMode=yes -o ConnectTimeout=10 -l kjaylee', sys.argv[1], sys.argv[2]])
Path(sys.argv[3]).write_text(json.dumps({'rsync_seconds': time.monotonic() - started,
                                       'rsync_exit_code': status}))
sys.exit(status)
SYNC_TIMING
# Encode values independently; filters may contain spaces or shell syntax.
quote() { python3 -c 'import shlex,sys; print(shlex.quote(sys.argv[1]), end="")' "$1"; }
command="bash -s -- $(quote "$commit") $(quote "$token") $(quote "$setup")"
for package in ${packages[@]+"${packages[@]}"}; do command+=" -p $(quote "$package")"; done
for target in ${targets[@]+"${targets[@]}"}; do command+=" --test $(quote "$target")"; done
for name in ${swift[@]+"${swift[@]}"}; do command+=" --swift $(quote "$name")"; done
command+=" --"
for arg in ${test_args[@]+"${test_args[@]}"}; do command+=" $(quote "$arg")"; done
# Each command argument was independently POSIX shell quoted above.
# shellcheck disable=SC2029
ssh "${ssh_options[@]}" "$HOST" "$command" > "$lane/test.log" 2>&1 <<'REMOTE_TEST' &
set -euo pipefail
commit=$1 token=$2 setup=$3; shift 3
base="$HOME/eastsea-lab/dev-speed"; snapshot="$base/source"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
cd "$snapshot"
python3 scripts/remote-resource-guard.py --preflight --root "$snapshot"
mkdir -p "$snapshot/tmp" "$base/tmp/tools"
export TMPDIR="$snapshot/tmp"
toolchain=$(dirname "$(rustup which --toolchain 1.98.1 cargo)")
export PATH="$base/tmp/tools:$toolchain:$PATH"
if ((setup)) || { [[ "${1:-}" == -p ]] && { ! command -v cargo-nextest >/dev/null || ! command -v sccache >/dev/null; }; }; then
  work=$(mktemp -d "$snapshot/tmp/remote-setup.XXXXXXXX")
  trap 'rm -rf "$work"' EXIT
  python3 scripts/remote-resource-guard.py --preflight --root "$snapshot"
  if ! command -v cargo-nextest >/dev/null; then
    curl --proto '=https' --tlsv1.2 --connect-timeout 10 --max-time 60 -fLsS https://get.nexte.st/0.9/mac -o "$work/nextest.tar.gz"
  fi
  if ! command -v sccache >/dev/null; then
    url=https://github.com/mozilla/sccache/releases/download/v0.18.0/sccache-v0.18.0-aarch64-apple-darwin.tar.gz
    curl --proto '=https' --tlsv1.2 --connect-timeout 10 --max-time 60 -fLsS "$url" -o "$work/sccache-v0.18.0-aarch64-apple-darwin.tar.gz"
    curl --proto '=https' --tlsv1.2 --connect-timeout 10 --max-time 60 -fLsS "$url.sha256" -o "$work/sccache.sha256"
  fi
  python3 - "$work" "$base/tmp/tools" <<'TOOLS'
from pathlib import Path, PurePosixPath
import hashlib, re, shutil, sys, tarfile
work, tools = map(Path, sys.argv[1:])
if (work / 'sccache.sha256').exists():
    expected = (work / 'sccache.sha256').read_text().strip()
    archive = work / 'sccache-v0.18.0-aarch64-apple-darwin.tar.gz'
    if not re.fullmatch('[0-9a-f]{64}', expected) or hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
        raise SystemExit('Official sccache checksum mismatch')
for archive, binary in [('nextest.tar.gz', 'cargo-nextest'), ('sccache-v0.18.0-aarch64-apple-darwin.tar.gz', 'sccache')]:
    if not (work / archive).exists(): continue
    with tarfile.open(work / archive) as source:
        members = [m for m in source.getmembers() if PurePosixPath(m.name).name == binary and m.isfile()]
        if len(members) != 1: raise SystemExit('Unexpected official tool archive: ' + archive)
        with source.extractfile(members[0]) as content, (tools / binary).open('wb') as output:
            shutil.copyfileobj(content, output)
        (tools / binary).chmod(0o755)
TOOLS
  python3 scripts/remote-resource-guard.py --preflight --root "$snapshot"
  rm -rf "$work"; trap - EXIT
fi
packages=() targets=() swift=()
while (($#)) && [[ "$1" != -- ]]; do
  case "$1" in
    -p) packages+=(-p "$2"); shift 2 ;;
    --test) targets+=(--test "$2"); shift 2 ;;
    --swift) swift+=("$2"); shift 2 ;;
  esac
done
shift
if ((${#packages[@]} == 0 && ${#swift[@]} == 0)); then exit 0; fi
export CARGO_HOME="$base/tmp/cargo-home" CARGO_BUILD_JOBS=4 AETHER_SWIFT_JOBS=2
export AETHER_BUILD_CACHE_ROOT="$base/targets" AETHER_BUILD_CACHE_GIB=16
export SCCACHE_DIR="$base/tmp/sccache" SCCACHE_CACHE_SIZE=8G
guard=()
if ((${#packages[@]})); then
  for tool in rustc cargo-nextest sccache; do
    command -v "$tool" >/dev/null || { echo "Missing $tool; run scripts/remote-test.sh --setup" >&2; exit 2; }
  done
  export RUSTC_WRAPPER="$(command -v sccache)"
  key=$({ rustc -vV; printf '%s\n' "${RUSTFLAGS-}" "${CARGO_ENCODED_RUSTFLAGS-}"; cat Cargo.toml rust-toolchain.toml; if [[ -f .cargo/config.toml ]]; then cat .cargo/config.toml; fi; } | /usr/bin/shasum -a 256 | cut -d' ' -f1)
  export CARGO_TARGET_DIR="$AETHER_BUILD_CACHE_ROOT/aether-${commit:0:16}-${key:0:16}"
  guard=(--sccache)
fi
runner="$snapshot/tmp/remote-command-$token.sh"
trap 'rm -f "$runner"' EXIT
cat > "$runner" <<'RUNNER'
set -euo pipefail
token=$1 package_count=$2 target_count=$3 swift_count=$4; shift 4
packages=() targets=() swift=()
while ((package_count-- > 0)); do packages+=("$1"); shift; done
while ((target_count-- > 0)); do targets+=("$1"); shift; done
while ((swift_count-- > 0)); do swift+=("$1"); shift; done
if ((${#packages[@]})); then
  export AETHER_DEV_TIMING_FILE="$TMPDIR/remote-$token-rust.json"
  bash scripts/run-rust-tests.sh -- ${packages[@]+"${packages[@]}"} ${targets[@]+"${targets[@]}"} --locked "$@"
fi
if ((${#swift[@]})); then
  export AETHER_DEV_TIMING_FILE="$TMPDIR/remote-$token-swift.json"
  bash scripts/test-swift-pure.sh "${swift[@]}"
fi
RUNNER
status=0
python3 scripts/remote-resource-guard.py --root "$snapshot" ${guard[@]+"${guard[@]}"} \
  --owner-file "$snapshot/tmp/remote-owner-$token.json" \
  --report-file "$snapshot/tmp/remote-$token-resources.json" -- bash "$runner" "$token" \
  "${#packages[@]}" "${#targets[@]}" "${#swift[@]}" \
  ${packages[@]+"${packages[@]}"} ${targets[@]+"${targets[@]}"} ${swift[@]+"${swift[@]}"} "$@" || status=$?
python3 - "$snapshot/tmp" "$token" "$status" <<'TIMING'
import json, sys
from pathlib import Path
root, token, status = Path(sys.argv[1]), sys.argv[2], int(sys.argv[3])
rows = []
for kind in ('rust', 'swift'):
    path = root / ('remote-' + token + '-' + kind + '.json')
    if path.is_file(): rows.append(json.loads(path.read_text()))
resources = root / ('remote-' + token + '-resources.json')
if rows or resources.is_file():
    result = {key: sum(row.get(key, 0) for row in rows)
              for key in ('queue_seconds', 'compile_seconds', 'run_seconds', 'overhead_seconds', 'wall_seconds')}
    result.update(location='poc-m3', exit_code=status, workloads=rows)
    if resources.is_file(): result['resources'] = json.loads(resources.read_text())
    print('dev-test timing: ' + json.dumps(result))
TIMING
exit "$status"
REMOTE_TEST
remote_pid=$!
status=0
wait "$remote_pid" || status=$?
remote_pid=
python3 - "$lane/test.log" "$lane/rsync.json" <<'TRANSPORT_TIMING'
import json, sys
from pathlib import Path
sync = json.loads(Path(sys.argv[2]).read_text())
for line in Path(sys.argv[1]).read_text().splitlines():
    if line.startswith('dev-test timing: '):
        record = json.loads(line.removeprefix('dev-test timing: '))
        record.update(sync)
        line = 'dev-test timing: ' + json.dumps(record)
    print(line, flush=True)
print('Remote rsync: %.2fs' % sync['rsync_seconds'])
TRANSPORT_TIMING
printf 'Remote snapshot: %s:~/%s\nLocal log: %s/test.log\n' "$HOST" "$remote" "$lane"
exit "$status"
