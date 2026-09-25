# Next-gen wallet & account frontier (verified 2026-09-26)

Context: Mac-native (later iOS) wallet + embedded verifying node, EVM-compatible chain, Secure Enclave keys, ≤300 KiB proofs verified on device.

## 1. Account abstraction

| Name | Status (Sep 2026) | Platform | Relevance | Source |
|---|---|---|---|---|
| EIP-7702 (EOA delegation) | Live since Pectra (2025-05-07); MetaMask (30M+ MAU) adopted late 2025; tens of millions of delegated/smart accounts | L1 + all major L2s | High: EOA keeps address, gains batching/sponsorship/alt-signers | [thirdweb](https://blog.thirdweb.com/account-abstraction-in-2026-how-eip-7702-and-erc-4337-are-transforming-ethereum-wallets-for-developers/), [Eco](https://eco.com/support/en/articles/15254037-erc-7702-deep-dive-2026-eoa-becomes-smart-wallet) |
| ERC-4337 EntryPoint v0.8 | Deployed; native 7702 support, ERC-712 UserOp hashing, `Simple7702Account` | Any EVM | High: reference account for 7702 delegation | [eth-infinitism releases](https://github.com/eth-infinitism/account-abstraction/releases) |
| ERC-4337 EntryPoint v0.9 | Released; parallel paymaster signing, block-number validity ranges, `getCurrentUserOpHash` | Any EVM | Medium: bundlers (Pimlico/Alchemy ≈65% of UserOps) still mostly on v0.7/v0.8 | same |
| EIP-7701 (native AA) | **Stagnant** (Apr 2026) | L1 | Low: superseded | [eip.tools](https://eip.tools/eip/7701) |
| EIP-8141 (Frame Transactions, tx type 0x06) | **SFI / "must-ship" for Hegotá** (post-Glamsterdam, ~2027). Base's EIP-8130 (keystore tx) proceeds separately after alignment talks ended 2026-09-15 | L1; Base diverges | High long-term: protocol-level validation/gas-payer/execute frames; plan for two tx formats | [The Block](https://www.theblock.co/news/ecosystems/2026-09-15-ethereum-base-account-abstraction-proposals-414775), [Everstake](https://everstake.one/resources/blog/native-account-abstraction-on-ethereum-what-eip-8141-means-for-validators) |
| RIP-7212 (P256 precompile, L2) | Live on OP Stack, Arbitrum, Polygon, zkSync etc. | L2 | High: 3,450 gas | [RIP-7212](https://github.com/ethereum/RIPs/blob/master/RIPS/rip-7212.md) |
| EIP-7951 `P256VERIFY` @0x100 | **Live on L1 since Fusaka (2025-12-03)**, 6,900 gas | L1 | Critical: makes Secure Enclave/passkey signers cheap everywhere | [EIP-7951](https://eips.ethereum.org/EIPS/eip-7951), [ethereum.org](https://ethereum.org/latest/building-on-ethereum-in-2026/) |

## 2. Passkeys / Secure Enclave signing

| Name | Status | Platform | Relevance | Source |
|---|---|---|---|---|
| Apple CryptoKit `SecureEnclave.P256.Signing` | Stable; macOS 26/iOS 26 SDK; keys non-exportable, biometric-gated | Swift | Primary signer | [Apple](https://developer.apple.com/documentation/cryptokit/secureenclave/p256/signing) |
| `cryptokit-rs` (Rust bridge to CryptoKit, SE P-256, PQ families) | v0.3 (2026), macOS 26 SDK | Rust→Swift | Lets Rust node core sign via SE | [docs.rs](https://docs.rs/cryptokit-rs/latest/cryptokit/) |
| `hardware-enclave` (GoDaddy) | Active; SE / TPM 2.0 P-256 sign + ECIES | Rust, cross-platform | Alternative if Windows later | [GitHub](https://github.com/godaddy/hardware-enclave) |
| `p256` (RustCrypto) | Stable; software verify, used inside zkVM guests | Rust | Verifier in node/prover | [crates.io](https://crates.io/crates/p256) |
| Coinbase `webauthn-sol` / Smart Wallet | Production; falls back to precompile-or-Solidity (FCL) verifier | Solidity | Reference for WebAuthn `clientDataJSON` parsing | [coinbase/smart-wallet](https://github.com/coinbase/smart-wallet/blob/main/README.md) |
| SP1 zkVM secp256r1 precompile | Since SP1 Turbo v4 | zkVM | P-256 verify cheap in-proof (bls: 512 sigs 6B→50M cycles shows precompile effect) | [Succinct](https://blog.succinct.xyz/sp1-turbo/) |

Cost: Solidity P-256 verify ≈200–330k gas; RIP-7212 3,450; EIP-7951 6,900 (vs ecrecover 3,000). Secure Enclave only does P-256 — it cannot hold secp256k1 keys, so the account must accept P-256 signers (7702 delegate or 4337 account).

## 3. Intents, session keys, sponsorship

| Name | Status | Platform | Relevance | Source |
|---|---|---|---|---|
| ERC-7683 cross-chain intents | Draft but production: Across (88% vol), UniswapX, LI.FI, Eco; Safe/Rabby/Argent emit orders; MetaMask 12.4 (Mar 2026) | EVM | Medium: bridge UX later | [Eco](https://eco.com/support/en/articles/14796366-erc-7683-cross-chain-intents-standard-deep-dive) |
| ERC-7715 permissions / session keys | Draft; MetaMask "Advanced permissions" + 7702 delegator | Wallet RPC | High: scoped, revocable dapp sessions | [Eco](https://eco.com/support/en/articles/15254037-erc-7702-deep-dive-2026-eoa-becomes-smart-wallet) |
| Paymasters (4337) / 7702 sponsor | ~$5.7M gas sponsored; 132M UserOps | EVM | High for onboarding on own chain | [Eco](https://eco.com/support/en/articles/15254040-what-is-a-paymaster-gas-sponsorship-explained-2026) |

## 4. Light-client / verify-by-proof wallets

| Name | Status | Platform | Relevance | Source |
|---|---|---|---|---|
| Helios (a16z) | Active Rust; Ethereum + OP Stack + Linea; WASM, iOS/macOS/Android; HeliosKit (Swift xcframework), react-native-helios | Rust/Swift | Direct model for "embedded verifying node" | [a16z/helios](https://github.com/a16z/helios), [HeliosKit](https://github.com/rkreutz/HeliosKit) |
| Nimbus Verified Proxy | Active (Sep 2026: P2P light-client sync default; libverifproxy C API) | Nim, desktop/server | Comparison only | [nimbus-eth1](https://github.com/status-im/nimbus-eth1/tree/master/nimbus_verified_proxy) |
| Kevlar | Dormant (2023-era) | CLI proxy | Low | [lightclients/kevlar](https://github.com/lightclients/kevlar) |
| SP1-Helios (ZK light client) | Maintained; sync-committee proof → tiny on-chain/on-device verify | Rust | Model for ≤300 KiB proof consumption | [Succinct](https://blog.succinct.xyz/succinctshipsprecompiles/) |

No shipping consumer wallet in 2026 verifies state by ZK proof on-device; Helios-style sync-committee light clients are the practical ceiling. A proof-verifying wallet is greenfield.

## 5. Post-quantum accounts

| Name | Status | Relevance | Source |
|---|---|---|---|
| EF PQ team / pq.ethereum.org | Formed Jan 2026; leanXMSS validator sigs | Roadmap tracking | [pq.ethereum.org](https://pq.ethereum.org/) |
| EIP-7932 Secondary Signature Algorithms | Draft framework | Future PQ signer slot | [EIP-7932](https://eips.ethereum.org/EIPS/eip-7932) |
| EIP-8030 P256 as 7932 type 0x01 | Draft (Sep 2025) | Native P-256 tx signing without AA | [EIP-8030](https://eips.ethereum.org/EIPS/eip-8030) |
| EIP-8141 signature agility | SFI for Hegotá | Per-account PQ migration path | [ethereum.org](https://ethereum.org/roadmap/security/quantum-resistance/) |

## 6. macOS/iOS specifics

| Item | Status | Relevance | Source |
|---|---|---|---|
| App Store 3.1.5 | Wallets allowed only from **Organization** developer accounts; SE + biometrics expected | Enroll as org before submission | [Apple guidelines](https://developer.apple.com/app-store/review/guidelines/) |
| macOS Tahoe 26.4 | Login keychain bound to SE; SE-backed SSH keys | Confirms SE-first design | [AppleInsider](https://appleinsider.com/articles/26/09/13/macos-tahoe-264-update-stops-you-from-copying-your-login-keychain) |
| Sparkle vs App Store | Sparkle (notarized DMG) avoids 3.1.5/IAP review; App Store gives trust + iOS parity | Ship Sparkle first, App Store for iOS | (industry practice) |
| Reference UX | Rabby (native macOS/Win desktop, pre-sign simulation, batch tx), Rainbow (mobile-first), MetaMask Snaps thin adoption | Copy Rabby's simulation + Rainbow's onboarding | [Protocol Signal](https://protocolsignal.com/comparisons/best-crypto-wallets/) |
| UniFFI (Mozilla) | Production (Firefox), pre-1.0; `uniffi-bindgen-swift`, `cargo swift` | Recommended Rust↔Swift | [uniffi-rs](https://github.com/mozilla/uniffi-rs) |
| swift-bridge | Maintained, lower-level | Only for hot paths | GitHub |

## Recommendation

**Account model:** EIP-7702-delegated EOA whose delegate contract accepts a **P-256 Secure Enclave owner** (WebAuthn-style or raw P-256) verified via `P256VERIFY` (EIP-7951 on L1 / RIP-7212 on L2 / your own chain's precompile). Keep a secp256k1 "compat key" only as an optional recovery/legacy signer. Support ERC-7715 session keys and a sponsoring paymaster on your chain. Design the signer abstraction so the same account can add an EIP-8141 frame path and a PQ signer (7932/8030) later without address change.

**Stack:** Rust core (node, Helios-style sync, proof verifier, `p256`, alloy/reth types) exposed via **UniFFI**; Swift/SwiftUI shell owning **CryptoKit SecureEnclave.P256** keys with biometric policy; simulation-before-sign UX à la Rabby.

**Build now:** 7702 delegate + P-256 owner, SE signing, Helios-style header/state verification, ≤300 KiB proof verifier, session keys, paymaster, Sparkle distribution + org Apple account.

**Wait:** EIP-8141 tx builder (Hegotá ~2027, format still moving; Base's 8130 fork), PQ signers (no finalized scheme), ERC-7683 (add after core ships), MetaMask Snaps (weak adoption).
