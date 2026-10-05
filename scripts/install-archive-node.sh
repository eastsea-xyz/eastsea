#!/bin/sh
# Install an Aether archive node on a storage host (roadmap B6).
#
#   scripts/install-archive-node.sh <host> <chain-id> <network.json>
#
# From the Mac (where this repo is checked out), against a Linux host over
# SSH (poc-nas by convention):
#   1. copy the source over with a tar pipe through ssh (vendor/ included
#      for the patched n0-mainline; .git/tmp/target excluded — poc-nas's
#      rsync refuses server-mode writes into ~/AI, and a tar overwrite keeps
#      target/ intact so rebuilds stay incremental);
#   2. install rustup in the host user's home when missing (no sudo, no
#      system changes) with the toolchain rust-toolchain.toml pins;
#   3. install zig as a userspace C toolchain when the host has no cc
#      (poc-nas ships no compiler; `zig cc -target x86_64-linux-gnu` carries
#      its own libc, so build scripts and the linker work without root);
#   4. copy network.json in;
#   5. write the restart-on-exit runner ~/AI/eastsea-archive/run-archive.sh;
#   6. kick off the release build in the background (the NAS CPU takes a
#      while the first time; everything after is incremental).
#
# Installing does NOT start the node. The start command is in
# docs/ops/archive-node.md — run it only when the chain should have an
# archive there.

set -eu

HOST=${1:?usage: install-archive-node.sh <host> <chain-id> <network.json>}
CHAIN=${2:?chain id}
NETWORK=${3:?path to network.json (with the committee identity)}
BASE='$HOME/AI/eastsea-archive'
ZIG_VERSION=0.17.0

[ -f "$NETWORK" ] || { echo "no such network.json: $NETWORK" >&2; exit 1; }

echo "==> source -> $HOST:$BASE/src"
# Not rsync: this NAS's rsync (3.4.1) rejects server-mode writes into ~/AI
# with "invalid path" (the tar pipe below works). A tar overwrite leaves
# stale files behind, but Cargo never compiles them (it walks the manifest);
# for a from-scratch refresh: ssh $HOST "rm -rf $BASE/src" first.
ssh "$HOST" "mkdir -p $BASE/src $BASE/logs"
COPYFILE_DISABLE=1 tar -cf - --no-mac-metadata \
  --exclude=.git --exclude=tmp --exclude=target --exclude=.claude --exclude=.omc . |
  ssh "$HOST" "tar -xf - -C $BASE/src"

echo "==> rustup (userspace, no sudo) when missing"
ssh "$HOST" 'test -x "$HOME/.cargo/bin/cargo" || {
  curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs |
    sh -s -- -y --profile minimal --default-toolchain 1.98.1 --no-modify-path
}'
# The toolchain the repo pins (rust-toolchain.toml) resolves from here on;
# install it up front so the build log does not interleave a toolchain download.
ssh "$HOST" 'cd '"$BASE"'/src && "$HOME/.cargo/bin/rustup" toolchain install 1.98.1 --profile minimal >/dev/null 2>&1 || true'

echo "==> zig (userspace C toolchain) when the host has no cc"
ssh "$HOST" 'command -v cc >/dev/null 2>&1 && exit 0
set -e
mkdir -p '"$BASE"'/toolchain; cd '"$BASE"'/toolchain
test -x zig/zig || {
  curl -sSLO https://ziglang.org/download/'"$ZIG_VERSION"'/zig-x86_64-linux-'"$ZIG_VERSION"'.tar.xz
  tar -xf zig-x86_64-linux-'"$ZIG_VERSION"'.tar.xz
  rm -f zig-x86_64-linux-'"$ZIG_VERSION"'.tar.xz
  mv zig-x86_64-linux-'"$ZIG_VERSION"' zig
}
printf "#!/bin/sh\n# zig cc wrapper. The target goes AFTER the caller arguments: the cc crate\n# passes the Rust triple (--target=x86_64-unknown-linux-gnu), which zig cannot\n# parse — a later -target wins, so ours (a zig triple) overrides it.\nexec \"\$(dirname \"\$0\")/zig/zig\" cc \"\$@\" -target x86_64-linux-gnu\n" > zigcc
printf "#!/bin/sh\nexec \"\$(dirname \"\$0\")/zig/zig\" c++ \"\$@\" -target x86_64-linux-gnu\n" > zigcxx
printf "#!/bin/sh\nexec \"\$(dirname \"\$0\")/zig/zig\" ar \"\$@\"\n" > zigar
chmod +x zigcc zigcxx zigar
echo \"int main(){return 0;}\" > zig-probe.c
./zigcc -o zig-probe zig-probe.c && ./zig-probe && rm -f zig-probe zig-probe.c
echo "zig cc ok: $($BASE/toolchain/zig/zig version)"'

echo "==> network.json"
scp -q "$NETWORK" "$HOST:$BASE/network-$CHAIN.json"

echo "==> runner"
ssh "$HOST" "cat > $BASE/run-archive.sh" <<'RUNNER'
#!/bin/sh
# run-archive.sh <chain-id> [public-base-url] — the archive node runner.
# Restarts the node after any crash (30 s apart), logs beside the data,
# keeps the log under ~10 MB. Exit 0 from the node ends it (deliberate stop).
set -u
CHAIN=${1:?chain id}
BASE_URL=${2:-http://$(ip -4 -o addr show scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1 | head -1):8545}
BASE=$HOME/AI/eastsea-archive
BIN=$BASE/src/target/release/aether
DATA=$BASE/$CHAIN/data
EXPORT=$BASE/export/$CHAIN
NET=$BASE/network-$CHAIN.json
LOG=$BASE/logs/archive-$CHAIN.log
[ -x "$BIN" ] || { echo "no binary at $BIN (is the build done? see $BASE/build.log)" >&2; exit 1; }
[ -f "$NET" ] || { echo "no $NET: copy the chain's network.json in (docs/ops/archive-node.md)" >&2; exit 1; }
mkdir -p "$DATA" "$EXPORT" "$BASE/logs"
# Trim: keep the previous log, start the current one fresh.
[ -f "$LOG" ] && [ "$(wc -c <"$LOG")" -gt 10000000 ] && mv "$LOG" "$LOG.1"
n=0
while :; do
  n=$((n + 1))
  echo "=== start #$n $(date -Is) base=$BASE_URL" >>"$LOG"
  "$BIN" archive \
    --network "$NET" \
    --data "$DATA" \
    --rpc-port 8545 \
    --bind 0.0.0.0 \
    --export-dir "$EXPORT" \
    --https-base "$BASE_URL" \
    >>"$LOG" 2>&1
  code=$?
  echo "=== exit $code $(date -Is)" >>"$LOG"
  [ "$code" -eq 0 ] && exit 0
  sleep 30
done
RUNNER
ssh "$HOST" "chmod +x $BASE/run-archive.sh"

echo "==> build (background; watch with: ssh $HOST tail -f $BASE/build.log)"
# zig when present (a host with no system cc — zig cc links, zig ar archives),
# the host's own toolchain otherwise.
ssh "$HOST" "cd $BASE/src && { test -x $BASE/toolchain/zigcc && export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=$BASE/toolchain/zigcc CC=$BASE/toolchain/zigcc CXX=$BASE/toolchain/zigcxx AR=$BASE/toolchain/zigar; }; nohup \"\$HOME/.cargo/bin/cargo\" build --release -p aether-node > $BASE/build.log 2>&1 &"

echo
echo "installed. When the build finishes and you mean to start it:"
echo "  ssh $HOST 'nohup $BASE/run-archive.sh $CHAIN > /dev/null 2>&1 &'"
echo "(do not start it on the live testnet without the founder's say-so — docs/ops/archive-node.md)"
