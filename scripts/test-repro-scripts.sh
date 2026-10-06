#!/usr/bin/env bash
# Tests for the reproducible-build scripts (gap G5). No cargo/Xcode build: these
# check the pieces the release scripts rely on, so a regression fails fast.
#
#   scripts/test-repro-scripts.sh
#
# 1. repro-env.sh pins SOURCE_DATE_EPOCH/ZERO_AR_DATE and derives the remap list
#    from the checkout and target dir (so the flags follow the build location),
#    the same list whether or not the target dir exists yet. The Darwin link
#    flag for a reproducible section order rides along, and -no_uuid does not:
#    it made cargo's own build-script binaries unloadable (see 4).
# 2. deterministic-zip.py packs the same content to the same bytes whatever the
#    mtimes and order, and to different bytes when the content changes.
# 3. repro-app-check.sh calls two identically built bundles reproducible when
#    only their signatures or their LC_UUID differ, and not reproducible when
#    the code differs.
# 4. aether_repro_fix_uuid turns two builds whose only difference is the
#    linker's LC_UUID into the same bytes, still runnable, whatever it is run
#    on twice.
# 5. macho-uuid.py reads the UUID dwarfdump reports, blanks exactly that field
#    (it is what makes 3 work) and rebuilds it from the file (4).
# 6. guest-stage.sh, the fixed path both prover builds go through, links the
#    checkout without ever clobbering a foreign file or linking it to itself
#    (build.rs reaches the stage from inside it), snapshots the pinned Jolt fork
#    in with the manifests rewritten to it (B7: the id depends on the fork's
#    contents, not its location), and serialises two builds that share it
#    (nested ones included) — the pieces that keep the proving program id off
#    the checkout path and the fork path.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/aether-repro-test-XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "ok: $*"; }

echo "=== [1/6] repro-env.sh ==="
# Fresh shell so the parent environment cannot leak in.
env -u SOURCE_DATE_EPOCH -u ZERO_AR_DATE -u RUSTFLAGS -u AETHER_REMAP_FLAGS \
  CARGO_TARGET_DIR="$tmp/target-one" bash -c '
    set -euo pipefail
    . "$1/scripts/repro-env.sh"
    case "$SOURCE_DATE_EPOCH" in ""|*[!0-9]*) echo "bad SOURCE_DATE_EPOCH: $SOURCE_DATE_EPOCH"; exit 1 ;; esac
    [ "$ZERO_AR_DATE" = 1 ] || { echo "ZERO_AR_DATE not 1"; exit 1; }
    for p in "$1=/aether-node" "$2=/aether-target"; do
      case "$AETHER_REMAP_FLAGS" in *"$p"*) ;; *) echo "missing remap $p in: $AETHER_REMAP_FLAGS"; exit 1 ;; esac
    done
    RUSTFLAGS="--cfg keep_me"
    aether_repro_rustflags "--remap-path-prefix=/x=/y"
    case "$RUSTFLAGS" in
      "--remap-path-prefix=/x=/y --cfg keep_me"*) ;;
      *) echo "aether_repro_rustflags lost the caller flags: $RUSTFLAGS"; exit 1 ;;
    esac
  ' _ "$repo" "$tmp/target-one" || fail "repro-env.sh"
pass "pins the epoch, zeroes archive dates, keeps caller RUSTFLAGS"

# The host link flags: RUSTFLAGS reaches every unit of the build, so only
# -reproducible goes there. -no_uuid must not, however tempting: dyld on macOS
# 26 refuses to load a Mach-O without LC_UUID, and RUSTFLAGS also compiles the
# build scripts (Mach-O binaries that cargo then runs). Calling the helper twice
# must not double the flag either: RUSTFLAGS is part of cargo's fingerprint, and
# a script may source another that already called it.
link_flags() {
  env -u SOURCE_DATE_EPOCH -u ZERO_AR_DATE -u RUSTFLAGS -u AETHER_REMAP_FLAGS \
    bash -c '. "$1/scripts/repro-env.sh"; aether_repro_rustflags
      aether_repro_link_flags; aether_repro_link_flags; printf "%s\n" "$RUSTFLAGS"' _ "$repo"
}
flags="$(link_flags)"
n=$(printf '%s' "$flags" | grep -o -- "-Wl,-reproducible" | wc -l | tr -d ' ')
[ "$n" = 1 ] || fail "-Wl,-reproducible appears $n times in RUSTFLAGS: $flags"
case "$flags" in
  *"-no_uuid"*) fail "no_uuid is in RUSTFLAGS, which would break the build: $flags" ;;
esac
pass "release links get -reproducible once, and never -no_uuid"

# The remap list must not depend on whether target/ is already there: a warm
# tree would otherwise compile with different flags than a cold one (different
# paths from the target dir, e.g. an OUT_DIR, reaching the binary).
remap_flags() { # remap_flags TARGET_DIR
  env -u SOURCE_DATE_EPOCH -u ZERO_AR_DATE -u RUSTFLAGS -u AETHER_REMAP_FLAGS \
    CARGO_TARGET_DIR="$1" bash -c '. "$1/scripts/repro-env.sh"; printf "%s\n" "$AETHER_REMAP_FLAGS"' _ "$repo"
}
mkdir -p "$tmp/target-two"
warm="$(remap_flags "$tmp/target-two")"
rmdir "$tmp/target-two"
cold="$(remap_flags "$tmp/target-two")"
[ "$warm" = "$cold" ] || fail "remap list changed with the target dir present: $cold vs $warm"
case "$warm" in *"$tmp/target-two=/aether-target"*) ;; *) fail "no target remap in: $warm" ;; esac
pass "same remap list for a cold and a warm target dir"

echo "=== [2/6] deterministic-zip.py ==="
src_a="$tmp/src_a" src_b="$tmp/src_b"
mkdir -p "$src_a/src" "$src_a/wasm"
printf 'console.log("hi");\n' > "$src_a/src/a.js"
printf 'wasm-bytes\n' > "$src_a/wasm/a.wasm"
printf '{"v":1}\n' > "$src_a/manifest.json"
printf 'test\n' > "$src_a/src/a.test.js"
# Same content, different mtimes, written in a different order.
mkdir -p "$src_b"
cp "$src_a/manifest.json" "$src_b/manifest.json"
cp -R "$src_a/wasm" "$src_b/wasm"
cp -R "$src_a/src" "$src_b/src"
touch -t 202001010000 "$src_a"/manifest.json "$src_a"/src/*.js "$src_a"/src/*.test.js "$src_a"/wasm/*
touch -t 202406060606 "$src_b"/manifest.json "$src_b"/src/*.js "$src_b"/src/*.test.js "$src_b"/wasm/*

SOURCE_DATE_EPOCH=1767225600 python3 scripts/deterministic-zip.py "$tmp/a.zip" "$src_a" manifest.json src wasm
SOURCE_DATE_EPOCH=1767225600 python3 scripts/deterministic-zip.py "$tmp/b.zip" "$src_b" manifest.json src wasm
ha=$(shasum -a 256 "$tmp/a.zip" | awk '{print $1}')
hb=$(shasum -a 256 "$tmp/b.zip" | awk '{print $1}')
[ "$ha" = "$hb" ] || fail "zip differs between identical content: $ha vs $hb"
python3 - "$tmp/a.zip" <<'PY' || fail "zip contents"
import sys, zipfile
names = sorted(zipfile.ZipFile(sys.argv[1]).namelist())
want = ["manifest.json", "src/a.js", "wasm/a.wasm"]  # a.test.js excluded
assert names == want, f"{names} != {want}"
PY
# A content change must change the archive (guards against a packer that
# silently drops files).
printf 'changed\n' > "$src_b/src/a.js"
SOURCE_DATE_EPOCH=1767225600 python3 scripts/deterministic-zip.py "$tmp/c.zip" "$src_b" manifest.json src wasm
hc=$(shasum -a 256 "$tmp/c.zip" | awk '{print $1}')
[ "$ha" != "$hc" ] || fail "zip did not change with the content"
pass "same content -> same bytes, .test. left out, changes show up"

echo "=== [3/6] repro-app-check.sh ==="
cat > "$tmp/main.c" <<'EOF'
#include <stdio.h>
int main(void) { puts("aether reproducible bundle"); return 0; }
EOF
mk_bundle() { # mk_bundle DIR
  mkdir -p "$1/Contents/MacOS"
  clang -O2 "$tmp/main.c" -o "$1/Contents/MacOS/AetherWallet"
}
mk_bundle "$tmp/A.app"
mk_bundle "$tmp/B.app"
# Only the signature differs (different identifiers), as in a release vs an
# independent rebuild.
codesign -s - -i com.aether.test.one --force "$tmp/A.app/Contents/MacOS/AetherWallet" >/dev/null 2>&1
sleep 1
codesign -s - -i com.aether.test.two --force "$tmp/B.app/Contents/MacOS/AetherWallet" >/dev/null 2>&1
sa=$(shasum -a 256 "$tmp/A.app/Contents/MacOS/AetherWallet" | awk '{print $1}')
sb=$(shasum -a 256 "$tmp/B.app/Contents/MacOS/AetherWallet" | awk '{print $1}')
[ "$sa" != "$sb" ] || fail "the two signed copies should differ before stripping"
scripts/repro-app-check.sh "$tmp/A.app" "$tmp/B.app" >/dev/null || fail "repro-app-check rejected identically built bundles"
pass "signature-only difference reported reproducible"

# A build in another directory is the same code with another LC_UUID: the linker
# derives it from the link inputs (measured on the node: only the 16 UUID bytes
# and the 32 signature bytes over it differed). That must not read as a
# difference, or Tier 2 could never pass.
cp -R "$tmp/A.app" "$tmp/D.app"
python3 - "$repo/scripts/macho-uuid.py" "$tmp/D.app/Contents/MacOS/AetherWallet" <<'PY'
import importlib.util, sys
sys.dont_write_bytecode = True  # no __pycache__ in the checkout
spec = importlib.util.spec_from_file_location("macho_uuid", sys.argv[1])
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
data = bytearray(open(sys.argv[2], "rb").read())
for off in mod.uuid_offsets(data):
    data[off:off + 16] = bytes(range(16))  # some other build directory's UUID
open(sys.argv[2], "wb").write(data)
PY
# Re-sign so the bundle is what a real build produces: valid, other identity.
codesign -s - -i com.aether.test.other --force "$tmp/D.app/Contents/MacOS/AetherWallet" >/dev/null 2>&1
ua=$(scripts/macho-uuid.py print "$tmp/A.app/Contents/MacOS/AetherWallet" | sed 's/.*: //')
ud=$(scripts/macho-uuid.py print "$tmp/D.app/Contents/MacOS/AetherWallet" | sed 's/.*: //')
[ "$ua" != "$ud" ] || fail "the fixture should differ in LC_UUID: $ua"
scripts/repro-app-check.sh "$tmp/A.app" "$tmp/D.app" >/dev/null || fail "repro-app-check rejected a UUID-only difference"
pass "a UUID-only difference reported reproducible"

# Different code must be reported as a difference (exit 1).
cat > "$tmp/other.c" <<'EOF'
#include <stdio.h>
int main(void) { puts("aether bundle with a change"); return 0; }
EOF
mkdir -p "$tmp/C.app/Contents/MacOS"
clang -O2 "$tmp/other.c" -o "$tmp/C.app/Contents/MacOS/AetherWallet"
if scripts/repro-app-check.sh "$tmp/A.app" "$tmp/C.app" >"$tmp/c.log" 2>&1; then
  cat "$tmp/c.log" >&2
  fail "repro-app-check accepted two different bundles"
fi
pass "a real code difference is reported (exit 1)"

echo "=== [4/6] aether_repro_fix_uuid ==="
fix_uuid() { # fix_uuid FILE
  env -u SOURCE_DATE_EPOCH -u ZERO_AR_DATE -u RUSTFLAGS -u AETHER_REMAP_FLAGS \
    bash -c '. "$1/scripts/repro-env.sh"; aether_repro_fix_uuid "$2"' _ "$repo" "$1"
}
sha() { shasum -a 256 "$1" | awk '{print $1}'; }
# Two builds of the same code that differ only in LC_UUID and the signature page
# over it: what two build directories gave for the node (48 bytes).
mkdir -p "$tmp/x" "$tmp/yyyyyyyyyy"
clang -O2 "$tmp/main.c" -o "$tmp/x/hello"
cp "$tmp/x/hello" "$tmp/yyyyyyyyyy/hello"
python3 - "$repo/scripts/macho-uuid.py" "$tmp/yyyyyyyyyy/hello" <<'PY'
import importlib.util, sys
sys.dont_write_bytecode = True  # no __pycache__ in the checkout
spec = importlib.util.spec_from_file_location("macho_uuid", sys.argv[1])
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
data = bytearray(open(sys.argv[2], "rb").read())
for off in mod.uuid_offsets(data):
    data[off:off + 16] = bytes(range(16))
open(sys.argv[2], "wb").write(data)
PY
codesign -s - -f --identifier hello "$tmp/x/hello" "$tmp/yyyyyyyyyy/hello" >/dev/null 2>&1
[ "$(sha "$tmp/x/hello")" != "$(sha "$tmp/yyyyyyyyyy/hello")" ] || fail "the fixture should differ before fix_uuid"
fix_uuid "$tmp/x/hello"
fix_uuid "$tmp/yyyyyyyyyy/hello"
[ "$(sha "$tmp/x/hello")" = "$(sha "$tmp/yyyyyyyyyy/hello")" ] \
  || fail "fix_uuid left two builds of the same code different"
[ "$("$tmp/x/hello")" = "aether reproducible bundle" ] || fail "the fixed binary does not run"
got=$(scripts/macho-uuid.py print "$tmp/x/hello" | sed 's/.*: //')
[ "$got" != "none" ] || fail "fix_uuid left the binary without a UUID (dyld refuses those)"
[ "$got" = "$(dwarfdump --uuid "$tmp/x/hello" | awk '{print $2}')" ] || fail "the rewritten UUID does not read back"
pass "two UUID-only builds made byte-identical, still runnable"

# The sidecar and the node are walked over by more than one release script: a
# second pass must not move the bytes, or the published hash would depend on how
# many scripts ran.
before=$(sha "$tmp/x/hello")
fix_uuid "$tmp/x/hello"
[ "$(sha "$tmp/x/hello")" = "$before" ] || fail "fix_uuid is not idempotent"
# A missing artifact must fail the build, not become an empty hash.
if fix_uuid "$tmp/no-such-binary" >/dev/null 2>&1; then fail "fix_uuid accepted a missing file"; fi
pass "idempotent, and a missing binary fails"

echo "=== [5/6] macho-uuid.py ==="
uuid_bin="$tmp/uuid-bin"
clang -O2 "$tmp/main.c" -o "$uuid_bin"
got=$(scripts/macho-uuid.py print "$uuid_bin" | sed 's/.*: //')
want=$(dwarfdump --uuid "$uuid_bin" | awk '{print $2}')
[ "$got" = "$want" ] || fail "macho-uuid.py read $got, dwarfdump says $want"
cp "$uuid_bin" "$tmp/uuid-zeroed"
scripts/macho-uuid.py zero "$tmp/uuid-zeroed"
got=$(scripts/macho-uuid.py print "$tmp/uuid-zeroed" | sed 's/.*: //')
[ "$got" = "00000000-0000-0000-0000-000000000000" ] || fail "zero left $got"
n=$({ cmp -l "$uuid_bin" "$tmp/uuid-zeroed" || true; } | wc -l | tr -d ' ')
[ "$n" = 16 ] || fail "zero touched $n bytes, not just the UUID's 16"
printf 'not a mach-o\n' > "$tmp/txt"
if scripts/macho-uuid.py print "$tmp/txt" >/dev/null 2>&1; then fail "macho-uuid.py accepted a text file"; fi
# rebuild: a UUID that is a digest of the file, so the same file always gets the
# same one and a file whose UUID was blanked gets a different one than the other
# blank copy would (that is what section 4 relies on).
scripts/macho-uuid.py rebuild "$tmp/uuid-zeroed"
got=$(scripts/macho-uuid.py print "$tmp/uuid-zeroed" | sed 's/.*: //')
[ "$got" != "00000000-0000-0000-0000-000000000000" ] || fail "rebuild left the UUID zeroed"
n=$({ cmp -l "$uuid_bin" "$tmp/uuid-zeroed" || true; } | wc -l | tr -d ' ')
[ "$n" = 16 ] || fail "rebuild touched $n bytes, not just the UUID's 16"
before=$(shasum -a 256 "$tmp/uuid-zeroed" | awk '{print $1}')
scripts/macho-uuid.py rebuild "$tmp/uuid-zeroed"
[ "$(shasum -a 256 "$tmp/uuid-zeroed" | awk '{print $1}')" = "$before" ] || fail "rebuild is not idempotent"
# A binary linked without a UUID cannot be rebuilt into one: dyld would not load
# it, and the tool must say so instead of silently doing nothing (-no_uuid is
# what this check exists to keep out of the build).
clang -O2 "$tmp/main.c" -Wl,-no_uuid -o "$tmp/no-uuid" 2>/dev/null
if scripts/macho-uuid.py rebuild "$tmp/no-uuid" >/dev/null 2>&1; then fail "rebuild invented a UUID"; fi
pass "reads the UUID dwarfdump reports, blanks exactly that field, rebuilds it, rejects non-Mach-O"

echo "=== [6/6] guest-stage.sh ==="
# The stage is what makes the program id independent of the checkout (cargo
# hashes the path of a dependency that lives outside the invoking workspace
# root; see scripts/guest-stage.sh). No cargo here: the stage's own behaviour —
# the remap it offers, the links it makes and refuses to overwrite, and the lock
# that lets two builds share one stage.
stage="$tmp/stage"
mkdir -p "$stage"
stage_real="$(cd -P "$stage" && pwd -P)"
# A miniature fork stands in for aether-jolt: two git repos whose pinned
# revisions are recorded in a fixture lock file. AETHER_JOLT_LOCK redirects the
# pin the stage verifies against, exactly what a hermetic test needs.
fork="$tmp/fork"
mkdir -p "$fork/jolt/jolt-sdk" "$fork/akita/crates/akita-algebra"
printf '[package]\nname = "jolt-sdk"\nversion = "0.1.0"\n' >"$fork/jolt/jolt-sdk/Cargo.toml"
printf '[package]\nname = "akita-algebra"\nversion = "0.1.0"\n' >"$fork/akita/crates/akita-algebra/Cargo.toml"
for r in jolt akita; do
  git -C "$fork/$r" init -q
  git -C "$fork/$r" add -A
  git -C "$fork/$r" -c user.name=test -c user.email=test@example.com commit -qm "pin"
done
lock="$tmp/jolt-fork.lock"
{
  echo "jolt-commit=$(git -C "$fork/jolt" rev-parse HEAD)"
  echo "jolt-archive-sha256=$(git -C "$fork/jolt" archive --format=tar HEAD | shasum -a 256 | cut -d' ' -f1)"
  echo "akita-commit=$(git -C "$fork/akita" rev-parse HEAD)"
  echo "akita-archive-sha256=$(git -C "$fork/akita" archive --format=tar HEAD | shasum -a 256 | cut -d' ' -f1)"
} >"$lock"
run_stage() { # run_stage SHELL-BODY [ARG...]
  AETHER_GUEST_STAGE="$stage" AETHER_JOLT="$fork" AETHER_JOLT_LOCK="$lock" \
    bash -c ". '$repo/scripts/guest-stage.sh'; $1" _ "${@:2}"
}
# Both spellings of the stage go to the same place: cargo hands rustc whichever
# one it happens to have (macOS /tmp -> /private/tmp), and a missing one leaks
# the builder's path into the ELF.
map="$(run_stage 'aether_guest_stage_remap /aether')"
case "$map" in *"--remap-path-prefix=$stage=/aether"*) ;; *) fail "no remap for the stage: $map" ;; esac
if [ "$stage_real" != "$stage" ]; then
  case "$map" in *"--remap-path-prefix=$stage_real=/aether"*) ;; *) fail "no remap for the physical stage: $map" ;; esac
fi
pass "remaps the stage, in both spellings"

# populate: a link per entry that exists, nothing for the ones that do not
# (the repo may have no .cargo/), and re-pointing when the tree moves — the
# stage is one path for every checkout, so it must follow the tree being built.
tree_a="$tmp/tree-a" tree_b="$tmp/tree-b"
mkdir -p "$tree_a/apps/prover" "$tree_a/crates" "$tree_b/apps/prover"
printf 'a\n' >"$tree_a/Cargo.toml"
printf 'b\n' >"$tree_b/Cargo.toml"
tree_b_real="$(cd -P "$tree_b" && pwd -P)"
run_stage 'aether_guest_stage_populate "$1"' "$tree_a"
for e in Cargo.toml crates; do
  [ -L "$stage/$e" ] || fail "populate did not link $e"
done
if [ -e "$stage/vendor" ] || [ -L "$stage/vendor" ]; then fail "populate invented vendor/"; fi
if [ ! -d "$stage/apps/prover" ] || [ -L "$stage/apps/prover" ]; then
  fail "apps/prover must be a real directory (it is the workspace root cargo hashes)"
fi
[ -L "$stage/apps/prover/target" ] || fail "the build output directory is not shared with the checkout"
run_stage 'aether_guest_stage_populate "$1"' "$tree_a"
[ -L "$stage/Cargo.toml" ] || fail "populate is not idempotent"
run_stage 'aether_guest_stage_populate "$1"' "$tree_b" || fail "populate failed on the second tree"
# Links point at the physical tree (populate resolves it), which is also the
# only form that works when the checkout is reached through a symlink.
[ "$(readlink "$stage/Cargo.toml")" = "$tree_b_real/Cargo.toml" ] || fail "populate did not re-point the stage"
# A build that reaches the stage from inside it — build.rs running
# build-guest.sh, which cargo starts with the stage as the package root — names
# the stage as its checkout. Linking that points every entry at itself and the
# build then finds no workspace root, so the stage must stay as it is.
run_stage 'aether_guest_stage_populate "$1"' "$stage" || fail "populate failed on the stage itself"
[ "$(readlink "$stage/Cargo.toml")" = "$tree_b_real/Cargo.toml" ] ||
  fail "populating from inside the stage pointed it at itself"
run_stage 'AETHER_GUEST_STAGE_LOCKED=1 aether_guest_stage_enter "$1"' "$stage" ||
  fail "enter refused the stage itself"
[ "$(readlink "$stage/Cargo.toml")" = "$tree_b_real/Cargo.toml" ] ||
  fail "entering from inside the stage pointed it at itself"
pass "links what exists, idempotent, follows the tree, never links to itself"

# B7: the fork — pinned revision, staged copy, rewritten manifests. With
# manifests present, populate copies them (not links: they must differ from the
# checkout's, which keep the absolute fork path for the developer workflow) with
# every fork reference moved inside the stage, and snapshots the pinned fork.
mkdir -p "$tree_a/apps/prover/guest/src" "$tree_a/apps/prover/src"
cat >"$tree_a/apps/prover/Cargo.toml" <<EOF
[package]
name = "aether-prover"
[dependencies]
jolt-sdk = { path = "/Volumes/workspace/aether-jolt/jolt/jolt-sdk" }
[patch."https://github.com/markosg04/akita"]
akita-algebra = { path = "/Volumes/workspace/aether-jolt/akita/crates/akita-algebra" }
EOF
cat >"$tree_a/apps/prover/guest/Cargo.toml" <<EOF
[package]
name = "aether-prover-guest"
[dependencies]
jolt = { package = "jolt-sdk", path = "/Volumes/workspace/aether-jolt/jolt/jolt-sdk" }
EOF
printf 'fn main() {}\n' >"$tree_a/apps/prover/src/main.rs"
run_stage 'aether_guest_stage_populate "$1"' "$tree_a" || fail "populate failed with manifests"
[ -f "$stage/apps/prover/Cargo.toml" ] && [ ! -L "$stage/apps/prover/Cargo.toml" ] ||
  fail "the staged prover manifest is not a copy"
grep -q 'path = "../../aether-jolt/jolt/jolt-sdk"' "$stage/apps/prover/Cargo.toml" ||
  fail "the staged prover manifest does not reference the staged fork"
grep -q 'path = "../../aether-jolt/akita/crates/akita-algebra"' "$stage/apps/prover/Cargo.toml" ||
  fail "a [patch] fork reference was not rewritten"
[ -d "$stage/apps/prover/guest" ] && [ ! -L "$stage/apps/prover/guest" ] ||
  fail "guest must be a real directory (its manifest is rewritten)"
grep -q 'path = "../../../aether-jolt/jolt/jolt-sdk"' "$stage/apps/prover/guest/Cargo.toml" ||
  fail "the staged guest manifest was not rewritten"
[ -f "$stage/aether-jolt/jolt/jolt-sdk/Cargo.toml" ] || fail "the pinned fork was not snapshotted"
[ -f "$stage/aether-jolt/.aether-stage-snapshot" ] || fail "the snapshot is not marked"
# Idempotent: the copies do not churn between builds (mtime is a cargo input).
sum_before="$(shasum -a 256 "$stage/apps/prover/Cargo.toml" | cut -d' ' -f1)"
run_stage 'aether_guest_stage_populate "$1"' "$tree_a" || fail "the second populate failed"
[ "$(shasum -a 256 "$stage/apps/prover/Cargo.toml" | cut -d' ' -f1)" = "$sum_before" ] ||
  fail "populate is not idempotent on the manifest"
pass "copies the manifests with the fork staged, snapshots the pinned fork"

# AETHER_JOLT is a developer override, not a second program id: the fork staged
# from a copy at another path is the same snapshot.
fork2="$tmp/fork-elsewhere"
mkdir -p "$fork2"
cp -R "$fork/jolt" "$fork2/jolt"
cp -R "$fork/akita" "$fork2/akita"
rm -rf "$stage/aether-jolt"
AETHER_GUEST_STAGE="$stage" AETHER_JOLT="$fork2" AETHER_JOLT_LOCK="$lock" \
  bash -c ". '$repo/scripts/guest-stage.sh'; aether_guest_stage_populate \"\$1\"" _ "$tree_a" ||
  fail "populate refused a fork at another path"
[ -f "$stage/aether-jolt/jolt/jolt-sdk/Cargo.toml" ] ||
  fail "the stage did not snapshot the fork from the override path"
pass "stages the same snapshot from a fork at another path"

# A fork that moved past the pin is refused, not silently built into an id
# nobody else can reproduce.
printf 'moved on\n' >>"$fork/jolt/jolt-sdk/Cargo.toml"
git -C "$fork/jolt" add -A
git -C "$fork/jolt" -c user.name=test -c user.email=test@example.com commit -qm "moved"
if run_stage 'aether_guest_stage_populate "$1"' "$tree_a" >/dev/null 2>&1; then
  fail "populate accepted a fork at an unpinned commit"
fi
git -C "$fork/jolt" reset -q --hard HEAD~1
# Uncommitted changes do not reach the snapshot (it is taken by commit), and do
# not break the build either: the dirty filetree is simply not what is staged.
rm -rf "$stage/aether-jolt"
printf 'dirty\n' >>"$fork/jolt/jolt-sdk/Cargo.toml"
run_stage 'aether_guest_stage_populate "$1"' "$tree_a" || fail "a dirty fork worktree broke populate"
if grep -q '^dirty$' "$stage/aether-jolt/jolt/jolt-sdk/Cargo.toml"; then
  fail "uncommitted changes leaked into the snapshot"
fi
git -C "$fork/jolt" checkout -q -- jolt-sdk/Cargo.toml
pass "refuses an unpinned fork, ignores a dirty worktree"

# The jolt CLI must be one built from the pinned revision: --version names the
# commit (however many digits that build's git abbreviated to), and anything
# else is refused with the fix.
jolt_good="$tmp/jolt-good"
printf '#!/bin/sh\necho "jolt 0.1.0 (%s 2026-10-06)"\n' \
  "$(git -C "$fork/jolt" rev-parse --short=7 HEAD)" >"$jolt_good"
chmod +x "$jolt_good"
run_stage 'aether_jolt_cli_check "$1"' "$jolt_good" || fail "the pinned CLI was refused"
for bad_hash in deadbeef0 nohash; do
  jolt_bad="$tmp/jolt-$bad_hash"
  if [ "$bad_hash" = nohash ]; then
    printf '#!/bin/sh\necho "jolt 0.1.0"\n' >"$jolt_bad"
  else
    printf '#!/bin/sh\necho "jolt 0.1.0 (%s 2026-10-06)"\n' "$bad_hash" >"$jolt_bad"
  fi
  chmod +x "$jolt_bad"
  if run_stage 'aether_jolt_cli_check "$1"' "$jolt_bad" >/dev/null 2>&1; then
    fail "a jolt CLI named '$bad_hash' passed the pin check"
  fi
done
pass "checks the jolt CLI against the pin"

# A real file where a link belongs means someone else owns that path: refuse,
# and leave it there.
rm -rf "$stage"
mkdir -p "$stage"
printf 'mine\n' >"$stage/Cargo.toml"
if run_stage 'aether_guest_stage_populate "$1"' "$tree_a" >/dev/null 2>&1; then
  fail "populate replaced a real file"
fi
[ "$(cat "$stage/Cargo.toml")" = mine ] || fail "populate clobbered the file it refused"
if run_stage 'aether_guest_stage_enter "$1"' "$tmp/no-such-checkout" >/dev/null 2>&1; then
  fail "enter accepted a missing checkout"
fi
pass "refuses to replace what it did not create"

# The lock: one stage, several builds. A second build waits for the first, and
# a nested one (host build -> build.rs -> build-guest.sh) does not wait at all:
# it inherits its parent's lock through AETHER_GUEST_STAGE_LOCKED, which it must
# leave alone.
rm -rf "$stage" "$stage.lock"
mkdir -p "$stage"
run_stage 'aether_guest_stage_lock; sleep 2; aether_guest_stage_unlock' &
holder=$!
for _ in $(seq 40); do
  [ -e "$stage.lock/owner" ] && break
  sleep 0.1
done
[ -e "$stage.lock/owner" ] || fail "the first build never took the lock"
[ "$(awk '{print $2}' "$stage.lock/owner")" = "$(hostname)" ] || fail "the lock does not name its host"
# This one can only answer after the holder let go (2s), not before.
started=$(date +%s)
if [ "$(run_stage 'aether_guest_stage_lock; echo took-it; aether_guest_stage_unlock')" != took-it ]; then
  fail "the second build never got the lock"
fi
elapsed=$(($(date +%s) - started))
[ "$elapsed" -ge 1 ] || fail "the second build did not wait for the first (${elapsed}s)"
wait "$holder" || fail "the lock holder failed"
if [ -e "$stage.lock" ]; then fail "the lock outlived its holders"; fi
# A nested build must not release the lock it did not take, nor block on it.
run_stage 'aether_guest_stage_lock; sleep 2; aether_guest_stage_unlock' &
holder=$!
for _ in $(seq 40); do
  [ -e "$stage.lock/owner" ] && break
  sleep 0.1
done
nested="$(run_stage 'AETHER_GUEST_STAGE_LOCKED=1 aether_guest_stage_enter "$1"
  [ -d "$AETHER_GUEST_STAGE.lock" ] && echo still-held' "$tree_a")"
[ "$nested" = still-held ] || fail "a nested build touched its parent's lock ($nested)"
wait "$holder" || fail "the second lock holder failed"
# The wait budget is a safety valve for a holder that is alive but stuck: a
# waiting build gives up (AETHER_GUEST_STAGE_LOCK_WAIT seconds, default 2h)
# instead of hanging forever.
run_stage 'aether_guest_stage_lock; sleep 3; aether_guest_stage_unlock' &
holder=$!
for _ in $(seq 40); do
  [ -e "$stage.lock/owner" ] && break
  sleep 0.1
done
if run_stage 'AETHER_GUEST_STAGE_LOCK_WAIT=1 aether_guest_stage_lock' >/dev/null 2>&1; then
  fail "a build waited past its lock budget"
fi
wait "$holder" || fail "the budgeted lock holder failed"
if [ -e "$stage.lock" ]; then fail "the budgeted holder's lock outlived it"; fi
# A holder that died (same host, pid gone) must not block the next build.
mkdir -p "$stage.lock"
echo "999999 $(hostname)" >"$stage.lock/owner"
run_stage 'aether_guest_stage_lock; aether_guest_stage_unlock' || fail "a stale lock was not recovered"
if [ -e "$stage.lock" ]; then fail "unlock left the lock"; fi
pass "serialises builds, tolerates nesting, recovers a dead holder, gives up on a stuck one"

echo
echo "all reproducible-build script tests passed"
