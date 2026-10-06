# Real-executor Solidity fixtures

`artifacts.json` maps `core/<Contract>`, `toolbox/<Contract>`, and
`support/<Contract>` to creation bytecode, compiler runtime bytecode, and ABI.
The Rust integration tests consume this checked-in manifest; running them does
not require Foundry, the toolbox checkout, or network access.

Regenerate from the repository root, using the installed headless Foundry CLI:

```sh
python3 scripts/generate-contract-fixtures.py --offline
```

The generator finds `forge` on PATH, falling back to `~/.foundry/bin/forge`.
`--forge /absolute/path/to/forge` overrides that choice. Remove `--offline` only
if a pinned compiler has not yet been installed. Builds run sequentially with
four Foundry threads, use the default profile, and set `TMPDIR` to the worktree's
`tmp/`. Foundry `out/` and `cache/` stay inside the repository and are ignored by
git. No app, node, cargo build, or external source checkout is launched.

The core build reads the actual `contracts/src` and existing `contracts/base`
imports with its existing solc 0.8.19 optimizer profile. Toolbox and support
fixtures pin solc 0.8.24, the Paris EVM target, and optimizer runs 200. The toolbox
source snapshot, with the explicitly listed audited local fixes below, includes all 17 examples, their system-contract dependencies, AMM
components and factories. Only recursively imported OpenZeppelin files are
vendored; the MIT license accompanies them. `provenance.json` records the source
revision, compiler profiles, every compiler input's SHA-256, and the manifest's
SHA-256. Source digests identify the exact inputs even if the external snapshot
had local changes. Regeneration never reads or writes the toolbox checkout.

Every concrete contract declared under each project's `src/` is included,
including nested factory-created contracts. Interfaces, abstract contracts,
and standalone library placeholders are excluded. Duplicate contract names
within one namespace and unresolved bytecode links make generation fail.

Audited local fixes applied to the copied toolbox sources:

- `Editions1155`: reject price and per-wallet cap values outside their packed
  `uint128` and `uint32` fields before narrowing.
- `SubscriptionManager`: reject expiry and contributed-principal growth outside
  their packed `uint64` and `uint88` fields before narrowing.
- `SimpleMultisig`: accept native funding with a payable receive function so
  signed native payouts can use normally deposited funds.
- `FixedPriceMarket`: ignore a royalty quoted with a zero receiver so seller
  proceeds do not become unreachable credits.
- `TokenTimeLock`: allow a new grant after the previous grant is fully withdrawn;
  keep completed history until replacement and reject active/unwithdrawn grants.

The core source also fixes `TokenVesting` intermediate multiplication overflow;
its creation bytecode comes directly from the corrected `contracts/src` source.
The toolbox checkout is unchanged. Source digests in `provenance.json` record the
corrected local versions. The local Foundry regressions require no forge-std:

```sh
mkdir -p tmp
TMPDIR="$PWD/tmp" ~/.foundry/bin/forge test \
  --root crates/contracts-onchain/fixtures/toolbox --threads 4 --offline
```


`runtime` is the compiler template, which may contain immutable-value
placeholders. Tests must deploy creation bytecode through the executor and use
the resulting state; substituting this template for constructor execution would
miss paid deployment state and immutable constructor values.

`support/TestToken` is a deliberately unrestricted harness token. It supports
minting, configurable transfer fees, transfers returning false, and one guarded
callback per transfer. Callback failure is recorded without reverting the outer
transfer. `setCallback(address,bytes)` enables a nonzero target;
`configureCallback(address,bytes,bool)` enables or disables it explicitly.
`callbackAttempted()`, `innerSuccess()`, and `innerOutput()` expose the attempt's
outcome and exact return/revert bytes. The
callback's caller is the token contract.

`support/NativeCallback` forwards arbitrary calls with
`execute(address,bytes,uint256)` and records configured native-payout reentry.
`executeOrRevert(address,bytes,uint256)` bubbles failed inner-call revert bytes;
`execute` instead returns the inner call's status and output.
`configure(address,bytes)` sets the reentry call; `setRejectPayment(bool)` makes
native payouts revert. It also accepts ERC-721 and ERC-1155 safe transfers. `setNFTCallback(bool)` enables the same callback on ERC-721 receipt;
`setRejectNFT(bool)` rejects that receipt. Both flags default to false.

`support/MarketNFT` supplies minting, transfer rejection, and configurable royalty
interface behavior: mode 0 omits ERC-2981 support; mode 1 quotes the configured
receiver/amount; mode 2 rejects interface detection; mode 3 rejects royaltyInfo.
It shares the already-vendored OpenZeppelin ERC-721 dependency. These support
contracts are test instruments, not production-contract candidates.
