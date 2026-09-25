# aether-node reuse research (verified 2026-09-25 via GitHub API + crates.io)

aptos-core, firewood and current Blockscout have licences that rule them out for a production chain. Everything else in scope is permissive (MIT, Apache or CC0) and maintained. No code was run and no project files were changed.

## 1. Parallel execution (Block-STM)
| Name | Repo | License | Status | How aether-node reuses it | Caveats |
|---|---|---|---|---|---|
| aptos-block-executor | github.com/aptos-labs/aptos-core | **Aptos "Innovation-Enabling Source Code License"**: no deploying any chain (mainnet or testnet) outside personal, non-commercial use; becomes Apache-2.0 4 years after each piece of code is published | Active (pushed 2026-09-25), 6.4k stars | Read it as a reference design only | **Licence blocks reuse.** Tied to Aptos types and MoveVM. Not on crates.io |
| RISE pevm | github.com/risechain/pevm | MIT | **Not archived**, but last commit 2026-06-05. crates.io has only 0.1.0 (2024). 355 stars | Git dependency. Uses revm 38 and alloy 2.0 | README says "work in progress, not production ready". The roadmap replaces revm with its own EVM |
| Galxe grevm | github.com/Galxe/grevm | MIT + Apache-2.0 (dual) | Active: last commit 2026-08-18, v2.2.6 (2026-06-17), 38 stars | **Best fit.** Git dependency on revm 40.0.3 and alloy-evm 0.36. Includes a `docs/use-with-reth.md` integration guide | Small community. 3 major versions behind the latest revm (43.0.3) |
| Monad | github.com/category-labs/monad | GPL-3.0 | Active | Reference design only | Written in C++, copyleft |

**Mainnet blocks as a correctness oracle: both have one.**
- **pevm:** the `pevm-fetch` CLI snapshots real Ethereum mainnet blocks into `data/ethereum`, and `crates/pevm/tests/mainnet.rs` runs them.
- **grevm:** `tests/mainnet.rs` uses the `Galxe/grevm-test-data` submodule.
- Either harness can be copied to test aether-node's parallel results against sequential revm.

## 2. EVM execution and node SDK
| Name | Repo | License | Status | How aether-node reuses it | Caveats |
|---|---|---|---|---|---|
| revm | github.com/bluealloy/revm | MIT | crate 43.0.3 (2026-09-24), 2.2k stars | Make the existing dependency the execution core | Breaking major releases come often |
| alloy-evm | alloy-rs | MIT/Apache (unverified) | 0.39.0 (2026-08-26) | Block-execution glue that sits on revm | — |
| reth SDK | github.com/paradigmxyz/reth | Apache-2.0/MIT | v2.6.0 (2026-09-17), 5.8k stars | Build the whole node on it. Examples folder has `custom-evm`, `custom-node-components`, `custom-engine-types`, `custom-payload-builder`, `custom-state-root`, `node-builder-api`. A custom consensus drives it through the Engine API | Heavy. Crates are pinned as git dependencies; crates.io `reth` is a 2023 placeholder |
| Tempo | github.com/tempoxyz/tempo | Apache-2.0 | Active, 1k stars | **Working template of reth plus a custom consensus (Commonware Simplex)** | Large codebase. Pins reth by git revision |
| Commonware | github.com/commonwarexyz/monorepo | Apache-2.0 | Active, 612 stars | Consensus and p2p primitives | — |
| Malachite | github.com/circlefin/malachite | Apache-2.0 | Active (2026-09-15) | Tendermint-style BFT library | — |
| alloy | github.com/alloy-rs/alloy | Apache-2.0/MIT | v2.5.0 (2026-09-23) | Types, RPC and signing | — |

## 3. Authenticated state storage
| Name | Repo | License | Status | How aether-node reuses it | Caveats |
|---|---|---|---|---|---|
| jmt | github.com/penumbra-zone/jmt | Apache-2.0 | crate 0.12.0; last commit 2026-01-07; 66 stars | Merkle tree over redb, replacing the JSON file | Maintenance is slow; the storage glue must be written |
| QMDB | github.com/LayerZero-Labs/qmdb | MIT | Last commit 2026-05-29; not on crates.io | Git dependency | Production readiness: unverified |
| firewood | github.com/ava-labs/firewood | **Ava Labs Ecosystem License** | Active, beta, crate 0.3.1 | Only on Avalanche | **Licence limits use to Avalanche or non-commercial research. Not usable here** |
| redb | github.com/cberner/redb | Apache-2.0/MIT | 4.3.0 (2026-09-15), 4.8k stars | **Easy win: plain key-value backend that replaces the JSON file** | Not authenticated on its own |
| MDBX | inside reth (`libmdbx` crate) | reth is Apache/MIT; libmdbx licence unverified | `libmdbx` 0.9.0 (2026-09-24) | Comes free if reth is adopted | — |

## 4. EVM correctness test suites
| Name | Repo | License | Status | How aether-node reuses it |
|---|---|---|---|---|
| ethereum/tests | github.com/ethereum/tests | MIT | Last push 2025-06-04, now legacy | Old GeneralStateTests |
| EEST | github.com/ethereum/execution-spec-tests | MIT | **Archived.** Moved into `ethereum/execution-specs` (CC0-1.0), latest fixtures `tests@v21.0.0` (2026-09-23) | Download the fixture tarballs from execution-specs releases |
| revm `revme` | `bins/revme` in the revm repo | MIT | Active | `revme statetest <fixtures dir>` runs state tests. Wire the same path into aether-node's executor |

## 5. Tools that work once the node speaks eth JSON-RPC
| Name | Repo | License | Status | Caveats |
|---|---|---|---|---|
| Blockscout | github.com/blockscout/blockscout | **Changed from GPL-3.0 to a custom "LicenseRef-Blockscout" licence on 2026-04-22** | v11.3.2 (2026-09-21), active | Monetised, SaaS or RaaS use needs a commercial licence. Whether pre-2026-04-22 versions stay GPL-3.0 is unverified |
| Otterscan | github.com/otterscan/otterscan | MIT | Last push 2026-02-02, 1.4k stars | Needs the `ots_` RPC namespace, which reth provides |
| Foundry / anvil | github.com/foundry-rs/foundry | Apache-2.0/MIT | Nightly builds daily, 10.6k stars | forge and cast work with any standard eth RPC. anvil is a separate local dev chain, not a client for the node |
| MetaMask | — | — | — | Needs `eth_chainId`, `eth_sendRawTransaction`, `eth_estimateGas`, receipts and EIP-1559 fee fields (unverified detail) |

## Recommended path
1. **Execution:** keep revm and replace the broken Block-STM executor with grevm (licence-clean and current).
2. **Correctness:** use grevm's or pevm's mainnet-block tests, plus `revme statetest` on the execution-specs fixtures.
3. **Storage:** replace the JSON file with redb, and add jmt if state proofs are needed.
4. **Longer term:** use Tempo's reth + Commonware layout as the template for a reth-based node with custom consensus.

Do not copy any aptos-core or firewood code, because of their licences.
