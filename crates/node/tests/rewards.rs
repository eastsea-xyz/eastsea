//! Node rewards on a chain whose genesis turned them on (docs/design/15-node-rewards.md):
//! Macs register through the real registry contract and answer their beacon
//! slots in blocks (no transaction); the first block of each epoch pays the
//! last epoch's node pool to their operators; proofs pay capped issuance only
//! to registered operators.

mod common;

use aether_execution::{proofs, PROVER_ESCROW};
use aether_light::block::ProofClaim;
use aether_node::chain::leader_address;
use aether_node::upgrade::{combine, sign_partial, Release, SignedUpgrade, Upgrade};
use aether_rewards as rewards;
use aether_rewards::{node_pool, proof_pool, proof_share};
use aether_types::{Address, U256};
use commonware_cryptography::{ed25519, Signer as _};
use common::{Mac, Net, Opts};

const CHAIN: u64 = 7_791;
/// Short epochs (four slots of three blocks) so that a sixteenth of an epoch's
/// proof share is less than one block's.
const EPOCH_BLOCKS: u64 = 12;
const OPERATORS: usize = 4;

fn net(node_rewards: bool) -> Net {
    Net::new(Opts { chain_id: CHAIN, node_rewards, epoch_blocks: EPOCH_BLOCKS, macs: OPERATORS as u8, min_streak: None, history_v2: false, reserve: None })
}

fn supply(net: &Net, others: &[Address]) -> U256 {
    let s = &net.parent.state;
    (0..OPERATORS).map(|i| net.balance(i)).sum::<U256>() + others.iter().map(|a| s.balance(a)).sum::<U256>()
}

fn signed(protocol: u32, activate_at: u64) -> SignedUpgrade {
    let (_, sharing, shares) = aether_light::devnet_threshold(4);
    let u = Upgrade {
        chain_id: CHAIN,
        protocol,
        activate_at,
        releases: vec![Release { platform: "macos-arm64-dmg".into(), version: "0.7.0".into(), blake3: "ab".repeat(32), url: "https://x".into() }],
        notes: String::new(),
        registrar: None,
    };
    let partials: Vec<_> = shares.iter().take(3).map(|(_, s)| sign_partial(&u, s)).collect();
    combine(&sharing, &partials).unwrap()
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
    let regs = (0..OPERATORS).map(|i| net.register(i)).collect();
    net.step(regs, Some(signed(2, 20)), vec![]);
    net.run_to(E - 1);
    assert!((0..OPERATORS as u64).all(|i| common::answered(&net.parent.state, i, 0) == 4));
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

    // Protocol 2 from block 20; block 21 records block 20's statement.
    net.run_to(21);
    // Block 22 (epoch 1): operator 0 proves block 20, a stranger proves block 21.
    let p = &net.parent;
    let op0 = net.operator(0);
    let claim = |h: u64, c: [u8; 32], who: Address| ProofClaim { height: h, prover: who, proof: hex::encode(aether_proving::block::claim(c, who)) };
    let c20 = proofs::commitment(&p.state, 20).unwrap();
    let c21 = p.statement.commitment;
    let (b0, s0) = (net.balance(0), p.state.balance(&stranger));
    let b22 = net.step(vec![], None, vec![claim(20, c20, op0), claim(21, c21, stranger)]);
    // A sixteenth of epoch 1's proof share is less than one block's share with 12-block epochs.
    let cap = proof_pool(1, E) / U256::from(16u8);
    assert!(cap < proof_share(20));
    assert_eq!(net.balance(0) - b0, cap);
    assert_eq!(net.parent.state.balance(&stranger), s0, "no issuance for a prover that registered no Mac");
    assert_eq!(b22.payouts.len(), 2);
    minted += cap;
    assert_eq!(supply(&net, &others), start + minted);
    // Operator 0's cap for epoch 1 is used up.
    let c22 = net.parent.statement.commitment;
    let b0 = net.balance(0);
    net.step(vec![], None, vec![claim(22, c22, op0)]);
    assert_eq!(net.balance(0), b0);
    assert_eq!(proofs::prover(&net.parent.state, 22), Some(op0));

    // Block 2E pays epoch 1 (everyone answered); then operator 3 goes silent for epoch 2.
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
