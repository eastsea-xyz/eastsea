//! Chain state shared by the consensus application, RPC and gossip.
//!
//! Every verified or proposed block's post-state is kept by digest so
//! competing forks can be validated; the finalized head is what RPC serves.

use crate::block::{Block, Payload, PublicKey};
use crate::inclusion::{self, InclusionPool};
use crate::store::{Commit, Store, StoreError};
use aether_crypto::{address_of, PublicKey as AetherPk};
use aether_execution::{
    execute_block, fees, BlockContext, BlockOutcome, FeePolicy, Receipt, WorldState,
};
use aether_hash::ChainHasher;
use aether_types::{Address, Canonical, FeeVector, GasVector, SignerScheme, TxEnvelope, TxHash, B256, U256};
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
/// Largest tx admitted, by its canonical encoding. Contract deploys are the
/// biggest legitimate payloads and the EVM caps their initcode at 49 KiB
/// (EIP-3860), so 128 KiB takes any real call while keeping 50,000 txs well
/// under a block's 8 MiB decode cap. Local admission policy, not consensus.
pub const MAX_TX_BYTES: usize = 128 * 1024;
/// All pending txs together, by the same count: their encodings' bytes. Like
/// `MAX_MEMPOOL`, it rejects new txs when full (nothing is evicted).
pub const MAX_MEMPOOL_BYTES: usize = 64 * 1024 * 1024;
/// A pending tx that has not made it into a block in this long leaves the pool
/// (a nonce gap it cannot close, or fee caps the base fee stays above).
pub const MEMPOOL_TTL: Duration = Duration::from_secs(10 * 60);

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
    /// History v2, for a new genesis only (docs/research/history-compression-2026.md,
    /// roadmap B): a block with no transactions and no proofs records no
    /// proof-market statement, so runs of empty blocks leave the state root and
    /// chain metadata unchanged and cost a few bytes each in era files; the
    /// node also seals every 8192 finalized blocks into an era file. Bound to
    /// the genesis hash, so nodes can never disagree on it. Off on 7780.
    pub history_v2: bool,
    /// Node rewards (docs/design/15-node-rewards.md): half of each block's
    /// issuance to the operators whose Macs beaconed, half to provers, 1/16 cap
    /// per operator. A new network's genesis parameter (needs the registry);
    /// off keeps the testnet's rules and genesis.
    pub node_rewards: bool,
    /// Founder reserve keys (docs/design/12-launch-plan.md, "창업자 Mac 안전망";
    /// needs node rewards): up to three voting keys seated only while fewer
    /// than four independent operators qualify for the voting set.
    pub reserve: Option<Reserve>,
}

/// The founder's reserve validator keys, a genesis parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reserve {
    /// The founder's operator address (its registered Macs are not independent).
    pub operator: Address,
    /// (ed25519 key hex, iroh node id) of each reserve key.
    pub members: Vec<(String, String)>,
}

impl Reserve {
    /// (key, node id) bytes of each reserve key.
    pub fn bytes(&self) -> Result<Vec<aether_rewards::ReserveKey>, String> {
        self.members
            .iter()
            .map(|(k, n)| {
                let key: [u8; 32] = hex::decode(k.trim_start_matches("0x"))
                    .ok()
                    .and_then(|b| b.try_into().ok())
                    .ok_or_else(|| format!("reserve key {k}: 32-byte hex"))?;
                let node = n
                    .parse::<aether_net::EndpointId>()
                    .map_err(|e| format!("reserve node {n}: {e}"))?;
                Ok((key, *node.as_bytes()))
            })
            .collect()
    }

    /// The reserve set at genesis in `state`, if any.
    pub fn of(state: &WorldState) -> Option<crate::rotation::Reserve> {
        let (operator, keys) = aether_rewards::reserve(state)?;
        Some(crate::rotation::Reserve {
            operator: format!("{operator:#x}"),
            members: keys
                .iter()
                .filter_map(|(k, n)| {
                    aether_net::EndpointId::from_bytes(n)
                        .ok()
                        .map(|id| (hex::encode(k), id.to_string()))
                })
                .collect(),
        })
    }
}

impl ChainConfig {
    pub fn genesis_state(&self) -> WorldState {
        let mut s = WorldState::default();
        for (a, v) in &self.alloc {
            s.set_balance(*a, *v).expect("genesis balance fits u128");
        }
        // The account contract P-256 accounts delegate to (EIP-7702) for batched calls.
        s.set_code(
            aether_execution::AETHER_ACCOUNT,
            aether_execution::aether_account_code(),
        )
        .expect("predeploy");
        if let Some(key) = self.registrar {
            let d = aether_execution::registry::Params::default();
            let params = aether_execution::registry::Params {
                epoch_blocks: if self.epoch_blocks == 0 {
                    d.epoch_blocks
                } else {
                    self.epoch_blocks
                },
                min_streak: self.min_streak.unwrap_or(d.min_streak),
                draw_epochs: self.draw_epochs.unwrap_or(d.draw_epochs),
            };
            aether_execution::registry::predeploy(&mut s, key, params).expect("registry predeploy");
            if self.node_rewards {
                aether_rewards::enable(&mut s);
                if let Some(r) = &self.reserve {
                    let keys = r.bytes().expect("valid reserve keys");
                    aether_rewards::set_reserve(&mut s, r.operator, &keys)
                        .expect("reserve keys");
                }
            }
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
            (
                i,
                address_of(&aether_crypto::Signer::public_key(&s)).expect("address"),
            )
        })
        .collect()
}

pub fn leader_address(leader: &PublicKey) -> Address {
    let pk = AetherPk {
        scheme: SignerScheme::Ed25519,
        bytes: leader.as_ref().to_vec(),
    };
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
    /// Proofs this block paid: (proven height, prover, amount). Node rewards
    /// paid by the first block of an epoch appear with the block's own height
    /// as the proven height (no block proves itself).
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
        meta_digest(
            &self.excess,
            self.handoff.as_deref(),
            self.seed.as_deref(),
            &self.schedule,
            &self.statement,
        )
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
    /// Each mempool tx's encoded size, and their sum: the pool's byte budget
    /// (`MAX_MEMPOOL_BYTES`) charges and releases exactly what was admitted.
    sizes: HashMap<TxHash, usize>,
    mempool_bytes: usize,
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
    /// Roots of the complete 8192-block eras and the open era's MMR leaves,
    /// from genesis (history proofs read at most two eras' blocks, never all).
    /// None when this node started from a checkpoint (it has no early blocks).
    pub history_index: Option<Arc<aether_state::mmr::EraIndex>>,
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
    /// Beacon answers checked against the finalized state, waiting for a
    /// block: by (epoch, slot, candidate index).
    beacon_pool: BTreeMap<(u64, u64, u64), aether_light::block::BeaconAnswer>,
    /// Where answers this node takes first go out to the other validators (validators only).
    pub beacon_out: Option<tokio::sync::mpsc::UnboundedSender<aether_light::block::BeaconAnswer>>,
    /// Pruning (roadmap B4): first height whose summary and receipts are kept.
    pub pruned_below: u64,
    /// The last era read back from its file (old blocks served over RPC).
    era_cache: Option<(u64, Arc<crate::era::Era>)>,
    /// The network's finalized height, as this node last heard from its
    /// upstream (None: nothing told it). How far behind it is shows in
    /// `aether_status` (`catching_up`, `behind`) and gates acting as a
    /// validator while still catching up.
    pub net_height: Option<u64>,
    /// Replay mode (`follow` catching up): commits skip the per-block fsync.
    /// Every block being replayed is certified and re-fetchable, so a power
    /// loss only replays them; the first durable commit after it anchors the file.
    pub relaxed: bool,
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
        let genesis = Block::genesis_with(cfg.chain_id, state.root(), cfg.history_v2);
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
            history: Arc::new(aether_state::mmr::Mmr::default().append(
                &ChainHasher::new(),
                0,
                &digest_bytes(&genesis.digest()),
            )),
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
            sizes: HashMap::new(),
            mempool_bytes: 0,
            inclusion: InclusionPool::default(),
            censor: None,
            committee: Default::default(),
            identity: None,
            epoch_start: 0,
            epoch_end: Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
            proposal: None,
            history_index: Some(Arc::new({
                let mut idx = aether_state::mmr::EraIndex::default();
                idx.push(
                    &ChainHasher::new(),
                    aether_state::mmr::leaf(
                        &ChainHasher::new(),
                        0,
                        &digest_bytes(&genesis.digest()),
                    ),
                );
                idx
            })),
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
            beacon_pool: BTreeMap::new(),
            beacon_out: None,
            pruned_below: 0,
            era_cache: None,
            net_height: None,
            relaxed: false,
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
                let digest = Digest::decode(cp.digest.as_slice())
                    .map_err(|_| StoreError::Corrupt("digest"))?;
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
                // A pruned store keeps the roots of the eras it dropped instead (roadmap B4).
                g.history_index = rebuild_history_index(&cp.blocks, &cp.era_roots, cp.pruned_below, exec.height, &exec.history).map(Arc::new);
                g.pruned_below = cp.pruned_below;
                g.finalized = exec;
                g.blocks = cp.blocks;
                g.receipts = cp.receipts;
                // Eras completed before a restart but not sealed yet.
                if g.cfg.history_v2 {
                    let (store, head) = (store.clone(), g.finalized.height);
                    std::thread::spawn(move || crate::era::seal_pending(&store, head));
                }
                g.store = Some(store);
            }
            None => {
                let mut g = chain.lock();
                let genesis_exec = g.finalized.clone();
                let summary = g.blocks.get(&0).cloned().expect("genesis summary");
                let (genesis_bytes, empty) = (
                    commonware_codec::Encode::encode(&genesis),
                    aether_state::mmr::Mmr::default(),
                );
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
                    staged: g.cfg.history_v2.then(|| crate::store::Staged {
                        block: &genesis_bytes,
                        era_start: Some(&empty),
                    }),
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

    /// How many blocks behind the network this node last knew itself to be
    /// (0 when caught up — and also when nothing ever told it a height, which
    /// is not the same thing: see `behind_known`).
    pub fn behind(&self) -> u64 {
        let g = self.lock();
        g.net_height.map_or(0, |n| n.saturating_sub(g.finalized.height))
    }

    /// `behind()`, but None while no network height is known: a node nothing
    /// has answered is not "0 behind" — it is any number of blocks stale, and
    /// must not act as though it were current. `catch_up` returns only once
    /// this is Some, and keeps the last height it heard so the margin stays
    /// meaningful after it (2026-09-29).
    pub fn behind_known(&self) -> Option<u64> {
        let g = self.lock();
        g.net_height.map(|n| n.saturating_sub(g.finalized.height))
    }

    /// Replay mode for `follow`: while a certified backlog is fetched and
    /// re-executed, commits skip the per-block fsync — every block is
    /// re-fetchable, so a power loss only replays them. Cleared at the tip:
    /// the next durable commit then anchors everything the replay built.
    pub fn set_relaxed(&self, relaxed: bool) {
        self.lock().relaxed = relaxed;
    }

    /// Adopt a committee-certified snapshot as the finalized head, executing
    /// nothing (`follow::jump` checked it against the certified block after
    /// it, and `snapshot::install_over` already wrote the store). Blocks
    /// between the old head and this one are skipped: as on a checkpoint
    /// start there is no early history index (the gap's blocks stay fetchable
    /// from era files, verified by the history root), while everything kept —
    /// old summaries, receipts, proofs — is certified history of this same
    /// chain. Nothing else moves: the mempool self-prunes against the new
    /// state on the next finalize, and pooled beacons older than the new head
    /// cannot make it into a block.
    pub fn adopt(&self, exec: Arc<Executed>, summary: BlockSummary) {
        let mut g = self.lock();
        let height = exec.height;
        g.executed.clear();
        g.executed.insert(exec.digest, exec.clone());
        g.blocks.insert(height, summary);
        g.finalized = exec;
        g.history_index = None;
        // Old finalized blocks are no longer provable here (their states are
        // gone); do not let them hold the prover's queue.
        g.recent.clear();
    }

    /// Key rounds only go up: a handoff built on `parent` must carry a round
    /// above the last handoff's in its ancestry (so none is accepted twice).
    fn round_floor(parent: &Executed) -> u64 {
        parent
            .handoff
            .as_ref()
            .map(|p| p.handoff.round)
            .unwrap_or(0)
    }

    /// Whether this node's voting set has handed over before the block after
    /// `parent`: it then builds and votes for nothing past the switch.
    pub fn retired_after(&self, parent: &Executed) -> bool {
        let start = self.lock().epoch_start;
        parent
            .handoff
            .as_ref()
            .is_some_and(|p| p.switch > start && parent.height + 1 >= p.switch)
    }

    /// The handoff state after `block`: the parent's, or the one it carries
    /// (valid only when signed by the committee and no other is still pending).
    fn next_handoff(
        &self,
        height: u64,
        parent: &Executed,
        carried: Option<&aether_light::block::Handoff>,
    ) -> Result<Option<Arc<crate::handoff::Pending>>, ChainError> {
        let Some(h) = carried else {
            return Ok(parent.handoff.clone());
        };
        if parent.handoff.as_ref().is_some_and(|p| height < p.switch) {
            return Err(ChainError::BadHandoff(
                "another handoff is still pending".into(),
            ));
        }
        // Rounds only go up: a signed handoff cannot be replayed later.
        if h.round <= Self::round_floor(parent) {
            return Err(ChainError::BadHandoff(format!(
                "handoff round {} is not above {}",
                h.round,
                Self::round_floor(parent)
            )));
        }
        let (identity, chain_id) = {
            let g = self.lock();
            (g.identity, g.cfg.chain_id)
        };
        let identity = identity.ok_or_else(|| {
            ChainError::BadHandoff("no committee identity (devnet dealer keys)".into())
        })?;
        crate::handoff::verify(chain_id, &identity, h).map_err(ChainError::BadHandoff)?;
        Ok(Some(Arc::new(crate::handoff::Pending {
            at: height,
            switch: height + crate::handoff::DELAY,
            handoff: h.clone(),
        })))
    }

    /// After a restart: a finalized handoff still ends this node's epoch at its
    /// switch, and the voting set proposed for this registry epoch still stands.
    pub fn resume(&self) {
        let mut g = self.lock();
        if let Some(p) = g.finalized.handoff.clone() {
            if p.switch > g.epoch_start {
                g.epoch_end
                    .store(p.switch - 1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let current = current_draw(&g.finalized.state, g.finalized.height);
        let load = |key: &str| g.store.as_ref().and_then(|s| s.meta(key).ok().flatten());
        let proposal: Option<(u64, Vec<(String, String)>)> = load(PROPOSAL)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .flatten();
        let pool: Option<(u64, Vec<(String, String)>)> = load(POOL)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .flatten();
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
        let Some(s) = carried else {
            return Ok(parent.seed.clone());
        };
        let draw = current_draw(&parent.state, height);
        if s.draw != draw || draw == 0 || parent.seed.as_ref().is_some_and(|p| p.1.draw >= draw) {
            return Err(ChainError::BadHandoff(format!(
                "seed for draw {} in draw {draw}",
                s.draw
            )));
        }
        let (identity, chain_id) = {
            let g = self.lock();
            (g.identity, g.cfg.chain_id)
        };
        let identity = identity.ok_or_else(|| {
            ChainError::BadHandoff("no committee identity (devnet dealer keys)".into())
        })?;
        crate::handoff::verify_seed(chain_id, &identity, s).map_err(ChainError::BadHandoff)?;
        Ok(Some(Arc::new((height, s.clone()))))
    }

    /// The draw seed to put in a block built on `parent`, if one is ready and not yet there.
    pub fn seed_for(&self, parent: &Executed) -> Option<aether_light::block::Seed> {
        let ready = self.lock().seed_ready.clone()?;
        let draw = current_draw(&parent.state, parent.height + 1);
        (ready.draw == draw && parent.seed.as_ref().is_none_or(|p| p.1.draw < draw))
            .then_some(ready)
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
        let fees = cfg.fees.then(|| FeePolicy {
            base: fees::base_fee(parent.excess, cfg.limits),
            proposer,
        });
        BlockContext {
            chain_id: cfg.chain_id,
            number: block.height().get(),
            timestamp: block.timestamp / 1000,
            beneficiary: if fees.is_some() {
                aether_execution::FEE_COLLECTOR
            } else {
                proposer
            },
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
    fn execute_certified(
        &self,
        block: &Block,
        parent: &Executed,
    ) -> Result<Arc<Executed>, ChainError> {
        self.execute_as(block, parent, true)
    }

    fn execute_as(
        &self,
        block: &Block,
        parent: &Executed,
        certified: bool,
    ) -> Result<Arc<Executed>, ChainError> {
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
        let (pre, payouts) = self.pre_state_with(
            parent,
            payload.version,
            &payload.proofs,
            &payload.beacons,
            certified,
        )?;
        let schedule =
            self.next_schedule(block.height().get(), parent, payload.upgrade.as_ref())?;
        let cfg = self.cfg();
        let ctx = Self::block_context(&cfg, block, parent);
        let mut out = execute_block(&pre, &ctx, &payload.txs)
            .map_err(|e| ChainError::Exec(format!("{e:?}")))?;
        // Protocol-1 blocks keep no statement (their metadata stays protocol-1).
        let statement = if records_statement(&cfg, &payload) {
            statement(&ctx, &payload.txs, &pre, &out)
        } else {
            [0; 32]
        };
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
        Ok(self.remember(
            block, parent, &ctx, out, tx_hashes, handoff, seed, schedule, statement, payouts,
        ))
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
        self.pre_state_with(parent, version, proofs, &[], certified)
    }

    /// `pre_state` of a block that also carries beacon answers.
    #[allow(clippy::type_complexity)]
    pub fn pre_state_with<'a>(
        &self,
        parent: &'a Executed,
        version: u32,
        proofs: &[aether_light::block::ProofClaim],
        answers: &[aether_light::block::BeaconAnswer],
        certified: bool,
    ) -> Result<(std::borrow::Cow<'a, WorldState>, Vec<(u64, Address, U256)>), ChainError> {
        let (running, migrate, verifier, history_v2, chain_id) = {
            let g = self.lock();
            (g.protocol, g.migrate, g.verifier.clone(), g.cfg.history_v2, g.cfg.chain_id)
        };
        let want = parent.next_protocol();
        if version != want {
            return Err(ChainError::Protocol(format!(
                "block under protocol {version}, scheduled {want}"
            )));
        }
        if version > running {
            return Err(ChainError::Protocol(format!(
                "UPGRADE REQUIRED: the chain runs protocol {version}, this node {running}"
            )));
        }
        let before = crate::upgrade::protocol_at(&parent.schedule, parent.height);
        if version < 2 && !proofs.is_empty() {
            return Err(ChainError::Protocol("proofs before protocol 2".into()));
        }
        let records = version >= 2 && parent.statement != Statement::default();
        let rotates = parent
            .schedule
            .iter()
            .any(|a| a.at == parent.height + 1 && a.registrar.is_some());
        // The record of the block that just left the claim window goes (so state stays bounded).
        let expired = (parent.height + 1).checked_sub(aether_execution::proofs::EXPIRY + 1);
        // Under history v2 empty blocks record nothing, so pruning cannot wait for a record.
        let stale = history_v2
            && expired.is_some_and(|old| aether_execution::proofs::recorded(&parent.state, old));
        let distributes = aether_rewards::distributes(&parent.state, parent.height + 1);
        let slots = aether_rewards::beacons::touches(&parent.state, parent.height + 1);
        // A committee takes over here: the seating of the founder's reserve keys
        // becomes state (read below, after `distribute`).
        let switches = parent.handoff.as_ref().is_some_and(|p| p.switch == parent.height + 1)
            && aether_rewards::reserve(&parent.state).is_some();
        if !answers.is_empty() && !aether_rewards::enabled(&parent.state) {
            return Err(ChainError::Protocol("beacon answers without node rewards".into()));
        }
        if version == before
            && !records
            && !rotates
            && !stale
            && !distributes
            && !slots
            && !switches
            && proofs.is_empty()
            && answers.is_empty()
        {
            return Ok((std::borrow::Cow::Borrowed(&parent.state), vec![]));
        }
        let mut state = parent.state.clone();
        // Its journal then holds only these system writes (see `with_activation`).
        state.clear_journal();
        for p in before + 1..=version {
            migrate(p, &mut state)
                .map_err(|e| ChainError::Protocol(format!("activating protocol {p}: {e}")))?;
        }
        // A committee-signed upgrade may replace the registrar key when it activates.
        for (x, y) in parent
            .schedule
            .iter()
            .filter(|a| a.at == parent.height + 1)
            .filter_map(|a| a.registrar)
        {
            aether_execution::registry::set_registrar(&mut state, (x.0, y.0));
        }
        if records {
            aether_execution::proofs::record(
                &mut state,
                parent.height,
                parent.statement.commitment,
                parent.statement.escrow,
            );
        }
        if records || stale {
            if let Some(old) = expired {
                aether_execution::proofs::prune(&mut state, old);
            }
        }
        struct Certified;
        impl ProofVerifier for Certified {
            fn verify(&self, _: &[u8], _: [u8; 32]) -> bool {
                true
            }
        }
        let verifier: Option<&dyn ProofVerifier> = if certified {
            Some(&Certified)
        } else {
            verifier.as_deref()
        };
        let mut payouts = Vec::new();
        if distributes {
            // The last epoch's node rewards, credited to operators directly (no claim).
            let d = aether_rewards::distribute(&mut state, parent.height + 1)
                .map_err(|e| ChainError::Exec(format!("node rewards: {e}")))?;
            payouts.extend(
                d.paid
                    .into_iter()
                    .filter(|(_, a)| !a.is_zero())
                    .map(|(op, a)| (parent.height + 1, op, a)),
            );
        }
        // The committee taking over here is the chain's record of the founder's
        // reserve keys being seated or unseated (the seating itself lives in the
        // node). After `distribute`, which pays the epoch by the committee that
        // ran it; the next epoch's `distribute` then reads this word.
        if switches {
            let pending = parent.handoff.as_ref().expect("a handoff switches here");
            aether_rewards::switch_reserve(&mut state, parent.height + 1, &pending.handoff.members);
        }
        // This epoch's beacon slots and the hash of a slot's block, then the answers.
        aether_rewards::beacons::on_block(&mut state, parent.height + 1, digest_bytes(&parent.digest));
        crate::beacons::apply(&mut state, chain_id, parent.height + 1, answers)
            .map_err(|e| ChainError::Protocol(format!("beacons: {e}")))?;
        payouts.extend(pay_proofs(&mut state, parent.height + 1, proofs, verifier)?);
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
            if g.attempted.contains(&h)
                || aether_execution::proofs::prover(&head.state, h).is_some()
                || !records_statement(&g.cfg, &b.payload()?)
            {
                return None;
            }
            Some((
                g.executed.get(&b.digest())?.clone(),
                g.executed.get(&b.parent)?.clone(),
                b.clone(),
            ))
        })?;
        g.attempted.insert(pick.0.height);
        Some(pick)
    }

    /// Inclusion proof of block `height` under the history root of block
    /// `anchor` (which commits blocks 0..anchor), and the block's hash. Reads
    /// the era roots plus at most two eras' block hashes under the lock; the
    /// hashing happens outside it.
    pub fn history_proof(
        &self,
        height: u64,
        anchor: u64,
    ) -> Result<(aether_state::mmr::MmrProof, String), String> {
        use aether_state::mmr::ERA_LEN;
        {
            let g = self.lock();
            if anchor == 0 || height >= anchor || anchor > g.finalized.height {
                return Err("need height < anchor <= finalized height".into());
            }
        }
        let (index, eras) = self.era_leaves(&[height / ERA_LEN, anchor / ERA_LEN])?;
        let h = ChainHasher::new();
        let open = index.eras.len() as u64;
        let e = height / ERA_LEN;
        let hash = if e < open {
            eras.get(&e).map(|(_, hashes)| hashes[(height % ERA_LEN) as usize])
        } else {
            self.lock()
                .blocks
                .get(&height)
                .and_then(|b| hex::decode(&b.hash).ok())
                .and_then(|d| d.try_into().ok())
        }
        .ok_or("block not kept here")?;
        let proof = index
            .prove(&h, anchor, height, |e| eras.get(&e).map(|(l, _)| l.clone()))
            .ok_or("no proof")?;
        Ok((proof, hex::encode(hash)))
    }

    /// Leaf digests and block hashes of the complete eras among `wanted`: from
    /// the kept block summaries, or from the era file once they are pruned (read
    /// back and checked against the era root this node keeps).
    #[allow(clippy::type_complexity)]
    fn era_leaves(
        &self,
        wanted: &[u64],
    ) -> Result<
        (
            Arc<aether_state::mmr::EraIndex>,
            BTreeMap<u64, (Vec<[u8; 32]>, Vec<[u8; 32]>)>,
        ),
        String,
    > {
        use aether_state::mmr::ERA_LEN;
        let h = ChainHasher::new();
        let (index, kept, from_file, store) = {
            let g = self.lock();
            let index = g
                .history_index
                .clone()
                .ok_or("this node started from a checkpoint and keeps no early history")?;
            let open = index.eras.len() as u64;
            let (mut kept, mut from_file) = (BTreeMap::new(), Vec::new());
            for &e in wanted.iter().filter(|e| **e < open) {
                if e * ERA_LEN < g.pruned_below {
                    from_file.push(e);
                    continue;
                }
                let hashes: Option<Vec<[u8; 32]>> = g
                    .blocks
                    .range(e * ERA_LEN..(e + 1) * ERA_LEN)
                    .map(|(_, b)| hex::decode(&b.hash).ok()?.try_into().ok())
                    .collect();
                kept.insert(e, hashes.ok_or("bad block hash")?);
            }
            (index, kept, from_file, g.store.clone())
        };
        let mut out = BTreeMap::new();
        for (e, hashes) in kept {
            let leaves = hashes
                .iter()
                .enumerate()
                .map(|(i, d)| aether_state::mmr::leaf(&h, e * ERA_LEN + i as u64, d))
                .collect();
            out.insert(e, (leaves, hashes));
        }
        for e in from_file {
            let era = self.load_era(store.as_deref(), e, Some(&index.eras[e as usize]))?;
            let hashes: Vec<[u8; 32]> = era.blocks.iter().map(|b| digest_bytes(&b.digest())).collect();
            let leaves = hashes
                .iter()
                .enumerate()
                .map(|(i, d)| aether_state::mmr::leaf(&h, e * ERA_LEN + i as u64, d))
                .collect();
            out.insert(e, (leaves, hashes));
        }
        Ok((index, out))
    }

    /// Era `era` from this node's era file, read back and checked (against
    /// `root` when given); the last one read is cached.
    pub fn load_era(
        &self,
        store: Option<&Store>,
        era: u64,
        root: Option<&[u8; 32]>,
    ) -> Result<Arc<crate::era::Era>, String> {
        if let Some((e, cached)) = self.lock().era_cache.clone() {
            if e == era && root.is_none_or(|r| *r == cached.root) {
                return Ok(cached);
            }
        }
        let store = store.ok_or("no store")?;
        let path = store.era_dir().join(crate::era::file_name(era));
        let bytes = std::fs::read(&path)
            .map_err(|_| format!("pruned: era {era} is pruned here and its file is not kept"))?;
        let decoded = crate::era::read(&bytes, root).map_err(|e| format!("era {era} file: {e}"))?;
        if decoded.index != era {
            return Err(format!("era file {} holds era {}", path.display(), decoded.index));
        }
        let decoded = Arc::new(decoded);
        self.lock().era_cache = Some((era, decoded.clone()));
        Ok(decoded)
    }

    /// Proof that era `era`'s root is in the history under block `anchor`'s
    /// history root (`aether_light::verify_era_root`): what a peer checks an
    /// era file it fetched from this node against.
    pub fn era_proof(&self, era: u64, anchor: u64) -> Result<aether_state::mmr::MmrProof, String> {
        use aether_state::mmr::{ERA_BITS, ERA_LEN};
        {
            // `anchor` counts blocks: up to the whole finalized chain (the history
            // root the next block will commit, what a peer at the same head holds).
            let g = self.lock();
            if anchor > g.finalized.height + 1 || (era + 1).saturating_mul(ERA_LEN) > anchor {
                return Err("need a complete era below anchor <= finalized height + 1".into());
            }
        }
        let (index, eras) = self.era_leaves(&[anchor / ERA_LEN])?;
        let open = index.eras.len() as u64;
        aether_state::mmr::prove_by_eras(
            &ChainHasher::new(),
            anchor,
            era * ERA_LEN,
            ERA_BITS,
            &index.eras,
            |e| {
                if e == open {
                    Some(index.open.clone())
                } else {
                    eras.get(&e).map(|(l, _)| l.clone())
                }
            },
        )
        .ok_or_else(|| "no proof".to_string())
    }

    /// Pruning (roadmap B4): drop the block summaries, receipts and finality
    /// proofs below `cutoff` (an era boundary at or below the head), on disk
    /// first and then in memory. The roots of the eras below it stay, so
    /// history proofs keep working (from the era files while they are kept).
    pub fn prune(&self, cutoff: u64) -> Result<crate::store::PruneReport, String> {
        use aether_state::mmr::ERA_LEN;
        let (store, roots) = {
            let g = self.lock();
            if cutoff <= g.pruned_below {
                return Ok(Default::default());
            }
            if !cutoff.is_multiple_of(ERA_LEN) || cutoff > g.finalized.height {
                return Err(format!("cannot prune below {cutoff}: not an era boundary at or below the head"));
            }
            let roots: Vec<(u64, [u8; 32])> = match &g.history_index {
                Some(idx) => (g.pruned_below / ERA_LEN..cutoff / ERA_LEN)
                    .filter_map(|e| idx.eras.get(e as usize).map(|r| (e, *r)))
                    .collect(),
                None => Vec::new(),
            };
            (g.store.clone(), roots)
        };
        let report = match &store {
            Some(s) => s.prune_below(cutoff, &roots).map_err(|e| e.to_string())?,
            None => Default::default(),
        };
        let mut g = self.lock();
        g.blocks = g.blocks.split_off(&cutoff);
        g.receipts.retain(|_, (h, _)| *h >= cutoff);
        g.pruned_below = cutoff;
        Ok(report)
    }

    /// Finalized block `height` read back from its era file (pruned heights).
    pub fn old_block(&self, height: u64) -> Result<Block, String> {
        use aether_state::mmr::ERA_LEN;
        let (store, root) = {
            let g = self.lock();
            let e = height / ERA_LEN;
            (g.store.clone(), g.history_index.as_ref().and_then(|i| i.eras.get(e as usize).copied()))
        };
        let era = self.load_era(store.as_deref(), height / ERA_LEN, root.as_ref())?;
        Ok(era.blocks[(height % ERA_LEN) as usize].clone())
    }

    /// Rewards `prover` received for proofs (this node's record since it started keeping one).
    pub fn rewards(&self, prover: &Address) -> Vec<Value> {
        self.recent_rewards(prover, usize::MAX)
    }

    /// The newest `limit` rewards of `prover`, oldest first.
    pub fn recent_rewards(&self, prover: &Address, limit: usize) -> Vec<Value> {
        // Read the store without holding the chain lock (a long history must not stall consensus).
        let store = self.lock().store.clone();
        let rows = store
            .and_then(|s| s.rewards(&prover.0 .0, limit).ok())
            .unwrap_or_default();
        rows.iter()
            .filter_map(|r| serde_json::from_slice(r).ok())
            .collect()
    }

    /// Keep a proof (verified by this node's verifier) for this node's next proposals.
    pub fn add_proof(&self, claim: aether_light::block::ProofClaim) -> Result<(), String> {
        self.add_proof_from(claim, false)
    }

    /// A proof this node's own prover made: no rate limit (nobody else can use this path).
    pub fn add_own_proof(&self, claim: aether_light::block::ProofClaim) -> Result<(), String> {
        self.add_proof_from(claim, true)
    }

    fn add_proof_from(
        &self,
        claim: aether_light::block::ProofClaim,
        own: bool,
    ) -> Result<(), String> {
        let open = |g: &Inner, h: u64| {
            let f = &g.finalized;
            aether_execution::proofs::claimable(&f.state, h, f.height + 1)
                .or_else(|e| {
                    if h == f.height && aether_execution::proofs::prover(&f.state, h).is_none() {
                        Ok(f.statement.commitment)
                    } else {
                        Err(e)
                    }
                })
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
            if !own
                && g.last_proof_check
                    .is_some_and(|t| now.duration_since(t) < PROOF_CHECK_INTERVAL)
            {
                return Err("busy; try again shortly".into());
            }
            let c = open(&g, claim.height)?;
            if !own {
                g.last_proof_check = Some(now);
            }
            (
                g.verifier
                    .clone()
                    .ok_or("this node does not verify proofs")?,
                c,
            )
        };
        let bytes = hex::decode(&claim.proof).map_err(|_| "proof is not hex")?;
        let output = aether_proving::block::claim(commitment, claim.prover);
        let key = *blake3::Hasher::new()
            .update(&output)
            .update(&bytes)
            .finalize()
            .as_bytes();
        if self.lock().rejected.contains(&key) {
            return Err("this proof was already refused".into());
        }
        match (bytes.len() <= MAX_PROOF_BYTES)
            .then(|| verifier.decide(&bytes, output))
            .flatten()
        {
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
                && (h == parent.height
                    || aether_execution::proofs::claimable(&parent.state, h, next).is_ok())
        };
        let mut seen = std::collections::BTreeSet::new();
        g.proof_pool
            .iter()
            .filter(|c| open(c.height) && seen.insert(c.height))
            .take(MAX_PROOFS_PER_BLOCK)
            .cloned()
            .collect()
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
            && (height == f.height
                || aether_execution::proofs::claimable(&f.state, height, f.height + 1).is_ok())
    }

    /// Drop these proofs from the pool (they no longer verify here).
    pub fn drop_proofs(&self, heights: &[u64]) {
        self.lock()
            .proof_pool
            .retain(|c| !heights.contains(&c.height));
    }

    /// Take a beacon answer into the pool if it is valid for the next block on
    /// the finalized head. Ok(true) when it is new here.
    pub fn add_beacon(&self, a: aether_light::block::BeaconAnswer) -> Result<bool, String> {
        let (state, height, chain_id) = {
            let g = self.lock();
            let f = &g.finalized;
            // The next block's slot writes first: the hash of a slot's block lands there.
            let view = crate::beacons::next_view(&f.state, f.height + 1, digest_bytes(&f.digest));
            (view, f.height + 1, g.cfg.chain_id)
        };
        let checked = crate::beacons::verify(&state, chain_id, height, &a)?;
        let key = (checked.due.epoch, checked.due.slot, a.index);
        let mut g = self.lock();
        if g.beacon_pool.contains_key(&key) {
            return Ok(false);
        }
        if g.beacon_pool.len() >= 16 * aether_rewards::beacons::MAX_ANSWERS_PER_BLOCK {
            return Err("beacon pool full".into());
        }
        g.beacon_pool.insert(key, a);
        Ok(true)
    }

    /// `add_beacon`, then send a new answer on to the other validators.
    pub fn submit_beacon(&self, a: aether_light::block::BeaconAnswer) -> Result<bool, String> {
        let new = self.add_beacon(a.clone())?;
        if new {
            if let Some(out) = self.lock().beacon_out.as_ref() {
                let _ = out.send(a);
            }
        }
        Ok(new)
    }

    /// Pooled answers valid in the block after `parent`, one per (Mac, slot).
    pub fn beacons_for(&self, parent: &Executed) -> Vec<aether_light::block::BeaconAnswer> {
        if !aether_rewards::enabled(&parent.state) {
            return vec![];
        }
        let (pool, chain_id): (Vec<_>, u64) = {
            let g = self.lock();
            (g.beacon_pool.values().cloned().collect(), g.cfg.chain_id)
        };
        // Check them on the parent with this block's slot writes applied (a slot's hash lands here).
        let height = parent.height + 1;
        let mut state = crate::beacons::next_view(&parent.state, height, digest_bytes(&parent.digest));
        let mut out = Vec::new();
        for a in pool {
            if out.len() == aether_rewards::beacons::MAX_ANSWERS_PER_BLOCK {
                break;
            }
            if let Ok(c) = crate::beacons::verify(&state, chain_id, height, &a) {
                aether_rewards::beacons::record(&mut state, &c.candidate, &c.due, c.attested);
                out.push(a);
            }
        }
        out
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
    fn admissible_upgrade(
        parent: &Executed,
        cfg: &ChainConfig,
        u: &crate::upgrade::Upgrade,
    ) -> Result<(), String> {
        // Protocol-1 nodes cannot read a registrar change: it may only be announced under protocol 2.
        if u.registrar.is_some() && parent.next_protocol() < 2 {
            return Err("a registrar change needs protocol 2".into());
        }
        let chain_id = cfg.chain_id;
        let notice = Self::notice(&parent.state, cfg.epoch_blocks);
        use crate::upgrade::{MAX_FIELD, MAX_RELEASES};
        let height = parent.height + 1;
        let (last_protocol, last_at) = parent
            .schedule
            .last()
            .map(|a| (a.protocol, a.at))
            .unwrap_or((1, 0));
        if u.chain_id != chain_id {
            return Err("upgrade for another chain".into());
        }
        if u.protocol <= last_protocol || u.activate_at <= last_at {
            return Err(format!(
                "protocol {} at {} is not after {last_protocol} at {last_at}",
                u.protocol, u.activate_at
            ));
        }
        if u.activate_at < height.saturating_add(notice) {
            return Err(format!(
                "activation at {} gives less than {notice} blocks of notice",
                u.activate_at
            ));
        }
        let long = |s: &String| s.len() > MAX_FIELD;
        if u.releases.len() > MAX_RELEASES
            || long(&u.notes)
            || u.releases
                .iter()
                .any(|r| long(&r.platform) || long(&r.version) || long(&r.blake3) || long(&r.url))
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
        let Some(s) = carried else {
            return Ok(parent.schedule.clone());
        };
        debug_assert_eq!(height, parent.height + 1);
        let (identity, cfg) = {
            let g = self.lock();
            (g.identity, g.cfg.clone())
        };
        Self::admissible_upgrade(parent, &cfg, &s.upgrade).map_err(ChainError::Protocol)?;
        let identity = identity.ok_or_else(|| {
            ChainError::Protocol("no committee identity (devnet dealer keys)".into())
        })?;
        crate::upgrade::verify(&identity, s).map_err(ChainError::Protocol)?;
        Ok(scheduled(&parent.schedule, &s.upgrade))
    }

    /// The upgrade to put in a block built on `parent`: the first known one not
    /// yet on chain that may go there now.
    pub fn upgrade_for(&self, parent: &Executed) -> Option<crate::upgrade::SignedUpgrade> {
        let g = self.lock();
        g.upgrades_known
            .iter()
            .find(|s| Self::admissible_upgrade(parent, &g.cfg, &s.upgrade).is_ok())
            .cloned()
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
            Some(f) => (
                f.base,
                fees::next_excess(parent.excess, out.gas, ctx.limits),
            ),
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
            history: Arc::new(parent.history.append(
                &ChainHasher::new(),
                block.height().get(),
                &digest_bytes(&block.digest()),
            )),
            schedule,
            statement: if statement == [0; 32] {
                Statement::default()
            } else {
                Statement {
                    commitment: statement,
                    escrow,
                }
            },
            payouts,
        });
        self.lock().executed.insert(block.digest(), exec.clone());
        exec
    }

    /// Candidate txs for a proposal: inclusion-list txs (each with the sender's
    /// earlier-nonce chain that must land before it), then the rest of the
    /// mempool (lowest nonce first per sender).
    pub fn mempool_candidates(&self) -> Vec<TxEnvelope> {
        let g = self.lock();
        let censored = |t: &TxEnvelope| {
            g.censor == Some(t.header.sender) || g.deprioritize == Some(t.header.sender)
        };
        let listed = if g.censor.is_some() {
            Vec::new()
        } else {
            g.inclusion.for_proposal()
        };
        let listed_hashes: std::collections::HashSet<TxHash> =
            listed.iter().map(aether_execution::tx_hash).collect();
        let rest: Vec<TxEnvelope> = g
            .mempool
            .iter()
            .filter(|(h, t)| !listed_hashes.contains(*h) && !censored(t))
            .map(|(_, t)| t.clone())
            .collect();
        // Listed txs ride with their senders' earlier nonces ahead of everything
        // else. One nonce order over the whole pool (the 2026-09-29 stall fix)
        // let a listed tx at a later nonce be pushed past the block's tx budget
        // by a flood of unrelated nonce-0 txs — block after block, without
        // anyone censoring anything (2026-09-29 red-team 5). The chain that
        // must land first goes first, so a listed tx needs only its own
        // sender's earlier nonces, never room past two thousand strangers.
        let mut wanted: HashMap<Address, u64> = HashMap::with_capacity(listed.len());
        for t in &listed {
            wanted
                .entry(t.header.sender)
                .and_modify(|top| *top = (*top).max(t.header.nonce))
                .or_insert(t.header.nonce);
        }
        let mut head: Vec<(bool, TxEnvelope)> = Vec::new();
        let mut tail: Vec<(bool, TxEnvelope)> = Vec::new();
        for t in rest {
            match wanted.get(&t.header.sender) {
                Some(top) if t.header.nonce <= *top => head.push((false, t)),
                _ => tail.push((false, t)),
            }
        }
        head.extend(listed.into_iter().map(|t| (true, t)));
        // Within the head (and between senders in it) nonce order still rules:
        // a listed tx whose sender's earlier nonces sit in the mempool would
        // fail up front, never be retried, and the block would then wrongly
        // leave it out — the original stall. For equal nonces a listed tx
        // still comes first.
        head.sort_by_key(|(listed, t)| (t.header.nonce, !*listed, t.header.sender));
        tail.sort_by_key(|(_, t)| (t.header.nonce, t.header.sender));
        head.extend(tail);
        let mut txs: Vec<TxEnvelope> = head.into_iter().map(|(_, t)| t).collect();
        txs.truncate(MAX_TXS_PER_BLOCK);
        txs
    }

    /// The oldest mempool txs waiting at least `min_age`, for this node's inclusion list.
    pub fn inclusion_candidates(&self, min_age: Duration, now: Instant) -> Vec<TxEnvelope> {
        let g = self.lock();
        let mut waiting: Vec<(Instant, TxHash)> = g
            .arrivals
            .iter()
            .filter(|(h, t)| {
                now.saturating_duration_since(**t) >= min_age && g.mempool.contains_key(*h)
            })
            .map(|(h, t)| (*t, *h))
            .collect();
        waiting.sort();
        waiting
            .into_iter()
            .take(inclusion::MAX_IL_TXS)
            .filter_map(|(_, h)| g.mempool.get(&h).cloned())
            .collect()
    }

    /// Listed txs that `exec` (a verified block) wrongly left out. Empty for a
    /// censoring devnet node, which ignores the lists.
    pub fn inclusion_violations(
        &self,
        exec: &Executed,
        ctx: &BlockContext,
        now: Instant,
    ) -> Vec<TxHash> {
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
        let size = tx.to_canonical_bytes().len();
        let mut g = self.lock();
        if g.receipts.contains_key(&h) || g.mempool.contains_key(&h) {
            return Ok(false);
        }
        if size > MAX_TX_BYTES {
            return Err(format!("transaction is {size} bytes, over the {MAX_TX_BYTES}-byte cap"));
        }
        if g.mempool.len() >= MAX_MEMPOOL {
            return Err("mempool full".into());
        }
        if g.mempool_bytes + size > MAX_MEMPOOL_BYTES {
            return Err("mempool byte budget full".into());
        }
        let state = &g.finalized.state;
        if tx.header.nonce < state.nonce(&tx.header.sender) {
            return Err("nonce already used".into());
        }
        if g.cfg.fees {
            admissible(&tx, state, Self::next_base_fee(&g.cfg, &g.finalized))?;
        }
        if g.pending_by_sender
            .get(&tx.header.sender)
            .copied()
            .unwrap_or(0)
            >= MAX_PER_SENDER
        {
            return Err(format!("sender has {MAX_PER_SENDER} pending transactions"));
        }
        *g.pending_by_sender.entry(tx.header.sender).or_default() += 1;
        g.mempool.insert(h, tx);
        g.sizes.insert(h, size);
        g.mempool_bytes += size;
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
        let (store, history_v2, previous_history, relaxed) = {
            let g = self.lock();
            (g.store.clone(), g.cfg.history_v2, g.finalized.history.clone(), g.relaxed)
        };
        // History v2: keep the block for its era file; an era's first block also keeps the history before it.
        let staged = history_v2.then(|| commonware_codec::Encode::encode(block));
        let era_start = (history_v2 && exec.height.is_multiple_of(aether_state::mmr::ERA_LEN))
            .then(|| {
                if exec.height == 0 {
                    Default::default()
                } else {
                    (*previous_history).clone()
                }
            });
        if let Some(store) = store {
            // Disk first: the in-memory head never runs ahead of what survives a crash.
            let write = Commit {
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
                staged: staged.as_ref().map(|b| crate::store::Staged {
                    block: b,
                    era_start: era_start.as_ref(),
                }),
            };
            (if relaxed { store.commit_relaxed(write) } else { store.commit(write) })
                .map_err(|e| ChainError::Store(e.to_string()))?;
            // An era's last block: seal it into a file, off the consensus path.
            if history_v2 && (exec.height + 1).is_multiple_of(aether_state::mmr::ERA_LEN) {
                let era = exec.height / aether_state::mmr::ERA_LEN;
                std::thread::spawn(move || crate::era::seal_logged(&store, era));
            }
        }
        let mut g = self.lock();
        for (h, r) in exec.tx_hashes.iter().zip(&exec.receipts) {
            g.receipts.insert(*h, (exec.height, r.clone()));
            g.mempool.remove(h);
            if let Some(s) = g.sizes.remove(h) {
                g.mempool_bytes -= s;
            }
        }
        let state = exec.state.clone();
        // Drop what can no longer run: a used nonce, a sender whose balance no
        // longer covers the tx (it paid for other txs meanwhile), or a tx left
        // waiting past the TTL. Otherwise txs admitted while affordable would
        // hold per-sender slots and pool space forever: enough of them would
        // fill the pool and stop the chain from taking new txs.
        let base = Self::next_base_fee(&g.cfg, &exec);
        let fees = g.cfg.fees;
        let now = Instant::now();
        let inner = &mut *g;
        let arrivals = &inner.arrivals;
        inner
            .mempool
            .retain(|h, tx| keep_in_pool(tx, arrivals.get(h).copied(), now, &state, base, fees));
        inner.arrivals.retain(|h, _| inner.mempool.contains_key(h));
        // The byte budget follows the pool: sizes of txs that left give their
        // bytes back, so the budget never leaks by a dropped tx.
        {
            let (mempool, sizes) = (&inner.mempool, &mut inner.sizes);
            let mut freed = 0;
            sizes.retain(|h, s| {
                let kept = mempool.contains_key(h);
                if !kept {
                    freed += *s;
                }
                kept
            });
            inner.mempool_bytes -= freed;
        }
        inner.pending_by_sender.clear();
        for t in inner.mempool.values() {
            *inner.pending_by_sender.entry(t.header.sender).or_default() += 1;
        }
        inner.inclusion.prune(&state, exec.height, Instant::now());
        // One leaf per height, in order (genesis is delivered again at start-up).
        if let Some(idx) = g
            .history_index
            .as_mut()
            .filter(|i| i.leaves() == exec.height)
        {
            Arc::make_mut(idx).push(
                &ChainHasher::new(),
                aether_state::mmr::leaf(
                    &ChainHasher::new(),
                    exec.height,
                    &digest_bytes(&exec.digest),
                ),
            );
        }
        g.blocks.insert(exec.height, summary);
        let previous = std::mem::replace(&mut g.finalized, exec.clone());
        // A finalized handoff ends the running epoch before its switch height.
        if let Some(p) = exec.handoff.as_ref().filter(|p| p.at == exec.height) {
            if p.switch > g.epoch_start {
                g.epoch_end
                    .store(p.switch - 1, std::sync::atomic::Ordering::SeqCst);
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
            let pool = crate::rotation::eligible(
                &previous.state,
                exec.height / params.epoch_blocks,
                params.min_streak,
            );
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
                let operator = |k: &str| {
                    ops.get(k)
                        .map(|o| if capped { o.clone() } else { k.to_string() })
                };
                // Protocol 3: qualifying Macs join (up to 16 seats) instead of
                // replacing members — and each seat goes where it lifts the
                // committee's worst hour of the day (13-roadmap.md, F).
                let reserve = Reserve::of(&exec.state);
                let availability = (exec.next_protocol() >= 3 || reserve.is_some())
                    .then(|| crate::rotation::availability(&exec.state));
                let hours = |k: &str| availability.as_ref().and_then(|a| a.get(k).copied());
                let drawn = if exec.next_protocol() >= 3 {
                    crate::rotation::draw_spread(&pool, &seed, operator, &g.committee, hours)
                } else {
                    crate::rotation::draw(&pool, &seed, operator, &g.committee)
                };
                // Founder reserve keys join or leave with the draw too.
                let drawn = match reserve {
                    Some(r) => crate::rotation::with_reserve(
                        drawn,
                        &pool,
                        &seed,
                        |k: &str| ops.get(k).cloned(),
                        &r,
                        &g.committee,
                        hours,
                    ),
                    None => drawn,
                };
                g.proposal = drawn.map(|m| (draw, m));
                keep(&g.store, PROPOSAL, &g.proposal);
            }
        }
        let floor = exec.height.saturating_sub(64);
        g.executed.retain(|_, e| e.height >= floor);
        for (proven, prover, amount) in &exec.payouts {
            let kind = if *proven == exec.height {
                "node"
            } else {
                "proof"
            };
            let record = serde_json::json!({ "kind": kind, "proven": proven, "amount": amount, "height": exec.height, "timestamp_ms": exec.timestamp });
            if let Some(Err(e)) = g.store.as_ref().map(|s| {
                s.put_reward(
                    &prover.0 .0,
                    exec.height,
                    *proven,
                    record.to_string().as_bytes(),
                )
            }) {
                tracing::warn!(%e, "could not keep a reward record");
            }
        }
        // Our proposal with proofs lost at this height: others may not verify them.
        if g.proof_proposal
            .is_some_and(|(h, d)| h == exec.height && d != exec.digest)
        {
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
        g.proof_pool.retain(|c| {
            aether_execution::proofs::claimable(state, c.height, now + 1).is_ok() || c.height == now
        });
        g.attempted.retain(|h| *h + 64 >= now);
        // Answers recorded, or of slots whose window closed, leave the pool.
        if !g.beacon_pool.is_empty() {
            use aether_rewards::beacons;
            let epoch_blocks = aether_execution::registry::epoch_blocks(state);
            let epoch = (now + 1) / epoch_blocks;
            let window = beacons::layout(epoch_blocks).map_or(0, |l| l.1);
            g.beacon_pool.retain(|(e, slot, index), _| {
                let b = beacons::beacon(state, *index);
                let recorded = b.epoch == *e && b.mask & (1 << slot) != 0;
                let open = *e == epoch && beacons::slot(state, *slot).is_some_and(|h| h + window > now);
                open && !recorded
            });
        }
        replacement_step(&mut g, &previous, &exec);
        reserve_step(&mut g, &previous, &exec);
        Ok(())
    }
}

/// The fee-policy checks a block would reject the tx for, applied at admission:
/// caps at least the current base fees (zero when uncongested), a prove budget covering the gas limit, and a
/// balance covering value, max exec fee and prove budget.
fn admissible(tx: &TxEnvelope, state: &WorldState, base: FeeVector) -> Result<(), String> {
    if tx.header.max_fee.exec < base.exec || tx.header.max_fee.prove < base.prove {
        return Err("fee caps below the base fee".into());
    }
    affordable(tx, state, base)
}

/// Whether a pending tx stays in the pool after a block: its nonce is still
/// ahead, its sender can still pay for it, and it has not waited past the TTL.
/// A tx whose fee caps are below the base fee stays (the fee may come down)
/// until the TTL.
fn keep_in_pool(
    tx: &TxEnvelope,
    arrived: Option<Instant>,
    now: Instant,
    state: &WorldState,
    base: FeeVector,
    fees: bool,
) -> bool {
    if tx.header.nonce < state.nonce(&tx.header.sender) {
        return false;
    }
    if arrived.is_some_and(|t| now.saturating_duration_since(t) >= MEMPOOL_TTL) {
        return false;
    }
    !fees || affordable(tx, state, base).is_ok()
}

/// The payload decodes, the prove budget covers the gas limit, and the sender's
/// balance covers value, gas at its fee cap and the prove budget.
fn affordable(tx: &TxEnvelope, state: &WorldState, base: FeeVector) -> Result<(), String> {
    let aether_types::TxPayload::Plain(bytes) = &tx.payload else {
        return Err("encrypted payloads are not supported yet".into());
    };
    let call = aether_execution::EvmCall::decode(bytes).map_err(|e| format!("payload: {e:?}"))?;
    if tx.header.gas.prove < call.gas_limit {
        return Err("prove budget below the gas limit".into());
    }
    let need = U256::from(call.gas_limit)
        .checked_mul(U256::from(tx.header.max_fee.exec))
        .and_then(|g| {
            U256::from(tx.header.gas.prove)
                .checked_mul(U256::from(base.prove))
                .and_then(|p| g.checked_add(p))
        })
        .and_then(|n| n.checked_add(call.value));
    if need.is_none_or(|n| state.balance(&tx.header.sender) < n) {
        return Err("insufficient funds for value, gas and prove budget".into());
    }
    Ok(())
}

fn digest_bytes(d: &Digest) -> [u8; 32] {
    d.as_ref().try_into().expect("sha256 digest is 32 bytes")
}

/// The history index at restart: roots of the pruned eras (roadmap B4), then a
/// leaf per kept summary up to the head. None when a height is missing (a node
/// started from a checkpoint) or the result is not the history the store
/// committed (a damaged root table).
fn rebuild_history_index(
    blocks: &BTreeMap<u64, BlockSummary>,
    era_roots: &[[u8; 32]],
    pruned_below: u64,
    head: u64,
    history: &aether_state::mmr::Mmr,
) -> Option<aether_state::mmr::EraIndex> {
    use aether_state::mmr::ERA_LEN;
    let h = ChainHasher::new();
    let pruned_eras = pruned_below / ERA_LEN;
    if !pruned_below.is_multiple_of(ERA_LEN) || (era_roots.len() as u64) < pruned_eras {
        return None;
    }
    let mut idx = aether_state::mmr::EraIndex {
        eras: era_roots[..pruned_eras as usize].to_vec(),
        open: Vec::new(),
    };
    for k in pruned_below..=head {
        let d = blocks
            .get(&k)
            .and_then(|b| hex::decode(&b.hash).ok())
            .and_then(|d| <[u8; 32]>::try_from(d).ok())?;
        idx.push(&h, aether_state::mmr::leaf(&h, k, &d));
    }
    if pruned_below > 0 && idx.mmr(&h) != *history {
        tracing::warn!(
            pruned_below,
            "stored era roots do not match the committed history; history proofs are off"
        );
        return None;
    }
    Some(idx)
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

/// Founder reserve keys (genesis parameter): at every registry epoch's first
/// block, seat them for the next epoch while fewer than four independent
/// operators qualify, and let them go once four or more do. From four on, the
/// keys are a liveness safety net (13-roadmap.md, F): they seat themselves
/// while the committee's worst hour of the day risks losing its quorum.
/// Proposes nothing while a proposal or a handoff is already on its way.
fn reserve_step(g: &mut Inner, previous: &Executed, exec: &Executed) {
    let Some(reserve) = Reserve::of(&exec.state) else {
        return;
    };
    let params = aether_execution::registry::params(&previous.state);
    let handing_over = exec
        .handoff
        .as_ref()
        .is_some_and(|p| p.switch > exec.height);
    // A draw's first block leaves it to the draw (which applies the same rule with its seed).
    let draw_start = exec
        .height
        .is_multiple_of(params.epoch_blocks * params.draw_epochs);
    if exec.height == 0
        || draw_start
        || !exec.height.is_multiple_of(params.epoch_blocks)
        || g.committee.members.is_empty()
        || g.proposal.is_some()
        || handing_over
    {
        return;
    }
    let pool = crate::rotation::eligible(
        &previous.state,
        exec.height / params.epoch_blocks,
        params.min_streak,
    );
    let ops = crate::rotation::operators(&exec.state);
    let availability = crate::rotation::availability(&exec.state);
    let next = crate::rotation::with_reserve(
        None,
        &pool,
        exec.digest.as_ref(),
        |k: &str| ops.get(k).cloned(),
        &reserve,
        &g.committee,
        |k: &str| availability.get(k).copied(),
    );
    if let Some(members) = next {
        tracing::info!(
            members = members.len(),
            "founder reserve keys change the voting set"
        );
        g.proposal = Some((current_draw(&exec.state, exec.height), members));
        keep(&g.store, PROPOSAL, &g.proposal);
    }
}

/// The draw a block at `height` belongs to (draws start at multiples of epoch_blocks × draw_epochs).
fn current_draw(state: &WorldState, height: u64) -> u64 {
    let p = aether_execution::registry::params(state);
    height / (p.epoch_blocks * p.draw_epochs)
}

/// Early replacement (docs/design/13-roadmap.md, F): at every registry epoch's
/// first block, hand a silent member's seat — fewer than two of the four beacon
/// slots answered in each of the last two epochs — to the best eligible
/// candidate, at most a third minus one seats per epoch. A substitution is a
/// reshare, and a reshare needs the old committee's quorum, so this must
/// happen before the quorum is lost, not after. Like `reserve_step`, it
/// proposes nothing while a proposal or a handoff is already on its way (the
/// keys' seating then waits for the next epoch).
fn replacement_step(g: &mut Inner, previous: &Executed, exec: &Executed) {
    if !aether_rewards::enabled(&exec.state) {
        return;
    }
    let params = aether_execution::registry::params(&previous.state);
    let handing_over = exec
        .handoff
        .as_ref()
        .is_some_and(|p| p.switch > exec.height);
    // A draw's first block leaves the voting set to the draw.
    let draw_start = exec
        .height
        .is_multiple_of(params.epoch_blocks * params.draw_epochs);
    if exec.height == 0
        || draw_start
        || !exec.height.is_multiple_of(params.epoch_blocks)
        || g.committee.members.is_empty()
        || g.proposal.is_some()
        || handing_over
    {
        return;
    }
    let pool = crate::rotation::eligible(
        &previous.state,
        exec.height / params.epoch_blocks,
        params.min_streak,
    );
    let ops = crate::rotation::operators(&exec.state);
    let availability = crate::rotation::availability(&exec.state);
    let recents = crate::rotation::recents(&exec.state);
    let next = crate::rotation::replace_silent(
        &g.committee,
        &pool,
        exec.digest.as_ref(),
        |k: &str| ops.get(k).cloned(),
        |k: &str| availability.get(k).copied(),
        |k: &str| recents.get(k).copied(),
        exec.height / params.epoch_blocks,
    );
    if let Some(members) = next {
        tracing::info!(
            members = members.len(),
            "a silent member is replaced while the quorum still stands"
        );
        g.proposal = Some((current_draw(&exec.state, exec.height), members));
        keep(&g.store, PROPOSAL, &g.proposal);
    }
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
    pub beacons: Vec<aether_light::block::BeaconAnswer>,
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
    let Extras {
        handoff,
        seed,
        upgrade,
        proofs,
        beacons,
    } = extras;
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
        beacons,
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

/// Whether a block gets a proof-market statement (recorded by its child).
/// Protocol-1 blocks never do; under history v2 neither does a block with no
/// transactions and no proofs: there is nothing to prove, and an empty block
/// then leaves the state root and chain metadata as they were.
pub fn records_statement(cfg: &ChainConfig, payload: &Payload) -> bool {
    payload.version >= 2 && !(cfg.history_v2 && payload.txs.is_empty() && payload.proofs.is_empty())
}

/// The statement commitment of a block that ran `txs` on `pre` under `ctx`.
/// Its pre-state is the one the transactions ran on (after the block's system
/// writes: activation, records, payouts, which validators check directly).
pub fn statement(
    ctx: &BlockContext,
    txs: &[TxEnvelope],
    pre: &WorldState,
    out: &BlockOutcome,
) -> [u8; 32] {
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
        let commitment = aether_execution::proofs::claimable(state, c.height, height)
            .map_err(|e| bad(format!("proof of block {}: {e:?}", c.height)))?;
        // The proof's output binds the payout address: nobody can reroute it.
        if !verifier.verify(&bytes, aether_proving::block::claim(commitment, c.prover)) {
            return Err(bad(format!("proof of block {} does not verify", c.height)));
        }
        let amount = if aether_rewards::enabled(state) {
            // Node rewards: issuance only to registered operators, 1/16 of the epoch's proof share each.
            aether_rewards::pay_proof(state, c.height, height, c.prover).map(|(paid, _)| paid)
        } else {
            aether_execution::proofs::pay(state, c.height, height, c.prover)
        }
        .map_err(|e| bad(format!("{e:?}")))?;
        paid.push((c.height, c.prover, amount));
    }
    Ok(paid)
}

/// `schedule` with `u`'s activation appended.
pub fn scheduled(
    schedule: &[crate::upgrade::Activation],
    u: &crate::upgrade::Upgrade,
) -> Arc<crate::upgrade::Schedule> {
    let mut next = schedule.to_vec();
    next.push(crate::upgrade::Activation {
        protocol: u.protocol,
        at: u.activate_at,
        registrar: u.registrar,
    });
    Arc::new(next)
}

#[cfg(test)]
mod pool_tests {
    use super::*;
    use aether_execution::EvmCall;
    use aether_types::{Bytes, TxHeader, TxPayload};

    const GWEI: u128 = 1_000_000_000;

    fn tx(sender: Address, nonce: u64, value: u64) -> TxEnvelope {
        let call = EvmCall {
            to: Some(Address::repeat_byte(0xaa)),
            value: U256::from(value),
            input: Bytes::new(),
            gas_limit: 21_000,
            delegate: None,
        };
        let payload = call.encode();
        let header = TxHeader {
            chain_id: 7780,
            sender,
            nonce,
            gas: GasVector {
                exec: 21_000,
                state: 0,
                prove: 21_000,
            },
            max_fee: FeeVector {
                exec: GWEI,
                state: 0,
                prove: 0,
            },
            tip: GWEI,
            payload_commitment: aether_execution::tx::payload_commitment(&payload),
            scheme: SignerScheme::P256,
        };
        TxEnvelope {
            header,
            payload: TxPayload::Plain(Bytes::from(payload)),
            signature: Bytes::new(),
        }
    }

    fn funded(a: Address, wei: u128) -> WorldState {
        let mut s = WorldState::default();
        s.set_balance(a, U256::from(wei)).unwrap();
        s
    }

    fn cfg(alloc: Vec<(Address, U256)>) -> ChainConfig {
        ChainConfig {
            chain_id: 7780,
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc,
            fees: false,
            registrar: None,
            epoch_blocks: 0,
            min_streak: None,
            draw_epochs: None,
            history_v2: false,
            node_rewards: false,
            reserve: None,
        }
    }

    /// An unsigned envelope (admission checks no signature) with `input` bytes
    /// of calldata and a 3M gas limit, as a deploy-sized call would carry.
    fn fat(sender: Address, nonce: u64, input: Vec<u8>) -> TxEnvelope {
        let call = EvmCall {
            to: Some(Address::repeat_byte(0xaa)),
            value: U256::ZERO,
            input: Bytes::from(input),
            gas_limit: 3_000_000,
            delegate: None,
        };
        let payload = call.encode();
        TxEnvelope {
            header: TxHeader {
                chain_id: 7780,
                sender,
                nonce,
                gas: GasVector { exec: 3_000_000, state: 0, prove: 3_000_000 },
                max_fee: FeeVector::default(),
                tip: 0,
                payload_commitment: aether_execution::tx::payload_commitment(&payload),
                scheme: SignerScheme::P256,
            },
            payload: TxPayload::Plain(Bytes::from(payload)),
            signature: Bytes::new(),
        }
    }

    /// `mempool_candidates` as it was before the 2026-09-29 fix (482400e):
    /// listed txs first, then the rest sorted by (nonce, sender). Kept verbatim
    /// so the regression tests below can show what it did to a listed tx whose
    /// sender's earlier nonces were still in the pool.
    fn candidates_2026_09_28(chain: &Chain) -> Vec<TxEnvelope> {
        let g = chain.lock();
        let censored = |t: &TxEnvelope| {
            g.censor == Some(t.header.sender) || g.deprioritize == Some(t.header.sender)
        };
        let listed = if g.censor.is_some() {
            Vec::new()
        } else {
            g.inclusion.for_proposal()
        };
        let listed_hashes: std::collections::HashSet<TxHash> =
            listed.iter().map(aether_execution::tx_hash).collect();
        let mut rest: Vec<TxEnvelope> = g
            .mempool
            .iter()
            .filter(|(h, t)| !listed_hashes.contains(*h) && !censored(t))
            .map(|(_, t)| t.clone())
            .collect();
        rest.sort_by_key(|t| (t.header.nonce, t.header.sender));
        let mut txs = listed;
        txs.extend(rest);
        txs.truncate(MAX_TXS_PER_BLOCK);
        txs
    }

    /// The 2026-09-28 fix's single nonce order over the whole pool, kept as the
    /// flood regression's baseline: it is what let 2,000 unrelated nonce-0 txs
    /// push a listed tx at a later nonce past the block's tx budget, block
    /// after block, without the inclusion check ever seeing it appendable.
    fn candidates_one_nonce_order(chain: &Chain) -> Vec<TxEnvelope> {
        let g = chain.lock();
        let censored = |t: &TxEnvelope| {
            g.censor == Some(t.header.sender) || g.deprioritize == Some(t.header.sender)
        };
        let listed = if g.censor.is_some() {
            Vec::new()
        } else {
            g.inclusion.for_proposal()
        };
        let listed_hashes: std::collections::HashSet<TxHash> =
            listed.iter().map(aether_execution::tx_hash).collect();
        let rest: Vec<TxEnvelope> = g
            .mempool
            .iter()
            .filter(|(h, t)| !listed_hashes.contains(*h) && !censored(t))
            .map(|(_, t)| t.clone())
            .collect();
        let mut txs: Vec<(bool, TxEnvelope)> = rest.into_iter().map(|t| (false, t)).collect();
        txs.extend(listed.into_iter().map(|t| (true, t)));
        txs.sort_by_key(|(listed, t)| (t.header.nonce, !*listed, t.header.sender));
        let mut txs: Vec<TxEnvelope> = txs.into_iter().map(|(_, t)| t).collect();
        txs.truncate(MAX_TXS_PER_BLOCK);
        txs
    }

    /// Signed transfers from `signer`, one per nonce in `nonces`: cheap enough
    /// that a block holds them all, so what lands is exactly what the ordering
    /// tried, in the order it tried it.
    fn transfers(key: &aether_crypto::P256Signer, nonces: std::ops::Range<u64>) -> Vec<TxEnvelope> {
        nonces
            .map(|nonce| {
                aether_execution::sign_call(key, 7780, nonce, 1, &EvmCall {
                    to: Some(Address::repeat_byte(0xaa)),
                    value: U256::from(1),
                    input: Bytes::new(),
                    gas_limit: 21_000,
                    delegate: None,
                })
                .unwrap()
            })
            .collect()
    }

    /// An inclusion list holding `txs` this node accepted at `seen` (bypassing
    /// gossip verification, which is another node's job).
    fn accept_list(chain: &Chain, txs: Vec<TxEnvelope>, seen: Instant) {
        use commonware_cryptography::Signer as _;
        let key = commonware_cryptography::ed25519::PrivateKey::from_seed(1);
        let il = inclusion::InclusionList::sign(&key, 1, 1_000, txs);
        assert!(chain.lock().inclusion.accept(&il, seen));
    }

    /// Build and execute (but not finalize) the block after `last` with `txs`,
    /// also returning its `BlockContext`: the append check must judge the block
    /// in the same context that built it.
    fn build_ctx(
        chain: &Chain,
        parent: &Arc<Executed>,
        last: &Block,
        txs: Vec<TxEnvelope>,
    ) -> (Arc<Executed>, BlockContext) {
        use crate::block::{Context, EPOCH};
        use commonware_consensus::types::{Round, View};
        use commonware_cryptography::Signer as _;
        let height = last.height.next();
        let leader = commonware_cryptography::ed25519::PrivateKey::from_seed(1).public_key();
        let context = Context { round: Round::new(EPOCH, View::new(height.get())), leader, parent: (View::new(height.get() - 1), last.digest()) };
        let skeleton = Block::new(context.clone(), last.digest(), height, height.get() * 1_000, bytes::Bytes::new());
        let ctx = Chain::block_context(&chain.cfg(), &skeleton, parent);
        let (pre, _) = chain.pre_state_with(parent, parent.next_protocol(), &[], &[], false).unwrap();
        let (payload, _) = build_payload(parent, &pre, &ctx, txs, Extras::default());
        let block = Block::new(context, last.digest(), height, height.get() * 1_000, payload.to_bytes());
        (chain.execute(&block, parent).unwrap(), ctx)
    }

    /// `sender`'s nonces that made it into `exec` (from `txs`), in block order.
    fn landed_nonces(exec: &Executed, txs: &[TxEnvelope], sender: Address) -> Vec<u64> {
        let mine: HashMap<TxHash, u64> = txs
            .iter()
            .filter(|t| t.header.sender == sender)
            .map(|t| (aether_execution::tx_hash(t), t.header.nonce))
            .collect();
        exec.tx_hashes.iter().filter_map(|h| mine.get(h).copied()).collect()
    }

    /// Build and execute (but not finalize) the block after `last` with `txs`.
    fn build(chain: &Chain, parent: &Arc<Executed>, last: &Block, txs: Vec<TxEnvelope>) -> (Block, Arc<Executed>) {
        use crate::block::{Context, EPOCH};
        use commonware_consensus::types::{Round, View};
        use commonware_cryptography::Signer as _;
        let height = last.height.next();
        let leader = commonware_cryptography::ed25519::PrivateKey::from_seed(1).public_key();
        let context = Context { round: Round::new(EPOCH, View::new(height.get())), leader, parent: (View::new(height.get() - 1), last.digest()) };
        let skeleton = Block::new(context.clone(), last.digest(), height, height.get() * 1_000, bytes::Bytes::new());
        let ctx = Chain::block_context(&chain.cfg(), &skeleton, parent);
        let (pre, _) = chain.pre_state_with(parent, parent.next_protocol(), &[], &[], false).unwrap();
        let (payload, _) = build_payload(parent, &pre, &ctx, txs, Extras::default());
        let block = Block::new(context, last.digest(), height, height.get() * 1_000, payload.to_bytes());
        let exec = chain.execute(&block, parent).unwrap();
        (block, exec)
    }

    #[test]
    fn a_tx_its_sender_can_no_longer_pay_for_leaves_the_pool() {
        let a = Address::repeat_byte(1);
        let t = tx(a, 0, 1_000);
        let now = Instant::now();
        let enough = funded(a, 21_000 * GWEI + 1_000);
        assert!(keep_in_pool(
            &t,
            Some(now),
            now,
            &enough,
            FeeVector::default(),
            true
        ));
        let drained = funded(a, 10);
        assert!(
            !keep_in_pool(&t, Some(now), now, &drained, FeeVector::default(), true),
            "admitted while affordable, then the balance went elsewhere"
        );
        assert!(
            keep_in_pool(&t, Some(now), now, &drained, FeeVector::default(), false),
            "without fees there is nothing to pay"
        );
    }

    #[test]
    fn a_tx_waiting_past_the_ttl_leaves_even_if_it_could_still_run() {
        let a = Address::repeat_byte(2);
        let gapped = tx(a, 5, 1); // nonce 0..4 never arrive: it can never run
        let state = funded(a, 10u128.pow(18));
        let t0 = Instant::now();
        assert!(keep_in_pool(
            &gapped,
            Some(t0),
            t0 + MEMPOOL_TTL / 2,
            &state,
            FeeVector::default(),
            true
        ));
        assert!(!keep_in_pool(
            &gapped,
            Some(t0),
            t0 + MEMPOOL_TTL,
            &state,
            FeeVector::default(),
            true
        ));
    }

    #[test]
    fn fee_caps_below_the_base_fee_wait_rather_than_leave() {
        let a = Address::repeat_byte(3);
        let t = tx(a, 0, 1);
        let state = funded(a, 10u128.pow(18));
        let now = Instant::now();
        let high = FeeVector {
            exec: 5 * GWEI,
            state: 0,
            prove: 0,
        };
        assert!(
            admissible(&t, &state, high).is_err(),
            "not admitted while the base fee is above its cap"
        );
        assert!(
            keep_in_pool(&t, Some(now), now, &state, high, true),
            "but an already pending one may wait for the fee to fall"
        );
    }

    #[test]
    fn a_tx_over_the_size_cap_is_refused_while_any_legit_deploy_fits() {
        let sender = Address::repeat_byte(7);
        let (chain, _) = Chain::new(cfg(vec![(sender, U256::from(10u128.pow(22)))]));
        let err = chain.add_to_mempool(fat(sender, 0, vec![9u8; 200 * 1024])).unwrap_err();
        assert!(err.contains("over the") && err.contains("cap"), "{err}");
        assert!(chain.lock().mempool.is_empty(), "nothing was admitted");
        // Deploy initcode is capped at 49 KiB by the EVM (EIP-3860); even twice
        // that stays a normal pool citizen.
        assert_eq!(chain.add_to_mempool(fat(sender, 0, vec![9u8; 100 * 1024])), Ok(true));
        let g = chain.lock();
        let (h, t) = g.mempool.iter().next().unwrap();
        assert_eq!(g.mempool_bytes, t.to_canonical_bytes().len());
        assert_eq!(g.mempool_bytes, g.sizes[h]);
    }

    #[test]
    fn the_byte_budget_bounds_the_pool_and_comes_back_with_a_block() {
        use aether_crypto::{P256Signer, Signer};
        let keys: Vec<P256Signer> = (0..17u8)
            .map(|i| {
                let mut s = [0u8; 32];
                s[0] = 0x5a;
                s[31] = i + 1;
                P256Signer::from_seed(&s).unwrap()
            })
            .collect();
        let senders: Vec<Address> = keys.iter().map(|k| address_of(&k.public_key()).unwrap()).collect();
        let (chain, genesis) = Chain::new(cfg(senders.iter().cloned().map(|a| (a, U256::from(10u128.pow(22)))).collect()));

        // The first sender's earliest nonces, signed, so a block can land them.
        // 130 KB of nonzero calldata floors at 21k + 130k×40 = 5.22M gas
        // (EIP-7623), so the calls carry 6M each.
        let landing: Vec<TxEnvelope> = (0..12)
            .map(|nonce| {
                aether_execution::sign_call(&keys[0], 7780, nonce, 1, &EvmCall {
                    to: Some(Address::repeat_byte(0xaa)),
                    value: U256::ZERO,
                    input: Bytes::from(vec![7u8; 130_000]),
                    gas_limit: 6_000_000,
                    delegate: None,
                })
                .unwrap()
            })
            .collect();
        // Fill past the 64 MiB budget: plenty of count slots, every sender
        // under its per-sender cap, every tx under the size cap, so the byte
        // budget is what stops this. The signed txs go in first so the block
        // below lands exactly what the pool holds.
        let mut admitted = 0;
        let mut full = String::new();
        for t in &landing {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
            admitted += 1;
        }
        'fill: for (i, sender) in senders[0..16].iter().enumerate() {
            let nonces = if i == 0 { 12..MAX_PER_SENDER as u64 } else { 0..MAX_PER_SENDER as u64 };
            for nonce in nonces {
                match chain.add_to_mempool(fat(*sender, nonce, vec![7u8; 130_000])) {
                    Ok(true) => admitted += 1,
                    Err(e) => {
                        full = e;
                        break 'fill;
                    }
                    Ok(false) => unreachable!("a fresh nonce cannot be known"),
                }
            }
        }
        assert_eq!(full, "mempool byte budget full", "the budget, not another cap, fills first");
        {
            let g = chain.lock();
            // One more fat tx would not have fit: the budget, not the loop, stopped it.
            assert!(MAX_MEMPOOL_BYTES - g.mempool_bytes < 130_200);
            assert!(g.mempool_bytes <= MAX_MEMPOOL_BYTES);
            assert_eq!(g.mempool_bytes, g.sizes.values().sum::<usize>(), "the budget is exactly the pool");
            assert_eq!(g.sizes.len(), g.mempool.len());
        }

        // A block lands the first sender's signed earliest nonces (each floors
        // at ~5.2M gas, so a 30M block holds about five); their bytes leave the
        // budget.
        let parent = chain.lock().finalized.clone();
        let (block, exec) = build(&chain, &parent, &genesis, landing);
        let landed = exec.tx_hashes.len();
        assert!(landed >= 2, "{landed} of the fat txs landed");
        let before = chain.lock();
        let (bytes_before, sizes_before) = (before.mempool_bytes, before.sizes.clone());
        drop(before);
        chain.finalize(&block).unwrap();
        let freed: usize = exec.tx_hashes.iter().map(|h| sizes_before[h]).sum();
        assert!(freed > 0);
        {
            let g = chain.lock();
            assert_eq!(g.mempool.len(), admitted - landed);
            assert_eq!(g.mempool_bytes, bytes_before - freed, "the budget comes back with the block");
            assert_eq!(g.sizes.values().sum::<usize>(), g.mempool_bytes);
        }
        assert_eq!(
            chain.add_to_mempool(fat(senders[16], 0, vec![7u8; 130_000])),
            Ok(true),
            "the freed budget admits again"
        );
    }

    /// The 2026-09-29 testnet stall as a unit: one sender with 64 pending txs,
    /// an inclusion list naming nonces 32..47. The listed tx sits at its
    /// sender's nonce 32 while nonces 0..31 are still in the pool, so a proposer
    /// that tried the list first skipped it for good — the voters then saw
    /// "nonce 32 appendable but missing" and refused every proposal at height
    /// 104408. Ordered with the pool (the fix), everything lands and the append
    /// check holds.
    #[test]
    fn listed_txs_at_a_later_nonce_land_after_their_senders_earlier_ones() {
        use aether_crypto::{P256Signer, Signer as _};
        let mut seed = [0u8; 32];
        seed[0] = 0x5b;
        let key = P256Signer::from_seed(&seed).unwrap();
        let sender = address_of(&key.public_key()).unwrap();
        let (chain, genesis) = Chain::new(cfg(vec![(sender, U256::from(10u128.pow(24)))]));
        let pending = transfers(&key, 0..64);
        for t in &pending {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
        }
        let seen = Instant::now();
        accept_list(&chain, pending[32..48].to_vec(), seen); // all 16 list slots
        let now = seen + Duration::from_secs(1); // past inclusion::FREEZE
        let parent = chain.lock().finalized.clone();

        // The fix: one nonce order over listed and unlisted alike.
        let (exec, ctx) = build_ctx(&chain, &parent, &genesis, chain.mempool_candidates());
        assert_eq!(
            landed_nonces(&exec, &pending, sender),
            (0..64).collect::<Vec<u64>>(),
            "a contiguous run, the listed nonces with it"
        );
        assert!(
            chain.inclusion_violations(&exec, &ctx, now).is_empty(),
            "nothing listed is missing and appendable"
        );

        // The pre-fix ordering: the list went before the sender's nonces 0..31,
        // so nonce 32 was tried against state nonce 0, skipped, and never
        // retried (it was not in `rest` either).
        let (exec, ctx) = build_ctx(&chain, &parent, &genesis, candidates_2026_09_28(&chain));
        assert_eq!(
            landed_nonces(&exec, &pending, sender),
            (0..32).collect::<Vec<u64>>(),
            "the listed txs were tried too early and never retried"
        );
        assert_eq!(
            chain.inclusion_violations(&exec, &ctx, now),
            vec![aether_execution::tx_hash(&pending[32])],
            "nonce 32 appendable but missing: the stall every validator saw"
        );
    }

    /// The same with several senders: only the named senders' earlier nonces
    /// hold their listed txs back, and every sender's landed nonces must stay
    /// one contiguous run.
    #[test]
    fn listed_txs_of_several_senders_wait_for_their_own_earlier_nonces() {
        use aether_crypto::{P256Signer, Signer as _};
        let key = |b: u8| {
            let mut seed = [0u8; 32];
            seed[0] = 0x5c;
            seed[31] = b;
            P256Signer::from_seed(&seed).unwrap()
        };
        let (a, b, c) = (key(1), key(2), key(3));
        let (addr_a, addr_b, addr_c) = (
            address_of(&a.public_key()).unwrap(),
            address_of(&b.public_key()).unwrap(),
            address_of(&c.public_key()).unwrap(),
        );
        let (chain, genesis) = Chain::new(cfg(vec![
            (addr_a, U256::from(10u128.pow(24))),
            (addr_b, U256::from(10u128.pow(24))),
            (addr_c, U256::from(10u128.pow(24))),
        ]));
        // A's nonces 32..37 and B's first two txs are listed; C is not named.
        let (tx_a, tx_b, tx_c) = (transfers(&a, 0..64), transfers(&b, 0..2), transfers(&c, 0..16));
        let mut listed = tx_a[32..38].to_vec();
        listed.extend_from_slice(&tx_b);
        for t in tx_a.iter().chain(&tx_b).chain(&tx_c) {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
        }
        let seen = Instant::now();
        accept_list(&chain, listed, seen);
        let now = seen + Duration::from_secs(1);
        let parent = chain.lock().finalized.clone();

        let (exec, ctx) = build_ctx(&chain, &parent, &genesis, chain.mempool_candidates());
        assert_eq!(landed_nonces(&exec, &tx_a, addr_a), (0..64).collect::<Vec<u64>>());
        assert_eq!(landed_nonces(&exec, &tx_b, addr_b), (0..2).collect::<Vec<u64>>());
        assert_eq!(landed_nonces(&exec, &tx_c, addr_c), (0..16).collect::<Vec<u64>>());
        assert!(chain.inclusion_violations(&exec, &ctx, now).is_empty());

        // Before the fix A's listed txs burned their chance before A's nonces
        // 0..31 lifted the state nonce; B's listed txs were at B's next nonce
        // already, so only A's tx went missing.
        let (exec, ctx) = build_ctx(&chain, &parent, &genesis, candidates_2026_09_28(&chain));
        assert_eq!(landed_nonces(&exec, &tx_a, addr_a), (0..32).collect::<Vec<u64>>());
        assert_eq!(landed_nonces(&exec, &tx_b, addr_b), (0..2).collect::<Vec<u64>>());
        assert_eq!(landed_nonces(&exec, &tx_c, addr_c), (0..16).collect::<Vec<u64>>());
        assert_eq!(
            chain.inclusion_violations(&exec, &ctx, now),
            vec![aether_execution::tx_hash(&tx_a[32])]
        );
    }

    /// The other hole in a single nonce order over the pool (2026-09-29
    /// red-team 5): 2,000 unrelated nonce-0 txs fill the block's whole tx
    /// budget before a listed tx at a later nonce is reached — truncated out
    /// every time, though nothing about it is unappendable. Its sender's
    /// earlier-nonce chain goes ahead of the strangers now, so it lands.
    #[test]
    fn a_listed_tx_survives_a_flood_of_unrelated_nonce_zero_txs() {
        use aether_crypto::{P256Signer, Signer as _};
        let mut seed = [0u8; 32];
        seed[0] = 0x5d;
        let key = P256Signer::from_seed(&seed).unwrap();
        let sender = address_of(&key.public_key()).unwrap();
        let strangers: Vec<Address> = (1..=MAX_TXS_PER_BLOCK as u64).map(|i| Address::from_word(U256::from(i).into())).collect();
        let mut alloc = vec![(sender, U256::from(10u128.pow(24)))];
        alloc.extend(strangers.iter().cloned().map(|a| (a, U256::from(10u128.pow(24)))));
        let (chain, genesis) = Chain::new(cfg(alloc));

        let pending = transfers(&key, 0..64);
        for t in &pending {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
        }
        for (i, a) in strangers.iter().enumerate() {
            assert_eq!(chain.add_to_mempool(tx(*a, 0, 1 + i as u64)), Ok(true), "stranger {i}");
        }
        let seen = Instant::now();
        accept_list(&chain, pending[32..48].to_vec(), seen); // all 16 list slots
        let now = seen + Duration::from_secs(1); // past inclusion::FREEZE
        let parent = chain.lock().finalized.clone();

        // The fix: the listed txs and the nonces they stand on come first, so
        // the strangers' volume cannot push them past the 2,000-tx cut. Gas
        // still stops the block mid-stranger (30M gas, 21k a tx) — after the
        // listed chain has landed.
        let (exec, ctx) = build_ctx(&chain, &parent, &genesis, chain.mempool_candidates());
        assert_eq!(
            landed_nonces(&exec, &pending, sender),
            (0..48).collect::<Vec<u64>>(),
            "the sender's nonces through the last listed one, strangers after"
        );
        assert!(
            chain.inclusion_violations(&exec, &ctx, now).is_empty(),
            "nothing listed was pushed past the tx budget"
        );

        // The 2026-09-28 single nonce order this replaces: nonce 0 goes first
        // — 2,001 txs of it — and everything at a later nonce fell past the
        // 2,000-tx cut. Worse, the sender's nonces 0..31 were cut with them,
        // so the inclusion check never saw the listed txs appendable and
        // raised no alarm: the push-out was quiet, block after block.
        let strung_out = candidates_one_nonce_order(&chain);
        assert_eq!(strung_out.len(), MAX_TXS_PER_BLOCK, "the strangers fill the budget");
        assert!(
            strung_out.iter().all(|t| t.header.nonce == 0),
            "and nothing at a later nonce survives the cut"
        );
        let offered: std::collections::HashSet<TxHash> =
            strung_out.iter().map(aether_execution::tx_hash).collect();
        for t in &pending[32..48] {
            assert!(!offered.contains(&aether_execution::tx_hash(t)), "the listed tx was offered");
        }
        let (exec, ctx) = build_ctx(&chain, &parent, &genesis, strung_out);
        assert_eq!(landed_nonces(&exec, &pending, sender), Vec::<u64>::new());
        assert!(
            chain.inclusion_violations(&exec, &ctx, now).is_empty(),
            "nothing appendable missing: the inclusion check stays quiet — that is the hole"
        );
    }
}
