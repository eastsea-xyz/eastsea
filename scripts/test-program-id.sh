#!/usr/bin/env bash
# The proving program id is a protocol artifact: validators verify against it,
# so it may move only when the guest's inputs move (scripts/guest-inputs.py),
# never with the commit it is built at. Since 2026-09-29 it moved with every
# commit and no shipped app's proofs verified (.claude/team/prover-070-mismatch.md).
#
#   scripts/test-program-id.sh            fast: a stand-in jolt CLI, no compile
#   scripts/test-program-id.sh --build    also two real builds of the guest
#                                         (apps/prover/build-guest.sh, cold, minutes)
#
# The stand-in CLI turns everything a guest compile can see into its "ELF":
# the timestamp the build exports and the staged sources of the Aether crates
# and the guest, by path relative to the stage. It is an oracle independent of
# guest-inputs.py: if a commit can reach the guest through the build, it reaches
# the stand-in's output too.
#
# AETHER_TEST_BUILD_GUEST=<file> runs the checks against another build-guest.sh
# (e.g. `git show <old>:apps/prover/build-guest.sh`) to show what fails there.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$(pwd -P)"
build=0
[ "${1:-}" = "--build" ] && build=1
tmp="$(mktemp -d "${TMPDIR:-/tmp}/aether-program-id-XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "ok: $*"; }
export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=advice.detachedHead GIT_CONFIG_VALUE_0=false
git_t() { git -C "$1" -c user.name=test -c user.email=test@example.com "${@:2}"; }

# --- a repository with the guest's sources -----------------------------------
# Only what the guest build and the checks need, from this working tree (so the
# scripts under test are the ones on disk, committed or not).
src="$tmp/src"
mkdir -p "$src"
(
  git ls-files -co --exclude-standard -- Cargo.toml README.md crates/crypto crates/hash crates/proving \
    crates/execution crates/state crates/types crates/node/Cargo.toml apps/prover scripts/guest-stage.sh \
    scripts/guest-inputs.py scripts/jolt-fork.lock apps/wallet/Sources/HealthCheck.swift
) | grep -v -E '^apps/prover/(target|target-guest)/' | tar -cf - -T - | tar -xf - -C "$src"
if [ -n "${AETHER_TEST_BUILD_GUEST:-}" ]; then
  cp "$AETHER_TEST_BUILD_GUEST" "$src/apps/prover/build-guest.sh"
  echo "(testing build-guest.sh from $AETHER_TEST_BUILD_GUEST)"
fi
chmod +x "$src/apps/prover/build-guest.sh" "$src/scripts/guest-inputs.py"
git_t "$src" init -q
git_t "$src" add -A
GIT_COMMITTER_DATE="2026-09-29T00:00:00Z" GIT_AUTHOR_DATE="2026-09-29T00:00:00Z" git_t "$src" commit -qm base
base="$(git -C "$src" rev-parse HEAD)"

# --- a miniature pinned fork and a stand-in jolt CLI -------------------------
fork="$tmp/fork"
mkdir -p "$fork/jolt/jolt-sdk" "$fork/akita/crates/akita-algebra"
printf '[package]\nname = "jolt-sdk"\nversion = "0.1.0"\n' >"$fork/jolt/jolt-sdk/Cargo.toml"
printf '[package]\nname = "akita-algebra"\nversion = "0.1.0"\n' >"$fork/akita/crates/akita-algebra/Cargo.toml"
for r in jolt akita; do
  git_t "$fork/$r" init -q
  git_t "$fork/$r" add -A
  git_t "$fork/$r" commit -qm pin
done
lock="$tmp/jolt-fork.lock"
{
  echo "jolt-commit=$(git -C "$fork/jolt" rev-parse HEAD)"
  echo "jolt-archive-sha256=$(git -C "$fork/jolt" archive --format=tar HEAD | shasum -a 256 | cut -d' ' -f1)"
  echo "akita-commit=$(git -C "$fork/akita" rev-parse HEAD)"
  echo "akita-archive-sha256=$(git -C "$fork/akita" archive --format=tar HEAD | shasum -a 256 | cut -d' ' -f1)"
} >"$lock"
cli="$tmp/jolt"
cat >"$cli" <<EOF
#!/usr/bin/env bash
set -euo pipefail
if [ "\${1:-}" = "--version" ]; then echo "jolt 0.1.0 ($(git -C "$fork/jolt" rev-parse --short=7 HEAD) 2026-10-07)"; exit 0; fi
target=""; prev=""
for a in "\$@"; do [ "\$prev" = "--target-dir" ] && target="\$a"; prev="\$a"; done
[ -n "\$target" ] || { echo "stand-in jolt: no --target-dir" >&2; exit 1; }
out="\$target/riscv64imac-zero-linux-musl/release/aether-prover-guest"
mkdir -p "\$(dirname "\$out")"
# Run from <stage>/apps/prover, as the real CLI is.
{
  echo "SOURCE_DATE_EPOCH=\${SOURCE_DATE_EPOCH:-} ZERO_AR_DATE=\${ZERO_AR_DATE:-}"
  cd ../..
  find -L crates/crypto crates/hash crates/proving crates/execution crates/state crates/types apps/prover/guest apps/prover/Cargo.toml apps/prover/Cargo.lock apps/prover/patches -type f \
    -not -path '*/tests/*' -not -path '*/target*' 2>/dev/null | LC_ALL=C sort | while read -r f; do
    printf '%s %s\n' "\$f" "\$(shasum -a 256 <"\$f" | cut -d' ' -f1)"
  done
} >"\$out"
EOF
chmod +x "$cli"

# Build the guest of commit $1 from checkout $2 and print its program id.
program_id() {
  local commit="$1" tree="$2"
  git -C "$tree" -c advice.detachedHead=false checkout -q "$commit"
  rm -rf "$tree/apps/prover/target-guest"
  (cd "$tree" && env -u RUSTFLAGS AETHER_GUEST_STAGE="$tmp/stage" AETHER_JOLT="$fork" \
    AETHER_JOLT_LOCK="$lock" JOLT_PATH="$cli" AETHER_GUEST_RUSTC_VV="rustc 1.95.0 (test)" \
    "${@:3}" apps/prover/build-guest.sh "$tmp/out.elf" 2>"$tmp/build.log") ||
    { cat "$tmp/build.log" >&2; fail "build-guest.sh failed at $commit"; }
  [ -z "${AETHER_TEST_VERBOSE:-}" ] || head -1 "$tmp/out.elf" >&2
  shasum -a 256 "$tmp/out.elf" | cut -d' ' -f1
}
inputs() {
  git -C "$2" -c advice.detachedHead=false checkout -q "$1"
  (cd "$2" && AETHER_JOLT_LOCK="$lock" AETHER_GUEST_RUSTC_VV="rustc 1.95.0 (test)" python3 scripts/guest-inputs.py "$2")
}

# --- the commits ---------------------------------------------------------------
# B: docs and wallet only, a week later.
printf '\nA docs-only change.\n' >>"$src/README.md"
printf '\n// a wallet-only change\n' >>"$src/apps/wallet/Sources/HealthCheck.swift"
git_t "$src" add -A
GIT_COMMITTER_DATE="2026-10-07T00:00:00Z" GIT_AUTHOR_DATE="2026-10-07T00:00:00Z" git_t "$src" commit -qm "docs: wallet and readme only"
docs="$(git -C "$src" rev-parse HEAD)"
# N: a node-only change (crates/node is not in the guest).
printf '\n# a node-only change\n' >>"$src/crates/node/Cargo.toml"
git_t "$src" add -A
GIT_COMMITTER_DATE="2026-10-08T00:00:00Z" git_t "$src" commit -qm "node only"
node="$(git -C "$src" rev-parse HEAD)"
# E: aether-execution changes (from base).
git -C "$src" -c advice.detachedHead=false checkout -q "$base"
printf '\n// an execution change\npub const _GUEST_INPUT_TEST: u8 = 1;\n' >>"$src/crates/execution/src/lib.rs"
git_t "$src" add -A
GIT_COMMITTER_DATE="2026-09-30T00:00:00Z" git_t "$src" commit -qm "execution change"
exec_commit="$(git -C "$src" rev-parse HEAD)"

echo "=== [1/3] guest-inputs.py: the inputs follow the guest, not the commit ==="
in_base="$(inputs "$base" "$src")"
[ ${#in_base} = 64 ] || fail "guest-inputs.py printed no digest: $in_base"
[ "$(inputs "$docs" "$src")" = "$in_base" ] || fail "a docs/wallet-only commit changed the guest inputs"
[ "$(inputs "$node" "$src")" = "$in_base" ] || fail "a node-only commit changed the guest inputs"
[ "$(inputs "$exec_commit" "$src")" != "$in_base" ] || fail "an aether-execution change did not change the guest inputs"
other="$tmp/elsewhere/checkout"
mkdir -p "$(dirname "$other")"
git clone -q "$src" "$other"
[ "$(inputs "$base" "$other")" = "$in_base" ] || fail "the guest inputs depend on the checkout path"
pass "docs/wallet/node-only commits keep the inputs; aether-execution moves them; any checkout path"
git -C "$src" -c advice.detachedHead=false checkout -q "$base"
cp "$src/apps/prover/Cargo.lock" "$tmp/Cargo.lock.orig"
# A different checksum for revm-primitives (in the closure through aether-execution).
python3 - "$src/apps/prover/Cargo.lock" <<'PY'
import re, sys
p = sys.argv[1]
s = open(p).read()
s2 = re.sub(r'(name = "revm-primitives"\n(?:[^\[].*\n)*?checksum = ")[0-9a-f]', r'\g<1>x', s, count=1)
assert s2 != s, "no revm-primitives checksum in Cargo.lock"
open(p, "w").write(s2)
PY
[ "$(inputs "$base" "$src")" != "$in_base" ] || fail "a Cargo.lock change in the guest's closure did not change the inputs"
cp "$tmp/Cargo.lock.orig" "$src/apps/prover/Cargo.lock"
printf '# moved\n' >>"$lock"
[ "$(inputs "$base" "$src")" != "$in_base" ] || fail "a Jolt fork pin change did not change the inputs"
sed -i '' -e '$d' "$lock"
[ "$(AETHER_GUEST_RUSTC_VV="rustc 1.96.0 (test)" python3 "$src/scripts/guest-inputs.py" "$src")" != "$in_base" ] ||
  fail "a toolchain change did not change the inputs"
[ "$(inputs "$base" "$src")" = "$in_base" ] || fail "restoring the inputs did not restore the digest"
pass "a closure Cargo.lock entry, the fork pin and the toolchain are inputs"

echo "=== [2/3] build-guest.sh: the program id follows the inputs, not the commit ==="
id_base="$(program_id "$base" "$src")"
id_docs="$(program_id "$docs" "$src")"
[ "$id_docs" = "$id_base" ] ||
  fail "a docs/wallet-only commit changed the program id ($id_base -> $id_docs): the commit reaches the guest build"
[ "$(program_id "$node" "$src")" = "$id_base" ] || fail "a node-only commit changed the program id"
[ "$(program_id "$docs" "$src" SOURCE_DATE_EPOCH=1791000000)" = "$id_base" ] ||
  fail "an exported SOURCE_DATE_EPOCH (release-mac.sh, builder-build.sh) reached the guest"
[ "$(program_id "$docs" "$other")" = "$id_base" ] || fail "another checkout path changed the program id"
[ "$(program_id "$exec_commit" "$src")" != "$id_base" ] || fail "an aether-execution change did not change the program id"
pass "same id for docs/wallet/node-only commits, an exported epoch and another path; aether-execution moves it"

echo "=== [3/3] real guest builds ==="
if [ "$build" != 1 ]; then
  echo "skipped (pass --build: two cold guest builds through apps/prover/build-guest.sh)"
  exit 0
fi
# The real CLI and fork, two commits of this repository's tree that differ only
# outside the guest's inputs, each built cold from its own checkout path.
# Under AETHER_TEST_REAL_DIR when set: each cold guest build leaves a target dir.
real="${AETHER_TEST_REAL_DIR:-$tmp}/real-$$"
for side in a b; do
  mkdir -p "$real/$side"
  git -C "$repo" worktree add -q --detach "$real/$side/src" HEAD
done
trap 'git -C "$repo" worktree remove --force "$real/a/src" 2>/dev/null; git -C "$repo" worktree remove --force "$real/b/src" 2>/dev/null; rm -rf "$tmp" "$real"' EXIT
# Side b gets a docs-only commit on top, a later commit time.
printf '\nA docs-only change for the program id check.\n' >>"$real/b/src/README.md"
git_t "$real/b/src" commit -qam "docs: program id check"
for side in a b; do
  (cd "$real/$side/src" && apps/prover/build-guest.sh "$real/$side/guest.elf")
  echo "$side $(git -C "$real/$side/src" log -1 --format='%h %cI') $(shasum -a 256 "$real/$side/guest.elf" | cut -d' ' -f1)"
done
[ "$(shasum -a 256 <"$real/a/guest.elf")" = "$(shasum -a 256 <"$real/b/guest.elf")" ] ||
  fail "two commits that leave the guest's inputs alone built different guests"
pass "real guest builds: same program id on two commits from two paths"
