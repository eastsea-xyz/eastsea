# EastSea Node

**English** · [한국어](README.ko.md) · [中文](README.zh-CN.md) · [日本語](README.ja.md) · [Tiếng Việt](README.vi.md) · [Español](README.es.md)

> Apple silicon Macs can register as voting nodes, and your Mac verifies your wallet itself. Today, validators on different home internet lines reach consensus over public paths, and the Mac and iPhone wallet apps work against that network.

[![Legal Disclaimer](https://img.shields.io/badge/Legal-Disclaimer%20%26%20Terms-red.svg)](DISCLAIMER.md)

> [!IMPORTANT]
> EastSea is built for production. **Mainnet has not launched yet**: the network running today is the public testnet (chain 7780), and testnet DBLN does not carry over. There is no token sale; the value of DBLN is set by the market, and nothing here promises a price or a return. The software is provided **"AS IS"** and has not had an independent security audit yet. Nothing here is investment, legal or tax advice. See [DISCLAIMER.md](DISCLAIMER.md).

## What it is

- **Mac-first.**
  - Wallet keys live in the Secure Enclave and every payment asks for Touch ID. There is no seed phrase.
  - If you lose the device and have not set up a recovery key, nobody can restore the account.
  - A second Apple device can be registered as the recovery key.
- **Verify, don't trust.**
  - The wallet checks each balance on the device: one BLS threshold signature from the validator committee, plus an EIP-7864 state proof.
  - It trusts the committee key it ships with, not a server's word.
- **No ports, no VPN.**
  - Validators and wallets find each other by node ID on the BitTorrent Mainline DHT.
  - They connect over iroh QUIC with hole punching, and fall back to a relay when that fails.
- **Built for AI agents too.**
  - `aether-agent` gives Claude Code, Codex, Antigravity, OpenClaw, Hermes or any MCP client a wallet.
  - Its key sits in the Secure Enclave, and the account contract enforces its spending limits on chain; only you can change them, with Touch ID.

## Try it

```bash
scripts/demo.sh          # start 4 validators; show transfers, a contract, proof checks, nodes agreeing
scripts/devnet.sh stop   # stop them
```

Requires the rustup toolchain 1.98.1 (`rust-toolchain.toml`). If Homebrew's rustc comes first on your PATH, run `export PATH="$HOME/.cargo/bin:$PATH"`.

### Wallet app (macOS, iOS)

```bash
scripts/build-wallet.sh           # macOS app
scripts/build-wallet.sh ios-sim   # iOS Simulator
```

- **Simple mode (default):**
  - A home screen with the balance and a balance chart, plus Send, Receive (QR) and Get test tokens.
  - Pages for activity, network status and recovery setup.
- **Developer mode:** proofs, state roots, raw logs and blocks. Use the switch at the top left to change modes.

### Browser extension wallet (Chrome, Edge, Brave, Arc)

```bash
scripts/build-extension.sh        # then load apps/extension unpacked from chrome://extensions
```

- **No app needed:** the key is made in the browser and encrypted with your password.
- **Pages:** get `window.aether` (EIP-1193, announced through EIP-6963). The EastSea DEX page uses it when it is installed; so does the open-source launchpad page — community-hosted, not hosted or promoted by Pipln.
- **Approvals:** connecting a site and every transaction open an approval window.
- Details: [apps/extension/README.md](apps/extension/README.md).

### Wallet for AI agents

```bash
scripts/build-agent.sh --install   # ~/.local/bin/aether-agent
aether-agent init                  # you, once: create keys; fund the account
aether-agent payee add --name Shop --address 0x...  # approve first payee with Touch ID
aether-agent setup all --apply     # register the MCP server "aether" with every agent tool you have installed
```

- **Tools:** status, wallet, balance, send, pay_many (one transaction), pay_token (new genesis), receipt, history.
- **Safe default:** payments stay off until the owner adds a named payee with `aether-agent payee add --name NAME --address 0x...` and Touch ID. The first session lasts seven days, with 1 DBLN per payment and 10 DBLN per 24 hours. Renew with `aether-agent policy renew`; stop immediately with `aether-agent stop` or **비서 멈추기** in the Mac wallet.
- **On-chain limits:** the contract checks payment caps, approved recipients, and expiry. The owner can explicitly choose `--allow anyone` after a warning. A tricked agent can still spend within its limits, and stopping cannot undo a transaction already submitted. Gas comes from a separate small balance. Token payments require a new-genesis account contract and `aether-agent token allow --address 0x... --per-tx UNITS --per-day UNITS` (token base units, Touch ID). Renewing or replacing a session clears its token permissions; the owner must approve them again.
- Details: [AGENTS.md](AGENTS.md) and [the skill file](agents/skills/aether-wallet/SKILL.md).

### Run a real network

On each validator machine, generate its own key. Then gather the public halves, run the key ceremony together, and start the nodes:

```bash
aether keygen --data ~/aether/v1                         # on each machine; the secret stays there
aether network v1.pub.json v2.pub.json … > network.json  # public halves only; give it to everyone
AETHER_NETWORK=network.json scripts/devnet.sh dkg 4      # other machines: aether dkg --network network.json --port … --data …
AETHER_NETWORK=network.json scripts/devnet.sh start 4    # other machines: aether node --network network.json …
# wallet: copy <data>/network.json (node IDs + committee key) to apps/wallet/Resources/
```

The committee key survives validator changes. Use `aether reshare` to move to a new validator set.

### Command line

```bash
target/debug/aether dev-accounts                               # public dev keys funded at a local dev genesis (never use for value)
target/debug/aether send --from-dev 1 --to 0x… --value 1000 --wait
target/debug/aether balance 0x… --rpc http://127.0.0.1:8547   # verified locally with a proof, not trusted
target/debug/aether batch --from-dev 1 --to 0xA,0xB --value 1  # several payments, one signature
target/debug/aether blocks 10
```

## Planned mainnet rules

The app shows the same text (`VotingRules.mainnetRewardsRule`):

> Planned for the future mainnet, which is not live: the rules may change before launch, and after it only by a committee-signed upgrade. No token sale, no premine and no founder allocation; the founder's Macs follow the same rules as everyone's. Half of each block's reward goes to registered Macs that stay online, shared every hour, and half to registered Macs that prove blocks. One operator gets at most 1/16 of each half, and the rest is never issued; once 16 operators are online, all of it is shared. The reward starts at 1 DBLN a block and shrinks 15% a year, down to a floor of 0.1 DBLN a block. Testnet DBLN does not carry over. Nothing here promises a price, a return or a way to cash out.

- **Registration:** a Mac joins through a registration service, currently run by Pipln, that checks an Apple DeviceCheck token with Apple. Apple does not sponsor or endorse EastSea.
- **Founder reserve keys:** The founder's only special permission: one Mac may run up to 3 reserve validator keys, and only while the network needs them. Hours they serve count as the founder's participation, under the same 1/16 cap as everyone; they add no extra share. (`VotingRules.founderReserveRule`, quoted word for word.)
- **What 1/16 does not do:** it is counted per wallet address. Someone with several wallets and several real Macs gets several shares.

## Status (2026-09-26)

| Area | Works today | Not yet |
|---|---|---|
| Consensus | Commonware simplex BFT: 4 validators, 1 s blocks, keeps going with one validator down. BLS12-381 threshold certificates (131 B, checked with one group key). Keys made locally; committee key from a dealerless DKG. Validator rotation by reshare. VRF-seeded random leader | On-chain committee changes, VRF committee selection |
| Network | iroh QUIC between validators and wallets, with hole punching or a relay. Addresses found by node ID on the Mainline DHT. Tailscale, CGNAT and loopback paths are never used. Tested with Macs on two different ISPs | On-chain validator list, own relays |
| Execution | revm: transfers, contract deployment and calls. Every validator re-executes each block and must match its block access list (BAL) and gas. Optimistic parallel execution gives the same result as sequential | Parallel tree commits |
| Fees | Separate base fees for execution and proving, adjusted like EIP-4844. The execution base fee is burned, and the proving fee goes to a prover escrow. Tips split 60% proposer, 20% prover escrow, 20% burned | Escrow claims per proven chunk |
| State | EIP-7864 binary tree (Poseidon2), with keys and roots matching the geth reference. Inclusion and absence proofs. Stored atomically per block in redb, and resumes from a checkpoint after a restart | Disk paging, snapshot sync |
| Accounts | P-256 (Secure Enclave), secp256k1 and Ed25519. EIP-7702 delegation to `AetherAccount` allows batched payments under one signature. A second device's Secure Enclave key can act as the recovery key | Session keys, multiple guardians, time-locked recovery |
| Clients | Mac and iOS wallets (Simple and Developer modes), the CLI, and `aether-agent` (MCP) all verify balances locally | ZK block proofs in the client, TestFlight |
| Censorship resistance | FOCIL-style inclusion lists: validators refuse to vote for a block that leaves out listed transactions | Encrypted mempool |
| Proving (spike) | Jolt zkVM proves real EastSea blocks, and the roots match native execution. About 270 transactions per hour per Mac. Proofs trail the chain | Metal backend, checkpoint proofs |

`legacy/` is the earlier single-node demo and has been replaced.

## Design

- [Implementation design](docs/design/00-overview.md): identity, decisions D1–D18, and the design of each layer
- [Research](docs/research/): the sources behind each decision, including [tokenomics 2026](docs/research/tokenomics-2026.md)
- [Spike results](docs/research/spike-2026-10.md)

## Tests

```bash
cargo test --workspace
cargo test -p aether-state --test eip7864_compat   # cross-check against the EIP-7864 reference
cargo test -p aether-execution --test fees         # fee split and conservation of value
```

## Layout

```
crates/
├── node/        # aether binary: validator (simplex + marshal), JSON-RPC, CLI, DKG
├── execution/   # revm execution, tx validation, BAL, fees, receipts, prove gas
├── state/       # EIP-7864 binary tree and proofs
├── types/       # envelopes, blocks, BAL, certificates, proofs
├── light/       # light client: committee key, certificate checks
├── net/         # iroh links, Mainline DHT discovery
├── ffi/         # wallet core for Swift (UniFFI)
├── hash/ crypto/ consensus/ proving/ da/
apps/
├── wallet/      # macOS and iOS wallet (SwiftUI)
└── agent/       # aether-agent: MCP server and JSON CLI for AI agents
agents/skills/   # SKILL.md for agent tools
contracts/       # AetherAccount (EIP-7702 batch + recovery)
spike/           # zkVM proving experiments
scripts/         # devnet.sh, demo.sh, build-wallet.sh, build-agent.sh
docs/design, docs/research
```

## License

Dual-licensed under MIT or Apache-2.0.
