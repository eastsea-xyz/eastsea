//! Stateless execution (launch plan step 6): a block re-executed on only its
//! witness gives the same pre-state root, receipts and post-state root as on
//! the full state, so a proof over the witness attests the real transition.

use aether_crypto::{P256Signer, Signer};
use aether_execution::account::encode_set_guardians;
use aether_execution::{
    aether_account_code, encode_execute, execute_block, execute_block_sequential, sign_call, BlockContext, EvmCall, FeePolicy, WorldState, AETHER_ACCOUNT,
};
use aether_types::{Address, Bytes, FeeVector, GasVector, TxEnvelope, U256};

const CHAIN: u64 = 7_790;
/// Stores 1 + slot 0 on every call (runtime: PUSH0 SLOAD PUSH1 1 ADD PUSH0 SSTORE STOP).
const COUNTER_INIT: &str = "600a600c600039600a6000f360005460010160005500";

fn signer(i: u8) -> P256Signer {
    let mut s = [0u8; 32];
    s[0] = 0x55;
    s[31] = i;
    P256Signer::from_seed(&s).unwrap()
}

fn addr(s: &P256Signer) -> Address {
    aether_crypto::address_of(&s.public_key()).unwrap()
}

fn ctx(n: u64) -> BlockContext {
    BlockContext {
        chain_id: CHAIN,
        number: n,
        timestamp: n,
        beneficiary: aether_execution::FEE_COLLECTOR,
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        fees: Some(FeePolicy { base: FeeVector::default(), proposer: Address::repeat_byte(0xbe) }),
    }
}

fn state(n: u8) -> WorldState {
    let mut s = WorldState::default();
    for i in 1..=n {
        s.set_balance(addr(&signer(i)), U256::from(10u128.pow(21))).unwrap();
    }
    // Unrelated accounts so that most of the tree stays out of the witness.
    for i in 0..200u8 {
        s.set_balance(Address::repeat_byte(i), U256::from(1_000u64 + i as u64)).unwrap();
    }
    s.set_code(AETHER_ACCOUNT, aether_account_code()).unwrap();
    s
}

fn call(to: Option<Address>, input: Bytes, delegate: Option<Address>) -> EvmCall {
    EvmCall { to, value: U256::ZERO, input, gas_limit: 1_000_000, delegate }
}

fn block_txs() -> Vec<TxEnvelope> {
    let (alice, bob, carol) = (signer(1), signer(2), signer(3));
    let a = addr(&alice);
    let transfers = vec![(addr(&bob), U256::from(5u64), Bytes::new()), (Address::repeat_byte(0xf0), U256::from(7u64), Bytes::new())];
    let guardian = ([7u8; 32], [8u8; 32]);
    let counter = alloy_primitives::hex::decode(COUNTER_INIT).unwrap();
    let created = a.create(2);
    vec![
        // Delegate and batch two payments (one to a new account), then set a guardian.
        sign_call(&alice, CHAIN, 0, 1, &call(Some(a), encode_execute(&transfers), Some(AETHER_ACCOUNT))).unwrap(),
        sign_call(&alice, CHAIN, 2, 1, &call(Some(a), encode_execute(&[(a, U256::ZERO, encode_set_guardians(&[guardian], 1, 600))]), None)).unwrap(),
        // Deploy a counter, then two calls to it from another account.
        sign_call(&alice, CHAIN, 3, 1, &EvmCall { to: None, value: U256::ZERO, input: counter.into(), gas_limit: 200_000, delegate: None }).unwrap(),
        sign_call(&carol, CHAIN, 0, 1, &call(Some(created), Bytes::new(), None)).unwrap(),
        sign_call(&carol, CHAIN, 1, 1, &call(Some(created), Bytes::new(), None)).unwrap(),
        // A plain payment to an existing account.
        sign_call(&bob, CHAIN, 0, 1, &EvmCall { to: Some(Address::repeat_byte(3)), value: U256::from(1u64), input: Bytes::new(), gas_limit: 21_000, delegate: None })
            .unwrap(),
    ]
}

#[test]
fn a_block_on_its_witness_matches_the_full_state() {
    let pre = state(3);
    let txs = block_txs();
    let mut recording = pre.clone();
    recording.record_access();
    let full = execute_block(&recording, &ctx(1), &txs).unwrap();
    assert!(full.receipts.iter().filter(|r| r.success).count() >= 5, "{:?}", full.receipts);

    let witness = pre.witness_for(&full.state);
    let bytes = postcard::to_allocvec(&witness).unwrap();
    let entries = pre.repo().entries().count();
    eprintln!("witness: {} stems, {} opaque nodes, {} codes, {} bytes (full state: {entries} entries)", witness.tree.stems.len(), witness.tree.opaque.len(), witness.codes.len(), bytes.len());
    assert!(witness.tree.stems.len() < entries / 4, "the witness is a small part of the state");

    let stateless = WorldState::from_witness(&postcard::from_bytes(&bytes).unwrap()).unwrap();
    assert_eq!(stateless.root(), pre.root(), "the witness proves the pre-state root");
    for out in [execute_block_sequential(&stateless, &ctx(1), &txs).unwrap(), execute_block(&stateless, &ctx(1), &txs).unwrap()] {
        assert_eq!(out.receipts, full.receipts);
        assert_eq!(out.gas, full.gas);
        assert_eq!(out.state.root(), full.state.root(), "the same post-state root");
    }
}

#[test]
fn a_witness_missing_what_the_block_reads_gives_no_result() {
    let pre = state(3);
    let txs = block_txs();
    let mut recording = pre.clone();
    recording.record_access();
    let full = execute_block(&recording, &ctx(1), &txs).unwrap();
    let mut witness = pre.witness_for(&full.state);

    // Without the account contract's code the delegated batch cannot run as before.
    let mut no_code = witness.clone();
    no_code.codes.clear();
    let s = WorldState::from_witness(&no_code).unwrap();
    let out = std::panic::catch_unwind(|| execute_block_sequential(&s, &ctx(1), &txs).map(|o| o.state.root()));
    assert!(!matches!(out, Ok(Ok(r)) if r == full.state.root()), "missing code cannot give the real post-state");

    // A dropped stem changes the pre-state root (or is refused when read).
    witness.tree.stems.remove(0);
    if let Ok(s) = WorldState::from_witness(&witness) {
        assert_ne!(s.root(), pre.root());
    }
}
