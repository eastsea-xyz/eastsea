//! The epoch randomness (docs/design/13-roadmap.md, 무작위성): every epoch of
//! a node-rewards network opens with a word in REWARDS' storage — the
//! keccak256 of the epoch and draw seed the committee threshold-signed (the
//! same source the voting-set draw uses), published before the epoch began.
//! Two nodes replaying the same blocks agree; epochs before the first seed
//! read zero.

mod common;

use alloy_primitives::keccak256;
use aether_execution::registry;
use aether_light::block::Seed;
use aether_node::chain::{Chain, Executed};
use aether_types::{Address, Bytes, TxEnvelope, U256};
use common::{Net, Opts};
use std::sync::Arc;

const CHAIN: u64 = 7_799;
/// Blocks per registry epoch; the draw span is 24 epochs (registry default).
const E: u64 = 12;
/// The first height of draw 1 (epoch_blocks × draw_epochs): its seed opens
/// every epoch after it until draw 2's.
const DRAW_AT: u64 = E * 24;
const MACS: usize = 4;

fn net() -> Net {
    Net::new(Opts { chain_id: CHAIN, node_rewards: true, epoch_blocks: E, macs: MACS as u8, min_streak: Some(0), history_v2: true, protocol: 1, reserve: None, fees: false, committee: None })
}

/// The word an epoch opened by `seed` must hold.
fn expected_word(epoch: u64, seed: &Seed) -> U256 {
    let mut preimage = b"aether-randomness/v1".to_vec();
    preimage.extend_from_slice(&epoch.to_be_bytes());
    preimage.extend_from_slice(&seed.draw.to_be_bytes());
    preimage.extend_from_slice(&hex::decode(&seed.signature).unwrap());
    U256::from_be_bytes(keccak256(preimage).0)
}

/// Build the next block on `a` (txs, and `seed` if the committee just signed
/// one), then execute and finalize the same bytes on `b`, an independent node
/// of the same genesis: both must agree on every state root, the randomness
/// words included.
fn step(a: &mut Net, b: &mut Net, txs: Vec<TxEnvelope>, seed: Option<Seed>) -> Arc<Executed> {
    let answers = a.answers();
    let (block, exec) = a.build_extras(txs, None, vec![], answers, vec![], None, seed).unwrap();
    let bexec = b.chain.execute(&block, &b.parent).unwrap();
    assert_eq!(bexec.state.root(), exec.state.root(), "two nodes diverged at height {}", exec.height);
    for e in 0..=exec.height / E {
        assert_eq!(aether_rewards::randomness(&bexec.state, e), aether_rewards::randomness(&exec.state, e));
    }
    a.chain.finalize(&block).unwrap();
    b.chain.finalize(&block).unwrap();
    a.parent = exec.clone();
    a.last = block.clone();
    b.parent = bexec;
    b.last = block;
    exec
}

#[test]
fn each_epoch_opens_with_the_draw_seeds_word_and_nodes_agree() {
    let (mut a, mut b) = (net(), net());
    assert_eq!(a.committee.identity(), b.committee.identity(), "the harness deals the same committee to both");
    // The Macs register (block 1), then the chain runs quietly up to draw 1.
    let reg: Vec<TxEnvelope> = (0..MACS).map(|i| a.register(i)).collect();
    step(&mut a, &mut b, reg, None);
    assert_eq!(registry::params(&a.parent.state).draw_epochs, 24);
    while a.parent.height < DRAW_AT - 1 {
        step(&mut a, &mut b, vec![], None);
    }

    // The first block of draw 1 carries the committee's seed for it. The epoch
    // it opens (and every one before it) has no word: no seed was on chain
    // when they opened, and words are never backfilled.
    let seed = a.committee.sign_seed(CHAIN, 1);
    let opened = step(&mut a, &mut b, vec![], Some(seed.clone()));
    assert_eq!(opened.height, DRAW_AT);
    assert_eq!(aether_rewards::randomness(&opened.state, DRAW_AT / E), U256::ZERO, "the epoch the seed landed in opened without one");

    // The next epoch opens with the word: keccak256 over the published seed.
    while a.parent.height < DRAW_AT + E {
        step(&mut a, &mut b, vec![], None);
    }
    let epoch = (DRAW_AT + E) / E;
    let exec = a.parent.clone();
    let word = aether_rewards::randomness(&exec.state, epoch);
    assert_eq!(word, expected_word(epoch, &seed));
    assert!(!word.is_zero());
    assert_eq!(aether_rewards::randomness(&exec.state, epoch - 1), U256::ZERO, "the seedless epoch before stays zero");

    // The same draw source serves the next epoch, but the epoch number makes
    // its word distinct.
    while a.parent.height < DRAW_AT + 2 * E {
        step(&mut a, &mut b, vec![], None);
    }
    assert_eq!(aether_rewards::randomness(&a.parent.state, epoch + 1), expected_word(epoch + 1, &seed));
    assert_ne!(aether_rewards::randomness(&a.parent.state, epoch + 1), word);

    // And the word sits where clients of contracts read it: REWARDS, at the
    // tagged slot of the epoch (contracts/src/Randomness.sol).
    let slot = (U256::from(9u64) << 200) | U256::from(epoch);
    assert_eq!(a.parent.state.storage(&aether_rewards::REWARDS, slot), word);

    // A contract can call the genesis predeploy, with the same result as the
    // storage proof. The ABI selector is randomness(uint64).
    let mut input = hex::decode("193873c0").unwrap();
    input.extend_from_slice(&U256::from(epoch).to_be_bytes::<32>());
    let ctx = Chain::block_context(&a.chain.cfg(), &a.last, &a.parent);
    let result = aether_execution::call(
        &a.parent.state, &ctx, Address::ZERO, Some(aether_rewards::REWARDS),
        Bytes::from(input), U256::ZERO, 100_000,
    ).unwrap();
    assert!(result.success);
    assert_eq!(U256::from_be_slice(&result.output), word);
}
