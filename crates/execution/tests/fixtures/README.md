# EVM compatibility fixtures

`evm_compat_token.bin.hex` and `evm_compat_nft.bin.hex` are test-only runtimes
compiled from `contracts/test/fixtures/EvmCompat.sol` with Solidity 0.8.19,
Paris target and optimizer 200. There are no constructor arguments or
immutables. They exercise standard caller ABIs against the frozen account
implementation and exact canonical predeploys; they are never genesis code.

Regenerate from the repository root, after the compile queue opens:

```sh
compat_root="$(git -C . rev-parse --show-toplevel)"
mkdir -p "$compat_root/tmp/evm-compat"
export TMPDIR="$compat_root/tmp/evm-compat"
export FOUNDRY_OUT="$TMPDIR/forge-out"
export FOUNDRY_CACHE_PATH="$TMPDIR/forge-cache"
~/.claude/playbooks/aether-team/wait-compile.sh
forge inspect --root contracts test/fixtures/EvmCompat.sol:EvmCompatToken deployedBytecode | sed 's/^0x//' > crates/execution/tests/fixtures/evm_compat_token.bin.hex
~/.claude/playbooks/aether-team/wait-compile.sh
forge inspect --root contracts test/fixtures/EvmCompat.sol:EvmCompatNft deployedBytecode | sed 's/^0x//' > crates/execution/tests/fixtures/evm_compat_nft.bin.hex
```

The ERC-2612 `v/r/s` method retains its secp256k1 semantics. The separate
`bytes` overload models an ERC-1271-aware caller. Plain `ecrecover` callers
cannot acquire P-256 support from an account implementation change.
