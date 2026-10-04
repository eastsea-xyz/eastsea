@echo off
cd /d "%~dp0"
echo ==========================================================
echo  Starting EastSea Sovereign Node (Windows Daemon)
echo  "Don't trust, verify - Run your own node on your laptop"
echo ==========================================================
cargo run --release --bin aether-node
pause
