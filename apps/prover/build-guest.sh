#!/usr/bin/env bash
# Build the prove_block guest ELF with the jolt CLI (riscv64imac-zero-linux-musl,
# std mode), exactly as jolt-sdk's generated `compile_prove_block` would, and copy
# it to OUT. The aether-prover host embeds that file (build.rs), so the binary
# never compiles the guest at runtime.
#
#   ./build-guest.sh [OUT]      default OUT: target-guest/prove_block.elf
#
# Needs: the `jolt` CLI (JOLT_PATH overrides) and the rustup toolchain on PATH.
# Memory parameters must match #[jolt::provable] on guest/src/lib.rs::prove_block.
# The guest compiles from the canonical stage (scripts/guest-stage.sh), not from
# the checkout: the path cargo sees is part of the program id.
set -euo pipefail
cd "$(dirname "$0")"
here="$(pwd)"
out="${1:-$here/target-guest/prove_block.elf}"
jolt_cmd="${JOLT_PATH:-jolt}"

cargo_home="${CARGO_HOME:-$HOME/.cargo}"

# When run from build.rs, drop the outer cargo's settings (they target the host).
for v in $(env | cut -d= -f1 | grep -E '^(CARGO|CARGO_.*|RUSTC.*|RUSTFLAGS|RUSTDOC.*|RUSTUP_TOOLCHAIN|TARGET|HOST|OUT_DIR|PROFILE|OPT_LEVEL|DEBUG|NUM_JOBS)$'); do
  unset "$v"
done

# The ELF is the program the proofs are about: prover and verifier must embed
# the same bytes. Strip machine-specific paths (panic locations) so a build
# does not depend on where the checkouts, cargo home or toolchain live.
script_dir="$(cd -P "$(pwd)" && pwd -P)"
repo="$(cd -P "$script_dir/../.." && pwd -P)"
# The guest is built through the canonical stage (scripts/guest-stage.sh): cargo
# hashes each path dependency's path into the metadata it mangles symbols with,
# and the Aether crates are above apps/prover's own workspace root, so building
# in the checkout directly puts the checkout's path into the guest ELF. The
# stage pins that path — and is shared mutable state, so this locks it.
. "$repo/scripts/guest-stage.sh"
stage="$(aether_guest_stage_path)"
# The guest ELF is the program the proofs are about: it must be built by the
# jolt CLI of the pinned fork revision (scripts/jolt-fork.lock), or a different
# CLI could change it. Checked here (standalone run) — the nested run through
# prover-program.sh checks it before the host build starts.
aether_jolt_cli_check "$jolt_cmd"
# build.rs runs this script while the host prover build is already staged, so
# cargo hands it the *stage* as the package root and `../..` above is the stage,
# not a checkout (a manual run from $stage/apps/prover looks the same). There the
# stage already names the real tree and its owner holds it: entering with the
# stage would repoint every link at itself.
if [ "$repo" = "$(cd -P "$stage" 2>/dev/null && pwd -P)" ]; then
  repo=""
else
  aether_guest_stage_enter "$repo"
fi
# Under the stage, so the string does not depend on how this script was reached.
target_dir="${GUEST_TARGET_DIR:-$stage/apps/prover/target-guest/aether-prover-guest-prove_block}"
# The guest's jolt sources are the stage's snapshot copy, so the fork's own
# location no longer needs to exist here; the remap below stays for anything
# that still reports it (an explicit JOLT_SRC keeps the old loud failure).
if [ -n "${JOLT_SRC:-}" ]; then
  jolt_src="$(cd "$JOLT_SRC" && pwd)"
else
  jolt_src="$(cd "$(aether_jolt_source)/jolt" 2>/dev/null && pwd -P || true)"
fi
sysroot="$(rustc --print sysroot)"
# The embedded ELF's hash is the program id: a protocol artifact that may move
# only when the guest's inputs do (scripts/guest-inputs.py), never with the
# commit it happens to be built at. So the guest gets one fixed timestamp,
# whatever the caller exported: the release scripts set SOURCE_DATE_EPOCH to the
# HEAD commit's time for the app's own artifacts (scripts/repro-env.sh), and
# inheriting that here would tie the program to every commit, docs-only ones
# included. Zeroed archive dates for the same reason.
export SOURCE_DATE_EPOCH="$AETHER_GUEST_SOURCE_DATE_EPOCH"
export ZERO_AR_DATE=1
# The inputs the id is a function of, read from the checkout the stage names
# (a nested run has no checkout of its own: the stage links lead back to it).
inputs_root="${repo:-$(dirname "$(cd -P "$stage/crates" && pwd -P)")}"
inputs="$(python3 "$inputs_root/scripts/guest-inputs.py" "$inputs_root")"
echo "build-guest: inputs $inputs" >&2
# A nested run has no checkout to remap (repo is empty, above): its sources are
# already under the stage, which is remapped in both spellings.
repo_remap=""
if [ -n "$repo" ]; then repo_remap=" --remap-path-prefix=$repo=/aether"; fi
# (rustc applies the last matching prefix, so the nested target dir goes last;
# the stage is remapped in both spellings — see scripts/repro-env.sh.)
jolt_remap=""
[ -n "$jolt_src" ] && jolt_remap=" --remap-path-prefix=$jolt_src=/jolt"
export ZEROOS_GUEST_RUSTFLAGS="$(aether_guest_stage_remap /aether)$repo_remap$jolt_remap --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$sysroot=/rustc --remap-path-prefix=$target_dir=/target"

# From the stage: the guest manifest, its `guest` member and every path
# dependency are then seen under one fixed path.
(cd "$stage/apps/prover" &&
  JOLT_FUNC_NAME=prove_block "$jolt_cmd" build -p aether-prover-guest \
    --mode std --backtrace off \
    --stack-size 4194304 --heap-size 268435456 \
    -- --release --target-dir "$target_dir" --features guest) >&2

elf="$target_dir/riscv64imac-zero-linux-musl/release/aether-prover-guest"
test -f "$elf" || { echo "build-guest: ELF not found at $elf" >&2; exit 1; }
mkdir -p "$(dirname "$out")"
cp "$elf" "$out"
echo "build-guest: $(shasum -a 256 "$out" | cut -d' ' -f1)  $out ($(wc -c < "$out" | tr -d ' ') bytes)" >&2
