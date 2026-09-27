# Aether

> Any Mac can be a validator, and your Mac verifies your wallet itself.

[![Legal Disclaimer](https://img.shields.io/badge/Legal-Disclaimer%20%26%20Terms-red.svg)](DISCLAIMER.md)

> [!IMPORTANT]
> Aether is experimental, non-commercial research software provided **"AS IS"**. It runs as a **testnet** only. It is not a production blockchain and has not been audited. All tokens (AETH) and rewards are test artifacts with **zero monetary value**. See [DISCLAIMER.md](DISCLAIMER.md).

## Download

Get the latest build from [Releases](https://github.com/kjaylee/aether-node/releases/latest).

- **Aether for macOS** (Apple silicon, macOS 14 or later): `Aether-<version>.dmg`, signed and notarized by Apple. Drag it to Applications. Updates arrive by themselves (Aether ▸ Check for Updates…).
- **Aether Wallet browser extension** (Chrome, Edge, Brave, Arc): `aether-extension-<version>.zip`. Unzip it, open `chrome://extensions`, turn on Developer mode, and choose Load unpacked. It works without the Mac app.

## What it is

- **A wallet on your Mac.**
  - The key lives in the Secure Enclave, and every payment asks for Touch ID. There is no seed phrase.
  - A second Apple device, or 24 recovery words, can be added as a recovery key.
- **Verify, don't trust.**
  - The wallet checks each balance on your Mac against the validator committee's signature and a state proof. It never takes a server's word for it.
- **A node, with one switch.**
  - With the switch on, the app follows the chain and re-executes every block on your Mac.
  - A Mac can then register as a voting node, and prove blocks on its GPU.
- **No ports, no VPN.**
  - Validators and wallets find each other on the BitTorrent Mainline DHT, used only to look up addresses, and connect over QUIC with hole punching.
- **For web pages.**
  - Pages can ask for a payment or a contract call. The browser extension shows the request for your approval, or the Mac app does through `aether://` links.
- **For AI agents.**
  - `aether-agent` gives Claude Code, Codex and any MCP client a wallet with spending limits enforced on chain. Only you can change the limits, with Touch ID.
  - In the app: Aether ▸ Install Command-Line Tools…, then `aether-agent init` and `aether-agent setup all --apply`.

## How mainnet will launch

Read this before you join. It is the same in the app.

- **No token sale, no premine, no founder share.** Every AETH on mainnet comes from block rewards, under the same rules for everyone.
- **Block rewards stay off until 16 different operators are voting.** Until then nobody, the founder included, earns anything. The chain turns rewards on by itself; nobody signs for it.
- **Joining early only puts your Mac ahead in the draw.** Voting Macs are drawn by how long they have been online without a break. There are no airdrops or referral rewards.
- **Mainnet is a new network.** Testnet AETH does not carry over.

## Source code

This repository used to hold an early prototype. It has been retired and does not describe Aether today.

The source of the current Aether will be published here under MIT or Apache-2.0, once these conditions are met:

- The testnet (chain 7780) has run for 30 days without a reset.
- The code has passed a secrets and personal-data scan.

Until then, the releases above are the way to run Aether.
