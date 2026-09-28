//! A node-rewards chain driven block by block: Macs register through the real
//! registry contract with real voting keys, and the Macs set online answer
//! their beacon slots in every block, as `candidate::beacon_loop` does.

#![allow(dead_code)]

use aether_crypto::{address_of, P256Signer, Signer};
use aether_execution::registry::{attestation_message, encode_register, REGISTRY};
use aether_execution::{sign_call, EvmCall};
use aether_light::block::{BeaconAnswer, ProofClaim, Reattestation};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{build_payload, Chain, ChainConfig, ChainError, Executed, Extras, Reserve};
use aether_node::upgrade::SignedUpgrade;
use aether_rewards::beacons;
use aether_types::{Address, GasVector, TxEnvelope, U256};
use commonware_codec::Encode as _;
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible, Signer as _};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

pub fn seed(b: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[0] = 0x5a;
    s[31] = b;
    s
}

pub fn addr(s: &P256Signer) -> Address {
    address_of(&s.public_key()).unwrap()
}

pub struct EchoVerifier;
impl aether_node::chain::ProofVerifier for EchoVerifier {
    fn verify(&self, proof: &[u8], commitment: [u8; 32]) -> bool {
        proof == commitment
    }
}

/// How a Mac behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mac {
    /// Answers every slot it can, re-attesting when due.
    Honest,
    /// Answers only the slots in this mask.
    Slots(u64),
    /// Answers, but never gets a re-attestation (its app gives no DeviceCheck token).
    NoReattest,
    /// Off.
    Off,
}

pub struct Net {
    pub chain: Chain,
    pub chain_id: u64,
    pub registrar: P256Signer,
    /// Operator wallets (each registers the Mac of the same index).
    pub ops: Vec<P256Signer>,
    /// Voting keys, one per Mac.
    pub voting: Vec<ed25519::PrivateKey>,
    pub behaviour: BTreeMap<usize, Mac>,
    nonces: HashMap<Address, u64>,
    pub parent: Arc<Executed>,
    pub last: Block,
}

pub struct Opts {
    pub chain_id: u64,
    pub node_rewards: bool,
    pub epoch_blocks: u64,
    pub macs: u8,
    pub min_streak: Option<u64>,
    pub reserve: Option<Reserve>,
}

impl Net {
    pub fn new(o: Opts) -> Self {
        let registrar = P256Signer::from_seed(&seed(0)).unwrap();
        let ops: Vec<P256Signer> = (1..=o.macs).map(|i| P256Signer::from_seed(&seed(i)).unwrap()).collect();
        let voting = (1..=o.macs as u64).map(ed25519::PrivateKey::from_seed).collect();
        let cfg = ChainConfig {
            chain_id: o.chain_id,
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: ops.iter().map(|s| (addr(s), U256::from(10u128.pow(20)))).collect(),
            fees: false,
            registrar: Some(aether_crypto::p256_xy(&registrar.public_key().bytes).unwrap()),
            epoch_blocks: o.epoch_blocks,
            min_streak: o.min_streak,
            draw_epochs: None,
            node_rewards: o.node_rewards,
            history_v2: false,
            reserve: o.reserve,
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
        Net { chain, chain_id: o.chain_id, registrar, ops, voting, behaviour: BTreeMap::new(), nonces: HashMap::new(), parent, last: genesis }
    }

    pub fn voting_key(&self, i: usize) -> [u8; 32] {
        self.voting[i].public_key().encode().as_ref().try_into().unwrap()
    }

    pub fn node_id(i: usize) -> [u8; 32] {
        *aether_net::SecretKey::from_bytes(&[i as u8 + 1; 32]).public().as_bytes()
    }

    fn call(&mut self, from: usize, input: aether_types::Bytes) -> TxEnvelope {
        let a = addr(&self.ops[from]);
        let n = self.nonces.entry(a).or_default();
        let call = EvmCall { to: Some(REGISTRY), value: U256::ZERO, input, gas_limit: 400_000, delegate: None };
        let tx = sign_call(&self.ops[from], self.chain_id, *n, 1, &call).unwrap();
        *n += 1;
        tx
    }

    /// Operator `i` registers Mac `i` (online from then on unless told otherwise).
    pub fn register(&mut self, i: usize) -> TxEnvelope {
        let op = addr(&self.ops[i]);
        let (key, node) = (self.voting_key(i), Self::node_id(i));
        let sig = self.registrar.sign(&attestation_message(self.chain_id, op, key, node, op)).unwrap();
        self.behaviour.entry(i).or_insert(Mac::Honest);
        self.call(i, encode_register(key, node, op, sig[..32].try_into().unwrap(), sig[32..64].try_into().unwrap()))
    }

    pub fn operator(&self, i: usize) -> Address {
        addr(&self.ops[i])
    }

    pub fn balance(&self, i: usize) -> U256 {
        self.parent.state.balance(&self.operator(i))
    }

    /// The registrar's re-attestation of Mac `i` for `period`.
    pub fn reattestation(&self, i: usize, period: u64) -> Reattestation {
        let sig = self.registrar.sign(&beacons::reattest_message(self.chain_id, &self.voting_key(i), period)).unwrap();
        Reattestation { period, r: hex::encode(&sig[..32]), s: hex::encode(&sig[32..64]) }
    }

    /// The state the next block checks answers against.
    pub fn view(&self) -> aether_execution::WorldState {
        let hash: [u8; 32] = self.parent.digest.as_ref().try_into().unwrap();
        aether_node::beacons::next_view(&self.parent.state, self.parent.height + 1, hash)
    }

    /// Mac `i`'s answers for the next block, by its behaviour (and whether it would need a re-attestation it lacks).
    pub fn answers_of(&self, i: usize) -> Vec<BeaconAnswer> {
        let view = self.view();
        let h = self.parent.height + 1;
        let Some(c) = aether_execution::registry::candidates(&view).into_iter().find(|c| c.validator_key == self.voting_key(i)) else {
            return vec![];
        };
        let how = self.behaviour.get(&i).copied().unwrap_or(Mac::Off);
        beacons::due(&view, h, &c)
            .into_iter()
            .filter(|d| match how {
                Mac::Off => false,
                Mac::Slots(mask) => mask & (1 << d.slot) != 0,
                Mac::NoReattest => !d.needs_attestation,
                Mac::Honest => true,
            })
            .map(|d| {
                let attest = d.needs_attestation.then(|| self.reattestation(i, d.period));
                aether_node::beacons::sign(&self.voting[i], self.chain_id, c.index, &d, attest)
            })
            .collect()
    }

    /// Every Mac's answers for the next block.
    pub fn answers(&self) -> Vec<BeaconAnswer> {
        (0..self.voting.len()).flat_map(|i| self.answers_of(i)).collect()
    }

    /// Build and execute the next block (not finalized).
    pub fn build(&self, txs: Vec<TxEnvelope>, upgrade: Option<SignedUpgrade>, proofs: Vec<ProofClaim>, answers: Vec<BeaconAnswer>) -> Result<(Block, Arc<Executed>), ChainError> {
        let (chain, parent) = (&self.chain, &self.parent);
        let height = self.last.height.next();
        let leader = ed25519::PrivateKey::from_seed(1).public_key();
        let context = Context { round: Round::new(EPOCH, View::new(height.get())), leader, parent: (View::new(height.get() - 1), self.last.digest()) };
        let ts = height.get() * 1_000;
        let skeleton = Block::new(context.clone(), self.last.digest(), height, ts, bytes::Bytes::new());
        let ctx = Chain::block_context(&chain.cfg(), &skeleton, parent);
        // The proposer's pre-state skips answers it cannot check; the block is then built with what is left.
        let (pre, _) = chain.pre_state_with(parent, parent.next_protocol(), &proofs, &answers, false)?;
        let n = txs.len();
        let (payload, out) = build_payload(parent, &pre, &ctx, txs, Extras { upgrade, proofs, beacons: answers, ..Default::default() });
        assert_eq!(payload.txs.len(), n, "every tx fits");
        assert!(out.receipts.iter().all(|r| r.success), "block {height}: a registry call failed");
        drop(pre);
        let block = Block::new(context, self.last.digest(), height, ts, payload.to_bytes());
        let exec = chain.execute(&block, parent)?;
        Ok((block, exec))
    }

    /// Build, execute and finalize the next block with every Mac's answers.
    pub fn step(&mut self, txs: Vec<TxEnvelope>, upgrade: Option<SignedUpgrade>, proofs: Vec<ProofClaim>) -> Arc<Executed> {
        let answers = self.answers();
        self.step_with(txs, upgrade, proofs, answers)
    }

    pub fn step_with(&mut self, txs: Vec<TxEnvelope>, upgrade: Option<SignedUpgrade>, proofs: Vec<ProofClaim>, answers: Vec<BeaconAnswer>) -> Arc<Executed> {
        let (block, exec) = self.build(txs, upgrade, proofs, answers).unwrap();
        self.chain.finalize(&block).unwrap();
        self.parent = exec.clone();
        self.last = block;
        exec
    }

    /// Empty blocks (with answers) until the head is at `height`.
    pub fn run_to(&mut self, height: u64) {
        while self.parent.height < height {
            self.step(vec![], None, vec![]);
        }
    }
}

/// Every Mac key that was ever answered in `epoch`, by candidate index.
pub fn answered(state: &aether_execution::WorldState, index: u64, epoch: u64) -> u64 {
    beacons::beacon(state, index).answered(epoch)
}

pub fn set_of(i: &[usize]) -> BTreeSet<usize> {
    i.iter().copied().collect()
}
