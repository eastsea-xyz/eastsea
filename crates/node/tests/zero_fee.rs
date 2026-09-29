//! G2 (docs/design/22-gas-pool.md): a zero-balance account transacts while
//! the base fee is 0 (the wallet sends tip 0, cap 0), and a zero-balance Mac
//! registers through the block's free lane even while the base fee is above
//! 0 — checked against the same registrar attestation, sharing the registry
//! contract's words and its per-epoch cap.

mod common;

use aether_crypto::{P256Signer, Signer as _};
use aether_execution::registry::{self, MAX_FREE_PER_BLOCK, REGISTRY};
use aether_execution::{sign_call_with, EvmCall};
use aether_light::block::NodeRegistration;
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{build_payload, Chain, ChainError, Extras};
use aether_node::upgrade::{Release, SignedUpgrade, Upgrade};
use aether_types::{Address, FeeVector, TxEnvelope, U256};
use commonware_codec::Encode as _;
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible as _, Signer as _};
use common::{addr, Net, Opts};
use std::sync::Arc;

const CHAIN: u64 = 7_795;
const E: u64 = 12;

fn net(macs: u8) -> Net {
    Net::new(Opts { chain_id: CHAIN, node_rewards: true, epoch_blocks: E, macs, min_streak: Some(0), history_v2: false, reserve: None, fees: true, committee: None })
}

fn signed(net: &Net, protocol: u32, activate_at: u64) -> SignedUpgrade {
    let u = Upgrade {
        chain_id: CHAIN,
        protocol,
        activate_at,
        releases: vec![Release { platform: "macos-arm64-dmg".into(), version: "0.7.0".into(), blake3: "ab".repeat(32), url: "https://x".into() }],
        notes: String::new(),
        registrar: None,
    };
    net.committee.sign_upgrade(&u)
}

/// A wallet with no balance on this chain (not one of the funded operators).
fn broke(seed: u8) -> P256Signer {
    P256Signer::from_seed(&common::seed(seed)).unwrap()
}

fn transfer(to: Address, value: U256) -> EvmCall {
    EvmCall { to: Some(to), value, input: Default::default(), gas_limit: 21_000, delegate: None }
}

/// The zero-tip, zero-cap envelope the wallet builds at a zero base fee (ffi
/// `fee_caps` with no balance).
fn free_call(s: &P256Signer, nonce: u64, call: &EvmCall) -> TxEnvelope {
    sign_call_with(s, CHAIN, nonce, FeeVector::default(), 0, call).unwrap()
}

/// `build_with` without the harness's every-tx-succeeds assert: a block whose
/// tx is allowed to revert (the contract path refusing at the epoch cap).
fn raw_build(net: &Net, txs: Vec<TxEnvelope>, registrations: Vec<NodeRegistration>) -> Result<(Block, Arc<aether_node::chain::Executed>), ChainError> {
    let (chain, parent) = (&net.chain, &net.parent);
    let height = net.last.height.next();
    let leader = ed25519::PrivateKey::from_seed(1).public_key();
    let context = Context { round: Round::new(EPOCH, View::new(height.get())), leader, parent: (View::new(height.get() - 1), net.last.digest()) };
    let ts = height.get() * 1_000;
    let skeleton = Block::new(context.clone(), net.last.digest(), height, ts, bytes::Bytes::new());
    let ctx = Chain::block_context(&chain.cfg(), &skeleton, parent);
    let (pre, _) = chain.pre_state_with(parent, parent.next_protocol(), &[], &[], &registrations, None, false)?;
    let (payload, _) = build_payload(parent, &pre, &ctx, txs, Extras { registrations, ..Default::default() });
    drop(pre);
    let block = Block::new(context, net.last.digest(), height, ts, payload.to_bytes());
    let exec = chain.execute(&block, parent)?;
    Ok((block, exec))
}

/// The error out of a build that must fail (the Ok side carries a non-Debug
/// `Executed`, so `unwrap_err` cannot be used).
fn err_of(r: Result<(Block, Arc<aether_node::chain::Executed>), ChainError>) -> ChainError {
    match r {
        Ok(_) => panic!("the block should not have been valid"),
        Err(e) => e,
    }
}

/// Build through the proposer's own registration path (pool →
/// `registrations_for`), execute and finalize.
fn step_lane(net: &mut Net, txs: Vec<TxEnvelope>) -> Arc<aether_node::chain::Executed> {
    let regs = net.chain.registrations_for(&net.parent);
    let (block, exec) = net.build_with(txs, None, vec![], vec![], regs, None).unwrap();
    net.chain.finalize(&block).unwrap();
    net.parent = exec.clone();
    net.last = block;
    exec
}

/// One near-full block of transfers from a funded operator: 1428 × 21k ≈ 30M
/// gas, twice the 15M target — free while it runs (the base fee was 0), and
/// it leaves the excess that makes the next base fee ~1 gwei.
fn congest(n: &mut Net) {
    let to = n.operator(0);
    let fill: Vec<TxEnvelope> = (0..1_428u64)
        .map(|i| free_call(&n.ops[0], i, &transfer(to, U256::ZERO)))
        .collect();
    n.step(fill, None, vec![]);
    assert!(Chain::next_base_fee(&n.chain.cfg(), &n.parent).exec > 0, "one block past target costs ~1 gwei");
}

#[test]
fn a_zero_balance_zero_tip_tx_is_free_and_included_below_target() {
    let mut n = net(1);
    n.run_to(2);
    // Below target load the base fee is 0: nobody needs a balance at all.
    assert_eq!(Chain::next_base_fee(&n.chain.cfg(), &n.parent).exec, 0);

    let (stranger, op) = (broke(99), n.operator(0));
    let free = free_call(&stranger, 0, &transfer(op, U256::ZERO));
    assert!(n.chain.add_to_mempool(free.clone()).unwrap(), "a zero-balance zero-tip tx is admissible at base fee 0");
    // A tipped tx from a funded account: both go in one block — the proposer
    // orders by (nonce, listed, sender), never by fee, so the free tx is not
    // starved behind paid ones (docs/design/22-gas-pool.md 1층).
    let tipped = sign_call_with(&n.ops[0], CHAIN, 0, FeeVector { exec: 1_000_000_000, state: 0, prove: 0 }, 1_000_000_000, &transfer(op, U256::ZERO)).unwrap();
    assert!(n.chain.add_to_mempool(tipped).unwrap());
    let txs = n.chain.mempool_candidates();
    let exec = step_lane(&mut n, txs);
    assert_eq!(exec.tx_hashes.len(), 2, "both the tipped and the zero-tip tx are included");
    assert!(exec.receipts.iter().all(|r| r.success));
    // The stranger paid nothing (there was nothing to pay) and the fee is still 0.
    assert_eq!(exec.state.balance(&addr(&stranger)), U256::ZERO);
    assert_eq!(exec.state.nonce(&addr(&stranger)), 1);
    assert_eq!(Chain::next_base_fee(&n.chain.cfg(), &n.parent).exec, 0, "21k gas is far below target");
}

#[test]
fn congestion_raises_the_base_fee_and_a_zero_balance_is_refused_the_mempool() {
    let mut n = net(1);
    n.run_to(2);
    congest(&mut n);
    let base = Chain::next_base_fee(&n.chain.cfg(), &n.parent);

    // The same zero-tip, zero-cap tx no longer fits: its caps are below the
    // raised base fee. That is what the free registration lane is for.
    let err = n.chain.add_to_mempool(free_call(&broke(99), 0, &transfer(n.operator(0), U256::ZERO))).unwrap_err();
    assert!(err.contains("below the base fee"), "{err}");
    // A tip-0 tx capped at base × 2 (the wallet's zero-balance rule) passes
    // that check but still cannot pay — the mempool says so up front instead
    // of the wallet sending a tx that can never run.
    let capped = sign_call_with(&broke(99), CHAIN, 0, FeeVector { exec: base.exec * 2, state: 0, prove: 0 }, 0, &transfer(n.operator(0), U256::ZERO)).unwrap();
    assert!(n.chain.add_to_mempool(capped).unwrap_err().contains("insufficient funds"));
}

#[test]
fn a_zero_balance_mac_registers_through_the_free_lane_under_congestion() {
    let mut n = net(1);
    n.run_to(2);
    congest(&mut n);

    // A brand-new wallet and Mac: no balance, no nonce. The item is what the
    // app's prepare_register_node → submit_signed produces.
    let (op, voting) = (broke(99), ed25519::PrivateKey::from_seed(99));
    let item = n.lane_registration(&op, &voting, Net::node_id(99), 0, n.parent.height + 600);
    assert!(n.chain.submit_registration(item.clone()).unwrap(), "pooled on first sight");
    assert!(!n.chain.submit_registration(item.clone()).unwrap(), "already pooled");

    let exec = step_lane(&mut n, vec![]);
    let key: [u8; 32] = voting.public_key().encode().as_ref().try_into().unwrap();
    assert_eq!(registry::index_of(&exec.state, &key), 1, "registered");
    let c = &registry::candidates(&exec.state)[0];
    assert_eq!(c.operator, addr(&op));
    assert_eq!(c.validator_key, key);
    // Nothing was paid and no tx was sent: the operator still has no balance.
    assert_eq!(exec.state.balance(&addr(&op)), U256::ZERO);
    assert_eq!(exec.state.nonce(&addr(&op)), 0);
    assert_eq!(registry::lane_nonce(&exec.state, &addr(&op)), 1, "the item is consumed once");

    // The wallet's receipt poll works on the item's id like on a tx hash.
    let id = aether_node::registrations::id(&item);
    let (h, r) = n.chain.lock().receipts[&id].clone();
    assert_eq!((h, r.success, r.gas_used), (exec.height, true, 0));

    // Replaying the consumed item is refused (not pooled, and a block with it
    // is invalid: the nonce is spent).
    assert!(n.chain.submit_registration(item.clone()).unwrap_err().contains("already registered"));
    assert!(n.chain.registrations_for(&n.parent).is_empty(), "the proposer does not carry it again");
    assert!(raw_build(&n, vec![], vec![item]).is_err(), "a block with a spent item is invalid");
}

#[test]
fn invalid_lane_items_make_the_block_invalid() {
    let mut n = net(1);
    n.run_to(2);
    let height = n.parent.height + 1;
    let good = n.lane_registration(&broke(99), &ed25519::PrivateKey::from_seed(99), Net::node_id(99), 0, height + 600);

    // A forged relay signature: not the operator wallet's.
    let mut forged = good.clone();
    let stranger = broke(98);
    let msg = registry::relay_message(CHAIN, forged.operator, &forged.validator_key.0, &forged.node_id.0, forged.beaconer, &forged.attestation, forged.nonce, forged.expiry);
    forged.signature = stranger.sign(&msg).unwrap().into();
    assert!(n.chain.add_registration(forged.clone()).unwrap_err().contains("operator wallet"));
    assert!(raw_build(&n, vec![], vec![forged]).is_err(), "and a block carrying it is invalid");

    // Expired: refused (the expiry is signed, so it cannot be stretched).
    let stale = n.lane_registration(&broke(97), &ed25519::PrivateKey::from_seed(97), Net::node_id(97), 0, height - 1);
    assert!(n.chain.add_registration(stale).unwrap_err().contains("expired"));
    let edge = n.lane_registration(&broke(97), &ed25519::PrivateKey::from_seed(97), Net::node_id(97), 0, height);
    assert!(n.chain.add_registration(edge).is_ok(), "valid in the block its expiry names");

    // Five items in one block: over the per-block bound (4, each two P-256 checks).
    let five: Vec<_> = (90..95u8)
        .map(|i| n.lane_registration(&broke(i), &ed25519::PrivateKey::from_seed(i as u64), Net::node_id(i as usize), 0, height + 600))
        .collect();
    let err = format!("{:?}", err_of(raw_build(&n, vec![], five)));
    assert!(err.contains("more than 4"), "{err}");
    // The good one (and the edge-expiry one) still land.
    assert!(n.chain.add_registration(good).is_ok());
    let exec = step_lane(&mut n, vec![]);
    assert_eq!(registry::candidates(&exec.state).len(), 2, "the edge item and the good one");
}

#[test]
fn the_lane_and_the_contract_write_the_same_words_and_share_the_epoch_cap() {
    // Two identical networks: Mac 1 registers through the paid contract on
    // one, through the free lane on the other, at the same height of the same
    // epoch (protocol 2, so the contract runs its own epoch cap).
    let mut contract = net(2);
    let mut lane = net(2);
    assert_eq!(contract.parent.state.root(), lane.parent.state.root(), "same genesis");
    contract.step(vec![], Some(signed(&contract, 2, 13)), vec![]);
    lane.step(vec![], Some(signed(&lane, 2, 13)), vec![]);
    contract.run_to(12);
    lane.run_to(12);

    let reg = contract.register(1);
    contract.step(vec![reg], None, vec![]);
    let item = lane.lane_registration(&lane.ops[1], &lane.voting[1], Net::node_id(1), 0, lane.parent.height + 600);
    assert!(lane.chain.submit_registration(item).unwrap());
    step_lane(&mut lane, vec![]);

    // The registry's own slots and the candidate's five words are identical:
    // the lane is the contract's register, minus the fee.
    let read = |n: &Net| -> Vec<(u64, U256)> {
        let s = &n.parent.state;
        (0u64..=9).map(|k| (k, s.storage(&REGISTRY, U256::from(k)))).collect()
    };
    assert_eq!(read(&contract), read(&lane), "slots 0..=9 (registrar, count, params, the v2 cap words) agree");
    let c = registry::candidates(&contract.parent.state);
    let l = registry::candidates(&lane.parent.state);
    assert_eq!(c, l, "the same candidate, word for word");
    assert_eq!(c.len(), 1);
    // The one word only the lane writes: the operator's spent-item count.
    assert_eq!(registry::lane_nonce(&contract.parent.state, &c[0].operator), 0);
    assert_eq!(registry::lane_nonce(&lane.parent.state, &l[0].operator), 1);

    // Now the shared cap (16 per epoch), on the lane network: fill the rest
    // through the lane (at most a block's worth, 4, each block), then both
    // paths refuse the 17th.
    let mut filled = registry::reg_count(&lane.parent.state);
    let mut next = 20u8;
    while filled < registry::MAX_PER_EPOCH {
        let take = (registry::MAX_PER_EPOCH - filled).min(MAX_FREE_PER_BLOCK as u64) as u8;
        for i in 0..take {
            let s = next + i;
            let item = lane.lane_registration(&broke(s), &ed25519::PrivateKey::from_seed(s as u64), Net::node_id(s as usize), 0, lane.parent.height + 600);
            assert!(lane.chain.submit_registration(item).unwrap());
        }
        next += take;
        filled += take as u64;
        let exec = step_lane(&mut lane, vec![]);
        assert_eq!(exec.registration_ids.len(), take as usize, "the block carries the whole batch");
        assert_eq!(registry::reg_count(&exec.state), filled);
    }
    let state = &lane.parent.state;
    assert_eq!(registry::reg_count(state), registry::MAX_PER_EPOCH);
    assert_eq!(registry::candidates(state).len() as u64, registry::MAX_PER_EPOCH);

    // The 17th lane item pools fine (its signatures are good) but no proposer
    // can carry it, and a block with it is invalid — the same words refuse it.
    let over = lane.lane_registration(&broke(9), &ed25519::PrivateKey::from_seed(9), Net::node_id(9), 0, lane.parent.height + 2);
    assert!(lane.chain.submit_registration(over.clone()).unwrap());
    assert!(lane.chain.registrations_for(&lane.parent).is_empty(), "the cap, not the pool, refuses it");
    let err = format!("{:?}", err_of(raw_build(&lane, vec![], vec![over.clone()])));
    assert!(err.contains("already took 16 of 16"), "{err}");

    // The contract path refuses too: op 0's paid register reverts with the
    // same cap, counted in the same words.
    let attempt = lane.register(0);
    let (block, exec) = raw_build(&lane, vec![attempt], vec![]).unwrap();
    assert!(!exec.receipts[0].success, "the contract's own cap reverts the 17th too");
    assert_eq!(registry::reg_count(&exec.state), registry::MAX_PER_EPOCH, "a refused registration counts for nothing");
    lane.chain.finalize(&block).unwrap();
    lane.parent = exec.clone();
    lane.last = block;

    // A new epoch resets the count for both paths (the expired 17th is long
    // gone from the pool; this is a fresh Mac's turn).
    lane.run_to(2 * E);
    let fresh = lane.lane_registration(&broke(10), &ed25519::PrivateKey::from_seed(10), Net::node_id(10), 0, lane.parent.height + 600);
    assert!(lane.chain.submit_registration(fresh).unwrap());
    let exec = step_lane(&mut lane, vec![]);
    assert_eq!((registry::reg_epoch(&exec.state), registry::reg_count(&exec.state)), (2, 1));
}
