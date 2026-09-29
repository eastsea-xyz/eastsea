#!/usr/bin/env bash
# The canonical build root for the proving program (gap G5).
#
# cargo hashes a path dependency's own path into `-C metadata` whenever that
# dependency is *not* under the workspace root of the invocation
# (`PackageId::stable_hash`: `path.strip_prefix(workspace_root).unwrap_or(path)`).
# apps/prover is its own workspace root (`[workspace] members = ["guest"]`) and
# the Aether crates live above it, so without this the checkout's absolute path
# lands in the metadata hash, then in every symbol name, and — through the code
# layout the Jolt guest build emits — in the guest ELF's `.text`. Measured: two
# checkouts gave ELFs of the same size but 52,068 bytes differing inside `.text`
# (no reordering: 99.75% of 16-byte windows appear in both files and 98% of them
# at the same offset), i.e. two proving program ids for one source tree.
# `--remap-path-prefix` cannot fix that: it rewrites the paths rustc *reports*,
# not the ones cargo *hashes*.
#
# So the prover is built through a **stage**: a fixed-path directory of symlinks
# to the checkout's prover and to the crates it compiles. Every path cargo sees
# is then the same string in every checkout, which makes metadata, layout and
# program id the same. The stage is shared mutable state — its links point at one
# checkout at a time — so builds that use it serialise on a lock:
#
#   . scripts/guest-stage.sh
#   aether_guest_stage_enter "$repo"                  # lock + point the stage here
#   stage="$(aether_guest_stage_path)"
#   (cd "$stage/apps/prover" && cargo build --release)
#
# AETHER_GUEST_STAGE=<path>  where the stage lives (default below). It is part of
# the program id, so every builder comparing hashes must use the same value.
# The caller's build must remap the stage prefix (`aether_guest_stage_remap`) to
# keep the path out of the ELF's panic locations.
#
# Only cargo's *inputs* go through here: outputs stay in the real tree
# (`<stage>/apps/prover/target` and `target-guest` are links into the checkout),
# which is why a build that runs through the stage leaves no copy behind. A build
# that starts *inside* the stage — the host prover's own build.rs runs
# apps/prover/build-guest.sh, which cargo then starts with the stage as the
# package root — names the stage as its checkout; populating that would point
# every link at itself, so it is left alone instead.
set -euo pipefail

# Where the stage lives. Fixed and absolute on purpose: a relative or
# per-checkout path would be hashed into the metadata again.
aether_guest_stage_path() {
  printf '%s\n' "${AETHER_GUEST_STAGE:-/tmp/aether-guest-stage}"
}

# --remap-path-prefix for the stage, in both spellings (given and physical:
# macOS /tmp -> /private/tmp), mapping it to $1. Empty if the stage is absent.
aether_guest_stage_remap() {
  local to="$1" stage real
  stage="$(aether_guest_stage_path)"
  printf -- ' --remap-path-prefix=%s=%s' "$stage" "$to"
  real="$(cd -P "$stage" 2>/dev/null && pwd -P)" || real=""
  if [ -n "$real" ] && [ "$real" != "$stage" ]; then
    printf -- ' --remap-path-prefix=%s=%s' "$real" "$to"
  fi
}

# Serialise builds that share the stage. A build whose owner died (same host,
# pid gone) is not waited for: its lock is dropped and retaken.
aether_guest_stage_lock() {
  local stage lock owner_pid owner_host budget waited=0
  stage="$(aether_guest_stage_path)"
  lock="$stage.lock"
  # A cold prover build with the Jolt dependencies can take a long time, so the
  # wait budget is generous (seconds; AETHER_GUEST_STAGE_LOCK_WAIT, 0 = forever).
  # It is only a safety valve: an owner that died is detected by pid and never
  # waited for.
  budget="${AETHER_GUEST_STAGE_LOCK_WAIT:-7200}"
  mkdir -p "$(dirname "$stage")"
  while ! mkdir "$lock" 2>/dev/null; do
    if read -r owner_pid owner_host <"$lock/owner" 2>/dev/null &&
      [ "$owner_host" = "$(hostname)" ] && ! kill -0 "$owner_pid" 2>/dev/null; then
      rm -rf "$lock"
      continue
    fi
    waited=$((waited + 1))
    [ "$waited" = 10 ] && echo "guest-stage: waiting for $lock (another prover build holds the canonical stage)" >&2
    if [ "$budget" -gt 0 ] && [ "$waited" -ge $((budget * 5)) ]; then
      echo "guest-stage: $lock has been held for ${budget}s; remove it if no prover build is running" >&2
      return 1
    fi
    sleep 0.2
  done
  echo "$$ $(hostname)" >"$lock/owner"
  _aether_guest_stage_lock="$lock"
}

aether_guest_stage_unlock() {
  [ -n "${_aether_guest_stage_lock:-}" ] || return 0
  rm -rf "$_aether_guest_stage_lock"
  _aether_guest_stage_lock=""
}

# Point the stage at the checkout root $1. Idempotent, and it will not replace
# anything it did not create: a real file where a link belongs means someone
# else is using that path for something else.
_aether_guest_stage_link() {
  local link="$1" target="$2"
  [ -e "$target" ] || return 0
  if [ -L "$link" ]; then
    [ "$(readlink "$link")" = "$target" ] || ln -sfn "$target" "$link"
  elif [ -e "$link" ]; then
    echo "guest-stage: $link is not a symlink; refusing to replace it" >&2
    return 1
  else
    ln -s "$target" "$link"
  fi
}

# Mirror $1 into the real directory $2, entry by entry, skipping what a build
# writes back (target/, .git/, and apps/, which is handled a level down so that
# apps/prover ends up a real directory rather than a link).
_aether_guest_stage_mirror() {
  local source="$1" target="$2" entry base
  mkdir -p "$target"
  for entry in "$source"/* "$source"/.[!.]*; do
    [ -e "$entry" ] || continue
    base="$(basename "$entry")"
    case "$base" in .git | apps | target | target-*) continue ;; esac
    _aether_guest_stage_link "$target/$base" "$entry"
  done
}

aether_guest_stage_populate() {
  local root stage stage_real
  root="$(cd -P "$1" && pwd -P)" || return 1
  stage="$(aether_guest_stage_path)"
  stage_real="$(cd -P "$stage" 2>/dev/null && pwd -P)" || stage_real="$stage"
  # A build that reaches the stage from inside it — the nested build.rs ->
  # build-guest.sh call, whose cargo hands it the stage path as the package root,
  # and a manual run from $stage/apps/prover — names the stage as its checkout.
  # Linking that would point every entry at itself (`crates -> crates`), and the
  # build would find no workspace root at all. The stage already names the real
  # tree there: leave it as it is.
  if [ "$root" = "$stage_real" ]; then return 0; fi
  # The checkout root supplies the crates the prover compiles and the workspace
  # manifest they inherit from, all reached through the same fixed path.
  _aether_guest_stage_mirror "$root" "$stage"
  mkdir -p "$stage/apps"
  # apps/prover is a real directory holding links: it *is* the workspace root of
  # the prover build, and a root that is itself a symlink would hand the real
  # checkout's path to anything that resolves it (the Jolt CLI walks the
  # workspace), which is the path the metadata hash is made of.
  _aether_guest_stage_mirror "$root/apps/prover" "$stage/apps/prover"
  # Build outputs stay in the checkout, so a build through the stage leaves the
  # artifacts where the tree expects them.
  mkdir -p "$root/apps/prover/target" "$root/apps/prover/target-guest"
  _aether_guest_stage_link "$stage/apps/prover/target" "$root/apps/prover/target"
  _aether_guest_stage_link "$stage/apps/prover/target-guest" "$root/apps/prover/target-guest"
}

# Lock (unless the caller already holds the lock) and point the stage at the
# checkout root $1. Echoes nothing; the caller knows the root it passed.
aether_guest_stage_enter() {
  [ -d "$1" ] || { echo "guest-stage: no such checkout: $1" >&2; return 1; }
  if [ -z "${AETHER_GUEST_STAGE_LOCKED:-}" ]; then
    aether_guest_stage_lock || return 1
    # shellcheck disable=SC2064
    trap 'aether_guest_stage_unlock; exit 1' INT TERM
    trap 'aether_guest_stage_unlock' EXIT
    # Exported (not an argument) so the nested build.rs -> build-guest.sh call
    # keeps the parent's lock instead of deadlocking on it.
    export AETHER_GUEST_STAGE_LOCKED=1
  fi
  aether_guest_stage_populate "$1"
}
