#!/usr/bin/env bash
# Reproducible-build check (gap G5: the proving program id differed by build
# path). Copies the tree to two directories of different lengths, builds the
# release artifacts in each, and compares their SHA-256:
#
#   node       target/<profile>/aether          (the validator binary)
#   ffi        target/<profile>/libaether_ffi.a (the wallet/agent core)
#   prover     scripts/prover-program.sh's program id + the sidecar binary
#   extension  dist/eastsea-extension-<version>.zip
#
#   scripts/repro-check.sh                    # all four
#   scripts/repro-check.sh node extension
#
# Two cold builds of the whole workspace take a while and need disk for two
# target dirs; the two sides run at once unless AETHER_REPRO_SERIAL=1.
#   AETHER_REPRO_WORK=<dir>   where to put the two trees (default $TMPDIR)
#   AETHER_REPRO_KEEP=1       keep them for inspection
#   AETHER_PROFILE=<name>     cargo profile (default release)
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
profile="${AETHER_PROFILE:-release}"

want_node=0 want_ffi=0 want_prover=0 want_extension=0
targets=("$@")
[ ${#targets[@]} -eq 0 ] && targets=(all)
for t in "${targets[@]}"; do
  case "$t" in
    all) want_node=1 want_ffi=1 want_prover=1 want_extension=1 ;;
    node) want_node=1 ;;
    ffi) want_ffi=1 ;;
    prover) want_prover=1 ;;
    extension) want_extension=1 ;;
    *) echo "usage: $0 [all|node|ffi|prover|extension]..." >&2; exit 2 ;;
  esac
done

if [ "$want_prover" = 1 ]; then
  # The prover needs the Jolt fork and the jolt CLI; without them the sidecar
  # cannot build at all, so skip it instead of failing the whole check.
  if [ ! -d "${AETHER_JOLT:-/Volumes/workspace/aether-jolt}/jolt" ] || ! command -v jolt >/dev/null; then
    echo "skipping prover: the Jolt fork (aether-jolt/jolt) and the jolt CLI are needed" >&2
    want_prover=0
  fi
fi

# One timestamp for both sides, so a difference can only come from the path.
. "$repo/scripts/repro-env.sh"

work="${AETHER_REPRO_WORK:-${TMPDIR:-/tmp}}"
mkdir -p "$work" # AETHER_REPRO_WORK may point at a directory that is not there yet
work="$(mktemp -d "$work/aether-repro-XXXXXX")"
# Two checkout paths of different lengths: a build that leaked its path shows up.
a="$work/a" b="$work/bbbbbbbbbb"
keep="${AETHER_REPRO_KEEP:-}"

cleanup() {
  if [ -n "$keep" ]; then echo "kept $work" >&2; else rm -rf "$work"; fi
}
trap cleanup EXIT

# A release tarball has no .git; the flag is only here to avoid surprises.
echo "repo       $repo"
echo "profile    $profile"
echo "epoch      $SOURCE_DATE_EPOCH"
echo "work       $work"

# The .git dir is not copied (the build must not need it), and neither are the
# build outputs of this tree: each side builds cold.
copy_tree() {
  rsync -a --delete \
    --exclude '.git' --exclude 'target' --exclude 'target-*' \
    --exclude '.claude' --exclude 'dist' --exclude 'node_modules' \
    --exclude 'apps/wallet/build' --exclude '*.log' \
    "$repo/" "$1/"
}
copy_tree "$a"
copy_tree "$b"

# sha256 of a built artifact, failing the side when it is not there: a missing
# file would otherwise be recorded as an empty hash, and comparing two empty
# strings is how a broken build reads as "DIFFER" instead of as a failure.
hash_of() {
  [ -f "$1" ] || { echo "missing artifact: $1" >&2; return 1; }
  shasum -a 256 "$1" | awk '{print $1}'
}

# Build one side and leave "artifact <sha256>" lines in $dir/hashes.txt. Runs in
# a subshell so a failure only fails that side.
build_side() {
  local dir="$1" pdir
  case "$profile" in release|bench) pdir=release ;; dev) pdir=debug ;; *) pdir="$profile" ;; esac
  if (
    set -euo pipefail
    cd "$dir"
    export CARGO_TARGET_DIR="$dir/target"
    # Re-derive the remap list from *this* copy: the parent sourced repro-env.sh
    # for the original checkout, whose paths never appear in the copy, so
    # inheriting it would leave the copy's paths in the binary. SOURCE_DATE_EPOCH
    # is already exported, so both sides keep the same timestamp.
    . "$dir/scripts/repro-env.sh"
    aether_repro_rustflags
    aether_repro_link_flags

    pkgs=()
    [ "$want_node" = 1 ] && pkgs+=(-p aether-node)
    [ "$want_ffi" = 1 ] && pkgs+=(-p aether-ffi)
    if [ ${#pkgs[@]} -gt 0 ]; then
      cargo build --profile "$profile" --locked "${pkgs[@]}"
    fi
    : > "$dir/hashes.txt"
    if [ "$want_node" = 1 ]; then
      # The linked binary still carries the linker's UUID; make it follow the
      # code, as the release scripts do, or the two hashes always differ.
      aether_repro_fix_uuid "$dir/target/$pdir/aether"
      h=$(hash_of "$dir/target/$pdir/aether") || exit 1
      echo "node $h" >> "$dir/hashes.txt"
    fi
    if [ "$want_ffi" = 1 ]; then
      h=$(hash_of "$dir/target/$pdir/libaether_ffi.a") || exit 1
      echo "ffi $h" >> "$dir/hashes.txt"
    fi
    if [ "$want_prover" = 1 ]; then
      id=$(scripts/prover-program.sh)
      echo "prover-program $id" >> "$dir/hashes.txt"
      # prover-program.sh builds the sidecar and leaves it with a content UUID.
      h=$(hash_of "${CARGO_TARGET_DIR:-apps/prover/target}/release/aether-prover") || exit 1
      echo "prover-binary $h" >> "$dir/hashes.txt"
    fi
    if [ "$want_extension" = 1 ]; then
      scripts/build-extension.sh --zip >/dev/null
      h=$(hash_of "$dir"/dist/eastsea-extension-*.zip) || exit 1
      echo "extension $h" >> "$dir/hashes.txt"
    fi
  ) >"$dir/build.log" 2>&1; then
    echo 0 > "$dir/status"
  else
    echo $? > "$dir/status"
  fi
}

if [ "${AETHER_REPRO_SERIAL:-}" = 1 ]; then
  echo "building $a ..."
  build_side "$a"
  echo "building $b ..."
  build_side "$b"
else
  echo "building both sides at once (AETHER_REPRO_SERIAL=1 to serialize) ..."
  build_side "$a" &
  pa=$!
  build_side "$b" &
  pb=$!
  wait "$pa" "$pb" || true
fi

ok=1
for dir in "$a" "$b"; do
  if [ "$(cat "$dir/status")" != 0 ]; then
    echo "build failed in $dir (see $dir/build.log)" >&2
    tail -20 "$dir/build.log" >&2
    ok=0
  fi
done
[ "$ok" = 1 ] || exit 1

# Compare every artifact both sides produced.
names=()
[ "$want_node" = 1 ] && names+=(node)
[ "$want_ffi" = 1 ] && names+=(ffi)
[ "$want_prover" = 1 ] && names+=(prover-program prover-binary)
[ "$want_extension" = 1 ] && names+=(extension)
printf '\n%-16s %-8s %s\n' artifact result sha256
for name in "${names[@]}"; do
  ha=$(awk -v k="$name" '$1 == k {print $2}' "$a/hashes.txt")
  hb=$(awk -v k="$name" '$1 == k {print $2}' "$b/hashes.txt")
  if [ -n "$ha" ] && [ "$ha" = "$hb" ]; then
    printf '%-16s %-8s %s\n' "$name" "ok" "$ha"
  else
    printf '%-16s %-8s %s vs %s\n' "$name" "DIFFER" "$ha" "$hb"
    ok=0
  fi
done

if [ "$ok" = 1 ]; then
  echo
  echo "reproducible: the same source gave the same bytes from two directories"
else
  echo
  echo "NOT reproducible: see the hashes above" >&2
  exit 1
fi
