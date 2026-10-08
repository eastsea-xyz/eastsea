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
    /// Kept outside the legacy postcard layout; these are signed display data.
    #[serde(skip)]
    pub upgrade_notices: Vec<crate::upgrade::SignedUpgrade>,
}

pub(crate) struct SnapshotSource {
    finalized: Arc<Executed>,
    summary: BlockSummary,
    notices: Vec<crate::upgrade::SignedUpgrade>,
}

impl SnapshotSource {
    fn capture(chain: &Chain) -> Self {
        let g = chain.lock();
        SnapshotSource {
            finalized: g.finalized.clone(),
            summary: g.blocks.get(&g.finalized.height).cloned().expect("finalized block summary"),
            notices: g.upgrade_notices.clone(),
        }
    }

    /// Conservative extra allocation for entries, serialization, the notice
    /// envelope and allocator growth. The source state already exists.
    pub(crate) fn estimated_peak_bytes(&self) -> u64 {
        let entries = self.finalized.state.repo().entries().count() as u64;
        let code_bytes: u64 = self.finalized.state.codes().values().map(|c| c.len() as u64).sum();
        entries.saturating_mul(64).saturating_add(code_bytes).saturating_mul(4).saturating_add(16 << 20)
    }

    pub(crate) fn build(self) -> Snapshot {
        let f = &self.finalized;
        Snapshot {
            summary: self.summary,
            entries: f.state.repo().entries().collect(),
            codes: f.state.codes().iter().map(|(k, v)| (*k, v.clone())).collect(),
            history: (*f.history).clone(),
            handoff: f.handoff.as_deref().cloned(),
            seed: f.seed.as_deref().cloned(),
            schedule: (*f.schedule).clone(),
            statement: f.statement,
            upgrade_notices: self.notices,
        }
    }
}

impl Snapshot {
    /// The node's finalized state as a snapshot.
    pub fn of(chain: &Chain) -> Snapshot {
        SnapshotSource::capture(chain).build()
    }

    /// A consistent finalized read handle. Cloning the Arc and small metadata
    /// is the only work done under the consensus mutex.
    pub(crate) fn source(chain: &Chain) -> SnapshotSource {
        SnapshotSource::capture(chain)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut legacy = postcard::to_allocvec(self).expect("snapshot serializes");
        if self.summary.archive_excess == 0
            && !self.schedule.first().is_some_and(|a| a.at == 0 && a.protocol > 1) {
            return legacy;
        }
        let archive_excess = self.summary.archive_excess;
        let notices = if archive_excess == 0 {
            serde_json::to_vec(&self.upgrade_notices)
        } else {
            serde_json::to_vec(&(archive_excess, &self.upgrade_notices))
        }.expect("notices serialize");
        let n = legacy.len();
        // Prepend the envelope in place; a second full-sized Vec would double
        // the serialized state's live memory until this function returned.
        legacy.reserve(8 + notices.len());
        legacy.resize(n + 8, 0);
        legacy.copy_within(0..n, 8);
        legacy[..4].copy_from_slice(if archive_excess == 0 { b"AUN2" } else { b"AUN3" });
        legacy[4..8].copy_from_slice(&(n as u32).to_be_bytes());
        legacy.extend_from_slice(&notices);
        legacy
    }

    pub fn from_bytes(b: &[u8]) -> Result<Snapshot, String> {
        if b.starts_with(b"AUN2") || b.starts_with(b"AUN3") {
            let size = b.get(4..8).ok_or("snapshot notice header")?;
            let n = u32::from_be_bytes(size.try_into().expect("four bytes")) as usize;
            let body = b.get(8..8 + n).ok_or("snapshot notice body")?;
            let mut snap: Snapshot = postcard::from_bytes(body).map_err(|e| format!("snapshot: {e}"))?;
            if b.starts_with(b"AUN3") {
                let (debt, notices) = serde_json::from_slice(&b[8 + n..]).map_err(|e| format!("snapshot archive metadata: {e}"))?;
                snap.summary.archive_excess = debt;
                snap.upgrade_notices = notices;
            } else {
                snap.upgrade_notices = serde_json::from_slice(&b[8 + n..]).map_err(|e| format!("snapshot notices: {e}"))?;
            }
            Ok(snap)
        } else {
            postcard::from_bytes(b).map_err(|e| format!("snapshot: {e}"))
        }
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
        if self.summary.archive_excess > aether_execution::fees::MAX_ENCODED_PAYLOAD_BYTES
            || (!(cfg.node_rewards || cfg.history_v2) && self.summary.archive_excess != 0) {
            return Err("snapshot archive debt is outside this chain's bounds".into());
        }
        // Everything outside the tree: fee excess, pending handoff, seed, protocol schedule, certified by the next block.
        if crate::chain::meta_digest_with_archive(&self.summary.excess, self.handoff.as_ref(), self.seed.as_ref(), &self.schedule, &self.statement, self.summary.archive_excess) != payload.parent_meta {
            return Err("snapshot metadata does not match the certified block".into());
        }
        for notice in &self.upgrade_notices {
            crate::upgrade::verify(identity, notice)?;
            if notice.upgrade.chain_id != cfg.chain_id
                || !self.schedule.iter().any(|a| a.protocol == notice.upgrade.protocol && a.at == notice.upgrade.activate_at) {
                return Err("snapshot notice is not in the certified schedule".into());
            }
        }
        // Code bytes are named by their keccak hash in the tree: check each.
        for (hash, code) in &self.codes {
            if alloy_primitives::keccak256(code) != *hash {
                return Err("snapshot code does not match its hash".into());
            }
        }
        // ...and the other way round (2026-09-29): the state root covers only
        // the tree, and `codes` rides beside it, so a peer could leave a
        // deployed contract's bytes out while every root still matches — the
        // next call into it would fail for want of its code. Every code hash
        // the tree names must arrive with its bytes.
        let carried: std::collections::HashSet<B256> = self.codes.iter().map(|(h, _)| *h).collect();
        for hash in named_code_hashes(&self.entries) {
            if !carried.contains(&hash) {
                return Err(format!("the tree names code {hash} the snapshot does not carry"));
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
            archive_excess: self.summary.archive_excess,
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
            call_targets: vec![],
            tx_hashes: vec![],
            gas: Default::default(),
            new_slots: 0,
            persistent_bytes: 0,
            settlement: aether_execution::fees::Settlement::default(),
            proposer: aether_types::Address::ZERO,
            base_fee: Default::default(),
            excess: self.summary.excess,
            archive_excess: self.summary.archive_excess,
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
        let genesis = crate::block::Block::genesis_with(cfg.chain_id, cfg.genesis_state().root(), cfg.history_v2, cfg.group);
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
                upgrade_notices: &self.upgrade_notices,
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
        self.install_over_with_meta(store, state, old, None)
    }

    pub(crate) fn install_over_with_meta(&self, store: &Store, state: &WorldState, old: impl IntoIterator<Item = ([u8; 32], [u8; 32])>, metadata: Option<(&str, &[u8])>) -> Result<(), String> {
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
        let commit = Commit {
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
                upgrade_notices: &self.upgrade_notices,
                statement: &self.statement,
                staged: None,
            };
        match metadata {
            Some((key, value)) => store.commit_with_meta(commit, key, value),
            None => store.commit(commit),
        }.map_err(|e| e.to_string())
    }
}

/// The code hashes the tree's account stems name, for `check`'s completeness
/// half: a code hash is sub 1 of an account stem — but the stem hash hides the
/// tree index, so code chunks past 128 (sub 0 of their own stems) and overflow
/// storage slots (every sub of their own stems) can wear the same shape, and
/// stems alone cannot be told apart. Two facts discriminate: an account stem
/// (tree index 0) never carries a leaf at subs 2..63, and a code of at most
/// 128 chunks lives wholly in its account's stem — rebuildable from the chunk
/// leaves, so under today's version-0 encoding it must hash to the leaf it
/// names. A stem that fails its check is not an account's: it names nothing.
/// What survives is demanded: whatever the sub-1 leaf holds is what execution
/// indexes `codes` by, so an honest snapshot always carries it. A contrived
/// storage layout can still pass every check and name a hash nobody has: the
/// snapshot is then refused and the jump falls back to replaying, which costs
/// time and nothing else (2026-09-29).
fn named_code_hashes(entries: &[([u8; 32], [u8; 32])]) -> Vec<B256> {
    use aether_state::layout::BasicData;
    /// Per stem: basic data (sub 0), code hash (sub 1), whether any sub 2..63
    /// is written, and the chunk leaves (subs 128..255).
    struct Stem {
        basic: Option<[u8; 32]>,
        hash: Option<[u8; 32]>,
        inner: bool,
        chunks: std::collections::BTreeMap<u8, [u8; 32]>,
    }
    let mut stems: std::collections::HashMap<[u8; 31], Stem> =
        std::collections::HashMap::with_capacity(entries.len());
    for (k, v) in entries {
        if *v == [0u8; 32] {
            continue; // a zero value is no leaf at all
        }
        let stem = stems
            .entry(k[..31].try_into().expect("31 bytes"))
            .or_insert(Stem { basic: None, hash: None, inner: false, chunks: Default::default() });
        match k[31] {
            0 => stem.basic.get_or_insert(*v),
            1 => stem.hash.get_or_insert(*v),
            2..=63 => {
                stem.inner = true;
                continue;
            }
            128..=255 => {
                stem.chunks.insert(k[31], *v);
                continue;
            }
            _ => continue,
        };
    }
    let chunk = |chunks: &std::collections::BTreeMap<u8, [u8; 32]>, k: u8| {
        chunks.get(&k).copied().unwrap_or([0u8; 32])
    };
    stems
        .into_iter()
        .filter_map(|(_, s)| {
            let (basic, hash) = (s.basic?, s.hash?);
            let d = BasicData::decode(&basic);
            if d.code_size == 0 || s.inner {
                return None;
            }
            // A code of at most 128 chunks never leaves the account's stem:
            // rebuild it (an unwritten chunk leaf is 31 zero code bytes) and,
            // under version-0 semantics, hold it to the hash it names.
            let size = d.code_size as usize;
            if size <= 128 * 31 {
                if d.version != 0 {
                    return Some(B256::from(hash));
                }
                let mut code = Vec::with_capacity(size);
                for k in 0..size.div_ceil(31) {
                    code.extend_from_slice(&chunk(&s.chunks, 128 + k as u8)[1..]);
                }
                code.truncate(size);
                if alloy_primitives::keccak256(&code) != hash {
                    return None;
                }
            }
            Some(B256::from(hash))
        })
        .collect()
}

/// The certified block's hash matches the snapshot summary (used by tests).
pub fn block_hash(b: &Block) -> String {
    format!("{}", b.digest())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ChainConfig {
        ChainConfig {
            chain_id: 7781,
            limits: aether_types::GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: vec![], fees: false, registrar: None, epoch_blocks: 0,
            min_streak: None, draw_epochs: None, history_v2: false, protocol: 1,
            node_rewards: false, committee: vec![], reserve: None, group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
        }
    }

    #[test]
    fn snapshot_build_does_not_hold_consensus_mutex() {
        let (chain, _) = Chain::new(cfg());
        let source = Snapshot::source(&chain);
        let guard = chain.lock();
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || tx.send(source.build().summary.height).unwrap());
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap(), 0,
            "snapshot entry copy must complete while consensus holds its mutex");
        drop(guard);
        thread.join().unwrap();
    }

    #[test]
    #[ignore = "manual physical-footprint measurement of a synthetic large state"]
    fn snapshot_build_allocation_probe() {
        let mut config = cfg();
        config.history_v2 = true;
        config.protocol = 2;
        let (chain, _) = Chain::new(config);
        let entries: Vec<_> = (0..100_000u32).map(|i| {
            let mut key = [0u8; 32];
            key[..4].copy_from_slice(&i.to_be_bytes());
            (key, [7u8; 32])
        }).collect();
        let state = WorldState::from_parts(entries, Default::default());
        {
            let mut g = chain.lock();
            let mut head = (*g.finalized).clone();
            head.state = state;
            g.finalized = Arc::new(head);
        }
        let pid = std::process::id();
        let before = crate::resources::footprint(pid).unwrap_or(0);
        let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let peak = Arc::new(std::sync::atomic::AtomicU64::new(before));
        let sampler = {
            let running = running.clone();
            let peak = peak.clone();
            std::thread::spawn(move || while running.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some(n) = crate::resources::footprint(pid) {
                    peak.fetch_max(n, std::sync::atomic::Ordering::Relaxed);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            })
        };
        let source = Snapshot::source(&chain);
        let estimate = source.estimated_peak_bytes();
        let snap = source.build();
        let wire = snap.to_bytes();
        running.store(false, std::sync::atomic::Ordering::Relaxed);
        sampler.join().unwrap();
        eprintln!("snapshot allocation probe: entries={}, wire={}, estimated_extra={}, sampled_footprint_delta={}",
            snap.entries.len(), wire.len(), estimate, peak.load(std::sync::atomic::Ordering::Relaxed).saturating_sub(before));
    }

    fn leaf(sub: u8, stem: &[u8; 31], v: [u8; 32]) -> ([u8; 32], [u8; 32]) {
        let mut k = [0u8; 32];
        k[..31].copy_from_slice(stem);
        k[31] = sub;
        (k, v)
    }

    fn basic(code_size: u32) -> [u8; 32] {
        aether_state::layout::BasicData { version: 0, code_size, nonce: 0, balance: 7 }
            .encode()
            .expect("encode")
    }

    /// An account stem for `code`: basic data claiming its size, its keccak as
    /// the code-hash leaf, and its chunkified code at subs 128...
    fn coded_stem(stem: &[u8; 31], code: &[u8]) -> Vec<([u8; 32], [u8; 32])> {
        let mut e = vec![
            leaf(0, stem, basic(code.len() as u32)),
            leaf(1, stem, alloy_primitives::keccak256(code).0),
        ];
        e.extend(
            aether_state::layout::chunkify_code(code)
                .into_iter()
                .enumerate()
                .map(|(k, c)| leaf(128 + k as u8, stem, c)),
        );
        e
    }

    #[test]
    fn only_an_account_stem_names_its_code_hash() {
        let stem = [7u8; 31];
        // A contract's own stem names its code hash (rebuilt and held to it).
        let code = (0..120u8).collect::<Vec<_>>();
        assert_eq!(
            named_code_hashes(&coded_stem(&stem, &code)),
            vec![B256::from(alloy_primitives::keccak256(&code).0)]
        );
        // A hash the rebuilt code does not hash to names nothing: the stem is
        // not an account's (storage wearing the shape).
        let mut wrong = coded_stem(&stem, &code);
        wrong[1].1[0] ^= 1;
        assert!(named_code_hashes(&wrong).is_empty());
        // A plain account: no code claimed, nothing named.
        assert!(named_code_hashes(&[leaf(0, &stem, basic(0))]).is_empty());
        // An overflow storage slot on sub 1 of its own stem is not a code hash.
        assert!(named_code_hashes(&[leaf(1, &[8u8; 31], [5; 32])]).is_empty());
        // A big contract's continuation stem wears its chunks at every sub from
        // 0 (chunk 128 lands at sub 0 of the next stem, so its "basic data" and
        // "code hash" are just code bytes). The leaves at subs 2..63 rule it
        // out: an account stem never writes there.
        let continuation: Vec<_> = aether_state::layout::chunkify_code(&code)
            .into_iter()
            .enumerate()
            .map(|(k, c)| leaf(k as u8, &[9u8; 31], c))
            .collect();
        assert!(named_code_hashes(&continuation).is_empty());
        // A code too big for one stem (more than 128 chunks) cannot be rebuilt
        // and checked: its account stem still names the hash.
        let big = (0..=u8::MAX).cycle().take(128 * 31 + 5).collect::<Vec<_>>();
        let hash = alloy_primitives::keccak256(&big).0;
        let mut big_stem = coded_stem(&stem, &big[..128 * 31]);
        big_stem[0].1 = basic(big.len() as u32);
        big_stem[1].1 = hash;
        assert_eq!(named_code_hashes(&big_stem), vec![B256::from(hash)]);
        // A zeroed code-hash leaf names nothing (a cleared account).
        let mut cleared = coded_stem(&stem, &code);
        cleared[1].1 = [0; 32];
        assert!(named_code_hashes(&cleared).is_empty());
    }
}
