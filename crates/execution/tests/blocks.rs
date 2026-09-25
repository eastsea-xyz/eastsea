//! End-to-end execution: signed txs -> revm -> EIP-7864 state, BAL, proofs.

use aether_crypto::{P256Signer, Secp256k1Signer, Signer};
use aether_execution::{build_block, execute_block, sign_call, BlockContext, EvmCall, ExecError, WorldState};
use aether_state::layout::basic_data_key;
use aether_state::StateRepository;
use aether_types::{Address, Bytes, GasVector, U256};

const CHAIN: u64 = 7_777;
/// Runtime: slot0 += 1. Init code copies the 10-byte runtime and returns it.
const COUNTER_INIT: &[u8] = &[
    0x60, 0x0a, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, 0x0a, 0x60, 0x00, 0xf3, // init
    0x60, 0x00, 0x54, 0x60, 0x01, 0x01, 0x60, 0x00, 0x55, 0x00, // runtime
];

fn seed(b: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[31] = b;
    s
}

fn ctx(n: u64) -> BlockContext {
    BlockContext {
        chain_id: CHAIN,
        number: n,
        timestamp: 1_700_000_000 + n,
        beneficiary: Address::repeat_byte(0xbe),
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 50_000_000 },
    }
}

fn funded(signers: &[&dyn Signer]) -> WorldState {
    let mut s = WorldState::default();
    for sg in signers {
        let a = aether_crypto::address_of(&sg.public_key()).unwrap();
        s.set_balance(a, U256::from(10u128.pow(21))).unwrap();
    }
    s
}

fn transfer(to: Address, wei: u64) -> EvmCall {
    EvmCall { to: Some(to), value: U256::from(wei), input: Bytes::new(), gas_limit: 21_000 }
}

#[test]
fn transfer_updates_balances_nonce_fee_and_bal() {
    let alice = P256Signer::from_seed(&seed(1)).unwrap();
    let a = aether_crypto::address_of(&alice.public_key()).unwrap();
    let bob = Address::repeat_byte(0xb0);
    let pre = funded(&[&alice]);
    let tx = sign_call(&alice, CHAIN, 0, 2, &transfer(bob, 1_000)).unwrap();

    let out = execute_block(&pre, &ctx(1), &[tx]).unwrap();
    let s = &out.state;
    assert_eq!(s.balance(&bob), U256::from(1_000));
    assert_eq!(s.nonce(&a), 1);
    assert_eq!(out.receipts[0].gas_used, 21_000);
    assert_eq!(s.balance(&a), U256::from(10u128.pow(21) - 1_000 - 21_000 * 2));
    assert_eq!(s.balance(&Address::repeat_byte(0xbe)), U256::from(21_000 * 2), "fee to proposer");
    assert_ne!(s.root(), pre.root());
    let touched: Vec<Address> = out.bal.accounts.iter().map(|x| x.address).collect();
    assert!(touched.contains(&a) && touched.contains(&bob));
    assert!(out.bal.validate_shape().is_ok());
}

#[test]
fn replay_and_bad_nonce_rejected() {
    let alice = Secp256k1Signer::from_seed(&seed(2)).unwrap();
    let pre = funded(&[&alice]);
    let tx = sign_call(&alice, CHAIN, 0, 1, &transfer(Address::repeat_byte(1), 5)).unwrap();
    assert!(matches!(execute_block(&pre, &ctx(1), &[tx.clone(), tx.clone()]), Err(ExecError::InvalidTx { index: 1, .. })));
    let (included, _) = build_block(&pre, &ctx(1), vec![tx.clone(), tx]);
    assert_eq!(included.len(), 1, "proposer drops the replay");
}

#[test]
fn deploy_and_call_contract() {
    let dev = P256Signer::from_seed(&seed(3)).unwrap();
    let mut state = funded(&[&dev]);
    let deploy = sign_call(&dev, CHAIN, 0, 1, &EvmCall { to: None, value: U256::ZERO, input: Bytes::from_static(COUNTER_INIT), gas_limit: 200_000 }).unwrap();
    let out = execute_block(&state, &ctx(1), &[deploy]).unwrap();
    let counter = out.receipts[0].contract_address.expect("created");
    assert!(out.receipts[0].success);
    state = out.state;
    assert_eq!(state.code(&counter).len(), 10);

    let calls: Vec<_> = (1..=2)
        .map(|n| sign_call(&dev, CHAIN, n, 1, &EvmCall { to: Some(counter), value: U256::ZERO, input: Bytes::new(), gas_limit: 100_000 }).unwrap())
        .collect();
    let out = execute_block(&state, &ctx(2), &calls).unwrap();
    assert!(out.receipts.iter().all(|r| r.success && r.prove_gas > 0));
    assert_eq!(out.state.storage(&counter, U256::ZERO), U256::from(2));
    let w = &out.bal.accounts.iter().find(|x| x.address == counter).unwrap().writes;
    assert_eq!(w.len(), 1, "slot 0 written");
    assert_eq!(w[0].1, 1, "last writer is tx #1");
}

#[test]
fn proposer_and_validator_agree_and_are_deterministic() {
    let s1 = P256Signer::from_seed(&seed(4)).unwrap();
    let s2 = Secp256k1Signer::from_seed(&seed(5)).unwrap();
    let pre = funded(&[&s1, &s2]);
    let txs = vec![
        sign_call(&s1, CHAIN, 0, 1, &transfer(Address::repeat_byte(9), 7)).unwrap(),
        sign_call(&s2, CHAIN, 0, 3, &transfer(Address::repeat_byte(9), 11)).unwrap(),
        sign_call(&s1, CHAIN, 1, 1, &transfer(Address::repeat_byte(8), 1)).unwrap(),
    ];
    let (included, built) = build_block(&pre, &ctx(1), txs.clone());
    assert_eq!(included.len(), 3);
    for _ in 0..3 {
        let v = execute_block(&pre, &ctx(1), &included).unwrap();
        assert_eq!(v.state.root(), built.state.root());
        assert_eq!(v.bal, built.bal);
        assert_eq!(v.receipts, built.receipts);
    }
}

#[test]
fn tampered_tx_invalidates_block() {
    let s = P256Signer::from_seed(&seed(6)).unwrap();
    let pre = funded(&[&s]);
    let mut tx = sign_call(&s, CHAIN, 0, 1, &transfer(Address::repeat_byte(1), 1)).unwrap();
    tx.header.max_fee.exec = 0;
    assert!(matches!(execute_block(&pre, &ctx(1), &[tx]), Err(ExecError::InvalidTx { index: 0, .. })));
}

#[test]
fn post_state_balance_is_provable() {
    let s = P256Signer::from_seed(&seed(7)).unwrap();
    let pre = funded(&[&s]);
    let bob = Address::repeat_byte(0x42);
    let out = execute_block(&pre, &ctx(1), &[sign_call(&s, CHAIN, 0, 1, &transfer(bob, 123)).unwrap()]).unwrap();
    let repo = out.state.repo();
    let proof = repo.prove(&[basic_data_key(repo.hasher(), &bob)]).remove(0);
    proof.verify(repo.hasher(), &repo.root()).unwrap();
    let data = aether_state::layout::BasicData::decode(&proof.value.unwrap());
    assert_eq!(data.balance, 123);
}
