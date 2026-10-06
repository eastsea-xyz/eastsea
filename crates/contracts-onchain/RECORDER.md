# Workflow recorder (native plan A0)

`aether_contracts_onchain` measures a **user workflow** (every transaction a
user task needs) on the real EastSea executor. No node, app or RPC server runs.
Every included transaction is replayed through `build_block`,
`execute_block` and `execute_block_sequential`, and is measured from the
executor's own outcome.

A Foundry gas number is not an EastSea cost. Publish these records instead.

## Measure a Foundry-built contract

Add a test module under `tests/contracts_onchain/`, register it in
`tests/contracts_onchain.rs`, then:

```rust
use aether_contracts_onchain::{harness::*, recorder, schema};

#[test]
fn my_contract_workflow() {
    let mut h = Harness::new();                          // new-genesis B5 context, actors 0..15 funded
    h.begin_workflow("my-lane/claim", &[1]);             // actor 1 is the user; other senders are relayers
    h.phase(recorder::SETUP_PHASE);                      // excluded from warm totals
    let code = foundry_creation_code("path/to/out/Claim.sol/Claim.json");
    let claim = h.deploy_creation(0, "my-lane/Claim", code, constructor_args);
    h.phase("action");
    h.ok(1, claim, claimCall { .. }.abi_encode(), U256::ZERO, "claim");
    h.revert(1, claim, claimCall { .. }.abi_encode(), U256::ZERO, "claim/replay");
    let sig = h.account_signature(1, h.addr(1), digest, "claim/typed"); // counts a typed signature
    let (record, json) = h.end_workflow();               // also appended to $CONTRACTS_ONCHAIN_WORKFLOWS
    schema::validate(&json).unwrap();
    assert_eq!(record.step("claim").new_slots, 1);
}
```

Fixture contracts compiled by `scripts/generate-contract-fixtures.py` deploy
with `h.deploy("support/Name", args)`. A test that only reads bytecode from a
Foundry artifact needs no fixture change.

## API

| Call | Effect |
|---|---|
| `Harness::new()` | New-genesis paid-state context: 100,000 u bucket, 32 u/height refill, floor fee vector (exec 0, state 10^12 wei/u, prove 0). |
| `begin_workflow(name, users)` / `end_workflow()` | Open/close a record. Returns `(WorkflowRecord, serde_json::Value)`. |
| `phase(name)`, `note(text)` | Label following steps; `"setup"` is excluded from warm totals. |
| `ok`, `revert`, `transact`, `deploy`, `deploy_creation` | Sign with P-256, include, replay and record. A wrong success/revert expectation panics. |
| `typed_signature(actor, label)` | Count an off-chain signature not made by a helper below. |
| `delegate_account(actor)` | EIP-7702 delegation to the genesis `EastSeaAccount`. |
| `self_batch(actor, calls, label, expect)` | `execute(Call[])`: one owner transaction signature, atomic. |
| `add_owner(account_actor, owner_actor, label)` | Add a device key through a self batch; returns its index. |
| `owner_relay(relayer, account, owner, key, calls, label, expect)` / `owner_relay_input(..)` | `ownerExecute` signed off-chain by an added owner (one typed signature), paid by the relayer. |
| `account_signature(actor, account, hash, label)` | ERC-1271 blob `r‖s‖x‖y` over the account's EIP-712 `signatureMessage(hash)` (one typed signature). |
| `foundry_creation_code(path)` | Creation bytecode from a Foundry `out/` artifact. |
| `schema::validate(&json)` | Strict check against the schema; undeclared fields fail. |

## What a record contains

Per transaction (`steps[]`): exec and prove gas; state units split into
`new_accounts`, `new_slots`, `code_bytes` and `archive_units`; signed-envelope,
metered receipt, canonical receipt and BAL bytes; events and topics; signed
state budget; fee paid, at floor and at state debts 62,500 / 75,000 / 99,999;
receipt root with a verified Merkle proof; created contracts and persisted code
hashes (an EIP-7702 designator also names its delegate and the delegate's code
hash).

The split is not estimated. It is derived from the pre/post state over every
BAL address and asserted to equal the executor's state units:
`U = 100*accounts + 100*slots + code_bytes + ceil((envelope + receipt)/32)`.
A missed account or slot fails the test.

Per workflow: user transaction signatures, typed-message signatures and relayer
transactions; per-phase, cold and warm totals; every failure with its payer and
fee; and the B5 binding limit. That limit is a full-bucket burst with recovery
heights, plus per-day ceilings at 10/50/100% of each shared per-height capacity:
state refill, encoded payload refill (envelope bytes only, an upper bound),
exec, prove, new slots, persisted bytes and the 2,000-transaction cap. The
smallest ceiling is named as binding. These are upper bounds, not throughput.

`environment` pins the chain id, fee vector, block limits, the account
runtime's code hash and the fixture manifest's SHA-256.

## Schema

`crates/contracts-onchain/schema/workflow-record.v1.schema.json`
(`eastsea.workflow-record/v1`). This file is canonical until it is copied to
`eastsea-toolbox/native/`. A schema change needs agreement from the lanes that
consume it (PLAN Lane A) and must update `src/schema.rs` tests.

## Brake interface

`fixtures/support/src/brake/IEastSeaLocalBrake.sol` defines the deterministic
local entry brake. It has no guardian, a predicate that reads only chain state,
and a permissionless, nonreverting, monotonic `tripBrake()`. Entries re-check
the predicate and exits never check the brake. `LocalBrake.sol` is the abstract
base. `BrakeReferenceVault.sol` is the reference contract, and `brakeSpec()`
returns the SHA-256 of its source. `tests/contracts_onchain/brake.rs` checks
the latch.

## Not covered yet

Live-chain inclusion/finality latency, node payload framing (covered by
`aether-node` `state_budget`), cold first use from zero balance (plan step 5),
wallet disappearance and history export.
