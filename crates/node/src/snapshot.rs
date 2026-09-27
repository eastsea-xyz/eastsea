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
use crate::chain::{BlockSummary, Chain, ChainConfig};
use crate::store::{Commit, Store};
use aether_execution::{Journal, WorldState};
use aether_types::{Bytes, B256};
use commonware_consensus::Heightable;
use commonware_cryptography::Digestible;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub summary: BlockSummary,
    pub entries: Vec<([u8; 32], [u8; 32])>,
    pub codes: Vec<(B256, Bytes)>,
    pub history: aether_state::mmr::Mmr,
    pub handoff: Option<crate::handoff::Pending>,
    pub seed: Option<(u64, aether_light::block::Seed)>,
    pub schedule: crate::upgrade::Schedule,
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
        if crate::chain::meta_digest(&self.summary.excess, self.handoff.as_ref(), self.seed.as_ref(), &self.schedule) != payload.parent_meta {
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

    /// Write the checked snapshot as the store's checkpoint; `Chain::open` then resumes from it.
    /// Only certified facts are kept of block H's summary (hash, state root, fee
    /// excess); the rest is left empty rather than served unverified.
    pub fn install(&self, store: &Store, state: &WorldState, cfg: &ChainConfig) -> Result<(), String> {
        let summary = BlockSummary {
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
        };
        if store.head().map_err(|e| e.to_string())?.is_some() {
            return Err("the store already holds a chain".into());
        }
        // The data belongs to this network's genesis (Chain::open checks it).
        let genesis = crate::block::Block::genesis(cfg.chain_id, cfg.genesis_state().root());
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
            })
            .map_err(|e| e.to_string())
    }
}

/// The certified block's hash matches the snapshot summary (used by tests).
pub fn block_hash(b: &Block) -> String {
    format!("{}", b.digest())
}
