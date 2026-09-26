#!/usr/bin/env bash
# Adversarial audit (docs/design/12-launch-plan.md step 2).
#   scripts/audit.sh [base]      base defaults to main
# 1. tests, clippy -D warnings, long robustness fuzz, simulation soak
# 2. independent reviews of `git diff base...HEAD` by every available model CLI (codex, claude)
# 3. each model's findings verified by a different model; CONFIRMED findings fail the audit
# Reports: target/audit/<timestamp>/. Set AUDIT_FAST=1 to skip the long fuzz/soak, AUDIT_NO_MODELS=1 to skip 2-3.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
base=${1:-main}
out="target/audit/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$out"
step() { printf '\n\033[1;36m== %s\033[0m\n' "$*"; }
fail=0

step "Tests"
cargo test -q --workspace --exclude aether-core 2>&1 | tail -3 | tee "$out/tests.txt" || fail=1
step "Clippy"
cargo clippy -q --workspace --exclude aether-core --all-targets -- -D warnings 2>&1 | tee "$out/clippy.txt" | tail -5 || fail=1
[ -s "$out/clippy.txt" ] && grep -q "^error" "$out/clippy.txt" && fail=1
if [ -z "${AUDIT_FAST:-}" ]; then
  step "Robustness fuzz (200k iterations)"
  AETHER_FUZZ_ITERS=200000 cargo test -q --release -p aether-node --test robustness 2>&1 | tail -3 | tee "$out/fuzz.txt" || fail=1
  step "Simulation soak (12 seeds x 10 virtual minutes)"
  AETHER_SIM_SEEDS=12 AETHER_SIM_SECS=600 cargo test -q --release -p aether-node --test sim soak -- --ignored 2>&1 | tail -3 | tee "$out/soak.txt" || fail=1
fi

if [ -z "${AUDIT_NO_MODELS:-}" ]; then
  git diff --stat "$base...HEAD" > "$out/diff-stat.txt"
  prompt=$(sed "s/BASE/$base/g" scripts/audit/review.md)
  reviewers=()
  command -v codex >/dev/null && reviewers+=(codex)
  command -v claude >/dev/null && reviewers+=(claude)
  run_model() { # model prompt outfile
    case "$1" in
      codex) codex exec -s read-only -C "$PWD" -o "$3" "$2" >/dev/null 2>&1 ;;
      claude) claude -p "$2" --allowedTools "Read Grep Glob Bash(git diff:*) Bash(git log:*) Bash(git show:*)" > "$3" 2>/dev/null ;;
    esac
  }
  step "Independent reviews: ${reviewers[*]:-none available}"
  for m in "${reviewers[@]}"; do run_model "$m" "$prompt" "$out/review-$m.txt" & done
  wait
  step "Cross-verification"
  confirmed=0
  for m in "${reviewers[@]}"; do
    grep '^FINDING' "$out/review-$m.txt" > "$out/findings-$m.txt" || true
    [ -s "$out/findings-$m.txt" ] || { echo "$m: no findings"; continue; }
    # A different model verifies (or the same one, if it is the only reviewer).
    v=$m; for o in "${reviewers[@]}"; do [ "$o" != "$m" ] && v=$o && break; done
    vprompt="$(sed "s/BASE/$base/g" scripts/audit/verify.md)
$(nl -w1 -s'. ' "$out/findings-$m.txt")"
    run_model "$v" "$vprompt" "$out/verify-$m-by-$v.txt"
    n=$(grep -c '| CONFIRMED |' "$out/verify-$m-by-$v.txt" || true)
    echo "$m: $(wc -l < "$out/findings-$m.txt" | tr -d ' ') findings, $n confirmed by $v"
    confirmed=$((confirmed + n))
  done
  [ "$confirmed" -gt 0 ] && fail=1
fi

step "Result"
echo "reports: $out"
if [ "$fail" -ne 0 ]; then echo "AUDIT FAILED"; exit 1; fi
echo "AUDIT PASSED"
