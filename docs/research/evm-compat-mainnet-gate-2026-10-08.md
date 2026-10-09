# EVM compatibility: B1–B3 mainnet gate

Branch: `codex/evm-compat`, from `lead-merge`.

Status: implementation and static review complete; execution gates are queued
behind the release hold. This report will record the actual gate results.

## Account compatibility (B1, B2)

The toolbox's 2026-10-06 H3/H4/H11 report predates the inherited fixes:
`ff6b31a` added ERC-1271 and token receivers to the new-genesis v2 runtime;
`bbc6ff6` added CREATE2 and Multicall3. This lane preserves those fixes and
adds end-to-end coverage; it does not introduce another account implementation.

The v2 account remains at `0x0000000000000000000000000000000000007702`,
with runtime hash
`0xdeaca4e6cc9787c233aeec5034a8884cf5ced4c6899299b3e76027a03f85288a`.
Its Solidity source, runtime bytes and storage layout are unchanged. The legacy
7780 runtime remains pinned to
`0x0b9ba7215eb4aee404f884d94f74c5d5279cdfa5dc7cb85b028c045cf17a9433`.

ERC-1271 checks low-s P-256 signatures via EIP-7951 P256VERIFY at `0x100`.
Signatures are `r || s || x || y`, over SHA-256 of the account's 66-byte typed
message, wrapping the verifier's hash in the account/chain domain. Original
and registered owner keys can sign; session keys and guardians cannot.
The pinned implementation already implements the ERC-721/1155 receiver
hooks and ERC-165.

Both new suites use actual 23-byte EIP-7702 delegation designators rather
than substituting account code for the delegation. They reconstruct signature
domains independently and verify real signatures without precompile mocks.
The OpenZeppelin receiver suite additionally exercises its unchanged `_safeMint`.

[ERC-2612](https://eips.ethereum.org/EIPS/eip-2612) specifies secp256k1 `v/r/s`.
An ERC-1271-aware permit fallback works with the account's P-256 signatures;
a token using only `ecrecover` still needs ordinary `approve` or Permit2.
The fallback token in these tests is a test fixture, not a new genesis token.
[Permit2 SignatureTransfer](https://developers.uniswap.org/docs/protocols/permit2/concepts/signature-transfer)
supports ERC-1271 without modifying Permit2. The account owner first approves
Permit2 on the token, then signs its spender/nonce/deadline-bound permit through
the account's signature wrapper.

The future v3 re-delegation contract is specified in
[09-wallet.md](../design/09-wallet.md): pin and verify the new CREATE2 address
and code hash, self-sign `EvmCall.delegate`, await finality, then check the
on-chain designator and nonce. An included transaction can retain delegation
even when its calls revert. No wallet UI or automatic migration is added here.

## Explicit genesis changes (B3)

The host node's `predeploys::all()` catalogue includes Permit2. Genesis retains
the released CREATE2/Multicall3 allocation, and adds Permit2 only when the frozen
genesis protocol is at least 4, with node rewards and history v2 enabled.
`mainnet::check` requires all three canonical code hashes for the current
new-network checklist. Protocol-3 cold participants retain their original
state root; running a newer implementation or activating protocol 4 later does
not retroactively install Permit2 or rewrite genesis.

| Contract | Genesis address | Runtime bytes | keccak256(runtime) |
|---|---|---:|---|
| Arachnid CREATE2 deployer | `0x4e59b44847b379578588920cA78FbF26c0B4956C` | 69 | `0x2fa86add0aed31f33a762c9d88e807c475bd51d0f52bd0955754b2608f7e4989` |
| Multicall3 | `0xcA11bde05977b3631167028862bE2a173976CA11` | 3,808 | `0xd5c15df687b16f2ff992fc8d767b4216323184a2bbc6ee2f9c398c318e770891` |
| Permit2 (added) | `0x000000000022D473030F116dDEE9F6B43aC78BA3` | 9,152 | `0xc67d1657868aa5146eaf24fb879fb1fdec3d2d493b3683a61c9c2f4fb2851131` |

Permit2 and Multicall3 are byte-identical to the Ethereum mainnet code in
`/Volumes/workspace/eastsea-toolbox/proof/mainnet-code/1-<address>.hex`.
There is no CREATE2 cache file there; its existing canonical bytes/hash pin
is retained. Permit2 includes the exact mainnet immutables, including the
chain-1 domain cache. Its original runtime recomputes the domain for the
EastSea chain id. The three contracts start with empty storage.

The Permit2 deployment condition is **`node_rewards && history_v2 && cfg.protocol >= 4`**.
The base two-contract catalogue keeps its original rewards/history condition. Mainnet
and fresh devnet/testnet genesis files use this same path, documented in
[mainnet-launch.md](../ops/mainnet-launch.md). Prospective protocol-4 networks
generate their ceremony records from that explicit new genesis. Existing
protocol-3 network files and records retain their original root, including
after a later protocol-4 activation. Existing 7780 gets none of these contracts: there is
no scheduled standard-predeploy system write in this release. Its shipped
network file, original genesis/account code and upgrade rules are unchanged.

EntryPoint is deferred. EastSeaAccount has no `validateUserOp`, there is no
bundler/paymaster integration in this scope, and no EntryPoint version/code
hash has been selected. B1–B3's contract flows do not require it. Adding it
later requires an explicit version/hash and an end-to-end UserOperation gate.

## Regression evidence and gates

The historical before-fix controls intentionally use the legacy runtime or
empty predeploy addresses. They do not claim that B1/B2 were absent in this
lane's starting v2 implementation.

| Requirement | Positive check | Before-fix control |
|---|---|---|
| ERC-1271 P-256 | Owner signature returns `0x1626ba7e`; wrong key/hash/malformed signatures fail | Legacy runtime cannot validate the same signature |
| Safe mint/transfer | Pinned v2 delegation accepts NFT; OpenZeppelin `_safeMint` invokes its hook | Legacy runtime reverts mint/transfer and leaves ownership unchanged |
| Canonical predeploy | Three hash/length pins, real new-genesis allocation; missing/corrupt Permit2 fails the gate | Old CREATE2/Multicall3-only allocation is insufficient |
| Multicall3 | `aggregate` returns token balance and account ERC-165 result with block number | Empty Multicall3 address gives no decodable result |
| Permit2 | Canonical `permitTransferFrom` moves tokens for a P-256 ERC-1271 owner | Legacy account rejects it; missing Permit2 moves nothing |
| Permit authorization | Wrong signer/hash/spender/chain rejected; failed nonce changes roll back; replay rejected | Same transfer cannot authorize twice |
| Permit fallback | ERC-1271-aware bytes permit sets allowance once; secp256k1 `v/r/s` remains usable | Legacy account rejects the fallback |

Verification commands, serialized through `wait-compile.sh`:

- `cargo test -j4 -p aether-node --tests` — pending.
- `cargo test -p aether-execution` — pending, including six new compatibility tests.
- `forge test --root contracts` — pending, including 14 new frozen-runtime tests.
- `forge test --root crates/contracts-onchain/fixtures/toolbox --match-contract AccountReceiverTest` — pending.
- Static review, `git diff --check`, and formatting checks for the new tests — passed.
- Canonical byte comparisons and independent `cast keccak` hashes — passed.
- Account v2/v1 source/artifact, `forks.rs` and 7780 network-file preservation diff — passed.

All build and temporary outputs are isolated under this worktree's `tmp/`.
No installed app, validator, real wallet data or running testnet was accessed.

## Changed files and remaining limits

- Genesis: `crates/execution/src/predeploys.rs`, `permit2.bin.hex`,
  `crates/node/src/mainnet.rs`, and the explanatory comment in `chain.rs`.
- Tests: `contracts/test/EastSeaAccountCompat.t.sol`,
  `contracts/test/fixtures/EvmCompat.sol`, read permissions in `contracts/foundry.toml`,
  `crates/execution/tests/evm_compat.rs`, its token/NFT runtime fixtures and
  regeneration README, and the OpenZeppelin `AccountReceiver.t.sol` safe-mint test.
- Documentation: `docs/ops/mainnet-launch.md`, `docs/design/09-wallet.md`, this report.

The implementation reuses the existing genesis catalog and frozen account;
no dependencies, account storage, protocol upgrade or production token are added.
Mainnet ceremony-root regeneration, future wallet signing/re-delegation UI,
plain ecrecover-only permits, existing 7780 compatibility and EIP-4337
integration remain the explicit limits above.
