#!/usr/bin/env bash
set -e
cd "$(dirname "$0")"
if [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
fi
echo "=========================================================="
echo " Starting EastSea Sovereign Node (Cross-Platform Daemon)  "
echo " 'Don't trust, verify - Run your own node on your laptop' "
echo "=========================================================="
cargo run --release --bin aether-node
