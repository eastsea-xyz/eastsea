//! `aether run`: one Mac, one process to keep running. It runs this Mac as a
//! validator while it is in the voting set and as a verifying follower (and
//! voting-node candidate) otherwise, and moves between the two on its own
//! (docs/design/07-consensus.md, "open voting nodes"):
//!
//! 1. When the chain proposes a new voting set (`aether_rotation`), old and new
//!    members reshare the committee key in the background into staged files;
//!    the running validators never stop for it.
//! 2. On success each running validator signs the handoff of its own staged
//!    reshare; at the threshold a block carries it (`handoff.rs`).
//! 3. Once the chain has finalized up to the switch height, every Mac installs
//!    its new role: validators that stay take the new share, new members start
//!    from the block they verified as followers, members that leave follow.
//!
//! A failed reshare changes nothing: the running set keeps going and the next
//! ceremony window tries again. No person acts at any step.

use crate::roster::{EpochStart, Member, NetworkFile};
use crate::rotation::{STAGED_NETWORK, STAGED_THRESHOLD};
use commonware_codec::DecodeExt;
use commonware_cryptography::bls12381::primitives::variant::MinSig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// The child's exit codes that no restart can fix; `aether run` ends with the
/// same code so the app shows the matching sentence and keeps the wallet on a
/// remote node. 3: the chain activated a newer protocol than this binary runs
/// (`main.rs` watch_upgrades). 5: protocol 2 is due and no proof verifier
/// could be installed (`main.rs` install_verifier).
pub const EXIT_UPGRADE_REQUIRED: i32 = 3;
pub const EXIT_NO_VERIFIER: i32 = 5;

/// The process exits with this code when another `aether` already holds this
/// data directory (red team #12): two apps must never run one node between
/// them. "Already running" is a state to surface, never a crash to restart.
pub const EXIT_LOCKED: i32 = 7;

/// A critical node task died. The child must not keep answering RPC as if it
/// were participating; unlike an untrusted vote journal this is restartable.
pub const EXIT_FATAL_TASK: i32 = 9;
/// A marshal cache failed to open. The supervisor quarantines only that
/// rebuildable cache before restarting the child.
pub const EXIT_REBUILDABLE_CACHE: i32 = 10;
/// Data-volume floor reached: the supervisor waits for free space before
/// restarting the child. No consensus journal writes happen while it waits.
pub const EXIT_DISK_LOW: i32 = 12;
/// `--chain-data` names a directory that does not exist (an unplugged disk
/// leaves its /Volumes path missing). The node never creates it: doing so
/// would quietly re-sync the whole chain onto the internal disk under a
/// /Volumes name. The app shows "the disk is not connected" and starts the
/// node again when the volume returns.
pub const EXIT_CHAIN_DATA_MISSING: i32 = 13;
/// Keys found where they must never be: in the chain-data directory (a
/// secondary disk that can be unplugged, lost or shared), or `--data` (the
/// key directory) on a removable or network volume. "keys must stay on this
/// Mac": the node refuses to start and never reads keys from there.
pub const EXIT_KEYS_ON_CHAIN_DATA: i32 = 14;

/// Files that are this Mac's identity: they live in `--data` (the key
/// directory, the internal disk) and nowhere else.
pub const KEY_FILES: [&str; 6] = [
    "validator.key", "validator.pub.json", "node-account.key", "threshold.json", "wallet-node.key", "key-binding.json",
];

/// Key files at the top level of the chain-data directory or of its
/// `follow`/`archive` subdirectories.
pub fn keys_in_chain_data(chain: &Path) -> Vec<PathBuf> {
    ["", "follow", "archive"]
        .iter()
        .map(|sub| if sub.is_empty() { chain.to_path_buf() } else { chain.join(sub) })
        .flat_map(|dir| KEY_FILES.iter().map(move |k| dir.join(k)))
        .filter(|p| p.exists())
        .collect()
}

/// Whether `--data` holding keys sits on a volume keys must not live on.
/// `external` is the volume test (`volume_is_external`), passed in so the
/// rule is a unit test.
pub fn keys_on_external_data(data: &Path, external: bool) -> bool {
    external && KEY_FILES.iter().any(|k| data.join(k).exists())
}

/// macOS: a path under /Volumes/ or on a volume statfs does not call local.
pub fn volume_is_external(path: &Path) -> bool {
    if path.starts_with("/Volumes/") { return true; }
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else { return false };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 { return false; }
    (st.f_flags as u64 & libc::MNT_LOCAL as u64) == 0
}
pub const CACHE_REPAIR_REQUEST: &str = "cache-repair-request";

fn rebuildable_panic(message: &str) -> Option<&str> {
    let name = message.strip_prefix("failed to initialize ")?.strip_suffix(" archive")?;
    ["verified", "notarized", "certified", "notarizations", "finalizations"].contains(&name).then_some(name)
}

#[derive(Debug, PartialEq, Eq)]
enum PanicAction<'a> {
    WaitForDisk,
    RepairCache(&'a str),
    Restart,
}

fn panic_action<'a>(message: &'a str, source: &str, free: Option<u64>, min: u64) -> PanicAction<'a> {
    // Commonware's archive panic discards the I/O cause. Space, rather than
    // the message alone, decides whether an archive needs repair. A journal
    // write failure at ENOSPC must never be treated as journal corruption.
    if min > 0 {
        match free {
            Some(bytes) if bytes < min => return PanicAction::WaitForDisk,
            None => return PanicAction::WaitForDisk,
            _ => {}
        }
    }
    // `marshal/standard` uses the same "finalizations archive" panic for a
    // non-cache finalized store. Only core/cache.rs initializes the delivery
    // caches whose partitions can be rebuilt from peers.
    if !source.ends_with("marshal/core/cache.rs") {
        return PanicAction::Restart;
    }
    rebuildable_panic(message).map_or(PanicAction::Restart, PanicAction::RepairCache)
}

fn disk_wait_needed(code: Option<i32>, data_disk_low: bool) -> bool {
    code == Some(EXIT_DISK_LOW)
        || (data_disk_low && matches!(code, Some(crate::store::EXIT_STORAGE | EXIT_FATAL_TASK)))
}

fn probe_writer(mut writer: impl std::io::Write) -> std::io::Result<()> {
    writer.write_all(b"[aether] log writer alive\n")?;
    writer.flush()
}

fn require_writer_alive(writer: impl std::io::Write) {
    if let Err(e) = probe_writer(writer) {
        // This message may itself fail, but the exit is still observed by the
        // parent. Never leave a half-alive validator serving RPC.
        let _ = std::io::Write::write_all(&mut std::io::stdout(), format!("log writer failed: {e}\n").as_bytes());
        std::process::exit(EXIT_FATAL_TASK);
    }
}

fn write_cache_repair_request(data: &Path, cache: &str) -> std::io::Result<()> {
    // A pre-existing request (including a symlink) is never overwritten. In
    // particular, a malformed data directory must not turn a cache panic into
    // a write to a key, threshold share, or vote journal.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let path = data.join(CACHE_REPAIR_REQUEST);
    let mut file = options.open(&path)?;
    let result = std::io::Write::write_all(&mut file, cache.as_bytes())
        .and_then(|_| file.sync_all());
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn panic_free_disk(data: &Path) -> Option<u64> {
    #[cfg(test)]
    if let Ok(value) = std::env::var("AETHER_TEST_PANIC_FREE_DISK") {
        return value.parse().ok();
    }
    crate::resources::free_disk(data)
}

/// Only a regular stderr file has a volume we can measure. Pipes and sockets
/// are still covered by the writer probe, but their destination is elsewhere.
#[cfg(unix)]
fn free_log_disk() -> Option<u64> {
    let mut file: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(libc::STDERR_FILENO, &mut file) } != 0
        || file.st_mode & libc::S_IFMT != libc::S_IFREG
    {
        return None;
    }
    let mut fs: libc::statfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::fstatfs(libc::STDERR_FILENO, &mut fs) } == 0)
        .then(|| (fs.f_bavail as u64).saturating_mul(fs.f_bsize as u64))
}

#[cfg(not(unix))]
fn free_log_disk() -> Option<u64> { None }

/// Install in node/follow children before starting runtime tasks. Commonware
/// catches and logs task panics, leaving RPC alive; a process-wide hook makes
/// critical task loss observable to `aether run` instead. A direct stderr
/// probe also catches tracing-subscriber's swallowed writer errors.
pub fn install_fatal_watch(data: PathBuf) {
    std::panic::set_hook(Box::new(move |info| {
        let message = info.payload().downcast_ref::<String>().map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied()).unwrap_or("non-string panic");
        let _ = std::io::Write::write_all(&mut std::io::stderr(), format!("fatal node task panic: {message} at {:?}\n", info.location()).as_bytes());
        exit_for_fatal_panic(&data, message, info.location().map_or("", |loc| loc.file()));
    }));
    std::thread::Builder::new().name("log-writer-watch".into()).spawn(|| loop {
        std::thread::sleep(Duration::from_secs(15));
        require_writer_alive(std::io::stderr());
    }).expect("start log writer watch");
}

fn exit_for_fatal_panic(data: &Path, message: &str, source: &str) -> ! {
    let min = crate::resources::monitor()
        .map(|monitor| monitor.limits.min_free_disk)
        .unwrap_or(crate::resources::Limits::default().min_free_disk);
    match panic_action(message, source, panic_free_disk(data), min) {
        PanicAction::WaitForDisk => std::process::exit(EXIT_DISK_LOW),
        PanicAction::RepairCache(cache) => {
            if write_cache_repair_request(data, cache).is_ok() {
                std::process::exit(EXIT_REBUILDABLE_CACHE);
            }
        }
        PanicAction::Restart => {}
    }
    std::process::exit(EXIT_FATAL_TASK);
}

const CACHE_QUARANTINE_KEEP: usize = 3;
const CACHE_QUARANTINE_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Only marshal's per-epoch delivery caches are rebuildable from peer blocks.
/// Archive names and partition suffixes are fixed by commonware's cache.rs.
fn cache_partition(name: &str, cache: &str, prefix: &str) -> bool {
    const CACHES: [&str; 5] = ["verified", "notarized", "certified", "notarizations", "finalizations"];
    const PARTS: [&str; 3] = ["metadata", "key", "value"];
    CACHES.iter().any(|kind| {
        if kind != &cache { return false; }
        PARTS.iter().any(|part| {
            let suffix = format!("-{kind}-{part}");
            let Some(stem) = name.strip_suffix(&suffix) else { return false };
            let Some((partition, epoch)) = stem.rsplit_once("-cache-") else { return false };
            partition == prefix && epoch.parse::<u64>().is_ok()
        })
    })
}

fn dir_size(path: &Path) -> Result<u64, String> {
    let mut size = 0u64;
    for item in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        let meta = std::fs::symlink_metadata(item.path()).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() { return Err("symlink inside marshal quarantine".into()); }
        size = size.saturating_add(if meta.is_dir() { dir_size(&item.path())? } else { meta.len() });
    }
    Ok(size)
}

fn prune_cache_quarantine(root: &Path) -> Result<(), String> {
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name().to_string_lossy().starts_with("pending-") { continue; }
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            dirs.push(entry);
        }
    }
    dirs.sort_by_key(|e| e.file_name());
    let mut total = dirs.iter().try_fold(0u64, |sum, d| dir_size(&d.path()).map(|n| sum.saturating_add(n)))?;
    while dirs.len() > CACHE_QUARANTINE_KEEP || total > CACHE_QUARANTINE_MAX_BYTES {
        let old = dirs.remove(0);
        let bytes = dir_size(&old.path())?;
        std::fs::remove_dir_all(old.path()).map_err(|e| e.to_string())?;
        total = total.saturating_sub(bytes);
    }
    Ok(())
}

/// Consume a child's exact repair request. A missing or forged request cannot
/// cause a journal, identity, share or state store to be moved.
fn repair_cache(data: &Path) -> Result<bool, String> {
    repair_cache_with(data, |from, to| std::fs::rename(from, to))
}

fn repair_cache_with(
    data: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<bool, String> {
    let request = data.join(CACHE_REPAIR_REQUEST);
    let request_meta = match std::fs::symlink_metadata(&request) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    if !request_meta.is_file() || request_meta.file_type().is_symlink() || request_meta.len() > 32 {
        return Err("cache repair request is not a regular file".into());
    }
    let cache = match std::fs::read_to_string(&request) {
        Ok(s) => s,
        Err(e) => return Err(e.to_string()),
    };
    let cache = cache.trim();
    if !["verified", "notarized", "certified", "notarizations", "finalizations"].contains(&cache) {
        return Err(format!("unrecognized cache repair request: {cache}"));
    }
    let prefix = match std::fs::read_to_string(data.join("partition")) {
        Ok(prefix) if !prefix.trim().is_empty() => prefix.trim().to_owned(),
        Ok(_) => return Err("empty partition prefix".into()),
        // The node also accepts older data dirs without a partition marker.
        // Use the same legacy lookup as main::partition_prefix so repair can
        // find the archive that actually failed in those directories.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::read_dir(data).map_err(|e| e.to_string())?
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .find_map(|name| name.strip_suffix("-blocks-metadata").map(str::to_owned))
                .unwrap_or_else(|| "aether".into())
        }
        Err(e) => return Err(e.to_string()),
    };
    let mut parts = Vec::new();
    for entry in std::fs::read_dir(data).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name().to_str().is_some_and(|n| cache_partition(n, cache, &prefix)) {
            if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                return Err("marshal cache partition is not a real directory".into());
            }
            parts.push(entry);
        }
    }
    let root = data.join("quarantine").join("marshal-cache");
    for parent in [data.join("quarantine"), root.clone()] {
        match std::fs::symlink_metadata(&parent) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err("marshal quarantine path is not a real directory".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&parent).map_err(|e| e.to_string())?;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    // A fixed pending directory makes each rename resumable after a crash or
    // an injected I/O failure. The request remains until all parts are moved.
    let pending = root.join(format!("pending-{cache}"));
    match std::fs::symlink_metadata(&pending) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
        Ok(_) => return Err("pending marshal quarantine is not a real directory".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if parts.is_empty() {
                // A crash after the final directory rename but before request
                // removal leaves a harmless stale request. Verify that a
                // completed batch of this cache exists before clearing it.
                let completed = std::fs::read_dir(&root).map_err(|e| e.to_string())?
                    .filter_map(Result::ok)
                    .filter(|e| e.file_name().to_string_lossy().chars().next().is_some_and(|c| c.is_ascii_digit()))
                    .any(|batch| std::fs::read_dir(batch.path()).ok().is_some_and(|entries| {
                        entries.filter_map(Result::ok).any(|part| part.file_name().to_str()
                            .is_some_and(|name| cache_partition(name, cache, &prefix)))
                    }));
                if !completed { return Err(format!("no {cache} marshal cache partition to repair")); }
                std::fs::remove_file(&request).map_err(|e| e.to_string())?;
                return Ok(true);
            }
            std::fs::create_dir(&pending).map_err(|e| e.to_string())?;
        }
        Err(e) => return Err(e.to_string()),
    }
    let mut moved = false;
    for entry in std::fs::read_dir(&pending).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir()
            || !entry.file_name().to_str().is_some_and(|n| cache_partition(n, cache, &prefix)) {
            return Err("unexpected entry in pending marshal quarantine".into());
        }
        moved = true;
    }
    for part in parts {
        rename(&part.path(), &pending.join(part.file_name())).map_err(|e| e.to_string())?;
        moved = true;
    }
    if !moved { return Err(format!("no {cache} marshal cache partition to repair")); }
    let aside = root.join(format!("{:020}-{}", now_ms(), std::process::id()));
    rename(&pending, &aside).map_err(|e| e.to_string())?;
    std::fs::remove_file(request).map_err(|e| e.to_string())?;
    prune_cache_quarantine(&root)?;
    tracing::warn!(cache, path = %aside.display(), "quarantined rebuildable marshal cache; restarting to refetch from peers");
    Ok(true)
}

/// Written when a validator refuses to resume voting (the engine's
/// [`crate::engine::EXIT_JOURNAL`], red team #4): it holds the committee
/// round the refusal belongs to. Voting stays off for that round — a lost
/// journal is never voted over with a fresh one — and returns with the next
/// round's share, which comes with a fresh journal.
const NO_VOTE_FILE: &str = "no-vote";

/// Record that voting must not resume for the committee's current `round`
/// (the child refused with [`crate::engine::EXIT_JOURNAL`]).
fn note_untrusted_journal(data: &Path, round: u64) {
    if let Ok(old) = std::fs::read_to_string(data.join(NO_VOTE_FILE)) {
        if old.trim().parse::<u64>().is_ok_and(|r| r >= round) {
            return; // an earlier, still-binding refusal
        }
    }
    let _ = crate::atomic::replace(&data.join(NO_VOTE_FILE), round.to_string().as_bytes(), 0o600);
}

/// Whether voting is off for the committee's current `round`.
fn voting_paused(data: &Path, round: u64) -> bool {
    std::fs::read_to_string(data.join(NO_VOTE_FILE))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .is_some_and(|r| r >= round)
}

pub struct Supervisor {
    /// The `aether` binary to run children with.
    pub exe: PathBuf,
    /// Keys, threshold.json, network.json and the validator's state; the
    /// follower keeps its state in `<data>/follow`.
    pub data: PathBuf,
    /// Validator p2p port.
    pub port: u16,
    /// Background reshare port (a running validator's node forwards reshare
    /// links over iroh to `port + 1`, so keep that default on public networks).
    pub reshare_port: u16,
    pub rpc_port: u16,
    /// Extra args for `aether node` (faucet, DeviceCheck, block time, …).
    pub node_args: Vec<String>,
    /// Extra args for `aether follow` (e.g. `--from-rpc`).
    pub follow_args: Vec<String>,
    /// Local tests: nodes talk plain TCP on loopback; each writes its p2p port
    /// to `<dir>/<voting key hex>` so the others can find it.
    pub dev_peer_dir: Option<PathBuf>,
    /// Explicit background reshare deadline. None derives it from the child
    /// protocol bound and the proposed player count.
    pub reshare_timeout: Option<Duration>,
    /// The ceremony record the child validator binds to at startup (audit 6).
    /// None falls back to `<data>/ceremony-check.json`, where verify-local
    /// stores it.
    pub ceremony: Option<PathBuf>,
    /// Where the bulky chain data lives (`--chain-data`, a secondary disk):
    /// the follower's `follow` (or `archive`) directory goes there; keys,
    /// threshold, network.json, run.lock, run-state.json and the validator's
    /// journals stay in `data`. None: everything in `data`.
    pub chain_data: Option<PathBuf>,
    /// `--archive`: the follower child keeps the full history like `aether
    /// archive` (replay from genesis, never a snapshot jump, era export),
    /// in `<chain dir>/archive`, apart from the normal `follow` directory.
    pub archive: bool,
}

/// The child `aether run` forwards SIGUSR1 to (0: none running).
static WAKE_TARGET: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// The Mac app sends SIGUSR1 to `aether run` on every power-source change,
/// sleep and wake, to wake the beacon loop (`candidate::beacon_loop`). Only
/// the children handle it; left at its default, the signal killed the
/// supervisor itself without a log line. Install before anything else in
/// `aether run`: the handler only forwards the signal to the current child.
pub fn install_wake_forwarding() {
    extern "C" fn forward(_: libc::c_int) {
        // Async-signal-safe: one atomic load and kill(2).
        let pid = WAKE_TARGET.load(std::sync::atomic::Ordering::Relaxed);
        if pid > 0 {
            unsafe { libc::kill(pid, libc::SIGUSR1) };
        }
    }
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = forward as extern "C" fn(libc::c_int) as libc::sighandler_t;
        // SA_RESTART: a wake must not fail the supervisor's waits with EINTR.
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGUSR1, &action, std::ptr::null_mut());
    }
}

/// The child that receives forwarded wakes (`None`: it exited).
pub fn set_wake_target(pid: Option<u32>) {
    WAKE_TARGET.store(pid.map_or(0, |p| p as i32), std::sync::atomic::Ordering::Relaxed);
}

/// The follower child's data directory: `<chain dir>/follow`, or
/// `<chain dir>/archive` in archive mode, where the chain dir is
/// `--chain-data` when set and `--data` otherwise.
pub fn follower_dir(data: &Path, chain_data: Option<&Path>, archive: bool) -> PathBuf {
    chain_data.unwrap_or(data).join(if archive { "archive" } else { "follow" })
}

/// The follower child's argv after the binary (`aether follow ...`), one
/// pure function so every mode is a unit test:
/// - default: `follow --exit-with-parent --network N --data <data>/follow
///   --rpc-port P --checkpoint [--keys <data> --candidate] <follow args>`;
/// - `--chain-data C`: the same with `--data C/follow`;
/// - `--archive`: `--data <chain dir>/archive`, no `--checkpoint`, and
///   `--archive-export <chain dir>/archive/era` (follow then runs exactly as
///   `aether archive` does: archive history mode, no snapshot start, no
///   snapshot jump, era files exported).
pub fn follower_args(
    network: &Path,
    data: &Path,
    chain_data: Option<&Path>,
    archive: bool,
    rpc_port: u16,
    candidate: bool,
    follow_args: &[String],
) -> Vec<String> {
    let dir = follower_dir(data, chain_data, archive);
    let mut out: Vec<String> = vec![
        "follow".into(),
        "--exit-with-parent".into(),
        "--network".into(),
        path_str(network),
        "--data".into(),
        path_str(&dir),
        "--rpc-port".into(),
        rpc_port.to_string(),
    ];
    if archive {
        // No `--checkpoint`: an archive owns every block from genesis.
        out.extend(["--archive-export".into(), path_str(&dir.join("era"))]);
    } else {
        out.push("--checkpoint".into());
    }
    if chain_data.is_some() || archive {
        // The wallet endpoint key stays exactly where it always was, on the
        // internal disk: the node id must not change with the chain's disk.
        out.extend(["--node-key".into(), path_str(&data.join("follow").join("wallet-node.key"))]);
    }
    if candidate {
        // The candidate's beacon keys are the Mac's own; the keyless
        // follower has none to send.
        out.extend(["--keys".into(), path_str(data), "--candidate".into()]);
    }
    out.extend(follow_args.iter().cloned());
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Validator,
    Candidate,
    /// This Mac has its key and former share, but its vote journal is
    /// untrusted in the current round. It follows without beacons or votes;
    /// the share may still help reshare into the next round.
    Paused,
    /// A follower without this Mac's identity: the key file is unreadable
    /// (red team #5), so it neither votes nor candidates — the wallet and the
    /// follower keep working until the key is restored.
    Keyless,
}

/// A background reshare for one proposed voting set.
struct Reshare {
    child: Child,
    started: Instant,
    timeout: Duration,
    proposal: Value,
    attempt: u64,
    retried: bool,
}

const RESHARE_RETURN_MARGIN: Duration = Duration::from_secs(15);
/// Leave time for both the full-roster child and, if a seat fails readiness,
/// one reduced-roster retry before the next proposal window.
const RESHARE_ATTEMPT_MARGIN: Duration = Duration::from_secs(60);

/// Allow the strict child to reach its last view, then wait through the full
/// share-readiness period, including the staged child's transcript relay.
pub fn default_reshare_timeout(players: usize) -> Duration {
    crate::dkg::Timeouts::default()
        .strict_return_bound(players)
        .saturating_add(crate::dkg::SHARE_READY_WINDOW)
        .saturating_add(RESHARE_RETURN_MARGIN)
}

fn effective_reshare_timeout(configured: Option<Duration>, players: usize, strict: bool) -> Duration {
    if strict {
        let minimum = default_reshare_timeout(players);
        configured.unwrap_or(minimum).max(minimum)
    } else {
        configured.unwrap_or(Duration::from_secs(300))
    }
}

pub(crate) fn reshare_attempt_players(current: usize, reserve: usize) -> usize {
    // Draws may change the proposed roster inside one window. Bound the
    // child deadline from the running committee, which stays fixed until a
    // handoff: a draw adds at most a third, plus any reserve seats.
    let growth = (current.saturating_sub(1) / 3).max(1);
    current.saturating_add(growth).saturating_add(reserve)
}

/// Number of one-second-or-slower blocks in two ceremonies plus a start margin.
pub(crate) fn reshare_attempt_blocks(players_bound: usize) -> Result<u64, String> {
    let window_ms = default_reshare_timeout(players_bound)
        .saturating_mul(2)
        .saturating_add(RESHARE_ATTEMPT_MARGIN)
        .as_millis();
    let blocks = window_ms.div_ceil(u128::from(crate::application::MIN_BLOCK_INTERVAL_MS));
    Ok(u64::try_from(blocks).map_err(|_| "reshare attempt window overflow")?.max(1))
}

pub(crate) fn reshare_attempt(height: u64, committee_start: u64, players_bound: usize) -> Result<u64, String> {
    let elapsed = height.checked_sub(committee_start).ok_or("rotation predates this committee")?;
    Ok(elapsed / reshare_attempt_blocks(players_bound)?)
}

fn reshare_round(old_round: u64, attempt: u64) -> Result<u64, String> {
    old_round.checked_add(attempt).and_then(|n| n.checked_add(1))
        .ok_or_else(|| "reshare round overflow".into())
}

/// The finalized proposal's draw identifies the DKG attempt. A supervisor's
/// sampled head may cross a ceremony-window edge while the proposal stays the
/// same; using that head would give honest players different signed rounds.
fn reshare_proposal_attempt(rot: &Value, strict: bool) -> Option<u64> {
    let draw = rot["epoch"].as_u64()?;
    if strict { draw.checked_mul(2) } else { Some(draw) }
}

/// What watching the child ended with.
enum Watched {
    /// A handoff was installed: the restart is the role change itself.
    Switched,
    /// The child exited on its own.
    Exited(std::process::ExitStatus),
}

/// One child exit, as the supervisor persists it (red team #1): the backoff
/// and the stop decision come from this history in `<data>/run-state.json`,
/// so even a restart of `aether run` itself does not reset the clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExitNote {
    /// When this child was started (unix ms).
    pub started_ms: u64,
    /// When it exited (unix ms).
    pub at_ms: u64,
    /// Its exit code, or `None` when a signal killed it.
    pub code: Option<i32>,
}

/// What the supervisor does after a child exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Start again after this long (the backoff doubles per quick exit).
    Again(Duration),
    /// Do not restart: end `aether run` with this code, so the app shows the
    /// matching sentence and keeps the wallet on a remote node.
    Stop(i32),
}

/// The window of "10분에 3번 넘게 죽으면 멈춤": more exits than
/// [`MAX_EXITS_IN_WINDOW`] inside it ends the run.
const WINDOW_MS: u64 = 10 * 60 * 1_000;
const MAX_EXITS_IN_WINDOW: usize = 3;
// A follower stalls for ten minutes before each exit, so the ordinary ten
// minute crash window can never catch a persistent transport failure.
const STALL_WINDOW_MS: u64 = 60 * 60 * 1_000;
/// A child that lived this long made progress; its exit does not deepen the
/// backoff (a real crash loop never gets here).
const PROGRESS_MS: u64 = 5 * 60 * 1_000;
const FIRST_BACKOFF_MS: u64 = 1_000;
const MAX_BACKOFF_MS: u64 = 60_000;

/// The decision after a child exited, over the persisted history (red team
/// #1): an exit code a restart cannot change ends the run with that code;
/// too many exits inside the window end it with the last code; anything else
/// backs off, doubling per consecutive quick exit up to a minute.
pub fn next_restart(exits: &[ExitNote], now_ms: u64) -> Next {
    let Some(last) = exits.last() else { return Next::Again(Duration::from_millis(FIRST_BACKOFF_MS)) };
    match last.code {
        Some(code @ (EXIT_UPGRADE_REQUIRED | crate::store::EXIT_STORAGE | EXIT_NO_VERIFIER | crate::candidate::EXIT_IDENTITY | EXIT_LOCKED)) => {
            return Next::Stop(code);
        }
        _ => {}
    }
    let recent = exits.iter().filter(|e| now_ms.saturating_sub(e.at_ms) <= WINDOW_MS).count();
    if recent > MAX_EXITS_IN_WINDOW {
        return Next::Stop(last.code.unwrap_or(1));
    }
    let stalled = exits.iter().filter(|e| e.code == Some(crate::follow::EXIT_STALLED)
        && now_ms.saturating_sub(e.at_ms) <= STALL_WINDOW_MS).count();
    if last.code == Some(crate::follow::EXIT_STALLED) && stalled > MAX_EXITS_IN_WINDOW {
        return Next::Stop(crate::follow::EXIT_STALLED);
    }
    let quick = exits
        .iter()
        .rev()
        .take_while(|e| e.at_ms.saturating_sub(e.started_ms) < PROGRESS_MS)
        .count();
    Next::Again(Duration::from_millis((FIRST_BACKOFF_MS << quick.saturating_sub(1).min(6)).min(MAX_BACKOFF_MS)))
}

/// The format `run-state.json` carries. A newer format is refused, not
/// adopted as empty: reading it as no history would reset the restart
/// budget — the same reason an unreadable file is an error below.
const RUN_STATE_VERSION: u32 = 1;

/// The persisted history, in a versioned envelope.
#[derive(serde::Serialize, serde::Deserialize)]
struct RunStateRecord {
    v: u32,
    exits: Vec<ExitNote>,
}

/// What `run-state.json` can be on disk: the envelope, or the bare array
/// this supervisor wrote before the envelope existed.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum RunStateFile {
    Record(RunStateRecord),
    Legacy(Vec<ExitNote>),
}

/// `<data>/run-state.json`: an unreadable history cannot be treated as a
/// fresh one, since that would let a crash loop reset its restart budget.
fn load_exits(data: &Path) -> Result<Vec<ExitNote>, String> {
    match std::fs::read(data.join("run-state.json")) {
        Ok(bytes) => match serde_json::from_slice::<RunStateFile>(&bytes) {
            Ok(RunStateFile::Record(r)) if r.v > RUN_STATE_VERSION => Err(format!(
                "run-state.json: format {} is newer than this binary knows ({}); update instead of resetting the restart budget",
                r.v, RUN_STATE_VERSION
            )),
            Ok(RunStateFile::Record(r)) => Ok(r.exits),
            Ok(RunStateFile::Legacy(exits)) => Ok(exits),
            Err(e) => Err(format!("run-state.json: {e}")),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("run-state.json: {e}")),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// What [`Supervisor::install`] reads out of the old world: the block the new
/// epoch anchors on, and the proof of it a joining Mac starts from. Both are
/// the child's to give, and the swap itself runs with that child already
/// stopped (no signer may see a half-swapped share and pointer) — a stopped
/// child answers no RPC, so the reads happen first, while it is still up.
struct Anchor {
    end_hash: String,
    finalized: Value,
}

impl Anchor {
    fn read(rpc: &str, switch: u64) -> Result<Self, String> {
        let end = switch.checked_sub(1).ok_or("handoff switch")?;
        let end_hash = rpc_call(rpc, "aether_getBlock", json!([end]))?["hash"]
            .as_str()
            .ok_or("no block before the switch")?
            .to_string();
        Ok(Self {
            end_hash,
            finalized: rpc_call(rpc, "aether_getFinalized", json!([end]))?,
        })
    }
}

impl Supervisor {
    fn min_free_disk(&self) -> u64 {
        self.node_args.iter().chain(&self.follow_args)
            .find_map(|arg| arg.strip_prefix("--min-free-disk="))
            .and_then(|s| if s == "auto" { None } else { crate::resources::parse_size(s).ok() })
            .unwrap_or(crate::resources::Limits::default().min_free_disk)
    }

    fn disk_low(&self) -> bool {
        let min = self.min_free_disk();
        min > 0 && crate::resources::free_disk(self.disk_dir()).is_none_or(|free| free < min)
    }

    fn log_disk_low(&self) -> bool {
        let min = self.min_free_disk();
        min > 0 && free_log_disk().is_some_and(|free| free < min)
    }

    fn wait_for_log_disk(&self, recovering: bool) {
        let min = self.min_free_disk();
        if min == 0 { return; }
        let resume = if recovering { min.saturating_add(crate::resources::DISK_RESUME) } else { min };
        let mut warned = false;
        while let Some(free) = free_log_disk() {
            if free >= resume { break; }
            if !warned {
                tracing::warn!(free, resume, "log volume almost full: waiting before starting the node child");
                warned = true;
            }
            std::thread::sleep(Duration::from_secs(15));
        }
        if warned { tracing::info!("log volume space recovered: restarting node child"); }
    }

    fn wait_for_disk(&self, recovering: bool) {
        self.exit_if_chain_data_missing();
        wait_for_data_disk(self.disk_dir(), self.min_free_disk(), recovering);
    }

    /// The volume the disk floor guards: the chain data's when it lives on
    /// a disk of its own, else the data directory's.
    pub fn disk_dir(&self) -> &Path {
        self.chain_data.as_deref().unwrap_or(&self.data)
    }

    /// `--chain-data` is set and does not exist (the disk is unplugged).
    pub fn chain_data_missing(&self) -> bool {
        self.chain_data.as_deref().is_some_and(|c| !c.is_dir())
    }

    /// An unplugged chain disk ends the run with its own code: waiting on
    /// a path that is gone would look like a full disk, and creating it
    /// would fill the internal one.
    fn exit_if_chain_data_missing(&self) {
        if self.chain_data_missing() {
            tracing::error!(chain_data = %self.disk_dir().display(), "the chain data disk is not connected; stopping (nothing written)");
            std::process::exit(EXIT_CHAIN_DATA_MISSING);
        }
    }

    /// The follower child's directory (see `follower_dir`).
    pub fn follow_dir(&self) -> PathBuf {
        follower_dir(&self.data, self.chain_data.as_deref(), self.archive)
    }

    fn network_path(&self) -> PathBuf {
        self.data.join("network.json")
    }

    fn rpc(&self) -> String {
        format!("http://127.0.0.1:{}", self.rpc_port)
    }

    /// The old world's anchor for a handoff this Mac is about to install: read
    /// now, from the child that holds it, because the install stops that child
    /// first and then changes its share and pointer on disk.
    fn anchor(&self, h: &Value) -> Result<Anchor, String> {
        Anchor::read(&self.rpc(), h["switch"].as_u64().ok_or("handoff switch")?)
    }

    /// This Mac's voting key hex, or `None` when the key file cannot be read.
    /// Losing the key never mints a new identity (red team #5): the caller
    /// keeps the Mac following instead.
    fn my_key(&self) -> Option<String> {
        match crate::candidate::CandidateKeys::load_or_create(&self.data) {
            Ok(keys) => Some(hex::encode(keys.validator_key())),
            Err(e) => {
                tracing::error!(
                    %e,
                    role = "follower",
                    "aether run: this Mac's identity files cannot be read, so it does not vote and \
                     does not candidate; restore the files in {} from a backup to vote again",
                    self.data.display(),
                );
                None
            }
        }
    }

    fn role(&self, me: Option<&str>) -> Result<Role, String> {
        let net = NetworkFile::load(&self.network_path())?;
        let Some(me) = me else { return Ok(Role::Keyless) };
        if voting_paused(&self.data, net.round) {
            // The engine refused this round's journal (red team #4): the Mac
            // follows until the committee moves on.
            return Ok(Role::Paused);
        }
        let member = net.validators.iter().any(|m| m.key == me);
        Ok(if member && self.data.join("threshold.json").exists() {
            Role::Validator
        } else {
            Role::Candidate
        })
    }

    /// `index@127.0.0.1:port` for every other member of `validators` that
    /// published its port (dev only; real networks use iroh node ids).
    fn tcp_peers(&self, validators: &[Member], me: &str, reshare: bool) -> Option<String> {
        let dir = self.dev_peer_dir.as_ref()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while validators.iter().any(|m| !dir.join(&m.key).exists()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
        }
        let list: Vec<String> = validators
            .iter()
            .enumerate()
            .filter(|(_, m)| m.key != me)
            .filter_map(|(i, m)| {
                let ports = std::fs::read_to_string(dir.join(&m.key)).ok()?;
                let mut it = ports
                    .split_whitespace()
                    .filter_map(|p| p.parse::<u16>().ok());
                let (p2p, resh) = (it.next()?, it.next()?);
                Some(format!(
                    "{}@127.0.0.1:{}",
                    i + 1,
                    if reshare { resh } else { p2p }
                ))
            })
            .collect();
        Some(list.join(","))
    }

    fn spawn(&self, role: Role, me: Option<&str>) -> Result<Child, String> {
        let mut cmd = Command::new(&self.exe);
        let net = self.network_path();
        match role {
            Role::Validator => {
                cmd.args([
                    "node",
                    "--exit-with-parent",
                    "--network",
                    &path_str(&net),
                    "--data",
                    &path_str(&self.data),
                ])
                .args([
                    "--port",
                    &self.port.to_string(),
                    "--rpc-port",
                    &self.rpc_port.to_string(),
                ]);
                if let Some(rec) = &self.ceremony {
                    cmd.args(["--ceremony", &path_str(rec)]);
                }
                if let Some(peers) = self.tcp_peers(&NetworkFile::load(&net)?.validators, me.expect("a validator has its key"), false)
                {
                    cmd.args(["--peers", &peers, "--offline"]);
                }
                cmd.args(&self.node_args);
            }
            Role::Candidate | Role::Paused | Role::Keyless => {
                cmd.args(follower_args(
                    &net,
                    &self.data,
                    self.chain_data.as_deref(),
                    self.archive,
                    self.rpc_port,
                    role == Role::Candidate,
                    &self.follow_args,
                ));
            }
        }
        tracing::info!(?role, "aether run: starting");
        cmd.spawn()
            .map_err(|e| format!("spawn {}: {e}", self.exe.display()))
    }

    /// Runs forever: (re)start the child for the current role; follow
    /// rotations. Restarts are bounded and backed off (red team #1): the exit
    /// history persists in `<data>/run-state.json`, a restart backs off
    /// 1 s → 60 s, and a child that keeps dying inside ten minutes ends
    /// `aether run` with the child's own exit code instead of looping.
    pub fn run(&self) -> Result<(), String> {
        // Key creation for a first run can write to this volume. Check it
        // before even loading the local identity, not only before children.
        self.wait_for_disk(false);
        self.wait_for_log_disk(false);
        let mut me = self.my_key();
        if let (Some(dir), Some(me)) = (&self.dev_peer_dir, &me) {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            std::fs::write(
                dir.join(me),
                format!("{} {}", self.port, self.reshare_port),
            )
            .map_err(|e| e.to_string())?;
        }
        let mut exits = load_exits(&self.data).unwrap_or_else(|e| {
            tracing::error!(%e, "restart history cannot be trusted; stopping");
            std::process::exit(crate::store::EXIT_STORAGE);
        });
        loop {
            self.wait_for_disk(false);
            self.wait_for_log_disk(false);
            // A completed handoff must be installed before any child reads
            // network.json or its share. If disk failure prevents that, keep
            // the validator stopped rather than starting with a mixed pair.
            if let Err(e) = finish_incomplete(&self.data) {
                tracing::error!(%e, "a completed handoff cannot be installed; keeping the signer stopped");
                std::process::exit(crate::store::EXIT_STORAGE);
            }
            if self.data.join(CACHE_REPAIR_REQUEST).exists() {
                // The parent may itself have stopped after the child wrote a
                // request or partway through the quarantine. Finish that
                // exact repair before opening the same corrupt archive again.
                let mut prospective = exits.clone();
                prospective.push(ExitNote { started_ms: now_ms(), at_ms: now_ms(), code: Some(EXIT_REBUILDABLE_CACHE) });
                if let Next::Stop(code) = next_restart(&prospective, now_ms()) {
                    return Err(format!("cache repair restart budget exhausted (exit {code})"));
                }
                match repair_cache(&self.data) {
                    Ok(true) => {}
                    Ok(false) => return Err("cache repair request disappeared before repair".into()),
                    Err(e) => return Err(format!("pending marshal cache repair: {e}")),
                }
            }
            let role = self.role(me.as_deref())?;
            let started_ms = now_ms();
            let mut child = self.spawn(role, me.as_deref())?;
            set_wake_target(Some(child.id()));
            let watched = self.watch(&mut child, role, me.as_deref());
            // The child may be reaped already: never signal a pid that can
            // be reused by an unrelated process.
            set_wake_target(None);
            match watched {
                Watched::Switched => {
                    // The restart is the role change itself, not a crash.
                    me = self.my_key();
                    exits.clear();
                    let _ = std::fs::remove_file(self.data.join("run-state.json"));
                }
                Watched::Exited(status) => {
                    // A sudden ENOSPC can beat the two-second resource sample.
                    // Storage exit 4 is retryable once this volume has room;
                    // other storage failures retain the existing stop policy.
                    let data_disk_low = self.disk_low();
                    let wait_for_space = disk_wait_needed(status.code(), data_disk_low);
                    if wait_for_space {
                        self.wait_for_disk(true);
                        // Waiting for space is the recovery action. Do not
                        // spend the crash budget or try to persist restart
                        // history onto the full volume.
                        continue;
                    }
                    if status.code() == Some(EXIT_FATAL_TASK) && self.log_disk_low() {
                        self.wait_for_log_disk(true);
                        continue;
                    }
                    if status.code() == Some(EXIT_REBUILDABLE_CACHE) {
                        // The panic requested a cache repair before another
                        // process filled the disk. Preserve the request until
                        // quarantine can create its destination safely.
                        if data_disk_low { self.wait_for_disk(true); }
                        // Refuse another destructive cache move once the persisted
                        // crash budget is exhausted, even if the request is valid.
                        let mut prospective = exits.clone();
                        prospective.push(ExitNote { started_ms, at_ms: now_ms(), code: status.code() });
                        if let Next::Stop(code) = next_restart(&prospective, now_ms()) {
                            tracing::error!(code, "marshal cache restart budget exhausted; preserving cache");
                            std::process::exit(code);
                        }
                        let mut repaired = false;
                        for attempt in 1..=3 {
                            if self.disk_low() { self.wait_for_disk(true); }
                            match repair_cache(&self.data) {
                                Ok(true) => { repaired = true; break; }
                                Ok(false) => {
                                    tracing::error!("cache repair exit had no request; stopping");
                                    break;
                                }
                                Err(e) => {
                                    tracing::error!(%e, attempt, "marshal cache repair attempt failed");
                                    std::thread::sleep(Duration::from_secs(5));
                                }
                            }
                        }
                        if !repaired {
                            tracing::error!("marshal cache repair could not complete; preserving pending quarantine and stopping");
                            std::process::exit(crate::store::EXIT_STORAGE);
                        }
                    }
                    exits.push(ExitNote { started_ms, at_ms: now_ms(), code: status.code() });
                    exits.retain(|e| now_ms().saturating_sub(e.at_ms) <= STALL_WINDOW_MS);
                    if let Err(e) = crate::atomic::replace(
                        &self.data.join("run-state.json"),
                        &serde_json::to_vec(&RunStateRecord { v: RUN_STATE_VERSION, exits: exits.clone() })
                            .unwrap_or_default(),
                        0o644,
                    ) {
                        tracing::error!(%e, "restart history could not be saved; stopping");
                        std::process::exit(crate::store::EXIT_STORAGE);
                    }
                    match next_restart(&exits, now_ms()) {
                        Next::Again(d) => {
                            tracing::warn!(code = ?status.code(), backoff_ms = d.as_millis() as u64, "aether run: child exited; restarting");
                            std::thread::sleep(d);
                        }
                        Next::Stop(code) => {
                            if code == crate::follow::EXIT_STALLED {
                                tracing::error!("follower could not make verified progress after four stalls within an hour; check upstream reachability and the preceding fetch errors");
                            }
                            tracing::error!(
                                code,
                                exits = exits.len(),
                                "aether run: the child keeps failing (or asked not to be restarted); stopping. \
                                 The wallet keeps working through other nodes; the app shows what to do"
                            );
                            std::process::exit(code);
                        }
                    }
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Watch the child; ends after installing a handoff (restart in the new
    /// role) or when the child exits on its own.
    fn watch(&self, child: &mut Child, role: Role, me: Option<&str>) -> Watched {
        let rpc = self.rpc();
        let strict_reshare = NetworkFile::load(&self.network_path()).is_ok_and(|net| net.chain_id != 7_780);
        let mut reshare: Option<Reshare> = None;
        let mut attempted: Option<u64> = None;
        let mut last_sign = Instant::now() - Duration::from_secs(60);
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if let Some(status) = child.try_wait().ok().flatten() {
                // An untrusted vote journal (red team #4) is not a crash to
                // retry: the next `aether node` would hit the same gate. Vote
                // off for this committee round instead; follow meanwhile.
                if status.code() == Some(crate::engine::EXIT_JOURNAL) {
                    match NetworkFile::load(&self.network_path()) {
                        Ok(net) => {
                            note_untrusted_journal(&self.data, net.round);
                            tracing::error!(
                                round = net.round,
                                "aether run: this Mac's vote journal cannot be trusted; following until the committee's next round"
                            );
                        }
                        Err(e) => tracing::warn!(%e, "aether run: could not read the round to pause voting for"),
                    }
                }
                tracing::warn!(%status, "aether run: child exited");
                stop(&mut reshare);
                return Watched::Exited(status);
            }
            if role == Role::Keyless
                && crate::candidate::CandidateKeys::load_or_create(&self.data).is_ok() {
                tracing::info!("aether run: restored identity is readable; re-evaluating the role");
                return Watched::Switched;
            }
            // 1. A proposed voting set: reshare to it in the background, once per finalized draw.
            if reshare.is_none() {
                if let Ok(rot) = rpc_call(&rpc, "aether_rotation", json!([])) {
                    let players = rot["next"].as_array().map_or(0, Vec::len);
                    let attempt = reshare_proposal_attempt(&rot, strict_reshare);
                    // Without this Mac's key there is no share to reshare and no
                    // seat to take (red team #5).
                    let involved = match me {
                        Some(me) => {
                            (role == Role::Validator || role == Role::Paused)
                                || rot["next"]
                                    .as_array()
                                    .is_some_and(|n| n.iter().any(|m| m["key"] == me))
                        }
                        None => false,
                    };
                    if let Some(attempt) = attempt.filter(|attempt| !rot.is_null() && involved && Some(*attempt) != attempted) {
                        attempted = Some(attempt);
                        match self.start_reshare(role, me.expect("involved means keyed"), &rot, attempt) {
                            Ok(c) => {
                                reshare = Some(Reshare {
                                    child: c,
                                    started: Instant::now(),
                                    timeout: effective_reshare_timeout(self.reshare_timeout, players, strict_reshare),
                                    proposal: rot.clone(),
                                    attempt,
                                    retried: false,
                                })
                            }
                            Err(e) => {
                                tracing::warn!(%e, "aether run: cannot reshare to the proposed set")
                            }
                        }
                    }
                }
            }
            let mut retry = None;
            if let Some(r) = reshare.as_mut() {
                match r.child.try_wait() {
                    Ok(Some(status)) => {
                        tracing::info!(%status, "aether run: background reshare finished");
                        if strict_reshare && status.success() && !r.retried {
                            retry = self.reduced_reshare_retry(r);
                        }
                        if strict_reshare && !status.success() {
                            let _ = std::fs::remove_file(self.data.join(STAGED_THRESHOLD));
                            let _ = std::fs::remove_file(self.data.join(STAGED_NETWORK));
                            let _ = std::fs::remove_file(self.data.join(crate::handoff::READY_FILE));
                        }
                        reshare = None;
                    }
                    _ if r.started.elapsed() > r.timeout => {
                        tracing::warn!(
                            "aether run: background reshare timed out; the running set carries on"
                        );
                        stop(&mut reshare);
                        if strict_reshare {
                            let _ = std::fs::remove_file(self.data.join(STAGED_THRESHOLD));
                            let _ = std::fs::remove_file(self.data.join(STAGED_NETWORK));
                            let _ = std::fs::remove_file(self.data.join(crate::handoff::READY_FILE));
                        }
                    }
                    _ => {}
                }
            }
            if let Some((proposal, attempt)) = retry {
                let next = proposal["next"].as_array().map_or(0, Vec::len);
                let seated = me.is_some_and(|key| proposal["next"].as_array()
                    .is_some_and(|members| members.iter().any(|member| member["key"] == key)));
                if role != Role::Candidate || seated {
                    match self.start_reshare(role, me.expect("a resharing member has its key"), &proposal, attempt) {
                        Ok(child) => {
                            reshare = Some(Reshare {
                                child,
                                started: Instant::now(),
                                timeout: effective_reshare_timeout(self.reshare_timeout, next, true),
                                proposal,
                                attempt,
                                retried: true,
                            });
                        }
                        Err(e) => tracing::warn!(%e, "aether run: cannot retry the reduced voting set"),
                    }
                }
            }
            // 2. A staged reshare: sign its handoff (again now and then, for peers that missed it).
            if role == Role::Validator
                && (!strict_reshare || reshare.is_none())
                && (if strict_reshare { self.data.join(crate::handoff::READY_FILE).exists() }
                    else { self.data.join(STAGED_THRESHOLD).exists() })
                && last_sign.elapsed() > Duration::from_secs(5)
            {
                last_sign = Instant::now();
                if let Err(e) = rpc_call(&rpc, "aether_signHandoff", json!([])) {
                    tracing::debug!(%e, "handoff not signed");
                }
            }
            // 3. A finalized handoff whose switch height the chain reached: install the new role.
            if let (Some(me), Ok(h)) = (me, rpc_call(&rpc, "aether_handoff", json!([]))) {
                if self.handoff_due(&h) {
                    if let Err(e) = self.check_joining_share(me, &h) {
                        tracing::warn!(%e, "aether run: cannot install a seated handoff without a usable share");
                        continue;
                    }
                    stop(&mut reshare);
                    // The install needs this from the old world, and only the
                    // child below has it: read before the stop.
                    let anchor = self.anchor(&h);
                    // The old validator must not keep signing while its share
                    // and network pointer are changed on disk.
                    let _ = child.kill();
                    let status = child.wait().expect("a stopped child can be reaped");
                    match anchor.and_then(|anchor| self.install(role, me, &h, &anchor)) {
                        Ok(()) => return Watched::Switched,
                        Err(e) => {
                            tracing::warn!(%e, "aether run: could not install the handoff");
                            return Watched::Exited(status);
                        }
                    }
                }
            }
        }
    }

    /// The handoff is new to this Mac and the chain finalized its last block before the switch.
    fn handoff_due(&self, h: &Value) -> bool {
        let (Some(round), Some(switch), Some(finalized)) = (
            h["round"].as_u64(),
            h["switch"].as_u64(),
            h["finalized"].as_u64(),
        ) else {
            return false;
        };
        let ours = NetworkFile::load(&self.network_path())
            .map(|n| n.round)
            .unwrap_or(0);
        round > ours && finalized + 1 >= switch
    }

    /// A new-genesis seat is installed only with its exact staged share,
    /// checked against the handoff polynomial and this Mac's voting key.
    fn check_joining_share(&self, me: &str, h: &Value) -> Result<(), String> {
        let current = NetworkFile::load(&self.network_path())?;
        if current.chain_id == 7_780 { return Ok(()); }
        let members: Vec<Member> = serde_json::from_value(h["members"].clone())
            .map_err(|e| format!("handoff members: {e}"))?;
        if !members.iter().any(|member| member.key == me) { return Ok(()); }
        let output_hex = h["output"].as_str().ok_or("handoff output")?;
        let round = h["round"].as_u64().ok_or("handoff round")?;
        let bytes = std::fs::read(self.data.join(STAGED_THRESHOLD))
            .map_err(|e| format!("no staged share for the new seat: {e}"))?;
        let key: crate::dkg::KeyFile = serde_json::from_slice(&bytes)
            .map_err(|e| format!("staged share: {e}"))?;
        if key.round != round || key.output != output_hex {
            return Err("staged share does not match the handoff".into());
        }
        let (output, share) = key.decode(members.len() as u32)?;
        let me = crate::block::PublicKey::decode(hex::decode(me).map_err(|e| e.to_string())?.as_slice())
            .map_err(|e| format!("local voting key: {e:?}"))?;
        if output.players().get(usize::from(share.index)) != Some(&me)
            || output.public().partial_public(share.index).ok() != Some(share.public::<MinSig>())
        {
            return Err("staged threshold share cannot sign for this seat".into());
        }
        if output.revealed().iter().any(|player| output.players().position(player).is_some()) {
            return Err("handoff reveals a seated player's threshold share".into());
        }
        Ok(())
    }

    /// The first round timed out with missing seats. Only a certified-output
    /// readiness record from that child can select a smaller retry roster;
    /// the retry gets the adjacent, strictly higher round for this proposal.
    fn reduced_reshare_retry(&self, r: &Reshare) -> Option<(Value, u64)> {
        let from: NetworkFile = serde_json::from_value(r.proposal["network"].clone()).ok()?;
        if from.chain_id == 7_780 { return None; }
        let record: crate::handoff::Readiness = serde_json::from_slice(
            &std::fs::read(self.data.join(crate::handoff::READY_FILE)).ok()?).ok()?;
        if record.round != reshare_round(from.round, r.attempt).ok()? { return None; }
        let proposed: Vec<Member> = serde_json::from_value(r.proposal["next"].clone()).ok()?;
        let proposed: Vec<_> = proposed.into_iter().map(|m| (m.key, m.node)).collect();
        if !aether_rewards::same_roster(&proposed, &record.members) { return None; }
        let reduced = match record.retry_members(from.chain_id) {
            Ok(Some(members)) => members,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(%error, "aether run: insufficient ready seats for a reduced reshare");
                return None;
            }
        };
        let mut proposal = r.proposal.clone();
        proposal["next"] = json!(reduced.iter().map(|(key, node)| json!({"key": key, "node": node})).collect::<Vec<_>>());
        let attempt = r.attempt.checked_add(1)?;
        tracing::warn!(round = reshare_round(from.round, attempt).ok()?, seats = reduced.len(),
            "aether run: retrying a fresh DKG round without unready seats");
        Some((proposal, attempt))
    }

    fn start_reshare(&self, role: Role, me: &str, rot: &Value, attempt: u64) -> Result<Child, String> {
        let ours = NetworkFile::load(&self.network_path())?;
        let from: NetworkFile = match role {
            Role::Validator | Role::Paused => ours.clone(),
            // Never reached (a keyless Mac is not `involved`), and a Mac with
            // no key has no share to reshare even if it were.
            Role::Keyless => return Err("a Mac without its key has no share to reshare".into()),
            Role::Candidate => {
                // The running set's public file, checked against the identity this Mac pins.
                let f: NetworkFile = serde_json::from_value(rot["network"].clone())
                    .map_err(|e| format!("rotation has no network: {e}"))?;
                if f.identity != ours.identity || f.chain_id != ours.chain_id {
                    return Err("the running set's network file is for another committee".into());
                }
                let out = f
                    .output
                    .clone()
                    .ok_or("the running set's network file has no output")?;
                let identity = aether_light::ValidatorSet::from_hex(
                    ours.identity.as_deref().unwrap_or_default(),
                )
                .map_err(|e| format!("{e:?}"))?;
                let decoded = crate::dkg::KeyFile {
                    round: f.round,
                    output: out,
                    identity: String::new(),
                    share: String::new(),
                }
                .decode_output(f.validators.len() as u32)?;
                if decoded.public().public() != identity.identity() {
                    return Err("the running set's output is for another identity".into());
                }
                f
            }
        };
        let next: Vec<Member> = serde_json::from_value(rot["next"].clone())
            .map_err(|e| format!("rotation next: {e}"))?;
        let to = NetworkFile {
            validators: next.clone(),
            identity: None,
            output: None,
            epochs: vec![],
            ..from.clone()
        };
        let (from_path, to_path) = (
            self.data.join("rotation-from.json"),
            self.data.join("rotation-to.json"),
        );
        std::fs::write(&from_path, serde_json::to_vec_pretty(&from).expect("json"))
            .map_err(|e| e.to_string())?;
        std::fs::write(&to_path, serde_json::to_vec_pretty(&to).expect("json"))
            .map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(self.data.join(STAGED_THRESHOLD));
        let _ = std::fs::remove_file(self.data.join(STAGED_NETWORK));
        if from.chain_id != 7_780 {
            let _ = std::fs::remove_file(self.data.join(crate::handoff::READY_FILE));
        }
        let mut cmd = Command::new(&self.exe);
        cmd.args([
            "reshare",
            "--stage",
            "--exit-with-parent",
            "--from",
            &path_str(&from_path),
            "--to",
            &path_str(&to_path),
        ]);
        if from.chain_id != 7_780 {
            // Failed ceremonies for later finalized draws must not replay the
            // same DKG round, signed logs, or durable randomness seed.
            let round = reshare_round(from.round, attempt)?;
            cmd.args(["--round", &round.to_string()]);
        }
        if role == Role::Validator {
            cmd.arg("--via-node");
        }
        cmd.args([
            "--port",
            &self.reshare_port.to_string(),
            "--data",
            &path_str(&self.data),
        ]);
        let union: Vec<Member> = from
            .validators
            .iter()
            .cloned()
            .chain(
                next.iter()
                    .filter(|m| !from.validators.contains(m))
                    .cloned(),
            )
            .collect();
        if let Some(peers) = self.tcp_peers(&union, me, true) {
            cmd.args(["--peers", &peers, "--offline"]);
        }
        tracing::info!(
            members = next.len(),
            "aether run: resharing to the proposed voting set in the background"
        );
        cmd.spawn().map_err(|e| e.to_string())
    }

    /// Take this Mac's role in the new voting set: files only; the caller restarts.
    ///
    /// Atomic across power cuts and full disks (red team #19): the whole
    /// generation — the network file and, for a seated member, its share — is
    /// prepared and synced in `<data>/gen/<round>/` first and marked complete
    /// (`.installed`); only then is it activated, by swapping the share and
    /// then `network.json` (the pointer every role decision reads against). A
    /// run that dies mid-install resumes it before a child starts
    /// ([`finish_incomplete`]); the old child is stopped before any active
    /// file changes, so no signer can observe an incomplete pair — and what
    /// the install needs from that child is read before it is stopped
    /// ([`Anchor`]).
    fn install(&self, role: Role, me: &str, h: &Value, anchor: &Anchor) -> Result<(), String> {
        self.check_joining_share(me, h)?;
        let switch = h["switch"].as_u64().ok_or("handoff switch")?;
        let round = h["round"].as_u64().ok_or("handoff round")?;
        let output = h["output"].as_str().ok_or("handoff output")?.to_string();
        let members: Vec<Member> = serde_json::from_value(h["members"].clone())
            .map_err(|e| format!("handoff members: {e}"))?;
        let end_hash = anchor.end_hash.clone();
        let ours = NetworkFile::load(&self.network_path())?;
        let mut next = NetworkFile {
            validators: members.clone(),
            identity: ours.identity.clone(),
            round,
            output: Some(output.clone()),
            ..ours.clone()
        };
        next.epochs.push(EpochStart {
            height: switch,
            parent: end_hash,
        });
        let staged: Option<crate::dkg::KeyFile> = std::fs::read(self.data.join(STAGED_THRESHOLD))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .filter(|k: &crate::dkg::KeyFile| k.output == output && k.round == round
                && next.identity.as_deref() == Some(k.identity.as_str()));
        let joining = members.iter().any(|m| m.key == me);

        // 1. Prepare the generation, before anything active changes. Anything
        //    that can fail (the proof a joining Mac starts from above all)
        //    fails here, with the old world still in place.
        let gen = self.data.join("gen").join(round.to_string());
        std::fs::create_dir_all(&gen).map_err(|e| e.to_string())?;
        crate::atomic::replace(
            &gen.join("network.json"),
            &serde_json::to_vec_pretty(&next).map_err(|e| e.to_string())?,
            0o644,
        )?;
        match &staged {
            Some(key) => crate::atomic::replace(
                &gen.join("threshold.json"),
                &serde_json::to_vec_pretty(key).map_err(|e| e.to_string())?,
                0o600,
            )?,
            None => {
                let _ = std::fs::remove_file(gen.join("threshold.json"));
            }
        }
        // 2. A joining candidate starts from the state it verified as a
        //    follower (and the anchor that proves it); marked, so a resumed
        //    install does not repeat the move.
        if joining && staged.is_some() && (role == Role::Candidate || role == Role::Paused) && !gen.join(".adopted").exists() {
            let fin = &anchor.finalized;
            if fin.is_null() {
                return Err("the follower has no proof of the block before the switch yet".into());
            }
            crate::atomic::replace(
                &self.data.join(crate::rotation::ANCHOR_FILE),
                fin.to_string().as_bytes(),
                0o644,
            )?;
            self.adopt_follower_state()?;
            crate::atomic::replace(&gen.join(".adopted"), b"", 0o644)?;
        }
        // 3. The generation is complete on disk: from here the install is
        //    resumable, never re-prepared.
        crate::atomic::replace(&gen.join(".installed"), b"", 0o644)?;

        // 4. Activate.
        activate_generation(&self.data, &gen)?;
        match (joining, staged.is_some()) {
            (true, true) => tracing::info!(switch, "aether run: this Mac votes from the switch height"),
            (true, false) => tracing::warn!(
                "aether run: in the new voting set without its share (the reshare did not finish here); following"
            ),
            (false, _) => tracing::info!(switch, "aether run: left the voting set; following"),
        }
        let _ = std::fs::remove_file(self.data.join(STAGED_THRESHOLD));
        let _ = std::fs::remove_file(self.data.join(STAGED_NETWORK));
        // A new round's share comes with a fresh journal: an old pause
        // (crate::engine::EXIT_JOURNAL) no longer binds.
        let _ = std::fs::remove_file(self.data.join(NO_VOTE_FILE));
        prune_generations(&self.data, round);
        Ok(())
    }

    /// Move a previous validator's storage aside and start from the follower's
    /// verified state (its node state and marshal archives would have a gap).
    /// The vote journal stays where it is (red team #4): it is the only record
    /// of the votes this Mac cast, and a stale epoch's journal is never reused
    /// anyway — the next round votes into its own partition.
    fn adopt_follower_state(&self) -> Result<(), String> {
        let prefix = std::fs::read_to_string(self.data.join("partition"))
            .map(|p| p.trim().to_string())
            .unwrap_or_else(|_| "aether".into());
        let stale: Vec<PathBuf> = std::fs::read_dir(&self.data)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n == "state.redb"
                        || (n.starts_with(&format!("{prefix}-"))
                            && !n.starts_with(&format!("{prefix}-consensus")))
                })
            })
            .collect();
        if !stale.is_empty() {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default();
            let aside = self.data.join(format!("stale-{secs}"));
            std::fs::create_dir_all(&aside).map_err(|e| e.to_string())?;
            for p in stale {
                let name = p.file_name().expect("listed entry has a name").to_owned();
                std::fs::rename(&p, aside.join(name)).map_err(|e| e.to_string())?;
            }
        }
        std::fs::copy(
            self.follow_dir().join("state.redb"),
            self.data.join("state.redb"),
        )
        .map_err(|e| format!("follower state: {e}"))?;
        Ok(())
    }

}

/// Also used by `aether run` before its first key, network, or lock write.
pub fn wait_for_data_disk(data: &Path, min: u64, recovering: bool) {
    if min == 0 { return; }
    let resume = if recovering { min.saturating_add(crate::resources::DISK_RESUME) } else { min };
    let mut warned = false;
    loop {
        let free = crate::resources::free_disk(data);
        if free.is_some_and(|bytes| bytes >= resume) { break; }
        if !warned {
            tracing::warn!(?free, resume, "disk almost full or unreadable: node stopped before writes; waiting for space");
            warned = true;
        }
        std::thread::sleep(Duration::from_secs(15));
    }
    if warned { tracing::info!("disk space recovered: resuming node startup"); }
}

/// Swap a prepared generation in (red team #19): the share first, then
/// `network.json`, the pointer every role decision reads against. A seated
/// member's new one, or none (a member that leaves drops its share, so no
/// quorum of forgotten old shares can sign). The child is stopped before this
/// starts, and the next run completes any interruption before spawning one.
fn activate_generation(data: &Path, gen: &Path) -> Result<(), String> {
    let network = std::fs::read(gen.join("network.json")).map_err(|e| e.to_string())?;
    let next = NetworkFile::load(&gen.join("network.json"))?;
    match std::fs::read(gen.join("threshold.json")) {
        Ok(share) => {
            let key: crate::dkg::KeyFile = serde_json::from_slice(&share)
                .map_err(|e| format!("prepared share: {e}"))?;
            if key.round != next.round || next.output.as_deref() != Some(key.output.as_str())
                || next.identity.as_deref() != Some(key.identity.as_str()) {
                return Err("prepared share does not match the generation's network".into());
            }
            crate::atomic::replace(&data.join("threshold.json"), &share, 0o600)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => erase(&data.join("threshold.json"))?,
        Err(e) => return Err(e.to_string()),
    }
    crate::atomic::replace(&data.join("network.json"), &network, 0o644)?;
    Ok(())
}

/// Whether the data dir does not yet hold `gen`'s world: the pointer, or the
/// share that world carries, differs. Activation swaps the share before the
/// pointer; a crash between them is precisely "share moved, pointer not".
/// Comparing both files also repairs a pair left by an older installer.
fn world_differs(data: &Path, gen: &Path) -> bool {
    let same = |a: std::path::PathBuf, b: std::path::PathBuf| std::fs::read(a).ok() == std::fs::read(b).ok();
    let share = |dir: &Path| dir.join("threshold.json");
    !same(data.join("network.json"), gen.join("network.json"))
        || if share(gen).exists() {
            !same(share(data), share(gen))
        } else {
            share(data).exists()
        }
}

/// Finish an install a power cut or a full disk interrupted (red team #19):
/// a generation marked complete (`.installed`) for a round at or newer than
/// the active network file, whose world the data dir does not hold yet, is
/// activated — the same swap, run again, idempotent. A generation without
/// the marker was never complete; the handoff that prepared it prepares it
/// again, so it is left alone.
pub fn finish_incomplete(data: &Path) -> Result<(), String> {
    let ours = NetworkFile::load(&data.join("network.json"))
        .map(|n| n.round)
        .unwrap_or(0);
    let mut gens = std::fs::read_dir(data.join("gen"))
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    gens.sort_by_key(|gen| gen.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0));
    for gen in gens {
        let Ok(round) = gen.file_name().and_then(|n| n.to_str()).unwrap_or_default().parse::<u64>() else {
            continue;
        };
        if round < ours || !gen.join(".installed").exists() || !world_differs(data, &gen) {
            continue;
        }
        activate_generation(data, &gen).map_err(|e| format!("could not finish committee round {round}: {e}"))?;
        tracing::warn!(round, "aether run: finished installing committee round {round} (a previous run was interrupted)");
    }
    Ok(())
}

/// Generations older than `keep` are history nobody reads again.
fn prune_generations(data: &Path, keep: u64) {
    let gens = std::fs::read_dir(data.join("gen"))
        .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect::<Vec<_>>())
        .unwrap_or_default();
    for gen in gens {
        if gen.file_name().and_then(|n| n.to_str()).unwrap_or_default().parse::<u64>().is_ok_and(|r| r < keep) {
            let _ = std::fs::remove_dir_all(gen);
        }
    }
}

fn stop(reshare: &mut Option<Reshare>) {
    if let Some(mut r) = reshare.take() {
        let _ = r.child.kill();
        let _ = r.child.wait();
    }
}

/// Hold `<data>/run.lock` exclusively for the whole run (red team #12): two
/// apps must never run one node between them. The lock lives in the open
/// file description, so keep the returned file for as long as `aether run`
/// runs; the children never re-take it (they are this run's own).
pub fn lock_data_dir(data: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::io::AsRawFd as _;
    std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(data.join("run.lock"))
        .map_err(|e| e.to_string())?;
    match unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } {
        0 => Ok(f),
        _ => Err(format!(
            "another aether already runs with the data directory {} (it holds run.lock)",
            data.display()
        )),
    }
}

/// Overwrite a secret file, then remove it.
fn erase(path: &Path) -> Result<(), String> {
    let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else {
        return Ok(());
    };
    write_secret(path, &vec![0u8; len as usize])?;
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

/// Secret files are written 0600, atomically: a crash mid-write never leaves
/// a truncated share behind (red team #5's atomic-replacement rule).
fn write_secret(path: &Path, bytes: &[u8]) -> Result<(), String> {
    crate::atomic::replace(path, bytes, 0o600)
}

/// Keys that identify this Mac; everything else in <data> belongs to one network.
const KEEP_ACROSS_NETWORKS: [&str; 4] = ["validator.key", "validator.pub.json", "node-account.key", "run.lock"];

/// Put `network` in `<data>/network.json`: the first time, or when it is a
/// different network (a testnet reset: other chain id or committee identity).
/// The old network's data is moved to `<data>/stale-<time>`, never deleted.
/// The same network with newer epochs (written by a reshare) is kept.
pub fn adopt_network(data: &Path, network: Option<&Path>) -> Result<(), String> {
    let ours = data.join("network.json");
    let Some(src) = network else {
        return if ours.exists() {
            Ok(())
        } else {
            Err("first run: pass --network <network.json>".into())
        };
    };
    let incoming = std::fs::read(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let theirs: NetworkFile = serde_json::from_slice(&incoming)
        .map_err(|e| format!("{} is not a network.json: {e}", src.display()))?;
    if let Some(record) = crate::mainnet::resolve_ceremony_record(None, data, Some(src)) {
        crate::mainnet::bind_network_to_record(&theirs, &incoming, &crate::mainnet::load_ceremony_record(&record)?)?;
    }
    if let Ok(current) = NetworkFile::load(&ours) {
        if current.chain_id == theirs.chain_id && current.identity == theirs.identity {
            // Audit 6, A6-4: on a new-genesis chain, the same chain id and
            // committee identity are NOT "the same network" — a local file
            // whose immutable genesis differs is stale, and keeping it means
            // voting under a genesis the ceremony did not check. Refuse until
            // an operator reconciles it on purpose (verify-local with the
            // coordinator's record, or move the old data aside). A reshare or
            // handoff evolution of the SAME genesis (new round/output/epochs)
            // adopts as before, and the legacy testnet id is out of scope.
            if !crate::mainnet::shipped_legacy_network(&incoming) {
                match (
                    crate::mainnet::record_genesis_of(&current),
                    crate::mainnet::record_genesis_of(&theirs),
                ) {
                    (Ok(a), Ok(b)) if a == b => {}
                    (Ok(_), Ok(_)) => {
                        return Err(format!(
                            "the local network.json is a stale file of chain {}: its immutable \
                             genesis (roster, registrar, rule flags, reserve) is not the one the \
                             incoming file carries. Run scripts/mainnet-genesis.sh verify-local \
                             with the coordinator's record, or move {} aside on purpose; do not \
                             vote under a genesis the ceremony did not check",
                            theirs.chain_id,
                            ours.display()
                        ));
                    }
                    (Err(why), _) | (_, Err(why)) => {
                        return Err(format!(
                            "the local network.json cannot be compared against the incoming \
                             genesis ({why}): not a file a ceremony wrote. Move {} aside on \
                             purpose and run verify-local with the coordinator's record",
                            ours.display()
                        ));
                    }
                }
            }
            return Ok(());
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let aside = data.join(format!("stale-{secs}"));
        std::fs::create_dir_all(&aside).map_err(|e| e.to_string())?;
        for e in std::fs::read_dir(data)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let name = e.file_name();
            let n = name.to_string_lossy();
            if KEEP_ACROSS_NETWORKS.contains(&n.as_ref()) || n.starts_with("stale-") {
                continue;
            }
            std::fs::rename(e.path(), aside.join(&name)).map_err(|e| e.to_string())?;
        }
        tracing::warn!(moved_to = %aside.display(), "a different network: the previous one's data was moved aside");
    }
    std::fs::copy(src, &ours)
        .map(|_| ())
        .map_err(|e| format!("{}: {e}", src.display()))
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn rpc_call(url: &str, method: &str, params: Value) -> Result<Value, String> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let v: Value = reqwest::blocking::Client::new()
        .post(url)
        .json(&body)
        .timeout(Duration::from_secs(5))
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    match v.get("error") {
        Some(e) => Err(e.to_string()),
        None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_log_write_is_detected() {
        struct Full;
        impl std::io::Write for Full {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> { Err(std::io::Error::from_raw_os_error(28)) }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        assert_eq!(probe_writer(Full).unwrap_err().raw_os_error(), Some(28));
    }

    #[test]
    fn enospc_in_log_writer_exits_the_node() {
        const CHILD: &str = "AETHER_TEST_LOG_ENOSPC_CHILD";
        if std::env::var_os(CHILD).is_some() {
            struct Full;
            impl std::io::Write for Full {
                fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                    Err(std::io::Error::from_raw_os_error(libc::ENOSPC))
                }
                fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
            }
            require_writer_alive(Full);
            unreachable!("a dead logger must stop the node");
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "supervisor::tests::enospc_in_log_writer_exits_the_node"])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(EXIT_FATAL_TASK));
    }

    #[test]
    fn cache_names_match_only_this_nodes_marshal_archives() {
        assert!(cache_partition("custom-cache-42-certified-key", "certified", "custom"));
        assert!(cache_partition("aether-cache-cache-0-verified-key", "verified", "aether-cache"));
        for name in ["other-cache-42-certified-key", "custom-cache-metadata",
            "custom-consensus-r42", "custom-cache-x-certified-key",
            "custom-cache-18446744073709551616-certified-key"] {
            assert!(!cache_partition(name, "certified", "custom"), "{name}");
        }
        assert!(!cache_partition("custom-cache-42-verified-key", "certified", "custom"));
    }

    #[test]
    fn enospc_on_vote_journal_or_cache_open_waits_without_repair() {
        let enospc = std::io::Error::from_raw_os_error(libc::ENOSPC);
        let journal = format!("unable to sync journal: Runtime(Io({enospc}))");
        let cache = "src/marshal/core/cache.rs";
        assert_eq!(panic_action(&journal, "src/simplex/voter/actor.rs", Some(0), 5 * crate::resources::GB), PanicAction::WaitForDisk);
        assert_eq!(panic_action("failed to initialize verified archive", cache, Some(0), 5 * crate::resources::GB), PanicAction::WaitForDisk);
        assert_eq!(panic_action(&journal, "src/simplex/voter/actor.rs", Some(10 * crate::resources::GB), 5 * crate::resources::GB), PanicAction::Restart);
        assert_eq!(panic_action("failed to initialize verified archive", cache, Some(10 * crate::resources::GB), 5 * crate::resources::GB), PanicAction::RepairCache("verified"));
        assert_eq!(panic_action("failed to initialize verified archive", cache, None, 5 * crate::resources::GB), PanicAction::WaitForDisk);
        assert_eq!(panic_action("failed to initialize finalizations archive", "src/marshal/standard/mod.rs", Some(10 * crate::resources::GB), 5 * crate::resources::GB), PanicAction::Restart,
            "the finalized-by-height archive is not a rebuildable marshal cache");
        assert!(disk_wait_needed(Some(EXIT_DISK_LOW), false), "the child saw low space before the parent sampled recovery");
        assert!(disk_wait_needed(Some(crate::store::EXIT_STORAGE), true));
        assert!(disk_wait_needed(Some(EXIT_FATAL_TASK), true));
        assert!(!disk_wait_needed(Some(crate::store::EXIT_STORAGE), false));
    }

    #[test]
    fn panicked_task_exits_instead_of_leaving_rpc_alive() {
        const CHILD: &str = "AETHER_TEST_FATAL_TASK_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
            install_fatal_watch(root.join("tmp"));
            std::thread::spawn(|| panic!("injected consensus task panic")).join().unwrap();
            unreachable!("the panic hook must exit before the task can join");
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "supervisor::tests::panicked_task_exits_instead_of_leaving_rpc_alive"])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(EXIT_FATAL_TASK));
    }

    #[test]
    fn journal_and_archive_enospc_exit_for_disk_wait_without_quarantining() {
        const CHILD: &str = "AETHER_TEST_DISK_PANIC_CHILD";
        if let Ok(kind) = std::env::var(CHILD) {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
            install_fatal_watch(root.join("tmp"));
            if kind == "journal" {
                panic!("unable to open journal: Runtime(WriteFailed): No space left on device");
            }
            panic!("failed to initialize verified archive");
        }
        for kind in ["journal", "archive"] {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "supervisor::tests::journal_and_archive_enospc_exit_for_disk_wait_without_quarantining"])
                .env(CHILD, kind)
                .env("AETHER_TEST_PANIC_FREE_DISK", "0")
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(EXIT_DISK_LOW), "{kind}");
        }
    }

    #[test]
    fn cache_recovery_uses_the_persisted_restart_budget() {
        let now = 1_000_000;
        let exits: Vec<_> = (0..4).map(|n| ExitNote {
            started_ms: now - 4_000 + n * 1_000,
            at_ms: now - 3_500 + n * 1_000,
            code: Some(EXIT_REBUILDABLE_CACHE),
        }).collect();
        assert!(matches!(next_restart(&exits[..3], now), Next::Again(_)));
        assert_eq!(next_restart(&exits, now), Next::Stop(EXIT_REBUILDABLE_CACHE));
    }

    #[test]
    fn repeated_ten_minute_follower_stalls_exhaust_the_hourly_budget() {
        let now = 4_000_000;
        let exits: Vec<_> = (0..4).map(|n| ExitNote {
            started_ms: now - (4 - n) * 11 * 60 * 1_000,
            at_ms: now - (3 - n) * 11 * 60 * 1_000,
            code: Some(crate::follow::EXIT_STALLED),
        }).collect();
        assert!(matches!(next_restart(&exits[..3], now), Next::Again(_)));
        assert_eq!(next_restart(&exits, now), Next::Stop(crate::follow::EXIT_STALLED));
    }

    #[cfg(unix)]
    #[test]
    fn cache_recovery_refuses_symlinked_partitions() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let data = root.join("tmp").join(format!("cache-link-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(data.join("votes")).unwrap();
        std::fs::write(data.join("votes/part"), b"preserve").unwrap();
        std::os::unix::fs::symlink(data.join("votes"), data.join("aether-cache-0-verified-key")).unwrap();
        std::fs::write(data.join(CACHE_REPAIR_REQUEST), b"verified").unwrap();
        assert!(repair_cache(&data).is_err());
        assert_eq!(std::fs::read(data.join("votes/part")).unwrap(), b"preserve");
        assert!(data.join(CACHE_REPAIR_REQUEST).exists());
        std::fs::remove_dir_all(data).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cache_request_never_overwrites_a_share_through_a_symlink() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let data = root.join("tmp").join(format!("cache-request-link-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("threshold.json"), b"secret share").unwrap();
        std::os::unix::fs::symlink(data.join("threshold.json"), data.join(CACHE_REPAIR_REQUEST)).unwrap();
        assert!(write_cache_repair_request(&data, "verified").is_err());
        assert!(repair_cache(&data).is_err());
        assert_eq!(std::fs::read(data.join("threshold.json")).unwrap(), b"secret share");
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn interrupted_cache_quarantine_resumes_without_moving_a_vote_journal() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let data = root.join("tmp").join(format!("cache-partial-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("partition"), b"aether-cache").unwrap();
        for part in ["metadata", "key", "value"] {
            let dir = data.join(format!("aether-cache-cache-0-verified-{part}"));
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("part"), b"corrupt").unwrap();
        }
        std::fs::create_dir(data.join("aether-consensus-r7")).unwrap();
        std::fs::write(data.join("aether-consensus-r7/part"), b"journal").unwrap();
        write_cache_repair_request(&data, "verified").unwrap();
        let mut moves = 0;
        assert!(repair_cache_with(&data, |from, to| {
            moves += 1;
            if moves == 2 { return Err(std::io::Error::from_raw_os_error(libc::ENOSPC)); }
            std::fs::rename(from, to)
        }).is_err());
        assert!(data.join("quarantine/marshal-cache/pending-verified").exists());
        assert!(repair_cache(&data).unwrap());
        assert!(!data.join(CACHE_REPAIR_REQUEST).exists());
        assert!(!data.join("aether-cache-cache-0-verified-value").exists());
        assert_eq!(std::fs::read(data.join("aether-consensus-r7/part")).unwrap(), b"journal");
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn corrupt_marshal_cache_is_quarantined_without_touching_votes_or_keys() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let data = root.join("tmp").join(format!("cache-repair-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("partition"), b"aether-cache").unwrap();
        for name in ["aether-cache-cache-0-verified-metadata", "aether-cache-cache-0-verified-key", "aether-cache-cache-0-verified-value", "aether-consensus-r7", "aether-cache-cache-0-finalizations-value"] {
            std::fs::create_dir(data.join(name)).unwrap();
            std::fs::write(data.join(name).join("part"), b"data").unwrap();
        }
        std::fs::write(data.join("threshold.json"), b"share").unwrap();
        std::fs::write(data.join(CACHE_REPAIR_REQUEST), b"verified").unwrap();
        assert!(repair_cache(&data).unwrap());
        assert!(data.join("aether-consensus-r7/part").exists());
        assert!(data.join("threshold.json").exists());
        assert!(data.join("aether-cache-cache-0-finalizations-value/part").exists());
        assert!(!data.join("aether-cache-cache-0-verified-value").exists());
        let aside = std::fs::read_dir(data.join("quarantine/marshal-cache")).unwrap().next().unwrap().unwrap().path();
        assert!(aside.join("aether-cache-cache-0-verified-value/part").exists());
        assert!(!aside.join("aether-consensus-r7").exists());
        assert_eq!(rebuildable_panic("failed to initialize verified archive"), Some("verified"));
        assert_eq!(rebuildable_panic("unable to open journal"), None);
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn corrupt_archive_panic_requests_repair_and_preserves_the_vote_journal() {
        const CHILD: &str = "AETHER_TEST_CORRUPT_CACHE_CHILD";
        if let Some(data) = std::env::var_os(CHILD) {
            exit_for_fatal_panic(&PathBuf::from(data), "failed to initialize verified archive", "src/marshal/core/cache.rs");
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let data = root.join("tmp").join(format!("cache-panic-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(data.join("aether-cache-cache-0-verified-key")).unwrap();
        std::fs::write(data.join("partition"), b"aether-cache").unwrap();
        std::fs::write(data.join("aether-cache-cache-0-verified-key/part"), b"corrupt").unwrap();
        std::fs::write(data.join("vote-journal"), b"preserve").unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "supervisor::tests::corrupt_archive_panic_requests_repair_and_preserves_the_vote_journal"])
            .env(CHILD, &data)
            .env("AETHER_TEST_PANIC_FREE_DISK", (10 * crate::resources::GB).to_string())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(EXIT_REBUILDABLE_CACHE));
        assert_eq!(std::fs::read_to_string(data.join(CACHE_REPAIR_REQUEST)).unwrap(), "verified");
        assert!(repair_cache(&data).unwrap());
        assert!(!data.join("aether-cache-cache-0-verified-key").exists());
        assert_eq!(std::fs::read(data.join("vote-journal")).unwrap(), b"preserve");
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn legacy_partition_prefix_is_recovered_without_a_marker() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let data = root.join("tmp").join(format!("cache-legacy-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(data.join("node-1-blocks-metadata")).unwrap();
        std::fs::create_dir(data.join("node-1-cache-0-verified-value")).unwrap();
        std::fs::write(data.join("node-1-cache-0-verified-value/part"), b"corrupt").unwrap();
        std::fs::write(data.join(CACHE_REPAIR_REQUEST), b"verified").unwrap();
        assert!(repair_cache(&data).unwrap());
        assert!(data.join("node-1-blocks-metadata").exists());
        assert!(!data.join("node-1-cache-0-verified-value").exists());
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn shipped_reshare_timeout_covers_child_and_share_readiness() {
        assert_eq!(crate::dkg::Timeouts::default().strict_return_bound(4), Duration::from_secs(322));
        assert_eq!(crate::dkg::POST_STAGE_RELAY, Duration::from_secs(60));
        assert_eq!(crate::dkg::SHARE_READY_WINDOW, Duration::from_secs(120));
        assert!(crate::dkg::SHARE_READY_WINDOW > crate::dkg::POST_STAGE_RELAY);
        assert_eq!(default_reshare_timeout(4), Duration::from_secs(457));
        for players in [4, 5] {
            let child = crate::dkg::Timeouts::default().strict_return_bound(players);
            assert!(default_reshare_timeout(players) > child + crate::dkg::SHARE_READY_WINDOW);
            assert_eq!(effective_reshare_timeout(Some(Duration::from_secs(1)), players, true), default_reshare_timeout(players),
                "an explicit short deadline must not kill a child before its worst-case return");
        }
    }

    #[test]
    fn new_genesis_handoff_refuses_a_seat_without_its_staged_share() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let dir = root.join("tmp").join(format!("handoff-seat-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let me = "01";
        let handoff = json!({"round": 1, "output": "00", "members": [{"key": me, "node": "node"}]});
        std::fs::write(dir.join("network.json"), serde_json::to_vec(&file(7_781, "aa")).unwrap()).unwrap();
        assert!(sup(&dir).check_joining_share(me, &handoff).unwrap_err().contains("no staged share"));
        std::fs::write(dir.join("network.json"), serde_json::to_vec(&file(7_780, "aa")).unwrap()).unwrap();
        assert!(sup(&dir).check_joining_share(me, &handoff).is_ok(), "chain 7780 keeps its existing install path");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_later_attempt_uses_a_fresh_reshare_round() {
        let old = 9;
        assert_eq!(reshare_round(old, 24).unwrap(), reshare_round(old, 24).unwrap(), "restart resumes its journal");
        assert!(reshare_round(old, 25).unwrap() > reshare_round(old, 24).unwrap(), "a new attempt gets new DKG randomness");
    }

    #[test]
    fn staggered_supervisors_share_a_round_for_one_finalized_proposal() {
        let old = 9;
        let samples = [80, 120, 200, 320];
        let mut from = file(7_781, "aa");
        from.validators = (0..4).map(|i| Member { key: i.to_string(), node: i.to_string() }).collect();
        // Four current validators can grow by one seat; the bound stays five
        // even if the current draw proposes a different candidate.
        let players_bound = reshare_attempt_players(from.validators.len(), 0);
        assert_eq!(players_bound, 5);
        let round_at = |height| {
            let proposal = json!({"epoch": 2, "height": height});
            reshare_round(old, reshare_proposal_attempt(&proposal, true).unwrap()).unwrap()
        };
        let rounds: Vec<_> = samples.into_iter().map(round_at).collect();
        assert!(rounds.iter().all(|round| *round == rounds[0]), "supervisors joining one ceremony must agree despite later observed heights");
        assert_eq!(round_at(200), rounds[0], "restart with the same proposal resumes the same round");
        assert_eq!(reshare_attempt(2_200, 2_000, players_bound).unwrap(), reshare_attempt(200, 0, players_bound).unwrap(), "a later committee uses its own chain-visible start");
        let blocks = reshare_attempt_blocks(players_bound).unwrap();
        assert_eq!(round_at(blocks - 1), rounds[0]);
        assert_eq!(round_at(blocks), rounds[0], "the same proposal keeps its round at the observer's window edge");
        let next = json!({"epoch": 3, "height": blocks});
        assert!(reshare_round(old, reshare_proposal_attempt(&next, true).unwrap()).unwrap() > rounds[0], "a later finalized draw uses a fresh round");
        assert!(u128::from(blocks) * u128::from(crate::application::MIN_BLOCK_INTERVAL_MS)
            > default_reshare_timeout(players_bound).saturating_mul(2).as_millis());
        assert!(default_reshare_timeout(players_bound)
            > crate::dkg::Timeouts::default().strict_return_bound(players_bound)
                .saturating_add(crate::dkg::SHARE_READY_WINDOW),
            "the supervisor must outlast the child's readiness return bound");
    }

    #[test]
    fn supervisors_observing_one_proposal_across_window_edge_use_one_round() {
        let players_bound = reshare_attempt_players(4, 3);
        let edge = reshare_attempt_blocks(players_bound).unwrap();
        let old_round = 9;
        let before = reshare_round(old_round, reshare_proposal_attempt(&json!({"epoch": 2, "height": edge - 1}), true).unwrap()).unwrap();
        let after = reshare_round(old_round, reshare_proposal_attempt(&json!({"epoch": 2, "height": edge}), true).unwrap()).unwrap();
        assert_eq!(before, after, "the finalized proposal, not the observer's head, must determine its DKG round");
        assert_ne!(reshare_attempt(edge - 1, 0, players_bound).unwrap(), reshare_attempt(edge, 0, players_bound).unwrap(), "sampled heads straddle the former round boundary");
    }

    fn file(chain_id: u64, identity: &str) -> NetworkFile {
        NetworkFile {
            chain_id,
            validators: vec![],
            identity: Some(identity.into()),
            round: 0,
            output: None,
            epochs: vec![],
            faucet: None,
            registrar: None,
            epoch_blocks: None,
            min_streak: None,
            draw_epochs: None,
            history: None,
            protocol: None,
            node_rewards: None,
            reserve: None,
            group: None,
            max_committee: None,
            genesis_validators: Some(vec![]),
            release: None,
        }
    }

    #[test]
    fn h1_adoption_checks_stored_record_before_moving_it() {
        let dir = std::env::temp_dir().join(format!("aether-adopt-pin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("incoming.json");
        let checked = file(7_801, "aa");
        let bytes = serde_json::to_vec(&checked).unwrap();
        std::fs::write(dir.join("network.json"), &bytes).unwrap();
        let record = crate::mainnet::ceremony_record(&checked, &bytes, 1).unwrap();
        std::fs::write(dir.join(crate::mainnet::CEREMONY_RECORD_FILE), serde_json::to_vec(&record).unwrap()).unwrap();
        let mut altered = checked.clone(); altered.chain_id = 7_780;
        std::fs::write(&src, serde_json::to_vec(&altered).unwrap()).unwrap();
        assert!(adopt_network(&dir, Some(&src)).is_err());
        assert_eq!(std::fs::read(dir.join("network.json")).unwrap(), bytes);
        assert!(dir.join(crate::mainnet::CEREMONY_RECORD_FILE).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_new_network_moves_the_old_data_aside_but_keeps_the_keys() {
        let dir = std::env::temp_dir().join(format!("aether-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.json");
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(&src, serde_json::to_vec(&file(1, "aa")).unwrap()).unwrap();
        adopt_network(&data, Some(&src)).unwrap();
        std::fs::write(data.join("state.redb"), b"old chain").unwrap();
        std::fs::write(data.join("validator.key"), b"key").unwrap();

        // Same network (e.g. with epochs from a reshare): untouched.
        adopt_network(&data, Some(&src)).unwrap();
        assert!(data.join("state.redb").exists());

        // A testnet reset: state moves aside, the Mac's keys stay.
        std::fs::write(&src, serde_json::to_vec(&file(1, "bb")).unwrap()).unwrap();
        adopt_network(&data, Some(&src)).unwrap();
        assert!(!data.join("state.redb").exists());
        assert!(data.join("validator.key").exists());
        assert_eq!(
            NetworkFile::load(&data.join("network.json"))
                .unwrap()
                .identity
                .as_deref(),
            Some("bb")
        );
        let aside = std::fs::read_dir(&data)
            .unwrap()
            .flatten()
            .find(|e| e.file_name().to_string_lossy().starts_with("stale-"))
            .unwrap();
        assert!(aside.path().join("state.redb").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Audit 6, A6-4: on a new-genesis chain, a local network.json with the
    /// same chain id and committee identity but a different immutable genesis
    /// is stale — `aether run` refuses it (an operator reconciles on purpose),
    /// instead of quietly voting under a genesis the ceremony did not check.
    /// A reshare/handoff evolution of the SAME genesis (new round, output,
    /// epochs) keeps adopting, and the legacy testnet id is out of scope.
    #[test]
    fn a_stale_local_genesis_is_reconciled_on_purpose_not_kept() {
        let dir = std::env::temp_dir().join(format!("aether-adopt-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.json");
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();

        // A new-genesis final file (history 2 + node rewards, a frozen opening
        // roster) and a local file that differs only in one genesis field.
        let mut genesis = file(7_801, "aa");
        genesis.history = Some(2);
        genesis.protocol = Some(3);
        genesis.node_rewards = Some(true);
        genesis.genesis_validators = Some(vec![Member { key: "01".into(), node: "node".into() }]);
        assert!(genesis.genesis_validators.is_some(), "the fixture carries its frozen genesis");
        let mut stale = genesis.clone();
        stale.registrar = Some("cd".repeat(32));

        std::fs::write(&src, serde_json::to_vec(&genesis).unwrap()).unwrap();
        adopt_network(&data, Some(&src)).unwrap(); // the first install
        std::fs::write(&src, serde_json::to_vec(&stale).unwrap()).unwrap();
        let err = adopt_network(&data, Some(&src)).unwrap_err();
        assert!(err.contains("genesis"), "{err}");
        assert_eq!(
            NetworkFile::load(&data.join("network.json")).unwrap().registrar,
            None,
            "the stale local file is neither overwritten nor kept voting"
        );

        // A reshare evolution of the SAME pinned genesis still adopts.
        let mut evolved = genesis.clone();
        evolved.round = 3;
        evolved.output = Some("cc".into());
        evolved.epochs.push(crate::roster::EpochStart { height: 100, parent: "aa".into() });
        std::fs::write(&src, serde_json::to_vec(&evolved).unwrap()).unwrap();
        adopt_network(&data, Some(&src)).unwrap();

        // Merely using the legacy id does not exempt an altered file.
        let legacy_data = dir.join("legacy");
        std::fs::create_dir_all(&legacy_data).unwrap();
        std::fs::write(&src, serde_json::to_vec(&file(7_780, "aa")).unwrap()).unwrap();
        adopt_network(&legacy_data, Some(&src)).unwrap();
        let mut legacy_other = file(7_780, "aa");
        legacy_other.registrar = Some("cd".repeat(32));
        std::fs::write(&src, serde_json::to_vec(&legacy_other).unwrap()).unwrap();
        assert!(adopt_network(&legacy_data, Some(&src)).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The follower child's argv in each storage mode (wallet ▸ 블록 데이터
    /// 위치 / 전체 기록 보관): where its data goes, and whether it may take
    /// the snapshot shortcuts.
    #[test]
    fn the_follower_child_argv_follows_the_storage_mode() {
        let net = Path::new("/int/node/network.json");
        let data = Path::new("/int/node");
        let ext = Path::new("/Volumes/Ext/EastSea");
        let extra = vec!["--max-shards=8".to_string()];
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();

        assert_eq!(
            follower_args(net, data, None, false, 18545, true, &extra),
            s(&["follow", "--exit-with-parent", "--network", "/int/node/network.json", "--data", "/int/node/follow",
                "--rpc-port", "18545", "--checkpoint", "--keys", "/int/node", "--candidate", "--max-shards=8"]),
            "default: everything under --data"
        );
        assert_eq!(
            follower_args(net, data, Some(ext), false, 18545, true, &extra),
            s(&["follow", "--exit-with-parent", "--network", "/int/node/network.json", "--data", "/Volumes/Ext/EastSea/follow",
                "--rpc-port", "18545", "--checkpoint", "--node-key", "/int/node/follow/wallet-node.key",
                "--keys", "/int/node", "--candidate", "--max-shards=8"]),
            "chain data on the chosen disk; the keys stay on the internal one"
        );
        assert_eq!(
            follower_args(net, data, None, true, 18545, false, &[]),
            s(&["follow", "--exit-with-parent", "--network", "/int/node/network.json", "--data", "/int/node/archive",
                "--rpc-port", "18545", "--archive-export", "/int/node/archive/era",
                "--node-key", "/int/node/follow/wallet-node.key"]),
            "archive: its own directory, no snapshot start, era export on"
        );
        assert_eq!(
            follower_args(net, data, Some(ext), true, 18545, true, &[]),
            s(&["follow", "--exit-with-parent", "--network", "/int/node/network.json", "--data", "/Volumes/Ext/EastSea/archive",
                "--rpc-port", "18545", "--archive-export", "/Volumes/Ext/EastSea/archive/era",
                "--node-key", "/int/node/follow/wallet-node.key", "--keys", "/int/node", "--candidate"]),
            "archive on the chosen disk, still a candidate"
        );
        assert_eq!(follower_dir(data, Some(ext), false), ext.join("follow"));
        assert_eq!(follower_dir(data, None, true), data.join("archive"));
    }

    fn key_dirs(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-keyguard-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("chain/follow")).unwrap();
        std::fs::create_dir_all(dir.join("node")).unwrap();
        dir
    }

    /// "keys must stay on this Mac": a key file in the chain-data directory
    /// (or its follow/archive top level) refuses the start.
    #[test]
    fn keys_in_chain_data_are_found() {
        let dir = key_dirs("chain");
        let chain = dir.join("chain");
        std::fs::write(chain.join("follow/state.redb"), b"blocks").unwrap();
        assert!(keys_in_chain_data(&chain).is_empty(), "block data alone is fine");
        std::fs::write(chain.join("follow/wallet-node.key"), [0u8; 32]).unwrap();
        assert_eq!(keys_in_chain_data(&chain), vec![chain.join("follow/wallet-node.key")]);
        std::fs::create_dir_all(chain.join("archive")).unwrap();
        std::fs::write(chain.join("archive/threshold.json"), b"{}").unwrap();
        std::fs::write(chain.join("validator.key"), b"k").unwrap();
        assert_eq!(keys_in_chain_data(&chain).len(), 3, "the top level, follow and archive");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The key directory on a removable or network volume, with keys in it,
    /// refuses; without keys (nothing to lose) or on the internal disk it runs.
    #[test]
    fn keys_on_an_external_key_dir_are_refused() {
        let dir = key_dirs("ext");
        let data = dir.join("node");
        assert!(!keys_on_external_data(&data, true), "no keys yet: nothing to protect");
        std::fs::write(data.join("validator.key"), b"k").unwrap();
        assert!(keys_on_external_data(&data, true));
        assert!(!keys_on_external_data(&data, false), "the internal disk is where keys belong");
        assert!(volume_is_external(Path::new("/Volumes/Backup/EastSea")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A network reset in the split layout moves nothing between the two
    /// directories: the keys stay in --data, the chain data is untouched.
    #[test]
    fn a_network_reset_keeps_keys_out_of_chain_data() {
        let dir = key_dirs("reset");
        let (data, chain) = (dir.join("node"), dir.join("chain"));
        let src = dir.join("source.json");
        let mut net = file(1, "aa");
        std::fs::write(&src, serde_json::to_vec(&net).unwrap()).unwrap();
        adopt_network(&data, Some(&src)).unwrap();
        for k in ["validator.key", "validator.pub.json", "node-account.key"] {
            std::fs::write(data.join(k), b"k").unwrap();
        }
        std::fs::write(chain.join("follow/state.redb"), b"blocks").unwrap();
        net.identity = Some("bb".into());
        std::fs::write(&src, serde_json::to_vec(&net).unwrap()).unwrap();
        adopt_network(&data, Some(&src)).unwrap();
        for k in ["validator.key", "validator.pub.json", "node-account.key"] {
            assert!(data.join(k).exists(), "{k} stays in the key directory");
            assert!(KEEP_ACROSS_NETWORKS.contains(&k));
        }
        assert!(keys_in_chain_data(&chain).is_empty(), "no key moved into chain data");
        assert_eq!(std::fs::read(chain.join("follow/state.redb")).unwrap(), b"blocks", "chain data untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The disk floor guards the volume the chain data is written to.
    #[test]
    fn the_disk_floor_watches_the_chain_data_volume() {
        let dir = std::env::temp_dir().join(format!("aether-chain-floor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = sup(&dir);
        assert_eq!(s.disk_dir(), dir.as_path());
        let ext = dir.join("ext");
        s.chain_data = Some(ext.clone());
        assert_eq!(s.disk_dir(), ext.as_path());
        assert!(s.chain_data_missing(), "a chain dir that does not exist is a missing disk");
        std::fs::create_dir_all(&ext).unwrap();
        assert!(!s.chain_data_missing());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn sup(dir: &Path) -> Supervisor {
        Supervisor {
            exe: std::env::current_exe().unwrap(),
            data: dir.to_path_buf(),
            port: 0,
            reshare_port: 0,
            rpc_port: 0,
            node_args: vec![],
            follow_args: vec![],
            dev_peer_dir: None,
            reshare_timeout: Some(Duration::from_secs(1)),
            ceremony: None,
            chain_data: None,
            archive: false,
        }
    }

    /// Red team #5: a seated validator whose key file is damaged (or gone)
    /// loses the vote, not the wallet — the supervisor runs it as a plain
    /// follower and never generates a replacement identity.
    #[test]
    fn a_mac_whose_key_is_unreadable_follows_instead_of_voting() {
        let dir = std::env::temp_dir().join(format!("aether-keyless-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::candidate::CandidateKeys::load_or_create(&dir).unwrap();
        std::fs::write(dir.join("threshold.json"), b"share").unwrap();
        let me = sup(&dir).my_key().expect("a saved key loads");
        let backup = std::fs::read(dir.join(crate::roster::KEY_FILE)).unwrap();

        let mut net = file(1, "aa");
        net.validators = vec![Member { key: me.clone(), node: "node".into() }];
        std::fs::write(dir.join("network.json"), serde_json::to_vec(&net).unwrap()).unwrap();
        let s = sup(&dir);
        assert_eq!(s.role(Some(&me)).unwrap(), Role::Validator, "a member with its share votes");

        // The incident: the key file no longer parses.
        std::fs::write(dir.join(crate::roster::KEY_FILE), b"torn write").unwrap();
        assert!(s.my_key().is_none(), "the damaged key does not load");
        assert_eq!(s.role(None).unwrap(), Role::Keyless, "and the Mac follows, keyless");
        assert!(
            dir.join("threshold.json").exists() && dir.join(crate::roster::KEY_FILE).exists(),
            "nothing was minted or erased meanwhile"
        );
        std::fs::write(dir.join(crate::roster::KEY_FILE), backup).unwrap();
        assert_eq!(s.my_key().as_deref(), Some(me.as_str()), "the restored backup revives the original identity");
        assert_eq!(s.role(Some(&me)).unwrap(), Role::Validator);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Red team #4: a validator that refused its journal (engine exit 8)
    /// follows for the rest of that committee round, and may vote again only
    /// once the committee has moved to a later round.
    #[test]
    fn an_untrusted_journal_pauses_voting_until_the_next_round() {
        let dir = std::env::temp_dir().join(format!("aether-novote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::candidate::CandidateKeys::load_or_create(&dir).unwrap();
        std::fs::write(dir.join("threshold.json"), b"share").unwrap();
        let me = sup(&dir).my_key().unwrap();
        let mut net = file(1, "aa");
        net.round = 7;
        net.validators = vec![Member { key: me.clone(), node: "node".into() }];
        std::fs::write(dir.join("network.json"), serde_json::to_vec(&net).unwrap()).unwrap();
        let s = sup(&dir);
        assert_eq!(s.role(Some(&me)).unwrap(), Role::Validator);

        // The child refuses with the journal exit code, mid-round.
        note_untrusted_journal(&dir, net.round);
        assert!(voting_paused(&dir, net.round));
        assert_eq!(s.role(Some(&me)).unwrap(), Role::Paused, "voting and beacons stay off for round 7");
        // An older refusal does not extend the pause; a newer round ends it.
        note_untrusted_journal(&dir, 3);
        assert!(voting_paused(&dir, net.round), "the round-7 refusal still binds");
        net.round = 8;
        std::fs::write(dir.join("network.json"), serde_json::to_vec(&net).unwrap()).unwrap();
        assert_eq!(s.role(Some(&me)).unwrap(), Role::Validator, "the next round votes again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Red team #4: adopting a follower's state moves the validator's state
    /// and archives aside, but never the vote journal — the one record of the
    /// votes this Mac cast.
    #[test]
    fn adopting_a_followers_state_keeps_the_vote_journal() {
        let dir = std::env::temp_dir().join(format!("aether-adopt-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("follow")).unwrap();
        std::fs::write(dir.join("partition"), b"aether").unwrap();
        for name in ["state.redb", "aether-finalizations-ordinal", "aether-finalized-blocks-ordinal"] {
            std::fs::write(dir.join(name), b"validator state").unwrap();
        }
        for name in ["aether-consensus", "aether-consensus-e1", "aether-consensus-r9"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join("0"), b"a vote").unwrap();
        }
        std::fs::write(dir.join("follow").join("state.redb"), b"follower state").unwrap();

        sup(&dir).adopt_follower_state().unwrap();
        assert!(dir.join("aether-consensus").join("0").exists(), "the vote journal stays");
        assert!(dir.join("aether-consensus-e1").exists());
        assert!(dir.join("aether-consensus-r9").exists());
        let aside = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .find(|e| e.file_name().to_string_lossy().starts_with("stale-"))
            .unwrap()
            .path();
        let moved: Vec<String> = std::fs::read_dir(&aside)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("aether-consensus"))
            .collect();
        assert!(moved.is_empty(), "no journal partition was moved: {moved:?}");
        assert!(aside.join("state.redb").exists(), "the state moved");
        assert!(aside.join("aether-finalizations-ordinal").exists(), "the archive moved");
        assert_eq!(std::fs::read(dir.join("state.redb")).unwrap(), b"follower state");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn exit(at: u64, uptime_ms: u64, code: i32) -> ExitNote {
        ExitNote { started_ms: at - uptime_ms, at_ms: at, code: Some(code) }
    }

    /// Red team #1: the supervisor no longer restarts a dying child every
    /// 2 s forever. Exit codes a restart cannot change stop immediately, four
    /// exits inside ten minutes stop the loop, and every quick exit doubles
    /// the backoff up to a minute — while a child that made progress does not
    /// deepen it.
    #[test]
    fn restarts_back_off_and_stop_instead_of_looping() {
        const NOW: u64 = 10_000_000_000;
        // Nothing yet: the first try waits only the base backoff.
        assert_eq!(next_restart(&[], NOW), Next::Again(Duration::from_secs(1)));
        // Consecutive quick exits: 1 s, 2 s, 4 s — but the fourth inside ten
        // minutes never restarts at all.
        let streak = |n: u64, base: u64| (0..n).map(|i| exit(base + i * 1_000, 1_000, 1)).collect::<Vec<_>>();
        for (n, want) in [(1u64, 1u64), (2, 2), (3, 4)] {
            assert_eq!(next_restart(&streak(n, NOW - n * 1_000), NOW), Next::Again(Duration::from_secs(want)));
        }
        assert_eq!(
            next_restart(&streak(4, NOW - 4_000), NOW),
            Next::Stop(1),
            "the fourth exit inside ten minutes never restarts"
        );
        // The fourth exit being a storage code stops with that code, so the
        // app can say what to do (free disk space).
        let storage = [exit(NOW - 3_000, 1_000, 1), exit(NOW - 2_000, 1_000, 1), exit(NOW - 1_000, 1_000, 1), exit(NOW, 1_000, crate::store::EXIT_STORAGE)];
        assert_eq!(next_restart(&storage, NOW), Next::Stop(crate::store::EXIT_STORAGE));
        // A streak that predates the window still deepens the backoff — the
        // loop is the same loop — up to the minute cap.
        let base = NOW - WINDOW_MS - 10 * 60 * 1_000;
        for (n, want) in [(4u64, 8u64), (5, 16), (6, 32), (8, 60)] {
            assert_eq!(next_restart(&streak(n, base), NOW), Next::Again(Duration::from_secs(want)), "{n} old quick exits back off {want} s");
        }
        // A child that ran five minutes made progress: the backoff starts over.
        let mut mixed = streak(2, NOW - 2_000);
        mixed.push(exit(NOW, PROGRESS_MS + 1, 1));
        assert_eq!(next_restart(&mixed, NOW), Next::Again(Duration::from_secs(1)));

        // Exit codes a restart cannot change propagate the moment they happen.
        for code in [
            EXIT_UPGRADE_REQUIRED,
            crate::store::EXIT_STORAGE,
            EXIT_NO_VERIFIER,
            crate::candidate::EXIT_IDENTITY,
            EXIT_LOCKED,
        ] {
            assert_eq!(next_restart(&[exit(NOW, 1_000, code)], NOW), Next::Stop(code));
        }
    }

    /// The history persists, and a damaged history file is refused — it can
    /// never silently reset the restart budget.
    #[test]
    fn the_exit_history_survives_a_restart_of_the_supervisor_itself() {
        let dir = std::env::temp_dir().join(format!("aether-runstate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_exits(&dir).unwrap().is_empty());
        let note = exit(now_ms(), 500, 9);
        let _ = crate::atomic::replace(
            &dir.join("run-state.json"),
            &serde_json::to_vec(&vec![note]).unwrap(),
            0o644,
        );
        assert_eq!(load_exits(&dir).unwrap(), vec![note], "the next run inherits the backoff");
        std::fs::write(dir.join("run-state.json"), b"{not json").unwrap();
        assert!(load_exits(&dir).is_err(), "a damaged history cannot reset the restart budget");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The history file carries a format version: the envelope round-trips,
    /// and a format from a newer binary is refused rather than read as
    /// empty — the restart budget must not reset in that direction either.
    #[test]
    fn the_exit_history_is_versioned() {
        let dir = std::env::temp_dir().join(format!("aether-runstate-v-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let note = exit(now_ms(), 500, 9);
        let _ = crate::atomic::replace(
            &dir.join("run-state.json"),
            &serde_json::to_vec(&RunStateRecord { v: RUN_STATE_VERSION, exits: vec![note] })
                .unwrap(),
            0o644,
        );
        assert_eq!(load_exits(&dir).unwrap(), vec![note]);
        std::fs::write(dir.join("run-state.json"), br#"{"v":2,"exits":[]}"#).unwrap();
        let err = load_exits(&dir).unwrap_err();
        assert!(err.contains("newer"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Red team #12: one data directory, one `aether`. A second taker of the
    /// lock fails; once the first lets go, it succeeds again.
    #[test]
    fn a_second_aether_on_the_same_data_dir_fails_to_start() {
        let dir = std::env::temp_dir().join(format!("aether-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = lock_data_dir(&dir).expect("the first taker locks");
        let err = lock_data_dir(&dir).expect_err("the second taker fails");
        assert!(err.contains("another aether"), "{err}");
        drop(first);
        assert!(lock_data_dir(&dir).is_ok(), "after the first lets go");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_network_reset_does_not_move_the_live_run_lock() {
        let dir = std::env::temp_dir().join(format!("aether-lock-reset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("source.json");
        let data = dir.join("node");
        std::fs::create_dir_all(&data).unwrap();
        let mut first = file(1, "aa");
        std::fs::write(&src, serde_json::to_vec(&first).unwrap()).unwrap();
        adopt_network(&data, Some(&src)).unwrap();
        let lock = lock_data_dir(&data).unwrap();
        first.identity = Some("bb".into());
        std::fs::write(&src, serde_json::to_vec(&first).unwrap()).unwrap();
        adopt_network(&data, Some(&src)).unwrap();
        assert!(data.join("run.lock").exists());
        assert!(lock_data_dir(&data).is_err(), "reset cannot unlock the directory while its process lives");
        drop(lock);
        assert!(lock_data_dir(&data).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The crash points of a committee install (red team #19), as states on
    /// disk. Each is what a run that died there leaves behind; every one must
    /// end in the same installed world once the next run resumes.
    #[test]
    fn an_interrupted_committee_install_resumes_to_the_same_world() {
        let dir = std::env::temp_dir().join(format!("aether-gen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut old = file(1, "aa");
        old.round = 5;
        std::fs::write(dir.join("network.json"), serde_json::to_vec(&old).unwrap()).unwrap();
        std::fs::write(dir.join("threshold.json"), b"the old share").unwrap();

        // The prepared generation for round 6, complete on disk.
        let mut next = file(1, "aa");
        next.round = 6;
        next.output = Some("the new output".into());
        let gen = dir.join("gen").join("6");
        std::fs::create_dir_all(&gen).unwrap();
        std::fs::write(gen.join("network.json"), serde_json::to_vec(&next).unwrap()).unwrap();
        let share = serde_json::to_vec(&crate::dkg::KeyFile {
            round: 6, output: "the new output".into(), identity: "aa".into(), share: "00".into(),
        }).unwrap();
        std::fs::write(gen.join("threshold.json"), &share).unwrap();

        // Crash before the completion marker: the old world is untouched.
        finish_incomplete(&dir).unwrap();
        assert_eq!(NetworkFile::load(&dir.join("network.json")).unwrap().round, 5, "an unmarked generation installs nothing");
        assert_eq!(std::fs::read(dir.join("threshold.json")).unwrap(), b"the old share");

        // Crash right after the marker (before any activation): the resume
        // swaps pointer and share in.
        std::fs::write(gen.join(".installed"), b"").unwrap();
        finish_incomplete(&dir).unwrap();
        assert_eq!(NetworkFile::load(&dir.join("network.json")).unwrap().round, 6, "the pointer moved");
        assert_eq!(std::fs::read(dir.join("threshold.json")).unwrap(), share);
        // And it lands there again from any earlier point, idempotently.
        finish_incomplete(&dir).unwrap();
        assert_eq!(NetworkFile::load(&dir.join("network.json")).unwrap().round, 6);

        // A later crash between pointer and share: pointer already 6, share
        // still the old one — the resume finishes the swap.
        std::fs::write(dir.join("threshold.json"), b"the old share").unwrap();
        finish_incomplete(&dir).unwrap();
        assert_eq!(std::fs::read(dir.join("threshold.json")).unwrap(), share);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A member that leaves the voting set drops its share as part of the
    /// install — and an interrupted install still drops it, on resume.
    #[test]
    fn a_leaving_member_drops_its_share_even_from_an_interrupted_install() {
        let dir = std::env::temp_dir().join(format!("aether-gen-leave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut old = file(1, "aa");
        old.round = 5;
        std::fs::write(dir.join("network.json"), serde_json::to_vec(&old).unwrap()).unwrap();
        std::fs::write(dir.join("threshold.json"), b"the old share").unwrap();

        let mut next = file(1, "bb"); // another committee: this Mac leaves
        next.round = 6;
        let gen = dir.join("gen").join("6");
        std::fs::create_dir_all(&gen).unwrap();
        std::fs::write(gen.join("network.json"), serde_json::to_vec(&next).unwrap()).unwrap();
        // No threshold.json in the generation: the seat went to someone else.
        std::fs::write(gen.join(".installed"), b"").unwrap();

        finish_incomplete(&dir).unwrap();
        assert_eq!(NetworkFile::load(&dir.join("network.json")).unwrap().round, 6);
        assert!(!dir.join("threshold.json").exists(), "the old share is gone, so no quorum of forgotten shares can sign");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Red team #19: the swap itself runs with the old child already stopped,
    /// so the two things the install reads from the old world — the block its
    /// epoch anchors on, and the proof a joining Mac starts from — are read
    /// while that child is still answering. An install that asked the stopped
    /// child would never finish a handoff: each node would restart, fail the
    /// same read and give up (`open_voting_nodes_take_over_the_chain_by_themselves`).
    #[test]
    fn a_handoff_install_reads_the_old_world_before_it_stops_the_child() {
        let dir = std::env::temp_dir().join(format!("aether-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let me = "a".repeat(64);
        let mut ours = file(1, "id");
        ours.round = 3;
        ours.validators = vec![Member { key: me.clone(), node: "node".into() }];
        std::fs::write(dir.join("network.json"), serde_json::to_vec(&ours).unwrap()).unwrap();
        std::fs::write(dir.join("threshold.json"), b"the old share").unwrap();

        // The chain finalized the handoff for round 4; this Mac leaves at the switch.
        let switch = 40u64;
        let h = json!({
            "round": 4,
            "switch": switch,
            "output": "the new committee",
            "finalized": switch - 1,
            "members": [{ "key": "b".repeat(64), "node": "node" }],
        });
        // The reads the install needs come from the child, before it is stopped.
        let anchor = Anchor { end_hash: "the block before the switch".into(), finalized: Value::Null };
        sup(&dir)
            .install(Role::Validator, &me, &h, &anchor)
            .expect("an install needs no running child");

        let next = NetworkFile::load(&dir.join("network.json")).unwrap();
        assert_eq!(next.round, 4);
        assert_eq!(next.validators.len(), 1);
        assert_eq!(next.validators[0].key, "b".repeat(64));
        assert!(!dir.join("threshold.json").exists(), "a Mac that leaves drops its share");
        assert!(dir.join("gen").join("4").join(".installed").exists(), "the generation is complete");
        let epoch = next.epochs.last().expect("the new round starts an epoch");
        assert_eq!(epoch.height, switch);
        assert_eq!(epoch.parent, "the block before the switch", "the epoch anchors on the block read from the child");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_prepared_share_never_advances_the_network_pointer() {
        let dir = std::env::temp_dir().join(format!("aether-gen-broken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut old = file(1, "aa");
        old.round = 5;
        std::fs::write(dir.join("network.json"), serde_json::to_vec(&old).unwrap()).unwrap();
        std::fs::write(dir.join("threshold.json"), b"the old share").unwrap();
        let mut next = file(1, "aa");
        next.round = 6;
        let gen = dir.join("gen/6");
        std::fs::create_dir_all(gen.join("threshold.json")).unwrap(); // injected I/O failure
        std::fs::write(gen.join("network.json"), serde_json::to_vec(&next).unwrap()).unwrap();
        std::fs::write(gen.join(".installed"), b"").unwrap();

        assert!(finish_incomplete(&dir).is_err(), "startup must stop on a broken completed generation");
        assert_eq!(NetworkFile::load(&dir.join("network.json")).unwrap().round, 5);
        assert_eq!(std::fs::read(dir.join("threshold.json")).unwrap(), b"the old share");
        std::fs::remove_dir(gen.join("threshold.json")).unwrap();
        std::fs::write(gen.join("threshold.json"), b"torn share").unwrap();
        assert!(finish_incomplete(&dir).is_err(), "malformed share bytes cannot become active");
        assert_eq!(NetworkFile::load(&dir.join("network.json")).unwrap().round, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
