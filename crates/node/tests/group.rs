//! Consensus groups (docs/design/13-roadmap.md, 그룹 분열 준비): a chain of
//! group G runs only group-G transactions in group-G blocks, the group is part
//! of what a tx's sender signs, and group 0 (every network today) keeps the
//! 7780 bytes and rules exactly as they were.

use aether_crypto::{address_of, P256Signer, Signer};
use aether_execution::{sign_call, sign_call_group, EvmCall};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{build_payload, Chain, ChainConfig, ChainError, Executed, Extras};
use aether_types::{Address, Bytes, GasVector, TxEnvelope, U256};
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{Digestible, Signer as _};
use std::sync::Arc;

/// A group-3 chain, as a group split would create (a new genesis of its own).
fn cfg() -> ChainConfig {
    ChainConfig {
        chain_id: 7_799,
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        alloc: vec![(sender(), U256::from(10u128.pow(20)))],
        fees: false,
        registrar: None,
        epoch_blocks: 0,
        min_streak: None,
        draw_epochs: None,
        history_v2: true,
        protocol: 1,
        node_rewards: true,
        committee: vec![],
        reserve: None,
        group: 3,
        max_committee: aether_node::rotation::GROW_UNTIL,
    }
}

fn signer() -> P256Signer {
    P256Signer::from_seed(&[0x5a; 32]).unwrap()
}

fn sender() -> Address {
    address_of(&signer().public_key()).unwrap()
}

fn transfer(value: u64) -> EvmCall {
    EvmCall { to: Some(Address::repeat_byte(0xaa)), value: U256::from(value), input: Bytes::new(), gas_limit: 21_000, delegate: None }
}

/// Build the block after `last` on `chain` (not executed): `extras.group` is
/// the group its payload carries, `txs` what a proposer would offer it.
fn build(chain: &Chain, parent: &Arc<Executed>, last: &Block, txs: Vec<TxEnvelope>, group: u16, foreign: Option<TxEnvelope>) -> Block {
    let height = last.height.next();
    let leader = commonware_cryptography::ed25519::PrivateKey::from_seed(1).public_key();
    let context = Context { round: Round::new(EPOCH, View::new(height.get())), leader, parent: (View::new(height.get() - 1), last.digest()) };
    let skeleton = Block::new(context.clone(), last.digest(), height, height.get() * 1_000, bytes::Bytes::new());
    let ctx = Chain::block_context(&chain.cfg(), &skeleton, parent);
    let (pre, _) = chain.pre_state(parent, parent.next_protocol(), &[], None, false).unwrap();
    let (mut payload, _) = build_payload(parent, &pre, &ctx, txs, Extras { group, ..Default::default() });
    // The proposer filters other groups' txs out; a faulty one puts them back.
    if let Some(tx) = foreign {
        payload.txs.push(tx);
    }
    Block::new(context, last.digest(), height, height.get() * 1_000, payload.to_bytes())
}

#[test]
fn wrong_group_tx_is_refused_by_the_pool_and_by_blocks() {
    let (chain, genesis) = Chain::new(cfg());
    let parent = chain.lock().finalized.clone();

    // A group-3 tx (signed under the v3 tag) is admitted; a group-0 or group-1
    // tx can never run in this chain's blocks, so the pool refuses it outright.
    let ours = sign_call_group(&signer(), 7_799, 0, 1, 3, &transfer(2)).unwrap();
    assert_eq!(ours.header.group(), 3);
    assert_eq!(chain.add_to_mempool(ours.clone()), Ok(true));
    for tx in [
        sign_call(&signer(), 7_799, 5, 1, &transfer(1)).unwrap(),          // group 0
        sign_call_group(&signer(), 7_799, 6, 1, 1, &transfer(1)).unwrap(), // group 1
    ] {
        let err = chain.add_to_mempool(tx).unwrap_err();
        assert!(err.contains("group-3 chain"), "{err}");
    }

    // A block of this group runs its tx (the v3 signature checked on the way
    // in); a block of another group, or one carrying another group's tx, is
    // refused before it executes anything.
    let ok = build(&chain, &parent, &genesis, vec![ours.clone()], 3, None);
    let exec = chain.execute(&ok, &parent).unwrap();
    assert_eq!(exec.tx_hashes.len(), 1, "the group-3 tx ran");
    chain.finalize(&ok).unwrap();
    assert_eq!(chain.lock().mempool.len(), 0, "the block's tx left the pool");

    assert!(
        matches!(chain.execute(&build(&chain, &parent, &genesis, vec![], 0, None), &parent), Err(ChainError::WrongGroup)),
        "a group-0 block on a group-3 chain"
    );
    let foreign = sign_call_group(&signer(), 7_799, 6, 1, 1, &transfer(1)).unwrap();
    assert!(
        matches!(chain.execute(&build(&chain, &parent, &genesis, vec![], 3, Some(foreign)), &parent), Err(ChainError::WrongGroup)),
        "a group-3 block carrying a group-1 tx"
    );

    // The same chain's blocks carry the group in their payload (the digest
    // commits to it), while a group-0 chain's keep the 7780 bytes.
    let payload = ok.payload().unwrap();
    assert_eq!(payload.group, 3);
    assert!(String::from_utf8_lossy(&ok.data).contains("\"group\":3"));
}

#[test]
fn a_group_zero_chain_keeps_the_7780_rules() {
    let (chain, genesis) = Chain::new(ChainConfig { chain_id: 7780, group: 0, history_v2: false, node_rewards: false, ..cfg() });
    assert_eq!(genesis, Block::genesis(7780, chain.lock().finalized.state.root()));
    assert!(chain.lock().finalized.state.code(&aether_rewards::REWARDS).is_empty(), "7780 has no randomness predeploy");
    let parent = chain.lock().finalized.clone();
    // Every 7780 tx (no group field) is a group-0 tx: it signs the v2 bytes
    // and runs exactly as before.
    let tx = sign_call(&signer(), 7780, 0, 1, &transfer(1)).unwrap();
    assert_eq!(tx.header.group(), 0);
    assert_eq!(chain.add_to_mempool(tx.clone()), Ok(true));
    let block = build(&chain, &parent, &genesis, vec![tx], 0, None);
    assert_eq!(chain.execute(&block, &parent).unwrap().tx_hashes.len(), 1);
    // A group-3 tx does not run on the 7780-format chain.
    let foreign = sign_call_group(&signer(), 7780, 1, 1, 3, &transfer(1)).unwrap();
    let err = chain.add_to_mempool(foreign.clone()).unwrap_err();
    assert!(err.contains("group 3"), "{err}");
    assert!(
        matches!(chain.execute(&build(&chain, &parent, &genesis, vec![], 0, Some(foreign)), &parent), Err(ChainError::WrongGroup)),
        "a group-3 tx in a group-0 payload"
    );
}

#[test]
fn committee_ceiling_is_bound_to_the_new_genesis() {
    let (_, a) = Chain::new(ChainConfig { group: 0, max_committee: 16, ..cfg() });
    let (_, b) = Chain::new(ChainConfig { group: 0, max_committee: 64, ..cfg() });
    assert_ne!(a.digest(), b.digest());
}
