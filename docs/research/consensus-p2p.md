# aether-node: open-source Rust stacks to reuse

Every repository below was checked through the GitHub API (archived flag, last push, license) and every crate through crates.io on 2026-09-25. Anything not confirmed is marked unverified.

## 1. Consensus

| Name | Repo | License | Status | Reuse for aether-node | Caveats |
|---|---|---|---|---|---|
| **Commonware `consensus::simplex`** (+ `marshal`) | github.com/commonwarexyz/monorepo | Apache-2.0 (the repo also has a LICENSE-MIT) | Very active. Last push 2026-09-25. Crates at `2026.9.0` (date-based versions). `simplex` and `marshal` are marked BETA (wire and storage formats stable). `aggregation` is ALPHA. | Implement its traits (`Automaton`/`Relay`) around your block and execution logic. You get BFT finality with BLS or ed25519 certificates. | It's Simplex, not a DAG. Not externally audited (unverified). Monthly releases can break the API, so pin versions. |
| Sui `consensus` (production Mysticeti) | github.com/MystenLabs/sui/tree/main/consensus | Apache-2.0 | Active (Sui main branch). Not on crates.io. | Copy it in or depend on it by git (`consensus/core`) if you really want a DAG-BFT. | Tightly tied to Sui types, config and metrics. Pulling it into a small node is heavy work. |
| MystenLabs/mysticeti | github.com/MystenLabs/mysticeti | Apache-2.0 | **Archived.** README says the code moved into Sui, plus a minimal version at `asonnino/mysticeti`. | Reference for the paper and algorithm only. | Unmaintained. |
| MystenLabs/narwhal | github.com/MystenLabs/narwhal | Apache-2.0 | **Archived**, last push 2022-10. | Read-only reference. | Dead. |
| Malachite (Tendermint) | github.com/circlefin/malachite | Apache-2.0 | Active, push 2026-09-15. **The Informal Systems team joined Circle**, and it is now the consensus for Circle's Arc L1. Crates renamed `arc-malachitebft-*` (latest `0.7.0-pre`). The old `informalsystems-malachitebft-*` crates stop at 0.5.0 (2025-08). | Tendermint engine you can use as a library, with pluggable app and network layers. | README says it has not been externally audited. Expect breaking changes before 1.0. |
| HotShot | github.com/EspressoSystems/HotShot | MIT | **Archived** 2025-02. Merged into `espresso-network` (no license detected by GitHub, unverified). | Not recommended. | Built for Espresso's sequencer. |
| Aptos Shoal/Raptr | inside aptos-core | Apache-2.0 (GitHub shows NOASSERTION, unverified) | Active monorepo. No standalone crate. | Reference only. | Impractical to extract. |

## 2. P2P

| Name | Repo | License | Status | Reuse | Caveats |
|---|---|---|---|---|---|
| rust-libp2p | github.com/libp2p/rust-libp2p | MIT | v0.57.0, released 2026-09-11 | Replaces the homegrown HTTP P2P and the DHT free-riding. Includes QUIC, TCP with noise encryption, gossipsub, Kademlia (kad), AutoNAT, DCUtR hole punching, relay, identify, request-response, mdns, rendezvous and upnp. | Large API surface and frequent breaking releases. You still write your own peer scoring. |
| iroh (n0) | github.com/n0-computer/iroh | Apache-2.0 (dual MIT, unverified) | **1.2.0** (stable 1.x line), 2026-09-11 | QUIC connections dialed by public key, hole punching, free public relays. Has gossip and blobs add-ons. | Relies on n0's relay servers by default, so self-host them for production. Discovery is not a Kademlia DHT. |
| Commonware `p2p` | same monorepo | Apache-2.0 | Crate is marked BETA/GAMMA; its `simulated` network module is ALPHA | Authenticated network restricted to a known validator list. Plugs straight into `simplex`. | Only for a known validator set. It is not an open, permissionless network. |

## 3. Threshold encryption (encrypted mempool)

| Name | Repo | License | Status | Reuse | Caveats |
|---|---|---|---|---|---|
| Commonware `bls12381::{dkg, tle}` | monorepo/cryptography | Apache-2.0 | Active | Run a distributed key generation (DKG) among validators, then use timelock/threshold encryption (`tle`) to encrypt transactions to a future round. This is the best fit to replace the XOR toy. | DKG stability levels are mixed (ALPHA/BETA/GAMMA). |
| ferveo | nucypher/ferveo (the anoma fork is stale since 2023-02) | **GPL-3.0** | `ferveo-nucypher` 0.4.0 (2025-08). Last push 2026-05. | Purpose-built for threshold-encrypted mempools. | GPL would force aether-node's own license to GPL. The project is slowing. |
| Shutter | shutter-network/rolling-shutter | MIT | Push 2026-08 | Design reference. | Written in Go. Not a Rust library. |
| threshold_crypto | poanetwork/threshold_crypto | MIT/Apache (unverified) | Last crate 0.4.0 in 2020, last push 2024-08 | Avoid. | Effectively abandoned. |
| blst | supranational/blst | Apache-2.0 | 0.3.17, active | Low-level BLS12-381 building block underneath. | Primitive only, no threshold protocol. |

## 4. Signatures

| Crate | Repo | Version | Notes |
|---|---|---|---|
| ed25519-dalek | dalek-cryptography/curve25519-dalek | **3.0.0** (2026-07) | Default choice for transaction and validator keys. 3.x breaks compatibility with 2.x. |
| k256 | RustCrypto/elliptic-curves | 0.14.0 (2026-07) | Pure-Rust secp256k1 ECDSA. Pair with `alloy-primitives` 1.7.3 for Ethereum `Address` and signer recovery. |
| secp256k1 | rust-bitcoin/rust-secp256k1 | 0.33.1 (2026-08) | Wraps the C libsecp256k1. Faster, but pulls in a C dependency. |

## 5. Deterministic simulation testing

| Tool | Repo | License | Status | Notes |
|---|---|---|---|---|
| Commonware `runtime::deterministic` | monorepo | Apache-2.0 | Active; the runtime crate is BETA | Seeded scheduler plus simulated network. Comes free if you adopt Commonware. |
| turmoil | tokio-rs/turmoil | MIT | 0.7.2 (2026-04), last push 2026-07 | Simulates a network for tokio code. Easiest to add on its own. |
| madsim | madsim-rs/madsim | Apache-2.0 | 0.2.34 (2025-10), last push 2026-02 | Replaces tokio at the crate level. Heavier setup, and activity is slowing. |

## 6. Free benchmark and test infrastructure

| Option | What's free | Caveats |
|---|---|---|
| GitHub Actions | Standard runners on public repos: no minute cap (from GitHub's documented policy; the limits page fetched only lists 2,000 minutes for private repos), 6-hour job limit, 20 concurrent jobs | Shared runners are noisy, so don't fail PRs on raw criterion timings from them. Hardware specs unverified. |
| CodSpeed | Open source: unlimited runs, repos and users; 3 months of history; PR checks; 600 minutes a month on dedicated machines ("macro runners"), then $0.032/min | Use `codspeed-criterion-compat` 5.0.2. It counts CPU instructions, so results are stable enough for regression checks. |
| Bencher.dev | Public projects free; the free dedicated runner allows 1 concurrent job and a 5-minute timeout | Pro plan is $100/month plus $1/hr for dedicated runners. |
| Commonware harnesses | `deployer` (AWS multi-region testnet tooling) and `bench` in the monorepo | Running on AWS costs money. |
| Sui benchmark scripts | Sui repo | Specific to Sui. |
| Own machines | poc-cuda, poc-m3 and poc-nas can run a small testnet over Tailscale | |

## Top recommendation (solo developer)

**Build on the Commonware stack.** Use `simplex` for consensus, `p2p` for networking between validators, `broadcast` to spread mempool transactions, `storage`, `runtime::deterministic` for simulation tests, and `cryptography` (ed25519 for signing, the BLS DKG plus `tle` for the encrypted mempool). It's one Apache-licensed, actively maintained family whose parts already fit together, with explicit stability levels. It replaces four of the fake subsystems at once and comes with deterministic tests.

The trade-off is dropping the "DAG-BFT" label: Simplex isn't a DAG, and the only maintained Rust DAG-BFT (Sui's `consensus`) is too tied to Sui to extract. If open, permissionless gossip for full nodes or RPC outside the validator set is needed later, add rust-libp2p 0.57 (gossipsub plus kad) or iroh 1.2 for that layer only. Use k256 with alloy-primitives only if Ethereum-style addresses are needed.

**Still unverified:** the aptos-core and espresso-network licenses, whether Commonware has had an external audit, and the GitHub runner hardware specs. No files in the aether-node repo were modified.
