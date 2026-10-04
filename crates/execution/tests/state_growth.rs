//! A5-1: finalized EVM state growth has a fixed, burned price even below target.

use aether_crypto::{P256Signer, Signer};
use aether_execution::{
    execute_block, execute_block_sequential, sign_call_with, BlockContext, EvmCall, ExecError,
    FeePolicy, WorldState, FEE_COLLECTOR,
};
use aether_types::{Address, Bytes, FeeVector, GasVector, TxEnvelope, U256};

const CHAIN: u64 = 7_777;
const PRICE: u128 = 1_000_000_000_000;
const WRITER: Address = Address::repeat_byte(0x77);
// calldata[0] is the slot; set it to one.
const WRITE: &[u8] = &[0x60, 0x01, 0x60, 0x00, 0x35, 0x55, 0x00];
// Set slot zero, then clear it before the transaction commits.
const SET_CLEAR: &[u8] = &[
    0x60, 0x01, 0x60, 0x00, 0x55, 0x60, 0x00, 0x60, 0x00, 0x55, 0x00,
];

fn signer(seed: u8) -> P256Signer {
    P256Signer::from_seed(&[seed; 32]).unwrap()
}
fn addr(s: &P256Signer) -> Address {
    aether_crypto::address_of(&s.public_key()).unwrap()
}
fn ctx(n: u64) -> BlockContext {
    BlockContext {
        chain_id: CHAIN,
        number: n,
        timestamp: n,
        beneficiary: FEE_COLLECTOR,
        limits: GasVector {
            exec: 30_000_000,
            state: 100_000,
            prove: 200_000_000,
        },
        fees: Some(FeePolicy {
            base: FeeVector {
                exec: 0,
                state: PRICE,
                prove: 0,
            },
            proposer: Address::repeat_byte(0xbe),
        }),
    }
}
fn signed(
    s: &P256Signer,
    nonce: u64,
    state_budget: u64,
    to: Option<Address>,
    input: Bytes,
    gas_limit: u64,
) -> TxEnvelope {
    let call = EvmCall {
        to,
        value: U256::ZERO,
        input,
        gas_limit,
        delegate: None,
    };
    let mut tx = sign_call_with(
        s,
        CHAIN,
        nonce,
        FeeVector {
            exec: 0,
            state: PRICE,
            prove: 0,
        },
        0,
        &call,
    )
    .unwrap();
    tx.header.gas.state = state_budget;
    let mut sig = s.sign(&tx.signing_bytes()).unwrap();
    sig.extend_from_slice(&s.public_key().bytes);
    tx.signature = Bytes::from(sig);
    tx
}
fn write(s: &P256Signer, nonce: u64, slot: u64) -> TxEnvelope {
    signed(
        s,
        nonce,
        100,
        Some(WRITER),
        Bytes::from(U256::from(slot).to_be_bytes::<32>().to_vec()),
        100_000,
    )
}

#[test]
fn below_target_writes_burn_for_every_new_slot_across_blocks() {
    let s = signer(1);
    let mut state = WorldState::default();
    state
        .set_balance(addr(&s), U256::from(10u128.pow(20)))
        .unwrap();
    state.set_code(WRITER, Bytes::from_static(WRITE)).unwrap();
    let original = state.balance(&addr(&s));
    let mut excess = GasVector::default();
    for n in 0..8 {
        let block = ctx(n + 1);
        assert_eq!(
            aether_execution::fees::base_fee(excess, block.limits).exec,
            0
        );
        assert_eq!(
            aether_execution::fees::base_fee(excess, block.limits).prove,
            0
        );
        let out = execute_block(&state, &block, &[write(&s, n, n)]).unwrap();
        assert!(out.receipts[0].success);
        assert_eq!(out.gas.state, 100);
        excess = aether_execution::fees::next_excess(excess, out.gas, block.limits);
        state = out.state;
    }
    assert_eq!(
        original - state.balance(&addr(&s)),
        U256::from(8 * 100 * PRICE)
    );
}

#[test]
fn zero_balance_writer_is_invalid() {
    let s = signer(2);
    let mut state = WorldState::default();
    state.set_code(WRITER, Bytes::from_static(WRITE)).unwrap();
    assert!(matches!(
        execute_block(&state, &ctx(1), &[write(&s, 0, 1)]),
        Err(ExecError::InvalidTx { .. })
    ));
}

#[test]
fn zero_balance_plain_transfer_remains_free() {
    let s = signer(3);
    let call = signed(
        &s,
        0,
        0,
        Some(Address::repeat_byte(4)),
        Bytes::new(),
        21_000,
    );
    let out = execute_block(&WorldState::default(), &ctx(1), &[call]).unwrap();
    assert!(out.receipts[0].success);
    assert_eq!(out.gas.state, 0);
    assert_eq!(out.state.balance(&addr(&s)), U256::ZERO);
}

#[test]
fn existing_slot_and_same_tx_set_clear_have_no_growth_charge() {
    let s = signer(4);
    let mut state = WorldState::default();
    state
        .set_balance(addr(&s), U256::from(10u128.pow(20)))
        .unwrap();
    state.set_code(WRITER, Bytes::from_static(WRITE)).unwrap();
    state.set_storage(WRITER, U256::from(7), U256::from(1));
    let before = state.balance(&addr(&s));
    let out = execute_block(&state, &ctx(1), &[write(&s, 0, 7)]).unwrap();
    assert_eq!(out.gas.state, 0);
    assert_eq!(out.state.balance(&addr(&s)), before);
    state
        .set_code(WRITER, Bytes::from_static(SET_CLEAR))
        .unwrap();
    let out = execute_block(
        &state,
        &ctx(1),
        &[signed(&s, 0, 100, Some(WRITER), Bytes::new(), 100_000)],
    )
    .unwrap();
    assert_eq!(out.gas.state, 0);
    assert_eq!(out.state.balance(&addr(&s)), before);
}

#[test]
fn deployment_pays_for_its_account_and_each_code_byte() {
    let s = signer(5);
    let mut state = WorldState::default();
    state
        .set_balance(addr(&s), U256::from(10u128.pow(20)))
        .unwrap();
    let before = state.balance(&addr(&s));
    // Init code returns one STOP byte as the persistent runtime.
    let init = Bytes::from_static(&[
        0x60, 0x01, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, 0x01, 0x60, 0x00, 0xf3, 0x00,
    ]);
    let out = execute_block(&state, &ctx(1), &[signed(&s, 0, 101, None, init, 200_000)]).unwrap();
    assert!(out.receipts[0].success);
    assert_eq!(out.gas.state, 101);
    assert_eq!(out.receipts[0].state_fee, U256::from(101 * PRICE));
    assert_eq!(out.settlement.burned_state, U256::from(101 * PRICE));
    assert_eq!(
        before - out.state.balance(&addr(&s)),
        U256::from(101 * PRICE)
    );
}

#[test]
fn funding_a_new_account_pays_once_for_the_account_record() {
    let s = signer(9);
    let recipient = Address::repeat_byte(0x90);
    let mut state = WorldState::default();
    state
        .set_balance(addr(&s), U256::from(10u128.pow(20)))
        .unwrap();
    let call = EvmCall {
        to: Some(recipient),
        value: U256::from(1),
        input: Bytes::new(),
        gas_limit: 21_000,
        delegate: None,
    };
    let mut tx = sign_call_with(
        &s,
        CHAIN,
        0,
        FeeVector {
            exec: 0,
            state: PRICE,
            prove: 0,
        },
        0,
        &call,
    )
    .unwrap();
    tx.header.gas.state = 100;
    let mut sig = s.sign(&tx.signing_bytes()).unwrap();
    sig.extend_from_slice(&s.public_key().bytes);
    tx.signature = Bytes::from(sig);
    let out = execute_block(&state, &ctx(1), &[tx]).unwrap();
    assert_eq!(out.gas.state, 100);
    assert_eq!(out.state.balance(&recipient), U256::from(1));
    assert_eq!(
        state.balance(&addr(&s)) - out.state.balance(&addr(&s)),
        U256::from(100 * PRICE + 1)
    );
}

#[test]
fn a_new_fee_collector_is_not_charged_as_user_state_growth() {
    let s = signer(10);
    let mut state = WorldState::default();
    state
        .set_balance(addr(&s), U256::from(10u128.pow(20)))
        .unwrap();
    let call = EvmCall {
        to: Some(addr(&s)),
        value: U256::ZERO,
        input: Bytes::new(),
        gas_limit: 21_000,
        delegate: None,
    };
    let tx = sign_call_with(
        &s,
        CHAIN,
        0,
        FeeVector {
            exec: 1,
            state: 0,
            prove: 0,
        },
        1,
        &call,
    )
    .unwrap();
    let out = execute_block(&state, &ctx(1), &[tx]).unwrap();
    assert_eq!(out.gas.state, 0);
    assert_eq!(out.receipts[0].state_fee, U256::ZERO);
}

#[test]
fn one_transaction_cannot_create_more_than_512_slots() {
    let s = signer(6);
    let mut state = WorldState::default();
    state
        .set_balance(addr(&s), U256::from(10u128.pow(20)))
        .unwrap();
    let mut code = Vec::new();
    for slot in 0..513u16 {
        code.extend_from_slice(&[0x60, 0x01, 0x61]);
        code.extend_from_slice(&slot.to_be_bytes());
        code.push(0x55);
    }
    code.push(0x00);
    state.set_code(WRITER, Bytes::from(code)).unwrap();
    let tx = signed(&s, 0, 51_300, Some(WRITER), Bytes::new(), 15_000_000);
    assert!(matches!(
        execute_block_sequential(&state, &ctx(1), &[tx]),
        Err(ExecError::InvalidTx { .. })
    ));
}

#[test]
fn a_block_cannot_create_more_than_512_slots() {
    let s = signer(7);
    let mut state = WorldState::default();
    state
        .set_balance(addr(&s), U256::from(10u128.pow(20)))
        .unwrap();
    state.set_code(WRITER, Bytes::from_static(WRITE)).unwrap();
    let txs: Vec<_> = (0..513).map(|n| write(&s, n, n)).collect();
    assert!(matches!(
        execute_block_sequential(&state, &ctx(1), &txs),
        Err(ExecError::LimitExceeded { index: 512 })
    ));
}

#[test]
fn legacy_context_keeps_state_growth_free_and_unmetered() {
    let s = signer(8);
    let mut state = WorldState::default();
    state.set_code(WRITER, Bytes::from_static(WRITE)).unwrap();
    let mut legacy = ctx(1);
    legacy.limits.state = u64::MAX;
    legacy.fees.as_mut().unwrap().base.state = 0;
    let call = EvmCall {
        to: Some(WRITER),
        value: U256::ZERO,
        input: Bytes::from(U256::from(9).to_be_bytes::<32>().to_vec()),
        gas_limit: 100_000,
        delegate: None,
    };
    let tx = sign_call_with(&s, CHAIN, 0, FeeVector::default(), 0, &call).unwrap();
    let out = execute_block(&state, &legacy, &[tx]).unwrap();
    assert!(out.receipts[0].success);
    assert_eq!(out.gas.state, 0);
    assert_eq!(out.receipts[0].state_fee, U256::ZERO);
    assert_eq!(out.state.balance(&addr(&s)), U256::ZERO);
    assert_eq!(out.state.storage(&WRITER, U256::from(9)), U256::from(1));
}

#[test]
fn parallel_and_sequential_replay_agree_on_state_fees() {
    let signers: Vec<_> = (20..25).map(signer).collect();
    let mut state = WorldState::default();
    state.set_code(WRITER, Bytes::from_static(WRITE)).unwrap();
    for s in &signers {
        state
            .set_balance(addr(s), U256::from(10u128.pow(20)))
            .unwrap();
    }
    let txs: Vec<_> = signers
        .iter()
        .enumerate()
        .map(|(slot, s)| write(s, 0, slot as u64))
        .collect();
    let parallel = execute_block(&state, &ctx(1), &txs).unwrap();
    let sequential = execute_block_sequential(&state, &ctx(1), &txs).unwrap();
    assert_eq!(parallel.state.root(), sequential.state.root());
    assert_eq!(parallel.gas, sequential.gas);
    assert_eq!(parallel.receipts, sequential.receipts);
    assert_eq!(
        parallel.settlement.burned_state,
        sequential.settlement.burned_state
    );
    assert_eq!(parallel.gas.state, 500);
}
