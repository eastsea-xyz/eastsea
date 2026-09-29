//! Era shards (roadmap B5; design in docs/design/15-node-rewards.md "C. 보관").
//!
//! Each sealed era file is cut into 32 Reed-Solomon shards of which any 16
//! restore it (`commonware_coding::ReedSolomon` over BLAKE3: a Merkle
//! commitment over the shards, a path per shard). Every registered candidate
//! Mac keeps the shards a public draw assigns it, so old eras survive even
//! when few Macs keep whole era files, at 2x the era's size across the
//! network instead of one copy per Mac.
//!
//! Phase 1 (mainnet day one) is holding and checking only: no consensus
//! changes, no reward weight, nothing on 7780 (history v2 networks only).
//!
//! Trust: a shard is checked against the era's shard commitment, and the
//! commitment is only as good as whoever computed it. Phase 1 needs nothing
//! more, because the decoded bytes are an era file, which is checked against
//! the era root in the certified history (`era::read`, `era_net::verify`); a
//! bad commitment or a bad shard only makes a restore fail, and the node asks
//! other holders. A later phase puts each era's shard commitment on chain
//! (the beacon carries shard proofs), which storage rewards would need.
//!
//! Assignment: for each sealed era, a draw seed
//! BLAKE3("aether-shard-draw" ‖ era ‖ era root). The era root is public,
//! kept forever by every node's history index, and nobody knows it before
//! the era is sealed, so nobody can pick which eras it will hold (a
//! committee-signed draw seed would not be fresher here; it comes with the
//! beacon phase). Within an era, shard `i` goes to the `replicas` candidate
//! nodes with the lowest BLAKE3("aether-shard" ‖ seed ‖ era ‖ i ‖ node id),
//! skipping nodes already holding their `max_shards` budget (default 64, the
//! 50 GB disk setting). Eras are assigned newest sealed era first, so a
//! node's budget fills with recent data and an old shard ages out as new
//! eras are sealed. Every node computes the same assignment from public
//! data (the registry's candidate node ids and the history index's era
//! roots); the node set drifts as Macs register and leave, which only moves
//! statistics in phase 1.
//!
//! Checking (phase 1, off-chain): a node that holds a shard of an era knows
//! the era's commitment, so it can check any other holder's answer for any
//! shard of that era (`aether_shard` over RPC). Results are kept per
//! candidate for the last 7 days and served as a public statistic
//! (`aether_shardStats`); a Mac that answers with a shard that fails its
//! Merkle path is counted failed. Nothing is paid, nothing is slashed.

use crate::chain::Chain;
use commonware_codec::{Decode as _, DecodeExt as _, Encode as _, EncodeSize as _};
use commonware_coding::{CodecConfig, Config, ReedSolomon, Scheme};
use commonware_cryptography::Blake3;
use commonware_parallel::Sequential;
use std::collections::BTreeMap;
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};

/// Shards any restore needs.
pub const MIN_SHARDS: u16 = 16;
/// Shards per era.
pub const TOTAL_SHARDS: u16 = 32;
/// Macs that hold each shard when there are that many candidates.
pub const REPLICAS: usize = 3;
/// Default shard budget per Mac (the 50 GB disk setting of docs/design/15-node-rewards.md).
pub const DEFAULT_MAX_SHARDS: usize = 64;
/// How long challenge results are kept.
pub const STATS_WINDOW: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);
/// How often a node reconciles what it holds and checks one shard.
pub const INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
/// Largest shard served over RPC in one answer (hex doubles it; both the
/// loopback HTTP and iroh paths carry more, but era files this big are rare).
pub const SERVE_MAX: usize = 8 << 20;
/// Largest shard read back from disk (an era file is at most `era_net::MAX_ERA_FILE`).
pub fn max_shard_size() -> usize {
    crate::era_net::MAX_ERA_FILE / MIN_SHARDS as usize + (1 << 20)
}

type Rs = ReedSolomon<Blake3>;
pub type Commitment = <Rs as Scheme>::Commitment;
pub type Shard = <Rs as Scheme>::Shard;
pub type CheckedShard = <Rs as Scheme>::CheckedShard;

pub fn config() -> Config {
    Config {
        minimum_shards: NonZeroU16::new(MIN_SHARDS).expect("nonzero"),
        extra_shards: NonZeroU16::new(TOTAL_SHARDS - MIN_SHARDS).expect("nonzero"),
    }
}

#[derive(Debug)]
pub enum ShardError {
    Coding(String),
    /// Fewer than `MIN_SHARDS` distinct checked shards.
    TooFew(usize),
    Encoding,
}

impl std::fmt::Display for ShardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

/// Cut an era file into `TOTAL_SHARDS` shards and their commitment.
pub fn encode(era_file: &[u8]) -> Result<(Commitment, Vec<Shard>), ShardError> {
    Rs::encode(&config(), era_file, &Sequential).map_err(|e| ShardError::Coding(format!("{e:?}")))
}

/// Check shard `index` against `commitment` (its Merkle path).
pub fn check(commitment: &Commitment, index: u16, shard: &Shard) -> Result<CheckedShard, ShardError> {
    Rs::check(&config(), commitment, index, shard).map_err(|e| ShardError::Coding(format!("{e:?}")))
}

/// Restore the era file from any `MIN_SHARDS` checked shards.
pub fn decode<'a>(commitment: &Commitment, shards: impl Iterator<Item = &'a CheckedShard>) -> Result<Vec<u8>, ShardError> {
    let shards: Vec<&CheckedShard> = shards.collect();
    if shards.len() < MIN_SHARDS as usize {
        return Err(ShardError::TooFew(shards.len()));
    }
    Rs::decode(&config(), commitment, shards.into_iter(), &Sequential).map_err(|e| ShardError::Coding(format!("{e:?}")))
}

/// A shard's bytes for disk or the wire.
pub fn to_bytes(shard: &Shard) -> Vec<u8> {
    shard.encode().to_vec()
}

/// A shard from bytes (at most `max` bytes of shard data).
pub fn from_bytes(bytes: &[u8], max: usize) -> Result<Shard, ShardError> {
    Shard::decode_cfg(bytes, &CodecConfig { maximum_shard_size: max }).map_err(|_| ShardError::Encoding)
}

// ---------------------------------------------------------------- assignment

/// The draw seed of an era: BLAKE3 over the era and its root. Public (the
/// history index keeps every era root), unpredictable before the era is
/// sealed, and the only input every node — including one that pruned the
/// era and fetches it back later — can compute on its own.
pub fn draw_seed(era: u64, era_root: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"aether-shard-draw");
    h.update(&era.to_be_bytes());
    h.update(era_root);
    *h.finalize().as_bytes()
}

fn score(seed: &[u8; 32], era: u64, shard: u16, node: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"aether-shard");
    h.update(seed);
    h.update(&era.to_be_bytes());
    h.update(&shard.to_be_bytes());
    h.update(node);
    *h.finalize().as_bytes()
}

/// Candidate nodes in draw order for one shard: lowest score first, node id
/// as the tie-break, so the order never depends on how the set was listed.
fn ranked(seed: &[u8; 32], era: u64, shard: u16, nodes: &[[u8; 32]]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_cached_key(|&i| (score(seed, era, shard, &nodes[i]), nodes[i]));
    order
}

/// The phase-1 assignment: the shards of eras `0..roots.len()` every node
/// holds. Eras are taken newest first; each shard goes to the `replicas`
/// lowest-ranked nodes that are still under `cap` shards, so a Mac's budget
/// fills with the newest eras and the oldest shards are the first to lose
/// holders. The result depends only on the (deduplicated) node set, the era
/// roots, `cap` and `replicas`: every node computes the same lists.
pub fn assignment(nodes: &[[u8; 32]], roots: &[[u8; 32]], cap: usize, replicas: usize) -> Vec<Vec<(u64, u16)>> {
    let mut nodes = nodes.to_vec();
    nodes.sort_unstable();
    nodes.dedup();
    let mut load = vec![0usize; nodes.len()];
    let mut held: Vec<Vec<(u64, u16)>> = vec![Vec::new(); nodes.len()];
    for era in (0..roots.len() as u64).rev() {
        if load.iter().all(|l| *l >= cap) {
            break; // nothing older can be assigned
        }
        let seed = draw_seed(era, &roots[era as usize]);
        for shard in 0..TOTAL_SHARDS {
            let mut taken = 0;
            for &i in &ranked(&seed, era, shard, &nodes) {
                if taken == replicas {
                    break;
                }
                if load[i] >= cap {
                    continue;
                }
                load[i] += 1;
                taken += 1;
                held[i].push((era, shard));
            }
        }
    }
    held
}

/// One node's share of [`assignment`]: its shard list, newest era first
/// (empty when `node` is not in the candidate set).
pub fn assigned(node: &[u8; 32], nodes: &[[u8; 32]], roots: &[[u8; 32]], cap: usize, replicas: usize) -> Vec<(u64, u16)> {
    let mut sorted = nodes.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    match sorted.binary_search(node) {
        Ok(i) => assignment(nodes, roots, cap, replicas)[i].clone(),
        Err(_) => Vec::new(),
    }
}

// -------------------------------------------------------------- what a node holds

/// The magic and version of a shard file on disk.
const FILE_MAGIC: &[u8; 8] = b"AETHSHRD";
const FILE_VERSION: u8 = 1;

fn file_name(era: u64, shard: u16) -> String {
    format!("era-{era:08}.{shard:02}.shard")
}

/// A shard file's bytes: magic, version, era, index, the era's commitment,
/// then the shard's own encoding (which carries its Merkle path).
fn file_bytes(era: u64, index: u16, commitment: &Commitment, shard: &Shard) -> Vec<u8> {
    let mut out = Vec::with_capacity(51 + shard.encode_size());
    out.extend_from_slice(FILE_MAGIC);
    out.push(FILE_VERSION);
    out.extend_from_slice(&era.to_le_bytes());
    out.extend_from_slice(&index.to_le_bytes());
    out.extend_from_slice(commitment.as_ref());
    out.extend_from_slice(&to_bytes(shard));
    out
}

/// Read a shard file back; `Ok(None)` when there is no such file.
fn parse_file(bytes: &[u8]) -> Result<(u64, u16, Commitment, Shard), String> {
    let bad = || format!("shard file is not the format this node writes ({} B)", bytes.len());
    if bytes.len() < 51 || &bytes[..8] != FILE_MAGIC || bytes[8] != FILE_VERSION {
        return Err(bad());
    }
    let era = u64::from_le_bytes(bytes[9..17].try_into().expect("8"));
    let index = u16::from_le_bytes(bytes[17..19].try_into().expect("2"));
    let commitment = Commitment::decode(&bytes[19..51]).map_err(|_| bad())?;
    let shard = from_bytes(&bytes[51..], max_shard_size()).map_err(|_| bad())?;
    Ok((era, index, commitment, shard))
}

/// What one pass over the chain sees: the assignment from this node's own
/// view of the registry and the history index.
pub struct View {
    /// Candidate node ids, sorted.
    pub nodes: Vec<[u8; 32]>,
    /// Roots of the sealed eras.
    pub roots: Vec<[u8; 32]>,
    /// This node's shard list (empty when it is no candidate).
    pub mine: Vec<(u64, u16)>,
    /// Every node's shard list, in `nodes` order.
    pub all: Vec<Vec<(u64, u16)>>,
}

impl View {
    /// The current assignment from `chain`'s finalized state and `me`'s place
    /// in it. `None` on networks without era files (not history v2) and while
    /// the node has no history index.
    pub fn of(chain: &Chain, cap: usize, me: Option<&[u8; 32]>) -> Option<View> {
        let g = chain.lock();
        if !g.cfg.history_v2 {
            return None;
        }
        let roots = g.history_index.as_ref()?.eras.clone();
        let mut nodes = aether_execution::registry::candidates(&g.finalized.state)
            .into_iter()
            .map(|c| c.node_id)
            .collect::<Vec<_>>();
        drop(g);
        nodes.sort_unstable();
        nodes.dedup();
        let all = assignment(&nodes, &roots, cap, REPLICAS);
        let mine = me.and_then(|m| nodes.binary_search(m).ok()).map(|i| all[i].clone()).unwrap_or_default();
        Some(View { nodes, roots, mine, all })
    }
}

/// The shards this Mac holds: the files under its data dir, the challenge
/// statistics it keeps about other candidates, and its own identity.
pub struct Shards {
    /// `<data>/shards`: this Mac's shard files and `stats.json`.
    dir: PathBuf,
    /// This Mac's candidate node id (its iroh key), when it has one.
    me: Option<[u8; 32]>,
    /// Shards this Mac holds at most.
    pub cap: usize,
    /// Challenge results per candidate node id: (unix seconds, verified).
    stats: std::sync::Mutex<BTreeMap<[u8; 32], Vec<(u64, bool)>>>,
}

impl Shards {
    /// The shard store of a node with `data` as its data dir. Loads the
    /// statistics kept over the last 7 days.
    pub fn new(data: &Path, me: Option<[u8; 32]>, cap: usize) -> Shards {
        let s = Shards { dir: data.join("shards"), me, cap, stats: std::sync::Mutex::new(BTreeMap::new()) };
        s.load_stats();
        s
    }

    fn stats_path(&self) -> PathBuf {
        self.dir.join("stats.json")
    }

    fn load_stats(&self) {
        let Ok(bytes) = std::fs::read(self.stats_path()) else { return };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return };
        let now = unix();
        let mut stats = self.stats.lock().expect("shard stats");
        for (node, entries) in v.as_object().into_iter().flatten() {
            let Ok(node) = hex::decode(node) else { continue };
            let Ok(node) = node.try_into() else { continue };
            let kept: Vec<(u64, bool)> = entries
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| Some((e[0].as_u64()?, e[1].as_bool()?)))
                .filter(|(ts, _)| *ts + STATS_WINDOW.as_secs() > now)
                .collect();
            if !kept.is_empty() {
                stats.insert(node, kept);
            }
        }
    }

    fn save_stats(&self, stats: &BTreeMap<[u8; 32], Vec<(u64, bool)>>) {
        let v: serde_json::Value = stats
            .iter()
            .map(|(n, e)| (hex::encode(n), serde_json::Value::Array(e.iter().map(|(ts, ok)| serde_json::json!([ts, ok])).collect())))
            .collect::<serde_json::Map<_, _>>()
            .into();
        if let Err(e) = std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(self.stats_path(), serde_json::to_vec(&v).unwrap_or_default())) {
            tracing::warn!(%e, "could not write shard stats");
        }
    }

    /// Record a challenge result about the candidate `node` and keep the
    /// file to the last 7 days.
    pub fn record(&self, node: &[u8; 32], verified: bool) {
        self.record_at(unix(), node, verified)
    }

    fn record_at(&self, now: u64, node: &[u8; 32], verified: bool) {
        let cutoff = now.saturating_sub(STATS_WINDOW.as_secs());
        let mut stats = self.stats.lock().expect("shard stats");
        let entry = stats.entry(*node).or_default();
        entry.push((now, verified));
        entry.retain(|(ts, _)| *ts > cutoff);
        if entry.is_empty() {
            stats.remove(node);
        }
        self.save_stats(&stats);
    }

    /// The shard files on disk, sorted.
    pub fn held(&self) -> Vec<(u64, u16)> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return out };
        for e in rd.flatten() {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(rest) = name.strip_prefix("era-") else { continue };
            let Some((era, rest)) = rest.split_once('.') else { continue };
            let Some((shard, _)) = rest.split_once('.') else { continue };
            if let (Ok(era), Ok(shard)) = (era.parse::<u64>(), shard.parse::<u16>()) {
                out.push((era, shard));
            }
        }
        out.sort_unstable();
        out
    }

    /// Write one shard (temporary name, then renamed).
    fn write(&self, era: u64, index: u16, commitment: &Commitment, shard: &Shard) -> Result<(), String> {
        use std::io::Write;
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let path = self.dir.join(file_name(era, index));
        let tmp = self.dir.join(format!("{}.tmp", file_name(era, index)));
        let bytes = file_bytes(era, index, commitment, shard);
        {
            let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
            f.write_all(&bytes).and_then(|_| f.sync_all()).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    /// Read shard (`era`, `index`) back: `Ok(None)` when not held, `Err` when
    /// the file is not a shard this node could have written or no longer
    /// matches its commitment (corrupted on disk: the holder rewrites it, and
    /// it is never served).
    pub fn read(&self, era: u64, index: u16) -> Result<Option<(Commitment, Shard)>, String> {
        let Ok(bytes) = std::fs::read(self.dir.join(file_name(era, index))) else { return Ok(None) };
        let (_, _, c, s) = parse_file(&bytes)?;
        check(&c, index, &s).map_err(|e| format!("shard {era}/{index} fails its own commitment: {e}"))?;
        Ok(Some((c, s)))
    }

    /// Delete the shard files that are no longer assigned; returns how many went.
    fn drop_except(&self, keep: &[(u64, u16)]) -> usize {
        let mut dropped = 0;
        for (era, shard) in self.held() {
            if !keep.contains(&(era, shard)) {
                if std::fs::remove_file(self.dir.join(file_name(era, shard))).is_ok() {
                    dropped += 1;
                }
            }
        }
        dropped
    }

    /// `aether_shard`: this node's shard (`era`, `index`) for a peer to check,
    /// with the era's commitment and this node's candidate id. `Ok(None)`: not held.
    pub fn serve(&self, era: u64, index: u16) -> Result<Option<serde_json::Value>, String> {
        let Some((commitment, shard)) = self.read(era, index)? else { return Ok(None) };
        let bytes = to_bytes(&shard);
        if bytes.len() > SERVE_MAX {
            return Err(format!("shard {era}/{index} is {} B: over the {} B serving limit", bytes.len(), SERVE_MAX));
        }
        Ok(Some(serde_json::json!({
            "era": era, "index": index,
            "node": self.me.map(hex::encode),
            "commitment": hex::encode(commitment.as_ref()),
            "shard": hex::encode(bytes),
        })))
    }

    /// `aether_shardStats`: this node's own holding and the challenge results
    /// it observed, per candidate, over the last 7 days. `chain` supplies the
    /// current assignment view (public data; every node computes the same).
    pub fn stats(&self, chain: &Chain) -> serde_json::Value {
        let now = unix();
        let cutoff = now.saturating_sub(STATS_WINDOW.as_secs());
        let stats = self.stats.lock().expect("shard stats").clone();
        let held = self.held();
        let mut candidates = Vec::new();
        if let Some(v) = View::of(chain, self.cap, self.me.as_ref()) {
            for (node, shards) in v.nodes.iter().zip(&v.all) {
                let observed = stats.get(node).map(|e| e.iter().filter(|(ts, _)| *ts > cutoff).collect::<Vec<_>>()).unwrap_or_default();
                let ok = observed.iter().filter(|e| e.1).count();
                candidates.push(serde_json::json!({
                    "node": hex::encode(node),
                    "assigned": shards.len(),
                    "checked": observed.len(),
                    "ok": ok,
                    "failed": observed.len() - ok,
                }));
            }
        }
        serde_json::json!({
            "me": self.me.map(hex::encode),
            "cap": self.cap,
            "replicas": REPLICAS,
            "window_days": STATS_WINDOW.as_secs() / 86_400,
            "held": held.iter().map(|(era, shard)| serde_json::json!({ "era": era, "shard": shard })).collect::<Vec<_>>(),
            "held_bytes": held.iter().filter_map(|(e, s)| std::fs::metadata(self.dir.join(file_name(*e, *s))).ok().map(|m| m.len())).sum::<u64>(),
            "candidates": candidates,
        })
    }
}

fn unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ------------------------------------------------------- holding and checking

/// Bring what this Mac holds in line with the assignment: write the shard
/// files it is owed — one era per call, newest first, from the era file it
/// keeps or one fetched and verified over the B4 path — and drop the ones it
/// no longer holds. A Mac that is not a registered candidate holds nothing.
pub async fn reconcile(chain: &Chain, upstream: Option<&crate::follow::Upstream>, s: &Shards) {
    let Some(v) = View::of(chain, s.cap, s.me.as_ref()) else { return };
    if v.roots.is_empty() {
        return;
    }
    if s.me.is_none_or(|me| !v.nodes.contains(&me)) {
        let dropped = s.drop_except(&[]);
        if dropped > 0 {
            tracing::info!(dropped, "no longer a registered candidate: holding no era shards");
        }
        return;
    }
    s.drop_except(&v.mine);
    // Below the free-space floor no new shard files are written (dropping what
    // is no longer assigned still ran above: it only frees space).
    if !crate::resources::disk_ok() {
        return;
    }
    let mut eras: Vec<u64> = v.mine.iter().map(|(e, _)| *e).collect();
    eras.dedup();
    for era in eras {
        let want: Vec<u16> = v
            .mine
            .iter()
            .filter(|&&(e, _)| e == era)
            .map(|&(_, shard)| shard)
            .filter(|&shard| s.read(era, shard).ok().flatten().is_none())
            .collect();
        if want.is_empty() {
            continue;
        }
        let root = v.roots[era as usize];
        let (mut bytes, mut fetched) = (chain.store().and_then(|st| crate::era_net::kept(&st, era)), false);
        if bytes.is_none() {
            let Some(u) = upstream else {
                tracing::debug!(era, "era file not kept here and no upstream to fetch it from: holding its shards waits");
                return;
            };
            match crate::era_net::fetch_into(chain, u, era).await {
                Ok(path) => match std::fs::read(&path) {
                    Ok(b) => {
                        bytes = Some(b);
                        fetched = true;
                    }
                    Err(e) => {
                        tracing::debug!(%e, era, "fetched era unreadable");
                        return;
                    }
                },
                Err(e) => {
                    tracing::debug!(%e, era, "could not fetch the era to shard");
                    return;
                }
            }
        }
        let Some(bytes) = bytes else { return };
        // Encoding trusts nothing on disk: the file is read back against the
        // era root in the certified history before it is cut.
        let encoded = tokio::task::spawn_blocking(move || {
            crate::era::read(&bytes, Some(&root)).map_err(|e| format!("era {era}: {e}"))?;
            encode(&bytes).map_err(|e| format!("era {era}: {e}"))
        })
        .await;
        let (commitment, shards) = match encoded {
            Ok(Ok(cut)) => cut,
            Ok(Err(e)) => {
                tracing::warn!(era, %e, "era does not read back: not sharded");
                return;
            }
            Err(e) => {
                tracing::warn!(%e, era, "shard encoding task failed");
                return;
            }
        };
        let mut written = 0;
        for &shard in &want {
            match s.write(era, shard, &commitment, &shards[shard as usize]) {
                Ok(()) => written += 1,
                Err(e) => tracing::warn!(era, shard, %e, "could not write a shard"),
            }
        }
        if fetched {
            // The era file was fetched only to cut: this node does not keep it.
            if let Some(st) = chain.store() {
                let _ = std::fs::remove_file(st.era_dir().join(crate::era::file_name(era)));
            }
        }
        tracing::info!(era, shards = written, total = s.held().len(), "era shards written");
        return; // one era per pass: the next pass takes the next one
    }
}

/// Verify a peer's `aether_shard` answer against the `commitment` we hold
/// for the era. `Ok` is the candidate the answer names; `Err` carries that
/// candidate (when the answer named one) so a bad shard is still attributed.
pub fn check_answer(commitment: &Commitment, era: u64, index: u16, answer: &serde_json::Value) -> Result<[u8; 32], (Option<[u8; 32]>, String)> {
    let node = answer["node"].as_str().and_then(|n| hex::decode(n).ok()).and_then(|n| <[u8; 32]>::try_from(n).ok());
    let wrong = |what: &str| Err((node, what.to_string()));
    if answer["era"].as_u64() != Some(era) || answer["index"].as_u64() != Some(index as u64) {
        return wrong("answer is for another shard");
    }
    if answer["commitment"].as_str().map(String::from) != Some(hex::encode(commitment.as_ref())) {
        return wrong("answer names another era's commitment");
    }
    let shard = answer["shard"]
        .as_str()
        .and_then(|s| hex::decode(s).ok())
        .ok_or((node, "no shard bytes".to_string()))?;
    let shard = from_bytes(&shard, SERVE_MAX).map_err(|e| (node, format!("shard bytes: {e}")))?;
    check(commitment, index, &shard).map(|_| ()).map_err(|e| (node, format!("merkle path: {e}")))?;
    node.ok_or((None, "answer names no candidate".to_string()))
}

/// Check one random shard of an era we hold (so the era's commitment is
/// known to us): ask the upstream for any shard of that era and record
/// whether it checks. The reachable set in phase 1 is the upstream's (the
/// validators a follower follows, the configured sources); a challenge that
/// reaches a node holding nothing is simply not counted.
pub async fn challenge(chain: &Chain, upstream: &crate::follow::Upstream, s: &Shards) {
    let held = s.held();
    if held.is_empty() {
        return;
    }
    let Some(v) = View::of(chain, s.cap, s.me.as_ref()) else { return };
    let era = held[(rand::random::<u64>() as usize) % held.len()].0;
    let Some((commitment, _)) = s.read(era, held.iter().find(|(e, _)| *e == era).map(|(_, s)| *s).unwrap_or(0)).ok().flatten() else {
        return;
    };
    let index = (rand::random::<u64>() % TOTAL_SHARDS as u64) as u16;
    let answer = match upstream.call("aether_shard", serde_json::json!([era, index])).await {
        Ok(a) => a,
        Err(e) => {
            tracing::debug!(%e, era, index, "shard challenge could not ask a peer");
            return;
        }
    };
    if answer.is_null() {
        return; // this peer holds shard (era, index) not: nothing to count
    }
    let known = |n: &[u8; 32]| v.nodes.binary_search(n).is_ok();
    match check_answer(&commitment, era, index, &answer) {
        Ok(node) => {
            if known(&node) {
                s.record(&node, true);
            }
            tracing::info!(era, index, node = %hex::encode(node), "shard challenge answered and verified");
        }
        Err((node, e)) => {
            tracing::warn!(era, index, %e, "shard challenge failed");
            // Phase 1 attributes a failure to the candidate the answer names
            // (statistics only; the answer itself is not authenticated).
            if let Some(node) = node.filter(|n| known(n)) {
                s.record(&node, false);
            }
        }
    }
}

/// The phase-1 loop (spawned only on history v2 networks): every `INTERVAL`,
/// check one shard of a peer and bring what this Mac holds in line with the
/// assignment. Nothing here touches consensus or rewards.
pub async fn run(chain: Chain, upstream: Option<std::sync::Arc<crate::follow::Upstream>>, s: std::sync::Arc<Shards>) {
    tracing::info!(cap = s.cap, replicas = REPLICAS, "era shards on: holding what the draw assigns (phase 1: checks are public statistics, no reward weight)");
    loop {
        if let Some(u) = upstream.as_deref() {
            challenge(&chain, u, &s).await;
        }
        reconcile(&chain, upstream.as_deref(), &s).await;
        tokio::time::sleep(INTERVAL).await;
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    fn data(n: usize, salt: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(n);
        let mut x = blake3::hash(&[salt]);
        while out.len() < n {
            out.extend_from_slice(x.as_bytes());
            x = blake3::hash(x.as_bytes());
        }
        out.truncate(n);
        out
    }

    fn checked(c: &Commitment, shards: &[Shard]) -> Vec<CheckedShard> {
        shards.iter().enumerate().map(|(i, s)| check(c, i as u16, s).unwrap()).collect()
    }

    #[test]
    fn any_16_of_32_restore_the_era() {
        let era = data(300_001, 1);
        let (c, shards) = encode(&era).unwrap();
        assert_eq!(shards.len(), TOTAL_SHARDS as usize);
        let all = checked(&c, &shards);
        let total: usize = shards.iter().map(|s| to_bytes(s).len()).sum();
        println!("{} B era -> {} B in 32 shards ({:.2}x)", era.len(), total, total as f64 / era.len() as f64);
        assert!(total < era.len() * 23 / 10, "about 2x plus Merkle paths");
        // Data shards only, parity shards only, interleaved, and pseudo-random subsets.
        let mut subsets: Vec<Vec<usize>> = vec![(0..16).collect(), (16..32).collect(), (0..32).step_by(2).collect(), (1..32).step_by(2).collect()];
        for k in 0..20u8 {
            let mut idx: Vec<usize> = (0..32).collect();
            idx.sort_by_key(|i| blake3::hash(&[k, *i as u8]).as_bytes()[0]);
            idx.truncate(16);
            subsets.push(idx);
        }
        for s in subsets {
            let back = decode(&c, s.iter().map(|&i| &all[i])).unwrap();
            assert_eq!(back, era, "subset {s:?}");
        }
        // 15 are not enough.
        assert!(matches!(decode(&c, all[..15].iter()), Err(ShardError::TooFew(15))));
    }

    #[test]
    fn bad_shards_and_foreign_commitments_are_refused() {
        let (c, shards) = encode(&data(50_000, 2)).unwrap();
        // A shard under another index, or with a flipped bit, fails its check.
        assert!(check(&c, 1, &shards[0]).is_err());
        let mut bytes = to_bytes(&shards[3]);
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let flipped = from_bytes(&bytes, 1 << 20).map(|s| check(&c, 3, &s).is_err()).unwrap_or(true);
        assert!(flipped, "a corrupted shard never checks");
        // Shards round-trip through bytes.
        assert_eq!(from_bytes(&to_bytes(&shards[5]), 1 << 20).unwrap(), shards[5]);
        // Shards of another era do not check against this era's commitment.
        let (c2, shards2) = encode(&data(50_000, 3)).unwrap();
        assert_ne!(c, c2);
        assert!(check(&c, 0, &shards2[0]).is_err());
        // Mixing shards checked under different commitments does not decode.
        let a = checked(&c, &shards);
        let b = checked(&c2, &shards2);
        let mixed: Vec<&CheckedShard> = a[..8].iter().chain(b[8..16].iter()).collect();
        assert!(decode(&c, mixed.into_iter()).is_err());
    }

    fn nodes_of(n: usize, salt: u8) -> Vec<[u8; 32]> {
        (0..n as u8).map(|i| *blake3::hash(&[salt, b'n', i]).as_bytes()).collect()
    }

    fn roots_of(eras: usize, salt: u8) -> Vec<[u8; 32]> {
        (0..eras as u8).map(|i| *blake3::hash(&[salt, b'r', i]).as_bytes()).collect()
    }

    /// Every shard's holder count, by era: `holders[e][s]` lists the nodes.
    fn holders_of(all: &[Vec<(u64, u16)>]) -> Vec<Vec<usize>> {
        let eras = all.iter().flat_map(|l| l.iter().map(|(e, _)| *e)).max().map_or(0, |m| m + 1) as usize;
        let mut out = vec![Vec::new(); eras * TOTAL_SHARDS as usize];
        for (node, list) in all.iter().enumerate() {
            for &(e, s) in list {
                out[e as usize * TOTAL_SHARDS as usize + s as usize].push(node);
            }
        }
        out
    }

    #[test]
    fn the_assignment_is_deterministic_even_and_capped() {
        for n in [1usize, 4, 20, 100] {
            let (nodes, roots) = (nodes_of(n, 9), roots_of(20, 7));
            let per_node = REPLICAS.min(n);
            // A budget with room for every shard of all 20 eras (twice the
            // mean load): the draw alone decides, no budget binds.
            let roomy = 2 * TOTAL_SHARDS as usize * per_node * 20 / n + 1;
            let all = assignment(&nodes, &roots, roomy, REPLICAS);
            assert_eq!(assignment(&nodes, &roots, roomy, REPLICAS), all, "N={n}: deterministic");
            // The input order of the candidate set never matters.
            let mut shuffled = nodes.clone();
            shuffled.reverse();
            assert_eq!(assignment(&shuffled, &roots, roomy, REPLICAS), all, "N={n}");
            assert!(all.iter().all(|l| l.len() <= roomy), "N={n}");
            // Enough budget: every shard of every era has `replicas` holders.
            for (s, holders) in holders_of(&all).iter().enumerate() {
                assert_eq!(holders.len(), per_node, "N={n}: shard {s}");
                let mut unique = holders.clone();
                unique.sort_unstable();
                unique.dedup();
                assert_eq!(unique.len(), holders.len(), "N={n}: a node holds a shard twice");
            }
            // Evenly: 20 eras × 32 shards × `per_node` holders over n nodes.
            // A random draw spreads loads by about √mean, so allow mean ±
            // 5√mean (and never a node under a third or over three times the
            // mean).
            let loads: Vec<usize> = all.iter().map(|l| l.len()).collect();
            let mean = 32 * per_node * 20 / n;
            let spread = 5.0 * (mean as f64).sqrt();
            let lo = ((mean as f64 - spread).max(mean as f64 / 3.0).ceil()) as usize;
            let hi = ((mean as f64 + spread).min(mean as f64 * 3.0).floor()) as usize;
            println!("N={n}: shards per node min {} max {} (mean {mean}, even within {lo}..={hi})", loads.iter().min().unwrap(), loads.iter().max().unwrap());
            assert!(loads.iter().all(|l| *l >= lo && *l <= hi), "N={n}: {loads:?}");
            // The per-node view agrees with the whole assignment (which is in
            // sorted node order).
            let mut sorted = nodes.clone();
            sorted.sort_unstable();
            sorted.dedup();
            for (i, node) in sorted.iter().enumerate() {
                assert_eq!(assigned(node, &nodes, &roots, roomy, REPLICAS), all[i], "N={n}: node {i}");
            }
            // A foreign node holds nothing.
            assert!(assigned(&[7u8; 32], &nodes, &roots, roomy, REPLICAS).is_empty());
            // Another set of era roots is another assignment (one node holds
            // every shard whatever the draw, so there is nothing to compare).
            if n > 1 {
                assert_ne!(assignment(&nodes, &roots_of(20, 8), roomy, REPLICAS), all, "N={n}");
            }
            // The default budget (64, the 50 GB setting): no node over it, and
            // the newest era is always fully replicated — a node can hold at
            // most 32 shards of one era, so no budget binds while it is drawn.
            let real = assignment(&nodes, &roots, DEFAULT_MAX_SHARDS, REPLICAS);
            assert!(real.iter().all(|l| l.len() <= DEFAULT_MAX_SHARDS), "N={n}");
            let newest = roots.len() as u64 - 1;
            for s in 0..TOTAL_SHARDS {
                let holders = real.iter().filter(|l| l.contains(&(newest, s))).count();
                assert_eq!(holders, per_node, "N={n}: shard {s} of the newest era");
            }
        }
    }

    #[test]
    fn a_small_budget_fills_with_the_newest_eras_first() {
        let (nodes, roots) = (nodes_of(4, 3), roots_of(6, 4));
        let all = assignment(&nodes, &roots, 40, REPLICAS);
        // 40 shards each over 4 nodes (160): the newest era takes 96 of them
        // and the leftover 64 go to the next one.
        assert!(all.iter().all(|l| l.len() == 40), "exactly the budget: {:?}", all.iter().map(|l| l.len()).collect::<Vec<_>>());
        let holders = holders_of(&all);
        let of = |era: usize| holders[era * TOTAL_SHARDS as usize..(era + 1) * TOTAL_SHARDS as usize].to_vec();
        assert!(of(5).iter().all(|h| h.len() == REPLICAS), "the newest era is fully held");
        assert_eq!(of(4).iter().map(|h| h.len()).sum::<usize>(), 64, "the leftover budget goes to the next era");
        assert!(of(4).iter().all(|h| h.len() <= REPLICAS), "nothing is over-replicated");
        assert!(of(3).iter().all(|h| h.is_empty()), "nothing beyond the two newest eras");
        assert!(of(0).iter().all(|h| h.is_empty()), "the oldest era aged out of every budget");
        // A node leaving moves only the shards it held (a roomy budget: pure
        // rendezvous hashing, nothing is rerouted by a cap). Indices in both
        // assignments are into the sorted node set; the leaver is sorted[0],
        // so every other index shifts down by one.
        let mut sorted = nodes.clone();
        sorted.sort_unstable();
        let roomy = 2 * TOTAL_SHARDS as usize * 6;
        let before = holders_of(&assignment(&nodes, &roots, roomy, REPLICAS));
        let fewer: Vec<[u8; 32]> = sorted.iter().copied().filter(|n| *n != sorted[0]).collect();
        let after = holders_of(&assignment(&fewer, &roots, roomy, REPLICAS));
        for (s, b) in before.iter().enumerate() {
            let kept = b.iter().filter(|&&n| n != 0 && after[s].contains(&(n - 1))).count();
            let expect = b.iter().filter(|&&n| n != 0).count();
            assert_eq!(kept, expect, "shard {s}: only the leaver's replicas moved");
        }
    }

    #[test]
    fn shard_files_round_trip_and_drop() {
        let dir = std::env::temp_dir().join(format!("aether-shards-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Shards::new(&dir, Some(*nodes_of(1, 0).first().unwrap()), DEFAULT_MAX_SHARDS);
        let (c, shards) = encode(&data(100_000, 5)).unwrap();
        s.write(3, 7, &c, &shards[7]).unwrap();
        s.write(3, 8, &c, &shards[8]).unwrap();
        assert_eq!(s.held(), vec![(3, 7), (3, 8)]);
        let (back, shard) = s.read(3, 7).unwrap().unwrap();
        assert_eq!(back, c);
        assert_eq!(to_bytes(&shard), to_bytes(&shards[7]));
        assert!(s.read(3, 9).unwrap().is_none(), "not held");
        check(&c, 7, &shard).unwrap();
        // Serving carries the commitment and the node's own id.
        let v = s.serve(3, 7).unwrap().unwrap();
        assert_eq!(v["node"], serde_json::json!(hex::encode(s.me.unwrap())));
        check_answer(&c, 3, 7, &v).unwrap();
        // A wrong index, a flipped byte, a foreign commitment: the answer fails.
        let mut bad = v.clone();
        bad["index"] = serde_json::json!(8);
        assert!(check_answer(&c, 3, 7, &bad).is_err());
        let mut flipped = v.clone();
        let mut bytes = hex::decode(flipped["shard"].as_str().unwrap()).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        flipped["shard"] = serde_json::json!(hex::encode(bytes));
        assert!(check_answer(&c, 3, 7, &flipped).is_err());
        let mut foreign = v.clone();
        let (c2, _) = encode(&data(100_000, 6)).unwrap();
        foreign["commitment"] = serde_json::json!(hex::encode(c2.as_ref()));
        assert!(check_answer(&c, 3, 7, &foreign).is_err());
        // Not assigned any more: the files go.
        assert_eq!(s.drop_except(&[(9, 1)]), 2);
        assert!(s.held().is_empty());
        assert!(s.serve(3, 7).unwrap().is_none());
        // A corrupted file is reported, never served as good.
        std::fs::create_dir_all(&dir.join("shards")).unwrap();
        let mut bytes = std::fs::read(dir.join("shards").join(file_name(3, 7))).unwrap_or_default();
        if bytes.len() < 60 {
            s.write(3, 7, &c, &shards[7]).unwrap();
            bytes = std::fs::read(dir.join("shards").join(file_name(3, 7))).unwrap();
        }
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(dir.join("shards").join(file_name(3, 7)), &bytes).unwrap();
        assert!(s.read(3, 7).is_err(), "a corrupted shard file never reads back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn statistics_keep_seven_days_and_survive_restart() {
        let dir = std::env::temp_dir().join(format!("aether-shards-stats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let now = 1_800_000_000u64;
        {
            let s = Shards::new(&dir, None, 8);
            s.record_at(now, &[1u8; 32], true);
            s.record_at(now, &[1u8; 32], false);
        }
        // A restart loads what is left; entries older than the window are not
        // loaded, even if the file still carries them.
        {
            let path = dir.join("shards").join("stats.json");
            let mut v = serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap();
            v[&hex::encode([2u8; 32])] = serde_json::json!([[unix() - STATS_WINDOW.as_secs() - 1, true]]);
            v[&hex::encode([3u8; 32])] = serde_json::json!([[unix(), true]]);
            std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
            let s = Shards::new(&dir, None, 8);
            {
                let stats = s.stats.lock().expect("shard stats");
                assert_eq!(stats.get(&[1u8; 32]).unwrap(), &vec![(now, true), (now, false)]);
                assert!(!stats.contains_key(&[2u8; 32]), "older than the window");
                assert!(stats.contains_key(&[3u8; 32]), "fresh");
            }
            s.record_at(now, &[1u8; 32], true); // a restart keeps recording
            assert_eq!(s.stats.lock().expect("shard stats").get(&[1u8; 32]).unwrap().len(), 3);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
