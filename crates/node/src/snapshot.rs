//! Checkpoint sync (docs/research/history-compression-2026.md, step 2): a new
//! Mac starts from a certified state instead of re-executing from genesis.
//!
//! A snapshot is the state at finalized height H (every tree entry and code)
//! plus what the next block needs that the state root does not cover: block
//! H's summary (fee-market excess, timestamp, hash), the history MMR peaks,
//! and the latest committee handoff and draw seed. It is checked against block
//! H+1, whose certificate the node verifies under the committee identity it
//! pins: H+1 builds on H's hash, commits H's state root and its history root.
//! Anything the roots do not cover is either signed by the committee (handoff,
//! seed) or makes the next block fail to execute to its certified result.

use crate::block::Block;
use crate::chain::{BlockSummary, Chain, ChainConfig, Executed};
use crate::store::{Commit, Store};
use aether_execution::{Journal, WorldState};
use aether_types::{Bytes, B256};
use commonware_consensus::Heightable;
use commonware_cryptography::sha256::Digest;
use commonware_cryptography::Digestible;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub summary: BlockSummary,
    pub entries: Vec<([u8; 32], [u8; 32])>,
    pub codes: Vec<(B256, Bytes)>,
    pub history: aether_state::mmr::Mmr,
    pub handoff: Option<crate::handoff::Pending>,
    pub seed: Option<(u64, aether_light::block::Seed)>,
    pub schedule: crate::upgrade::Schedule,
    pub statement: crate::chain::Statement,
}

impl Snapshot {
    /// The node's finalized state as a snapshot.
    pub fn of(chain: &Chain) -> Snapshot {
        let g = chain.lock();
        let f = &g.finalized;
        Snapshot {
            summary: g.blocks.get(&f.height).cloned().expect("finalized block summary"),
            entries: f.state.repo().entries().collect(),
            codes: f.state.codes().iter().map(|(k, v)| (*k, v.clone())).collect(),
            history: (*f.history).clone(),
            handoff: f.handoff.as_deref().cloned(),
            seed: f.seed.as_deref().cloned(),
            schedule: (*f.schedule).clone(),
            statement: f.statement,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("snapshot serializes")
    }

    pub fn from_bytes(b: &[u8]) -> Result<Snapshot, String> {
        postcard::from_bytes(b).map_err(|e| format!("snapshot: {e}"))
    }

    /// Check the snapshot against `next`, the certified block after it (the
    /// caller verified `next`'s certificate). Returns the rebuilt state.
    pub fn check(&self, next: &Block, cfg: &ChainConfig, identity: &aether_light::Identity) -> Result<WorldState, String> {
        let payload = next.payload().ok_or("next block has no payload")?;
        if next.height().get() != self.summary.height + 1 || format!("{}", next.parent) != self.summary.hash {
            return Err("the certified block does not build on the snapshot's block".into());
        }
        let state = WorldState::from_parts(self.entries.clone(), self.codes.iter().cloned().collect());
        if state.root() != payload.parent_state_root || state.root() != self.summary.state_root {
            return Err("snapshot state does not match the certified state root".into());
        }
        let h = aether_hash::ChainHasher::new();
        // Peak heights follow from the leaf count (they drive later appends).
        let heights: Vec<u8> = (0..64u8).rev().filter(|b| self.history.leaves >> b & 1 == 1).collect();
        if self.history.peaks.iter().map(|(ht, _)| *ht).collect::<Vec<_>>() != heights {
            return Err("snapshot history peaks do not fit its size".into());
        }
        if B256::from(self.history.root(&h)) != payload.history_root || self.history.leaves != self.summary.height + 1 {
            return Err("snapshot history does not match the certified history root".into());
        }
        // Everything outside the tree: fee excess, pending handoff, seed, protocol schedule, certified by the next block.
        if crate::chain::meta_digest(&self.summary.excess, self.handoff.as_ref(), self.seed.as_ref(), &self.schedule, &self.statement) != payload.parent_meta {
            return Err("snapshot metadata does not match the certified block".into());
        }
        // Code bytes are named by their keccak hash in the tree: check each.
        for (hash, code) in &self.codes {
            if alloy_primitives::keccak256(code) != *hash {
                return Err("snapshot code does not match its hash".into());
            }
        }
        if let Some(p) = &self.handoff {
            crate::handoff::verify(cfg.chain_id, identity, &p.handoff)?;
        }
        if let Some((_, s)) = &self.seed {
            crate::handoff::verify_seed(cfg.chain_id, identity, s)?;
        }
        Ok(state)
    }

    /// Only certified facts of block H's summary (hash, state root, fee
    /// excess); the rest is left empty rather than served unverified.
    fn minimal_summary(&self) -> BlockSummary {
        BlockSummary {
            height: self.summary.height,
            hash: self.summary.hash.clone(),
            parent: String::new(),
            timestamp_ms: 0,
            proposer: Default::default(),
            state_root: self.summary.state_root,
            parent_state_root: Default::default(),
            txs: vec![],
            gas_used: 0,
            prove_gas: 0,
            base_fee: Default::default(),
            excess: self.summary.excess,
        }
    }

    /// The head this snapshot installs, for `Chain::adopt`: an `Executed`
    /// holding only what later blocks and RPC answers need — the certified
    /// state and the handoff, seed, schedule, statement and fee excess the
    /// certified block after the snapshot committed (timestamp and proposer
    /// stay empty: nothing uncertified is served).
    pub fn head(&self, state: WorldState) -> (Arc<Executed>, BlockSummary) {
        let summary = self.minimal_summary();
        let digest: [u8; 32] = hex::decode(&self.summary.hash).ok().and_then(|b| b.try_into().ok()).expect("snapshot block hash");
        let exec = Arc::new(Executed {
            height: self.summary.height,
            digest: Digest(digest),
            timestamp: 0,
            state,
            receipts: vec![],
            tx_hashes: vec![],
            gas: Default::default(),
            proposer: aether_types::Address::ZERO,
            base_fee: Default::default(),
            excess: self.summary.excess,
            handoff: self.handoff.clone().map(Arc::new),
            seed: self.seed.clone().map(Arc::new),
            history: Arc::new(self.history.clone()),
            schedule: Arc::new(self.schedule.clone()),
            statement: self.statement,
            payouts: vec![],
            registration_ids: vec![],
        });
        (exec, summary)
    }

    /// Write the checked snapshot as the store's checkpoint; `Chain::open` then resumes from it.
    pub fn install(&self, store: &Store, state: &WorldState, cfg: &ChainConfig) -> Result<(), String> {
        let summary = self.minimal_summary();
        if store.head().map_err(|e| e.to_string())?.is_some() {
            return Err("the store already holds a chain".into());
        }
        // The data belongs to this network's genesis (Chain::open checks it).
        let genesis = crate::block::Block::genesis_with(cfg.chain_id, cfg.genesis_state().root(), cfg.history_v2);
        store.put_meta(crate::chain::GENESIS, &crate::chain::genesis_digest(&genesis)).map_err(|e| e.to_string())?;
        let digest: [u8; 32] = hex::decode(&self.summary.hash).ok().and_then(|b| b.try_into().ok()).ok_or("snapshot block hash")?;
        let diff = Journal { writes: self.entries.iter().map(|(k, v)| (*k, Some(*v))).collect(), codes: self.codes.clone() };
        store
            .commit(Commit {
                height: self.summary.height,
                digest,
                root: state.root(),
                diff: &diff,
                summary: &summary,
                receipts: vec![],
                handoff: self.handoff.as_ref(),
                seed: self.seed.as_ref(),
                history: &self.history,
                schedule: &self.schedule,
                statement: &self.statement,
                staged: None,
            })
            .map_err(|e| e.to_string())
    }

    /// Swap the checked snapshot in over a store that already holds a chain
    /// (`follow::jump`): one atomic commit drops the keys the old finalized
    /// state had that the snapshot does not and writes every changed snapshot
    /// entry, so a later `Store::load` rebuilds exactly the snapshot's state.
    /// The old and new entries are held in memory while writing (the snapshot
    /// already was, during the download). Old rows outside the state tree —
    /// summaries, receipts, era files — stay: they are certified history of
    /// this same chain.
    pub fn install_over(&self, store: &Store, state: &WorldState, old: impl IntoIterator<Item = ([u8; 32], [u8; 32])>) -> Result<(), String> {
        if store.head().map_err(|e| e.to_string())?.is_none() {
            return Err("no chain to jump over".into());
        }
        let old: HashMap<[u8; 32], [u8; 32]> = old.into_iter().collect();
        let kept: std::collections::HashSet<&[u8; 32]> = self.entries.iter().map(|(k, _)| k).collect();
        // A `None` write removes a key the snapshot drops; changed entries
        // overwrite in place. The two never name the same key, and unchanged
        // entries need no write at all.
        let mut writes: Vec<([u8; 32], Option<[u8; 32]>)> = Vec::with_capacity(old.len().max(self.entries.len()));
        for k in old.keys() {
            if !kept.contains(k) {
                writes.push((*k, None));
            }
        }
        writes.extend(self.entries.iter().filter(|(k, v)| old.get(k) != Some(v)).map(|(k, v)| (*k, Some(*v))));
        let summary = self.minimal_summary();
        let digest: [u8; 32] = hex::decode(&self.summary.hash).ok().and_then(|b| b.try_into().ok()).ok_or("snapshot block hash")?;
        let diff = Journal { writes, codes: self.codes.clone() };
        store
            .commit(Commit {
                height: self.summary.height,
                digest,
                root: state.root(),
                diff: &diff,
                summary: &summary,
                receipts: vec![],
                handoff: self.handoff.as_ref(),
                seed: self.seed.as_ref(),
                history: &self.history,
                schedule: &self.schedule,
                statement: &self.statement,
                staged: None,
            })
            .map_err(|e| e.to_string())
    }
}

/// The certified block's hash matches the snapshot summary (used by tests).
pub fn block_hash(b: &Block) -> String {
    format!("{}", b.digest())
}
