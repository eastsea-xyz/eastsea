//! Deterministic local entry brake (native plan step 3): the reference vault's
//! latch halts entry, keeps exits open, cannot be set by a caller while the
//! predicate is false, never reverts, never unlatches and has no guardian.
use super::harness::*;
use aether_contracts_onchain::schema;
use alloy_sol_types::{sol, SolCall, SolError, SolValue};
use sha2::{Digest, Sha256};

sol! {
    interface Vault {
        function deposit(uint256 amount);
        function withdraw(uint256 amount);
        function liabilities() returns (uint256);
        function brakeState() returns (uint8 state, address guardian, uint64 since);
        function brakeSpec() returns (string uri, bytes32 docSha256);
        function brakePredicate() returns (bool holds, bytes32 reason);
        function tripBrake() returns (bool latched);
        error EntryBraked(bytes32 reason);
        event BrakeLatched(bytes32 indexed reason, uint64 since);
    }
    interface Seizable {
        function mint(address to, uint256 amount);
        function seize(address from, uint256 amount);
        function approve(address spender, uint256 amount) returns (bool);
        function balanceOf(address who) returns (uint256);
    }
}

const SPEC: &[u8] = include_bytes!("../../fixtures/support/src/brake/BrakeReferenceVault.sol");

fn tag(s: &str) -> B256 {
    let mut b = [0u8; 32];
    b[..s.len()].copy_from_slice(s.as_bytes());
    B256::from(b)
}

fn state(h: &Harness, vault: Address) -> (u8, Address, u64) {
    let r = Vault::brakeStateCall::abi_decode_returns(&h.view(9, vault, Vault::brakeStateCall {}.abi_encode())).unwrap();
    (r.state, r.guardian, r.since)
}

fn predicate(h: &Harness, vault: Address) -> (bool, B256) {
    let r = Vault::brakePredicateCall::abi_decode_returns(&h.view(9, vault, Vault::brakePredicateCall {}.abi_encode())).unwrap();
    (r.holds, r.reason)
}

fn amount(n: u64) -> U256 {
    U256::from(n)
}

#[test]
fn reference_brake_latch_halts_entry_and_keeps_exits_open() {
    let mut h = Harness::new();
    let token = h.deploy("support/SeizableToken", vec![]);
    let spec_hash = B256::from_slice(&Sha256::digest(SPEC));
    let vault = h.deploy("support/BrakeReferenceVault", (token, spec_hash).abi_encode_params());
    for user in [1u8, 2] {
        let who = h.addr(user);
        h.ok(0, token, Seizable::mintCall { to: who, amount: amount(1_000) }.abi_encode(), U256::ZERO, "SeizableToken/mint");
        h.ok(user, token, Seizable::approveCall { spender: vault, amount: U256::MAX }.abi_encode(), U256::ZERO, "SeizableToken/approve");
    }

    // Declared spec and no guardian.
    let spec = Vault::brakeSpecCall::abi_decode_returns(&h.view(9, vault, Vault::brakeSpecCall {}.abi_encode())).unwrap();
    assert_eq!(spec.docSha256, spec_hash, "the per-item spec document is pinned by hash");
    assert!(spec.uri.ends_with("fixtures/support/src/brake/BrakeReferenceVault.sol"));
    assert_eq!(state(&h, vault), (0, Address::ZERO, 0));

    let (anyone, issuer) = (4u8, 5u8);
    let deposit = |n| Vault::depositCall { amount: amount(n) }.abi_encode();
    let withdraw = |n| Vault::withdrawCall { amount: amount(n) }.abi_encode();
    let trip = Vault::tripBrakeCall {}.abi_encode();
    h.begin_workflow("brake/reference-vault-latch", &[1, 2]);
    h.note("anyone (actor 4) trips the brake; actor 5 plays the token issuer");
    h.ok(1, vault, deposit(100), U256::ZERO, "vault/deposit-1");
    h.ok(2, vault, deposit(50), U256::ZERO, "vault/deposit-2");

    // While the predicate is false a caller cannot pause: success, no change.
    assert_eq!(predicate(&h, vault), (false, B256::ZERO));
    let r = h.ok(anyone, vault, trip.clone(), U256::ZERO, "brake/trip-while-predicate-false");
    assert!(!Vault::tripBrakeCall::abi_decode_returns(&r.output).unwrap());
    assert!(r.events.is_empty());
    assert_eq!(state(&h, vault).0, 0);

    // Issuer seizure creates a backing deficit; entry re-checks the predicate
    // and refuses even before anyone latches.
    h.ok(issuer, token, Seizable::seizeCall { from: vault, amount: amount(60) }.abi_encode(), U256::ZERO, "issuer/seize");
    assert_eq!(predicate(&h, vault), (true, tag("DEFICIT")));
    let r = h.revert(1, vault, deposit(10), U256::ZERO, "vault/deposit-refused-by-predicate");
    assert_eq!(Vault::EntryBraked::abi_decode(&r.output).unwrap().reason, tag("DEFICIT"));

    // Anyone latches; the call never reverts; the latch is monotonic.
    let r = h.ok(anyone, vault, trip.clone(), U256::ZERO, "brake/trip-latches");
    assert!(Vault::tripBrakeCall::abi_decode_returns(&r.output).unwrap());
    assert_eq!(r.events.len(), 1);
    assert_eq!(r.events[0].topics[1], tag("DEFICIT"));
    let latched_at = h.timestamp() - 1;
    assert_eq!(state(&h, vault), (1, Address::ZERO, latched_at));
    let r = h.ok(anyone, vault, trip.clone(), U256::ZERO, "brake/trip-again-noop");
    assert!(Vault::tripBrakeCall::abi_decode_returns(&r.output).unwrap() && r.events.is_empty());

    // Exits stay open, subject to what the vault actually holds.
    h.ok(2, vault, withdraw(50), U256::ZERO, "vault/withdraw-2-after-latch");
    h.revert(1, vault, withdraw(100), U256::ZERO, "vault/withdraw-1-beyond-cash");
    h.ok(1, vault, withdraw(40), U256::ZERO, "vault/withdraw-1-partial");

    // Backing restored: predicate false, latch still holds, entry still closed.
    h.ok(issuer, token, Seizable::mintCall { to: vault, amount: amount(60) }.abi_encode(), U256::ZERO, "issuer/restore-backing");
    assert_eq!(predicate(&h, vault), (false, B256::ZERO));
    assert_eq!(state(&h, vault), (1, Address::ZERO, latched_at));
    let r = h.revert(1, vault, deposit(10), U256::ZERO, "vault/deposit-refused-after-latch");
    assert_eq!(Vault::EntryBraked::abi_decode(&r.output).unwrap().reason, tag("LATCHED"));
    h.ok(1, vault, withdraw(60), U256::ZERO, "vault/withdraw-1-rest");
    let liabilities = Vault::liabilitiesCall::abi_decode_returns(&h.view(9, vault, Vault::liabilitiesCall {}.abi_encode())).unwrap();
    assert_eq!(liabilities, U256::ZERO);
    let (record, json) = h.end_workflow();
    schema::validate(&json).unwrap();

    let failures: Vec<_> = record.failures().map(|s| s.label.as_str()).collect();
    assert_eq!(
        failures,
        ["vault/deposit-refused-by-predicate", "vault/withdraw-1-beyond-cash", "vault/deposit-refused-after-latch"]
    );
    assert!(record.failures().all(|s| s.fee_paid_wei > 0 && s.new_slots == 0));
    let latch = record.step("brake/trip-latches");
    assert_eq!(latch.new_slots, 1, "the latch occupies the brake slot once");
    assert_eq!(record.step("brake/trip-again-noop").new_slots, 0);
    assert_eq!(record.step("brake/trip-while-predicate-false").new_slots, 0);
}

#[test]
fn reference_brake_has_no_privileged_path() {
    let abi = artifacts()["support/BrakeReferenceVault"]["abi"].as_array().unwrap();
    let mut mutating: Vec<&str> = abi
        .iter()
        .filter(|e| e["type"] == "function" && e["stateMutability"] != "view" && e["stateMutability"] != "pure")
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    mutating.sort();
    assert_eq!(mutating, ["deposit", "tripBrake", "withdraw"], "no pause, unlatch, rescue or admin function");
    assert!(!abi.iter().any(|e| e["type"] == "fallback" || e["type"] == "receive"));
    let ctor = abi.iter().find(|e| e["type"] == "constructor").unwrap();
    let inputs: Vec<&str> = ctor["inputs"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert_eq!(inputs, ["token_", "specSha256"], "no guardian or founder key at construction");
}
