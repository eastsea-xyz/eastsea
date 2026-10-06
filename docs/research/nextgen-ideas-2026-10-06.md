# Next-gen chain ideas worth taking into EastSea (2026-10-06)

Founder request (2026-10-06): "솔라나 말고도 차세대 블록체인들 조사해서 아이디어 좋은 것들 다 가져와."
Scope: what other chains have shipped or are shipping that fits **consumer Macs as nodes (wallet = node), no founder control, hide the tech**.
This builds on the team's earlier frontier scans (all in `docs/research/`): `nextgen-consensus.md`, `nextgen-execution.md`, `nextgen-state.md`, `nextgen-wallet.md`, `nextgen-zk.md`, `nextgen-da-mev-ai.md` (all dated 2026-09-26), plus `scaling-paths-2026.md`, `public-read-access-2026-10-05.md`, and `app-launch-discovery-2026-10-05.md`. I did not repeat their findings here; I only refer to them.
Evidence comes from web searches run on 2026-10-06. Most metrics come from vendor blogs or aggregators (blockeden, eco.com, kucoin) and are marked **(unverified)** unless they come from primary docs.

## 0. Summary

1. **EastSea already has about a third of the "next-gen" list.** Examples: Simplex with threshold BLS (Tempo, launched March 2026, runs the same Commonware Simplex and reth stack, so the base is validated), block access lists with a static DAG (Glamsterdam is only now shipping them), inclusion lists, P-256 with 7702 (passkey accounts), paid state growth, history expiry through era files and torrents, a binary state tree, a receipts root, and a light client with a fixed group key. These are not re-proposed.
2. **The big theme of 2025-2026 is that users never think about gas.** Sui made stablecoin transfers gasless at the protocol level (2026-05-21; about $65B moved in the first 5 days, unverified). Plasma has zero-fee USDT through a protocol paymaster. Tempo has reserved "payment lanes" and fees paid in stablecoins. Base has sub-accounts and spend permissions, so most actions need no prompt. **This is EastSea's biggest gap:** since A6, every transaction on the new genesis pays the state fee, and the sponsor pool (E14 / doc 22 tier 3) is still only a design.
3. **The second theme is "the wallet is the distribution channel".** Telegram/TON mini apps report about 500M MAU (unverified). Base app and Farcaster mini apps follow the same pattern. EastSea has this planned as the app registry (doc 31). The new idea to add is per-app sub-accounts with spend caps.
4. **Free find: timelock encryption needs no consensus change.** The committee already threshold-signs a predictable message `seed_message(chain_id, draw)` under `SEED_NAMESPACE` with the fixed group key (`crates/node/src/handoff.rs:105`). Anyone can encrypt to "draw N", and the ciphertext opens when that draw seed is published. This enables sealed bids, fair launches and commit-reveal at the app level (Sui Seal and Shutter do this with separate key committees).
5. **Before beta: build nothing.** The beta gate is frozen, and none of these ideas is genesis-only (the transaction format can change later through a payload version). Two zero-code checks are listed in §3.
6. **Rejected because they conflict with the founder principles:** Hyperliquid/Aptos protocol order books, Berachain proof of liquidity, EigenLayer restaking, NEAR chain-signature custody, zkLogin/keyless as the primary key, foundation-run paymasters, permissioned validator sets (Tempo), Arweave-style endowments, and company-owned protocol infrastructure (Farcaster). See §4.

Legend. **Cat**: (a) app/wallet-level, ships anytime · (b) node, non-consensus (signed app release) · (c) consensus change, needs a protocol upgrade with 7-day notice · (d) does not fit. **Cost**: S ≤1 week · M 2-4 weeks · L 1-3 months · XL >3 months. **Pri**: MUST / NICE / WATCH / no.

---

## 1. Already in EastSea (do not re-propose)

| Idea (source) | EastSea equivalent | Where |
|---|---|---|
| Simplex + threshold BLS, ~0.6-1 s deterministic finality (Tempo mainnet 2026-03, Commonware) | Same | `07-consensus.md`, `28-dkg-agreement-spec.md` |
| Block-level access lists + static parallel DAG (EIP-7928, Glamsterdam Q4 2026; grevm) | BAL as a validity rule, Block-STM | `04-execution.md` |
| FOCIL inclusion lists (EIP-7805, Hegota 2027) | `inclusion_list` in the payload | `04-execution.md`, `crates/node` |
| Passkey smart accounts (Base Smart Wallet, Coinbase webauthn-sol; EIP-7951 P256VERIFY) | Secure Enclave P-256 + 7702 + ERC-1271 | `09-wallet.md`, `EastSeaAccount.sol` |
| Native AA, every account is a contract (zkSync, Starknet) | Same effect through 7702 delegation to `EastSeaAccount` | |
| Multidimensional / state gas (EIP-8037) | Burst bucket for state growth + 3 fee dimensions | `27-state-fee.md` |
| History expiry (EIP-4444), era files | 30-day window + era files + torrents | `12-launch-plan.md`, `13-roadmap.md` |
| Binary unified state tree (EIP-7864, Verkle abandoned) | Implemented | `nextgen-state.md` |
| Receipts commitment for light clients | B3 receipts root | `04-execution.md` |
| Succinct light-client verification (Mina's "22 KB chain", Helios) | One BLS check against the fixed group identity per block; Jolt block proofs | `crates/light`, `public-read-access` |
| Unbiasable randomness beacon (drand, Aptos randomness) | Threshold-BLS epoch randomness | `Randomness.sol` |
| Session keys / scoped agent permissions (ERC-7715, MetaMask Advanced Permissions) | Agent session keys (E11) | `09-wallet.md` |
| Social / guardian recovery (Argent) | Recovery keys, paper key, delayed recovery | `09-wallet.md` |
| Multisig with delays (Safe, Squads) | `EastSeaVault` | `16-vault.md` |
| Name service (ENS, TON DNS) | `EastSeaNames`, burn fee, no auction | `26-name-service.md` |
| Shareable call links (Solana Actions/Blinks) | `aether://call` (E5-lite) | `12-launch-plan.md` |
| Trustless swaps without a custodial bridge | HTLC `AtomicSwap` | `21-crosschain.md` |
| Signed, reproducible client releases on chain | `ReleaseLog` 2/3 builders | `19-release-approval.md` |
| Free lane for special transactions (Plasma, Sui system txs) | Free registration lane; zero base fee below target | `22-gas-pool.md` |
| Single-VM simplification (Sei SIP-3 drops CosmWasm, EVM-only since 2026-04) | EVM-only from day 1 | |
| Proof market with many small provers (Succinct network, Boundless) | Proof share for registered Macs, capped at 1/16 per operator | `13-protocol-2.md`, `15-node-rewards.md` |

Planned but not built. These are confirmed as good by the survey, so keep them; they are not new: E12 x402, E13 streaming, **E14 paymaster / doc 22 tier 3 sponsor pool**, E16 payment requests, doc 31 app registry, doc 32 health signal, C3 erasure-coded propagation, and encrypted mempool / BTE (`nextgen-da-mev-ai.md`).

---

## 2. Idea catalog (new or sharpened ideas)

### 2.1 Fees and payments: "the user never sees gas"

| # | Idea | Source | Evidence | EastSea mapping | Value (consumer Mac) | Cost | Risk | Cat | Pri |
|---|---|---|---|---|---|---|---|---|---|
| 1 | **Protocol transfer lane**: plain DBLN (and later any ERC-20 `transfer`) to an existing account gets reserved space in each block and a flat tiny fee, so a launchpad mint storm cannot price out payments | Tempo payment lanes; Sui Address Balances gasless transfers (2026-05-21); Plasma zero-fee USDT | Sui: about $65B in 5 days, over $1T in stablecoin transfers since 2025-08 (unverified). Plasma: $2.0B TVL 2026-04 (unverified). Tempo: mainnet 2026-03, Visa and Stripe as validators | Second bucket next to the state bucket: N transfer slots per block. Transfers that touch no new state pay only the bytes fee from 27, priced from a separate lane base fee | High: "send money" is the core consumer action and must stay cheap and predictable | M | Spam through many tiny transfers; must charge new-account state (already 100 units) | c | **MUST** (first upgrade) |
| 2 | **Open sponsor contract (paymaster)**: anyone funds a pool that pays fees for users matching a rule (new Mac, this app, under X/day) | ERC-4337 paymasters, zkSync/Starknet native paymasters, Sui sponsored transactions | Paymasters sponsored about 132M UserOps (unverified, `nextgen-wallet.md`); zkSync native AA since 2023-03 | Already E14 / doc 22 tier 3. **Sharpen:** let *app builders* fund their own users (Sui model) through a sponsor field verified by the account contract and relayed by any node. No Pipln-funded pool. | High: zero-balance onboarding after A6 broke "balance 0 works" | M | Sponsor drain (needs per-device caps; can use the DeviceCheck registration) | a | **MUST** |
| 3 | **Pay fees in any token** through a built-in fee swap | Tempo Fee AMM; zkSync/Starknet ERC-20 paymasters | Tempo mainnet (docs) | Paymaster variant of #2 that takes bridged USDC or any token at the DEX price. Only after a bridged stablecoin and the DEX exist | Medium | M | Oracle and DEX manipulation; legal (stablecoin handling, receive-only rule holds) | a | NICE (after DEX) |
| 4 | **Payment memo / invoice reference** carried with a transfer | Tempo TIP-20 memos; Stellar memo | Tempo docs | E16 payment requests: put a 32-byte memo hash in calldata of a `transferWithMemo` helper. No format change | Medium: merchants and agents reconcile payments | S | none | a | NICE |
| 5 | **Local fee markets / per-hot-spot surcharge**: congestion on one contract raises fees only for transactions that write that contract | Solana local fee markets; Monad and Sui object congestion control | Solana: "eliminated global congestion" but created "noisy neighbor" write-lock attacks (zealynx 2026) | The BAL already lists each transaction's write set. Add a per-hot-key surcharge where the base fee rises per key | Medium-high once a launchpad/DEX lives on chain | L | Noisy-neighbor attack, extra fee complexity | c | NICE (6-12 m) |

### 2.2 Accounts and wallet UX: "no prompts for small things"

| # | Idea | Source | Evidence | EastSea mapping | Value | Cost | Risk | Cat | Pri |
|---|---|---|---|---|---|---|---|---|---|
| 6 | **Per-app sub-accounts + spend permissions**: an app in the built-in browser gets its own sub-account, which can spend up to X/day from the main account without Touch ID | Base Account Sub Accounts (ERC-7895) + Spend Permissions | Base docs (live) | Extend the E11 session keys: one session key per app registry `appId`, a cap, expiry, and a "revoke all" button. The wallet creates it on first connect | High: removes the Touch ID popup on every in-app action; also isolates apps | M | Bad cap defaults; a phishing app gets the cap → keep caps small and show them | a | **MUST** |
| 7 | **Per-dApp address isolation** (privacy): each app sees a different address | Ethereum Kohaku roadmap (EF, phase 2, 2025-26) | Kohaku SDK alpha (unverified) | Falls out of #6 if sub-accounts are fresh addresses | Medium (privacy without a mixer) | S (with #6) | Users confused by many addresses → hide them, show one balance | a | NICE |
| 8 | **Atomic multi-step with result piping** ("PTB"-style) | Sui Programmable Transaction Blocks | Core Sui primitive, widely used | `prepare_batch` (7702 batching) exists. Add a typed "recipe" builder: swap → send output → stake, simulated as one step | Medium | M | Simulation mismatch | a | NICE |
| 9 | **Synced-passkey recovery signer**: an optional iCloud-synced WebAuthn passkey as an extra recovery key, so a lost Mac or iPhone restores with only Apple ID + device PIN | Base/Coinbase Smart Wallet (synced passkeys); zkSync SSO | Coinbase Smart Wallet in production | Needs WebAuthn `clientDataJSON` verification in the account (not in code today). Ship through a new account deploy + re-delegate | High: the paper key is the weakest consumer step | M | Apple ID takeover → recovery only through the existing 48 h delay | a | NICE (3 m) |
| 10 | **Login with Google/Apple as the account key** (zkLogin, Aptos Keyless) | Sui zkLogin (≥4M transactions by 2024-05), Aptos Keyless | Sui and Aptos blogs | Needs on-chain OIDC JWK updates (validator oracle → consensus), a salt service, and a prover service | Easy onboarding, but the SE key is already zero-seed | L | Third-party identity dependency; a central salt/prover service conflicts with no-founder-control | d (as primary) | no |
| 11 | **"Sign in with EastSea"** (SIWE / EIP-4361 with ERC-1271) for web services | Sign in with Ethereum / Base; Nostr NIP-07 | Widely deployed | ERC-1271 exists. Add a SIWE message format and a wallet prompt | Medium: gives external services a reason to integrate | S | none | a | NICE |
| 12 | **Pay a contact** (name in Apple Contacts / iMessage share) | Telegram wallet (pay in chat); TON usernames | TON Wallet: 100M+ activations in 2024, US launch 2025-07 (unverified) | Store an `.aeth` name in a contact card and resolve it in Send; share a request link via the iMessage share sheet | High: distribution through existing social graphs | S-M | iMessage extension review | a | NICE |
| 13 | **Post-quantum signer slot** in the account | NEAR ML-DSA (FIPS-204), 2026; EIP-8141 / 7932 | NEAR PQ keys (cryptobriefing, unverified date); EF PQ team | The account accepts a second signer type when a cheap verifier exists. CryptoKit PQ API availability needs checking. Contract-level first, precompile later | Low now, high later | L | Large signatures and gas | a→c | WATCH |

### 2.3 Node, RPC and distribution

| # | Idea | Source | Evidence | EastSea mapping | Value | Cost | Risk | Cat | Pri |
|---|---|---|---|---|---|---|---|---|---|
| 14 | **Send-and-wait RPC** (`realtime_sendRawTransaction`: submit and return the receipt in one call) + **`eth_subscribe`** streams (heads, logs, own-account changes) | MegaETH Realtime API (mainnet 2026-02-09) | MegaETH docs | Node RPC addition over iroh `aether/rpc/1` and the read gateway. With 1 s finality, a receipt arrives in about 1 s and the "Sent ✓" moment needs no polling | Medium-high: snappier wallet and dApps; cuts RPC load | S-M | none (read path) | b | **MUST** (3 m) |
| 15 | **Wallet-embedded mini apps with a manifest** | Telegram/TON Mini Apps (about 500M MAU, unverified), Farcaster/Base mini apps | blockeden 2026-01 (unverified) | = doc 31 app registry (planned). Add from the survey: share-link deep links and no-signup first open (sponsored by the app, #2 + #6) | High | (planned) | Apple 4.7 / 3.1.5 | a | MUST (planned) |
| 16 | **Third-party front-end fees, disclosed on chain** ("builder codes") | Hyperliquid builder codes | Hyperliquid fee data (unverified) | Registry field: a front end declares its fee address and rate; the wallet shows it before signing. Pipln takes nothing | Medium: lets independent teams earn money, so they build without Pipln | S | Legal (third party bears it) | a | NICE |
| 17 | **Proof-only follower mode**: a low-disk Mac follows by verifying the committee certificate + Jolt proof + state diff instead of re-executing | EIP-8025 optional execution proofs (Hegota); Mina; stateless clients | EF zkEVM blog (devnets) | Follower option once proofs are reliably on time (the 5-day proof outage shows they are not yet) | Medium: older and low-disk Macs can still serve | L | Proof liveness; the follower trusts the committee for system writes (doc 13 §1) | b | WATCH (6-12 m) |
| 18 | **Recursive "whole-chain" proof**: a new phone checks one proof that covers genesis → now | Mina (22 KB chain); SP1-Helios | Mina Mesa devnet 2026-08-19 | The fixed group key already makes header sync O(1). Recursion adds **execution** validity for the whole history | Low-medium (the BLS path already suffices for safety) | XL | Prover cost | b | WATCH |
| 19 | **App bundle and blob storage on Mac nodes** | Walrus (467 TB in year 1, unverified), Filecoin Onchain Cloud (2026-01), Arweave | sui.io and decrypt 2026 | App registry bundles by content hash, served over the existing torrent/iroh paths; any node can seed | Medium | M | Storage rewards stay off (protocol 4 skeleton); no paid storage promises | b | NICE |

### 2.4 Consensus and protocol (post-launch upgrades)

| # | Idea | Source | Evidence | EastSea mapping | Value | Cost | Risk | Cat | Pri |
|---|---|---|---|---|---|---|---|---|---|
| 20 | **Timelock / conditional encryption with the committee key** (encrypt to "draw N"; later to "block H") | Sui Seal (mainnet 2025-09, 5-of-8 MPC key servers), Shutter (Gnosis), drand tlock | Seal docs; Shutter live since 2024-07 | **Draw version needs no consensus change:** `seed_message(chain_id, draw)` is predictable and signed by the fixed group key (`handoff.rs:105`); Commonware `bls12381::tle` exists. A height-scheduled version (sign `(chain_id, height)` each block) is (c) | High for fair launches, sealed-bid name auctions, commit-reveal games, and the future encrypted mempool | S (a) / M (c) | The draw cadence follows rotations, not wall-clock time; if ≥ threshold of the committee colludes, a ciphertext opens early (same trust as finality) | a, later c | NICE (3 m) |
| 21 | **Encrypted mempool** (decrypt at commit) | Shutter, Ferveo, Commonware BTE | `nextgen-da-mev-ai.md` | Already recommended there; #20 is the first step | High vs. MEV on the launchpad and DEX | L | Latency +1 block | c | WATCH (6-12 m) |
| 22 | **Fast-path finality (Votor 20+20 / Minimmit)** | Solana Alpenglow (mainnet activation started 2026-09-28); Commonware Minimmit (paper) | Anza schedule; `nextgen-consensus.md` | Only worth it with more than about 50 validators; we have 4-16 | Low now | L | Safety margin drops to n/5 | c | WATCH |
| 23 | **Erasure-coded block propagation** | Solana Rotor/Turbine, PeerDAS, Commonware coding | PeerDAS live 2025-12 | = C3 (planned). Matters when uplinks on home connections are slow | Medium | L | — | c | (planned) |
| 24 | **Multiple concurrent proposers** (censorship resistance) | Mysticeti DAG, Ethereum MCP research | Sui mainnet | FOCIL inclusion lists already cover most of it at 4-16 seats | Low | XL | Complexity | c | no (for now) |
| 25 | **ZK-compressed / rent-free cold state** | Solana ZK Compression v2 (70-1000×, Q3 2025, unverified) | dextools 2026 (unverified) | Paid state growth already bounds disk; compression would need a new account kind | Low | XL | EVM incompatibility | c | no |

### 2.5 Evaluated: does not fit

| # | Idea | Source | Why not for EastSea |
|---|---|---|---|
| 26 | Protocol-native order book (HyperCore; Aptos framework CLOB 2026) | Hyperliquid ($4T+ cumulative perps by 2026-05, unverified) | A regulated trading venue built into the protocol by Pipln (doc 12 excludes leverage and prediction; DEX kept out of genesis). Also needs datacenter-grade validators |
| 27 | Proof of Liquidity | Berachain | TVL fell from $3.35B to about $0.39B (−88%), BERA down >90%, investor refund-clause controversy (blockeden 2026-01-26, unverified). Ties security to DeFi yield; incompatible with Mac-operator rewards |
| 28 | Restaking / AVS | EigenLayer (slashing live 2026-04-17, about $15-18B exposed, unverified) | Needs ETH stake and external security; Mac-only Sybil design is the opposite |
| 29 | MPC "chain signatures" controlling BTC/ETH addresses | NEAR (Intents $10B volume, unverified) | The committee would hold users' foreign assets, which is a custodial bridge (forbidden in doc 21). Keep HTLC + light-client bridge |
| 30 | Intents / solver networks for cross-chain swaps | NEAR Intents, ERC-7683 | Fits **later** as a quote format for HTLC maker bots (a, NICE 6-12 m). A solver network with a privileged relayer does not fit |
| 31 | Sharding (TON workchains, NEAR Nightshade) | TON, NEAR | `scaling-paths-2026.md`: splitting the committee is an anti-pattern at 16 seats |
| 32 | Datacenter-class sequencer (10 ms blocks) | MegaETH, Monad (MonadDb on raw NVMe) | Needs 100-core or specialized hardware; consumer Macs cannot be that node. Take the RPC ideas (#14) instead |
| 33 | Sovereign DA layer (Celestia, Avail) / EigenDA | | Our Macs *are* the DA layer; an external DA adds a second network dependency. Revisit only if we settle to Ethereum |
| 34 | ePBS / proposer preconfirmations (EIP-7917 lookahead) | Ethereum | No builder market at 4-16 seats; 1 s finality makes preconfirmations nearly pointless |
| 35 | Shielded pools (Railgun, Privacy Pools, Aleo USAD) | Aleo USAD mainnet 2026-02 | Mixer-like features are on the "will not build" list (doc 12). Take only the wallet-side privacy (#7) |
| 36 | UTXO + predicates, native multi-asset | Fuel | Breaks EVM tooling; 7702 account policies cover predicates |
| 37 | Permanent-storage endowment | Arweave | Needs a long-dated token treasury promise; conflicts with the "no rights/no promises" stance |
| 38 | Fully on-chain social graph | Farcaster (Neynar bought it 2026-01, then sought a new owner and disbanded, 2026-08) | Lesson only: a protocol whose hubs, client and money flow sit in one company dies with that company, which confirms no-founder-control. Not a feature to build now |
| 39 | Nostr secp256k1 identity / NIP-46 bunker | Nostr | The Secure Enclave cannot hold secp256k1 keys; offer #11 (SIWE) instead |
| 40 | Second independent client (Firedancer) | Solana | Right long-term goal, XL effort; for now use reproducible builds + shadow execution (have) + an independent light verifier. Revisit when external teams exist |

---

## 3. Prioritized adoption roadmap

### Before beta (gate frozen: no consensus or node changes)

| Item | Type | Why it is safe |
|---|---|---|
| Check that the committee **group public key stays fixed across reshare** on the new genesis, and document that `seed_message(chain_id, draw)` is a public, stable API (needed by #20) | doc only | No code |
| Check that nothing in the beta payload or transaction format blocks a later **sponsor field** (#2) or a **transfer lane** bucket (#1), meaning both can arrive by payload version | doc only | No code; confirms that neither is genesis-only |

Nothing else. None of the 40 ideas is genesis-only.

### First 3 months after beta

| Order | Item | # | Cat | Cost |
|---|---|---|---|---|
| 1 | Open sponsor contract (app builders fund their own users; per-Mac caps) | 2 | a | M |
| 2 | Send-and-wait RPC + `eth_subscribe` | 14 | b | S-M |
| 3 | Per-app sub-accounts + spend caps (with registry phase 1) | 6, 7 | a | M |
| 4 | Protocol transfer lane: design → upgrade drill → 7-day notice | 1 | c | M |
| 5 | Timelock encryption to draw N (SDK + one demo: sealed-bid or fair-launch commit) | 20 | a | S |
| 6 | Payment memo helper + "pay a contact" + SIWE | 4, 11, 12 | a | S each |
| 7 | Synced-passkey recovery signer (WebAuthn verify in a new account version) | 9 | a | M |

### 6-12 months

| Item | # | Cat | Trigger |
|---|---|---|---|
| Per-hot-key fee surcharge (local fee markets) | 5 | c | Launchpad/DEX congestion visible in fees |
| Encrypted mempool (height-scheduled key, decrypt at commit) | 20c, 21 | c | DEX/launchpad live; MEV complaints |
| Third-party front-end fee disclosure in the registry | 16 | a | External front ends appear |
| App bundle storage on nodes | 19 | b | Registry phase 2 |
| Pay fees in any token | 3 | a | Bridged USDC + DEX exist |
| Intent-style quotes for HTLC maker bots | 30 | a | Atomic swap phase 1 shipped |
| Proof-only follower mode | 17 | b | 30 days of on-time proofs |
| Erasure-coded propagation (C3), fast-path finality | 23, 22 | c | Over 16 seats / over about 50 validators |
| PQ signer slot | 13 | a→c | CryptoKit PQ API confirmed + cheap verifier |

---

## 4. Conflicts with founder principles

| Idea | Principle hit | Ruling |
|---|---|---|
| Foundation-funded paymaster (Plasma, early Sui sponsors) | No founder control / Pipln pays nothing | Only **open** pools (anyone funds) and the burn-funded pool from doc 22. Pipln never funds or allowlists |
| zkLogin / Keyless (#10) | No central service; hide the tech | Needs a salt service and OIDC key oracle run by someone, plus a Google/Apple dependency. Reject as primary key |
| Seal-style key servers run by named companies | No founder control | Use the existing committee key (#20) |
| Permissioned validators (Tempo: Stripe, Visa) | Open Mac committee | Take the payment lane idea (#1), not the governance |
| Protocol order book, PoL, restaking, chain signatures (#26-29) | No regulated finance; no custody; no founder-run venue | Reject |
| Third-party "builder codes" (#16) | OK only if Pipln takes 0 and has no curation | The registry shows fees as data; no ranking boost |
| Transfer lane / sponsor caps using DeviceCheck registration | The registrar is a Pipln-held key (bounded) | Acceptable: reuses an existing bounded role; must not add new Pipln powers |
| Mini apps on iOS (#15) | Consumer distribution vs. Apple 3.1.5 | No coin rewards for referrals on iOS (`app-launch-discovery` §0.5) |
| Sub-accounts (#6, #7) | Hide the tech | The user sees one balance and per-app limits, never addresses |

---

## Sources (accessed 2026-10-06)

- Sui gasless stablecoin transfers / Address Balances: https://www.sui.io/blog/sui-launches-gasless-stablecoin-transfers, https://docs.sui.io/develop/transaction-payment/gasless-stablecoin-transfers, https://cryptobriefing.com/sui-gasless-stablecoin-transfers-protocol-level/ (volume figure unverified)
- Sui zkLogin / sponsored tx / PTBs: https://blog.sui.io/account-abstraction-explained/, https://blog.sui.io/sui-primitives-revolutionize-onchain-gaming/
- Sui Seal: https://seal.mystenlabs.com/, https://blog.sui.io/introducing-decentralized-seal-key-server-testnet/, https://rubynodes.io/news/article/1/seal-mpc-mainnet
- Walrus year one: https://www.sui.io/blog/celebrating-walrus-one-year-anniversary.md, https://decrypt.co/362558/all-to-play-for-walrus-hits-450tb-of-data-stored-amid-renewed-ai-push
- Aptos Keyless / 2026 roadmap: https://aptos.dev/build/guides/aptos-keyless/introduction, https://everstake.one/resources/blog/aptos-news-2026
- MegaETH Realtime API, mainnet 2026-02-09: https://docs.megaeth.com/realtime-api, https://docs.megaeth.com/mini-block, https://goldrush.dev/docs/changelog/20260206-megaeth-mainnet-now-supported
- Hyperliquid: https://blog.portals.fi/hyperliquid-perp-dex-dominance/, https://www.quicknode.com/blog/hyperliquid-developer-stack (figures unverified)
- Berachain PoL review: https://blockeden.xyz/blog/2026/01/26/berachain-one-year-later-proof-of-liquidity-reality-check/ (unverified)
- Solana local fees / Firedancer / ZK compression: https://www.zealynx.io/research/smart-contracts/solana-2026-security, https://www.blockdaemon.com/blog/what-is-firedancers-status-and-what-does-it-mean-for-solana, https://www.dextools.io/news/solana-firedancer-alpenglow-ecosystem-momentum-june-2026-ko
- Alpenglow activation 2026-09-28: https://cryptoticker.io/en/solana-alpenglow-activation-date-validator-check/, https://alchemy.com/blog/solana-alpenglow
- Tempo: https://www.quicknode.com/blog/quicknode-launches-support-for-tempo-mainnet, https://blockeden.xyz/blog/2026/03/10/stripe-tempo-blockchain-stablecoin-payment-chain/
- Plasma: https://eco.com/support/en/articles/11802920-what-is-plasma-xpl-stablecoin-l1-in-2026 (unverified)
- Monad: https://blockeden.xyz/de/blog/2026/04/12/monad-mainnet-parallel-evm-tps-vs-distribution-thesis/ (unverified)
- Sei SIP-3: https://blog.sei.io/announcements/the-sip-3-upgrade-making-way-for-sei-giga/
- Glamsterdam status: https://thedefiant.io/news/blockchains/ethereum-glamsterdam-final-devnet-200m-gas-limit-target, https://www.moonpay.com/learn/cryptocurrency/ethereums-glamsterdam-upgrade-explained
- EIP-7917 / preconfirmations: https://eips.ethereum.org/EIPS/eip-7917
- Kohaku: https://blog.quicknode.com/ethereum-kohaku-wallet-privacy-roadmap
- EigenLayer slashing 2026-04-17: https://blockeden.xyz/blog/2026/04/18/eigenlayer-avs-slashing-activation-15b-restaking-reality-check/ (unverified)
- NEAR Intents / chain signatures / PQ: https://nansen.ai/post/nansen-near-quarterly-report-q2-2026, https://cryptobriefing.com/?p=349560
- TON / Telegram: https://blockeden.xyz/blog/2026/01/26/ton-telegram-web3-onramp-mini-apps-500-million-users/, https://www.nbcnewyork.com/news/business/money-report/telegrams-crypto-wallet-goes-live-to-its-87-million-u-s-users/6343765/ (unverified)
- Base Sub Accounts / Spend Permissions: https://docs.base.org/base-account/improve-ux/sub-accounts
- zkSync / Starknet native AA: https://docs.zksync.io/zksync-protocol/account-abstraction, https://eco.com/support/en/articles/15254047-erc-4337-vs-erc-7702-vs-native-aa-2026-standards-compared
- Fuel predicates: https://docs.fuel.network/docs/fuel-book/the-architecture/the-fuelvm/
- Mina Mesa: https://minaprotocol.com/blog/minas-mesa-upgrade-what-to-expect
- Aleo USAD: https://www.theblock.co/post/389101/privacy-preserving-usad-stablecoin-launches-aleo-layer-1-mainnet-paxos-partnership
- Farcaster / Neynar: https://neynar.com/blog/neynar-is-acquiring-farcaster, https://thedefiant.io/news/nfts-and-web3/neynar-seeks-new-owner-farcaster-clanker
- Nostr NIP-46: https://opensats.org/topics/remote-signing
- Filecoin / Arweave: https://blockeden.xyz/forum/t/filecoin-pivoted-to-onchain-cloud-arweave-launched-a-parallel-computing-layer-and-walrus-costs-5x-less-than-aws-the-storage-wars-just-got-real/530
- Earlier EastSea scans (Celestia/Avail/PeerDAS, Shutter/FOCIL, Block-STM/grevm, EIP-7864, Jolt, 7702/P256): `docs/research/nextgen-*.md` (2026-09-26)
