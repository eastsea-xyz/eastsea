//! Node rewards on a chain whose genesis turned them on (docs/design/15-node-rewards.md):
//! Macs register through the real registry contract and beacon with ordinary
//! transactions; the first block of each epoch pays the last epoch's node pool
//! to their operators; proofs pay capped issuance only to registered operators.

use aether_crypto::{address_of, P256Signer, Signer};
use aether_execution::registry::{attestation_message, encode_beacon, encode_register, REGISTRY};
use aether_rewards as rewards;
use aether_rewards::{node_pool, proof_pool, proof_share};
use aether_execution::{proofs, sign_call, EvmCall, PROVER_ESCROW};
use aether_light::block::ProofClaim;
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{build_payload, leader_address, Chain, ChainConfig, Executed, Extras};
use aether_node::upgrade::{combine, sign_partial, Release, SignedUpgrade, Upgrade};
use aether_types::{Address, GasVector, TxEnvelope, U256};
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible, Signer as _};
use std::collections::HashMap;
use std::sync::Arc;

const CHAIN: u64 = 7_791;
const EPOCH_BLOCKS: u64 = 10;
const OPERATORS: u8 = 4;

fn seed(b: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[0] = 0x5a;
    s[31] = b;
    s
}

struct Net {
    chain: Chain,
    registrar: P256Signer,
    ops: Vec<P256Signer>,
    nonces: HashMap<Address, u64>,
    parent: Arc<Executed>,
    last: Block,
}

struct EchoVerifier;
impl aether_node::chain::ProofVerifier for EchoVerifier {
    fn verify(&self, proof: &[u8], commitment: [u8; 32]) -> bool {
        proof == commitment
    }
}

fn addr(s: &P256Signer) -> Address {
    address_of(&s.public_key()).unwrap()
}

impl Net {
    fn new(node_rewards: bool) -> Self {
        let registrar = P256Signer::from_seed(&seed(0)).unwrap();
        let ops: Vec<P256Signer> = (1..=OPERATORS).map(|i| P256Signer::from_seed(&seed(i)).unwrap()).collect();
        let cfg = ChainConfig {
            chain_id: CHAIN,
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: ops.iter().map(|s| (addr(s), U256::from(10u128.pow(20)))).collect(),
            fees: false,
            registrar: Some(aether_crypto::p256_xy(&registrar.public_key().bytes).unwrap()),
            epoch_blocks: EPOCH_BLOCKS,
            min_streak: None,
            draw_epochs: None,
            node_rewards,
        };
        let (chain, genesis) = Chain::new(cfg);
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        {
            let mut g = chain.lock();
            g.identity = Some(*sharing.public());
            g.protocol = 2;
            g.verifier = Some(Arc::new(EchoVerifier));
        }
        let parent = chain.lock().finalized.clone();
        Net { chain, registrar, ops, nonces: HashMap::new(), parent, last: genesis }
    }

    fn call(&mut self, from: usize, input: aether_types::Bytes) -> TxEnvelope {
        let a = addr(&self.ops[from]);
        let n = self.nonces.entry(a).or_default();
        let tx = sign_call(&self.ops[from], CHAIN, *n, 1, &EvmCall { to: Some(REGISTRY), value: U256::ZERO, input, gas_limit: 400_000, delegate: None }).unwrap();
        *n += 1;
        tx
    }

    /// Operator `i` registers its Mac (it also sends the beacons, as its own beaconer).
    fn register(&mut self, i: usize) -> TxEnvelope {
        let op = addr(&self.ops[i]);
        let (key, node) = ([i as u8 + 1; 32], [0x22; 32]);
        let sig = self.registrar.sign(&attestation_message(CHAIN, op, key, node, op)).unwrap();
        self.call(i, encode_register(key, node, op, sig[..32].try_into().unwrap(), sig[32..64].try_into().unwrap()))
    }

    fn beacon(&mut self, i: usize) -> TxEnvelope {
        self.call(i, encode_beacon([i as u8 + 1; 32]))
    }

    /// Build, execute and finalize the next block.
    fn step(&mut self, txs: Vec<TxEnvelope>, upgrade: Option<SignedUpgrade>, proofs: Vec<ProofClaim>) -> Arc<Executed> {
        let (chain, parent) = (&self.chain, &self.parent);
        let height = self.last.height.next();
        let leader = ed25519::PrivateKey::from_seed(1).public_key();
        let context = Context { round: Round::new(EPOCH, View::new(height.get())), leader, parent: (View::new(height.get() - 1), self.last.digest()) };
        let ts = height.get() * 1_000;
        let skeleton = Block::new(context.clone(), self.last.digest(), height, ts, bytes::Bytes::new());
        let ctx = Chain::block_context(&chain.cfg(), &skeleton, parent);
        let (pre, _) = chain.pre_state(parent, parent.next_protocol(), &proofs, false).unwrap();
        let n = txs.len();
        let (payload, out) = build_payload(parent, &pre, &ctx, txs, Extras { upgrade, proofs, ..Default::default() });
        assert_eq!(payload.txs.len(), n, "every tx fits");
        assert!(out.receipts.iter().all(|r| r.success), "block {height}: a registry call failed");
        drop(pre);
        let block = Block::new(context, self.last.digest(), height, ts, payload.to_bytes());
        let exec = chain.execute(&block, parent).unwrap();
        chain.finalize(&block).unwrap();
        self.parent = exec.clone();
        self.last = block;
        exec
    }

    fn balance(&self, i: usize) -> U256 {
        self.parent.state.balance(&addr(&self.ops[i]))
    }

    fn supply(&self, others: &[Address]) -> U256 {
        let s = &self.parent.state;
        (0..self.ops.len()).map(|i| self.balance(i)).sum::<U256>() + others.iter().map(|a| s.balance(a)).sum::<U256>()
    }
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
fn operators_whose_macs_beacon_are_paid_each_epoch_and_proofs_are_capped() {
    let mut net = Net::new(true);
    let proposer = leader_address(&ed25519::PrivateKey::from_seed(1).public_key());
    let stranger = Address::repeat_byte(0x99);
    let others = [proposer, stranger, PROVER_ESCROW];
    let start = net.supply(&others);
    assert!(rewards::enabled(&net.parent.state));

    // Epoch 0: four operators register (a registration is that epoch's beacon).
    let regs = (0..OPERATORS as usize).map(|i| net.register(i)).collect();
    net.step(regs, Some(signed(2, 20)), vec![]);
    while net.parent.height < EPOCH_BLOCKS - 1 {
        net.step(vec![], None, vec![]);
    }
    let before: Vec<U256> = (0..OPERATORS as usize).map(|i| net.balance(i)).collect();

    // Block 10 pays epoch 0: N = 4 new Macs (warm-up 0.5) get 1/32 of the pool each, no claim.
    let b10 = net.step(vec![], None, vec![]);
    let pool0 = node_pool(0, EPOCH_BLOCKS);
    for (i, b) in before.iter().enumerate() {
        assert_eq!(net.balance(i) - b, pool0 / U256::from(32u8), "operator {i}");
    }
    assert_eq!(b10.payouts.len(), OPERATORS as usize);
    assert!(b10.payouts.iter().all(|(h, _, a)| *h == 10 && *a == pool0 / U256::from(32u8)));
    let mut minted = pool0 / U256::from(32u8) * U256::from(OPERATORS);
    assert_eq!(net.supply(&others), start + minted);
    // Epoch 1 (blocks 10..20): everyone beacons at 11 (gas goes to the proposer).
    let beacons = (0..OPERATORS as usize).map(|i| net.beacon(i)).collect();
    net.step(beacons, None, vec![]);

    // Epoch 2: operator 3 goes silent.
    while net.parent.height < 2 * EPOCH_BLOCKS - 1 {
        net.step(vec![], None, vec![]);
    }
    let beacons = (0..3).map(|i| net.beacon(i)).collect();
    net.step(beacons, None, vec![]);
    minted += node_pool(1, EPOCH_BLOCKS) / U256::from(32u8) * U256::from(OPERATORS);
    assert_eq!(net.supply(&others), start + minted);
    net.step(vec![], None, vec![]); // 21: records block 20's statement (protocol 2 from 20)

    // Block 22: operator 0 proves block 20, a stranger proves block 21.
    let p = &net.parent;
    let op0 = addr(&net.ops[0]);
    let claim = |h: u64, c: [u8; 32], who: Address| ProofClaim { height: h, prover: who, proof: hex::encode(aether_proving::block::claim(c, who)) };
    let c20 = proofs::commitment(&p.state, 20).unwrap();
    let c21 = p.statement.commitment;
    let (b0, s0) = (net.balance(0), p.state.balance(&stranger));
    let b22 = net.step(vec![], None, vec![claim(20, c20, op0), claim(21, c21, stranger)]);
    // A sixteenth of epoch 2's proof share is less than one block's share with 10-block epochs.
    let cap = proof_pool(2, EPOCH_BLOCKS) / U256::from(16u8);
    assert!(cap < proof_share(20));
    assert_eq!(net.balance(0) - b0, cap);
    assert_eq!(net.parent.state.balance(&stranger), s0, "no issuance for a prover that registered no Mac");
    assert_eq!(b22.payouts.len(), 2);
    minted += cap;
    assert_eq!(net.supply(&others), start + minted);
    // Operator 0's cap for epoch 2 is used up.
    let c22 = net.parent.statement.commitment;
    let b0 = net.balance(0);
    net.step(vec![], None, vec![claim(22, c22, op0)]);
    assert_eq!(net.balance(0), b0);
    assert_eq!(proofs::prover(&net.parent.state, 22), Some(op0));

    while net.parent.height < 3 * EPOCH_BLOCKS - 1 {
        net.step(vec![], None, vec![]);
    }
    let silent = net.balance(3);
    let b30 = net.step(vec![], None, vec![]);
    assert_eq!(net.balance(3), silent, "a silent operator gets nothing");
    assert_eq!(b30.payouts.len(), 3);
    minted += node_pool(2, EPOCH_BLOCKS) / U256::from(32u8) * U256::from(3u8);
    assert_eq!(net.supply(&others), start + minted);
    // Far less than the issuance so far: few operators, warm-up, caps.
    let issued: U256 = (1..=net.parent.height).map(proofs::issuance).sum();
    assert!(minted * U256::from(4u8) < issued);
}

#[test]
fn without_the_genesis_parameter_nothing_changes() {
    let mut net = Net::new(false);
    // A genesis parameter: it changes the genesis root only when on.
    assert_ne!(net.parent.state.root(), Net::new(true).parent.state.root());
    assert!(!rewards::enabled(&net.parent.state));
    let regs = (0..OPERATORS as usize).map(|i| net.register(i)).collect();
    net.step(regs, None, vec![]);
    let before: Vec<U256> = (0..OPERATORS as usize).map(|i| net.balance(i)).collect();
    while net.parent.height < EPOCH_BLOCKS + 1 {
        let b = net.step(vec![], None, vec![]);
        assert!(b.payouts.is_empty());
    }
    assert_eq!(before, (0..OPERATORS as usize).map(|i| net.balance(i)).collect::<Vec<_>>());
    // The node-reward account holds nothing.
    assert_eq!(net.parent.state.storage(&rewards::REWARDS, U256::ZERO), U256::ZERO);
}
