# Soak test

Runs on the test Macs until mainnet.

- `monitor.sh` (every minute): chain height, validators that proposed in the last 100 blocks, and optionally other Macs' nodes over ssh. Alerts (macOS notification + `~/aether-soak/alerts.log`) on a stall, a missing validator, a node falling behind, or a hash mismatch. Liveness is read from the chain, so a Mac with Tailscale off is still covered.
- `traffic.mjs`: two transfers a second and a 1,000-transfer burst every six hours through the extension's wallet code; one JSON line a minute in `~/aether-soak/traffic.log` (sent, confirmed, p50/p95 confirmation time, errors, mempool).

launchd runs `monitor.sh` from `~/aether-soak/bin` (macOS does not let launchd's bash read scripts on external volumes): copy it there after changes.
