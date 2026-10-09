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
//! Catch-up: blocks are fetched in pipelined batches spread over every
//! upstream (`spread`: a few requests in flight per validator, "server busy"
//! waited out, block ranges where the validator serves them), executed in
//! order. A Mac more than `JUMP_BEHIND` blocks behind jumps FIRST to the
//! network's certified snapshot (checked against the certified block after
//! it, as a checkpoint start is; the download is pinned to the validator that
//! served the manifest and restarts when that snapshot moves on), and the
//! gap's certified blocks are backfilled afterwards, newest first, in the
//! background (`backfill`). Archive nodes never jump (audit 7 A7-1).
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
/// replaying (~5 min of 1 s blocks). The founder's rule (2026-10-07): a node
/// that comes back far behind jumps first and backfills the gap after, so it
/// is usable at the tip in minutes, not after hours of replay.
pub const JUMP_BEHIND: u64 = 300;
/// How many times one validator's snapshot download restarts on a fresh
/// manifest after "snapshot moved on" before the next validator is tried.
const SNAPSHOT_RESTARTS: usize = 3;
/// How long the follower replays before trying a failed jump again (each try
/// downloads a snapshot; one that fails every round only costs the replay).
const JUMP_RETRY: Duration = Duration::from_secs(30);
/// How long a validator whose snapshot is being built is waited for.
const MANIFEST_WAITS: u32 = 30;
/// A follower that cannot make verified progress for this long gives its
/// supervisor a chance to rebuild the transport. This also covers a peer
/// connection that never returns a height after a restart.
const STALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Restartable follower failure (unlike storage, identity or protocol exits).
pub const EXIT_STALLED: i32 = 11;
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

    pub(crate) fn store(&self) -> Option<&crate::store::Store> {
        self.store.as_deref()
    }

    pub fn get(&self, height: u64) -> Option<Value> {
        match &self.store {
            Some(s) => s.proof(height).ok().flatten().and_then(|b| serde_json::from_slice(&b).ok()),
            None => self.inner.lock().expect("archive lock").get(&height).cloned(),
        }
    }
}

/// Largest snapshot a new Mac downloads. This is a wire bound, not a memory
/// budget: the inbound memory guard below turns it into one. Replay remains
/// the fallback.
const MAX_SNAPSHOT: usize = 1 << 30;
/// Smallest chunk accepted (bounds the number of requests).
const MIN_SNAPSHOT_CHUNK: usize = 64 << 10;

/// How many wire bytes one downloaded byte can become in memory, at the worst
/// moment: the file read back for decoding (`Snapshot::from_bytes` needs the
/// whole slice) overlapping the decoded entries, then `Snapshot::check`
/// cloning those entries to rebuild the state. Measured, not guessed, by
/// `follower_decode_allocation_probe` (see the test module): the follower's
/// real path on a 300,000-entry synthetic snapshot measured wire=19,200,348
/// bytes and a sampled peak delta of 94,749,056 bytes — ~4.9× the wire —
/// while the server-side 100,000-entry probe's estimate puts the extra at
/// ~6.6× its wire size and its sampler caught only a quarter of that (1 ms
/// sampling misses transients), so 8 rounds the measurement up with headroom
/// for those blind spots. The budget it feeds is
/// `resources::snapshot_memory_budget` — the same headroom and pressure
/// policy the serving side already applies, not a second one.
pub const SNAPSHOT_DECODE_AMPLIFICATION: u64 = 8;

/// How long a memory-budget refusal stands before the same advertised size is
/// measured again (memory frees up when other apps quit; a peer that serves a
/// smaller snapshot is never blocked by it).
const MEMORY_REFUSAL_COOLDOWN: Duration = Duration::from_secs(300);

/// The last snapshot size refused by the memory guard, and when. Kept so the
/// follow loop cannot spin asking the same peer for the same oversized
/// manifest every round — the refusal is retried at cooldown pace, and the
/// replay fallback makes progress in between.
static MEMORY_REFUSED: Mutex<Option<(u64, std::time::Instant)>> = Mutex::new(None);

fn note_refusal(size: u64) {
    *MEMORY_REFUSED.lock().expect("snapshot refusal state") = Some((size, std::time::Instant::now()));
}

fn last_refusal() -> Option<(u64, std::time::Instant)> {
    *MEMORY_REFUSED.lock().expect("snapshot refusal state")
}

/// Whether a size refused `at` still blocks a manifest of `size` bytes now.
/// Same-or-larger sizes are blocked inside the cooldown; smaller ones and
/// stale ones are measured afresh.
fn refusal_blocks(last: Option<(u64, std::time::Instant)>, size: u64, now: std::time::Instant) -> bool {
    last.is_some_and(|(refused, at)| size >= refused && now.duration_since(at) < MEMORY_REFUSAL_COOLDOWN)
}

/// Worst-case decode/build peak of `size` wire bytes against a budget.
fn fits_memory_budget(size: u64, budget: u64) -> bool {
    size.saturating_mul(SNAPSHOT_DECODE_AMPLIFICATION) <= budget
}

/// The inbound memory guard's decision on the numbers (budget as a parameter
/// so the arithmetic and the message stay testable; `require_memory` reads the
/// real one). An honest large snapshot is not misbehaviour: the message says
/// replay continues, and nothing here ever touches peer trust.
fn require_memory_with(size: u64, budget: Result<u64, String>) -> Result<(), String> {
    let budget = budget?;
    if fits_memory_budget(size, budget) {
        return Ok(());
    }
    Err(format!(
        "snapshot of {size} bytes needs up to {} bytes to decode and check but the memory budget is {budget} bytes: replaying instead",
        size.saturating_mul(SNAPSHOT_DECODE_AMPLIFICATION)
    ))
}

/// Refuse a peer's snapshot this Mac cannot decode within the memory headroom
/// the node already computes ([`crate::resources::snapshot_memory_budget`] —
/// one quarter of available memory, capped by the configured node budget,
/// refused outright under critical pressure; warn defers to the budget, the
/// same policy the serving side applies in `resources::snapshot_gate_for`).
/// A size judgement is remembered
/// for [`MEMORY_REFUSAL_COOLDOWN`]; a pressure reading is not, since it can
/// lift on its own.
fn require_memory(size: u64) -> Result<(), String> {
    let budget = crate::resources::snapshot_memory_budget();
    let verdict = require_memory_with(size, budget.clone());
    if verdict.is_err() {
        if let Ok(b) = budget {
            warn!(
                size,
                budget = b,
                "refusing the peer's snapshot for memory; replaying instead (the peer did nothing wrong)"
            );
            note_refusal(size);
        } else {
            warn!(size, "refusing the peer's snapshot; replaying instead");
        }
    }
    verdict
}

/// The file a download assembles into, instead of RAM: created clean, streamed
/// through, hash-checked incrementally, and removed by `Drop` on success,
/// failure and the next start alike.
const WORKSPACE_FILE: &str = "snapshot-download.part";

/// Remove a crashed download's partial workspace file (before a new download
/// and at node startup, via `open_store_with`). Returns whether one was there.
fn clean_stale_workspace(dir: &std::path::Path) -> bool {
    std::fs::remove_file(dir.join(WORKSPACE_FILE)).is_ok()
}

struct WorkspaceFile {
    path: std::path::PathBuf,
    file: std::fs::File,
    hasher: blake3::Hasher,
    written: u64,
    /// The advertised size: a peer sending past it fails the download here.
    limit: u64,
}

impl WorkspaceFile {
    fn create(dir: &std::path::Path, size: u64) -> Result<Self, String> {
        clean_stale_workspace(dir);
        let path = dir.join(WORKSPACE_FILE);
        let file = std::fs::File::create(&path).map_err(|e| format!("snapshot workspace file: {e}"))?;
        Ok(WorkspaceFile { path, file, hasher: blake3::Hasher::new(), written: 0, limit: size })
    }

    /// One chunk onto the file and into the running hash.
    fn append(&mut self, part: &[u8]) -> Result<(), String> {
        if self.written.saturating_add(part.len() as u64) > self.limit {
            return Err("the peer sent more snapshot bytes than it advertised".into());
        }
        std::io::Write::write_all(&mut self.file, part).map_err(|e| format!("snapshot workspace file: {e}"))?;
        self.hasher.update(part);
        self.written += part.len() as u64;
        Ok(())
    }

    /// The download's integrity gate: exact size, exact BLAKE3. Only then is
    /// the file read back whole — the existing decode API
    /// (`Snapshot::from_bytes(&[u8])`) needs the whole slice in memory, so
    /// this is the one deliberate second copy of the wire bytes;
    /// [`SNAPSHOT_DECODE_AMPLIFICATION`] keeps the budget honest about it
    /// rather than redesigning decoding. `Drop` removes the file after.
    fn finish(self, size: usize, want: &str) -> Result<Vec<u8>, String> {
        if self.written != size as u64 || self.hasher.finalize().to_hex().to_string() != want {
            return Err("snapshot download does not match its BLAKE3".into());
        }
        std::fs::read(&self.path).map_err(|e| format!("snapshot workspace file: {e}"))
    }
}

impl Drop for WorkspaceFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Download `size` bytes in `chunk`-sized pieces, at most
/// [`SNAPSHOT_PARALLEL`] in flight, streaming each onto the workspace file
/// under `dir` — never holding the assembled snapshot in RAM — and return the
/// hash-verified bytes. `fetch_chunk` supplies chunk `index`'s exact
/// `expected` length; the file refuses anything past `size`. Failure removes
/// the workspace file ([`WorkspaceFile`]'s `Drop`).
async fn download_streamed<F, Fut>(
    dir: &std::path::Path,
    size: usize,
    chunk: usize,
    want: &str,
    fetch_chunk: F,
) -> Result<Vec<u8>, String>
where
    F: Fn(u64, usize) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    let mut ws = WorkspaceFile::create(dir, size as u64)?;
    let take = |index: u64| {
        let end = ((index + 1) * chunk as u64).min(size as u64);
        (end - index * chunk as u64) as usize
    };
    let fetch_chunk = fetch_chunk;
    futures::stream::iter(0..size.div_ceil(chunk) as u64)
        .map(move |index| fetch_chunk(index, take(index)))
        .buffered(SNAPSHOT_PARALLEL)
        .try_fold(&mut ws, |ws, part| async move {
            ws.append(&part)?;
            Ok(ws)
        })
        .await?;
    ws.finish(size, want)
}

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
                    // A full validator is not a bad one (2026-10-07: every
                    // busy answer rotated the shared connection, which moved
                    // a snapshot download onto a validator with another
                    // snapshot): keep it, the caller waits.
                    Err(e) if crate::spread::is_busy(e) => false,
                    // Nor is an older one: "method not found" (the prover's
                    // `aether_proverProgram` on 7780, every 5 s) rotated the
                    // shared connection under a snapshot download too.
                    Err(e) if crate::spread::unknown_method(e) => false,
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

    /// How many sources this upstream asks.
    pub fn sources(&self) -> usize {
        match self {
            Upstream::Http(urls) => urls.len(),
            Upstream::Iroh(c, _) => c.len(),
        }
    }

    /// Requests one source may have in flight while catching up: iroh
    /// validators admit 16 per peer (`aether_net`), so half of that; a
    /// loopback HTTP source has no such gate.
    pub fn per_source(&self) -> usize {
        match self {
            Upstream::Http(_) => 64,
            Upstream::Iroh(..) => crate::spread::PER_SOURCE,
        }
    }

    /// The source the shared calls are talking to now (where a download starts).
    fn preferred(&self) -> usize {
        match self {
            Upstream::Http(_) => 0,
            Upstream::Iroh(c, _) => c.preferred(),
        }
    }

    /// A stable name for source `i` (its URL or node id).
    pub fn source_key(&self, i: usize) -> String {
        match self {
            Upstream::Http(urls) => urls.get(i).cloned().unwrap_or_default(),
            Upstream::Iroh(c, _) => c.node_key(i).unwrap_or_default(),
        }
    }

    /// Ask source `i` alone — on its own connection for iroh, never moving the
    /// shared one. A refusal ("server busy" included) is the answer; nothing
    /// rotates. Catch-up spreads its requests with this (`spread`), and a
    /// snapshot download pins every chunk to the source of its manifest.
    pub async fn call_at(&self, i: usize, method: &str, params: Value) -> Result<Value, String> {
        let answer = match self {
            Upstream::Http(urls) => match urls.get(i) {
                Some(url) => http_call(url, method, &params).await,
                None => Err(format!("no source {i}")),
            },
            Upstream::Iroh(c, _) => c.call_at(i, method, params).await.map_err(|e| e.to_string()),
        };
        if answer.is_ok() {
            crate::chain::tick();
        }
        answer
    }

    /// The first non-null answer (null when every source has none).
    pub async fn first(&self, method: &str, params: Value) -> Result<Value, String> {
        self.ask(method, params, |v| Ok((!v.is_null()).then_some(v))).await.map(|v| v.unwrap_or(Value::Null))
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.ask(method, params, |v| Ok(Some(v))).await.map(|v| v.unwrap_or(Value::Null))
    }

    /// The network's finalized height, corroborated (pre-audit 7 PA7-06):
    /// the first answer AHEAD of `ours` is evidence enough and wins outright,
    /// but an answer claiming we are at the tip is an unsigned claim — a
    /// source stuck at (or lying about) a low tip must not hide an honest
    /// ahead-of-us alternative behind "first answer wins". So on a
    /// not-ahead answer every remaining HTTP source is asked too and the
    /// highest claim wins. The iroh client corroborates across its own peer
    /// set the same way (PA7B-06), moving to the peer with the best claim.
    pub async fn net_height(&self, ours: u64) -> Result<u64, String> {
        self.net_height_with_release(ours, None).await
    }

    async fn net_height_with_release(&self, ours: u64, chain: Option<&Chain>) -> Result<u64, String> {
        let ask_one = |v: Value| -> Result<u64, String> {
            if let Some(chain) = chain { chain.discover_release_hint(&v["release"]); }
            v["height"].as_u64().ok_or_else(|| "no upstream height".to_string())
        };
        match self {
            Upstream::Iroh(c, _) => {
                let v = c.call("aether_status", json!([])).await.map_err(|e| e.to_string())?;
                crate::chain::tick();
                let h = ask_one(v)?;
                if h > ours {
                    return Ok(h);
                }
                // Not ahead: this peer's claim is unsigned — a peer stuck at
                // (or lying about) a low tip must not hide an honest
                // ahead-of-us one behind "first answer wins" (PA7-06 gave
                // HTTP sources this; the default follower transport gets it
                // too, PA7B-06). Ask every OTHER peer directly, keep the
                // highest claim, and move to the peer that made it so the
                // next reads start there.
                let mut best = h;
                let mut best_at: Option<usize> = None;
                for (i, v) in c.ask_others("aether_status", json!([])).await {
                    let Ok(h2) = ask_one(v) else { continue };
                    crate::chain::tick();
                    if h2 > best {
                        best = h2;
                        best_at = Some(i);
                    }
                }
                if best > ours {
                    if let Some(i) = best_at {
                        c.rotate_to(i).await;
                    }
                    return Ok(best);
                }
                Ok(best)
            }
            Upstream::Http(urls) => {
                let mut best: Option<u64> = None;
                let mut last_err = String::from("no upstream");
                for url in urls {
                    match http_call(url, "aether_status", &json!([])).await {
                        Ok(v) => match ask_one(v) {
                            Ok(h) => {
                                crate::chain::tick();
                                // Ahead of us and of every claim so far: stop
                                // asking, this is the corroborated answer.
                                if h > ours && best.is_none_or(|b| h > b) {
                                    return Ok(h);
                                }
                                best = Some(best.map_or(h, |b| b.max(h)));
                            }
                            Err(e) => last_err = e,
                        },
                        Err(e) => last_err = e,
                    }
                }
                best.ok_or(last_err)
            }
        }
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

/// The peer's manifest, parsed and wire-bounds-checked (the resource guards
/// run in `guard` right after this, before any chunk is fetched).
fn manifest_limits(v: &Value) -> Result<(u64, usize, String, usize), String> {
    let (height, size, want) = (
        v["height"].as_u64().ok_or("no snapshot height")?,
        v["size"].as_u64().ok_or("no snapshot size")? as usize,
        v["blake3"].as_str().unwrap_or_default().to_string(),
    );
    let chunk = v["chunk"].as_u64().ok_or("no snapshot chunk size")? as usize;
    if size > MAX_SNAPSHOT || !(MIN_SNAPSHOT_CHUNK..=MAX_RESPONSE / 2).contains(&chunk) {
        return Err(format!("snapshot of {size} bytes in chunks of {chunk} is outside the limits"));
    }
    Ok((height, size, want, chunk))
}

/// Why a snapshot download from one source stopped.
enum Stop {
    /// That source's snapshot moved on mid-download: ask it again.
    MovedOn(String),
    /// That source cannot serve one now: try the next.
    Source(String),
}

/// A source's manifest, waiting while it builds one (bounded).
async fn manifest_at(upstream: &Upstream, i: usize) -> Result<Value, String> {
    for _ in 0..MANIFEST_WAITS {
        match crate::spread::call_patient(upstream, i, "aether_snapshot", &json!([])).await {
            Err(e) if e.contains("already queued") || e.contains("already running") => {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            other => return other,
        }
    }
    Err("the snapshot build did not finish".into())
}

/// One source's snapshot, every chunk from that same source (2026-10-07:
/// chunks fetched through the shared, rotating connection landed on
/// validators holding other snapshots — "snapshot moved on" every round).
async fn download_from(
    upstream: &Upstream,
    i: usize,
    dir: &std::path::Path,
    above: Option<u64>,
    guard: &(dyn Fn(u64) -> Result<(), String> + Send + Sync),
) -> Result<crate::snapshot::Snapshot, Stop> {
    let v = manifest_at(upstream, i).await.map_err(Stop::Source)?;
    if v.is_null() {
        return Err(Stop::Source("no snapshot there".into()));
    }
    let (height, size, want, chunk) = manifest_limits(&v).map_err(Stop::Source)?;
    // Too close to be worth a download: decided on the manifest, before any chunk.
    if let Some(above) = above.filter(|a| height <= *a) {
        return Err(Stop::Source(format!("the snapshot at {height} is not past {above}; replaying instead")));
    }
    // A smaller snapshot on another source may fit this host's budget.
    guard(size as u64).map_err(Stop::Source)?;
    // The one stage whose work is not blocks: name it, so a frozen height
    // during the download reads as progress, not as a stall (red team #2).
    crate::chain::set_stage(Some("snapshot"));
    let bytes = download_streamed(dir, size, chunk, &want, |index, expected| async move {
        let c = crate::spread::call_patient(upstream, i, "aether_snapshotChunk", &json!([height, index])).await?;
        let data = decode_snapshot_chunk(&c, expected)?;
        crate::chain::tick();
        Ok::<Vec<u8>, String>(data)
    })
    .await
    .map_err(|e| if e.contains("moved on") { Stop::MovedOn(e) } else { Stop::Source(e) })?;
    let snap = crate::snapshot::Snapshot::from_bytes(&bytes).map_err(Stop::Source)?;
    if snap.summary.height != height {
        return Err(Stop::Source("snapshot height does not match".into()));
    }
    Ok(snap)
}

/// The upstream's snapshot, downloaded and checked against its BLAKE3
/// (authenticity comes from the certified block after it, in `check`).
/// Every chunk comes from the source that served the manifest; when that
/// source's snapshot moves on mid-download (a validator rebuilds every 120
/// blocks when asked, and old validators serve only their current one), the
/// download restarts there at the new height ([`SNAPSHOT_RESTARTS`] times),
/// then moves to the next source. `above`: a snapshot at or below it is
/// refused on its manifest (`None`: any). `guard` runs with the advertised size
/// before the first chunk is fetched: disk space, and the inbound memory
/// budget against the worst-case decode/build peak. The chunks stream onto a
/// workspace file under `dir` (never the whole snapshot in RAM) and are
/// hash-checked incrementally.
async fn download_with(
    upstream: &Upstream,
    dir: &std::path::Path,
    above: Option<u64>,
    guard: &(dyn Fn(u64) -> Result<(), String> + Send + Sync),
) -> Result<crate::snapshot::Snapshot, String> {
    let n = upstream.sources();
    let first = upstream.preferred();
    let mut last = String::from("no upstream");
    for k in 0..n {
        let i = (first + k) % n;
        for _ in 0..SNAPSHOT_RESTARTS {
            match download_from(upstream, i, dir, above, guard).await {
                Ok(snap) => return Ok(snap),
                Err(Stop::MovedOn(e)) => {
                    info!(source = i, %e, "the snapshot moved on mid-download; restarting at its new height");
                    last = e;
                }
                Err(Stop::Source(e)) => {
                    last = e;
                    break;
                }
            }
        }
    }
    Err(last)
}

/// [`download_with`] with the shipped guard: enough room for the recovery on
/// the volume the database lives on (red team #7), and the measured inbound
/// memory budget (A3-5) — a recently refused size is answered from the
/// cooldown without re-measuring, so the loop cannot spin on one peer.
async fn download(upstream: &Upstream, dir: &std::path::Path, above: Option<u64>) -> Result<crate::snapshot::Snapshot, String> {
    download_with(upstream, dir, above, &|size| {
        if refusal_blocks(last_refusal(), size, std::time::Instant::now()) {
            return Err(format!(
                "snapshot of {size} bytes was already refused by the memory budget; replaying instead"
            ));
        }
        require_space(dir, size).and_then(|()| require_memory(size))
    })
    .await
}

/// Checkpoint sync: fetch the upstream's snapshot and the certified block
/// after it, check both, and write the snapshot as `store`'s checkpoint.
/// Returns the snapshot height.
pub async fn checkpoint(upstream: &Upstream, set: &ValidatorSet, cfg: &crate::chain::ChainConfig, store: &crate::store::Store) -> Result<u64, String> {
    let dir = store.path().parent().unwrap_or_else(|| std::path::Path::new("."));
    let snap = download(upstream, dir, None).await?;
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
    // A download that crashed mid-stream leaves its workspace file behind (A3-5):
    // every start wipes it, whether or not this run downloads again.
    clean_stale_workspace(data);
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
    let ours = chain.finalized_height();
    let snap = download(upstream, store.path().parent().unwrap_or_else(|| std::path::Path::new(".")), Some(ours + JUMP_BEHIND)).await?;
    let h = snap.summary.height;
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
    let gap = crate::backfill::merged(chain, ours + 1, h - 1);
    let metadata = crate::backfill::encode(gap);
    snap.install_over_with_meta(&store, &state, old, Some((crate::backfill::META, &metadata)))?;
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

/// A successful status answer is not progress when it advertises blocks we
/// cannot fetch. An unknown network height is likewise not evidence of being
/// at the tip: poc-m3 kept timing out at the same height for days.
struct StallWatch {
    since: Option<std::time::Instant>,
    height: u64,
    failures: u64,
}

impl StallWatch {
    fn new(height: u64) -> Self {
        Self { since: None, height, failures: 0 }
    }

    fn reset(&mut self, height: u64) {
        *self = Self::new(height);
    }

    /// Called after attempts and while one is pending. The caller resets the
    /// clock on active snapshot work before it checks a pending attempt.
    /// Only VERIFIED progress (an adopted block, `height > self.height`)
    /// clears the clock; every round without it — including a round that
    /// "succeeded" against an unsigned status claiming nothing new — keeps
    /// the clock running (pre-audit 7 PA7-06: a false-low-tip source used to
    /// erase the stall history on every answer, hiding the stall from the
    /// ten-minute self-heal for as long as it kept answering).
    ///
    /// A corroborated at-tip answer is the other case: `net_height` returns
    /// the HIGHEST claim across every source (PA7-06 corroborated the HTTP
    /// sources, PA7B-06 the default transport), so net == our height means
    /// every reachable source agrees the chain itself is at our height —
    /// the committee halting for maintenance or an outage, not a failure to
    /// follow available progress (pre-audit 7b PA7B-07). Waiting at such a
    /// tip clears the clock instead of spending it: a caught-up follower
    /// used to exit every stall window through a genuine pause, and the
    /// fourth exit in an hour made the supervisor stop permanently — the
    /// follower never resumed when consensus did.
    fn observe(&mut self, now: std::time::Instant, height: u64, net: Option<u64>, failed: bool) -> bool {
        if height > self.height {
            self.reset(height);
            return false;
        }
        if net == Some(height) {
            self.reset(height);
            return false;
        }
        self.height = height;
        self.failures += u64::from(failed);
        // One slow request or a short outage should not restart a healthy Mac.
        now.duration_since(*self.since.get_or_insert(now)) >= STALL_TIMEOUT
    }
}

/// Audit 7 A7-4: a status height is a fetch hint, not a fact — the same
/// evidence rule the validator startup gate applies (`StartupEvidence`
/// below, with the same bounded attempts and quarantine). A source
/// claiming a height ahead of the honest tip — a bug, a lie, a stale cache
/// — wins `net_height` outright, the fetch can certify nothing past the
/// real tip, and every round then observes `Some(H')` while adopting
/// nothing: the stall clock ran through a genuine pause, the follower
/// exited 11 every ten minutes, and the supervisor's four-per-hour stop
/// ended it for good while an honest source was configured all along. A
/// round that adopts nothing against an ahead claim is one more
/// unsupported fetch of that claim; after [`CLAIM_FETCH_ATTEMPTS`] such
/// rounds the height is quarantined for [`CLAIM_QUARANTINE`] and reads as
/// "at our own height", so the corroborated at-tip exemption
/// (`StallWatch`) keeps the pause free. The quarantine is per height: an
/// honest source resuming at a DIFFERENT height is never clamped, and the
/// quarantine expires so the claim is re-checked, not ignored forever.
#[derive(Default)]
struct AheadClaims {
    /// The last ahead claim no source could certify, and how many rounds it
    /// has now survived unsupported.
    claim: Option<(u64, u8)>,
    quarantined_until: Option<std::time::Instant>,
    /// The same per-run evidence for jumps: no new jump attempt before this
    /// (one failed jump costs [`JUMP_RETRY`] of replay, not a snapshot
    /// download every round)…
    jump_retry_at: Option<std::time::Instant>,
    /// …and the last jump that succeeded (from, to), for the run loop to
    /// backfill the heights it skipped.
    jumped: Option<(u64, u64)>,
}

impl AheadClaims {
    /// The height this round may act on. A quarantined claim reads as
    /// at-tip (our own height); every other height — a different, possibly
    /// honest ahead claim, or this one after the quarantine expires —
    /// passes through untouched.
    fn trusted(&self, claimed: u64, ours: u64, now: std::time::Instant) -> u64 {
        if claimed > ours
            && self.claim.is_some_and(|(h, _)| h == claimed)
            && self.quarantined_until.is_some_and(|until| now < until)
        {
            return ours;
        }
        claimed
    }

    /// One round's verdict on `claimed`: certified blocks at or past it
    /// clear the claim; anything else is another unsupported fetch of it.
    fn record(&mut self, claimed: u64, verified: bool, now: std::time::Instant) {
        if verified {
            self.claim = None;
            self.quarantined_until = None;
            return;
        }
        // An expired quarantine restarts the count: the claim gets its
        // bounded fetches again rather than an instant re-quarantine.
        let rounds = match self.claim {
            _ if self.quarantined_until.is_some_and(|until| now >= until) => 0,
            Some((h, n)) if h == claimed => n,
            _ => 0,
        };
        let rounds = rounds.saturating_add(1);
        if rounds >= CLAIM_FETCH_ATTEMPTS {
            self.quarantined_until = now.checked_add(CLAIM_QUARANTINE);
        }
        self.claim = Some((claimed, rounds));
    }
}

/// Follow the chain forever: verify, execute and persist each next block.
/// `joining`: this Mac's voting key when it is a candidate. When a finalized
/// handoff seats it, the follower stops before the switch height (for up to
/// `HOLD`), so `aether run` can start it as a voting node from that block.
/// `no_jump`: an archive node replays the gap instead of snapshot-jumping —
/// a jump keeps only certified facts of the snapshot block, so the store
/// loses the history index era export needs (audit 7 A7-1).
pub async fn run(
    chain: Chain,
    upstream: std::sync::Arc<Upstream>,
    set: ValidatorSet,
    archive: std::sync::Arc<FinalityArchive>,
    joining: Option<String>,
    no_jump: bool,
) {
    const HOLD: Duration = Duration::from_secs(120);
    let mut last_log = 0;
    let mut held_since: Option<std::time::Instant> = None;
    let mut stall = StallWatch::new(chain.finalized_height());
    // The ahead-claim evidence outlives every round (audit 7 A7-4): a claim
    // that keeps failing its fetches stays quarantined across restarts of
    // this loop, not just within one round.
    let mut claims = AheadClaims::default();
    // One height per round while idle at the tip (as before); a full batch
    // while there is a backlog to fetch.
    let mut window = 1u64;
    // The gap a jump skipped comes back in the background, newest first
    // (`backfill`); an unfinished one resumes after a restart.
    let mut backfilling: Option<Background> = None;
    if !no_jump {
        if let Some(gap) = crate::backfill::stored(&chain) {
            backfilling = Some(spawn_backfill(&chain, &upstream, &set, &archive, gap));
        }
    }
    loop {
        if claims.jumped.take().is_some() {
            // The snapshot commit already folded in the unfinished plan.
            drop(backfilling.take());
            if let Some(gap) = crate::backfill::stored(&chain) {
                backfilling = Some(spawn_backfill(&chain, &upstream, &set, &archive, gap));
            }
        }
        if !crate::resources::disk_ok() {
            // The disk guard has paused writes. Freeing space is recovery;
            // restarting the transport would only waste the restart budget.
            stall.reset(chain.finalized_height());
            crate::chain::set_stage(Some("disk"));
            window = 1;
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        // A handoff that seats this Mac: never run past its switch height
        // (`aether run` adopts this follower's state there), and hold once it
        // is reached.
        let cap = match hold_cap(&chain, joining.as_deref()) {
            Hold::Free => u64::MAX,
            Hold::Approach(switch) => switch - 1,
            Hold::Seated => {
                held_since.get_or_insert_with(std::time::Instant::now);
                if held_since.unwrap().elapsed() < HOLD {
                    // Waiting for the supervisor is intentional, not a
                    // transport failure. Resume with a fresh stall budget.
                    stall.reset(chain.finalized_height());
                    window = 1;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                // The supervisor never came: follow the chain again.
                u64::MAX
            }
        };
        let mut attempt = Box::pin(advance(
            &chain,
            &upstream,
            &set,
            Some(&archive),
            &mut claims,
            cap,
            window,
            &mut last_log,
            !no_jump,
        ));
        let mut activity = crate::chain::activity();
        let result = loop {
            tokio::select! {
                result = &mut attempt => break result,
                _ = tokio::time::sleep(Duration::from_secs(30)) => {
                    let height = chain.finalized_height();
                    if !crate::resources::disk_ok() {
                        stall.reset(height);
                        break Err("disk almost full; pausing follow".into());
                    }
                    let current_activity = crate::chain::activity();
                    if crate::chain::stage() == Some("snapshot") && current_activity != activity {
                        stall.reset(height);
                    } else if stall.observe(std::time::Instant::now(), height, chain.lock().net_height, true) {
                        tracing::error!(height, failures = stall.failures,
                            "follower upstream call made no verified progress for ten minutes; restarting its transport");
                        std::process::exit(EXIT_STALLED);
                    }
                    activity = current_activity;
                }
            }
        };
        let height = chain.finalized_height();
        let net = chain.lock().net_height;
        let storage_failure = matches!(&result, Err(e) if e.starts_with(STORE_FAILED));
        if storage_failure || !crate::resources::disk_ok() {
            // Storage has its own recovery path; low space pauses the loop.
            stall.reset(height);
        } else if stall.observe(std::time::Instant::now(), height, net, result.is_err()) {
            tracing::error!(height, ?net, failures = stall.failures, last_error = ?result.as_ref().err(),
                "follower made no verified progress for ten minutes; restarting its transport");
            std::process::exit(EXIT_STALLED);
        }
        match result {
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

fn spawn_backfill(
    chain: &Chain,
    upstream: &std::sync::Arc<Upstream>,
    set: &ValidatorSet,
    archive: &std::sync::Arc<FinalityArchive>,
    gap: crate::backfill::Gap,
) -> Background {
    Background(tokio::spawn(crate::backfill::run(chain.clone(), upstream.clone(), set.clone(), archive.clone(), gap)))
}

/// A background task that ends with its owner: the follow loop's backfill
/// never outlives the loop (a restarted loop resumes it from the store).
struct Background(tokio::task::JoinHandle<u64>);

impl Drop for Background {
    fn drop(&mut self) {
        self.0.abort();
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
/// `allow_jump`: false for an archive node, which must replay its gap — a
/// snapshot jump keeps no per-block history, so the history index era export
/// needs would stay gone forever (audit 7 A7-1).
/// `claims` (audit 7 A7-4) carries the ahead-claim evidence across rounds:
/// a claimed height nothing can certify is quarantined and reads as at-tip,
/// so it cannot spend the follower's restart budget through the stall
/// watchdog while the committee is simply paused.
async fn advance(
    chain: &Chain,
    upstream: &Upstream,
    set: &ValidatorSet,
    archive: Option<&FinalityArchive>,
    claims: &mut AheadClaims,
    cap: u64,
    window: u64,
    last_log: &mut u64,
    allow_jump: bool,
) -> Result<u64, String> {
    // Each round starts unnamed (red team #2): a stage that still holds names
    // itself again below; one that has finished does not linger in aether_status.
    crate::chain::set_stage(None);
    let mut adopted = 0u64;
    let mut window = window.max(1);
    loop {
        let ours = chain.finalized_height();
        // The stored answer dies with the probe that earned it (audit 7
        // A7-3): a follower at its upstream's tip keeps `Some(H)` here, and
        // when the probe then fails, both watchdog paths used to keep
        // feeding that stale at-tip answer to `StallWatch`, whose exemption
        // reset the clock forever. Forget it first; a successful probe
        // stores a fresh answer right below.
        chain.lock().net_height = None;
        // The height this round fetches toward is corroborated across
        // sources (PA7-06): a single false-low-tip answer used to cap the
        // fetch at our own height, so a round "succeeded" fetching nothing.
        let hint = upstream.net_height_with_release(ours, Some(chain)).await?;
        // The claimed height is a fetch hint, not a fact (audit 7 A7-4):
        // what this round OBSERVES (and jumps toward) is the trusted
        // reading, while the pipeline below still fetches toward the
        // CLAIMED height — a quarantined lie must not hide an honest
        // source's real progress behind it.
        let net = claims.trusted(hint, ours, std::time::Instant::now());
        chain.lock().net_height = Some(net);
        let may_jump = claims.jump_retry_at.is_none_or(|at| std::time::Instant::now() >= at);
        if allow_jump && net > ours + JUMP_BEHIND && adopted == 0 && may_jump {
            match jump(chain, upstream, set).await {
                Ok(to) => {
                    claims.jump_retry_at = None;
                    claims.jumped = Some((ours, to));
                    info!(from = ours, to, skipped = to - ours - 1, "jumped to a certified snapshot (the gap's blocks stay fetchable from era files)");
                    log_follow(chain, last_log);
                    chain.set_relaxed(false);
                    // The jump certified blocks at and past the claim: it
                    // was real (audit 7 A7-4).
                    claims.record(hint, true, std::time::Instant::now());
                    return Ok(to - ours);
                }
                Err(e) => {
                    crate::chain::set_stage(None);
                    claims.jump_retry_at = std::time::Instant::now().checked_add(JUMP_RETRY);
                    warn!(from = ours, %e, retry_in_s = JUMP_RETRY.as_secs(), "could not jump to a certified snapshot; replaying instead")
                }
            }
        }
        let last = pipeline(
            chain,
            upstream,
            set,
            archive,
            ours + 1,
            hint.min(cap),
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
            // This round adopted nothing while a source still claimed more
            // (audit 7 A7-4): the claim survived another fetch unsupported.
            if adopted == 0 && hint > ours {
                claims.record(hint, false, std::time::Instant::now());
                warn!(
                    claimed = hint,
                    height = ours,
                    "an ahead status claim certified no block; treating it as at-tip until it does (audit 7 A7-4)"
                );
            }
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
    // Spans dealt round-robin over every source, a few requests in flight
    // per source (`spread`), adopted strictly in order as they arrive.
    let n = upstream.sources().max(1);
    let slots = crate::spread::Slots::new(n, upstream.per_source());
    let slots = &slots;
    let mut fetched = futures::stream::iter(crate::spread::spans(from, to).into_iter().enumerate())
        .map(|(k, (a, b))| async move { (a, b, crate::spread::fetch_span(upstream, set, slots, k, a, b).await) })
        .buffered(2 * n);
    let mut last = from - 1;
    while let Some((a, b, r)) = fetched.next().await {
        let blocks = match r {
            Ok(blocks) => blocks,
            // The source pruned this era (roadmap B4): replay it from an era file instead.
            Err(e) if e.contains("pruned") => {
                drop(fetched);
                return match catch_up_era(chain, upstream, set, a).await {
                    Ok(to) => {
                        info!(from = a, to, "replayed a pruned era from its era file");
                        Ok(to.max(last))
                    }
                    Err(e) => {
                        warn!(height = a, %e, "upstream pruned this era and it could not be fetched");
                        Ok(last)
                    }
                };
            }
            Err(e) => {
                warn!(height = a, %e, "upstream");
                break;
            }
        };
        let full = blocks.len() as u64 == b - a + 1;
        let count = blocks.len();
        for (j, (block, proof)) in blocks.into_iter().enumerate() {
            let h = block.height.get();
            let batch_end = h == to || (!full && j + 1 == count);
            // A backlog replays without the per-block fsync (redb holds
            // those commits until a durable one); the batch's last block
            // commits durably and anchors it, so a crash mid-replay loses
            // only the open batch — certified blocks, they replay again.
            chain.set_relaxed(!batch_end);
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
        // A short span is where the sources' tip is: nothing past it yet.
        if !full {
            break;
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
        new_slots: 0,
        persistent_bytes: 0,
        proposer: summary.proposer,
        base_fee: summary.base_fee,
        excess: summary.excess,
        archive_excess: summary.archive_excess,
        settlement: Default::default(),
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
    let mut claims = AheadClaims::default();
    loop {
        let ours = chain.finalized_height();
        if let Err(e) = advance(chain, upstream, set, None, &mut claims, u64::MAX, window, &mut last_log, true).await {
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
/// A bogus status answer gets two bounded chances to supply its claimed
/// finalization before its claim is ignored for a minute.
const CLAIM_FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const CLAIM_FETCH_ATTEMPTS: u8 = 2;
const CLAIM_QUARANTINE: Duration = Duration::from_secs(60);

#[derive(Default)]
struct PeerEvidence {
    certified: Option<u64>,
    failed_attempts: u8,
    quarantined_until: Option<std::time::Instant>,
}

#[derive(Default)]
struct StartupEvidence(BTreeMap<String, PeerEvidence>);

impl StartupEvidence {
    fn certified_heights(&self) -> Vec<u64> {
        self.0.values().filter_map(|p| p.certified).collect()
    }

    fn certified(&self, peer: &str) -> Option<u64> {
        self.0.get(peer).and_then(|p| p.certified)
    }

    fn should_probe(&self, peer: &str, claimed: u64, now: std::time::Instant) -> bool {
        let Some(p) = self.0.get(peer) else { return true };
        if p.certified.is_some_and(|h| h >= claimed) { return false; }
        !p.quarantined_until.is_some_and(|until| now < until)
    }

    fn unresolved_ahead(&self, peer: &str, claimed: u64, ours: u64, margin: u64, now: std::time::Instant) -> bool {
        claimed > ours.saturating_add(margin)
            && self.certified(peer).is_none_or(|h| h < claimed)
            && self.0.get(peer).is_none_or(|p| !p.quarantined_until.is_some_and(|until| now < until))
    }

    fn record(&mut self, peer: &str, claimed: u64, verified: bool, now: std::time::Instant) {
        let p = self.0.entry(peer.to_owned()).or_default();
        if verified {
            p.certified = Some(p.certified.unwrap_or(0).max(claimed));
            p.failed_attempts = 0;
            p.quarantined_until = None;
        } else {
            if p.quarantined_until.is_some_and(|until| now >= until) {
                p.failed_attempts = 0;
                p.quarantined_until = None;
            }
            p.failed_attempts = p.failed_attempts.saturating_add(1);
            if p.failed_attempts >= CLAIM_FETCH_ATTEMPTS {
                p.quarantined_until = now.checked_add(CLAIM_QUARANTINE);
            }
        }
    }
}

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

/// The startup gate's decision from one roster census. Legacy callers pass
/// reported heights; new-genesis callers pass only certificate-backed heights
/// and the time since startup without any such evidence.
pub fn may_start_voting(answered: &[u64], ours: u64, margin: u64, silent_for: Duration, patience: Duration) -> VoteStart {
    if answered.is_empty() {
        return if silent_for >= patience { VoteStart::FailOpen } else { VoteStart::Wait };
    }
    if answered.iter().all(|h| *h <= ours + margin) { VoteStart::AtTip } else { VoteStart::Wait }
}

/// Ask every roster peer where it claims to be — a census, not the first answer: one
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
            Ok(conn) => match tokio::time::timeout(Duration::from_secs(10), aether_net::rpc_call(&conn, "aether_status", json!([]))).await {
                Ok(Ok(v)) => v["height"].as_u64().map(|h| (*n, h)),
                answer => {
                    tracing::debug!(peer = %n, ?answer, "answered no height");
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
/// the whole roster where it claims to be (`ask` returns every peer that
/// answered). On new-genesis chains, each height is checked against a verified
/// block and finalization certificate before it controls the gate or the
/// network height. Two failed bounded fetches quarantine an unsupported claim;
/// repeated status answers do not reset the five-minute patience clock.
/// Legacy chains retain their original status-height behavior. The gate decides:
///
/// - nobody beyond `margin` ahead → done, voting may start;
/// - a peer ahead → catch up from the tallest one (`from` builds an upstream
///   pointed at it) and ask again;
/// - no usable evidence for `patience` → done, with a warning.
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
    let new_genesis = {
        let cfg = chain.cfg();
        cfg.node_rewards || cfg.history_v2
    };
    let mut evidence = StartupEvidence::default();
    loop {
        let answers = ask().await;
        if new_genesis {
            // Probe each claim through the same certificate and block checks as
            // follower catch-up. A status answer alone never counts as height.
            let now = std::time::Instant::now();
            let from_ref = &from;
            let probes = answers.iter().filter(|(peer, h)| evidence.should_probe(&peer.to_string(), *h, now)).map(|(peer, h)| async move {
                if *h == 0 {
                    // Genesis is fixed by the local network file; no finality
                    // certificate exists for it and it cannot be ahead.
                    return (peer.to_string(), *h, true);
                }
                let upstream = from_ref(peer);
                let verified = matches!(tokio::time::timeout(CLAIM_FETCH_TIMEOUT, fetch(&upstream, set, *h)).await, Ok(Ok(Some((block, _)))) if block.height.get() == *h);
                (peer.to_string(), *h, verified)
            });
            for (peer, height, verified) in futures::future::join_all(probes).await {
                if !verified { warn!(%peer, claimed_height = height, "roster height has no verified finalization"); }
                evidence.record(&peer, height, verified, std::time::Instant::now());
            }
            if let Some(net) = evidence.certified_heights().into_iter().max() {
                chain.lock().net_height = Some(net);
            }
        } else if answers.is_empty() {
            warn!(silent_for = ?heard.elapsed(), ?patience, "no roster peer answers yet");
        } else {
            heard = std::time::Instant::now();
            // The highest answer is the network's height: beacon answers and
            // `aether_status` gate on it, and the engine covers the rest.
            let net = answers.iter().map(|(_, h)| *h).max().expect("a peer answered");
            chain.lock().net_height = Some(net);
        }
        let heights: Vec<u64> = if new_genesis {
            let certified = evidence.certified_heights();
            let current_proof = answers.iter().any(|(peer, claimed)| evidence.certified(&peer.to_string()).is_some_and(|h| h >= *claimed));
            if !current_proof && certified.iter().all(|h| *h <= chain.finalized_height().saturating_add(margin)) {
                // Old at-tip evidence does not replace a fresh census. A
                // certified peer ahead remains a blocker even if it vanishes.
                Vec::new()
            } else {
                certified
            }
        } else {
            answers.iter().map(|(_, h)| *h).collect()
        };
        if new_genesis && answers.iter().any(|(peer, h)| evidence.unresolved_ahead(&peer.to_string(), *h, chain.finalized_height(), margin, std::time::Instant::now())) {
            tokio::time::sleep(ASK_AGAIN.min(patience)).await;
            continue;
        }
        // Genesis needs no certificate, but a lone genesis answer must not
        // turn an unsupported claim of a later chain into instant permission
        // for a truly stale node. Give the normal patience window to recover
        // evidence before taking the same fail-open risk as total silence.
        if new_genesis && heights.iter().all(|h| *h == 0)
            && answers.iter().any(|(_, h)| *h > chain.finalized_height().saturating_add(margin))
            && heard.elapsed() < patience
        {
            tokio::time::sleep(ASK_AGAIN.min(patience)).await;
            continue;
        }
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
        if answers.is_empty() || (new_genesis && heights.is_empty()) {
            // Nothing answered, so there is nothing to catch up from: ask
            // again after a while, until the patience window runs out.
            tokio::time::sleep(ASK_AGAIN.min(patience)).await;
            continue;
        }
        // Somebody is ahead: catch up from the tallest peer that answered.
        let target = if new_genesis {
            answers.iter().filter_map(|(peer, _)| evidence.certified(&peer.to_string()).map(|h| (peer.clone(), h))).max_by_key(|(_, h)| *h)
        } else {
            answers.iter().max_by_key(|(_, h)| *h).cloned()
        };
        let Some((peer, at)) = target else {
            tokio::time::sleep(ASK_AGAIN.min(patience)).await;
            continue;
        };
        if new_genesis && at <= chain.finalized_height().saturating_add(margin) {
            // A previously certified higher peer is offline; keep waiting for
            // it or another source, but do not catch up from a smaller claim.
            tokio::time::sleep(ASK_AGAIN.min(patience)).await;
            continue;
        }
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

pub(crate) fn check(set: &ValidatorSet, h: u64, v: Value) -> Result<Option<(Block, Value)>, String> {
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
    use aether_test_support::Port;

    /// A JSON-RPC source on loopback whose answers `answer` decides.
    async fn mock_source(answer: impl Fn(&Value) -> Value + Send + Sync + 'static) -> String {
        use axum::{extract::State, routing::post, Json, Router};
        type Answer = std::sync::Arc<dyn Fn(&Value) -> Value + Send + Sync>;
        async fn handle(State(f): State<Answer>, Json(req): Json<Value>) -> Json<Value> {
            Json(f(&req))
        }
        let app = Router::new().route("/", post(handle)).with_state(std::sync::Arc::new(answer) as Answer);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{port}")
    }

    /// 2026-10-07, the founder's Mac: any failed chunk request moved the
    /// rest of a snapshot download to the next validator, whose snapshot sat
    /// at another height ("snapshot moved on" every round, for hours). The
    /// download now stays on the source of its manifest: a busy answer is
    /// waited out there, and a snapshot that moved on is restarted there at
    /// its new height. The second source, holding another snapshot, is never
    /// asked for a chunk.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_snapshot_download_stays_on_the_source_of_its_manifest() {
        let config = crate::chain::ChainConfig {
            chain_id: 7784,
            limits: aether_types::GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: vec![], fees: false, registrar: None, epoch_blocks: 0,
            min_streak: None, draw_epochs: None, history_v2: true, protocol: 1,
            node_rewards: false, committee: vec![], reserve: None, group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
        };
        let (chain, _) = crate::chain::Chain::new(config);
        let bytes = crate::snapshot::Snapshot::of(&chain).to_bytes();
        let (height, size, digest) = (chain.finalized_height(), bytes.len(), blake3::hash(&bytes).to_hex().to_string());
        let manifests = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let chunks = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let first = {
            let (manifests, chunks, digest) = (manifests.clone(), chunks.clone(), digest.clone());
            mock_source(move |req| {
                use std::sync::atomic::Ordering::SeqCst;
                let err = |m: String| json!({ "jsonrpc": "2.0", "id": req["id"], "error": { "code": -32000, "message": m } });
                match req["method"].as_str() {
                    Some("aether_snapshot") => {
                        manifests.fetch_add(1, SeqCst);
                        json!({ "jsonrpc": "2.0", "id": req["id"], "result": { "height": height, "size": size, "blake3": digest, "chunk": 1 << 20 } })
                    }
                    Some("aether_snapshotChunk") => match chunks.fetch_add(1, SeqCst) {
                        // An old validator's busy answer (no hint), then its
                        // snapshot moving on under the download.
                        0 => err(aether_net::BUSY.to_string()),
                        1 => err(format!("snapshot moved on to height {height}")),
                        _ => json!({ "jsonrpc": "2.0", "id": req["id"], "result": { "data": hex::encode(&bytes) } }),
                    },
                    _ => err("method not found".into()),
                }
            })
            .await
        };
        let asked_second = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let second = {
            let asked = asked_second.clone();
            mock_source(move |req| {
                if req["method"] == "aether_snapshotChunk" {
                    asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                if req["method"] == "aether_snapshot" {
                    return json!({ "jsonrpc": "2.0", "id": req["id"], "result": { "height": height, "size": size, "blake3": digest, "chunk": 1 << 20 } });
                }
                json!({ "jsonrpc": "2.0", "id": req["id"], "error": { "code": -32000, "message": "snapshot moved on to height 99" } })
            })
            .await
        };
        let dir = std::env::temp_dir().join(format!("aether-pinned-download-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let up = Upstream::Http(vec![first, second]);
        let snap = download_with(&up, &dir, None, &|_| Ok(())).await.expect("the download completes on its own source");
        assert_eq!(snap.summary.height, height);
        assert_eq!(manifests.load(std::sync::atomic::Ordering::SeqCst), 2, "restarted once on a fresh manifest");
        assert_eq!(asked_second.load(std::sync::atomic::Ordering::SeqCst), 0, "no chunk went to another source");
        // A manifest that is not past the floor is refused before any chunk.
        let before = chunks.load(std::sync::atomic::Ordering::SeqCst);
        let refused = download_with(&up, &dir, Some(height), &|_| Ok(())).await.unwrap_err();
        assert!(refused.contains("not past"), "{refused}");
        assert_eq!(chunks.load(std::sync::atomic::Ordering::SeqCst), before, "decided on the manifest alone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_snapshot_source_does_not_hide_a_usable_one() {
        let cfg = crate::chain::ChainConfig {
            chain_id: 7784,
            limits: aether_types::GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: vec![], fees: false, registrar: None, epoch_blocks: 0,
            min_streak: None, draw_epochs: None, history_v2: true, protocol: 1,
            node_rewards: false, committee: vec![], reserve: None, group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
        };
        let (chain, _) = crate::chain::Chain::new(cfg);
        // This test exercises download source selection only. Adoption still
        // checks the decoded snapshot against the next certified block.
        let mut snapshot = crate::snapshot::Snapshot::of(&chain);
        snapshot.summary.height = 10;
        let bytes = snapshot.to_bytes();
        let (size, digest) = (bytes.len(), blake3::hash(&bytes).to_hex().to_string());
        let stale_digest = digest.clone();
        let stale = mock_source(move |req| {
            assert_eq!(req["method"], "aether_snapshot", "stale manifest must not download chunks");
            json!({ "id": req["id"], "result": { "height": 0, "size": size, "blake3": stale_digest, "chunk": 1 << 20 } })
        }).await;
        let fresh = mock_source(move |req| {
            if req["method"] == "aether_snapshot" {
                json!({ "id": req["id"], "result": { "height": 10, "size": size, "blake3": digest, "chunk": 1 << 20 } })
            } else {
                json!({ "id": req["id"], "result": { "data": hex::encode(&bytes) } })
            }
        }).await;
        let dir = std::env::temp_dir().join(format!("aether-stale-snapshot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let snap = download_with(&Upstream::Http(vec![stale, fresh.clone()]), &dir, Some(1), &|_| Ok(())).await.expect("try the fresh source after a stale manifest");
        assert_eq!(snap.summary.height, 10);
        let oversized = mock_source(move |req| {
            assert_eq!(req["method"], "aether_snapshot", "refused size must not download chunks");
            json!({ "id": req["id"], "result": { "height": 10, "size": size * 2, "blake3": "00".repeat(32), "chunk": 1 << 20 } })
        }).await;
        let snap = download_with(&Upstream::Http(vec![oversized, fresh]), &dir, None, &|wire| {
            if wire <= size as u64 { Ok(()) } else { Err("snapshot exceeds this host's budget".into()) }
        }).await.expect("a smaller snapshot on another peer still fits");
        assert_eq!(snap.summary.height, 10);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn repeated_upstream_timeouts_at_one_height_restart_the_follower() {
        let start = std::time::Instant::now();
        let mut watch = StallWatch::new(144_651);
        // poc-m3 never obtained a status after restarting; it repeatedly
        // timed out connecting to four peers for block 144652.
        assert!(!watch.observe(start, 144_651, None, true));
        assert!(!watch.observe(start + Duration::from_secs(9 * 60), 144_651, None, true));
        assert!(watch.observe(start + STALL_TIMEOUT, 144_651, None, true));
        assert_eq!(EXIT_STALLED, 11);
    }

    #[test]
    fn an_upstream_call_that_never_returns_is_also_stalled() {
        let start = std::time::Instant::now();
        let mut watch = StallWatch::new(144_651);
        // The pending-call watchdog samples every 30 seconds, even if
        // `advance` never returns from a transport call.
        for tick in 0..20 {
            assert!(!watch.observe(start + Duration::from_secs(tick * 30), 144_651, None, true));
        }
        assert!(watch.observe(start + STALL_TIMEOUT, 144_651, None, true));
    }

    #[test]
    fn successful_status_with_missing_blocks_does_not_hide_a_stall() {
        let start = std::time::Instant::now();
        let mut watch = StallWatch::new(100);
        // pipeline logs fetch errors and returns Ok(last). Each round can
        // therefore succeed without obtaining any advertised block.
        for seconds in [0, 30, 300, 599] {
            assert!(!watch.observe(start + Duration::from_secs(seconds), 100, Some(200), false));
        }
        assert!(watch.observe(start + STALL_TIMEOUT, 100, Some(200), false));
    }

    #[test]
    fn intentional_pause_resets_the_transport_failure_budget() {
        let start = std::time::Instant::now();
        let mut watch = StallWatch::new(100);
        assert!(!watch.observe(start, 100, None, true));
        // Disk/storage recovery or a handoff hold can last beyond the
        // transport timeout, and storage recovery may roll height back.
        watch.reset(99);
        let resumed = start + STALL_TIMEOUT * 2;
        assert!(!watch.observe(resumed, 99, None, true));
        assert!(!watch.observe(resumed + STALL_TIMEOUT - Duration::from_secs(1), 99, None, true));
        assert!(watch.observe(resumed + STALL_TIMEOUT, 99, None, true));
    }

    #[test]
    fn verified_progress_clears_the_stall_clock_and_an_unfetched_ahead_claim_does_not() {
        let start = std::time::Instant::now();
        let mut watch = StallWatch::new(100);
        assert!(!watch.observe(start, 100, Some(10_000), false));
        assert!(!watch.observe(start + Duration::from_secs(9 * 60), 100, Some(10_000), false));
        // A status response alone did not resolve the gap.
        assert!(watch.observe(start + STALL_TIMEOUT, 100, Some(10_000), false));
        assert!(!watch.observe(start + STALL_TIMEOUT + Duration::from_secs(1), 101, Some(10_000), false));
        // An at-tip answer where every source agrees (net == ours, the
        // highest claim across sources) is a corroborated pause: the clock
        // waits (PA7B-07), however long the pause runs — including windows
        // that would have exited while the committee was simply halted
        // (post-PA7-06 code kept the clock running through every such
        // answer, four exits an hour exhausting the restart budget).
        assert!(!watch.observe(start + STALL_TIMEOUT * 2, 101, Some(101), false));
        assert!(!watch.observe(start + STALL_TIMEOUT * 4, 101, Some(101), false));
    }

    /// PA7B-07's own scenario: a genuine chain pause — the committee halted
    /// for maintenance or an outage, every source agreeing the network is at
    /// our height (net_height corroborates the HIGHEST claim, so an honest
    /// ahead source would have said more) — is waiting, not a stall. The
    /// post-PA7-06 clock ran through every such answer: a caught-up follower
    /// exited every ten minutes of the pause, the fourth exit in an hour
    /// stopped the supervisor permanently, and when consensus resumed the
    /// follower was gone. Waiting now costs nothing, and recovery is the
    /// ordinary two: the first new block is verified progress, and a REAL
    /// stall afterwards (no corroboration at all) still fires within one
    /// window.
    #[test]
    fn a_genuine_chain_pause_is_survived_and_resumed() {
        let start = std::time::Instant::now();
        let mut watch = StallWatch::new(100);
        // Forty minutes of honest pause, well past the stall window — and a
        // failed round inside it (transport blips happen during halts too).
        assert!(!watch.observe(start, 100, Some(100), true));
        for seconds in [60u64, 600, 1_800, 2_400] {
            assert!(
                !watch.observe(start + Duration::from_secs(seconds), 100, Some(100), false),
                "an honest pause of {seconds}s is waiting, not a stall"
            );
        }
        // Consensus resumes: the first adopted block is verified progress
        // and clears the clock the ordinary way.
        assert!(!watch.observe(start + Duration::from_secs(2_401), 101, Some(200), false));
        // And a real stall after the pause — no source answering at all —
        // fires within one window of the last verified progress.
        assert!(!watch.observe(start + Duration::from_secs(2_402), 101, None, true));
        assert!(watch.observe(start + Duration::from_secs(2_402) + STALL_TIMEOUT, 101, None, true));
    }

    /// The corroborated height (PA7-06): a not-ahead answer does not win
    /// outright — the remaining sources are asked, and an honest ahead source
    /// corrects a false-low-tip one in either order. The old `first`-based
    /// height returned the low source's 100 and the round fetched nothing.
    #[tokio::test]
    async fn a_false_low_tip_is_corroborated_across_alternative_sources() {
        let spawn_status = |height: u64| async move {
            let app = axum::Router::new().route("/", axum::routing::post(move |body: axum::body::Bytes| async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                let id = v.get("id").cloned().unwrap_or(Value::Null);
                axum::Json(json!({ "jsonrpc": "2.0", "id": id, "result": { "height": height } }))
            }));
            let port = Port::reserve().expect("reserve status RPC port");
            let addr = port.addr();
            let listener = port.bind_tcp().unwrap();
            listener.set_nonblocking(true).unwrap();
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            tokio::spawn(async move {
                let _port = port;
                axum::serve(listener, app).await
            });
            format!("http://{addr}")
        };
        let (low, ahead) = (spawn_status(100).await, spawn_status(200).await);
        let ours = 100u64;
        // The stuck source listed first is where the old code stopped.
        assert_eq!(Upstream::Http(vec![low.clone(), ahead.clone()]).net_height(ours).await.unwrap(), 200);
        // An ahead answer first still wins outright…
        assert_eq!(Upstream::Http(vec![ahead.clone(), low.clone()]).net_height(ours).await.unwrap(), 200);
        // …and a lone low answer is still the best claim there is — returned
        // as the (unsigned) height, with the stall clock the safety net.
        assert_eq!(Upstream::Http(vec![low]).net_height(ours).await.unwrap(), 100);
    }

    /// The same corroboration on the DEFAULT follower transport (pre-audit 7b
    /// PA7B-06): the iroh `net_height` used to take the current peer's claim
    /// alone, so a caught-up follower parked on a peer stuck at a low tip
    /// believed the network was there too — no catch-up, no rotation,
    /// indefinitely, while HTTP sources already corroborated (PA7-06). The
    /// iroh path now asks every OTHER peer directly, keeps the highest claim,
    /// and moves the client to the peer that made it. The old code returned
    /// the stale peer's 100 here and never moved.
    #[tokio::test]
    async fn an_iroh_follower_corroborates_height_across_alternate_peers() {
        let status_server = |height: u64| async move {
            let secret = aether_net::SecretKey::generate();
            let id = secret.public();
            let server = aether_net::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .relay_mode(iroh::RelayMode::Disabled)
                .alpns(vec![aether_net::ALPN_RPC.to_vec()])
                .secret_key(secret)
                .bind()
                .await
                .unwrap();
            let port = server.bound_sockets().into_iter().find(|a| a.is_ipv4()).expect("an IPv4 socket").port();
            let router = aether_net::serve_rpc(server, move |req: Value| async move {
                json!({ "jsonrpc": "2.0", "id": req["id"], "result": { "height": height } })
            });
            let addr = aether_net::EndpointAddr::from_parts(
                id,
                [aether_net::TransportAddr::Ip(std::net::SocketAddr::from((
                    std::net::Ipv4Addr::LOCALHOST,
                    port,
                )))],
            );
            (addr, router)
        };
        // The stale peer is first: the client connects there (start = 0) and
        // its answer says we are AT the tip (100) — not ahead, so every other
        // peer must be asked before that claim is believed.
        let (stale, _stale_router) = status_server(100).await;
        let (ahead, _ahead_router) = status_server(200).await;
        let client = aether_net::RpcClient::with_addrs(vec![stale, ahead]).await.unwrap();
        let up = Upstream::Iroh(client, Default::default());
        assert_eq!(up.net_height(100).await, Ok(200), "the honest peer's 200 must beat the stale peer's 100");
        // And the client moved to it: the next ask answers from the honest
        // peer outright (a fresh scan starts there), never touching the
        // stale one first again.
        assert_eq!(up.net_height(150).await, Ok(200), "the client rotates to the peer with the best claim");
    }

    /// Audit 7 A7-3: the stored network height must never outlive the probe
    /// that earned it. A follower at its upstream's tip H keeps
    /// `net_height = Some(H)`; when every later status request fails,
    /// `advance` returned early without touching the field, and both the
    /// pending-attempt watchdog and the completed-error path kept feeding
    /// `Some(H)` to `StallWatch` — whose at-tip exemption (PA7B-07) reset
    /// the clock on every observation, so the ten-minute transport restart
    /// never fired however long the failure lasted. The round now forgets
    /// the stored answer before it probes: a failed round observes `None`
    /// (the clock runs), and a successful at-tip round re-stores a fresh
    /// answer below (the healthy-pause exemption itself is untouched — see
    /// `a_genuine_chain_pause_is_survived_and_resumed`).
    #[tokio::test]
    async fn a_failed_height_probe_does_not_inherit_an_old_at_tip_answer() {
        let config = crate::chain::ChainConfig {
            chain_id: 7783,
            limits: aether_types::GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: vec![], fees: false, registrar: None, epoch_blocks: 0,
            min_streak: None, draw_epochs: None, history_v2: true, protocol: 2,
            node_rewards: false, committee: vec![], reserve: None, group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
        };
        let (chain, _) = crate::chain::Chain::new(config);
        // The follower reached its upstream's tip at genesis and stored the
        // corroborated at-tip answer…
        chain.lock().net_height = Some(0);
        // …then every status request fails (nothing listens there).
        let upstream = Port::reserve().expect("reserve unreachable upstream RPC port");
        let err = advance(
            &chain,
            &Upstream::Http(vec![format!("http://{}", upstream.addr())]),
            &aether_light::ValidatorSet::devnet(4),
            None,
            &mut AheadClaims::default(),
            u64::MAX,
            1,
            &mut 0,
            true,
        )
        .await
        .unwrap_err();
        assert!(!err.is_empty(), "the dead upstream must fail the round");
        // The failed round must not leave the old at-tip answer behind for
        // the watchdog to keep exempting the stall with.
        assert_eq!(
            chain.lock().net_height, None,
            "a failed height probe must not inherit an old at-tip answer"
        );
    }

    /// Audit 7 A7-4: a status height is a fetch hint, not a fact. The
    /// committee paused at H, the honest sources stay at H, and one source
    /// claims a height far above it. `net_height` prefers an ahead answer,
    /// the fetch cannot certify any block past the real tip, so every round
    /// adopted nothing while observing `Some(H')` — the stall clock ran,
    /// the follower exited 11 every ten minutes, and the supervisor's
    /// four-per-hour stop ended it for good while an honest source was
    /// configured all along. Two rounds that adopt nothing against an ahead
    /// claim quarantine that height for a minute: while quarantined it
    /// reads as "at our own height", so the corroborated at-tip exemption
    /// keeps the pause free — and a DIFFERENT ahead height (the honest
    /// source resuming) is never clamped.
    #[tokio::test]
    async fn a_false_ahead_claim_during_a_pause_does_not_stall_the_follower() {
        let status_server = |height: u64| async move {
            let app = axum::Router::new().route("/", axum::routing::post(move |body: axum::body::Bytes| async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                let id = v.get("id").cloned().unwrap_or(Value::Null);
                // A status answer claims its height; nothing past it is
                // certified (the committee paused, so there is nothing).
                let result = if v.get("method").and_then(Value::as_str) == Some("aether_status") {
                    json!({ "height": height })
                } else {
                    Value::Null
                };
                axum::Json(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
            }));
            let port = Port::reserve().expect("reserve status RPC port");
            let addr = port.addr();
            let listener = port.bind_tcp().unwrap();
            listener.set_nonblocking(true).unwrap();
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            tokio::spawn(async move {
                let _port = port;
                axum::serve(listener, app).await
            });
            format!("http://{addr}")
        };
        // The lying source is listed first: its ahead answer is what
        // `net_height` takes (an ahead claim wins outright), and the honest
        // source sits at our own height (genesis).
        let (liar, honest) = (status_server(10_000).await, status_server(0).await);
        let up = Upstream::Http(vec![liar, honest]);
        let config = crate::chain::ChainConfig {
            chain_id: 7784,
            limits: aether_types::GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: vec![], fees: false, registrar: None, epoch_blocks: 0,
            min_streak: None, draw_epochs: None, history_v2: true, protocol: 2,
            node_rewards: false, committee: vec![], reserve: None, group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
        };
        let (chain, _) = crate::chain::Chain::new(config);
        let set = aether_light::ValidatorSet::devnet(4);
        let mut claims = AheadClaims::default();
        async fn round(
            chain: &Chain,
            up: &Upstream,
            set: &ValidatorSet,
            claims: &mut AheadClaims,
        ) -> Result<u64, String> {
            advance(chain, up, set, None, claims, u64::MAX, 1, &mut 0, false).await
        }
        // Two rounds adopt nothing against the unsupported claim…
        for _ in 0..2 {
            assert_eq!(round(&chain, &up, &set, &mut claims).await.unwrap(), 0);
            assert_eq!(
                chain.lock().net_height, Some(10_000),
                "before quarantine the claim is still acted on"
            );
        }
        // …so the third round treats it as at-tip: the stored height the
        // stall watchdog sees is OUR height, and the pause costs nothing.
        assert_eq!(round(&chain, &up, &set, &mut claims).await.unwrap(), 0);
        assert_eq!(
            chain.lock().net_height, Some(0),
            "a quarantined unsupported claim must read as at-tip, not as a forever-away tip"
        );
        // The quarantine is per height: an honest source resuming (a new,
        // different height ahead of us) is never clamped by the old lie.
        assert_eq!(claims.trusted(1, 0, std::time::Instant::now()), 1);
        // And it expires, so a claim gets re-checked rather than being
        // believed or ignored forever.
        assert_eq!(
            claims.trusted(10_000, 0, std::time::Instant::now() + CLAIM_QUARANTINE + Duration::from_secs(1)),
            10_000
        );
    }

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
        // Legacy status heights still work exactly as before. For new genesis,
        // this same decision is fed only certified heights (see tests below).
        assert_eq!(may_start_voting(&[121], 100, BEHIND_MARGIN, Duration::ZERO, patience), VoteStart::Wait);
        assert_eq!(may_start_voting(&[98, 500], 100, BEHIND_MARGIN, patience, patience), VoteStart::Wait);
        // Nobody answered: not "0 behind" — wait, and only after the patience
        // window fail open.
        assert_eq!(may_start_voting(&[], 100, BEHIND_MARGIN, patience - Duration::from_millis(1), patience), VoteStart::Wait);
        assert_eq!(may_start_voting(&[], 100, BEHIND_MARGIN, patience, patience), VoteStart::FailOpen);
    }

    #[test]
    fn unproved_absurd_claim_cannot_hold_a_new_genesis_restart_gate() {
        let start = std::time::Instant::now();
        let mut evidence = StartupEvidence::default();
        evidence.record("honest-a", 100, true, start);
        evidence.record("honest-b", 100, true, start);
        for attempt in 0..2 {
            let now = start + Duration::from_secs(attempt);
            assert!(evidence.should_probe("liar", 1_000_000_000_000, now));
            evidence.record("liar", 1_000_000_000_000, false, now);
            if attempt == 0 {
                assert!(evidence.unresolved_ahead("liar", 1_000_000_000_000, 100, BEHIND_MARGIN, now));
            }
        }
        assert!(!evidence.should_probe("liar", 1_000_000_000_000, start + Duration::from_secs(3)));
        assert!(!evidence.unresolved_ahead("liar", 1_000_000_000_000, 100, BEHIND_MARGIN, start + Duration::from_secs(3)));
        assert!(evidence.should_probe("liar", 1_000_000_000_000, start + CLAIM_QUARANTINE + Duration::from_secs(1)));
        assert_eq!(may_start_voting(&evidence.certified_heights(), 100, BEHIND_MARGIN, Duration::from_secs(3), STARTUP_PATIENCE), VoteStart::AtTip);
        // If no honest peer can supply a certificate, repeated status answers
        // still cannot reset the five-minute fail-open clock.
        let only_liar = StartupEvidence::default();
        assert_eq!(may_start_voting(&only_liar.certified_heights(), 100, BEHIND_MARGIN, STARTUP_PATIENCE, STARTUP_PATIENCE), VoteStart::FailOpen);
    }

    #[test]
    fn certified_ahead_peer_still_blocks_voting_even_if_it_is_quarantined_later() {
        let start = std::time::Instant::now();
        let mut evidence = StartupEvidence::default();
        evidence.record("honest", 500, true, start);
        evidence.record("honest", 1_000_000_000_000, false, start + Duration::from_secs(1));
        evidence.record("honest", 1_000_000_000_000, false, start + Duration::from_secs(2));
        assert_eq!(evidence.certified_heights(), vec![500]);
        assert_eq!(may_start_voting(&evidence.certified_heights(), 100, BEHIND_MARGIN, STARTUP_PATIENCE, STARTUP_PATIENCE), VoteStart::Wait);
        assert_eq!(may_start_voting(&evidence.certified_heights(), 480, BEHIND_MARGIN, STARTUP_PATIENCE, STARTUP_PATIENCE), VoteStart::AtTip);
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

    /// A fresh, empty directory per test (the crate has no tempfile dependency).
    fn scratch(name: &str) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "aether-follow-{}-{}-{}",
            name,
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// A3-5 (a): a manifest whose worst-case decode/build peak does not fit the
    /// memory budget is refused before any chunk is fetched, and the refusal is
    /// remembered long enough that the follow loop cannot spin on the same
    /// peer-advertised size — while never counting as peer misbehaviour.
    #[test]
    fn a_snapshot_beyond_the_memory_budget_is_refused_and_not_retried_in_a_loop() {
        const AMP: u64 = SNAPSHOT_DECODE_AMPLIFICATION;
        // The worst-case peak is the advertised size times the amplification.
        assert!(fits_memory_budget(8, 8 * AMP), "exactly the budget fits");
        assert!(!fits_memory_budget(8, 8 * AMP - 1), "one byte of peak over is refused");
        assert!(!fits_memory_budget(u64::MAX, u64::MAX - 1), "the peak saturates instead of wrapping past the budget");
        // The refusal names the memory budget (not the peer) and says replay continues.
        let err = require_memory_with(1 << 30, Ok(64 << 20)).unwrap_err();
        assert!(err.contains("memory budget"), "{err}");
        assert!(err.contains("replaying"), "{err}");
        assert!(!err.contains("lied"), "an honest large snapshot is not misbehaviour: {err}");
        // A peer under critical pressure is refused without a size judgement too.
        assert!(require_memory_with(1, Err("pressure".into())).is_err());
        // The cooldown: the same size asked again within it is still refused
        // without a second budget measurement, a smaller snapshot is not.
        let now = std::time::Instant::now();
        assert!(!refusal_blocks(None, 1 << 30, now), "nothing was refused yet");
        assert!(refusal_blocks(Some((1 << 30, now)), 1 << 30, now + Duration::from_secs(1)));
        assert!(!refusal_blocks(Some((1 << 30, now)), 1 << 29, now + Duration::from_secs(1)),
            "a smaller snapshot than the one refused is a different question");
        assert!(!refusal_blocks(Some((1 << 30, now)), 1 << 30, now + MEMORY_REFUSAL_COOLDOWN + Duration::from_secs(1)),
            "after the cooldown the budget may have changed: measure again");
    }

    /// A3-5 (b): a peer that stops mid-download leaves no workspace file behind.
    #[tokio::test]
    async fn a_stalled_download_leaves_no_workspace_file() {
        let dir = scratch("stalled");
        // A leftover from a previous crash is cleaned up before a new download.
        std::fs::write(dir.join(WORKSPACE_FILE), b"stale").unwrap();
        let err = download_streamed(&dir, 2 << 20, 1 << 20, "00", |index, _| async move {
            if index == 1 {
                Err("peer stopped mid-download".into())
            } else {
                Ok(vec![0u8; 1 << 20])
            }
        })
        .await
        .unwrap_err();
        assert_eq!(err, "peer stopped mid-download");
        assert!(!dir.join(WORKSPACE_FILE).exists(), "failure must remove the workspace file");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A3-5 (b): a download that completes but does not hash to the manifest's
    /// BLAKE3 (or truncates) is refused and cleaned up.
    #[tokio::test]
    async fn a_mismatched_download_is_refused_and_cleaned_up() {
        let dir = scratch("mismatch");
        let data = vec![7u8; 3 << 20];
        let want = crate::rpc::blake3_hex(&data);
        let truncated = download_streamed(&dir, 2 << 20, 1 << 20, &want, |_, n| {
            let part = vec![7u8; n];
            async move { Ok(part) }
        })
        .await; // supplies 2 MiB of the 2 MiB asked — but the hash is of 3 MiB
        assert!(truncated.is_err(), "the assembled bytes must match the advertised hash");
        assert!(!dir.join(WORKSPACE_FILE).exists(), "a refused snapshot leaves no file");
        let honest = crate::rpc::blake3_hex(&data[..1 << 20]);
        assert!(download_streamed(&dir, 1 << 20, 1 << 20, &honest, |_, n| {
            let part = data[..n].to_vec();
            async move { Ok(part) }
        })
        .await
        .is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A3-5 (c): the happy path streams every chunk through the workspace file
    /// and returns exactly the assembled bytes; the file is gone afterwards.
    #[tokio::test]
    async fn the_happy_path_streams_through_the_workspace_file() {
        let dir = scratch("happy");
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let want = crate::rpc::blake3_hex(&data);
        let chunk = 1 << 20;
        let out = download_streamed(&dir, data.len(), chunk, &want, |index, n| {
            let part = data[index as usize * chunk..][..n].to_vec();
            async move { Ok(part) }
        })
        .await
        .expect("an honest download assembles");
        assert_eq!(out.len(), data.len());
        assert_eq!(out, data, "the streamed bytes are the assembled snapshot");
        assert!(!dir.join(WORKSPACE_FILE).exists(), "success must remove the workspace file");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A3-5 (2): the workspace file left by a crashed download is removed the
    /// next time the node starts a download (and at startup via `open_store`).
    #[test]
    fn a_stale_workspace_file_is_removed_before_reuse() {
        let dir = scratch("stale");
        assert!(!clean_stale_workspace(&dir), "a fresh dir has nothing to clean");
        assert!(!dir.join(WORKSPACE_FILE).exists(), "nothing lingers");
        std::fs::write(dir.join(WORKSPACE_FILE), b"leftover").unwrap();
        assert!(clean_stale_workspace(&dir), "the crashed download's file goes");
        assert!(!dir.join(WORKSPACE_FILE).exists());
        assert!(!clean_stale_workspace(&dir), "a second start finds nothing to clean");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The measured justification for [`SNAPSHOT_DECODE_AMPLIFICATION`]: the
    /// follower's real inbound path on a synthetic snapshot — stream the wire
    /// bytes through the workspace file, read them back (the one deliberate
    /// full copy `Snapshot::from_bytes` needs), decode, then do exactly what
    /// `Snapshot::check` does (`entries.clone()` + `WorldState::from_parts` +
    /// root) — sampled by physical footprint, like the server-side
    /// `snapshot_build_allocation_probe` it answers to. The wire bytes already
    /// in the baseline stand for the file on disk, so the delta is the pure
    /// inbound peak.
    #[test]
    #[ignore = "manual physical-footprint measurement of the follower's inbound peak"]
    fn follower_decode_allocation_probe() {
        use crate::chain::ChainConfig;
        let config = ChainConfig {
            chain_id: 7781,
            limits: aether_types::GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: vec![], fees: false, registrar: None, epoch_blocks: 0,
            min_streak: None, draw_epochs: None, history_v2: true, protocol: 2,
            node_rewards: false, committee: vec![], reserve: None, group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
        };
        let entries: Vec<_> = (0..300_000u32).map(|i| {
            let mut key = [0u8; 32];
            key[..4].copy_from_slice(&i.to_be_bytes());
            (key, [7u8; 32])
        }).collect();
        let state = aether_execution::WorldState::from_parts(entries, Default::default());
        let (chain, _) = crate::chain::Chain::new(config);
        {
            let mut g = chain.lock();
            let mut head = (*g.finalized).clone();
            head.state = state;
            g.finalized = std::sync::Arc::new(head);
        }
        let wire = crate::snapshot::Snapshot::of(&chain).to_bytes();
        drop(chain);
        let pid = std::process::id();
        let before = crate::resources::footprint(pid).unwrap_or(0);
        let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let peak = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(before));
        let sampler = {
            let (running, peak) = (running.clone(), peak.clone());
            std::thread::spawn(move || while running.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some(n) = crate::resources::footprint(pid) {
                    peak.fetch_max(n, std::sync::atomic::Ordering::Relaxed);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            })
        };
        let dir = scratch("probe");
        let want = crate::rpc::blake3_hex(&wire);
        let chunk = 1 << 20;
        let bytes = futures::executor::block_on(download_streamed(&dir, wire.len(), chunk, &want, |index, n| {
            let part = wire[index as usize * chunk..][..n].to_vec();
            async move { Ok(part) }
        }))
        .expect("probe download");
        let decoded = crate::snapshot::Snapshot::from_bytes(&bytes).expect("probe decode");
        // Exactly what `Snapshot::check` allocates before its root comparison:
        let checked = aether_execution::WorldState::from_parts(
            decoded.entries.clone(),
            decoded.codes.iter().cloned().collect(),
        );
        let _ = checked.root();
        running.store(false, std::sync::atomic::Ordering::Relaxed);
        sampler.join().unwrap();
        let delta = peak.load(std::sync::atomic::Ordering::Relaxed).saturating_sub(before);
        let ratio = (delta as f64 / wire.len() as f64).ceil();
        eprintln!(
            "follower inbound probe: entries={}, wire={}, sampled_footprint_delta={}, amplification(ceil)={}",
            decoded.entries.len(), wire.len(), delta, ratio
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
