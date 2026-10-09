#!/usr/bin/env bash
# Dev/test only. Release, staticlib and guest entry points keep their own setup.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
export PATH="$HOME/.cargo/bin:$PATH"
if [ "$(uname -s)" = Darwin ]; then
  export CARGO_BUILD_JOBS=4
else
  export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
fi
mkdir -p "$root/tmp"
if [ "${AETHER_TEST_TMP_ACTIVE:-0}" != 1 ]; then
  export TMPDIR="$root/tmp"
fi
if [ -z "${RUSTC_WRAPPER:-}" ]; then
  RUSTC_WRAPPER="$(command -v sccache || true)"
  export RUSTC_WRAPPER
fi
if [ -z "$RUSTC_WRAPPER" ]; then
  echo "sccache is required for dev builds; install it first" >&2
  exit 2
fi
case "${1:-}" in
  test|check|clippy|build|nextest) ;;
  *) echo "usage: scripts/dev-cargo.sh {test|check|clippy|build|nextest} [ARGS...]" >&2; exit 2 ;;
esac
for argument in "$@"; do
  case "$argument" in
    -r|--release|--profile|--profile=*|--cargo-profile|--cargo-profile=*|--manifest-path|--manifest-path=*|--workspace|--all|-paether-ffi|--package=aether-ffi|*apps/prover*|*jolt*|aether-ffi)
      echo "dev-cargo rejects release, alternate profiles/manifests, guest and staticlib builds: $argument" >&2
      exit 2 ;;
  esac
done
# Explicit package selection keeps the default workspace/staticlib out of this lane.
selected=0
for argument in "$@"; do
  case "$argument" in -p|--package|--package=*|-p?*) selected=1 ;; esac
done
if [ "$selected" != 1 ]; then
  echo "dev-cargo requires explicit -p CRATE; whole-workspace and staticlib gates belong to the lead" >&2
  exit 2
fi
exec python3 "$root/scripts/build-cache.py" run -- "$root/scripts/compile-gate.sh" cargo "$@"
