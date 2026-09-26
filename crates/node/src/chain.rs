//! Chain state shared by the consensus application, RPC and gossip.
//!
//! Every verified or proposed block's post-state is kept by digest so
//! competing forks can be validated; the finalized head is what RPC serves.

use crate::block::{Block, Payload, PublicKey};
use crate::inclusion::{self, InclusionPool};
use crate::store::{Commit, Store, StoreError};
use aether_crypto::{address_of, PublicKey as AetherPk};
use aether_execution::{execute_block, fees, BlockContext, BlockOutcome, FeePolicy, Receipt, WorldState};
use aether_types::{Address, FeeVector, GasVector, SignerScheme, TxEnvelope, TxHash, B256, U256};
use commonware_consensus::Heightable;
use commonware_cryptography::{sha256::Digest, Digestible};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const MAX_TXS_PER_BLOCK: usize = 2_000;
pub const MAX_MEMPOOL: usize = 50_000;

#[derive(Clone, Debug)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub limits: GasVector,
    pub alloc: Vec<(Address, U256)>,
    /// Fee policy v0 (docs/research/tokenomics-2026.md): exponential base fees,
    /// base exec burned, prove fees and 20% of tips to the prover escrow.
    pub fees: bool,
}

impl ChainConfig {
    pub fn genesis_state(&self) -> WorldState {
        let mut s = WorldState::default();
        for (a, v) in &self.alloc {
            s.set_balance(*a, *v).expect("genesis balance fits u128");
        }
        // The account contract P-256 accounts delegate to (EIP-7702) for batched calls.
        s.set_code(aether_execution::AETHER_ACCOUNT, aether_execution::aether_account_code()).expect("predeploy");
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
            arrivals: HashMap::new(),
            inclusion: InclusionPool::default(),
            censor: None,
            deprioritize: None,
            store: None,
        };
        (Chain(Arc::new(Mutex::new(inner))), genesis)
    }

    /// Open with durable state: resume from the stored checkpoint, or start at
    /// genesis and persist it.
    pub fn open(cfg: ChainConfig, store: Store) -> Result<(Self, Block), StoreError> {
        let (chain, genesis) = Self::new(cfg);
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
                });
                let mut g = chain.lock();
                g.executed.insert(digest, exec.clone());
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

    pub fn cfg(&self) -> ChainConfig {
        self.lock().cfg.clone()
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
        if let Some(done) = self.get(&block.digest()) {
            return Ok(done);
        }
        let payload = block.payload().ok_or(ChainError::BadPayload)?;
        if payload.parent_state_root != parent.state.root() {
            return Err(ChainError::ParentRootMismatch);
        }
        if payload.txs.len() > MAX_TXS_PER_BLOCK {
            return Err(ChainError::BadPayload);
        }
        let cfg = self.cfg();
        let ctx = Self::block_context(&cfg, block, parent);
        let out = execute_block(&parent.state, &ctx, &payload.txs).map_err(|e| ChainError::Exec(format!("{e:?}")))?;
        if out.bal != payload.bal {
            return Err(ChainError::BalMismatch);
        }
        if out.gas != payload.gas {
            return Err(ChainError::GasMismatch);
        }
        Ok(self.remember(block, parent, &ctx, out, payload.txs.iter().map(aether_execution::tx_hash).collect()))
    }

    pub fn remember(&self, block: &Block, parent: &Executed, ctx: &BlockContext, out: BlockOutcome, tx_hashes: Vec<TxHash>) -> Arc<Executed> {
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
    pub fn add_to_mempool(&self, tx: TxEnvelope) -> bool {
        let h = aether_execution::tx_hash(&tx);
        let mut g = self.lock();
        if g.mempool.len() >= MAX_MEMPOOL || g.receipts.contains_key(&h) || g.mempool.contains_key(&h) {
            return false;
        }
        if tx.header.nonce < g.finalized.state.nonce(&tx.header.sender) {
            return false;
        }
        g.mempool.insert(h, tx);
        g.arrivals.insert(h, Instant::now());
        true
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
                self.execute(block, &parent)?
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
        inner.inclusion.prune(&state, exec.height, Instant::now());
        g.blocks.insert(exec.height, summary);
        g.finalized = exec.clone();
        let floor = exec.height.saturating_sub(64);
        g.executed.retain(|_, e| e.height >= floor);
        Ok(())
    }
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

pub fn build_payload(parent: &Executed, ctx: &BlockContext, candidates: Vec<TxEnvelope>) -> (Payload, aether_execution::BlockOutcome) {
    let (txs, out) = aether_execution::build_block(&parent.state, ctx, candidates);
    (Payload { parent_state_root: parent.state.root(), txs, bal: out.bal.clone(), gas: out.gas }, out)
}
