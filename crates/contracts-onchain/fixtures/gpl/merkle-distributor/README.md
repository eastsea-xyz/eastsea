# Uniswap MerkleDistributor (GPL-3.0-or-later, test fixture only)

The B0 claims comparison original. **This folder is licensed GPL-3.0-or-later**
(see `LICENSE`), separately from the rest of this repository. It is used only
by the executor measurement tests; nothing here is linked into, shipped with
or deployed by any EastSea binary.

- `src/MerkleDistributor.sol`, `src/interfaces/IMerkleDistributor.sol`:
  unmodified from `Uniswap/merkle-distributor` at
  `25a79e8ec8c22076a735b1a675b961c8184e7931` (Uniswap Labs, GPL-3.0-or-later).
- `lib/openzeppelin-contracts`: the files those import, unmodified from
  OpenZeppelin Contracts `v4.7.0` (MIT, `lib/openzeppelin-contracts/LICENSE`).
- Compiler: the upstream `hardhat.config.ts` settings, solc 0.8.17, optimizer
  5,000 runs, default EVM target.

`artifacts.json` (bytecode compiled from these sources, also GPL) is written by
`python3 scripts/generate-native-fixtures.py --offline`.
