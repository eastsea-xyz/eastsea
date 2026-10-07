#!/usr/bin/env bash
# Build the proving sidecar (apps/prover) and print its program id (the guest
# ELF's SHA-256). Node builds export it as AETHER_PROVER_PROGRAM so a node only
# verifies proofs of the program the protocol names.
#   export AETHER_PROVER_PROGRAM=$(scripts/prover-program.sh)
# The id must not depend on where the checkout lives (gap G5): both the guest
# (apps/prover/build-guest.sh) and the host sidecar are built from the canonical
# stage (scripts/guest-stage.sh), and the paths rustc reports are remapped here.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
. scripts/repro-env.sh
. scripts/guest-stage.sh
jolt="${AETHER_JOLT:-/Volumes/workspace/aether-jolt}"
[ -d "$jolt/jolt" ] || { echo "the Jolt fork (aether-jolt/jolt) is needed to build the prover" >&2; exit 1; }
# The CLI drives the guest build; one built from another revision of the fork
# could change the id, so it has to match the pin (scripts/jolt-fork.lock).
aether_jolt_cli_check "${JOLT_PATH:-jolt}"
# cargo hashes the path of every dependency that lives outside the workspace
# root of the invocation — for apps/prover, which is its own workspace root,
# that is every Aether crate — so the checkout's path would otherwise land in
# the metadata (symbol names, code layout) and, through the guest the host
# embeds, in the program id. remap-path-prefix does not reach that hash; the
# stage does.
aether_guest_stage_enter "$(pwd -P)"
stage="$(aether_guest_stage_path)"
# The host links jolt-sdk and akita from the Jolt fork by absolute path, so
# remap that tree too or the host binary follows the fork's location.
aether_repro_rustflags "--remap-path-prefix=$(cd "$jolt" && pwd)=/jolt$(aether_guest_stage_remap /aether-node)"
compile_guard="$HOME/.claude/playbooks/aether-team/wait-compile.sh"
if [ -x "$compile_guard" ]; then "$compile_guard"; fi
(cd "$stage/apps/prover" && MACOSX_DEPLOYMENT_TARGET=14.0 cargo build -q --release --locked) >&2
# The binary lands in CARGO_TARGET_DIR when one is set (shared build dirs). Like
# the node it carries the linker's UUID, which follows the build directory, and
# it ships as it is built: rewrite the UUID from the code here, once.
bin="${CARGO_TARGET_DIR:-$stage/apps/prover/target}/release/aether-prover"
aether_repro_fix_uuid "$bin"
# The id is a function of the guest's inputs alone (scripts/guest-inputs.py);
# name them beside it, so two builds that disagree can be told apart at once.
echo "prover-program: guest inputs $(python3 scripts/guest-inputs.py "$(pwd -P)")" >&2
"$bin" info | python3 -c 'import json,sys; print(json.load(sys.stdin)["guest_elf_sha256"])'
