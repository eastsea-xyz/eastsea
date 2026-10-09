#!/bin/bash
# launchd restarts unsuccessful exits. Reserve successful exit for the node's
# terminal key-binding refusal (15); every other child exit remains retryable.
# Run the node directly so its status is never hidden by caffeinate.
set -u
[ "$#" -gt 0 ] || exit 1

# Require the explicit data directory used by every generated job. A fresh
# RunAtLoad/bootstrap must honor a refusal left by a previous process, too.
bad_data() {
  echo "launchd node wrapper requires one valid explicit --data directory" >&2
  exit 1
}
data=
data_seen=0
needs_data=0
for arg in "$@"; do
  if [ "$needs_data" -eq 1 ]; then
    case "$arg" in ''|-*) bad_data ;; esac
    data=$arg
    needs_data=0
    continue
  fi
  case "$arg" in
    --) break ;;
    --data)
      [ "$data_seen" -eq 0 ] || bad_data
      data_seen=1
      needs_data=1 ;;
    --data=*)
      [ "$data_seen" -eq 0 ] || bad_data
      data_seen=1
      data=${arg#--data=}
      case "$data" in ''|-*) bad_data ;; esac ;;
  esac
done
[ "$data_seen" -eq 1 ] && [ "$needs_data" -eq 0 ] || bad_data
if [ -e "$data/key-binding-refused" ] || [ -L "$data/key-binding-refused" ]; then
  echo "key binding refused (persisted); automatic restart disabled; owner recovery is required" >&2
  exit 0
fi

child=0
awake=0
# shellcheck disable=SC2329 # Invoked by the TERM/INT traps below.
terminate() {
  trap - TERM INT
  if [ "$child" -gt 0 ]; then
    kill -TERM "$child" 2>/dev/null || true
    wait "$child" 2>/dev/null || true
  fi
  if [ "$awake" -gt 0 ]; then kill -TERM "$awake" 2>/dev/null || true; fi
  exit "$1"
}
trap 'terminate 143' TERM
trap 'terminate 130' INT

"$@" &
child=$!
/usr/bin/caffeinate -s -w "$child" &
awake=$!
wait "$child"
status=$?
child=0
kill -TERM "$awake" 2>/dev/null || true
wait "$awake" 2>/dev/null || true

if [ "$status" -eq 15 ]; then
  echo "key binding refused (exit 15); automatic restart disabled; owner recovery is required" >&2
  exit 0
fi
# Preserve the original KeepAlive behavior for even an unexpected clean exit.
[ "$status" -ne 0 ] || status=1
exit "$status"
