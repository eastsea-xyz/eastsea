#!/usr/bin/env bash
# Run only changed Rust packages and their transitive workspace dependents.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$ROOT/tmp"
export TMPDIR="$ROOT/tmp"
selector=()
extra=()
mode=run
while (($#)); do
  case "$1" in
    --base|--metadata-file)
      [[ $# -ge 2 ]] || { echo "$1 requires a value" >&2; exit 2; }
      selector+=("$1" "$2"); shift 2 ;;
    --list) mode=list; shift ;;
    --dry-run) mode=dry; shift ;;
    -h|--help)
      echo 'Usage: scripts/test-affected.sh [--base REF] [--list|--dry-run] [-- NEXTEST_ARGS...]'
      exit 0 ;;
    --) shift; extra=("$@"); break ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done
for arg in ${extra[@]+"${extra[@]}"}; do
  case "$arg" in
    --release|--profile|--profile=*|--cargo-profile|--cargo-profile=*|--manifest-path|--manifest-path=*|--workspace|--all|--package|--package=*|-p|-p*|--exclude|--exclude=*)
      echo "Selection/profile override is not supported by test-affected: $arg" >&2
      exit 2 ;;
  esac
done
selection="$(python3 "$ROOT/scripts/affected-crates.py" ${selector[@]+"${selector[@]}"})"
if [[ "$mode" == list ]]; then
  [[ -z "$selection" ]] || printf '%s\n' "$selection"
  exit 0
fi
if [[ -z "$selection" ]]; then
  echo 'No affected Rust workspace packages; skipping compilation.'
  exit 0
fi
command=("$ROOT/scripts/run-rust-tests.sh")
node=false
packages=()
while IFS= read -r package; do
  packages+=(-p "$package")
  [[ "$package" != aether-node ]] || node=true
done <<< "$selection"
if $node; then command+=(--ram); fi
command+=(-- "${packages[@]}" ${extra[@]+"${extra[@]}"})
if [[ "$mode" == dry ]]; then printf '%q ' "${command[@]}"; printf '\n'; exit 0; fi
if ! command -v cargo-nextest >/dev/null 2>&1; then
  echo 'cargo-nextest is required; install it using the setup steps in docs/ops/dev-loop.md.' >&2
  exit 1
fi
exec "${command[@]}"
