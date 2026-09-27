//! Chain state shared by the consensus application, RPC and gossip.
//!
//! Every verified or proposed block's post-state is kept by digest so
//! competing forks can be validated; the finalized head is what RPC serves.

use crate::block::{Block, Payload, PublicKey};
use crate::inclusion::{self, InclusionPool};
use crate::store::{Commit, Store, StoreError};
use aether_crypto::{address_of, PublicKey as AetherPk};
use aether_execution::{execute_block, fees, BlockContext, BlockOutcome, FeePolicy, Receipt, WorldState};
use aether_hash::ChainHasher;
use aether_types::{Address, FeeVector, GasVector, SignerScheme, TxEnvelope, TxHash, B256, U256};
use commonware_consensus::Heightable;
use commonware_cryptography::{sha256::Digest, Digestible};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const MAX_TXS_PER_BLOCK: usize = 2_000;
pub const MAX_MEMPOOL: usize = 50_000;
/// Pending txs one sender may have (spam from one key cannot fill the pool).
pub const MAX_PER_SENDER: usize = 64;

#[derive(Clone, Debug)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub limits: GasVector,
    pub alloc: Vec<(Address, U256)>,
    /// Fee policy v0 (docs/research/tokenomics-2026.md): exponential base fees,
    /// base exec burned, prove fees and 20% of tips to the prover escrow.
    pub fees: bool,
    /// DeviceCheck registrar key: predeploys the voting-node registry at genesis.
    pub registrar: Option<([u8; 32], [u8; 32])>,
    /// Blocks per voting-node epoch (0 = the default).
    pub epoch_blocks: u64,
    /// Voting-set draw parameters (None = the defaults).
    pub min_streak: Option<u64>,
    pub draw_epochs: Option<u64>,
}

impl ChainConfig {
    pub fn genesis_state(&self) -> WorldState {
        let mut s = WorldState::default();
        for (a, v) in &self.alloc {
            s.set_balance(*a, *v).expect("genesis balance fits u128");
        }
        // The account contract P-256 accounts delegate to (EIP-7702) for batched calls.
        s.set_code(aether_execution::AETHER_ACCOUNT, aether_execution::aether_account_code()).expect("predeploy");
        if let Some(key) = self.registrar {
            let d = aether_execution::registry::Params::default();
            let params = aether_execution::registry::Params {
                epoch_blocks: if self.epoch_blocks == 0 { d.epoch_blocks } else { self.epoch_blocks },
                min_streak: self.min_streak.unwrap_or(d.min_streak),
                draw_epochs: self.draw_epochs.unwrap_or(d.draw_epochs),
            };
            aether_execution::registry::predeploy(&mut s, key, params).expect("registry predeploy");
        }
        s
    }
}

/// Deterministic public development keys (like hardhat/anvil accounts).
/// NEVER use these for anything of value.
pub fn dev_seed(i: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[0] = 0xae;
    s[31] = i;
    s
}

pub fn dev_accounts(n: u8) -> Vec<(u8, Address)> {
    (1..=n)
        .map(|i| {
            let s = aether_crypto::P256Signer::from_seed(&dev_seed(i)).expect("valid dev seed");
            (i, address_of(&aether_crypto::Signer::public_key(&s)).expect("address"))
        })
        .collect()
}

pub fn leader_address(leader: &PublicKey) -> Address {
    let pk = AetherPk { scheme: SignerScheme::Ed25519, bytes: leader.as_ref().to_vec() };
    address_of(&pk).expect("ed25519 address")
}

#[derive(Clone)]
pub struct Executed {
    pub height: u64,
    pub digest: Digest,
    pub timestamp: u64,
    pub state: WorldState,
    pub receipts: Vec<Receipt>,
    pub tx_hashes: Vec<TxHash>,
    pub gas: GasVector,
    pub proposer: Address,
    /// Base fees this block paid.
    pub base_fee: FeeVector,
    /// Fee-market excess after this block (its child's base fee derives from it).
    pub excess: GasVector,
    /// The latest committee handoff in this block's ancestry (inclusive).
    pub handoff: Option<Arc<crate::handoff::Pending>>,
    /// The latest draw seed in this block's ancestry (inclusive), with the height that carried it.
    pub seed: Option<Arc<(u64, aether_light::block::Seed)>>,
    /// Peaks of the MMR over blocks 0..=height; its root goes in the child's payload.
    pub history: Arc<aether_state::mmr::Mmr>,
    /// Protocol activations put on chain up to this block (committee-signed upgrades).
    pub schedule: Arc<crate::upgrade::Schedule>,
    /// What a proof of this block proves, and its escrow share (the child records both, protocol 2).
    pub statement: Statement,
    /// Proofs this block paid: (proven height, prover, amount).
    pub payouts: Vec<(u64, Address, U256)>,
}

/// A block's statement commitment (aether_proving::block) and escrow share.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Statement {
    pub commitment: [u8; 32],
    pub escrow: U256,
}

/// Proofs per block, and the largest proof accepted (protocol 2).
pub const MAX_PROOFS_PER_BLOCK: usize = 2;
pub const MAX_PROOF_BYTES: usize = 128 << 10;
/// Real proofs are ~98 KB; anything much smaller is refused before any work.
const MIN_PROOF_BYTES: usize = 32 << 10;
/// Proof RPC: at most one verification started per interval (it is public).
const PROOF_CHECK_INTERVAL: Duration = Duration::from_millis(500);
/// Blocks without proofs in this node's proposals after one with proofs lost.
const PROOF_BACKOFF: u64 = 100;

/// Checks a block proof against a statement commitment (the pinned sidecar in a node).
pub trait ProofVerifier: Send + Sync {
    fn verify(&self, proof: &[u8], commitment: [u8; 32]) -> bool;
    /// Some(answer) when the verifier actually decided; None when it could not
    /// (a transient failure, never to be remembered as a rejection).
    fn decide(&self, proof: &[u8], commitment: [u8; 32]) -> Option<bool> {
        Some(self.verify(proof, commitment))
    }
}

/// Hash of the chain metadata a block leaves outside the state tree.
pub fn meta_digest(
    excess: &GasVector,
    handoff: Option<&crate::handoff::Pending>,
    seed: Option<&(u64, aether_light::block::Seed)>,
    schedule: &[crate::upgrade::Activation],
    statement: &Statement,
) -> B256 {
    // Protocol-1 encoding until an activation carries a registrar or a statement
    // is recorded: binaries of either protocol agree on protocol-1 blocks.
    let schedule: Vec<Value> = schedule
        .iter()
        .map(|a| match a.registrar {
            None => serde_json::json!([a.protocol, a.at]),
            Some(r) => serde_json::json!([a.protocol, a.at, r]),
        })
        .collect();
    let bytes = if *statement == Statement::default() {
        serde_json::to_vec(&(excess, handoff, seed, schedule))
    } else {
        serde_json::to_vec(&(excess, handoff, seed, schedule, statement))
    }
    .expect("metadata serializes");
    B256::from(aether_hash::Hasher::hash_bytes(&ChainHasher::new(), &bytes))
}

impl Executed {
    pub fn meta_digest(&self) -> B256 {
        meta_digest(&self.excess, self.handoff.as_deref(), self.seed.as_deref(), &self.schedule, &self.statement)
    }

    /// The protocol whose rules the block after this one follows.
    pub fn next_protocol(&self) -> u32 {
        crate::upgrade::protocol_at(&self.schedule, self.height + 1)
    }
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct BlockSummary {
    pub height: u64,
    pub hash: String,
    pub parent: String,
    pub timestamp_ms: u64,
    pub proposer: Address,
    /// State root after executing this block (committed in the child header).
    pub state_root: B256,
    pub parent_state_root: B256,
    pub txs: Vec<TxHash>,
    pub gas_used: u64,
    pub prove_gas: u64,
    #[serde(default)]
    pub base_fee: FeeVector,
    #[serde(default)]
    pub excess: GasVector,
}

pub struct Inner {
    pub cfg: ChainConfig,
    executed: HashMap<Digest, Arc<Executed>>,
    pub finalized: Arc<Executed>,
    pub blocks: BTreeMap<u64, BlockSummary>,
    pub receipts: HashMap<TxHash, (u64, Receipt)>,
    pub mempool: BTreeMap<TxHash, TxEnvelope>,
    /// When each mempool tx arrived (inclusion lists name the oldest).
    arrivals: HashMap<TxHash, Instant>,
    /// Txs named by inclusion lists (FOCIL).
    pub inclusion: InclusionPool,
    /// Devnet fault injection: act as a proposer that censors this sender and
    /// ignores inclusion lists. Used to test censorship resistance.
    pub censor: Option<Address>,
    /// Devnet fault injection: leave this sender out of mempool ordering but
    /// still honour inclusion lists (its txs then land only via lists).
    pub deprioritize: Option<Address>,
    /// Durable finalized state; None = memory only (tests).
    store: Option<Arc<Store>>,
    /// Pending txs per sender (bounded by `MAX_PER_SENDER`).
    pub pending_by_sender: HashMap<Address, usize>,
    /// The running voting set (validators only; empty on followers).
    pub committee: crate::rotation::Committee,
    /// Committee identity: handoffs must be signed under it (None: devnet dealer keys).
    pub identity: Option<aether_light::Identity>,
    /// First height of the epoch this node's voting set runs (0 = genesis).
    pub epoch_start: u64,
    /// Last height of the running epoch once a handoff is finalized (MAX = open);
    /// the consensus epocher reads it, so the running set re-proposes its last
    /// block until it is final, then stops.
    pub epoch_end: Arc<std::sync::atomic::AtomicU64>,
    /// The voting set proposed for the current registry epoch (from its first block).
    pub proposal: Option<(u64, Vec<(String, String)>)>,
    /// A committee-signed handoff waiting to be put in a block.
    pub handoff_ready: Option<aether_light::block::Handoff>,
    /// The Macs eligible for the current draw, frozen at its first block: (draw, pool).
    pub pool: Option<(u64, Vec<(String, String)>)>,
    /// A committee-signed draw seed waiting to be put in a block.
    pub seed_ready: Option<aether_light::block::Seed>,
    /// MMR leaves of every finalized block from genesis (for history proofs),
    /// shared so proofs are built without holding the chain lock. None when
    /// this node started from a checkpoint (it does not have early blocks).
    pub history_leaves: Option<Arc<Vec<[u8; 32]>>>,
    /// The newest protocol this node runs; blocks under a later one are refused.
    pub protocol: u32,
    /// One-time state changes of each protocol at its activation block.
    pub migrate: aether_execution::forks::Migration,
    /// Committee-signed upgrades this node knows of (from its upgrades folder),
    /// put on chain by its proposals until they are there.
    pub upgrades_known: Vec<crate::upgrade::SignedUpgrade>,
    /// Checks block proofs (protocol 2); None: this node refuses blocks carrying proofs.
    pub verifier: Option<Arc<dyn ProofVerifier>>,
    /// Proofs received and verified locally, waiting to go in a block.
    pub proof_pool: Vec<aether_light::block::ProofClaim>,
    /// The last finalized blocks (the prover builds its inputs from them).
    recent: std::collections::VecDeque<Block>,
    /// Heights this node's prover already took up.
    attempted: std::collections::BTreeSet<u64>,
    /// When the proof RPC last started a verification (rate limit).
    last_proof_check: Option<Instant>,
    /// This node's last proposal carrying proofs: (height, block).
    proof_proposal: Option<(u64, Digest)>,
    /// No proofs in this node's proposals before this height.
    proof_backoff_until: u64,
    /// Proof submissions (claim output + proof) this node's RPC already refused.
    rejected: std::collections::HashSet<[u8; 32]>,
}

#[derive(Clone)]
pub struct Chain(pub Arc<Mutex<Inner>>);

#[derive(Debug)]
pub enum ChainError {
    BadPayload,
    ParentRootMismatch,
    Exec(String),
    BalMismatch,
    GasMismatch,
    UnknownParent,
    Store(String),
    /// A different block was finalized at a height we already finalized: a
    /// safety failure (or this node was pointed at another chain). Never ignored.
    ConflictingFinality {
        height: u64,
    },
    BadHandoff(String),
    /// Wrong protocol version, an invalid upgrade, or one this node does not run.
    Protocol(String),
    /// The payload's history root or parent metadata hash does not match the chain before it.
    HistoryMismatch,
}

impl Chain {
    pub fn new(cfg: ChainConfig) -> (Self, Block) {
        let state = cfg.genesis_state();
        let genesis = Block::genesis(cfg.chain_id, state.root());
        let exec = Arc::new(Executed {
            height: 0,
            digest: genesis.digest(),
            timestamp: 0,
            state,
            receipts: vec![],
            tx_hashes: vec![],
            gas: GasVector::default(),
            proposer: Address::ZERO,
            base_fee: FeeVector::default(),
            excess: GasVector::default(),
            handoff: None,
            seed: None,
            history: Arc::new(aether_state::mmr::Mmr::default().append(&ChainHasher::new(), 0, &digest_bytes(&genesis.digest()))),
            schedule: Arc::new(Vec::new()),
            statement: Statement::default(),
            payouts: vec![],
        });
        let mut executed = HashMap::new();
        executed.insert(genesis.digest(), exec.clone());
        let mut blocks = BTreeMap::new();
        blocks.insert(0, summary(&genesis, &exec, B256::ZERO));
        let inner = Inner {
            cfg,
            executed,
            finalized: exec,
            blocks,
            receipts: HashMap::new(),
            mempool: BTreeMap::new(),
            pending_by_sender: HashMap::new(),
            arrivals: HashMap::new(),
            inclusion: InclusionPool::default(),
            censor: None,
            committee: Default::default(),
            identity: None,
            epoch_start: 0,
            epoch_end: Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
            proposal: None,
            history_leaves: Some(Arc::new(vec![aether_state::mmr::leaf(&ChainHasher::new(), 0, &digest_bytes(&genesis.digest()))])),
            handoff_ready: None,
            pool: None,
            seed_ready: None,
            deprioritize: None,
            store: None,
            protocol: crate::upgrade::PROTOCOL,
            migrate: aether_execution::forks::activate,
            upgrades_known: Vec::new(),
            verifier: None,
            proof_pool: Vec::new(),
            recent: Default::default(),
            attempted: Default::default(),
            last_proof_check: None,
            proof_proposal: None,
            proof_backoff_until: 0,
            rejected: Default::default(),
        };
        (Chain(Arc::new(Mutex::new(inner))), genesis)
    }

    /// Open with durable state: resume from the stored checkpoint, or start at
    /// genesis and persist it.
    pub fn open(cfg: ChainConfig, store: Store) -> Result<(Self, Block), StoreError> {
        let (chain, genesis) = Self::new(cfg);
        // The store belongs to one genesis: refuse data of another instead of diverging from it.
        let ours = genesis_digest(&genesis);
        match store.meta(GENESIS)? {
            Some(d) if d.as_slice() != ours.as_slice() => return Err(StoreError::OtherGenesis),
            Some(_) => {}
            // Only a new store takes this genesis; an unmarked one with a chain is of unknown origin.
            None if store.head()?.is_none() => store.put_meta(GENESIS, &ours)?,
            None => return Err(StoreError::OtherGenesis),
        }
        let store = Arc::new(store);
        match store.load()? {
            Some(cp) => {
                use commonware_codec::DecodeExt;
                let digest = Digest::decode(cp.digest.as_slice()).map_err(|_| StoreError::Corrupt("digest"))?;
                let summary = cp.blocks.get(&cp.height).cloned();
                let mut state = cp.state;
                state.clear_journal();
                let exec = Arc::new(Executed {
                    height: cp.height,
                    digest,
                    timestamp: summary.as_ref().map(|b| b.timestamp_ms).unwrap_or_default(),
                    state,
                    receipts: vec![],
                    tx_hashes: summary.as_ref().map(|b| b.txs.clone()).unwrap_or_default(),
                    gas: GasVector::default(),
                    proposer: summary.as_ref().map(|b| b.proposer).unwrap_or_default(),
                    base_fee: summary.as_ref().map(|b| b.base_fee).unwrap_or_default(),
                    excess: summary.as_ref().map(|b| b.excess).unwrap_or_default(),
                    handoff: cp.handoff.map(Arc::new),
                    seed: cp.seed.map(Arc::new),
                    history: Arc::new(cp.history),
                    schedule: Arc::new(cp.schedule),
                    statement: cp.statement,
                    payouts: vec![],
                });
                let mut g = chain.lock();
                g.executed.insert(digest, exec.clone());
                // History proofs need every block from genesis; a checkpoint-started node has none before it.
                let h = ChainHasher::new();
                g.history_leaves = (0..=exec.height)
                    .map(|k| {
                        cp.blocks
                            .get(&k)
                            .and_then(|b| hex::decode(&b.hash).ok())
                            .and_then(|d| <[u8; 32]>::try_from(d).ok())
                            .map(|d| aether_state::mmr::leaf(&h, k, &d))
                    })
                    .collect::<Option<Vec<_>>>()
                    .map(Arc::new);
                g.finalized = exec;
                g.blocks = cp.blocks;
                g.receipts = cp.receipts;
                g.store = Some(store);
            }
            None => {
                let mut g = chain.lock();
                let genesis_exec = g.finalized.clone();
                let summary = g.blocks.get(&0).cloned().expect("genesis summary");
                store.commit(Commit {
                    height: 0,
                    digest: digest_bytes(&genesis_exec.digest),
                    root: genesis_exec.state.root(),
                    diff: genesis_exec.state.journal(),
                    summary: &summary,
                    receipts: vec![],
                    handoff: None,
                    seed: None,
                    history: &genesis_exec.history,
                    schedule: &genesis_exec.schedule,
                    statement: &genesis_exec.statement,
                })?;
                g.store = Some(store);
            }
        }
        Ok((chain, genesis))
    }

    pub fn finalized_height(&self) -> u64 {
        self.lock().finalized.height
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().expect("chain lock poisoned")
    }

    pub fn get(&self, d: &Digest) -> Option<Arc<Executed>> {
        self.lock().executed.get(d).cloned()
    }

    /// The durable store, if any (finality proofs a follower kept).
    pub fn store(&self) -> Option<Arc<Store>> {
        self.lock().store.clone()
    }

    pub fn cfg(&self) -> ChainConfig {
        self.lock().cfg.clone()
    }

    /// Key rounds only go up: a handoff built on `parent` must carry a round
    /// above the last handoff's in its ancestry (so none is accepted twice).
    fn round_floor(parent: &Executed) -> u64 {
        parent.handoff.as_ref().map(|p| p.handoff.round).unwrap_or(0)
    }

    /// Whether this node's voting set has handed over before the block after
    /// `parent`: it then builds and votes for nothing past the switch.
    pub fn retired_after(&self, parent: &Executed) -> bool {
        let start = self.lock().epoch_start;
        parent.handoff.as_ref().is_some_and(|p| p.switch > start && parent.height + 1 >= p.switch)
    }

    /// The handoff state after `block`: the parent's, or the one it carries
    /// (valid only when signed by the committee and no other is still pending).
    fn next_handoff(
        &self,
        height: u64,
        parent: &Executed,
        carried: Option<&aether_light::block::Handoff>,
    ) -> Result<Option<Arc<crate::handoff::Pending>>, ChainError> {
        let Some(h) = carried else { return Ok(parent.handoff.clone()) };
        if parent.handoff.as_ref().is_some_and(|p| height < p.switch) {
            return Err(ChainError::BadHandoff("another handoff is still pending".into()));
        }
        // Rounds only go up: a signed handoff cannot be replayed later.
        if h.round <= Self::round_floor(parent) {
            return Err(ChainError::BadHandoff(format!("handoff round {} is not above {}", h.round, Self::round_floor(parent))));
        }
        let (identity, chain_id) = {
            let g = self.lock();
            (g.identity, g.cfg.chain_id)
        };
        let identity = identity.ok_or_else(|| ChainError::BadHandoff("no committee identity (devnet dealer keys)".into()))?;
        crate::handoff::verify(chain_id, &identity, h).map_err(ChainError::BadHandoff)?;
        Ok(Some(Arc::new(crate::handoff::Pending { at: height, switch: height + crate::handoff::DELAY, handoff: h.clone() })))
    }

    /// After a restart: a finalized handoff still ends this node's epoch at its
    /// switch, and the voting set proposed for this registry epoch still stands.
    pub fn resume(&self) {
        let mut g = self.lock();
        if let Some(p) = g.finalized.handoff.clone() {
            if p.switch > g.epoch_start {
                g.epoch_end.store(p.switch - 1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let current = current_draw(&g.finalized.state, g.finalized.height);
        let load = |key: &str| g.store.as_ref().and_then(|s| s.meta(key).ok().flatten());
        let proposal: Option<(u64, Vec<(String, String)>)> = load(PROPOSAL).and_then(|b| serde_json::from_slice(&b).ok()).flatten();
        let pool: Option<(u64, Vec<(String, String)>)> = load(POOL).and_then(|b| serde_json::from_slice(&b).ok()).flatten();
        g.proposal = proposal.filter(|(d, _)| *d == current);
        g.pool = pool.filter(|(d, _)| *d == current);
    }

    /// The seed state after a block at `height`: the parent's, or the one it
    /// carries (only the current draw's, once, signed by the committee).
    pub fn next_seed(
        &self,
        height: u64,
        parent: &Executed,
        carried: Option<&aether_light::block::Seed>,
    ) -> Result<Option<Arc<(u64, aether_light::block::Seed)>>, ChainError> {
        let Some(s) = carried else { return Ok(parent.seed.clone()) };
        let draw = current_draw(&parent.state, height);
        if s.draw != draw || draw == 0 || parent.seed.as_ref().is_some_and(|p| p.1.draw >= draw) {
            return Err(ChainError::BadHandoff(format!("seed for draw {} in draw {draw}", s.draw)));
        }
        let (identity, chain_id) = {
            let g = self.lock();
            (g.identity, g.cfg.chain_id)
        };
        let identity = identity.ok_or_else(|| ChainError::BadHandoff("no committee identity (devnet dealer keys)".into()))?;
        crate::handoff::verify_seed(chain_id, &identity, s).map_err(ChainError::BadHandoff)?;
        Ok(Some(Arc::new((height, s.clone()))))
    }

    /// The draw seed to put in a block built on `parent`, if one is ready and not yet there.
    pub fn seed_for(&self, parent: &Executed) -> Option<aether_light::block::Seed> {
        let ready = self.lock().seed_ready.clone()?;
        let draw = current_draw(&parent.state, parent.height + 1);
        (ready.draw == draw && parent.seed.as_ref().is_none_or(|p| p.1.draw < draw)).then_some(ready)
    }

    /// The handoff to put in a block built on `parent`, if one is ready and allowed.
    pub fn handoff_for(&self, parent: &Executed) -> Option<aether_light::block::Handoff> {
        let ready = self.lock().handoff_ready.clone()?;
        if ready.round <= Self::round_floor(parent) {
            return None;
        }
        let pending = parent.handoff.as_ref();
        if pending.is_some_and(|p| parent.height + 1 < p.switch || p.handoff == ready) {
            return None;
        }
        Some(ready)
    }

    /// Execution context of `block` on top of `parent` (base fees derive from the parent's excess).
    pub fn block_context(cfg: &ChainConfig, block: &Block, parent: &Executed) -> BlockContext {
        let proposer = leader_address(&block.context.leader);
        let fees = cfg.fees.then(|| FeePolicy { base: fees::base_fee(parent.excess, cfg.limits), proposer });
        BlockContext {
            chain_id: cfg.chain_id,
            number: block.height().get(),
            timestamp: block.timestamp / 1000,
            beneficiary: if fees.is_some() { aether_execution::FEE_COLLECTOR } else { proposer },
            limits: cfg.limits,
            fees,
        }
    }

    /// Base fees the child of `parent` pays.
    pub fn next_base_fee(cfg: &ChainConfig, parent: &Executed) -> FeeVector {
        if cfg.fees {
            fees::base_fee(parent.excess, cfg.limits)
        } else {
            FeeVector::default()
        }
    }

    /// Validate and execute `block` on top of `parent`, remembering the result.
    pub fn execute(&self, block: &Block, parent: &Executed) -> Result<Arc<Executed>, ChainError> {
        self.execute_as(block, parent, false)
    }

    /// Execute a block the committee already finalized: its proofs were
    /// verified by the quorum that certified it, so this node applies them
    /// without asking its own verifier (whose local failure must never stop it
    /// from following the chain).
    fn execute_certified(&self, block: &Block, parent: &Executed) -> Result<Arc<Executed>, ChainError> {
        self.execute_as(block, parent, true)
    }

    fn execute_as(&self, block: &Block, parent: &Executed, certified: bool) -> Result<Arc<Executed>, ChainError> {
        if let Some(done) = self.get(&block.digest()) {
            return Ok(done);
        }
        let payload = block.payload().ok_or(ChainError::BadPayload)?;
        // Canonical bytes only: no unknown fields, no second encoding of the same block.
        if payload.to_bytes() != block.data {
            return Err(ChainError::BadPayload);
        }
        if payload.parent_state_root != parent.state.root() {
            return Err(ChainError::ParentRootMismatch);
        }
        if payload.history_root != B256::from(parent.history.root(&ChainHasher::new())) {
            return Err(ChainError::HistoryMismatch);
        }
        if payload.parent_meta != parent.meta_digest() {
            return Err(ChainError::HistoryMismatch);
        }
        if payload.txs.len() > MAX_TXS_PER_BLOCK {
            return Err(ChainError::BadPayload);
        }
        let (pre, payouts) = self.pre_state(parent, payload.version, &payload.proofs, certified)?;
        let schedule = self.next_schedule(block.height().get(), parent, payload.upgrade.as_ref())?;
        let cfg = self.cfg();
        let ctx = Self::block_context(&cfg, block, parent);
        let mut out = execute_block(&pre, &ctx, &payload.txs).map_err(|e| ChainError::Exec(format!("{e:?}")))?;
        // Protocol-1 blocks keep no statement (their metadata stays protocol-1).
        let statement = if payload.version >= 2 { statement(&ctx, &payload.txs, &pre, &out) } else { [0; 32] };
        with_activation(&pre, &mut out);
        if out.bal != payload.bal {
            return Err(ChainError::BalMismatch);
        }
        if out.gas != payload.gas {
            return Err(ChainError::GasMismatch);
        }
        let handoff = self.next_handoff(block.height().get(), parent, payload.handoff.as_ref())?;
        let seed = self.next_seed(block.height().get(), parent, payload.seed.as_ref())?;
        let tx_hashes = payload.txs.iter().map(aether_execution::tx_hash).collect();
        Ok(self.remember(block, parent, &ctx, out, tx_hashes, handoff, seed, schedule, statement, payouts))
    }

    /// The state the block after `parent` executes on: the parent's, plus the
    /// one-time changes of each protocol that activates at that block. Refuses
    /// a block whose version is not the scheduled one, or is newer than this node runs.
    #[allow(clippy::type_complexity)]
    pub fn pre_state<'a>(
        &self,
        parent: &'a Executed,
        version: u32,
        proofs: &[aether_light::block::ProofClaim],
        certified: bool,
    ) -> Result<(std::borrow::Cow<'a, WorldState>, Vec<(u64, Address, U256)>), ChainError> {
        let (running, migrate, verifier) = {
            let g = self.lock();
            (g.protocol, g.migrate, g.verifier.clone())
        };
        let want = parent.next_protocol();
        if version != want {
            return Err(ChainError::Protocol(format!("block under protocol {version}, scheduled {want}")));
        }
        if version > running {
            return Err(ChainError::Protocol(format!("UPGRADE REQUIRED: the chain runs protocol {version}, this node {running}")));
        }
        let before = crate::upgrade::protocol_at(&parent.schedule, parent.height);
        if version < 2 && !proofs.is_empty() {
            return Err(ChainError::Protocol("proofs before protocol 2".into()));
        }
        let records = version >= 2 && parent.statement != Statement::default();
        let rotates = parent.schedule.iter().any(|a| a.at == parent.height + 1 && a.registrar.is_some());
        if version == before && !records && !rotates && proofs.is_empty() {
            return Ok((std::borrow::Cow::Borrowed(&parent.state), vec![]));
        }
        let mut state = parent.state.clone();
        // Its journal then holds only these system writes (see `with_activation`).
        state.clear_journal();
        for p in before + 1..=version {
            migrate(p, &mut state).map_err(|e| ChainError::Protocol(format!("activating protocol {p}: {e}")))?;
        }
        // A committee-signed upgrade may replace the registrar key when it activates.
        for (x, y) in parent.schedule.iter().filter(|a| a.at == parent.height + 1).filter_map(|a| a.registrar) {
            aether_execution::registry::set_registrar(&mut state, (x.0, y.0));
        }
        if records {
            aether_execution::proofs::record(&mut state, parent.height, parent.statement.commitment, parent.statement.escrow);
            // The record of the block that just left the claim window goes (one per block, so state stays bounded).
            if let Some(old) = (parent.height + 1).checked_sub(aether_execution::proofs::EXPIRY + 1) {
                aether_execution::proofs::prune(&mut state, old);
            }
        }
        struct Certified;
        impl ProofVerifier for Certified {
            fn verify(&self, _: &[u8], _: [u8; 32]) -> bool {
                true
            }
        }
        let verifier: Option<&dyn ProofVerifier> = if certified { Some(&Certified) } else { verifier.as_deref() };
        let payouts = pay_proofs(&mut state, parent.height + 1, proofs, verifier)?;
        Ok((std::borrow::Cow::Owned(state), payouts))
    }

    /// The newest finalized block nobody proved yet that this node can build a
    /// prover input for (its parent's state is still in memory); taken once.
    pub fn provable(&self) -> Option<(Arc<Executed>, Arc<Executed>, Block)> {
        let mut g = self.lock();
        let head = g.finalized.clone();
        // Oldest first among the recent blocks: none left behind to expire.
        let pick = g.recent.iter().find_map(|b| {
            let h = b.height().get();
            if g.attempted.contains(&h) || aether_execution::proofs::prover(&head.state, h).is_some() || b.payload()?.version < 2 {
                return None;
            }
            Some((g.executed.get(&b.digest())?.clone(), g.executed.get(&b.parent)?.clone(), b.clone()))
        })?;
        g.attempted.insert(pick.0.height);
        Some(pick)
    }

    /// Rewards `prover` received for proofs (this node's record since it started keeping one).
    pub fn rewards(&self, prover: &Address) -> Vec<Value> {
        self.recent_rewards(prover, usize::MAX)
    }

    /// The newest `limit` rewards of `prover`, oldest first.
    pub fn recent_rewards(&self, prover: &Address, limit: usize) -> Vec<Value> {
        // Read the store without holding the chain lock (a long history must not stall consensus).
        let store = self.lock().store.clone();
        let rows = store.and_then(|s| s.rewards(&prover.0 .0, limit).ok()).unwrap_or_default();
        rows.iter().filter_map(|r| serde_json::from_slice(r).ok()).collect()
    }

    /// Keep a proof (verified by this node's verifier) for this node's next proposals.
    pub fn add_proof(&self, claim: aether_light::block::ProofClaim) -> Result<(), String> {
        self.add_proof_from(claim, false)
    }

    /// A proof this node's own prover made: no rate limit (nobody else can use this path).
    pub fn add_own_proof(&self, claim: aether_light::block::ProofClaim) -> Result<(), String> {
        self.add_proof_from(claim, true)
    }

    fn add_proof_from(&self, claim: aether_light::block::ProofClaim, own: bool) -> Result<(), String> {
        let open = |g: &Inner, h: u64| {
            let f = &g.finalized;
            aether_execution::proofs::claimable(&f.state, h, f.height + 1)
                .or_else(|e| if h == f.height && aether_execution::proofs::prover(&f.state, h).is_none() { Ok(f.statement.commitment) } else { Err(e) })
                .map_err(|e| format!("{e:?}"))
        };
        let (verifier, commitment) = {
            let mut g = self.lock();
            // Cheap refusals first: a block already covered, or verifying too often (this is a public RPC).
            if g.proof_pool.iter().any(|c| c.height == claim.height) {
                return Err("a proof of this block is already waiting".into());
            }
            // Malformed submissions never take a verification slot.
            if !(2 * MIN_PROOF_BYTES..=2 * MAX_PROOF_BYTES).contains(&claim.proof.len())
                || !claim.proof.len().is_multiple_of(2)
                || !claim.proof.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("the proof is not a hex string of an allowed size".into());
            }
            let now = Instant::now();
            if !own && g.last_proof_check.is_some_and(|t| now.duration_since(t) < PROOF_CHECK_INTERVAL) {
                return Err("busy; try again shortly".into());
            }
            let c = open(&g, claim.height)?;
            if !own {
                g.last_proof_check = Some(now);
            }
            (g.verifier.clone().ok_or("this node does not verify proofs")?, c)
        };
        let bytes = hex::decode(&claim.proof).map_err(|_| "proof is not hex")?;
        let output = aether_proving::block::claim(commitment, claim.prover);
        let key = *blake3::Hasher::new().update(&output).update(&bytes).finalize().as_bytes();
        if self.lock().rejected.contains(&key) {
            return Err("this proof was already refused".into());
        }
        match (bytes.len() <= MAX_PROOF_BYTES).then(|| verifier.decide(&bytes, output)).flatten() {
            Some(true) => {}
            Some(false) => {
                // A definite refusal is remembered for the public RPC only (block proofs are always checked afresh).
                let mut g = self.lock();
                if g.rejected.len() > 4096 {
                    g.rejected.clear();
                }
                g.rejected.insert(key);
                return Err("the proof does not verify".into());
            }
            None => return Err("the proof could not be checked now; try again".into()),
        }
        let mut g = self.lock();
        // Still open after the (slow) check: not proven or expired meanwhile.
        open(&g, claim.height)?;
        if !g.proof_pool.iter().any(|c| c.height == claim.height) {
            g.proof_pool.push(claim);
        }
        Ok(())
    }

    /// Verified proofs this node may put in the block after `parent` (first come,
    /// at most two, still claimable there), unless its recent proposals with
    /// proofs did not make it (some validators could not verify them).
    pub fn proofs_for(&self, parent: &Executed) -> Vec<aether_light::block::ProofClaim> {
        if parent.next_protocol() < 2 {
            return vec![];
        }
        let g = self.lock();
        if g.proof_backoff_until > parent.height {
            return vec![];
        }
        let next = parent.height + 1;
        let open = |h: u64| {
            aether_execution::proofs::prover(&parent.state, h).is_none()
                && (h == parent.height || aether_execution::proofs::claimable(&parent.state, h, next).is_ok())
        };
        let mut seen = std::collections::BTreeSet::new();
        g.proof_pool.iter().filter(|c| open(c.height) && seen.insert(c.height)).take(MAX_PROOFS_PER_BLOCK).cloned().collect()
    }

    /// A proposal carrying proofs at `height` was built; if a different block
    /// is finalized there, stop proposing proofs for a while.
    pub fn proposed_with_proofs(&self, height: u64, digest: Digest) {
        self.lock().proof_proposal = Some((height, digest));
    }

    /// Whether block `height` can still be proven (recorded or the head, not proven, not expired).
    pub fn proof_open(&self, height: u64) -> bool {
        let g = self.lock();
        let f = &g.finalized;
        aether_execution::proofs::prover(&f.state, height).is_none()
            && (height == f.height || aether_execution::proofs::claimable(&f.state, height, f.height + 1).is_ok())
    }

    /// Drop these proofs from the pool (they no longer verify here).
    pub fn drop_proofs(&self, heights: &[u64]) {
        self.lock().proof_pool.retain(|c| !heights.contains(&c.height));
    }

    /// Blocks' notice between an upgrade landing on chain and its activation:
    /// one voting-node epoch, so every running Mac sees it well before (the
    /// genesis epoch length on a chain without a registry).
    fn notice(state: &WorldState, genesis_epoch: u64) -> u64 {
        if state.code(&aether_execution::registry::REGISTRY).is_empty() && genesis_epoch > 0 {
            return genesis_epoch;
        }
        aether_execution::registry::params(state).epoch_blocks
    }

    /// Whether `u` may go on chain in the block after `parent`: signed for this
    /// chain, a newer protocol than any scheduled, activating after the last one
    /// and at least `notice` blocks later, and of bounded size.
    fn admissible_upgrade(parent: &Executed, cfg: &ChainConfig, u: &crate::upgrade::Upgrade) -> Result<(), String> {
        // Protocol-1 nodes cannot read a registrar change: it may only be announced under protocol 2.
        if u.registrar.is_some() && parent.next_protocol() < 2 {
            return Err("a registrar change needs protocol 2".into());
        }
        let chain_id = cfg.chain_id;
        let notice = Self::notice(&parent.state, cfg.epoch_blocks);
        use crate::upgrade::{MAX_FIELD, MAX_RELEASES};
        let height = parent.height + 1;
        let (last_protocol, last_at) = parent.schedule.last().map(|a| (a.protocol, a.at)).unwrap_or((1, 0));
        if u.chain_id != chain_id {
            return Err("upgrade for another chain".into());
        }
        if u.protocol <= last_protocol || u.activate_at <= last_at {
            return Err(format!("protocol {} at {} is not after {last_protocol} at {last_at}", u.protocol, u.activate_at));
        }
        if u.activate_at < height.saturating_add(notice) {
            return Err(format!("activation at {} gives less than {notice} blocks of notice", u.activate_at));
        }
        let long = |s: &String| s.len() > MAX_FIELD;
        if u.releases.len() > MAX_RELEASES
            || long(&u.notes)
            || u.releases.iter().any(|r| long(&r.platform) || long(&r.version) || long(&r.blake3) || long(&r.url))
        {
            return Err("upgrade too large".into());
        }
        Ok(())
    }

    /// The activation schedule after a block at `height`: the parent's, plus the
    /// upgrade it carries (checked and verified under the committee identity).
    fn next_schedule(
        &self,
        height: u64,
        parent: &Executed,
        carried: Option<&crate::upgrade::SignedUpgrade>,
    ) -> Result<Arc<crate::upgrade::Schedule>, ChainError> {
        let Some(s) = carried else { return Ok(parent.schedule.clone()) };
        debug_assert_eq!(height, parent.height + 1);
        let (identity, cfg) = {
            let g = self.lock();
            (g.identity, g.cfg.clone())
        };
        Self::admissible_upgrade(parent, &cfg, &s.upgrade).map_err(ChainError::Protocol)?;
        let identity = identity.ok_or_else(|| ChainError::Protocol("no committee identity (devnet dealer keys)".into()))?;
        crate::upgrade::verify(&identity, s).map_err(ChainError::Protocol)?;
        Ok(scheduled(&parent.schedule, &s.upgrade))
    }

    /// The upgrade to put in a block built on `parent`: the first known one not
    /// yet on chain that may go there now.
    pub fn upgrade_for(&self, parent: &Executed) -> Option<crate::upgrade::SignedUpgrade> {
        let g = self.lock();
        g.upgrades_known.iter().find(|s| Self::admissible_upgrade(parent, &g.cfg, &s.upgrade).is_ok()).cloned()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn remember(
        &self,
        block: &Block,
        parent: &Executed,
        ctx: &BlockContext,
        out: BlockOutcome,
        tx_hashes: Vec<TxHash>,
        handoff: Option<Arc<crate::handoff::Pending>>,
        seed: Option<Arc<(u64, aether_light::block::Seed)>>,
        schedule: Arc<crate::upgrade::Schedule>,
        statement: [u8; 32],
        payouts: Vec<(u64, Address, U256)>,
    ) -> Arc<Executed> {
        let escrow = out.settlement.to_escrow;
        let (base_fee, excess) = match &ctx.fees {
            Some(f) => (f.base, fees::next_excess(parent.excess, out.gas, ctx.limits)),
            None => (FeeVector::default(), GasVector::default()),
        };
        let exec = Arc::new(Executed {
            height: block.height().get(),
            digest: block.digest(),
            timestamp: block.timestamp,
            state: out.state,
            receipts: out.receipts,
            tx_hashes,
            gas: out.gas,
            proposer: leader_address(&block.context.leader),
            base_fee,
            excess,
            handoff,
            seed,
            history: Arc::new(parent.history.append(&ChainHasher::new(), block.height().get(), &digest_bytes(&block.digest()))),
            schedule,
            statement: if statement == [0; 32] { Statement::default() } else { Statement { commitment: statement, escrow } },
            payouts,
        });
        self.lock().executed.insert(block.digest(), exec.clone());
        exec
    }

    /// Candidate txs for a proposal: inclusion-list txs first, then the mempool
    /// (lowest nonce first per sender).
    pub fn mempool_candidates(&self) -> Vec<TxEnvelope> {
        let g = self.lock();
        let censored = |t: &TxEnvelope| g.censor == Some(t.header.sender) || g.deprioritize == Some(t.header.sender);
        let listed = if g.censor.is_some() { Vec::new() } else { g.inclusion.for_proposal() };
        let listed_hashes: std::collections::HashSet<TxHash> = listed.iter().map(aether_execution::tx_hash).collect();
        let mut rest: Vec<TxEnvelope> = g.mempool.iter().filter(|(h, t)| !listed_hashes.contains(*h) && !censored(t)).map(|(_, t)| t.clone()).collect();
        rest.sort_by_key(|t| (t.header.nonce, t.header.sender));
        let mut txs = listed;
        txs.extend(rest);
        txs.truncate(MAX_TXS_PER_BLOCK);
        txs
    }

    /// The oldest mempool txs waiting at least `min_age`, for this node's inclusion list.
    pub fn inclusion_candidates(&self, min_age: Duration, now: Instant) -> Vec<TxEnvelope> {
        let g = self.lock();
        let mut waiting: Vec<(Instant, TxHash)> =
            g.arrivals.iter().filter(|(h, t)| now.saturating_duration_since(**t) >= min_age && g.mempool.contains_key(*h)).map(|(h, t)| (*t, *h)).collect();
        waiting.sort();
        waiting.into_iter().take(inclusion::MAX_IL_TXS).filter_map(|(_, h)| g.mempool.get(&h).cloned()).collect()
    }

    /// Listed txs that `exec` (a verified block) wrongly left out. Empty for a
    /// censoring devnet node, which ignores the lists.
    pub fn inclusion_violations(&self, exec: &Executed, ctx: &BlockContext, now: Instant) -> Vec<TxHash> {
        let listed = {
            let g = self.lock();
            if g.censor.is_some() {
                return Vec::new();
            }
            g.inclusion.enforceable(now)
        };
        let full = exec.tx_hashes.len() >= MAX_TXS_PER_BLOCK;
        inclusion::violations(&listed, &exec.tx_hashes, full, &exec.state, ctx, exec.gas)
    }

    /// Returns false if the pool is full or the tx is already known.
    /// Admit a (signature-checked) tx: `Ok(true)` if new, `Ok(false)` if already
    /// known, `Err` if it could never execute or the pool is full, so txs that
    /// would only ever be skipped cannot pile up for free.
    pub fn add_to_mempool(&self, tx: TxEnvelope) -> Result<bool, String> {
        let h = aether_execution::tx_hash(&tx);
        let mut g = self.lock();
        if g.receipts.contains_key(&h) || g.mempool.contains_key(&h) {
            return Ok(false);
        }
        if g.mempool.len() >= MAX_MEMPOOL {
            return Err("mempool full".into());
        }
        let state = &g.finalized.state;
        if tx.header.nonce < state.nonce(&tx.header.sender) {
            return Err("nonce already used".into());
        }
        if g.cfg.fees {
            admissible(&tx, state, Self::next_base_fee(&g.cfg, &g.finalized))?;
        }
        if g.pending_by_sender.get(&tx.header.sender).copied().unwrap_or(0) >= MAX_PER_SENDER {
            return Err(format!("sender has {MAX_PER_SENDER} pending transactions"));
        }
        *g.pending_by_sender.entry(tx.header.sender).or_default() += 1;
        g.mempool.insert(h, tx);
        g.arrivals.insert(h, Instant::now());
        Ok(true)
    }

    /// Adopt a finalized block (delivered in order by marshal) and persist it.
    pub fn finalize(&self, block: &Block) -> Result<(), ChainError> {
        let height = block.height().get();
        {
            let g = self.lock();
            if height <= g.finalized.height && height != 0 {
                // At-least-once delivery, or already restored from disk: must be the same block.
                let ours = g.blocks.get(&height).map(|b| b.hash.clone());
                if ours.is_some_and(|h| h != format!("{}", block.digest())) {
                    return Err(ChainError::ConflictingFinality { height });
                }
                return Ok(());
            }
        }
        let exec = match self.get(&block.digest()) {
            Some(e) => e,
            None => {
                // Not verified locally (e.g. backfilled): execute on the finalized parent.
                let parent = self.lock().finalized.clone();
                if parent.digest != block.parent {
                    return Err(ChainError::UnknownParent);
                }
                self.execute_certified(block, &parent)?
            }
        };
        let payload = block.payload().ok_or(ChainError::BadPayload)?;
        let summary = summary(block, &exec, payload.parent_state_root);
        let store = self.lock().store.clone();
        if let Some(store) = store {
            // Disk first: the in-memory head never runs ahead of what survives a crash.
            store
                .commit(Commit {
                    height: exec.height,
                    digest: digest_bytes(&exec.digest),
                    root: exec.state.root(),
                    diff: exec.state.journal(),
                    summary: &summary,
                    receipts: exec.tx_hashes.iter().copied().zip(exec.receipts.iter()).collect(),
                    handoff: exec.handoff.as_deref().filter(|p| p.at == exec.height),
                    seed: exec.seed.as_deref().filter(|s| s.0 == exec.height),
                    history: &exec.history,
                    schedule: &exec.schedule,
                    statement: &exec.statement,
                })
                .map_err(|e| ChainError::Store(e.to_string()))?;
        }
        let mut g = self.lock();
        for (h, r) in exec.tx_hashes.iter().zip(&exec.receipts) {
            g.receipts.insert(*h, (exec.height, r.clone()));
            g.mempool.remove(h);
        }
        let state = exec.state.clone();
        g.mempool.retain(|_, tx| tx.header.nonce >= state.nonce(&tx.header.sender));
        let inner = &mut *g;
        inner.arrivals.retain(|h, _| inner.mempool.contains_key(h));
        inner.pending_by_sender.clear();
        for t in inner.mempool.values() {
            *inner.pending_by_sender.entry(t.header.sender).or_default() += 1;
        }
        inner.inclusion.prune(&state, exec.height, Instant::now());
        // One leaf per height, in order (genesis is delivered again at start-up).
        if let Some(leaves) = g.history_leaves.as_mut().filter(|l| l.len() as u64 == exec.height) {
            Arc::make_mut(leaves).push(aether_state::mmr::leaf(&ChainHasher::new(), exec.height, &digest_bytes(&exec.digest)));
        }
        g.blocks.insert(exec.height, summary);
        let previous = std::mem::replace(&mut g.finalized, exec.clone());
        // A finalized handoff ends the running epoch before its switch height.
        if let Some(p) = exec.handoff.as_ref().filter(|p| p.at == exec.height) {
            if p.switch > g.epoch_start {
                g.epoch_end.store(p.switch - 1, std::sync::atomic::Ordering::SeqCst);
            }
            if g.handoff_ready.as_ref() == Some(&p.handoff) {
                g.handoff_ready = None;
            }
            // This draw's voting set is on its way in: nobody reshares to it again.
            g.proposal = None;
            keep(&g.store, PROPOSAL, &g.proposal);
        }
        // A draw's pool is frozen from the state its first block builds on (before its seed exists).
        let params = aether_execution::registry::params(&previous.state);
        let span = params.epoch_blocks * params.draw_epochs;
        if exec.height > 0 && exec.height.is_multiple_of(span) {
            let pool = crate::rotation::eligible(&previous.state, exec.height / params.epoch_blocks, params.min_streak);
            g.pool = Some((exec.height / span, pool));
            g.proposal = None;
            keep(&g.store, POOL, &g.pool);
        }
        // With the draw's seed on chain, everyone draws the same next voting set.
        if let Some(s) = exec.seed.as_ref().filter(|s| s.0 == exec.height) {
            if g.seed_ready.as_ref() == Some(&s.1) {
                g.seed_ready = None;
            }
            if let Some((draw, pool)) = g.pool.clone().filter(|(d, _)| *d == s.1.draw) {
                let seed = hex::decode(&s.1.signature).unwrap_or_default();
                // The per-operator seat cap is a protocol-2 rule: before it, every key is its own operator.
                let ops = crate::rotation::operators(&exec.state);
                let capped = exec.next_protocol() >= 2;
                g.proposal = crate::rotation::draw(&pool, &seed, |k| ops.get(k).map(|o| if capped { o.clone() } else { k.to_string() }), &g.committee)
                    .map(|m| (draw, m));
                keep(&g.store, PROPOSAL, &g.proposal);
            }
        }
        let floor = exec.height.saturating_sub(64);
        g.executed.retain(|_, e| e.height >= floor);
        for (proven, prover, amount) in &exec.payouts {
            let record = serde_json::json!({ "proven": proven, "amount": amount, "height": exec.height, "timestamp_ms": exec.timestamp });
            if let Some(Err(e)) = g.store.as_ref().map(|s| s.put_reward(&prover.0 .0, exec.height, *proven, record.to_string().as_bytes())) {
                tracing::warn!(%e, "could not keep a reward record");
            }
        }
        // Our proposal with proofs lost at this height: others may not verify them.
        if g.proof_proposal.is_some_and(|(h, d)| h == exec.height && d != exec.digest) {
            g.proof_backoff_until = exec.height + PROOF_BACKOFF;
            g.proof_proposal = None;
        }
        g.recent.push_back(block.clone());
        while g.recent.len() > 32 {
            g.recent.pop_front();
        }
        // Proofs of blocks now proven (or expired) leave the pool.
        let now = exec.height;
        let state = &exec.state;
        g.proof_pool.retain(|c| aether_execution::proofs::claimable(state, c.height, now + 1).is_ok() || c.height == now);
        g.attempted.retain(|h| *h + 64 >= now);
        Ok(())
    }
}

/// The fee-policy checks a block would reject the tx for, applied at admission:
/// caps at least the current base fees (zero when uncongested), a prove budget covering the gas limit, and a
/// balance covering value, max exec fee and prove budget.
fn admissible(tx: &TxEnvelope, state: &WorldState, base: FeeVector) -> Result<(), String> {
    let aether_types::TxPayload::Plain(bytes) = &tx.payload else { return Err("encrypted payloads are not supported yet".into()) };
    let call = aether_execution::EvmCall::decode(bytes).map_err(|e| format!("payload: {e:?}"))?;
    if tx.header.max_fee.exec < base.exec || tx.header.max_fee.prove < base.prove {
        return Err("fee caps below the base fee".into());
    }
    if tx.header.gas.prove < call.gas_limit {
        return Err("prove budget below the gas limit".into());
    }
    let need = U256::from(call.gas_limit)
        .checked_mul(U256::from(tx.header.max_fee.exec))
        .and_then(|g| U256::from(tx.header.gas.prove).checked_mul(U256::from(base.prove)).and_then(|p| g.checked_add(p)))
        .and_then(|n| n.checked_add(call.value));
    if need.is_none_or(|n| state.balance(&tx.header.sender) < n) {
        return Err("insufficient funds for value, gas and prove budget".into());
    }
    Ok(())
}

fn digest_bytes(d: &Digest) -> [u8; 32] {
    d.as_ref().try_into().expect("sha256 digest is 32 bytes")
}

fn summary(block: &Block, e: &Executed, parent_state_root: B256) -> BlockSummary {
    BlockSummary {
        height: e.height,
        hash: format!("{}", block.digest()),
        parent: format!("{}", block.parent),
        timestamp_ms: block.timestamp,
        proposer: e.proposer,
        state_root: e.state.root(),
        parent_state_root,
        txs: e.tx_hashes.clone(),
        gas_used: e.gas.exec,
        prove_gas: e.gas.prove,
        base_fee: e.base_fee,
        excess: e.excess,
    }
}

/// Store keys: the proposed voting set, the frozen draw pool, and the genesis the data belongs to.
const PROPOSAL: &str = "proposal";
pub const GENESIS: &str = "genesis";

/// The genesis block hash a store is marked with.
pub fn genesis_digest(genesis: &Block) -> [u8; 32] {
    digest_bytes(&genesis.digest())
}
const POOL: &str = "pool";

/// The draw a block at `height` belongs to (draws start at multiples of epoch_blocks × draw_epochs).
fn current_draw(state: &WorldState, height: u64) -> u64 {
    let p = aether_execution::registry::params(state);
    height / (p.epoch_blocks * p.draw_epochs)
}

fn keep<T: Serialize + ?Sized>(store: &Option<Arc<Store>>, key: &str, value: &T) {
    if let Some(store) = store {
        if let Err(e) = store.put_meta(key, &serde_json::to_vec(value).expect("serializes")) {
            tracing::warn!(%e, key, "could not keep the voting-set draw state");
        }
    }
}

/// What a proposal carries besides transactions.
#[derive(Default)]
pub struct Extras {
    pub handoff: Option<aether_light::block::Handoff>,
    pub seed: Option<aether_light::block::Seed>,
    pub upgrade: Option<crate::upgrade::SignedUpgrade>,
    pub proofs: Vec<aether_light::block::ProofClaim>,
}

/// Build a payload on `parent`; `pre` is `Chain::pre_state` for the parent's next protocol.
pub fn build_payload(
    parent: &Executed,
    pre: &WorldState,
    ctx: &BlockContext,
    candidates: Vec<TxEnvelope>,
    extras: Extras,
) -> (Payload, aether_execution::BlockOutcome) {
    let (txs, out) = aether_execution::build_block(pre, ctx, candidates);
    let history_root = B256::from(parent.history.root(&ChainHasher::new()));
    let parent_meta = parent.meta_digest();
    let Extras { handoff, seed, upgrade, proofs } = extras;
    let payload = Payload {
        version: parent.next_protocol(),
        parent_state_root: parent.state.root(),
        history_root,
        parent_meta,
        txs,
        bal: out.bal.clone(),
        gas: out.gas,
        handoff,
        seed,
        upgrade,
        proofs,
    };
    (payload, out)
}

/// An activation block's persisted diff starts with the activation's writes
/// (execution clears the journal it starts from).
#[allow(clippy::ptr_arg)] // Owned (migrated) vs Borrowed (untouched) is what matters here
pub fn with_activation(pre: &std::borrow::Cow<'_, WorldState>, out: &mut BlockOutcome) {
    if let std::borrow::Cow::Owned(migrated) = pre {
        out.state.prepend_journal(migrated.journal());
    }
}

/// The statement commitment of a block that ran `txs` on `pre` under `ctx`.
/// Its pre-state is the one the transactions ran on (after the block's system
/// writes: activation, records, payouts, which validators check directly).
pub fn statement(ctx: &BlockContext, txs: &[TxEnvelope], pre: &WorldState, out: &BlockOutcome) -> [u8; 32] {
    aether_proving::block::BlockStatement {
        ctx: ctx.clone(),
        txs_hash: aether_proving::block::txs_hash(txs),
        activate: vec![],
        pre_state_root: pre.root(),
        post_state_root: out.state.root(),
        gas: out.gas,
    }
    .commitment()
}

/// Check and pay a block's proofs (at most two, distinct heights, bounded,
/// each verified against the recorded statement of its block).
fn pay_proofs(
    state: &mut WorldState,
    height: u64,
    proofs: &[aether_light::block::ProofClaim],
    verifier: Option<&dyn ProofVerifier>,
) -> Result<Vec<(u64, Address, U256)>, ChainError> {
    let bad = |m: String| ChainError::Protocol(m);
    if proofs.len() > MAX_PROOFS_PER_BLOCK {
        return Err(bad(format!("more than {MAX_PROOFS_PER_BLOCK} proofs")));
    }
    let mut paid = Vec::new();
    for c in proofs {
        let verifier = verifier.ok_or_else(|| bad("no proof verifier on this node".into()))?;
        if c.proof.len() > 2 * MAX_PROOF_BYTES || c.height >= height {
            return Err(bad(format!("proof of block {} is out of bounds", c.height)));
        }
        let bytes = hex::decode(&c.proof).map_err(|_| bad("proof is not hex".into()))?;
        let commitment = aether_execution::proofs::claimable(state, c.height, height).map_err(|e| bad(format!("proof of block {}: {e:?}", c.height)))?;
        // The proof's output binds the payout address: nobody can reroute it.
        if !verifier.verify(&bytes, aether_proving::block::claim(commitment, c.prover)) {
            return Err(bad(format!("proof of block {} does not verify", c.height)));
        }
        let amount = aether_execution::proofs::pay(state, c.height, height, c.prover).map_err(|e| bad(format!("{e:?}")))?;
        paid.push((c.height, c.prover, amount));
    }
    Ok(paid)
}

/// `schedule` with `u`'s activation appended.
pub fn scheduled(schedule: &[crate::upgrade::Activation], u: &crate::upgrade::Upgrade) -> Arc<crate::upgrade::Schedule> {
    let mut next = schedule.to_vec();
    next.push(crate::upgrade::Activation { protocol: u.protocol, at: u.activate_at, registrar: u.registrar });
    Arc::new(next)
}
