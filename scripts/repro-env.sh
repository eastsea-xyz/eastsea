# shellcheck shell=bash
# Reproducible-build environment (gap G5: the proving program id differed by
# build path). Source it before any release build:
#
#   . scripts/repro-env.sh
#   aether_repro_rustflags              # adds the --remap-path-prefix list
#
# It pins the timestamp (SOURCE_DATE_EPOCH), zeroes archive member dates
# (ZERO_AR_DATE=1) and exports AETHER_REMAP_FLAGS: the --remap-path-prefix list
# that keeps the builder's paths (checkout, cargo home, rustup, target dir) out
# of binaries and archives. With it the same source gives the same bytes in any
# directory; scripts/repro-check.sh builds twice to prove it.
# https://reproducible-builds.org/docs/source-date-epoch/

# pwd -P: cargo canonicalizes the paths it hands rustc (/tmp -> /private/tmp on
# macOS), so the remap has to be built from the physical path or it misses.
_aether_repro_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

# One timestamp for every artifact of a release: the commit's, so a rebuild by
# anyone else lands on the same second. No .git (release tarball) -> a fixed
# date, which is just as reproducible.
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
  if SOURCE_DATE_EPOCH=$(git -C "$_aether_repro_root" log -1 --pretty=%ct 2>/dev/null) && [ -n "$SOURCE_DATE_EPOCH" ]; then
    export SOURCE_DATE_EPOCH
  else
    export SOURCE_DATE_EPOCH=1767225600 # 2026-01-01T00:00:00Z
  fi
fi

# Apple's ar/libtool otherwise stamp archive members with the build time, so a
# .a rebuilds differently every second (libaether_ffi.a).
export ZERO_AR_DATE=1

# rustc applies the *last* matching prefix, so a target dir nested under the
# checkout is remapped after the checkout itself.
_aether_repro_target="${CARGO_TARGET_DIR:-$_aether_repro_root/target}"
case "$_aether_repro_target" in
  /*) ;;
  *) _aether_repro_target="$_aether_repro_root/$_aether_repro_target" ;;
esac
if [ -d "$_aether_repro_target" ]; then
  _aether_repro_target="$(cd "$_aether_repro_target" && pwd -P)"
fi
_aether_repro_cargo_home="${CARGO_HOME:-$HOME/.cargo}"
_aether_repro_rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
if [ -d "$_aether_repro_cargo_home" ]; then
  _aether_repro_cargo_home="$(cd "$_aether_repro_cargo_home" && pwd -P)"
fi
if [ -d "$_aether_repro_rustup_home" ]; then
  _aether_repro_rustup_home="$(cd "$_aether_repro_rustup_home" && pwd -P)"
fi

AETHER_REMAP_FLAGS="--remap-path-prefix=$_aether_repro_root=/aether-node"
AETHER_REMAP_FLAGS="$AETHER_REMAP_FLAGS --remap-path-prefix=$_aether_repro_cargo_home/registry/src=/cargo"
AETHER_REMAP_FLAGS="$AETHER_REMAP_FLAGS --remap-path-prefix=$_aether_repro_cargo_home/git/checkouts=/cargo"
AETHER_REMAP_FLAGS="$AETHER_REMAP_FLAGS --remap-path-prefix=$_aether_repro_rustup_home=/rustup"
AETHER_REMAP_FLAGS="$AETHER_REMAP_FLAGS --remap-path-prefix=$_aether_repro_target=/aether-target"
export AETHER_REMAP_FLAGS

# Append the remap list to RUSTFLAGS, keeping anything the caller set (e.g. the
# extension's getrandom --cfg) and any extra flags passed here in front.
aether_repro_rustflags() {
  local extra="${1:-}"
  # Calling it twice (a script sourcing another that already did) must not
  # append the list again.
  case "${RUSTFLAGS:-}" in
    *"$AETHER_REMAP_FLAGS"*)
      export RUSTFLAGS="$extra${extra:+ }${RUSTFLAGS}"
      return 0
      ;;
  esac
  export RUSTFLAGS="$extra${extra:+ }${RUSTFLAGS:-}${RUSTFLAGS:+ }$AETHER_REMAP_FLAGS"
}
