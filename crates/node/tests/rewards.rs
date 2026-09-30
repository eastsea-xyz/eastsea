//! Node rewards on a chain whose genesis turned them on (docs/design/15-node-rewards.md):
//! Macs register through the real registry contract and answer their beacon
//! slots in blocks (no transaction); the first block of each epoch pays the
//! last epoch's node pool to their operators; proofs pay capped issuance only
//! to registered operators.

mod common;

use aether_execution::{proofs, PROVER_ESCROW};
use aether_light::block::ProofClaim;
use aether_node::chain::leader_address;
use aether_rewards as rewards;
use aether_rewards::{node_pool, proof_pool, proof_share};
use aether_types::{Address, U256};
use commonware_cryptography::{ed25519, Signer as _};
use common::{Mac, Net, Opts};

const CHAIN: u64 = 7_791;
/// Short epochs (twelve slots of three blocks, a one-block answer window) so a
/// day stays 288 blocks while a sixteenth of an epoch's proof share spans
/// several blocks' worth.
const EPOCH_BLOCKS: u64 = 36;
const OPERATORS: usize = 4;

/// Node rewards put the chain under the mainnet rules (`Chain::admissible_upgrade`:
/// an ordinary upgrade needs seven days of notice), so the genesis starts at
/// protocol 2 — the rules the proof market below needs — instead of scheduling
/// that upgrade a few blocks ahead.
fn net(node_rewards: bool) -> Net {
    Net::new(Opts { chain_id: CHAIN, node_rewards, epoch_blocks: EPOCH_BLOCKS, macs: OPERATORS as u8, min_streak: None, history_v2: false, protocol: 2, fees: false, reserve: None, committee: None })
}

fn supply(net: &Net, others: &[Address]) -> U256 {
    let s = &net.parent.state;
    (0..OPERATORS).map(|i| net.balance(i)).sum::<U256>() + others.iter().map(|a| s.balance(a)).sum::<U256>()
}

#[test]
fn operators_whose_macs_answer_are_paid_each_epoch_and_proofs_are_capped() {
    const E: u64 = EPOCH_BLOCKS;
    let mut net = net(true);
    let proposer = leader_address(&ed25519::PrivateKey::from_seed(1).public_key());
    let stranger = Address::repeat_byte(0x99);
    let others = [proposer, stranger, PROVER_ESCROW];
    let start = supply(&net, &others);
    assert!(rewards::enabled(&net.parent.state));

    // Epoch 0: four operators register at block 1 and answer every slot from block 2 on.
    // The genesis runs protocol 2 (the proof market) from height 0.
    let regs = (0..OPERATORS).map(|i| net.register(i)).collect();
    net.step(regs, None, vec![]);
    net.run_to(E - 1);
    assert!((0..OPERATORS as u64).all(|i| common::answered(&net.parent.state, i, 0) == rewards::SLOTS));
    let before: Vec<U256> = (0..OPERATORS).map(|i| net.balance(i)).collect();

    // Block E pays epoch 0: N = 4 new Macs (warm-up 0.5) get 1/32 of the pool each, no claim.
    let first = net.step(vec![], None, vec![]);
    let pool0 = node_pool(0, E);
    for (i, b) in before.iter().enumerate() {
        assert_eq!(net.balance(i) - b, pool0 / U256::from(32u8), "operator {i}");
    }
    assert_eq!(first.payouts.len(), OPERATORS);
    assert!(first.payouts.iter().all(|(h, _, a)| *h == E && *a == pool0 / U256::from(32u8)));
    let mut minted = pool0 / U256::from(32u8) * U256::from(OPERATORS);
    assert_eq!(supply(&net, &others), start + minted);

    // Protocol 2 from block E + 1; block E + 2 records its statement.
    net.run_to(E + 2);
    // Block E + 3: operator 0 proves block E + 1, a stranger proves E + 2.
    // Both stay in epoch 1, whose proof-share cap (a sixteenth of 36 blocks'
    // worth) takes several proofs to exhaust.
    let p = &net.parent;
    let op0 = net.operator(0);
    let claim = |h: u64, c: [u8; 32], who: Address| ProofClaim { height: h, prover: who, proof: hex::encode(aether_proving::block::claim(c, who)) };
    let c37 = proofs::commitment(&p.state, E + 1).unwrap();
    let c38 = p.statement.commitment;
    let (b0, s0) = (net.balance(0), p.state.balance(&stranger));
    let b39 = net.step(vec![], None, vec![claim(E + 1, c37, op0), claim(E + 2, c38, stranger)]);
    assert_eq!(net.balance(0) - b0, proof_share(E + 1), "one proof: one block's worth");
    assert_eq!(net.parent.state.balance(&stranger), s0, "no issuance for a prover that registered no Mac");
    assert_eq!(b39.payouts.len(), 2);
    minted += proof_share(E + 1);
    assert_eq!(supply(&net, &others), start + minted);
    // Operator 0's cap for epoch 1: proofs of E + 3 and E + 4 land the rest of it...
    let cap = proof_pool(1, E) / U256::from(16u8);
    assert_eq!(cap, proof_share(E + 1) * U256::from(36u8) / U256::from(16u8), "36 blocks' worth, a sixteenth each");
    let c39 = net.parent.statement.commitment;
    net.step(vec![], None, vec![claim(E + 3, c39, op0)]);
    assert_eq!(net.balance(0) - b0, proof_share(E + 1) * U256::from(2u8), "a second block's worth");
    let c40 = net.parent.statement.commitment;
    net.step(vec![], None, vec![claim(E + 4, c40, op0)]);
    assert_eq!(net.balance(0) - b0, cap, "the third proof exhausts the cap exactly");
    assert_eq!(proofs::prover(&net.parent.state, E + 4), Some(op0));
    // ...and a fourth past it mints nothing, though it still proves the block.
    let c41 = net.parent.statement.commitment;
    net.step(vec![], None, vec![claim(E + 5, c41, op0)]);
    assert_eq!(net.balance(0) - b0, cap);
    assert_eq!(proofs::prover(&net.parent.state, E + 5), Some(op0));
    minted += cap - proof_share(E + 1);
    assert_eq!(supply(&net, &others), start + minted);

    // Block 2E pays epoch 1 (everyone answered); then operator 3 goes silent for epoch 2.
    net.run_to(2 * E - 1);
    net.step(vec![], None, vec![]);
    minted += node_pool(1, E) / U256::from(32u8) * U256::from(OPERATORS);
    assert_eq!(supply(&net, &others), start + minted);
    net.behaviour.insert(3, Mac::Off);
    net.run_to(3 * E - 1);
    let silent = net.balance(3);
    let paid = net.step(vec![], None, vec![]);
    assert_eq!(net.balance(3), silent, "a silent operator gets nothing");
    assert_eq!(paid.payouts.len(), 3);
    minted += node_pool(2, E) / U256::from(32u8) * U256::from(3u8);
    assert_eq!(supply(&net, &others), start + minted);
    // Far less than the issuance so far: few operators, warm-up, caps.
    let issued: U256 = (1..=net.parent.height).map(rewards::issuance).sum();
    assert!(minted * U256::from(4u8) < issued);
}

#[test]
fn without_the_genesis_parameter_nothing_changes() {
    let mut net = net(false);
    // A genesis parameter: it changes the genesis root only when on.
    assert_ne!(net.parent.state.root(), self::net(true).parent.state.root());
    assert!(!rewards::enabled(&net.parent.state));
    let regs = (0..OPERATORS).map(|i| net.register(i)).collect();
    net.step(regs, None, vec![]);
    let before: Vec<U256> = (0..OPERATORS).map(|i| net.balance(i)).collect();
    while net.parent.height < EPOCH_BLOCKS + 1 {
        assert!(net.answers().is_empty(), "no slots without node rewards");
        let b = net.step(vec![], None, vec![]);
        assert!(b.payouts.is_empty());
    }
    assert_eq!(before, (0..OPERATORS).map(|i| net.balance(i)).collect::<Vec<_>>());
    // The node-reward account holds nothing.
    assert_eq!(net.parent.state.storage(&rewards::REWARDS, U256::ZERO), U256::ZERO);
    // A block carrying answers is refused on such a network.
    let fake = aether_light::block::BeaconAnswer { index: 0, slot: 0, signature: "00".repeat(64), attest: None };
    assert!(net.build(vec![], None, vec![], vec![fake]).is_err());
}
