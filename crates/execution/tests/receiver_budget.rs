//! Client budgets for empty-calldata value sends on paid-state chains.

use aether_crypto::{P256Signer, Signer};
use aether_execution::block::{receipt_persistent_bytes, tx_persistent_bytes};
use aether_execution::{
    check_admission_cost, execute_block, fees, plain_transfer_gas_limit, recommended_state_budget,
    sign_call_with, validate_stateless, BlockContext, EvmCall, FeePolicy, WorldState,
    FEE_COLLECTOR,
};
use aether_types::{Address, Bytes, Canonical, FeeVector, GasVector, TxEnvelope, U256};

const CHAIN: u64 = 7_777;
const RECEIVER: Address = Address::repeat_byte(0x55);
const DELEGATE: Address = Address::repeat_byte(0x77);
// receive(): store CALLER, ORIGIN, CHAINID, CALLVALUE, NUMBER and TIMESTAMP.
const RECEIVE: &[u8] = &[
    0x33, 0x60, 0x00, 0x55, 0x32, 0x60, 0x01, 0x55, 0x46, 0x60, 0x02, 0x55, 0x34, 0x60, 0x03, 0x55,
    0x43, 0x60, 0x04, 0x55, 0x42, 0x60, 0x05, 0x55, 0x00,
];

fn context() -> BlockContext {
    BlockContext {
        chain_id: CHAIN,
        number: 1,
        timestamp: 1,
        beneficiary: FEE_COLLECTOR,
        limits: GasVector {
            exec: 30_000_000,
            state: 100_000,
            prove: 200_000_000,
        },
        fees: Some(FeePolicy {
            base: FeeVector {
                exec: 0,
                state: fees::STATE_UNIT_PRICE,
                prove: 0,
            },
            proposer: Address::repeat_byte(0xbe),
        }),
    }
}

fn signed(signer: &P256Signer, call: &EvmCall, budget: u64) -> TxEnvelope {
    let mut tx = sign_call_with(
        signer,
        CHAIN,
        0,
        FeeVector {
            exec: 0,
            state: fees::STATE_UNIT_PRICE,
            prove: 0,
        },
        0,
        call,
    )
    .unwrap();
    tx.header.gas.state = budget;
    let mut signature = signer.sign(&tx.signing_bytes()).unwrap();
    signature.extend_from_slice(&signer.public_key().bytes);
    tx.signature = Bytes::from(signature);
    tx
}

fn check_receiver(recipient_code: Bytes, target_code: Option<Bytes>, gas_limit: u64) {
    let signer = P256Signer::from_seed(&[11; 32]).unwrap();
    let sender = aether_crypto::address_of(&signer.public_key()).unwrap();
    let balance = U256::from(10u128.pow(20));
    let mut state = WorldState::default();
    state.set_balance(sender, balance).unwrap();
    state.set_code(RECEIVER, recipient_code.clone()).unwrap();
    if let Some(code) = target_code {
        state.set_code(DELEGATE, code).unwrap();
    }
    let call = EvmCall {
        to: Some(RECEIVER),
        value: U256::from(4883),
        input: Bytes::new(),
        gas_limit,
        delegate: None,
    };
    let budget = recommended_state_budget(&call, Some(balance), fees::STATE_UNIT_PRICE);
    assert_eq!(budget, gas_limit / 200 + fees::STATE_ACCOUNT_UNITS + 16);
    let tx = signed(&signer, &call, budget);
    assert_eq!(validate_stateless(&tx, CHAIN).unwrap(), call);
    let admitted =
        check_admission_cost(&state, &context(), &tx).expect("the receiver fits the signed budget");
    let out = execute_block(&state, &context(), std::slice::from_ref(&tx)).unwrap();
    let receipt = &out.receipts[0];
    assert!(
        receipt.success,
        "six fresh slots must fit the wallet's execution allowance"
    );
    assert_eq!(admitted.gas, out.gas);
    let archived = (tx_persistent_bytes(&tx) + receipt_persistent_bytes(receipt))
        .div_ceil(fees::RECEIPT_BYTES_PER_STATE_UNIT);
    assert_eq!(out.gas.state, 6 * fees::STATE_SLOT_UNITS + archived);
    assert!(out.gas.state <= budget);
    assert!(out.gas.exec < gas_limit);
    assert_eq!(
        receipt.state_fee,
        U256::from(out.gas.state) * U256::from(fees::STATE_UNIT_PRICE)
    );
    assert_eq!(out.settlement.burned_state, receipt.state_fee);
    assert_eq!(
        balance - out.state.balance(&sender),
        call.value + receipt.state_fee
    );
    assert_eq!(out.state.balance(&RECEIVER), call.value);
    let sender_word = U256::from_be_slice(sender.as_slice());
    for (slot, expected) in [
        sender_word,
        sender_word,
        U256::from(CHAIN),
        call.value,
        U256::from(1),
        U256::from(1),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(out.state.storage(&RECEIVER, U256::from(slot)), expected);
    }
    assert_eq!(out.state.code(&RECEIVER), recipient_code);
}

#[test]
fn receiver_budget_wallet_send_to_six_slot_receive_is_admitted_and_charged() {
    check_receiver(
        Bytes::from_static(RECEIVE),
        None,
        plain_transfer_gas_limit(RECEIVE),
    );
}

#[test]
fn receiver_budget_cli_empty_data_value_call_is_admitted_and_charged() {
    check_receiver(Bytes::from_static(RECEIVE), None, 300_000);
}

#[test]
fn receiver_budget_existing_7702_recipient_runs_and_pays_for_its_slots() {
    let mut designator = vec![0xef, 0x01, 0x00];
    designator.extend_from_slice(DELEGATE.as_slice());
    let gas_limit = plain_transfer_gas_limit(&designator);
    check_receiver(
        designator.into(),
        Some(Bytes::from_static(RECEIVE)),
        gas_limit,
    );
}

#[test]
fn receiver_budget_eoa_envelope_bytes_stay_unchanged_only_at_intrinsic_gas() {
    let signer = P256Signer::from_seed(&[11; 32]).unwrap();
    let balance = Some(U256::from(10u128.pow(20)));
    let call = EvmCall {
        to: Some(RECEIVER),
        value: U256::from(4883),
        input: Bytes::new(),
        gas_limit: plain_transfer_gas_limit(&[]),
        delegate: None,
    };
    assert_eq!(call.gas_limit, 21_000);
    let budget = recommended_state_budget(&call, balance, fees::STATE_UNIT_PRICE);
    assert_eq!(budget, 216);
    let legacy = signed(&signer, &call, 216);
    let current = signed(&signer, &call, budget);
    assert_eq!(current.to_canonical_bytes(), legacy.to_canonical_bytes());
    let zero_value = EvmCall {
        value: U256::ZERO,
        ..call.clone()
    };
    assert_eq!(
        recommended_state_budget(&zero_value, balance, fees::STATE_UNIT_PRICE),
        116
    );
    assert_eq!(recommended_state_budget(&call, balance, 0), 0);

    // Larger limits must use the contract estimate even with empty calldata.
    // The transfer shortcut is reserved for the 21,000-gas no-code path.
    let executable = EvmCall {
        gas_limit: 300_000,
        ..call
    };
    assert_eq!(
        recommended_state_budget(&executable, balance, fees::STATE_UNIT_PRICE),
        1616
    );
}
