# EVM compatibility fixtures

`evm_compat_token.bin.hex` and `evm_compat_nft.bin.hex` are test-only runtimes
compiled from `contracts/test/fixtures/EvmCompat.sol` with Solidity 0.8.19,
Paris target and optimizer 200. There are no constructor arguments or
immutables. They exercise standard caller ABIs against the frozen account
implementation and exact canonical predeploys; they are never genesis code.

The integration recovered these fixtures with the installed Solidity 0.8.19
compiler's standard JSON interface, using source name
`test/fixtures/EvmCompat.sol`, Paris and 200 optimizer runs. The decoded runtime
bytes are pinned below; the SHA-256 values exclude the hex text and newline.

| Fixture | Runtime bytes | Runtime SHA-256 |
| --- | ---: | --- |
| `evm_compat_token.bin.hex` | 3690 | `91cba10e7b82d9900b0728b9df99c2a5792e3e6ba9dba293af2c3958811a0e09` |
| `evm_compat_nft.bin.hex` | 1500 | `8407ac9910e743cb47169e5b15a37c4e7cf8e2b445b2113d7eba8205342e0d0d` |

Regenerate from the repository root, after the compile queue opens:

```sh
compat_root="$(git -C . rev-parse --show-toplevel)"
mkdir -p "$compat_root/tmp/evm-compat"
export TMPDIR="$compat_root/tmp/evm-compat"
export FOUNDRY_OUT="$TMPDIR/forge-out"
export FOUNDRY_CACHE_PATH="$TMPDIR/forge-cache"
~/.claude/playbooks/aether-team/wait-compile.sh
forge inspect --root contracts test/fixtures/EvmCompat.sol:EvmCompatToken deployedBytecode | sed 's/^0x//' > crates/node/tests/fixtures/evm-compat/evm_compat_token.bin.hex
~/.claude/playbooks/aether-team/wait-compile.sh
forge inspect --root contracts test/fixtures/EvmCompat.sol:EvmCompatNft deployedBytecode | sed 's/^0x//' > crates/node/tests/fixtures/evm-compat/evm_compat_nft.bin.hex
```

The ERC-2612 `v/r/s` method retains its secp256k1 semantics. The separate
`bytes` overload models an ERC-1271-aware caller. Plain `ecrecover` callers
cannot acquire P-256 support from an account implementation change.
