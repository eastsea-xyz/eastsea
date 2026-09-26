//! FOCIL inclusion lists: signatures, committee, pool freeze and the append check.

use aether_crypto::P256Signer;
use aether_execution::{build_block, sign_call, tx_hash, BlockContext, EvmCall, WorldState};
use aether_node::chain::dev_seed;
use aether_node::inclusion::{committee, violations, IlError, InclusionList, InclusionPool, FREEZE};
use aether_types::{Address, Bytes, GasVector, TxEnvelope, U256};
use commonware_cryptography::{ed25519, Signer as _};
use std::time::{Duration, Instant};

const CHAIN: u64 = 7777;

fn key(i: u64) -> ed25519::PrivateKey {
    aether_light::devnet_validator_key(i)
}

fn validators(n: u64) -> Vec<ed25519::PublicKey> {
    (1..=n).map(|i| key(i).public_key()).collect()
}

fn signer(dev: u8) -> P256Signer {
    P256Signer::from_seed(&dev_seed(dev)).unwrap()
}

fn sender(dev: u8) -> Address {
    aether_crypto::address_of(&aether_crypto::Signer::public_key(&signer(dev))).unwrap()
}

fn transfer(dev: u8, nonce: u64, value: u64) -> TxEnvelope {
    let call = EvmCall { to: Some(Address::repeat_byte(0xbe)), value: U256::from(value), input: Bytes::new(), gas_limit: 21_000, delegate: None };
    sign_call(&signer(dev), CHAIN, nonce, 1, &call).unwrap()
}

fn funded(devs: &[u8]) -> WorldState {
    let mut s = WorldState::default();
    for d in devs {
        s.set_balance(sender(*d), U256::from(10u128.pow(24))).unwrap();
    }
    s
}

fn ctx() -> BlockContext {
    BlockContext {
        chain_id: CHAIN,
        number: 1,
        timestamp: 1,
        beneficiary: Address::ZERO,
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        fees: None,
    }
}

#[test]
fn signed_list_verifies_and_tampering_fails() {
    let il = InclusionList::sign(&key(2), 2, 10, vec![transfer(1, 0, 5)]);
    assert_eq!(il.verify(&validators(4), CHAIN), Ok(()));

    let mut wrong_member = il.clone();
    wrong_member.member = 3;
    assert_eq!(wrong_member.verify(&validators(4), CHAIN), Err(IlError::BadSignature));

    let mut swapped = il.clone();
    swapped.txs = vec![transfer(1, 0, 6)];
    assert_eq!(swapped.verify(&validators(4), CHAIN), Err(IlError::BadSignature));

    let outsider = InclusionList::sign(&key(9), 9, 10, vec![]);
    assert_eq!(outsider.verify(&validators(4), CHAIN), Err(IlError::NotMember));

    let too_many = InclusionList::sign(&key(1), 1, 10, (0..17).map(|n| transfer(1, n, 1)).collect());
    assert_eq!(too_many.verify(&validators(4), CHAIN), Err(IlError::TooManyTxs));
}

#[test]
fn committee_rotates_for_large_sets() {
    assert_eq!(committee(5, 4), vec![1, 2, 3, 4]);
    let a = committee(1, 20);
    let b = committee(2, 20);
    assert_eq!(a.len(), 8);
    assert_ne!(a, b);
    assert!(a.iter().all(|m| (1..=20).contains(m)));
}

#[test]
fn pool_enforces_only_after_freeze_and_dedups() {
    let t0 = Instant::now();
    let mut pool = InclusionPool::default();
    let il = InclusionList::sign(&key(1), 1, 3, vec![transfer(1, 0, 5)]);
    assert!(pool.accept(&il, t0));
    assert!(!pool.accept(&il, t0), "same (member, height) is accepted once");
    assert_eq!(pool.for_proposal().len(), 1);
    assert!(pool.enforceable(t0 + Duration::from_millis(100)).is_empty());
    assert_eq!(pool.enforceable(t0 + FREEZE).len(), 1);

    // Once the finalized state consumed the nonce, the entry is dropped.
    let mut state = funded(&[1]);
    let (_, out) = build_block(&state, &ctx(), vec![transfer(1, 0, 5)]);
    state = out.state;
    pool.prune(&state, 3, t0);
    assert!(pool.is_empty());
}

#[test]
fn append_check_catches_censorship_but_not_invalid_txs() {
    let pre = funded(&[1, 2]);
    let listed = vec![transfer(1, 0, 5), transfer(2, 0, 7)];

    // A block with only dev 1's tx censors dev 2.
    let (txs, out) = build_block(&pre, &ctx(), vec![listed[0].clone()]);
    let hashes: Vec<_> = txs.iter().map(tx_hash).collect();
    assert_eq!(violations(&listed, &hashes, false, &out.state, &ctx(), out.gas), vec![tx_hash(&listed[1])]);

    // Full blocks are exempt.
    assert!(violations(&listed, &hashes, true, &out.state, &ctx(), out.gas).is_empty());

    // Including both satisfies the list.
    let (txs, out) = build_block(&pre, &ctx(), listed.clone());
    let hashes: Vec<_> = txs.iter().map(tx_hash).collect();
    assert!(violations(&listed, &hashes, false, &out.state, &ctx(), out.gas).is_empty());

    // A listed tx that could not execute (unfunded sender) is not a violation.
    let unfunded = vec![transfer(3, 0, 1)];
    let (_, out) = build_block(&pre, &ctx(), vec![]);
    assert!(violations(&unfunded, &[], false, &out.state, &ctx(), out.gas).is_empty());

    // Nor is one whose nonce was already used in this block's ancestry.
    let (_, after) = build_block(&pre, &ctx(), vec![listed[1].clone()]);
    assert!(violations(&listed[1..], &[], false, &after.state, &ctx(), GasVector::default()).is_empty());
}
