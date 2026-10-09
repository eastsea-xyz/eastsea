#!/usr/bin/env bash
# Snapshot portable Rust packages to the explicitly authorized Linux builder.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
HOST=poc-cuda
# Preserve this alias's user/key settings while binding only the authorized peer.
AUTHORIZED_IP=100.121.197.74
BASE=/mnt/ssd1/aether-dev
setup=0 dry=0
packages=() test_args=()
usage() { echo 'Usage: scripts/remote-test.sh [--setup] [--dry-run] -p PACKAGE [-p PACKAGE ...] [-- NEXTEST_FILTER ...]'; }
while (($#)); do
  case "$1" in
    --setup) setup=1; shift ;;
    --dry-run) dry=1; shift ;;
    -p|--package) (($# >= 2)) || { usage >&2; exit 2; }; packages+=("$2"); shift 2 ;;
    --) shift; test_args=("$@"); break ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unsupported option: $1" >&2; usage >&2; exit 2 ;;
  esac
done
for package in ${packages[@]+"${packages[@]}"}; do
  case "$package" in
    aether-types|aether-hash|aether-crypto|aether-state|aether-consensus|aether-da|aether-execution) ;;
    *) echo "Package is not approved for remote tests: $package" >&2; exit 2 ;;
  esac
done
if ((${#packages[@]} == 0 && setup == 0)); then usage >&2; exit 2; fi
# Arguments after -- are nextest filters, never Cargo options or a guest target.
for arg in ${test_args[@]+"${test_args[@]}"}; do
  [[ "$arg" != -* ]] || { echo 'Only test-name filters are accepted after --.' >&2; exit 2; }
done
command -v python3 >/dev/null
[[ -x /usr/bin/rsync ]] || { echo 'The macOS system /usr/bin/rsync is required.' >&2; exit 1; }
base=${AETHER_BUILD_BASE:-lead-merge}
commit=$(git -C "$ROOT" merge-base HEAD "$base")
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || exit 1
if ((dry)); then
  printf 'Host: %s (bound to 100.121.197.74)\nSnapshot: %s/<unique-lane>\nShared target: %s/targets/aether-%s-<compiler-config-hash>\n' "$HOST" "$BASE" "$BASE" "${commit:0:16}"
  printf 'Packages:'; printf ' %s' ${packages[@]+"${packages[@]}"}; printf '\n'
  printf 'Setup: %s; snapshot includes tracked and unignored uncommitted Rust inputs; no SSH or transfer in dry-run.\n' "$setup"
  exit 0
fi
# Read-only preflight precedes snapshot creation and every remote mutation.
ssh -o BatchMode=yes -o ConnectTimeout=10 -o "Hostname=$AUTHORIZED_IP" "$HOST" \
  'test "$(uname -s)" = Linux && test -d /mnt/ssd1' || {
  echo 'Authorized poc-cuda Linux peer is unreachable or lacks /mnt/ssd1.' >&2
  exit 1
}
mkdir -p "$ROOT/tmp"
TMPDIR="$ROOT/tmp"; export TMPDIR
lane=$(mktemp -d "$ROOT/tmp/remote-test.XXXXXXXX")
name=${lane##*/}
remote="$BASE/lanes/$name"
# Produce a source-only file list. Do not transfer application data, credentials,
# ignored files, symlinks, other worktrees, guest projects, or local build caches.
python3 - "$ROOT" "$lane/files" <<'PY'
import pathlib, re, subprocess, sys
root=pathlib.Path(sys.argv[1]); output=pathlib.Path(sys.argv[2])
files=subprocess.check_output(['git','-C',str(root),'ls-files','-z','--cached','--others','--exclude-standard']).split(b'\0')
allowed={'Cargo.toml','Cargo.lock','rust-toolchain.toml','scripts/dev-cargo.sh','scripts/build-cache.py','scripts/compile-gate.sh','scripts/compile-gate.py','scripts/run-rust-tests.sh','scripts/test-tmpdir.sh','.cargo/config.toml','.config/nextest.toml'}
deny=re.compile(r'(^|/)(?:\.git|target|tmp|\.cache|\.env[^/]*|secrets?(?:[._-][^/]*)?|credentials?(?:[._-][^/]*)?|id_rsa|id_ed25519|\.ssh|\.aws|\.claude|\.omx)(/|$)|\.(?:pem|key|p12|pfx|keystore)$',re.I)
with output.open('wb') as out:
 for raw in sorted(set(files)):
  if not raw: continue
  name=raw.decode('utf-8'); path=root/name
  if name not in allowed and not name.startswith(('crates/','legacy/','vendor/n0-mainline/')): continue
  if deny.search(name) or path.is_symlink() or not path.is_file(): continue
  if any(parent.is_symlink() for parent in path.parents if parent != root): continue
  if any(part in ('guest','guests','aether-jolt') for part in pathlib.PurePosixPath(name).parts): continue
  data=path.read_bytes()
  if re.search(rb'-----BEGIN (?:[A-Z ]*PRIVATE KEY|OPENSSH PRIVATE KEY)-----',data):
   raise SystemExit('Refusing private-key material in source snapshot: '+name)
  out.write(raw+b'\0')
PY
# Destination is generated internally; no user input is interpolated into SSH.
ssh -o BatchMode=yes -o ConnectTimeout=10 -o "Hostname=$AUTHORIZED_IP" "$HOST" "umask 077; mkdir -p '$BASE/lanes'; mkdir '$remote'"
/usr/bin/rsync -a -e "ssh -o BatchMode=yes -o ConnectTimeout=10 -o Hostname=$AUTHORIZED_IP" --from0 --files-from="$lane/files" --exclude='.git' --exclude='target' --exclude='tmp' "$ROOT/" "$HOST:$remote/"
if ((setup)); then
  ssh -o BatchMode=yes -o ConnectTimeout=10 -o "Hostname=$AUTHORIZED_IP" "$HOST" "bash -s -- '$remote'" <<'REMOTE_SETUP'
set -euo pipefail
snapshot=$1
mkdir -p "$snapshot/tmp" "$HOME/.cargo/bin"
export TMPDIR="$snapshot/tmp" PATH="$HOME/.cargo/bin:$PATH"
if ! command -v rustup >/dev/null; then
  curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o "$TMPDIR/rustup-init.sh"
  sh "$TMPDIR/rustup-init.sh" -y --no-modify-path --profile minimal
fi
cd "$snapshot"
rustup show active-toolchain >/dev/null
# Official nextest installer; pinned repository Rust toolchain drives Cargo.
if ! command -v cargo-nextest >/dev/null; then
  curl --proto '=https' --tlsv1.2 -fsSL https://get.nexte.st/latest/linux -o "$TMPDIR/nextest.tar.gz"
  tar -xzf "$TMPDIR/nextest.tar.gz" -C "$HOME/.cargo/bin"
fi
if ! command -v sccache >/dev/null; then cargo install sccache --locked; fi
cargo nextest --version
sccache --version
REMOTE_SETUP
fi
if ((${#packages[@]} == 0)); then exit 0; fi
# POSIX single-quote encoding preserves spaces and shell metacharacters in filters.
quote() { python3 -c 'import shlex,sys; print(shlex.quote(sys.argv[1]), end="")' "$1"; }
command="bash -s -- $(quote "$remote") $(quote "$commit")"
for package in ${packages[@]+"${packages[@]}"}; do command+=" $(quote "$package")"; done
command+=" --"
for arg in ${test_args[@]+"${test_args[@]}"}; do command+=" $(quote "$arg")"; done
ssh -o BatchMode=yes -o ConnectTimeout=10 -o "Hostname=$AUTHORIZED_IP" "$HOST" "$command" <<'REMOTE_TEST' 2>&1 | tee "$lane/test.log"
set -euo pipefail
snapshot=$1 commit=$2; shift 2
export PATH="$HOME/.cargo/bin:$PATH"
cd "$snapshot"
mkdir -p "$snapshot/tmp"
export TMPDIR="$snapshot/tmp"
for tool in rustc cargo-nextest sccache; do
  command -v "$tool" >/dev/null || { echo "Missing $tool; run scripts/remote-test.sh --setup -p PACKAGE" >&2; exit 1; }
done
# Same base commit/toolchain/profile shares artifacts across fresh worktree lanes.
# Include flags and Cargo config/profile manifests so incompatible settings split.
key=$({ rustc -vV; printf '%s\n' "${RUSTFLAGS-}" "${CARGO_ENCODED_RUSTFLAGS-}"; cat Cargo.toml rust-toolchain.toml; if [[ -f .cargo/config.toml ]]; then cat .cargo/config.toml; fi; } | sha256sum | cut -d' ' -f1)
export AETHER_BUILD_CACHE_ROOT=/mnt/ssd1/aether-dev/targets
export CARGO_TARGET_DIR="$AETHER_BUILD_CACHE_ROOT/aether-${commit:0:16}-${key:0:16}"
export SCCACHE_DIR=/mnt/ssd1/aether-dev/sccache
export CARGO_BUILD_JOBS=12
args=()
while (($#)) && [[ "$1" != -- ]]; do args+=(-p "$1"); shift; done
shift
started=$SECONDS
bash scripts/run-rust-tests.sh --ram -- "${args[@]}" --locked "$@" 2>&1 | tee "$snapshot/tmp/test.log"
printf 'Remote tests elapsed: %ss\n' "$((SECONDS-started))"
REMOTE_TEST
printf 'Remote snapshot: %s:%s\nLocal log: %s/test.log\n' "$HOST" "$remote" "$lane"
