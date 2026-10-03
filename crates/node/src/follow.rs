//! Follower node (docs/design/12-launch-plan.md P1): any Mac runs the full
//! chain without being a validator.
//!
//! It pulls each finalized block and its finality certificate from validators,
//! checks the certificate against the committee key, re-executes the block
//! (state root, block access list and gas must match, as a validator checks
//! them), and persists the result. Wallets on this Mac then ask it instead of a
//! remote node; transactions they submit are forwarded upstream. Nothing a
//! validator sends is trusted beyond the certificate.
//!
//! Catch-up: blocks are fetched in pipelined batches (many requests in flight,
//! executed in order), and a Mac that slept for hours — more than
//! `JUMP_BEHIND` blocks behind — jumps to the network's certified snapshot
//! instead of replaying (checked against the certified block after it, as a
//! checkpoint start is; the gap's blocks stay fetchable from era files).
//! `catch_up` runs the same machinery for a validator before it starts voting.

use crate::block::Block;
use crate::chain::Chain;
use aether_light::{from_hex, verify_finalized_chain, ValidatorSet, MAX_BLOCK_BYTES};
use commonware_codec::Decode as _;
use futures::{StreamExt as _, TryStreamExt as _};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;
use tracing::{info, warn};

/// How far behind the network a node jumps to a certified snapshot instead of
/// replaying (~30 min of 1 s blocks).
pub const JUMP_BEHIND: u64 = 2_000;
/// Marks a storage failure in a follow error, so the loop heals the store
/// instead of retrying the same block on a dead database (the incident of
/// 2026-09-29: a full disk, and 288 retries of one block).
const STORE_FAILED: &str = "storage failed";
/// How far behind a node stops acting as a validator (proposing, voting,
/// answering beacons) until it has caught up.
pub const BEHIND_MARGIN: u64 = 20;
/// Blocks fetched (and verified) in parallel while following: enough that a
/// round trip is amortized over many blocks (a 150 ms link still syncs at
/// hundreds of blocks a second) without holding long-lived buffers.
const PIPELINE: u64 = 256;
/// Snapshot chunks fetched in parallel.
const SNAPSHOT_PARALLEL: usize = 8;

/// Certified blocks this follower verified, served to wallets and other
/// followers as `aether_getFinalized` (the same JSON a validator answers with).
/// Kept in the node's store, so the history survives restarts and stays
/// served when this Mac becomes a voting node.
#[derive(Default)]
pub struct FinalityArchive {
    store: Option<std::sync::Arc<crate::store::Store>>,
    /// Without a store (tests): the most recent `KEEP` heights in memory.
    inner: Mutex<BTreeMap<u64, Value>>,
}

const KEEP: usize = 8_192;

impl FinalityArchive {
    pub fn new(store: Option<std::sync::Arc<crate::store::Store>>) -> Self {
        FinalityArchive { store, inner: Default::default() }
    }

    pub fn insert(&self, height: u64, proof: Value) {
        if let Some(s) = &self.store {
            if let Err(e) = s.put_proof(height, proof.to_string().as_bytes()) {
                warn!(height, %e, "could not keep the finality proof");
            }
            return;
        }
        let mut g = self.inner.lock().expect("archive lock");
        g.insert(height, proof);
        while g.len() > KEEP {
            g.pop_first();
        }
    }

    pub fn get(&self, height: u64) -> Option<Value> {
        match &self.store {
            Some(s) => s.proof(height).ok().flatten().and_then(|b| serde_json::from_slice(&b).ok()),
            None => self.inner.lock().expect("archive lock").get(&height).cloned(),
        }
    }
}

/// Largest snapshot a new Mac downloads. It is held in memory (twice, while
/// decoding) before the certified block checks it, so it stays well under the
/// smallest Mac's memory; replaying from genesis remains the fallback.
const MAX_SNAPSHOT: usize = 1 << 30;
/// Smallest chunk accepted (bounds the number of requests).
const MIN_SNAPSHOT_CHUNK: usize = 64 << 10;

/// Largest upstream response accepted (a block and its certificate, hex encoded).
const MAX_RESPONSE: usize = 4 * MAX_BLOCK_BYTES as usize + (1 << 20);

/// Where certified blocks come from: validators' RPC over HTTP (local networks)
/// or over iroh (found by node id on the Mainline DHT).
pub enum Upstream {
    Http(Vec<String>),
    /// The client, and how many answers in a row gave nothing new.
    Iroh(aether_net::RpcClient, std::sync::atomic::AtomicU32),
}

impl Upstream {
    /// Ask each source in turn until `accept` takes an answer. Every answer
    /// that came back — including "nothing new at the tip" — is a step of
    /// work (red team #2): a watcher comparing `activity` across polls can
    /// tell a node that is asking from one where nothing moves.
    async fn ask<T>(&self, method: &str, params: Value, accept: impl Fn(Value) -> Result<Option<T>, String> + Send + Sync) -> Result<Option<T>, String> {
        let answer = self.ask_upstream(method, params, &accept).await;
        if answer.is_ok() {
            crate::chain::tick();
        }
        answer
    }

    async fn ask_upstream<T>(&self, method: &str, params: Value, accept: &(dyn Fn(Value) -> Result<Option<T>, String> + Send + Sync)) -> Result<Option<T>, String> {
        match self {
            Upstream::Iroh(c, misses) => {
                use std::sync::atomic::Ordering::Relaxed;
                let answer = c.call(method, params).await.map_err(|e| e.to_string()).and_then(accept);
                // Switch validators when one serves data that does not verify, or has
                // had nothing new for a while (it may be lagging behind the others).
                let stale = match &answer {
                    Ok(Some(_)) => {
                        misses.store(0, Relaxed);
                        false
                    }
                    Ok(None) => misses.fetch_add(1, Relaxed) + 1 >= 10,
                    Err(_) => true,
                };
                if stale {
                    misses.store(0, Relaxed);
                    c.rotate().await;
                }
                answer
            }
            Upstream::Http(urls) => {
                let mut last = Err(String::from("no upstream"));
                for url in urls {
                    match http_call(url, method, &params).await.and_then(accept) {
                        Ok(Some(v)) => return Ok(Some(v)),
                        other => last = other,
                    }
                }
                last
            }
        }
    }

    /// The first non-null answer (null when every source has none).
    pub async fn first(&self, method: &str, params: Value) -> Result<Value, String> {
        self.ask(method, params, |v| Ok((!v.is_null()).then_some(v))).await.map(|v| v.unwrap_or(Value::Null))
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.ask(method, params, |v| Ok(Some(v))).await.map(|v| v.unwrap_or(Value::Null))
    }
}

/// One pooled client for every upstream call: a follower catching up asks
/// dozens of requests a second, and a client per request paid a TCP (or TLS)
/// handshake every time.
fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

async fn http_call(url: &str, method: &str, params: &Value) -> Result<Value, String> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let mut r = http().post(url).json(&body).timeout(Duration::from_secs(10)).send().await.map_err(|e| e.to_string())?;
    if r.content_length().is_some_and(|n| n as usize > MAX_RESPONSE) {
        return Err("response too large".into());
    }
    let mut buf = Vec::new();
    while let Some(chunk) = r.chunk().await.map_err(|e| e.to_string())? {
        if buf.len() + chunk.len() > MAX_RESPONSE {
            return Err("response too large".into());
        }
        buf.extend_from_slice(&chunk);
    }
    let v: Value = serde_json::from_slice(&buf).map_err(|e| e.to_string())?;
    match v.get("error") {
        Some(e) => Err(e.to_string()),
        None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
    }
}

/// The network's finalized height, from the first source that answers.
async fn net_height(upstream: &Upstream) -> Result<u64, String> {
    upstream.first("aether_status", json!([])).await?["height"]
        .as_u64()
        .ok_or_else(|| "no upstream height".into())
}

/// The margin a snapshot recovery keeps free on top of the snapshot itself
/// (docs/design/24-self-healing.md: 5 GB): the tree it installs goes into the
/// database beside the file that is still there, and compaction after it
/// wants room of its own.
pub const RECOVERY_RESERVE: u64 = 5 << 30;

/// Free bytes on the volume holding `dir` (0 when it cannot be read).
fn free_bytes(dir: &std::path::Path) -> u64 {
    let Ok(c) = std::ffi::CString::new(dir.to_string_lossy().as_bytes()) else { return 0 };
    let mut v: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut v) } != 0 {
        return 0;
    }
    (v.f_bavail as u64).saturating_mul(v.f_frsize as u64)
}

/// Refuse a recovery the disk cannot finish (red team #7): a snapshot
/// recovery on a full disk deletes nothing but writes twice its size — the
/// download in memory and the tree into the database — beside the file that
/// is still there, and redb wants to compact after it. Running out of space
/// mid-recovery is exactly the incident that keeps repeating.
fn require_space(dir: &std::path::Path, size: u64) -> Result<(), String> {
    let free = free_bytes(dir);
    let needed = size.saturating_mul(2).saturating_add(RECOVERY_RESERVE);
    if !enough(free, size) {
        return Err(format!(
            "only {free} bytes are free but a snapshot recovery of {size} bytes needs {needed}: \
             free disk space, and until then this Mac reads through other nodes"
        ));
    }
    Ok(())
}

/// The guard's decision, on the numbers: twice the snapshot plus the fixed
/// reserve, saturating (a size no disk holds is simply refused).
fn enough(free: u64, size: u64) -> bool {
    free >= size.saturating_mul(2).saturating_add(RECOVERY_RESERVE)
}

/// Refuse a peer's wrong-sized response before allocating its decoded bytes.
pub fn decode_snapshot_chunk(value: &Value, expected: usize) -> Result<Vec<u8>, String> {
    let hex = value["data"].as_str().ok_or("no chunk data")?;
    if expected > MAX_RESPONSE / 2 || hex.len() != expected * 2 {
        return Err("snapshot chunk of the wrong size".to_string());
    }
    hex::decode(hex).map_err(|e| e.to_string())
}

/// The upstream's snapshot, downloaded and checked against its BLAKE3
/// (authenticity comes from the certified block after it, in `check`).
/// Chunks are fetched in parallel: each costs a round trip on a slow link.
/// `guard` runs with the advertised size before the first chunk is fetched.
async fn download_with(
    upstream: &Upstream,
    guard: &(dyn Fn(u64) -> Result<(), String> + Send + Sync),
) -> Result<crate::snapshot::Snapshot, String> {
    let v = upstream.first("aether_snapshot", json!([])).await?;
    let (height, size, want) = (
        v["height"].as_u64().ok_or("no snapshot height")?,
        v["size"].as_u64().ok_or("no snapshot size")? as usize,
        v["blake3"].as_str().unwrap_or_default().to_string(),
    );
    let chunk = v["chunk"].as_u64().ok_or("no snapshot chunk size")? as usize;
    if size > MAX_SNAPSHOT || !(MIN_SNAPSHOT_CHUNK..=MAX_RESPONSE / 2).contains(&chunk) {
        return Err(format!(
            "snapshot of {size} bytes in chunks of {chunk} is outside the limits"
        ));
    }
    guard(size as u64)?;
    // The one stage whose work is not blocks: name it, so a frozen height
    // during the download reads as progress, not as a stall (red team #2).
    crate::chain::set_stage(Some("snapshot"));
    let chunk_at = |index: u64| async move {
        let c = upstream
            .first("aether_snapshotChunk", json!([height, index]))
            .await?;
        // Every chunk full-size except the last; never more than advertised.
        let end = ((index + 1) * chunk as u64).min(size as u64);
        let data = decode_snapshot_chunk(&c, (end - index * chunk as u64) as usize)?;
        crate::chain::tick();
        Ok::<Vec<u8>, String>(data)
    };
    let mut bytes = Vec::with_capacity(size.min(64 << 20));
    if size <= chunk {
        bytes.extend(chunk_at(0).await?);
    } else {
        let parts: Vec<Vec<u8>> = futures::stream::iter(0..size.div_ceil(chunk) as u64)
            .map(chunk_at)
            .buffered(SNAPSHOT_PARALLEL)
            .try_collect()
            .await?;
        for p in parts {
            bytes.extend(p);
        }
    }
    // Integrity of the download; authenticity comes from the certified block below.
    if bytes.len() != size || crate::rpc::blake3_hex(&bytes) != want {
        return Err("snapshot download does not match its BLAKE3".into());
    }
    let snap = crate::snapshot::Snapshot::from_bytes(&bytes)?;
    if snap.summary.height != height {
        return Err("snapshot height does not match".into());
    }
    Ok(snap)
}

/// [`download_with`] with the shipped guard: enough room for the recovery,
/// checked against the volume the database lives on (red team #7).
async fn download(upstream: &Upstream, dir: &std::path::Path) -> Result<crate::snapshot::Snapshot, String> {
    download_with(upstream, &|size| require_space(dir, size)).await
}

/// Checkpoint sync: fetch the upstream's snapshot and the certified block
/// after it, check both, and write the snapshot as `store`'s checkpoint.
/// Returns the snapshot height.
pub async fn checkpoint(upstream: &Upstream, set: &ValidatorSet, cfg: &crate::chain::ChainConfig, store: &crate::store::Store) -> Result<u64, String> {
    let dir = store.path().parent().unwrap_or_else(|| std::path::Path::new("."));
    let snap = download(upstream, dir).await?;
    let h = snap.summary.height;
    let next = wait_certified(upstream, set, h + 1).await?;
    let state = snap.check(&next, cfg, set.identity())?;
    snap.install(store, &state, cfg)?;
    info!(
        height = h,
        entries = snap.entries.len(),
        "checkpoint: started from a certified snapshot (history not replayed)"
    );
    Ok(h)
}

/// Bad data, as opposed to a bad disk (a disk error fails the start-up and is
/// retried on the same file; bad data never gets better, so the file is moved
/// aside and the history is re-fetched). `Chain::open`'s full check reports
/// these too — [`reset_store`] handles both tiers.
///
/// A schema newer than this binary reads ([`StoreError::TooNew`]) is
/// deliberately not corruption: the file holds intact newer data, so it is
/// never moved aside — the node stops with the update-required exit code and
/// the app installs the newer release ([`is_too_new_error`]).
pub fn is_corruption(e: &crate::store::StoreError) -> bool {
    matches!(
        e,
        crate::store::StoreError::Corrupt(_)
            | crate::store::StoreError::RootMismatch { .. }
            | crate::store::StoreError::Unreadable(_)
    )
}

/// Whether an `open_store` error means the database's schema is newer than
/// this binary reads (`StoreError::TooNew`, red team #15): the child stops
/// with [`crate::supervisor::EXIT_UPGRADE_REQUIRED`] — the app's existing
/// "update required" handling — and the data is never moved aside or deleted.
/// String form, because `open_store` reports strings; `StoreError`'s `Display`
/// is its `Debug`, so the variant name is in it.
pub fn is_too_new_error(e: &str) -> bool {
    e.contains("TooNew")
}

/// Move the state database (and, for a validator, its marshal archive
/// partitions, which would otherwise have a gap to a fresh state) aside under
/// `<data>/corrupt-<time>/`, never deleted, and open a fresh one. Everything
/// else in the data dir — keys and the consensus vote journal above all —
/// stays where it is.
fn move_aside(data: &std::path::Path, why: &str) -> Result<(), String> {
    let prefix = std::fs::read_to_string(data.join("partition"))
        .map(|p| p.trim().to_string())
        .unwrap_or_else(|_| "aether".into());
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let aside = data.join(format!("corrupt-{secs}"));
    std::fs::create_dir_all(&aside).map_err(|e| e.to_string())?;
    let gone: Vec<std::path::PathBuf> = std::fs::read_dir(data)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                // Never the consensus journal (`{prefix}-consensus*`): a
                // validator that lost its record of past votes could sign a
                // second, conflicting vote for a view it already voted in
                // (red team 2026-09-29, self-healing #4). Only the state and
                // the block archive, which re-sync from certified blocks.
                .is_some_and(|n| {
                    n == "state.redb"
                        || (n.starts_with(&format!("{prefix}-")) && !n.starts_with(&format!("{prefix}-consensus")))
                })
        })
        .collect();
    // The state moves first: a crash mid-move then leaves a fresh state under
    // an archive that is only older (which replays block by block), never a
    // state checkpoint ahead of a half-gone archive.
    let mut gone = gone;
    gone.sort_by_key(|p| p.file_name().and_then(|n| n.to_str()).map(|n| n != "state.redb").unwrap_or(true));
    for p in gone {
        let name = p.file_name().expect("listed entry has a name").to_owned();
        std::fs::rename(&p, aside.join(name)).map_err(|e| e.to_string())?;
    }
    warn!(moved_to = %aside.display(), why, "the state database does not verify: moved it aside (never deleted); the chain re-syncs, certified block by certified block");
    Ok(())
}

/// Open the node's state database, verifying what it claims (the cheap tier-1
/// check; `Chain::open` does the full one). A database that does not verify is
/// moved aside ([`move_aside`]) and a fresh one opens in its place — the caller
/// re-syncs through the usual paths (`--checkpoint`, or the snapshot jump once
/// the network is more than [`JUMP_BEHIND`] ahead). A disk error fails the
/// start-up instead: a restart then retries the same file once space frees.
/// Returns the store and whether the old file was moved aside.
pub fn open_store(data: &std::path::Path) -> Result<(crate::store::Store, bool), String> {
    open_store_with(data, std::sync::Arc::new(crate::store::Store::open))
}

/// [`open_store`] with a custom way to open the database file: the hidden
/// `--dev-storage-fault` of the self-healing tests runs the real process on a
/// disk that fills. No release path passes one.
pub fn open_store_with(
    data: &std::path::Path,
    open: std::sync::Arc<dyn Fn(&std::path::Path) -> Result<crate::store::Store, crate::store::StoreError> + Send + Sync>,
) -> Result<(crate::store::Store, bool), String> {
    let path = data.join("state.redb");
    let reset = |why: &str| -> Result<crate::store::Store, String> {
        move_aside(data, why)?;
        open(&path).map_err(|e| e.to_string())
    };
    match open(&path) {
        Ok(s) => match s.verify_head() {
            Ok(()) => Ok((s, false)),
            Err(e) if is_corruption(&e) => Ok((reset(&e.to_string())?, true)),
            Err(e) => return Err(e.to_string()),
        },
        Err(e) if is_corruption(&e) => Ok((reset(&e.to_string())?, true)),
        Err(e) => return Err(e.to_string()),
    }
}

/// The same move-aside when `Chain::open` refuses a database that passed tier 1
/// (its state rebuild does not match the checkpoint, a row does not decode…):
/// a fresh store the caller fills from a certified snapshot or a replay.
pub fn reset_store(data: &std::path::Path, e: &crate::store::StoreError) -> Result<crate::store::Store, String> {
    move_aside(data, &e.to_string())?;
    crate::store::Store::open(&data.join("state.redb")).map_err(|e| e.to_string())
}

/// Certified block `h`, waiting for the network to finalize it (a snapshot can
/// be a little behind the tip).
async fn wait_certified(upstream: &Upstream, set: &ValidatorSet, h: u64) -> Result<Block, String> {
    for _ in 0..60 {
        if let Some((block, _)) = fetch(upstream, set, h).await? {
            return Ok(block);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!("no certified block {h} after the snapshot"))
}

/// A node that slept for hours jumps instead of replaying: fetch the network's
/// snapshot, check it against the certified block after it (as a checkpoint
/// start does), then swap it in over this chain's state and adopt it as the
/// finalized head. The blocks skipped are not kept; they stay fetchable from
/// era files, verified by the certified history root. Returns the height
/// jumped to.
async fn jump(chain: &Chain, upstream: &Upstream, set: &ValidatorSet) -> Result<u64, String> {
    let store = chain.store().ok_or("no store to jump in")?;
    let snap = download(upstream, store.path().parent().unwrap_or_else(|| std::path::Path::new("."))).await?;
    let h = snap.summary.height;
    let ours = chain.finalized_height();
    if h <= ours + JUMP_BEHIND {
        return Err(format!(
            "snapshot at {h} is only {} blocks ahead of {ours}; replaying instead",
            h.saturating_sub(ours)
        ));
    }
    let next = wait_certified(upstream, set, h + 1).await?;
    let state = snap.check(&next, &chain.cfg(), set.identity())?;
    // The old state's keys go with the swap, so the store ends up holding exactly the snapshot.
    let old: Vec<([u8; 32], [u8; 32])> = chain.lock().finalized.state.repo().entries().collect();
    snap.install_over(&store, &state, old)?;
    let (exec, summary) = snap.head(state);
    chain.adopt(exec, summary);
    chain.lock().upgrade_notices = snap.upgrade_notices;
    Ok(h)
}

/// How long the follow loop waits after an error before trying again (red
/// team #16): a full disk is a condition only a person changes, so retries
/// back off for half a minute instead of hammering a disk that is full; a
/// network blip is still quick.
fn error_backoff(err: &str) -> Duration {
    if err.contains("No space left") || err.contains("free disk space") {
        Duration::from_secs(30)
    } else {
        Duration::from_millis(400)
    }
}

/// Follow the chain forever: verify, execute and persist each next block.
/// `joining`: this Mac's voting key when it is a candidate. When a finalized
/// handoff seats it, the follower stops before the switch height (for up to
/// `HOLD`), so `aether run` can start it as a voting node from that block.
pub async fn run(
    chain: Chain,
    upstream: std::sync::Arc<Upstream>,
    set: ValidatorSet,
    archive: std::sync::Arc<FinalityArchive>,
    joining: Option<String>,
) {
    const HOLD: Duration = Duration::from_secs(120);
    let mut last_log = 0;
    let mut held_since: Option<std::time::Instant> = None;
    // One height per round while idle at the tip (as before); a full batch
    // while there is a backlog to fetch.
    let mut window = 1u64;
    loop {
        // A handoff that seats this Mac: never run past its switch height
        // (`aether run` adopts this follower's state there), and hold once it
        // is reached.
        let cap = match hold_cap(&chain, joining.as_deref()) {
            Hold::Free => u64::MAX,
            Hold::Approach(switch) => switch - 1,
            Hold::Seated => {
                held_since.get_or_insert_with(std::time::Instant::now);
                if held_since.unwrap().elapsed() < HOLD {
                    window = 1;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                // The supervisor never came: follow the chain again.
                u64::MAX
            }
        };
        match advance(
            &chain,
            &upstream,
            &set,
            Some(&archive),
            cap,
            window,
            &mut last_log,
        )
        .await
        {
            Ok(0) => {
                window = 1;
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
            Ok(_) => window = PIPELINE,
            Err(e) => {
                chain.set_relaxed(false);
                if e.starts_with(STORE_FAILED) {
                    recover(&chain).await;
                } else {
                    warn!(height = chain.finalized_height() + 1, backoff_ms = error_backoff(&e).as_millis() as u64, %e, "upstream");
                }
                window = 1;
                tokio::time::sleep(error_backoff(&e)).await;
            }
        }
    }
}

/// What a pending handoff that seats this Mac means for the follower.
enum Hold {
    /// No handoff seats it: follow freely.
    Free,
    /// One does, before its switch height: `u64` is the switch.
    Approach(u64),
    /// It is at the switch height: hold for `aether run` to adopt the state.
    Seated,
}

fn hold_cap(chain: &Chain, joining: Option<&str>) -> Hold {
    let g = chain.lock();
    let Some(p) = g
        .finalized
        .handoff
        .as_ref()
        .filter(|p| joining.is_some_and(|k| p.handoff.members.iter().any(|(m, _)| m == k)))
    else {
        return Hold::Free;
    };
    if g.finalized.height + 1 >= p.switch {
        Hold::Seated
    } else {
        Hold::Approach(p.switch)
    }
}

/// One round of following: learn the network's finalized height (kept for
/// `aether_status`), jump to a certified snapshot when more than `JUMP_BEHIND`
/// behind, then fetch, verify and finalize blocks with `window` requests in
/// flight. A batch that adopts everything continues straight into the next
/// (each costs one height round trip and one batch; the last batch of a
/// backlog stops where the network's tip answers nothing). Returns how many
/// blocks were adopted (0: at the tip, or an error is being retried).
async fn advance(
    chain: &Chain,
    upstream: &Upstream,
    set: &ValidatorSet,
    archive: Option<&FinalityArchive>,
    cap: u64,
    window: u64,
    last_log: &mut u64,
) -> Result<u64, String> {
    // Each round starts unnamed (red team #2): a stage that still holds names
    // itself again below; one that has finished does not linger in aether_status.
    crate::chain::set_stage(None);
    let mut adopted = 0u64;
    let mut window = window.max(1);
    loop {
        let net = net_height(upstream).await?;
        chain.lock().net_height = Some(net);
        let ours = chain.finalized_height();
        if net > ours + JUMP_BEHIND && adopted == 0 {
            match jump(chain, upstream, set).await {
                Ok(to) => {
                    info!(from = ours, to, skipped = to - ours - 1, "jumped to a certified snapshot (the gap's blocks stay fetchable from era files)");
                    log_follow(chain, last_log);
                    chain.set_relaxed(false);
                    return Ok(to - ours);
                }
                Err(e) => {
                    warn!(from = ours, %e, "could not jump to a certified snapshot; replaying instead")
                }
            }
        }
        let last = pipeline(
            chain,
            upstream,
            set,
            archive,
            ours + 1,
            net.min(cap),
            window,
        )
        .await?;
        adopted += last - ours;
        if last > ours {
            log_follow(chain, last_log);
        }
        // A short batch means the tip (or a source that stopped answering);
        // the caller decides when to try again. A full one means a backlog:
        // keep going without asking for the height again.
        if window == 1 || last < ours + window {
            chain.set_relaxed(false);
            return Ok(adopted);
        }
        window = PIPELINE;
    }
}

/// Fetch, verify and finalize blocks `from..=cap` (at most `window` of them)
/// with `window` requests in flight: certificates are checked as each answer
/// arrives, blocks are executed in order, and nothing past a block that does
/// not execute is adopted. Stops at the first height no source has yet, or
/// replays its era file when the source pruned it. Returns the last height
/// adopted (`from - 1` when the batch made no progress).
async fn pipeline(
    chain: &Chain,
    upstream: &Upstream,
    set: &ValidatorSet,
    archive: Option<&FinalityArchive>,
    from: u64,
    cap: u64,
    window: u64,
) -> Result<u64, String> {
    let to = cap.min(from + window - 1);
    if to < from {
        return Ok(from - 1);
    }
    let heights: Vec<u64> = (from..=to).collect();
    let fetched = futures::future::join_all(heights.iter().map(|h| fetch(upstream, set, *h))).await;
    let mut last = from - 1;
    for (h, r) in heights.into_iter().zip(fetched) {
        match r {
            Ok(Some((block, proof))) => {
                // A backlog replays without the per-block fsync (redb holds
                // those commits until a durable one); the batch's last block
                // commits durably and anchors it, so a crash mid-replay loses
                // only the open batch — certified blocks, they replay again.
                chain.set_relaxed(h != to);
                match chain.finalize(&block) {
                    Ok(()) => {
                        if let Some(a) = archive {
                            a.insert(h, proof);
                        }
                        last = h;
                    }
                    Err(e) => {
                        // Storage is not a block that will not execute: say
                        // what failed and let the caller heal the store
                        // (2026-09-29: a full disk was logged as a bad block
                        // and retried 288 times).
                        if matches!(e, crate::chain::ChainError::Store(_)) {
                            tracing::error!(height = h, ?e, "storage failed while committing a finalized block");
                            return Err(format!("{STORE_FAILED}: {e:?}"));
                        }
                        warn!(
                            height = h,
                            ?e,
                            "certified block did not execute to the same result; not adopting it"
                        );
                        return Ok(last);
                    }
                }
            }
            Ok(None) => break,
            // The source pruned this era (roadmap B4): replay it from an era file instead.
            Err(e) if e.contains("pruned") => match catch_up_era(chain, upstream, set, h).await {
                Ok(to) => {
                    info!(from = h, to, "replayed a pruned era from its era file");
                    return Ok(to.max(last));
                }
                Err(e) => {
                    warn!(height = h, %e, "upstream pruned this era and it could not be fetched");
                    return Ok(last);
                }
            },
            Err(e) => {
                warn!(height = h, %e, "upstream");
                break;
            }
        }
    }
    Ok(last)
}

/// Heal the chain's store after a storage failure (docs/design/24-self-healing.md
/// layer 1): close the database, re-open it with backoff until the disk takes
/// a write again, then roll the chain back to the last durable checkpoint —
/// blocks of a replayed backlog that were never fsynced may be gone, and
/// everything above the checkpoint is certified, so the loop fetches and
/// re-executes it. A disk that never heals ends the process with the storage
/// code: the app restarts the node, whose startup check re-syncs (a certified
/// snapshot jump) if the file turns out to be damaged.
async fn recover(chain: &Chain) {
    let Some(store) = chain.store() else { return };
    // A store re-opening after a disk failure is a stage all its own: the
    // height freezes for its whole backoff, and that is the healing working
    // (red team #2/#16).
    crate::chain::set_stage(Some("storage"));
    let healed = tokio::task::spawn_blocking({
        let store = store.clone();
        move || crate::store::Recovery::from_env().reopen(&store)
    })
    .await;
    match healed {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            tracing::error!(%e, "the store database did not recover; exiting so the app restarts the node");
            std::process::exit(crate::store::EXIT_STORAGE);
        }
        Err(e) => tracing::error!(%e, "the store recovery task"),
    }
    chain.set_relaxed(false);
    match store.load() {
        Ok(Some(cp)) => rollback(chain, cp),
        // A store that holds nothing, or does not verify, is corruption:
        // exiting hands it to the startup integrity check, which moves the
        // file aside and re-syncs from a certified snapshot.
        other => {
            tracing::error!(other = ?other.map(|_| ()), "the re-opened store does not verify; exiting so the app restarts the node");
            std::process::exit(crate::store::EXIT_STORAGE);
        }
    }
}

/// Roll the chain back to the store's checkpoint: the same restore
/// `Chain::open` does at a restart, without restarting. Summaries and
/// receipts above the checkpoint stay until the chain passes their heights
/// again (they are certified history of this same chain); history proofs
/// need the early blocks, so none is served until then, as after a jump.
fn rollback(chain: &Chain, cp: crate::store::Checkpoint) {
    use commonware_codec::DecodeExt;
    let Ok(digest) = commonware_cryptography::sha256::Digest::decode(cp.digest.as_slice()) else {
        tracing::error!("the checkpoint's digest does not decode");
        return;
    };
    let Some(summary) = cp.blocks.get(&cp.height).cloned() else {
        tracing::error!(height = cp.height, "the checkpoint's block summary is gone");
        return;
    };
    let mut state = cp.state;
    state.clear_journal();
    let exec = std::sync::Arc::new(crate::chain::Executed {
        height: cp.height,
        digest,
        timestamp: summary.timestamp_ms,
        state,
        receipts: vec![],
        tx_hashes: summary.txs.clone(),
        gas: Default::default(),
        proposer: summary.proposer,
        base_fee: summary.base_fee,
        excess: summary.excess,
        handoff: cp.handoff.map(std::sync::Arc::new),
        seed: cp.seed.map(std::sync::Arc::new),
        history: std::sync::Arc::new(cp.history),
        schedule: std::sync::Arc::new(cp.schedule),
        statement: cp.statement,
        payouts: vec![],
        registration_ids: vec![],
    });
    let height = exec.height;
    chain.adopt(exec, summary);
    chain.lock().upgrade_notices = cp.upgrade_notices;
    tracing::warn!(height, "rolled the chain back to the last durable checkpoint; re-fetching what came after it");
}

fn log_follow(chain: &Chain, last_log: &mut u64) {
    let (height, root) = {
        let g = chain.lock();
        (g.finalized.height, g.finalized.state.root())
    };
    if height - *last_log >= 100 || height.is_multiple_of(10) {
        info!(height, %root, "followed");
        *last_log = height;
    }
}

/// Catch a validator up before it starts voting (`run_node` calls this before
/// the consensus engine exists, so a committee member that slept cannot
/// propose, vote or answer beacons on a chain it cannot yet execute): follow
/// the network with the follower machinery — a certified snapshot jump
/// included — until within `margin` blocks of its finalized height. Returns
/// how many blocks were adopted (jumped or replayed). A catch-up that never
/// heard the network's height is not success (2026-09-29): an unknown height
/// reads as "0 behind" and would start the validator on a guess, so this
/// returns Err and the caller retries. On success the last height heard
/// stays set — it is at most `margin` stale — so `behind()` keeps meaning
/// something until this node's own finalizations carry it past the tip.
pub async fn catch_up(
    chain: &Chain,
    upstream: &Upstream,
    set: &ValidatorSet,
    margin: u64,
) -> Result<u64, String> {
    let start = chain.finalized_height();
    let mut last_log = 0;
    let mut window = PIPELINE;
    let mut knew_height = false;
    loop {
        let ours = chain.finalized_height();
        if let Err(e) = advance(chain, upstream, set, None, u64::MAX, window, &mut last_log).await {
            if e.starts_with(STORE_FAILED) {
                recover(chain).await;
            } else {
                warn!(height = ours + 1, %e, "catching up");
            }
        }
        // `advance` refreshes the height every round and nothing here clears
        // it anymore, so `Some` means the network really answered.
        knew_height |= chain.lock().net_height.is_some();
        let behind = chain.behind();
        if behind <= margin {
            if !knew_height {
                // Nothing ever answered a height: "0 behind" is the unknown
                // reading as zero, not being current. Not caught up.
                return Err("never learned the network height".into());
            }
            chain.set_relaxed(false);
            return Ok(chain.finalized_height() - start);
        }
        window = if chain.finalized_height() > ours {
            PIPELINE
        } else {
            1
        };
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

/// How long a starting validator waits for any roster peer to answer at all
/// before it starts voting anyway (the fail-open the old code had, now only
/// after a real wait). Far past a whole network restarting at once — every
/// member answers from its stored finalized state within seconds of its
/// endpoint binding — and short enough that a partitioned validator comes
/// back on its own (docs/design/24-self-healing.md: 모든 검증자 동시 재시작).
pub const STARTUP_PATIENCE: Duration = Duration::from_secs(5 * 60);

/// One round of catching up from one peer before the roster is asked again
/// (a source that answers a height but no blocks must not hold the gate forever).
const CATCH_UP_ROUND: Duration = Duration::from_secs(120);
/// How long one round waits before the roster is asked again.
const ASK_AGAIN: Duration = Duration::from_secs(5);

/// Whether a member that has not voted yet may start.
#[derive(Debug, PartialEq, Eq)]
pub enum VoteStart {
    /// Every peer that answered puts nobody beyond `margin` ahead: at the tip.
    /// The engine covers the last few blocks through consensus itself.
    AtTip,
    /// No peer answered at all for the whole patience window. Starting anyway
    /// is the old fail-open, waited out: the chain's own rules (no vote on a
    /// block this node cannot execute) keep a stale member harmless, and peers
    /// that never come back are not fixed by waiting longer.
    FailOpen,
    /// A reachable peer is ahead: not yet.
    Wait,
}

/// The startup gate's decision from one roster census: the finalized heights
/// the peers that answered reported, where we are, how long since any of them
/// last answered, and how much silence is tolerated. A height is "heard" only
/// from a peer that answered — an unknown height never reads as zero.
pub fn may_start_voting(answered: &[u64], ours: u64, margin: u64, silent_for: Duration, patience: Duration) -> VoteStart {
    if answered.is_empty() {
        return if silent_for >= patience { VoteStart::FailOpen } else { VoteStart::Wait };
    }
    if answered.iter().all(|h| *h <= ours + margin) { VoteStart::AtTip } else { VoteStart::Wait }
}

/// Ask every roster peer where it is — a census, not the first answer: one
/// `aether_status` per peer, in parallel, each bounded. Peers that do not
/// answer are absent (a restarting network comes up one by one; a peer still
/// catching up serves read-only answers from its stored finalized state).
pub async fn roster_heights(
    endpoint: &aether_net::Endpoint,
    nodes: &[aether_net::EndpointId],
) -> Vec<(aether_net::EndpointId, u64)> {
    let asked = futures::future::join_all(nodes.iter().map(|n| async move {
        let addr = aether_net::EndpointAddr::from(*n);
        match aether_net::connect_rpc(endpoint, &addr, Duration::from_secs(10)).await {
            Ok(conn) => match aether_net::rpc_call(&conn, "aether_status", json!([])).await {
                Ok(v) => v["height"].as_u64().map(|h| (*n, h)),
                Err(e) => {
                    tracing::debug!(peer = %n, %e, "answered no height");
                    None
                }
            },
            Err(e) => {
                tracing::debug!(peer = %n, %e, "unreachable");
                None
            }
        }
    }))
    .await;
    asked.into_iter().flatten().collect()
}

/// Catch a restarting committee member up before it starts voting, and return
/// when it may: the gate every validator passes on startup. Each round asks
/// the whole roster where it is (`ask` returns every peer that answered, with
/// its finalized height — a census, not the first answer), records the highest
/// answer as the network's height, and decides:
///
/// - nobody beyond `margin` ahead → done, voting may start;
/// - a peer ahead → catch up from the tallest one (`from` builds an upstream
///   pointed at it) and ask again;
/// - no answer at all for `patience` → done, with a warning.
///
/// Returns how many blocks were adopted. The caller must already serve
/// read-only answers on its public endpoint (`main.rs` serves from its stored
/// finalized state before this runs): a network where every validator
/// restarts at once has nobody voting, so nobody would ever hear a height —
/// the censuses break that, and the tallest member, finding nobody ahead,
/// always proceeds first and then serves the rest its blocks.
pub async fn catch_up_before_voting<P, A, AFut>(
    chain: &Chain,
    set: &ValidatorSet,
    margin: u64,
    patience: Duration,
    ask: A,
    from: impl Fn(&P) -> Upstream,
) -> u64
where
    P: Clone + std::fmt::Display,
    A: Fn() -> AFut,
    AFut: Future<Output = Vec<(P, u64)>>,
{
    let start = chain.finalized_height();
    let mut heard = std::time::Instant::now();
    loop {
        let answers = ask().await;
        if answers.is_empty() {
            warn!(silent_for = ?heard.elapsed(), ?patience, "no roster peer answers yet");
        } else {
            heard = std::time::Instant::now();
            // The highest answer is the network's height: beacon answers and
            // `aether_status` gate on it, and the engine covers the rest.
            let net = answers.iter().map(|(_, h)| *h).max().expect("a peer answered");
            chain.lock().net_height = Some(net);
        }
        let heights: Vec<u64> = answers.iter().map(|(_, h)| *h).collect();
        match may_start_voting(&heights, chain.finalized_height(), margin, heard.elapsed(), patience) {
            VoteStart::AtTip => break,
            VoteStart::FailOpen => {
                warn!(
                    silent_for = ?heard.elapsed(),
                    "no roster peer ever answered: starting to vote anyway (the old fail-open, after a real wait)"
                );
                break;
            }
            VoteStart::Wait => {}
        }
        if answers.is_empty() {
            // Nothing answered, so there is nothing to catch up from: ask
            // again after a while, until the patience window runs out.
            tokio::time::sleep(ASK_AGAIN.min(patience)).await;
            continue;
        }
        // Somebody is ahead: catch up from the tallest peer that answered.
        let (peer, at) = answers.iter().max_by_key(|(_, h)| *h).expect("a peer answered").clone();
        info!(peer = %peer, peer_height = at, ours = chain.finalized_height(), "a peer is ahead; catching up before voting");
        let upstream = from(&peer);
        match tokio::time::timeout(CATCH_UP_ROUND, catch_up(chain, &upstream, set, margin)).await {
            Ok(Ok(n)) if n > 0 => info!(height = chain.finalized_height(), blocks = n, "caught up before voting"),
            Ok(Ok(_)) => {}
            Ok(Err(e)) => warn!(%e, "could not catch up before voting; asking the roster again"),
            Err(_) => warn!("catch-up before voting timed out; asking the roster again"),
        }
        // A round dropped mid-replay is dropped without clearing its replay
        // mode: blocks from here on (voting) commit durably.
        chain.set_relaxed(false);
        tokio::time::sleep(ASK_AGAIN.min(patience)).await;
    }
    chain.finalized_height() - start
}

/// Replay the rest of the era holding `next` from its era file: the file is
/// checked against the history root of a later block whose certificate this
/// follower verified, then every block is re-executed as usual. Returns the
/// last height adopted.
pub async fn catch_up_era(chain: &Chain, upstream: &Upstream, set: &ValidatorSet, next: u64) -> Result<u64, String> {
    use aether_state::mmr::ERA_LEN;
    let era = next / ERA_LEN;
    let tip = upstream.first("aether_status", json!([])).await?["height"].as_u64().ok_or("no upstream height")?;
    if tip < (era + 1) * ERA_LEN {
        return Err(format!("upstream has not finished era {era}"));
    }
    // A certified block after the era: its history root commits every block before it.
    let mut anchor = None;
    for h in (tip.saturating_sub(8)..=tip).rev() {
        if let Some((block, _)) = fetch(upstream, set, h).await? {
            let root = block.payload().ok_or("anchor payload")?.history_root;
            anchor = Some(crate::era_net::HistoryAnchor { leaves: h, root });
            break;
        }
    }
    let anchor = anchor.ok_or("no certified block to anchor the era on")?;
    let known = chain.lock().history_index.as_ref().and_then(|i| i.eras.get(era as usize).copied());
    let (_, e) = crate::era_net::fetch(upstream, era, &anchor, known.as_ref()).await?;
    let c = chain.clone();
    tokio::task::spawn_blocking(move || {
        let mut last = next - 1;
        for b in e.blocks.iter().filter(|b| b.height.get() >= next) {
            c.finalize(b)
                .map_err(|e| format!("era block {} did not execute: {e:?}", b.height.get()))?;
            last = b.height.get();
        }
        Ok(last)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Block `h` and its certificate, verified; `None` if no source has it yet.
/// A source that answers with nothing or with a bad certificate is skipped.
pub async fn fetch(upstream: &Upstream, set: &ValidatorSet, h: u64) -> Result<Option<(Block, Value)>, String> {
    upstream.ask("aether_getFinalized", json!([h]), |v| check(set, h, v)).await
}

fn check(set: &ValidatorSet, h: u64, v: Value) -> Result<Option<(Block, Value)>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let hex = |s: &Value| from_hex(s.as_str().unwrap_or_default()).map_err(|e| format!("{e:?}"));
    let (bb, fb) = (hex(&v["block"])?, hex(&v["finalization"])?);
    let links = v["links"].as_array().map(|a| a.iter().map(hex).collect::<Result<Vec<_>, _>>()).transpose()?.unwrap_or_default();
    let verified = verify_finalized_chain(set, &bb, &fb, &links).map_err(|e| format!("certificate: {e:?}"))?;
    if verified.height != h {
        return Err(format!("asked for block {h}, got a certificate for {}", verified.height));
    }
    let block = Block::decode_cfg(bb.as_slice(), &Block::codec_config(MAX_BLOCK_BYTES)).map_err(|e| format!("block: {e}"))?;
    // Keep the canonical encoding, never the upstream's text (which may be padded).
    let links: Vec<String> = links.iter().map(|l| aether_light::to_hex(l)).collect();
    Ok(Some((block, json!({ "height": h, "block": aether_light::to_hex(&bb), "finalization": aether_light::to_hex(&fb), "links": links }))))
}

/// Forward txs submitted to this follower to the validators, retrying for a
/// while so a brief outage does not lose them.
pub async fn forward(upstream: std::sync::Arc<Upstream>, mut rx: tokio::sync::mpsc::UnboundedReceiver<aether_types::TxEnvelope>) {
    while let Some(tx) = rx.recv().await {
        let up = upstream.clone();
        tokio::spawn(async move {
            let mut wait = Duration::from_secs(1);
            for attempt in 1..=6 {
                match up.call("aether_sendTransaction", json!([tx])).await {
                    Ok(_) => return,
                    Err(e) if attempt == 6 => warn!(%e, "gave up forwarding a transaction upstream"),
                    Err(_) => {}
                }
                tokio::time::sleep(wait).await;
                wait *= 2;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_chunk_rejects_wrong_length_before_decoding() {
        assert_eq!(decode_snapshot_chunk(&json!({"data": "00ff"}), 2).unwrap(), [0, 255]);
        assert!(decode_snapshot_chunk(&json!({"data": "00ff"}), 1).is_err());
        assert!(decode_snapshot_chunk(&json!({"data": "gg"}), 1).is_err());
        assert!(decode_snapshot_chunk(&json!({}), 1).is_err());
        assert!(decode_snapshot_chunk(&json!({"data": ""}), MAX_RESPONSE).is_err());
    }

    /// The startup gate's decision: a height only counts when a peer answered
    /// it, every answered peer must put nobody beyond the margin ahead, and
    /// silence fails open only once the patience window has really passed.
    #[test]
    fn when_a_member_may_start_voting() {
        let patience = Duration::from_secs(300);
        // Peers at or behind us: at the tip, including a peer that lags (the
        // margin is what "caught up" means; the engine covers the rest).
        assert_eq!(may_start_voting(&[100, 100, 98], 100, BEHIND_MARGIN, Duration::ZERO, patience), VoteStart::AtTip);
        assert_eq!(may_start_voting(&[120], 100, BEHIND_MARGIN, Duration::ZERO, patience), VoteStart::AtTip);
        // A peer beyond the margin: wait — even after any silence elsewhere,
        // for as long as that peer keeps answering.
        assert_eq!(may_start_voting(&[121], 100, BEHIND_MARGIN, Duration::ZERO, patience), VoteStart::Wait);
        assert_eq!(may_start_voting(&[98, 500], 100, BEHIND_MARGIN, patience, patience), VoteStart::Wait);
        // Nobody answered: not "0 behind" — wait, and only after the patience
        // window fail open.
        assert_eq!(may_start_voting(&[], 100, BEHIND_MARGIN, patience - Duration::from_millis(1), patience), VoteStart::Wait);
        assert_eq!(may_start_voting(&[], 100, BEHIND_MARGIN, patience, patience), VoteStart::FailOpen);
    }

    /// Red team #7/#16: a recovery the disk cannot hold never starts, and a
    /// full disk is not retried at network speed.
    #[test]
    fn space_is_checked_before_a_recovery_and_full_disks_back_off() {
        let (size, free) = (1u64 << 30, RECOVERY_RESERVE + 2 * (1u64 << 30));
        assert!(enough(free, size), "twice the snapshot plus the reserve is exactly enough");
        assert!(!enough(free - 1, size), "one byte short is not");
        assert!(!enough(RECOVERY_RESERVE, size), "the reserve alone does not fit the snapshot twice over");
        assert!(!enough(u64::MAX - 1, u64::MAX), "a size no disk holds is refused (the need saturates past every disk)");
        // The shipped guard reads a real volume and says the one sentence.
        let dir = std::env::temp_dir();
        assert!(free_bytes(&dir) > 0, "a readable volume reports its free bytes");
        let err = require_space(&dir, u64::MAX).unwrap_err();
        assert!(err.contains("free disk space"), "{err}");
        assert_eq!(error_backoff(&err), Duration::from_secs(30), "the space refusal backs off");
        assert_eq!(error_backoff("io: No space left on device"), Duration::from_secs(30));
        assert_eq!(error_backoff("connection refused"), Duration::from_millis(400));
    }
}
