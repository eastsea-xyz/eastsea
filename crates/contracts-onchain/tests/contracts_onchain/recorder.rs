//! A0 recorder: real P-256 self batches, added-owner relays and an ERC-1271
//! consumer on the EastSeaAccount runtime, each recorded as a user workflow
//! whose JSON is checked against the schema.
use super::harness::*;
use aether_contracts_onchain::{recorder, schema};
use aether_execution::EvmCall;
use alloy_primitives::keccak256;
use alloy_sol_types::{sol, SolCall};
use serde_json::Value;

sol! {
    interface Token {
        function mint(address to, uint256 amount);
        function transfer(address to, uint256 amount) returns (bool);
        function balanceOf(address who) returns (uint256);
    }
    interface IntentBook {
        function digest(bytes32 payload) returns (bytes32);
        function accept(address signer, bytes32 payload, bytes signature);
        function accepted(address signer, bytes32 payload) returns (bool);
    }
}

fn wei(v: &Value) -> u128 {
    v.as_str().unwrap().parse().unwrap()
}

fn mint(h: &mut Harness, token: Address, to: Address, amount: u64) {
    h.ok(0, token, Token::mintCall { to, amount: U256::from(amount) }.abi_encode(), U256::ZERO, "SeizableToken/mint");
}

fn transfer(token: Address, to: Address, amount: U256) -> (Address, U256, Bytes) {
    (token, U256::ZERO, Token::transferCall { to, amount }.abi_encode().into())
}

/// Cross-checks every record must satisfy, independent of the workflow.
fn check_record(json: &Value) {
    schema::validate(json).unwrap_or_else(|e| panic!("schema: {e}"));
    for step in json["steps"].as_array().unwrap() {
        let units = &step["units"];
        let total = 100 * units["new_accounts"].as_u64().unwrap()
            + 100 * units["new_slots"].as_u64().unwrap()
            + units["code_bytes"].as_u64().unwrap()
            + units["archive_units"].as_u64().unwrap();
        assert_eq!(total, step["state_units"].as_u64().unwrap(), "{}", step["label"]);
        let bytes = &step["bytes"];
        assert_eq!(
            units["archive_units"].as_u64().unwrap(),
            (bytes["envelope"].as_u64().unwrap() + bytes["receipt_metered"].as_u64().unwrap()).div_ceil(32)
        );
        assert!(step["receipt_evidence"]["proof_verified"].as_bool().unwrap());
        assert_eq!(wei(&step["fee"]["floor_wei"]), u128::from(step["state_units"].as_u64().unwrap()) * 1_000_000_000_000);
        let congestion: Vec<u128> = step["fee"]["congestion_wei"].as_array().unwrap().iter().map(|c| wei(&c["fee_wei"])).collect();
        let floor = wei(&step["fee"]["floor_wei"]);
        // fake_exponential at debt 62,500 / 75,000 / 99,999: ~e^1, e^2, e^4 times the floor.
        assert!(congestion[0] >= floor * 271 / 100 && congestion[1] >= floor * 738 / 100 && congestion[2] >= floor * 54);
    }
    for share in json["b5"]["cold"]["sustained_per_day"].as_array().unwrap() {
        let ceilings = share["ceilings"].as_object().unwrap();
        let min = ceilings.values().map(|v| v.as_u64().unwrap()).min().unwrap();
        assert_eq!(share["workflows_per_day"].as_u64().unwrap(), min);
        assert_eq!(ceilings[share["binding_limit"].as_str().unwrap()].as_u64().unwrap(), min);
        let units = json["totals"]["cold"]["state_units"].as_u64().unwrap();
        let pct = share["refill_share_percent"].as_u64().unwrap();
        assert_eq!(ceilings["state_refill"].as_u64().unwrap(), 32 * 86_400 * pct / 100 / units);
    }
}

#[test]
fn p256_self_batch_is_one_signature_and_its_costs_are_exact() {
    let mut h = Harness::new();
    let token = h.deploy("support/SeizableToken", vec![]);
    let user = h.addr(0);
    mint(&mut h, token, user, 1_000);
    let fresh = Address::repeat_byte(0x51);
    let existing = h.addr(1);
    let new_holder = h.addr(2);

    h.begin_workflow("p256-self-batch/native-new+native-existing+erc20-new-holder", &[0]);
    h.note("same three payments as p256-separate-transactions; only the account batch differs");
    h.phase(recorder::SETUP_PHASE);
    let account = h.delegate_account(0);
    assert_eq!(account, user);
    h.phase("action");
    let calls = vec![
        (fresh, U256::from(1), Bytes::new()),
        (existing, U256::from(5), Bytes::new()),
        transfer(token, new_holder, U256::from(10)),
    ];
    h.self_batch(0, &calls, "self-batch/pay-three", true);
    // A failing last call rolls the whole batch back; the user still pays.
    let bad = vec![(existing, U256::from(1), Bytes::new()), transfer(token, new_holder, U256::from(10u64.pow(18)))];
    let before = h.state.balance(&existing);
    h.self_batch(0, &bad, "self-batch/atomic-revert", false);
    assert_eq!(h.state.balance(&existing), before, "reverted batch paid nobody");
    let (batched, json) = h.end_workflow();
    check_record(&json);

    let setup = batched.step("EastSeaAccount/delegate-p256-account");
    assert_eq!(setup.code_bytes, 23, "EIP-7702 designator persisted");
    let change = &setup.code_changes[0];
    assert_eq!(change.address, account);
    assert_eq!(
        change.delegate,
        Some((aether_execution::AETHER_ACCOUNT, keccak256(aether_execution::aether_account_code_v2())))
    );
    let pay = batched.step("self-batch/pay-three");
    assert!(pay.success && pay.by_user);
    assert_eq!((pay.new_accounts, pay.new_slots, pay.code_bytes), (1, 1, 0), "fresh account + new token holder");
    let revert = batched.step("self-batch/atomic-revert");
    assert_eq!((revert.new_accounts, revert.new_slots, revert.code_bytes), (0, 0, 0));
    assert_eq!(revert.state_units, revert.archive_units, "a revert pays archive units only");
    assert!(revert.fee_paid_wei > 0);
    assert_eq!(json["failures"].as_array().unwrap().len(), 1);
    assert_eq!(json["failures"][0]["payer_role"], "user");
    assert_eq!(json["signatures"]["user_transaction"], 3);
    assert_eq!(json["signatures"]["user_typed_message"], 0);
    assert_eq!(json["signatures"]["warm_user_total"], 2);
    assert_eq!(json["totals"]["warm"]["transactions"], 2);
    assert_eq!(
        json["environment"]["account_runtime_code_hash"].as_str().unwrap(),
        keccak256(aether_execution::aether_account_code_v2()).to_string()
    );

    // Control: the same payments as three plain transactions from the same account.
    let fresh2 = Address::repeat_byte(0x52);
    let holder2 = h.addr(3);
    h.begin_workflow("p256-separate-transactions/native-new+native-existing+erc20-new-holder", &[0]);
    let native = |to: Address, value: u64| EvmCall {
        to: Some(to),
        value: U256::from(value),
        input: Bytes::new(),
        gas_limit: TX_GAS,
        delegate: None,
    };
    h.transact(0, native(fresh2, 1), "separate/native-new", true);
    h.transact(0, native(existing, 5), "separate/native-existing", true);
    h.ok(0, token, Token::transferCall { to: holder2, amount: U256::from(10) }.abi_encode(), U256::ZERO, "separate/erc20-new-holder");
    let (separate, json) = h.end_workflow();
    check_record(&json);
    assert_eq!(separate.user_transaction_signatures(), 3);
    assert_eq!(batched.steps.iter().filter(|s| s.success && s.phase == "action").count(), 1, "one batch signature");
    let (b, s) = (batched.step("self-batch/pay-three"), separate.totals());
    assert_eq!(b.new_accounts + b.new_slots, s.new_accounts + s.new_slots, "same state growth");
    assert!(b.archive_units < s.archive_units, "one envelope instead of three");
}

#[test]
fn added_owner_relay_is_paid_by_the_relayer_and_replay_is_refused() {
    let mut h = Harness::new();
    let relayer = 3;
    h.begin_workflow("p256-added-owner-relay/pay-fresh-recipient", &[0, 2]);
    h.note("owner device 2 signs off-chain; relayer 3 pays its own transaction (no reimbursement)");
    h.phase(recorder::SETUP_PHASE);
    let account = h.delegate_account(0);
    let key = h.add_owner(0, 2, "EastSeaAccount/add-owner-device-2");
    h.phase("action");
    let fresh = Address::repeat_byte(0x61);
    let calls = vec![(fresh, U256::from(7), Bytes::new())];
    let input = h.owner_relay_input(account, 2, key, &calls, "owner-relay/pay");
    let (account_before, relayer_before) = (h.state.balance(&account), h.state.balance(&h.addr(relayer)));
    h.ok(relayer, account, input.clone(), U256::ZERO, "owner-relay/pay");
    assert_eq!(h.state.balance(&account), account_before - U256::from(7), "the account pays only the payment");
    let relayer_paid = relayer_before - h.state.balance(&h.addr(relayer));
    h.revert(relayer, account, input, U256::ZERO, "owner-relay/replay");
    let again = h.owner_relay_input(account, 2, key, &calls, "owner-relay/pay-again");
    h.ok(relayer, account, again, U256::ZERO, "owner-relay/pay-again");
    // An index that does not hold the signing key: refused.
    let wrong = h.owner_relay_input(account, 2, key + 1, &calls, "owner-relay/missing-key");
    h.revert(relayer, account, wrong, U256::ZERO, "owner-relay/missing-key");
    let (record, json) = h.end_workflow();
    check_record(&json);

    let step = record.step("owner-relay/pay");
    assert!(!step.by_user && step.sender == h.addr(relayer));
    assert_eq!(U256::from(step.fee_paid_wei), relayer_paid, "the relayer pays the whole fee");
    assert_eq!((step.new_accounts, step.new_slots), (1, 1), "fresh recipient + first owner nonce");
    let again = record.step("owner-relay/pay-again");
    assert_eq!((again.new_accounts, again.new_slots), (0, 0), "existing recipient, nonce slot already occupied");
    assert_eq!(json["signatures"]["user_transaction"], 2, "delegate + add owner");
    assert_eq!(json["signatures"]["user_typed_message"], 3);
    assert_eq!(json["signatures"]["relayer_transaction"], 4);
    assert_eq!(json["signatures"]["warm_user_total"], 3, "the user's warm cost is off-chain signatures only");
    let failures = json["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 2);
    assert!(failures.iter().all(|f| f["payer_role"] == "relayer" && wei(&f["fee_paid_wei"]) > 0));
    assert_eq!(h.owners(account).1, 2, "two consumed owner nonces");
}

#[test]
fn erc1271_signature_drives_a_signature_checker_consumer() {
    let mut h = Harness::new();
    let book = h.deploy("support/SignedIntentBook", vec![]);
    let other = h.delegate_account(1);
    let relayer = 3;
    h.begin_workflow("p256-erc1271/relayed-signed-intent", &[0]);
    h.phase(recorder::SETUP_PHASE);
    let account = h.delegate_account(0);
    h.phase("action");
    let payload = keccak256("intent #1");
    let digest = |h: &Harness, payload| {
        IntentBook::digestCall::abi_decode_returns(&h.view(0, book, IntentBook::digestCall { payload }.abi_encode())).unwrap()
    };
    let sig = h.account_signature(0, account, digest(&h, payload), "erc1271/intent-signature");
    let accept = |signer, payload, signature: &Bytes| {
        IntentBook::acceptCall { signer, payload, signature: signature.clone() }.abi_encode()
    };
    h.ok(relayer, book, accept(account, payload, &sig), U256::ZERO, "erc1271/relayed-accept");
    h.revert(relayer, book, accept(account, payload, &sig), U256::ZERO, "erc1271/replay");
    h.revert(relayer, book, accept(other, payload, &sig), U256::ZERO, "erc1271/cross-account");
    // A key that does not own the account (not the user's signature: not counted).
    let (r, s) = h.p256_sign(1, &[0u8; 32]);
    let (x, y) = h.p256_xy(1);
    let forged = Bytes::from([r, s, x, y].concat());
    h.revert(relayer, book, accept(account, keccak256("intent #2"), &forged), U256::ZERO, "erc1271/non-owner-key");
    let (record, json) = h.end_workflow();
    check_record(&json);

    let accepted = IntentBook::acceptedCall::abi_decode_returns(&h.view(
        0,
        book,
        IntentBook::acceptedCall { signer: account, payload }.abi_encode(),
    ))
    .unwrap();
    assert!(accepted);
    let step = record.step("erc1271/relayed-accept");
    assert_eq!(step.new_slots, 1, "the retained replay record");
    assert_eq!(step.events, 1);
    assert_eq!(json["signatures"]["user_typed_message"], 1);
    assert_eq!(json["signatures"]["user_transaction"], 1, "delegation only");
    assert_eq!(json["signatures"]["relayer_transaction"], 4);
    assert_eq!(json["failures"].as_array().unwrap().len(), 3);
}

/// The API other lanes use: a Foundry artifact measured in about twenty lines.
#[test]
fn a_foundry_artifact_is_measured_with_the_public_api() {
    let dir = std::env::temp_dir().join(format!("recorder-example-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let artifact = dir.join("SeizableToken.json");
    let object = format!("0x{}", alloy_primitives::hex::encode(bytecode("support/SeizableToken")));
    std::fs::write(&artifact, serde_json::json!({"bytecode": {"object": object}}).to_string()).unwrap();

    let mut h = Harness::new();
    h.begin_workflow("example/token-mint-transfer", &[0]);
    h.phase("setup");
    let token = h.deploy_creation(0, "example/SeizableToken", foundry_creation_code(&artifact), vec![]);
    h.phase("action");
    let me = h.addr(0);
    h.ok(0, token, Token::mintCall { to: me, amount: U256::from(5) }.abi_encode(), U256::ZERO, "mint");
    h.ok(0, token, Token::transferCall { to: h.addr(1), amount: U256::from(2) }.abi_encode(), U256::ZERO, "transfer");
    let (record, json) = h.end_workflow();
    std::fs::remove_dir_all(&dir).unwrap();

    check_record(&json);
    let deploy = record.step("deploy/wallet-recommended-budget");
    assert_eq!(deploy.created, Some(token));
    assert_eq!(deploy.code_bytes, h.state.code(&token).len() as u64);
    assert_eq!(deploy.new_accounts, 1);
    assert_eq!(record.step("transfer").new_slots, 1, "new holder balance");
    assert!(json["b5"]["warm"]["sustained_per_day"][2]["workflows_per_day"].as_u64().unwrap() > 0);
}

#[test]
fn schema_checker_rejects_drift() {
    let mut h = Harness::new();
    h.begin_workflow("schema/drift", &[0]);
    h.transact(
        0,
        EvmCall { to: Some(h.addr(1)), value: U256::ONE, input: Bytes::new(), gas_limit: TX_GAS, delegate: None },
        "native",
        true,
    );
    let (_, json) = h.end_workflow();
    schema::validate(&json).unwrap();
    let mut extra = json.clone();
    extra["undeclared"] = Value::Bool(true);
    assert!(schema::validate(&extra).is_err());
    let mut missing = json.clone();
    missing["steps"][0]["units"].as_object_mut().unwrap().remove("new_slots");
    assert!(schema::validate(&missing).is_err());
    let mut bad = json;
    bad["steps"][0]["fee"]["paid_wei"] = Value::from(5);
    assert!(schema::validate(&bad).is_err());
}
