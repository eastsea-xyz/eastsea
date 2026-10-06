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
# The fork is part of the stage too (checklist B7): <stage>/aether-jolt holds a
# copy of the pinned Jolt·Akita revisions (see scripts/jolt-fork.lock), and the
# staged apps/prover manifests reference it relatively — `../../aether-jolt` —
# so the metadata hash, and with it the program id, depends on the fork's
# *contents* and never on where the fork or the checkout live. AETHER_JOLT
# selects the fork to snapshot (default: the canonical checkout path); a fork
# whose HEAD or archive hash differs from the pin is refused, not built.
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

# --- The Jolt fork: pinned revision, staged copy (checklist B7) ------------
#
# The proving program id used to follow the fork's location too: the manifests
# referenced /Volumes/workspace/aether-jolt absolutely, so a builder whose fork
# lived anywhere else either failed to read it or hashed a different path into
# the metadata (a second program id for the same source). The stage now holds
# its own copy: the manifests it hands cargo reference <stage>/aether-jolt
# relatively, and the copy is snapshotted by commit from $AETHER_JOLT and
# verified against scripts/jolt-fork.lock. The id then depends on the fork's
# contents — its pinned commits — and on nothing else about where anyone keeps
# it. AETHER_JOLT_LOCK overrides the pin file (tests use it with a fixture).

aether_jolt_source() {
  printf '%s\n' "${AETHER_JOLT:-/Volumes/workspace/aether-jolt}"
}

_aether_jolt_lock_path() {
  printf '%s\n' "${AETHER_JOLT_LOCK:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/jolt-fork.lock}"
}

# Echo the pinned value for $1 (jolt-commit, jolt-archive-sha256, akita-commit,
# akita-archive-sha256); fails when the lock file or the key is missing.
_aether_jolt_pin() {
  local lock value
  lock="$(_aether_jolt_lock_path)"
  [ -f "$lock" ] || { echo "guest-stage: Jolt fork pin not found: $lock" >&2; return 1; }
  value="$(grep -E "^$1=" "$lock" | tail -n 1 | cut -d= -f2-)"
  [ -n "$value" ] || { echo "guest-stage: no '$1=' in $lock" >&2; return 1; }
  printf '%s\n' "$value"
}

# The jolt CLI drives the guest build, so it must be built from the pinned fork
# revision or a different CLI could change the guest ELF. Its --version names
# the commit it was built from (the fork's build.rs reads git), abbreviated to
# however many digits that build's git chose — compare over the CLI's own
# length so a small clone (shorter abbreviations) still matches the pin.
aether_jolt_cli_check() {
  local cmd="$1" want out hash
  local re='\(([0-9a-f]{7,})[) ]'
  want="$(_aether_jolt_pin jolt-commit)" || return 1
  if ! out="$("$cmd" --version 2>&1)"; then
    echo "guest-stage: cannot run the jolt CLI: $cmd --version: $out" >&2
    return 1
  fi
  if ! [[ $out =~ $re ]]; then
    echo "guest-stage: $cmd --version does not name a commit: $out" >&2
    echo "  install the pinned CLI: cargo install --path $(aether_jolt_source)/jolt --locked" >&2
    return 1
  fi
  hash="${BASH_REMATCH[1]}"
  if [ "${want:0:${#hash}}" != "$hash" ]; then
    echo "guest-stage: the jolt CLI is from commit $hash but the fork is pinned at $want" >&2
    echo "  install the pinned CLI: cargo install --path $(aether_jolt_source)/jolt --locked" >&2
    return 1
  fi
}

# Verify the fork at $1 against the pin and snapshot the pinned revisions into
# <stage>/aether-jolt (git archive: the committed contents only, so a dirty
# worktree cannot leak into the program id). Extraction is skipped when the
# stage already holds exactly this commit pair; the commits and their archive
# hashes are still verified, so a fork that moved is refused either way.
aether_guest_stage_fork() {
  local fork="$1" stage dir marker want_j want_a have sum
  stage="$(aether_guest_stage_path)"
  dir="$stage/aether-jolt"
  marker="$dir/.aether-stage-snapshot"
  want_j="$(_aether_jolt_pin jolt-commit)" || return 1
  want_a="$(_aether_jolt_pin akita-commit)" || return 1
  for repo in jolt akita; do
    case "$repo" in jolt) want="$want_j" ;; akita) want="$want_a" ;; esac
    if ! have="$(git -C "$fork/$repo" rev-parse HEAD 2>/dev/null)"; then
      echo "guest-stage: $fork/$repo is not a git checkout" >&2
      echo "  clone the pinned fork (see scripts/jolt-fork.lock) and point AETHER_JOLT at it" >&2
      return 1
    fi
    if [ "$have" != "$want" ]; then
      echo "guest-stage: the $repo fork at $fork/$repo is at $have, but the prover pins $want" >&2
      echo "  check out the pinned commit, or update scripts/jolt-fork.lock (the program id changes with it)" >&2
      return 1
    fi
    sum="$(git -C "$fork/$repo" archive --format=tar "$want" | shasum -a 256 | cut -d' ' -f1)"
    if [ "$sum" != "$(_aether_jolt_pin "$repo-archive-sha256")" ]; then
      echo "guest-stage: the $repo fork's content does not match its pinned commit: archive of $want hashes to $sum" >&2
      return 1
    fi
  done
  if [ -f "$marker" ] && [ "$(cat "$marker")" = "$want_j $want_a" ]; then return 0; fi
  if [ -e "$dir" ] && [ ! -f "$marker" ]; then
    echo "guest-stage: $dir already exists and is not our snapshot; remove it" >&2
    return 1
  fi
  rm -rf "$dir"
  mkdir -p "$dir/jolt" "$dir/akita"
  git -C "$fork/jolt" archive --format=tar "$want_j" | tar -x -C "$dir/jolt"
  git -C "$fork/akita" archive --format=tar "$want_a" | tar -x -C "$dir/akita"
  echo "$want_j $want_a" >"$marker"
}

# Copy the checkout's manifest $1 to the staged path $2 with every fork
# reference rewritten to $3 (the stage-relative prefix). A copy, not a link:
# it must differ from the checkout's file, which keeps the developer workflow
# (an absolute fork path that cargo resolves directly) working where it is.
_aether_guest_stage_manifest() {
  local src="$1" staged="$2" prefix="$3" fork tmp
  [ -f "$src" ] || return 0
  [ -L "$staged" ] && rm "$staged" # a stage from before the rewrite holds a link
  if [ -e "$staged" ] && [ ! -f "$staged" ]; then
    echo "guest-stage: $staged is not a file; refusing to replace it" >&2
    return 1
  fi
  fork="$(aether_jolt_source)"
  tmp="$(mktemp)"
  sed -e "s|/Volumes/workspace/aether-jolt|$prefix|g" -e "s|$fork|$prefix|g" "$src" >"$tmp" || { rm -f "$tmp"; return 1; }
  # Whatever fork path the manifest grew must be one of the two spellings above;
  # an absolute reference left behind would put some builder's path in the id.
  if grep -E 'aether-jolt' "$tmp" | grep -qE '"/'; then
    echo "guest-stage: $src references the Jolt fork at a path the stage cannot rewrite:" >&2
    grep -E 'aether-jolt' "$src" | grep -E '"/' | sed 's/^/  /' >&2
    rm -f "$tmp"
    return 1
  fi
  if [ -f "$staged" ] && cmp -s "$tmp" "$staged"; then rm -f "$tmp"; return 0; fi
  mv "$tmp" "$staged"
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
# apps/prover ends up a real directory rather than a link). Any further names
# name entries this level stages itself (the manifests, rewritten below).
_aether_guest_stage_mirror() {
  local source="$1" target="$2"
  shift 2
  local entry base skip
  mkdir -p "$target"
  for entry in "$source"/* "$source"/.[!.]*; do
    [ -e "$entry" ] || continue
    base="$(basename "$entry")"
    case "$base" in .git | apps | target | target-*) continue ;; esac
    for skip in "$@"; do [ "$base" = "$skip" ] && continue 2; done
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
  # Linking would point every entry at itself (`crates -> crates`), and the
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
  # workspace), which is the path the metadata hash is made of. guest/ is a real
  # directory for the same reason — its manifest is one of the rewritten copies
  # — and the two manifests are copies with the fork referenced *inside* the
  # stage (B7), so the id follows the pinned snapshot, not the fork's location.
  _aether_guest_stage_mirror "$root/apps/prover" "$stage/apps/prover" guest Cargo.toml
  if [ -L "$stage/apps/prover/guest" ]; then rm "$stage/apps/prover/guest"; fi # an older stage's link
  mkdir -p "$stage/apps/prover/guest"
  _aether_guest_stage_mirror "$root/apps/prover/guest" "$stage/apps/prover/guest" Cargo.toml
  _aether_guest_stage_manifest "$root/apps/prover/Cargo.toml" "$stage/apps/prover/Cargo.toml" "../../aether-jolt"
  _aether_guest_stage_manifest "$root/apps/prover/guest/Cargo.toml" "$stage/apps/prover/guest/Cargo.toml" "../../../aether-jolt"
  aether_guest_stage_fork "$(aether_jolt_source)"
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
