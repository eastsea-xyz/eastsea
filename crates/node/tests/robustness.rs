//! Robustness of everything that parses or checks untrusted bytes
//! (docs/design/12-launch-plan.md step 2): peers' blocks, RPC transactions,
//! payloads, and the proofs and certificates a wallet is handed.
//!
//! Random and mutated inputs must never panic, and any change to a valid
//! signed or proven object must be rejected. Deterministic (fixed seed);
//! `AETHER_FUZZ_ITERS` raises the iteration count for long runs.

use aether_crypto::P256Signer;
use aether_execution::{execute_block, sign_call, validate_stateless, BlockContext, EvmCall, WorldState};
use aether_light::block::Payload;
use aether_light::{verify_finalized, ValidatorSet};
use aether_state::layout::basic_data_key;
use aether_state::StateRepository;
use aether_types::{Address, Bytes, GasVector, TxEnvelope, U256};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha20Rng;

const CHAIN: u64 = 7_777;

fn iters(default: usize) -> usize {
    std::env::var("AETHER_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(0xae7e_0001)
}

fn random_bytes(r: &mut ChaCha20Rng, max: usize) -> Vec<u8> {
    let n = r.random_range(0..=max);
    (0..n).map(|_| r.random()).collect()
}

/// Flip, insert, delete or overwrite a few bytes.
fn mutate(r: &mut ChaCha20Rng, v: &[u8]) -> Vec<u8> {
    let mut out = v.to_vec();
    for _ in 0..r.random_range(1..=3) {
        match r.random_range(0..4) {
            0 if !out.is_empty() => {
                let i = r.random_range(0..out.len());
                out[i] ^= 1 << r.random_range(0..8);
            }
            1 => {
                let i = r.random_range(0..=out.len());
                out.insert(i, r.random());
            }
            2 if !out.is_empty() => {
                let i = r.random_range(0..out.len());
                out.remove(i);
            }
            _ if !out.is_empty() => {
                let i = r.random_range(0..out.len());
                out[i] = r.random();
            }
            _ => out.push(r.random()),
        }
    }
    out
}

fn signed_tx() -> TxEnvelope {
    let s = P256Signer::from_seed(&[7u8; 32]).unwrap();
    let call = EvmCall { to: Some(Address::repeat_byte(1)), value: U256::from(5u64), input: Bytes::from_static(b"hello"), gas_limit: 50_000, delegate: None };
    sign_call(&s, CHAIN, 3, 1_000_000_000, &call).unwrap()
}

#[test]
fn call_decoder_never_panics_and_round_trips() {
    let mut r = rng();
    let valid = EvmCall {
        to: Some(Address::repeat_byte(9)),
        value: U256::from(1u64),
        input: Bytes::from_static(&[1, 2, 3]),
        gas_limit: 21_000,
        delegate: Some(Address::repeat_byte(7)),
    }
    .encode();
    for i in 0..iters(20_000) {
        let input = if i % 2 == 0 { random_bytes(&mut r, 256) } else { mutate(&mut r, &valid) };
        if let Ok(call) = EvmCall::decode(&input) {
            // Canonical: whatever decodes re-encodes to the same bytes.
            assert_eq!(call.encode(), input, "non-canonical call encoding accepted");
        }
    }
}

#[test]
fn any_change_to_a_signed_tx_is_rejected() {
    let mut r = rng();
    let tx = signed_tx();
    assert!(validate_stateless(&tx, CHAIN).is_ok());
    let json = serde_json::to_vec(&tx).unwrap();
    let mut decoded = 0;
    for _ in 0..iters(10_000) {
        let bytes = mutate(&mut r, &json);
        let Ok(t) = serde_json::from_slice::<TxEnvelope>(&bytes) else { continue };
        decoded += 1;
        if t != tx {
            assert!(validate_stateless(&t, CHAIN).is_err(), "a modified transaction still validates: {t:?}");
        }
    }
    assert!(decoded > 0, "mutations never produced a parseable tx; the test checks nothing");
    // Field-level changes a relay could make.
    let mut t = tx.clone();
    t.header.tip += 1;
    assert!(validate_stateless(&t, CHAIN).is_err(), "tip is not signed");
    let mut t = tx.clone();
    t.header.gas.prove += 1;
    assert!(validate_stateless(&t, CHAIN).is_err(), "prove budget is not signed");
    let mut t = tx.clone();
    t.header.max_fee.prove += 1;
    assert!(validate_stateless(&t, CHAIN).is_err(), "prove fee cap is not signed");
    assert!(validate_stateless(&tx, CHAIN + 1).is_err(), "replay on another chain");
}

#[test]
fn payload_decoder_never_panics() {
    let mut r = rng();
    let valid = Payload { txs: vec![signed_tx()], ..Default::default() }.to_bytes();
    for i in 0..iters(5_000) {
        let input = if i % 2 == 0 { random_bytes(&mut r, 512) } else { mutate(&mut r, &valid) };
        let _ = Payload::from_bytes(&input);
    }
}

#[test]
fn light_client_rejects_garbage_certificates() {
    let mut r = rng();
    let set = ValidatorSet::devnet(4);
    for _ in 0..iters(3_000) {
        let block = random_bytes(&mut r, 300);
        let cert = random_bytes(&mut r, 300);
        assert!(verify_finalized(&set, &block, &cert).is_err());
    }
}

#[test]
fn any_change_to_a_state_proof_is_rejected() {
    let mut r = rng();
    let s = P256Signer::from_seed(&[8u8; 32]).unwrap();
    let mut pre = WorldState::default();
    pre.set_balance(aether_crypto::address_of(&aether_crypto::Signer::public_key(&s)).unwrap(), U256::from(10u128.pow(20))).unwrap();
    let ctx = BlockContext {
        chain_id: CHAIN,
        number: 1,
        timestamp: 1,
        beneficiary: Address::repeat_byte(0xbe),
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        fees: None,
    };
    let bob = Address::repeat_byte(0x42);
    let call = EvmCall { to: Some(bob), value: U256::from(123u64), input: Bytes::new(), gas_limit: 21_000, delegate: None };
    let out = execute_block(&pre, &ctx, &[sign_call(&s, CHAIN, 0, 1, &call).unwrap()]).unwrap();
    let repo = out.state.repo();
    let root = repo.root();
    let proof = repo.prove(&[basic_data_key(repo.hasher(), &bob)]).remove(0);
    proof.verify(repo.hasher(), &root).unwrap();
    let json = serde_json::to_vec(&proof).unwrap();
    let mut decoded = 0;
    for _ in 0..iters(5_000) {
        let bytes = mutate(&mut r, &json);
        let Ok(p) = serde_json::from_slice::<aether_state::Proof>(&bytes) else { continue };
        decoded += 1;
        if p != proof {
            assert!(p.verify(repo.hasher(), &root).is_err(), "a modified proof still verifies");
        }
    }
    assert!(decoded > 0);
}
