#!/usr/bin/env bash
# Sequential/nested compiles share a slot held by their live caller or ancestor.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd -P)
mkdir -p "$root/tmp"
gate_dir="$HOME/.claude/playbooks/aether-team"
gate="$gate_dir/wait-compile.sh"

ancestors=""
ancestor=$PPID
while true; do
  case "$ancestor" in ''|*[!0-9]*) break;; esac
  [ "$ancestor" -gt 1 ] || break
  kill -0 "$ancestor" 2>/dev/null || break
  ancestors="$ancestors $ancestor "
  next=$(/bin/ps -p "$ancestor" -o ppid= 2>/dev/null | tr -d '[:space:]') || break
  [ "$next" != "$ancestor" ] || break
  ancestor=$next
done

for slot in "$gate_dir/compile-sem"/slot-*; do
  [ -d "$slot" ] || continue
  owner=$(cat "$slot/pid" 2>/dev/null) || continue
  worktree=$(cat "$slot/worktree" 2>/dev/null) || continue
  [ "$worktree" = "$root" ] || continue
  case "$owner" in ''|*[!0-9]*) continue;; esac
  case "$ancestors" in
    *" $owner "*)
      if kill -0 "$owner" 2>/dev/null; then
        if [ "$#" -gt 0 ]; then
          export AETHER_COMPILE_GATE_HELD=1
          exec "$@"
        fi
        exit 0
      fi;;
  esac
done

if [ "$#" -gt 0 ]; then
  exec python3 "$root/scripts/compile-gate.py" "$@"
fi
[ -x "$gate" ] || { echo "FAIL compile gate is missing: $gate" >&2; exit 1; }
# Keep the calling shell as the slot owner; a short-lived helper would leave
# only the gate's 120-second retention to cover the subsequent compiler.
exec "$gate"
