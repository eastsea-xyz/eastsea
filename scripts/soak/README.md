# Soak test

Runs on the test Macs until mainnet.

- `monitor.sh` (every minute): chain height, the newest finalized block's age, validators that proposed in the last 100 blocks, and optionally other Macs' nodes over ssh. Alerts (macOS notification + `~/aether-soak/alerts.log`) when finalization is slower than 30 s (`AETHER_SOAK_FINALITY`), validators stop answering, a validator stops proposing, a node falls behind, or a hash mismatches. Each problem alerts once when it starts and once when it clears (open problems: `~/aether-soak/open/`). Liveness is read from the chain, so a Mac with Tailscale off is still covered.
- `traffic.mjs`: two transfers a second and a 1,000-transfer burst every six hours through the extension's wallet code; one JSON line a minute in `~/aether-soak/traffic.log` (sent, confirmed, p50/p95 confirmation time, errors, mempool).

launchd runs `monitor.sh` from `~/aether-soak/bin` (macOS does not let launchd's bash read scripts on external volumes): copy it there after changes (`cp scripts/soak/monitor.sh ~/aether-soak/bin/`). The script reads nothing from the repo.
