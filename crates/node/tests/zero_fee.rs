//! A fresh zero-balance account cannot leave a free nonce record or receipt.
//! A zero-balance Mac still registers through the bounded free lane, even
//! while the execution base fee is above zero.

mod common;

use aether_crypto::{P256Signer, Signer as _};
use aether_execution::registry::{self, MAX_FREE_PER_BLOCK, REGISTRY};
use aether_execution::{sign_call_with, EvmCall};
use aether_light::block::NodeRegistration;
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{build_payload, Chain, ChainError, Extras};
use aether_node::chain::BlockSummary;
use aether_node::store::{Commit, Staged, Store};
use aether_types::{Address, Bytes, FeeVector, TxEnvelope, U256};
use commonware_codec::Encode as _;
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible as _, Signer as _};
use common::{addr, Mac, Net, Opts};
use std::sync::Arc;

const CHAIN: u64 = 7_795;
const E: u64 = 12;

/// Node rewards put these chains under the mainnet rules (`Chain::admissible_upgrade`:
/// an ordinary upgrade needs seven days of notice), so the genesis starts at
/// protocol 2 — the registry v2 code and its own epoch cap, which the lane and
/// the contract share — instead of scheduling that upgrade a few blocks ahead.
fn net(macs: u8) -> Net {
    Net::new(Opts { chain_id: CHAIN, node_rewards: true, epoch_blocks: E, macs, min_streak: Some(0), history_v2: false, protocol: 2, reserve: None, fees: true, committee: None })
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

fn paid_call(s: &P256Signer, nonce: u64, call: &EvmCall, state_units: u64) -> TxEnvelope {
    let mut tx = sign_call_with(
        s, CHAIN, nonce,
        FeeVector { exec: 0, state: aether_execution::fees::STATE_UNIT_PRICE, prove: 0 },
        0, call,
    ).unwrap();
    tx.header.gas.state = state_units;
    let mut sig = s.sign(&tx.signing_bytes()).unwrap();
    sig.extend_from_slice(&s.public_key().bytes);
    tx.signature = Bytes::from(sig);
    tx
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
/// gas, twice the 15M target — zero execution fee but paid receipt bytes, and
/// it leaves the excess that makes the next base fee ~1 gwei.
fn congest(n: &mut Net) {
    let to = n.operator(0);
    let fill: Vec<TxEnvelope> = (0..1_428u64)
        .map(|i| paid_call(&n.ops[0], i, &transfer(to, U256::ZERO), 100))
        .collect();
    n.step(fill, None, vec![]);
    assert!(Chain::next_base_fee(&n.chain.cfg(), &n.parent).exec > 0, "one block past target costs ~1 gwei");
}

#[test]
fn a_fresh_zero_balance_sender_is_rejected_but_existing_accounts_can_pay_only_state_fee() {
    let mut n = net(1);
    n.run_to(2);
    // Below target load the execution base fee is 0. Persistent receipt and
    // first-sender bytes still have a fixed fee.
    assert_eq!(Chain::next_base_fee(&n.chain.cfg(), &n.parent).exec, 0);

    let (stranger, op) = (broke(99), n.operator(0));
    let free = free_call(&stranger, 0, &transfer(op, U256::ZERO));
    assert!(n.chain.add_to_mempool(free.clone()).is_err());
    let ctx = Chain::block_context(&n.chain.cfg(), &n.last, &n.parent);
    assert!(aether_execution::execute_block(&n.parent.state, &ctx, &[free]).is_err());
    let paid = paid_call(&n.ops[0], 0, &transfer(op, U256::ZERO), 100);
    assert!(n.chain.add_to_mempool(paid).unwrap());
    let txs = n.chain.mempool_candidates();
    let exec = step_lane(&mut n, txs);
    assert_eq!(exec.tx_hashes.len(), 1);
    assert!(exec.receipts.iter().all(|r| r.success));
    assert!(exec.receipts[0].state_gas > 0);
    assert!(exec.receipts[0].state_fee > U256::ZERO);
    assert_eq!(exec.state.balance(&addr(&stranger)), U256::ZERO);
    assert_eq!(exec.state.nonce(&addr(&stranger)), 0);
    assert_eq!(Chain::next_base_fee(&n.chain.cfg(), &n.parent).exec, 0, "21k gas is far below target");
}

#[test]
fn mempool_and_block_execution_both_reject_a_zero_balance_state_writer() {
    let mut n = net(1);
    let init = Bytes::from_static(&[
        0x60, 0x06, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, 0x06, 0x60, 0x00, 0xf3,
        0x60, 0x01, 0x60, 0x00, 0x55, 0x00,
    ]);
    let deploy_call = EvmCall { to: None, value: U256::ZERO, input: init, gas_limit: 200_000, delegate: None };
    let mut deploy = sign_call_with(
        &n.ops[0], CHAIN, 0,
        FeeVector { exec: 0, state: aether_execution::fees::STATE_UNIT_PRICE, prove: 0 },
        0, &deploy_call,
    ).unwrap();
    deploy.header.gas.state = 1_000;
    let mut sig = n.ops[0].sign(&deploy.signing_bytes()).unwrap();
    sig.extend_from_slice(&n.ops[0].public_key().bytes);
    deploy.signature = Bytes::from(sig);
    let deployed = n.step(vec![deploy], None, vec![]);
    let writer = deployed.receipts[0].contract_address.unwrap();
    assert!(deployed.receipts[0].state_gas > 106);

    let zero = free_call(&broke(99), 0, &EvmCall { to: Some(writer), value: U256::ZERO, input: Default::default(), gas_limit: 100_000, delegate: None });
    let pool_err = n.chain.add_to_mempool(zero.clone()).unwrap_err();
    assert!(pool_err.contains("state") || pool_err.contains("insufficient funds"), "{pool_err}");
    let future = free_call(&broke(98), 5, &EvmCall { to: Some(writer), value: U256::ZERO, input: Default::default(), gas_limit: 100_000, delegate: None });
    let future_err = n.chain.add_to_mempool(future).unwrap_err();
    assert!(future_err.contains("state") || future_err.contains("insufficient funds"), "{future_err}");
    let ctx = Chain::block_context(&n.chain.cfg(), &n.last, &n.parent);
    let block_err = aether_execution::execute_block(&n.parent.state, &ctx, &[zero]).err().unwrap();
    assert!(format!("{block_err:?}").contains("state") || format!("{block_err:?}").contains("insufficient funds"));
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
    assert_eq!(n.chain.cfg().limits.state, u64::MAX, "genesis config uses the activation sentinel");
    assert_eq!(Chain::block_context(&n.chain.cfg(), &n.last, &n.parent).limits.state, aether_execution::fees::MAX_STATE_UNITS_PER_BLOCK);
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
fn a_zero_balance_operator_cannot_register_by_contract_on_a_state_fee_chain() {
    let mut n = net(1);
    let op = broke(99);
    let voting = ed25519::PrivateKey::from_seed(99);
    let item = n.lane_registration(&op, &voting, Net::node_id(99), 0, n.parent.height + 600);
    let att: [u8; 64] = item.attestation.as_ref().try_into().unwrap();
    let call = EvmCall {
        to: Some(REGISTRY), value: U256::ZERO,
        input: registry::encode_register(item.validator_key.0, item.node_id.0, item.beaconer, att[..32].try_into().unwrap(), att[32..].try_into().unwrap()),
        gas_limit: 400_000, delegate: None,
    };
    let direct = free_call(&op, 0, &call);
    let err = n.chain.add_to_mempool(direct).unwrap_err();
    assert!(err.contains("state") || err.contains("insufficient funds"), "{err}");
    assert_eq!(registry::candidates(&n.parent.state).len(), 0);

    assert!(n.chain.submit_registration(item).unwrap());
    let exec = step_lane(&mut n, vec![]);
    assert_eq!(registry::candidates(&exec.state).len(), 1);
    assert_eq!(exec.state.balance(&addr(&op)), U256::ZERO);
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
    // epoch (the genesis is protocol 2, so the contract runs its own epoch cap).
    let mut contract = net(2);
    let mut lane = net(2);
    assert_eq!(contract.parent.state.root(), lane.parent.state.root(), "same genesis");
    assert_eq!(Chain::block_context(&lane.chain.cfg(), &lane.last, &lane.parent).limits.state, aether_execution::fees::MAX_STATE_UNITS_PER_BLOCK);
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

/// Current-nonce admission uses the executor's exact state and receipt cost,
/// across many calldata lengths, recipient kinds and signed budgets.
#[test]
fn randomized_admission_matches_single_tx_execution_cost() {
    let n = net(8);
    let ctx = Chain::block_context(&n.chain.cfg(), &n.last, &n.parent);
    let mut random = 0x6a09_e667_f3bc_c908u64;
    for i in 0..80usize {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        let sender = i % n.ops.len();
        let to = if random & 1 == 0 { n.operator((sender + 1) % n.ops.len()) } else { Address::repeat_byte(i as u8 + 20) };
        let input = vec![(random >> 8) as u8; (random as usize >> 16) % 48];
        let call = EvmCall { to: Some(to), value: U256::from((random >> 3) & 1), input: Bytes::from(input), gas_limit: 70_000, delegate: None };
        let budget = [0, 8, 16, 32, 120, 200][i % 6];
        let tx = paid_call(&n.ops[sender], 0, &call, budget);
        let admitted = n.chain.add_to_mempool(tx.clone());
        let executed = aether_execution::execute_block(&n.parent.state, &ctx, std::slice::from_ref(&tx));
        assert_eq!(matches!(admitted, Ok(true)), executed.is_ok(), "case {i}: budget {budget}, recipient {to}");
        if let Ok(out) = executed {
            let cost = aether_execution::check_admission_cost(&n.parent.state, &ctx, &tx).unwrap();
            assert_eq!(cost.gas.state, out.receipts[0].state_gas, "state units for case {i}");
            assert_eq!(cost.persistent_bytes, out.persistent_bytes, "persisted bytes for case {i}");
            assert_eq!(cost.gas, out.gas, "gas vector for case {i}");
        }
    }
}

#[test]
fn contiguous_future_nonce_admission_matches_its_executed_prefix_cost() {
    let n = net(1);
    let ctx = Chain::block_context(&n.chain.cfg(), &n.last, &n.parent);
    let call = transfer(n.operator(0), U256::ZERO);
    let first = paid_call(&n.ops[0], 0, &call, 100);
    let second = paid_call(&n.ops[0], 1, &call, 100);
    let first_cost = aether_execution::check_admission_cost(&n.parent.state, &ctx, &first).unwrap();
    assert!(n.chain.add_to_mempool(first.clone()).unwrap());
    assert!(n.chain.add_to_mempool(second.clone()).unwrap());
    let admitted = aether_execution::check_admission_cost(&n.parent.state, &ctx, &second).unwrap();
    let executed = aether_execution::execute_block(&n.parent.state, &ctx, &[first, second]).unwrap();
    assert_eq!(executed.receipts.len(), 2);
    assert_eq!(admitted.gas.state, executed.receipts[1].state_gas);
    assert_eq!(admitted.persistent_bytes, executed.persistent_bytes - first_cost.persistent_bytes);
    assert_eq!(admitted.gas.exec, executed.receipts[1].gas_used);
    assert_eq!(admitted.gas.prove, executed.receipts[1].prove_gas);
}

#[test]
fn mixed_registration_beacon_and_paid_tx_preserve_admission_cost() {
    // A 48-block epoch has live beacon slots; the shorter registration tests
    // use 12-block epochs, which intentionally disable beacon scheduling.
    let mut n = Net::new(Opts {
        chain_id: CHAIN, node_rewards: true, epoch_blocks: 48, macs: 1,
        min_streak: Some(0), history_v2: false, protocol: 2,
        reserve: None, fees: true, committee: None,
    });
    let first = n.lane_registration(&broke(99), &n.voting[0], Net::node_id(99), 0, 600);
    assert!(n.chain.submit_registration(first).unwrap());
    step_lane(&mut n, vec![]);
    n.behaviour.insert(0, Mac::Honest);
    while n.answers().is_empty() && n.parent.height < 96 {
        n.step(vec![], None, vec![]);
    }
    let answers = n.answers();
    assert!(!answers.is_empty(), "a registered Mac reaches a beacon answer slot");
    let new_reg = n.lane_registration(&broke(98), &ed25519::PrivateKey::from_seed(98), Net::node_id(98), 0, n.parent.height + 600);
    let tx = paid_call(&n.ops[0], 0, &transfer(n.operator(0), U256::ZERO), 100);
    let ctx = Chain::block_context(&n.chain.cfg(), &n.last, &n.parent);
    let admission = aether_execution::check_admission_cost(&n.parent.state, &ctx, &tx).unwrap();
    assert!(n.chain.add_to_mempool(tx.clone()).unwrap());
    let (pre, _) = n.chain.pre_state_with(&n.parent, n.parent.next_protocol(), &[], &answers, std::slice::from_ref(&new_reg), None, false).unwrap();
    let direct = aether_execution::execute_block(&pre, &ctx, std::slice::from_ref(&tx)).unwrap();
    assert_eq!(admission.gas.state, direct.receipts[0].state_gas);
    assert_eq!(admission.persistent_bytes, direct.persistent_bytes);
    let (block, exec) = n.build_with(vec![tx], None, vec![], answers, vec![new_reg], None).unwrap();
    assert_eq!(exec.receipts[0].state_gas, admission.gas.state);
    assert_eq!(exec.persistent_bytes, admission.persistent_bytes);
    assert_eq!(exec.registration_ids.len(), 1);
    n.chain.finalize(&block).unwrap();
}

fn logger_init(records: usize) -> Bytes {
    let mut runtime = Vec::with_capacity(records * 6 + 1);
    for _ in 0..records {
        runtime.extend_from_slice(&[0x61, 0x10, 0x00, 0x60, 0x00, 0xa0]); // LOG0(0, 4096)
    }
    runtime.push(0x00);
    let len = u16::try_from(runtime.len()).unwrap();
    let mut init = vec![
        0x61, (len >> 8) as u8, len as u8,
        0x61, 0x00, 0x0f,
        0x60, 0x00, 0x39,
        0x61, (len >> 8) as u8, len as u8,
        0x60, 0x00, 0xf3,
    ];
    init.extend_from_slice(&runtime);
    Bytes::from(init)
}

/// The audit's 400 × 4096-byte LOG0 call must reserve receipt bytes before it
/// can enter the pool or appear in a finalized receipt.
#[test]
fn audit_logger_cannot_persist_receipts_for_free() {
    let mut n = net(1);
    let deploy = paid_call(
        &n.ops[0], 0,
        &EvmCall { to: None, value: U256::ZERO, input: logger_init(400), gas_limit: 1_000_000, delegate: None },
        8_000,
    );
    let deployed = n.step(vec![deploy], None, vec![]);
    let logger = deployed.receipts[0].contract_address.unwrap();
    let call = EvmCall { to: Some(logger), value: U256::ZERO, input: Bytes::new(), gas_limit: 14_000_000, delegate: None };
    let free = free_call(&n.ops[0], 1, &call);
    let err = n.chain.add_to_mempool(free).unwrap_err();
    assert!(err.contains("state growth"), "{err}");

    let first = paid_call(&n.ops[0], 1, &call, 60_000);
    assert!(n.chain.add_to_mempool(first.clone()).unwrap());
    let (block, exec) = n.build_with(vec![first], None, vec![], vec![], vec![], None).unwrap();
    assert_eq!(exec.receipts.len(), 1);
    assert_eq!(exec.receipts[0].events.len(), 400);
    assert!(exec.receipts[0].state_gas >= 1_638_400 / 32);
    assert!(exec.persistent_bytes <= aether_execution::fees::MAX_PERSISTENT_BYTES_PER_BLOCK);
    n.chain.finalize(&block).unwrap();
}

/// A nearly full LOG0 block is the worst case for JSON hex expansion. Measure
/// the key/value bytes actually committed by redb, including activity indexes.
#[test]
fn log_heavy_block_stored_rows_fit_the_pinned_expansion_bound() {
    assert!(aether_execution::fees::MAX_PERSISTENT_BYTES_PER_BLOCK
        * aether_execution::fees::MAX_STORED_BYTES_PER_METERED_BYTE
        <= 32 * 1024 * 1024);
    let mut n = net(1);
    let deploy = paid_call(
        &n.ops[0], 0,
        &EvmCall { to: None, value: U256::ZERO, input: logger_init(480), gas_limit: 1_000_000, delegate: None },
        8_000,
    );
    let deployed = n.step(vec![deploy], None, vec![]);
    let logger = deployed.receipts[0].contract_address.unwrap();
    let call = EvmCall { to: Some(logger), value: U256::ZERO, input: Bytes::new(), gas_limit: 16_777_216, delegate: None };
    let tx = paid_call(&n.ops[0], 1, &call, 90_000);
    let ctx = Chain::block_context(&n.chain.cfg(), &n.last, &n.parent);
    aether_execution::check_admission_cost(&n.parent.state, &ctx, &tx).expect("near-cap logger must be individually valid");
    let (block, exec) = n.build_with(vec![tx.clone()], None, vec![], vec![], vec![], None).unwrap();
    assert_eq!(exec.receipts[0].events.len(), 480);
    assert!(exec.persistent_bytes > 1_900_000, "exercise a near-cap block");

    let dir = std::env::current_dir().unwrap().join("tmp").join(format!("m2-stored-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("state.redb")).unwrap();
    let before = store.stats().unwrap();
    let receipt = &exec.receipts[0];
    let history = aether_node::account_history::transaction(&tx, receipt, block.height.get(), 0, block.timestamp, false, true);
    let summary = BlockSummary {
        height: block.height.get(), hash: hex::encode(block.digest()), parent: hex::encode(n.last.digest()),
        timestamp_ms: block.timestamp, proposer: n.operator(0), state_root: exec.state.root(),
        parent_state_root: n.parent.state.root(), txs: exec.tx_hashes.clone(), gas_used: exec.gas.exec,
        prove_gas: exec.gas.prove, base_fee: FeeVector::default(), excess: Default::default(),
    };
    let diff = exec.state.journal();
    let staged = block.encode();
    store.commit_with_history(Commit {
        height: summary.height, digest: [7; 32], root: exec.state.root(), diff: &diff,
        summary: &summary, receipts: vec![(exec.tx_hashes[0], receipt)], handoff: None,
        seed: None, history: &exec.history, schedule: &exec.schedule, upgrade_notices: &[],
        statement: &exec.statement, staged: Some(Staged { block: &staged, era_start: None }),
    }, &history, false).unwrap();
    let after = store.stats().unwrap();
    let stored = ["receipts", "account_history", "account_block_keys", "blocks", "era_blocks"].iter().map(|name| {
        let old = before.tables.iter().find(|t| t.name == *name).unwrap().stored;
        after.tables.iter().find(|t| t.name == *name).unwrap().stored - old
    }).sum::<u64>();
    eprintln!("M2 log-heavy stored={stored} metered={} ratio={:.3}", exec.persistent_bytes, stored as f64 / exec.persistent_bytes as f64);
    assert!(stored <= aether_execution::fees::MAX_PAID_STORED_BYTES_PER_BLOCK);
    assert!(stored <= exec.persistent_bytes * aether_execution::fees::MAX_STORED_BYTES_PER_METERED_BYTE,
        "stored {stored} vs metered {}", exec.persistent_bytes);
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Two 280 × 4096-byte LOG0 calls fit both gas limits but exceed the 2 MiB
/// persistent-byte cap. Proposer exclusion and validator rejection agree.
#[test]
fn receipt_byte_cap_binds_before_gas_caps_for_proposer_and_validator() {
    let mut n = net(1);
    let deploy = paid_call(
        &n.ops[0], 0,
        &EvmCall { to: None, value: U256::ZERO, input: logger_init(280), gas_limit: 1_000_000, delegate: None },
        8_000,
    );
    let deployed = n.step(vec![deploy], None, vec![]);
    let logger = deployed.receipts[0].contract_address.unwrap();
    let call = EvmCall { to: Some(logger), value: U256::ZERO, input: Bytes::new(), gas_limit: 10_000_000, delegate: None };
    let first = paid_call(&n.ops[0], 1, &call, 45_000);
    let second = paid_call(&n.ops[0], 2, &call, 45_000);
    let ctx = Chain::block_context(&n.chain.cfg(), &n.last, &n.parent);
    let first_out = aether_execution::execute_block(&n.parent.state, &ctx, std::slice::from_ref(&first)).unwrap();
    let second_cost = aether_execution::check_admission_cost(&first_out.state, &ctx, &second).unwrap();
    assert!(first_out.gas.state + second_cost.gas.state < ctx.limits.state);
    assert!(first_out.gas.exec + second_cost.gas.exec < ctx.limits.exec);
    assert!(first_out.persistent_bytes + second_cost.persistent_bytes > aether_execution::fees::MAX_PERSISTENT_BYTES_PER_BLOCK);
    let (_, proposal) = raw_build(&n, vec![first.clone(), second.clone()], vec![]).unwrap();
    assert_eq!(proposal.receipts.len(), 1, "proposer omits the over-cap receipt");
    let ctx = Chain::block_context(&n.chain.cfg(), &n.last, &n.parent);
    assert!(aether_execution::execute_block(&n.parent.state, &ctx, &[first, second]).is_err(), "validator rejects the over-cap pair");
}
