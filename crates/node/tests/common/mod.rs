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
use aether_node::dkg::{Ceremony, DkgOutput, KeyFile, Msg, Round as KeyRound, To};
use aether_node::upgrade::SignedUpgrade;
use aether_rewards::{beacons, DAY_EPOCHS};
use aether_types::{Address, GasVector, TxEnvelope, U256};
use commonware_codec::Encode as _;
use commonware_consensus::types::{Round, View};
use commonware_cryptography::bls12381::primitives::group::Share;
use commonware_cryptography::{ed25519, Digestible, Signer as _};
use commonware_utils::{ordered::Set, TryCollect};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
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
    /// Answers every slot of the hours this bit mask covers — a Mac asleep
    /// through the rest of the day (hour = epoch % 24).
    Awake(u32),
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
    /// The genesis committee the harness dealt (identity and shares), for
    /// tests that drive committee handoffs.
    pub committee: Committee,
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
    /// Mainnet-flag networks run history v2 (quiet empty blocks).
    pub history_v2: bool,
    /// Protocol the genesis starts under (1 = the testnet's; 3 = the mainnet's,
    /// every rule on from height 0 with no signed upgrade).
    pub protocol: u32,
    pub reserve: Option<Reserve>,
    /// Fee policy v0 (base fees, tips): the mainnet rules, off on 7780.
    pub fees: bool,
    /// The genesis committee the chain records (node-rewards networks): by
    /// default the dealt one, but the rule must also work for a short one
    /// (the recorded word, not the dealt set, is what the seating rule reads).
    pub committee: Option<Vec<(String, String)>>,
}

impl Net {
    pub fn new(o: Opts) -> Self {
        let registrar = P256Signer::from_seed(&seed(0)).unwrap();
        let ops: Vec<P256Signer> = (1..=o.macs).map(|i| P256Signer::from_seed(&seed(i)).unwrap()).collect();
        let voting = (1..=o.macs as u64).map(ed25519::PrivateKey::from_seed).collect();
        let committee = Committee::genesis();
        let cfg = ChainConfig {
            chain_id: o.chain_id,
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: ops.iter().map(|s| (addr(s), U256::from(10u128.pow(20)))).collect(),
            fees: o.fees,
            registrar: Some(aether_crypto::p256_xy(&registrar.public_key().bytes).unwrap()),
            epoch_blocks: o.epoch_blocks,
            min_streak: o.min_streak,
            draw_epochs: None,
            node_rewards: o.node_rewards,
            history_v2: o.history_v2,
            group: 0,
            max_committee: aether_node::rotation::GROW_UNTIL,
            protocol: o.protocol,
            committee: o.committee.clone().unwrap_or_else(Committee::genesis_members),
            reserve: o.reserve,
        };
        let (chain, genesis) = Chain::new(cfg);
        {
            let mut g = chain.lock();
            g.identity = Some(committee.identity());
            // At least protocol 2: the harness may drive an upgrade to it.
            g.protocol = o.protocol.max(2);
            g.verifier = Some(Arc::new(EchoVerifier));
        }
        let parent = chain.lock().finalized.clone();
        Net { chain, chain_id: o.chain_id, registrar, ops, voting, committee, behaviour: BTreeMap::new(), nonces: HashMap::new(), parent, last: genesis }
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

    /// Operator `i` registers a key it does not own (a reserve key, say): a
    /// direct transaction with a valid attestation, not one the app built.
    pub fn register_raw(&mut self, from: usize, key: [u8; 32], node: [u8; 32]) -> TxEnvelope {
        let op = addr(&self.ops[from]);
        let sig = self.registrar.sign(&attestation_message(self.chain_id, op, key, node, op)).unwrap();
        self.call(from, encode_register(key, node, op, sig[..32].try_into().unwrap(), sig[32..64].try_into().unwrap()))
    }

    pub fn operator(&self, i: usize) -> Address {
        addr(&self.ops[i])
    }

    /// A free-lane registration item (G2): the registrar attests the Mac
    /// (voting key `voting`, node id `node`) for `op`, and `op` signs the relay
    /// message — the item the app's `prepare_register_node` builds and
    /// `submit_signed` completes. Valid in blocks up to `expiry`, with relay
    /// `nonce`.
    pub fn lane_registration(&self, op: &P256Signer, voting: &ed25519::PrivateKey, node: [u8; 32], nonce: u64, expiry: u64) -> aether_light::block::NodeRegistration {
        use aether_execution::registry::relay_message;
        let operator = addr(op);
        let key: [u8; 32] = voting.public_key().encode().as_ref().try_into().unwrap();
        let attestation = self.registrar.sign(&attestation_message(self.chain_id, operator, key, node, operator)).unwrap();
        let signature = op.sign(&relay_message(self.chain_id, operator, &key, &node, operator, &attestation, nonce, expiry)).unwrap();
        aether_light::block::NodeRegistration {
            operator,
            validator_key: key.into(),
            node_id: node.into(),
            beaconer: operator,
            attestation: attestation.into(),
            signature: signature.into(),
            operator_key: op.public_key().bytes.into(),
            nonce,
            expiry,
        }
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
                Mac::Awake(mask) => mask & (1 << (d.epoch % DAY_EPOCHS)) != 0,
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
        self.build_with(txs, upgrade, proofs, answers, vec![], None)
    }

    /// `build` carrying a committee handoff and free-lane registrations.
    pub fn build_with(&self, txs: Vec<TxEnvelope>, upgrade: Option<SignedUpgrade>, proofs: Vec<ProofClaim>, answers: Vec<BeaconAnswer>, registrations: Vec<aether_light::block::NodeRegistration>, handoff: Option<aether_light::block::Handoff>) -> Result<(Block, Arc<Executed>), ChainError> {
        let (block, exec) = self.build_extras(txs, upgrade, proofs, answers, registrations, handoff, None)?;
        assert!(exec.receipts.iter().all(|r| r.success), "block {}: a registry call failed", exec.height);
        Ok((block, exec))
    }

    /// `build` carrying a handoff and a draw seed, without asserting the
    /// receipts (a test may expect a call to revert).
    #[allow(clippy::too_many_arguments)]
    pub fn build_extras(&self, txs: Vec<TxEnvelope>, upgrade: Option<SignedUpgrade>, proofs: Vec<ProofClaim>, answers: Vec<BeaconAnswer>, registrations: Vec<aether_light::block::NodeRegistration>, handoff: Option<aether_light::block::Handoff>, seed: Option<aether_light::block::Seed>) -> Result<(Block, Arc<Executed>), ChainError> {
        let (chain, parent) = (&self.chain, &self.parent);
        let height = self.last.height.next();
        let leader = ed25519::PrivateKey::from_seed(1).public_key();
        let context = Context { round: Round::new(EPOCH, View::new(height.get())), leader, parent: (View::new(height.get() - 1), self.last.digest()) };
        let ts = height.get() * 1_000;
        let skeleton = Block::new(context.clone(), self.last.digest(), height, ts, bytes::Bytes::new());
        let ctx = Chain::block_context(&chain.cfg(), &skeleton, parent);
        // The proposer's pre-state skips answers it cannot check; the block is then built with what is left.
        let (pre, _) = chain.pre_state_with(parent, parent.next_protocol(), &proofs, &answers, &registrations, seed.as_ref(), false)?;
        let n = txs.len();
        let (payload, _) = build_payload(parent, &pre, &ctx, txs, Extras { handoff, upgrade, seed, proofs, beacons: answers, registrations, group: chain.cfg().group });
        assert_eq!(payload.txs.len(), n, "every tx fits");
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

    /// Build, execute and finalize the next block carrying a committee
    /// handoff, with every Mac's answers.
    pub fn step_handoff(&mut self, handoff: aether_light::block::Handoff) -> Arc<Executed> {
        let answers = self.answers();
        let (block, exec) = self.build_with(vec![], None, vec![], answers, vec![], Some(handoff)).unwrap();
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

/// Mac `i`'s committee entry (voting key hex, iroh node id): the pair a
/// committee or roster word holds, the same one `register` would give.
pub fn mac_entry(i: usize) -> (String, String) {
    let key = ed25519::PrivateKey::from_seed(i as u64 + 1).public_key();
    let node = aether_net::SecretKey::from_bytes(&[i as u8 + 1; 32]).public();
    (hex::encode(key.encode()), node.to_string())
}

/// The handoff members for a committed roster: every key must be one this
/// harness holds — a Mac's `voting` key or one of `reserve` — with the node
/// id exactly as the roster records it.
pub fn seat_of(net: &Net, roster: &[(String, String)], reserve: &[ed25519::PrivateKey]) -> Vec<(ed25519::PrivateKey, String)> {
    let key_of = |k: &str| {
        (0..net.voting.len())
            .find(|&i| hex::encode(net.voting_key(i)) == k)
            .map(|i| net.voting[i].clone())
            .or_else(|| reserve.iter().find(|r| hex::encode(r.public_key().encode()) == k).cloned())
    };
    roster
        .iter()
        .map(|(k, node)| (key_of(k).expect("a key this harness holds"), node.clone()))
        .collect()
}

/// Every Mac key that was ever answered in `epoch`, by candidate index.
pub fn answered(state: &aether_execution::WorldState, index: u64, epoch: u64) -> u64 {
    beacons::beacon(state, index).answered(epoch)
}

pub fn set_of(i: &[usize]) -> BTreeSet<usize> {
    i.iter().copied().collect()
}

/// A DKG or reshare round run in memory: every key in `online` runs a
/// `Ceremony` over a plain in-order queue — tests here want committees, not
/// loss (tests/dkg.rs runs the same rounds over a lossy, reordering one).
fn ceremony(
    online: &[ed25519::PrivateKey],
    round: &KeyRound,
    shares: &BTreeMap<ed25519::PublicKey, Share>,
    seed: u64,
) -> BTreeMap<ed25519::PublicKey, KeyFile> {
    struct Queue {
        pks: Vec<ed25519::PublicKey>,
        q: VecDeque<(ed25519::PublicKey, ed25519::PublicKey, Msg)>,
    }
    impl Queue {
        fn send(&mut self, from: &ed25519::PublicKey, out: Vec<(To, Msg)>) {
            for (to, msg) in out {
                match to {
                    To::One(p) => self.q.push_back((from.clone(), p, msg)),
                    To::All => self.pks.iter().filter(|p| **p != *from).for_each(|t| self.q.push_back((from.clone(), t.clone(), msg.clone()))),
                }
            }
        }
    }
    let pks: Vec<ed25519::PublicKey> = online.iter().map(|k| k.public_key()).collect();
    let mut net = Queue { pks: pks.clone(), q: VecDeque::new() };
    let mut cs = Vec::new();
    for (i, k) in online.iter().enumerate() {
        let (c, out) = Ceremony::start(ChaCha20Rng::seed_from_u64(seed * 31 + i as u64), k.clone(), round.clone(), shares.get(&pks[i]).cloned()).unwrap();
        net.send(&pks[i], out);
        cs.push(c);
    }
    let mut files = BTreeMap::new();
    let mut proposals = vec![None; cs.len()];
    for tick in 0..600 {
        for _ in 0..net.q.len() {
            let Some((from, to, msg)) = net.q.pop_front() else { break };
            if let Some(i) = pks.iter().position(|p| *p == to) {
                let out = cs[i].on_message(&from, msg);
                net.send(&to, out);
            }
        }
        for i in 0..cs.len() {
            let mut out = cs[i].pending_deals();
            if cs[i].all_acked() || tick > 20 {
                out.extend(cs[i].close_dealing());
            }
            if cs[i].is_player() && tick > 60 && cs[i].have_quorum_logs() {
                if let Some((digest, msg)) = cs[i].propose_transcript() {
                    if proposals[i].as_ref() != Some(&digest) {
                        proposals[i] = Some(digest);
                        out.push((To::All, msg));
                    }
                }
            }
            if tick > 60 { out.extend(cs[i].tick_agreement()); }
            if cs[i].is_player() && !files.contains_key(&pks[i]) {
                if let Some(digest) = cs[i].certified_transcript() {
                    let (o, s) = cs[i].finish_decided(&mut ChaCha20Rng::seed_from_u64(7), &digest).unwrap();
                    files.insert(pks[i].clone(), KeyFile::new(round.round, &o, &s));
                }
            }
            out.extend(cs[i].rebroadcast());
            net.send(&pks[i], out);
        }
        if pks.iter().enumerate().filter(|(i, _)| cs[*i].is_player()).all(|(_, pk)| files.contains_key(pk)) {
            return files;
        }
    }
    panic!("the key round did not complete");
}

/// A voting committee the harness dealt: the members' keys and their shares of
/// one threshold identity — the genesis committee, or the one a handoff names.
pub struct Committee {
    /// Members' ed25519 keys, in roster order.
    pub keys: Vec<ed25519::PrivateKey>,
    /// Each member's `KeyFile` (output hex, identity, share), by public key.
    pub files: BTreeMap<ed25519::PublicKey, KeyFile>,
}

impl Committee {
    /// The genesis committee: a fresh four-key DKG (fixed seeds).
    pub fn genesis() -> Self {
        let keys: Vec<ed25519::PrivateKey> = (0..4u64).map(|i| ed25519::PrivateKey::from_seed(200 + i)).collect();
        let players: Set<ed25519::PublicKey> = keys.iter().map(|k| k.public_key()).try_collect().unwrap();
        let dealt = ceremony(&keys, &KeyRound::dkg(players, 0), &BTreeMap::new(), 91);
        Self::of(keys, dealt)
    }

    /// The genesis committee's members, as a recorded committee word holds
    /// them (the dealt keys with node ids of their own).
    pub fn genesis_members() -> Vec<(String, String)> {
        (0..4u64)
            .map(|i| {
                let key = ed25519::PrivateKey::from_seed(200 + i).public_key();
                let node = aether_net::SecretKey::from_bytes(&[0xa0 + i as u8; 32]).public();
                (hex::encode(key.encode()), node.to_string())
            })
            .collect()
    }

    fn of(keys: Vec<ed25519::PrivateKey>, mut dealt: BTreeMap<ed25519::PublicKey, KeyFile>) -> Self {
        let files = keys.iter().map(|k| (k.public_key(), dealt.remove(&k.public_key()).expect("every member finished"))).collect();
        Committee { keys, files }
    }

    /// The committee's threshold identity, which every handoff must keep.
    pub fn identity(&self) -> aether_light::Identity {
        *self.output().public().public()
    }

    /// The committee's DKG output: the sharing of its identity.
    fn output(&self) -> DkgOutput {
        self.files.values().next().unwrap().decode(self.keys.len() as u32).unwrap().0
    }

    /// Sign `u` with a quorum of this committee, as a scheduled upgrade needs.
    pub fn sign_upgrade(&self, u: &aether_node::upgrade::Upgrade) -> aether_node::upgrade::SignedUpgrade {
        let sharing = self.output();
        let n = self.keys.len() as u32;
        let partials: Vec<_> = self.keys.iter().map(|k| aether_node::upgrade::sign_partial(u, &self.files[&k.public_key()].decode(n).unwrap().1)).collect();
        let quorum = sharing.public().required() as usize;
        aether_node::upgrade::combine(sharing.public(), &partials[..quorum]).unwrap()
    }

    /// The committee's threshold signature on draw `draw`'s seed, as the
    /// voting committee produces it between draws.
    pub fn sign_seed(&self, chain_id: u64, draw: u64) -> aether_light::block::Seed {
        let sharing = self.output();
        let n = self.keys.len() as u32;
        let partials: Vec<_> = self
            .keys
            .iter()
            .map(|k| {
                let share = &self.files[&k.public_key()].decode(n).unwrap().1;
                aether_node::handoff::check_seed_partial(chain_id, sharing.public(), draw, &aether_node::handoff::sign_seed_partial(chain_id, draw, share)).unwrap()
            })
            .collect();
        aether_node::handoff::combine_seed(sharing.public(), draw, &partials[..sharing.public().required() as usize]).unwrap()
    }

    /// Reshare the identity to `members` (ed25519 key, node id) at key round
    /// `round` and sign the handoff for it with this committee's shares, as
    /// the running committee does when the chain seats or unseats the
    /// founder's reserve keys. The new committee (to sign the next handoff
    /// from) comes back with it.
    pub fn handoff_to(
        &self,
        chain_id: u64,
        round: u64,
        members: &[(ed25519::PrivateKey, String)],
    ) -> (Committee, aether_light::block::Handoff) {
        let keys: Vec<ed25519::PrivateKey> = members.iter().map(|(k, _)| k.clone()).collect();
        let players: Set<ed25519::PublicKey> = keys.iter().map(|k| k.public_key()).try_collect().unwrap();
        // The current share holders deal; the new members receive (a key in
        // both sets runs one ceremony over both roles).
        let mut online = self.keys.clone();
        for k in &keys {
            if !online.iter().any(|o| o.public_key() == k.public_key()) {
                online.push(k.clone());
            }
        }
        let n = self.keys.len() as u32;
        let shares: BTreeMap<_, _> = self.keys.iter().map(|k| (k.public_key(), self.files[&k.public_key()].decode(n).unwrap().1)).collect();
        let previous = self.output();
        let dealt = ceremony(&online, &KeyRound::reshare(previous.clone(), players, round), &shares, round + 500);
        let next = Self::of(keys, dealt);
        let h = aether_light::block::Handoff {
            round,
            output: next.files.values().next().unwrap().output.clone(),
            members: members.iter().map(|(k, node)| (hex::encode(k.public_key().encode()), node.clone())).collect(),
            signature: String::new(),
        };
        // A quorum of the running committee signs (the sharing's own threshold,
        // three of the four genesis keys and more once a handoff has grown it).
        let partials: Vec<_> = self.keys.iter().map(|k| {
            let s = &shares[&k.public_key()];
            aether_node::handoff::check_partial(chain_id, previous.public(), &h, &aether_node::handoff::sign_partial(chain_id, &h, s)).unwrap()
        }).collect();
        let signed = aether_node::handoff::combine(previous.public(), &h, &partials[..previous.public().required() as usize]).unwrap();
        (next, signed)
    }
}
