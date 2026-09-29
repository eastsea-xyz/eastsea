# shellcheck shell=bash
# Reproducible-build environment (gap G5: the proving program id differed by
# build path). Source it before any release build:
#
#   . scripts/repro-env.sh
#   aether_repro_rustflags              # adds the --remap-path-prefix list
#   aether_repro_link_flags             # -reproducible on the Darwin link
#   aether_repro_fix_uuid target/release/aether    # after the link (see below)
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

# Physical form of a path, whether or not it exists yet. cargo hands rustc some
# paths canonicalized (/tmp -> /private/tmp on macOS) and some as given, and the
# flags must not depend on which: canonicalizing only when the directory
# happens to exist would give a cold tree and a warm one (target/ already
# there) different RUSTFLAGS, so cargo would rebuild everything -- and any path
# from the target dir that reached the binary, like an OUT_DIR, would differ.
_aether_repro_canon() {
  local path="$1" head
  if [ -d "$path" ]; then (cd "$path" && pwd -P); return; fi
  head="$(dirname "$path")"
  [ "$head" = "$path" ] && { printf '%s' "$path"; return; }
  printf '%s/%s' "$(_aether_repro_canon "$head")" "$(basename "$path")"
}

# Both spellings of a prefix, so whichever form rustc sees is rewritten. rustc
# applies the *last* matching prefix, so the target dir (nested under the
# checkout) is listed after the checkout itself.
_aether_repro_remap() { # $1 path as given, $2 replacement
  local real
  real="$(_aether_repro_canon "$1")"
  printf -- ' --remap-path-prefix=%s=%s' "$1" "$2"
  [ "$real" = "$1" ] || printf -- ' --remap-path-prefix=%s=%s' "$real" "$2"
}

_aether_repro_target="${CARGO_TARGET_DIR:-$_aether_repro_root/target}"
case "$_aether_repro_target" in
  /*) ;;
  *) _aether_repro_target="$_aether_repro_root/$_aether_repro_target" ;;
esac
_aether_repro_cargo_home="${CARGO_HOME:-$HOME/.cargo}"
_aether_repro_rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"

AETHER_REMAP_FLAGS="$(_aether_repro_remap "$_aether_repro_root" /aether-node)"
AETHER_REMAP_FLAGS="$AETHER_REMAP_FLAGS$(_aether_repro_remap "$_aether_repro_cargo_home/registry/src" /cargo)"
AETHER_REMAP_FLAGS="$AETHER_REMAP_FLAGS$(_aether_repro_remap "$_aether_repro_cargo_home/git/checkouts" /cargo)"
AETHER_REMAP_FLAGS="$AETHER_REMAP_FLAGS$(_aether_repro_remap "$_aether_repro_rustup_home" /rustup)"
AETHER_REMAP_FLAGS="$AETHER_REMAP_FLAGS$(_aether_repro_remap "$_aether_repro_target" /aether-target)"
AETHER_REMAP_FLAGS="${AETHER_REMAP_FLAGS# }"
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

# Darwin link flags for a release build. Call it after aether_repro_rustflags.
#
#   -reproducible  the linker otherwise orders sections the way it happened to
#                  walk them
#
# The linker also records an LC_UUID and an ad-hoc signature page over it, and
# that pair is what is left of the path difference after the flag above:
# measured on the node, two build directories gave 48 differing bytes (the 16
# UUID bytes and 32 signature bytes), byte-identical once the UUID is rewritten
# by aether_repro_fix_uuid below. There is no flag for it -- `ld -help` lists
# none -- and -no_uuid is not an option: dyld on macOS 26 refuses to load a
# Mach-O without LC_UUID ("missing LC_UUID load command", SIGABRT), so nothing
# that has to run can carry it. In RUSTFLAGS it broke every cargo build script
# (each one is a Mach-O binary too), and the node itself has to run.
#
# wasm (scripts/build-extension.sh) gets nothing here: wasm-ld takes no
# -reproducible.
aether_repro_link_flags() {
  case "${RUSTFLAGS:-}" in *"-Wl,-reproducible"*) return 0 ;; esac
  export RUSTFLAGS="${RUSTFLAGS:-}${RUSTFLAGS:+ }-C link-arg=-Wl,-reproducible"
}

# Make a linked Mach-O byte-identical between build directories: rewrite its
# LC_UUID into a digest of the file and sign ad-hoc again, the same way on both
# sides. The signature has to be stripped first (the linker's ad-hoc signature
# covers the UUID) and re-made after (an arm64 binary with a broken signature
# does not start either). The identifier stays what the linker would have used
# -- the output file's basename -- unless one is passed.
#
#   aether_repro_fix_uuid target/release/aether [identifier]
aether_repro_fix_uuid() {
  local file="$1" id="${2:-$(basename "$1")}"
  case "$(uname -s)" in Darwin) ;; *) return 0 ;; esac
  [ -f "$file" ] || { echo "aether_repro_fix_uuid: no such file: $file" >&2; return 1; }
  codesign --remove-signature "$file" >/dev/null 2>&1 || true
  python3 "$_aether_repro_root/scripts/macho-uuid.py" rebuild "$file"
  codesign --force --sign - --identifier "$id" "$file"
}
