//! Shared setup for the native-plan B0-B2 head-to-head measurements: the
//! EastSea-native templates (`fixtures/native`, MIT) against their originals
//! (`fixtures/gpl/merkle-distributor`, GPL-3.0-or-later, and the toolbox
//! examples in the shared manifest) on the real executor.
//!
//! Every comparison uses one `Harness` per path from the same new genesis, the
//! same SeizableToken funding and the same P-256 batch wallet (EIP-7702
//! delegation to the genesis EastSeaAccount), so the only difference between
//! the paths is the contract under test.
use super::harness::*;
use aether_contracts_onchain::{recorder, schema};
use aether_execution::{AccountCall, EvmCall};
use alloy_primitives::hex;
use alloy_sol_types::{sol, SolCall};
use serde_json::Value;
use std::sync::OnceLock;

sol! {
    interface Erc20 {
        function mint(address to, uint256 amount);
        function seize(address from, uint256 amount);
        function approve(address spender, uint256 amount) returns (bool);
        function transfer(address to, uint256 amount) returns (bool);
        function balanceOf(address who) returns (uint256);
    }
}

/// Amount minted to every warm holder before any workflow opens.
pub const FUNDING: u128 = 1_000_000_000_000_000_000_000_000;

fn manifest(cell: &'static OnceLock<Value>, text: &'static str) -> &'static Value {
    cell.get_or_init(|| serde_json::from_str(text).expect("fixture manifest"))
}

/// Creation bytecode of `native/<Name>` or `gpl/<Name>`.
pub fn fixture_code(key: &str) -> Bytes {
    static NATIVE: OnceLock<Value> = OnceLock::new();
    static GPL: OnceLock<Value> = OnceLock::new();
    let m = if key.starts_with("native/") {
        manifest(&NATIVE, include_str!("../../fixtures/native/artifacts.json"))
    } else if key.starts_with("gpl/") {
        manifest(&GPL, include_str!("../../fixtures/gpl/merkle-distributor/artifacts.json"))
    } else {
        return bytecode(key);
    };
    let raw = m["artifacts"][key]["bytecode"].as_str().unwrap_or_else(|| panic!("missing {key}"));
    Bytes::from(hex::decode(raw.trim_start_matches("0x")).unwrap())
}

/// Deploy as a recorded step with its own label (the harness's `deploy*`
/// helpers use one fixed step label). Returns the created address.
pub fn deploy(h: &mut Harness, actor: u8, key: &str, args: Vec<u8>, label: &str) -> Address {
    let mut input = fixture_code(key).to_vec();
    input.extend(args);
    let call = EvmCall { to: None, value: U256::ZERO, input: input.into(), gas_limit: TX_GAS, delegate: None };
    let receipt = h.transact(actor, call, label, true);
    let address = receipt.contract_address.expect("created");
    h.label(address, key);
    address
}

/// Actors 0..=8 already hold the token (warm holders); 10..=15 never do
/// (fresh holders: their first receipt occupies a new balance slot).
pub const WARM_HOLDERS: std::ops::RangeInclusive<u8> = 0..=8;

/// The same token and funding for every path. Nothing here is recorded.
pub fn funded_token(h: &mut Harness) -> Address {
    let token = h.deploy("support/SeizableToken", vec![]);
    for actor in WARM_HOLDERS {
        let to = h.addr(actor);
        h.ok(0, token, Erc20::mintCall { to, amount: U256::from(FUNDING) }.abi_encode(), U256::ZERO, "fund/mint");
    }
    token
}

pub fn approve(token: Address, spender: Address, amount: U256) -> AccountCall {
    (token, U256::ZERO, Erc20::approveCall { spender, amount }.abi_encode().into())
}

pub fn call(to: Address, input: Vec<u8>) -> AccountCall {
    (to, U256::ZERO, input.into())
}

pub fn balance(h: &Harness, token: Address, who: Address) -> U256 {
    U256::from_be_slice(&h.view(0, token, Erc20::balanceOfCall { who }.abi_encode()))
}

pub fn seize(h: &mut Harness, token: Address, from: Address, amount: U256) {
    h.ok(9, token, Erc20::seizeCall { from, amount }.abi_encode(), U256::ZERO, "issuer/seize");
}

/// Close a workflow, validate it against the schema and the unit identity,
/// and print one comparable summary line.
pub fn finish(h: &mut Harness) -> (recorder::WorkflowRecord, Value) {
    let (record, json) = h.end_workflow();
    schema::validate(&json).unwrap_or_else(|e| panic!("schema: {e}"));
    for step in json["steps"].as_array().unwrap() {
        let u = &step["units"];
        let sum = 100 * u["new_accounts"].as_u64().unwrap()
            + 100 * u["new_slots"].as_u64().unwrap()
            + u["code_bytes"].as_u64().unwrap()
            + u["archive_units"].as_u64().unwrap();
        assert_eq!(sum, step["state_units"].as_u64().unwrap(), "{}", step["label"]);
        assert!(step["receipt_evidence"]["proof_verified"].as_bool().unwrap());
    }
    let day = |k: &str, i: usize| {
        let s = &json["b5"][k]["sustained_per_day"][i];
        format!("{}({})", s["workflows_per_day"], s["binding_limit"].as_str().unwrap())
    };
    eprintln!(
        "NATIVE_B {} cold_u={} warm_u={} cold_tx={} warm_tx={} user_sigs={} warm_user_sigs={} failures={} warm_day@10/50/100={}/{}/{}",
        json["workflow"].as_str().unwrap(),
        json["totals"]["cold"]["state_units"],
        json["totals"]["warm"]["state_units"],
        json["totals"]["cold"]["transactions"],
        json["totals"]["warm"]["transactions"],
        json["signatures"]["user_total"],
        json["signatures"]["warm_user_total"],
        json["failures"].as_array().unwrap().len(),
        day("warm", 0),
        day("warm", 1),
        day("warm", 2),
    );
    (record, json)
}

/// Start a workflow in its setup phase (excluded from warm totals) and
/// delegate every listed user's P-256 account to the genesis EastSeaAccount
/// (identical on every path). The caller switches to `h.phase("action")`
/// after any shared deployment.
pub fn begin(h: &mut Harness, name: &str, users: &[u8], delegate: &[u8]) {
    h.begin_workflow(name, users);
    h.note("same new genesis, token funding and P-256 batch wallet on both paths");
    h.phase(recorder::SETUP_PHASE);
    for actor in delegate {
        h.delegate_account(*actor);
    }
}
