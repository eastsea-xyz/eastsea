//! A5-1: finalized EVM state growth has a fixed, burned price even below target.

use aether_crypto::{P256Signer, Signer};
use aether_execution::block::{receipt_persistent_bytes, tx_persistent_bytes};
use aether_execution::{
    build_block, execute_block, execute_block_sequential, sign_call_with, BlockContext, EvmCall,
    ExecError, FeePolicy, WorldState, FEE_COLLECTOR,
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
        200,
        Some(WRITER),
        Bytes::from(U256::from(slot).to_be_bytes::<32>().to_vec()),
        100_000,
    )
}

fn archived_units(tx: &TxEnvelope, receipt: &aether_execution::Receipt) -> u64 {
    (tx_persistent_bytes(tx) + receipt_persistent_bytes(receipt)).div_ceil(32)
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
        let tx = write(&s, n, n);
        let out = execute_block(&state, &block, std::slice::from_ref(&tx)).unwrap();
        assert!(out.receipts[0].success);
        assert_eq!(out.gas.state, 100 + archived_units(&tx, &out.receipts[0]));
        excess = aether_execution::fees::next_excess(excess, out.gas, block.limits);
        state = out.state;
    }
    assert!(original - state.balance(&addr(&s)) > U256::from(8 * 100 * PRICE));
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
fn zero_balance_plain_transfer_cannot_create_a_free_sender() {
    let s = signer(3);
    let call = signed(
        &s,
        0,
        0,
        Some(Address::repeat_byte(4)),
        Bytes::new(),
        21_000,
    );
    let state = WorldState::default();
    assert!(aether_execution::block::check_admission(&state, &ctx(1), &call).is_err());
    assert!(matches!(
        execute_block(&state, &ctx(1), &[call]),
        Err(ExecError::InvalidTx { .. })
    ));
    assert!(state.account(&addr(&s)).is_none());
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
    assert!(out.gas.state > 0);
    assert_eq!(
        before - out.state.balance(&addr(&s)),
        out.receipts[0].state_fee
    );
    state
        .set_code(WRITER, Bytes::from_static(SET_CLEAR))
        .unwrap();
    let out = execute_block(
        &state,
        &ctx(1),
        &[signed(&s, 0, 100, Some(WRITER), Bytes::new(), 100_000)],
    )
    .unwrap();
    assert!(out.gas.state > 0);
    assert_eq!(
        before - out.state.balance(&addr(&s)),
        out.receipts[0].state_fee
    );
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
    let tx = signed(&s, 0, 300, None, init, 200_000);
    let out = execute_block(&state, &ctx(1), std::slice::from_ref(&tx)).unwrap();
    assert!(out.receipts[0].success);
    let charged = 101 + archived_units(&tx, &out.receipts[0]);
    assert_eq!(out.gas.state, charged);
    assert_eq!(
        out.receipts[0].state_fee,
        U256::from(charged) * U256::from(PRICE)
    );
    assert_eq!(
        out.settlement.burned_state,
        U256::from(charged) * U256::from(PRICE)
    );
    assert_eq!(
        before - out.state.balance(&addr(&s)),
        U256::from(charged) * U256::from(PRICE)
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
    tx.header.gas.state = 200;
    let mut sig = s.sign(&tx.signing_bytes()).unwrap();
    sig.extend_from_slice(&s.public_key().bytes);
    tx.signature = Bytes::from(sig);
    let out = execute_block(&state, &ctx(1), &[tx.clone()]).unwrap();
    let charged = 100 + archived_units(&tx, &out.receipts[0]);
    assert_eq!(out.gas.state, charged);
    assert_eq!(out.state.balance(&recipient), U256::from(1));
    assert_eq!(
        state.balance(&addr(&s)) - out.state.balance(&addr(&s)),
        U256::from(charged) * U256::from(PRICE) + U256::from(1)
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
            state: PRICE,
            prove: 0,
        },
        1,
        &call,
    )
    .unwrap();
    let mut tx = tx;
    tx.header.gas.state = 100;
    let mut sig = s.sign(&tx.signing_bytes()).unwrap();
    sig.extend_from_slice(&s.public_key().bytes);
    tx.signature = Bytes::from(sig);
    let out = execute_block(&state, &ctx(1), &[tx.clone()]).unwrap();
    assert_eq!(out.gas.state, archived_units(&tx, &out.receipts[0]));
    assert_eq!(
        out.receipts[0].state_fee,
        U256::from(out.gas.state) * U256::from(PRICE)
    );
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
    let first = execute_block_sequential(&state, &ctx(1), &txs[..512]).unwrap();
    assert_eq!(first.new_slots, 512);
    assert!(!aether_execution::can_append(&first.state, &ctx(1), first.gas, first.new_slots, first.persistent_bytes, &txs[512]));
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
fn chain_7780_keeps_fresh_zero_balance_sender_and_legacy_receipt_bytes() {
    #[derive(serde::Serialize)]
    struct LegacyReceipt<'a> {
        tx_hash: aether_types::TxHash,
        success: bool,
        gas_used: u64,
        prove_gas: u64,
        contract_address: Option<Address>,
        logs: u32,
        output: &'a Bytes,
    }

    let s = signer(28);
    let mut legacy = ctx(1);
    legacy.chain_id = 7780;
    legacy.limits.state = u64::MAX;
    legacy.fees.as_mut().unwrap().base.state = 0;
    let call = EvmCall {
        to: Some(Address::repeat_byte(0x44)),
        value: U256::ZERO,
        input: Bytes::new(),
        gas_limit: 21_000,
        delegate: None,
    };
    let tx = sign_call_with(&s, 7780, 0, FeeVector::default(), 0, &call).unwrap();
    let out = execute_block(&WorldState::default(), &legacy, &[tx]).unwrap();
    let receipt = &out.receipts[0];
    assert!(receipt.success);
    assert_eq!(receipt.state_gas, 0);
    assert_eq!(receipt.state_fee, U256::ZERO);
    assert_eq!(out.persistent_bytes, 0);
    assert_eq!(out.state.nonce(&addr(&s)), 1);
    let old_shape = LegacyReceipt {
        tx_hash: receipt.tx_hash,
        success: receipt.success,
        gas_used: receipt.gas_used,
        prove_gas: receipt.prove_gas,
        contract_address: receipt.contract_address,
        logs: receipt.logs,
        output: &receipt.output,
    };
    assert_eq!(
        postcard::to_allocvec(receipt).unwrap(),
        postcard::to_allocvec(&old_shape).unwrap()
    );
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
    assert_eq!(
        parallel.gas.state,
        500 + txs
            .iter()
            .zip(&parallel.receipts)
            .map(|(tx, receipt)| archived_units(tx, receipt))
            .sum::<u64>()
    );
}

fn logger(records: usize) -> Bytes {
    let mut code = Vec::with_capacity(records * 6 + 1);
    for _ in 0..records {
        // LOG0(offset=0, size=4096), reading zero-initialized memory.
        code.extend_from_slice(&[0x61, 0x10, 0x00, 0x60, 0x00, 0xa0]);
    }
    code.push(0x00);
    Bytes::from(code)
}

#[test]
fn audit_logger_cannot_create_free_receipt_bytes() {
    let s = signer(30);
    let mut state = WorldState::default();
    state
        .set_balance(addr(&s), U256::from(10u128.pow(20)))
        .unwrap();
    state.set_code(WRITER, logger(400)).unwrap();
    let free = signed(&s, 0, 0, Some(WRITER), Bytes::new(), 15_000_000);
    let before = state.root();
    assert!(aether_execution::check_admission(&state, &ctx(1), &free).is_err());
    assert!(matches!(
        execute_block(&state, &ctx(1), std::slice::from_ref(&free)),
        Err(ExecError::InvalidTx { .. })
    ));
    let (included, proposed) = build_block(&state, &ctx(1), vec![free]);
    assert!(included.is_empty());
    assert!(proposed.receipts.is_empty());
    assert_eq!(proposed.state.root(), before);

    let paid = signed(&s, 0, 60_000, Some(WRITER), Bytes::new(), 15_000_000);
    let out = execute_block(&state, &ctx(1), std::slice::from_ref(&paid)).unwrap();
    assert_eq!(out.receipts[0].logs, 400);
    assert_eq!(
        out.receipts[0]
            .events
            .iter()
            .map(|e| e.data.len())
            .sum::<usize>(),
        400 * 4096
    );
    assert_eq!(out.gas.state, archived_units(&paid, &out.receipts[0]));
    assert!(out.receipts[0].state_fee > U256::ZERO);
}

#[test]
fn cumulative_persistent_byte_cap_is_identical_for_proposer_and_validator() {
    let s = signer(31);
    let mut state = WorldState::default();
    state
        .set_balance(addr(&s), U256::from(10u128.pow(20)))
        .unwrap();
    state.set_code(WRITER, logger(150)).unwrap();
    let txs: Vec<_> = (0..4)
        .map(|nonce| signed(&s, nonce, 25_000, Some(WRITER), Bytes::new(), 5_500_000))
        .collect();
    let first = execute_block(&state, &ctx(1), &txs[..3]).unwrap();
    assert!(first.persistent_bytes < aether_execution::fees::MAX_PERSISTENT_BYTES_PER_BLOCK);
    assert!(
        first.persistent_bytes + 600_000 > aether_execution::fees::MAX_PERSISTENT_BYTES_PER_BLOCK
    );
    assert!(!aether_execution::can_append(
        &first.state,
        &ctx(1),
        first.gas,
        first.new_slots,
        first.persistent_bytes,
        &txs[3]
    ));
    assert!(matches!(
        execute_block(&state, &ctx(1), &txs),
        Err(ExecError::LimitExceeded { index: 3 })
    ));
    let (included, proposed) = build_block(&state, &ctx(1), txs);
    assert_eq!(included.len(), 3);
    assert_eq!(proposed.gas, first.gas);
    assert_eq!(proposed.persistent_bytes, first.persistent_bytes);
}

#[test]
fn audit_714_fresh_senders_cannot_make_unpaid_accounts() {
    let state = WorldState::default();
    let txs: Vec<_> = (0..714u16)
        .map(|n| {
            let mut seed = [0u8; 32];
            seed[..2].copy_from_slice(&(n + 1).to_be_bytes());
            let s = P256Signer::from_seed(&seed).unwrap();
            signed(
                &s,
                0,
                0,
                Some(Address::repeat_byte(0x42)),
                Bytes::new(),
                21_000,
            )
        })
        .collect();
    assert!(matches!(
        execute_block(&state, &ctx(1), &txs),
        Err(ExecError::InvalidTx { index: 0, .. })
    ));
    let (included, proposed) = build_block(&state, &ctx(1), txs);
    assert!(included.is_empty());
    assert_eq!(proposed.state.root(), state.root());
    assert!(proposed.receipts.is_empty());
}

/// Live run 2026-10-06 (docs/research/contracts-live-2026-10-06.md): once the
/// B5 burst is spent, a deploy that needs more state units than the rolling
/// budget has left was refused with "transaction exceeds block gas limit" —
/// the same words as a genuinely oversized transaction, so a wallet could not
/// tell "too big, never" from "busy, retry shortly". Admission now names the
/// state budget, what the tx needs, what is left and how long the refill takes.
#[test]
fn a_spent_state_budget_refusal_says_state_budget_and_when_to_retry() {
    let s = signer(9);
    let mut state = WorldState::default();
    state
        .set_balance(addr(&s), U256::from(10u128.pow(20)))
        .unwrap();
    state.set_code(WRITER, Bytes::from_static(WRITE)).unwrap();
    // A spent budget also means the exponential state surcharge (fees.rs
    // `state_base_fee`), so the wallet signs a cap above it, as the CLI does
    // from the node's quoted base fee.
    let call = EvmCall {
        to: Some(WRITER),
        value: U256::ZERO,
        input: Bytes::from(U256::from(1u64).to_be_bytes::<32>().to_vec()),
        gas_limit: 100_000,
        delegate: None,
    };
    let fees = FeeVector { exec: 0, state: 1_000 * PRICE, prove: 0 };
    let mut tx = sign_call_with(&s, CHAIN, 0, fees, 0, &call).unwrap();
    tx.header.gas.state = 200;
    let mut sig = s.sign(&tx.signing_bytes()).unwrap();
    sig.extend_from_slice(&s.public_key().bytes);
    tx.signature = Bytes::from(sig);
    let need = aether_execution::check_admission_cost(&state, &ctx(1), &tx)
        .unwrap()
        .gas
        .state;

    let mut spent = ctx(1);
    spent.limits.state = need - 40;
    let err = aether_execution::check_admission_cost(&state, &spent, &tx).unwrap_err();
    assert!(err.contains("state budget"), "{err}");
    assert!(!err.contains("block gas limit"), "{err}");
    assert!(err.contains(&format!("needs {need} state units")), "{err}");
    assert!(err.contains(&format!("{} are available", need - 40)), "{err}");
    // 40 missing units at 32 per block: two blocks of refill.
    assert!(err.contains("about 2 blocks"), "{err}");

    // An exec-gas overrun is never worded as a temporary state budget.
    let mut tight = ctx(1);
    tight.limits.exec = 1_000;
    let err = aether_execution::check_admission_cost(&state, &tight, &tx).unwrap_err();
    assert!(!err.contains("state budget"), "{err}");
}
