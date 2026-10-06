# EastSea-native B0-B2 templates (measurement fixture)

Unmodified copies of the public eastsea-toolbox `native/` sources at `main`
`a7e8724b88aab9da64ae269dce9716d07238c872`:

| Here | Toolbox path |
|---|---|
| `src/claims/src/ClaimCampaigns.sol` | `native/claims/src/ClaimCampaigns.sol` |
| `src/streams/src/GrantLedger.sol` | `native/streams/src/GrantLedger.sol` |
| `src/escrow/src/EscrowBook.sol` | `native/escrow/src/EscrowBook.sol` |
| `src/common/src/*.sol` | `native/common/src/*.sol` |

Licence: MIT (SPDX header in every file). Compiler profile matches
`native/foundry.toml`: solc 0.8.31, Osaka, optimizer 200. Runtime sizes match
the toolbox GAS.md figures (7,767 / 6,681 / 10,206 B).

`artifacts.json` (creation code, runtime, ABI and input SHA-256s) is written by
`python3 scripts/generate-native-fixtures.py --offline`. The executor tests
`tests/contracts_onchain/native_b_*.rs` read it; Foundry is not needed to run them.
