//! Block proofs through the `aether-prover` sidecar (apps/prover, protocol 2,
//! docs/design/13-protocol-2.md).
//!
//! Two sidecar processes: one verifies proofs for consensus (answers in well
//! under a second once warm), one proves blocks (a minute or so each), so a
//! long proof never delays a vote. The protocol pins the proving program: a
//! node whose sidecar embeds another guest ELF refuses to use it.

use crate::chain::{Chain, ProofVerifier};
use aether_light::block::ProofClaim;
use aether_types::Address;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

/// SHA-256 of the guest ELF the protocol proves with, set by the release build
/// (`AETHER_PROVER_PROGRAM`); unset in development builds (any program).
pub const PROGRAM: Option<&str> = option_env!("AETHER_PROVER_PROGRAM");

/// The sidecar binary: `$AETHER_PROVER`, next to this executable (the app's
/// Helpers), or the development build in apps/prover.
pub fn find_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AETHER_PROVER") {
        return Some(PathBuf::from(p));
    }
    let exe = std::env::current_exe().ok()?;
    let beside = exe.with_file_name("aether-prover");
    if beside.exists() {
        return Some(beside);
    }
    dev_binary(&exe).filter(|dev| dev.exists())
}

/// Where the development build keeps the sidecar, for an executable at `exe`
/// (apps/prover has its own target dir). Built from the running executable:
/// `CARGO_MANIFEST_DIR` would bake the builder's absolute path into the binary,
/// so the same source built in another directory would differ (gap G5).
fn dev_binary(exe: &Path) -> Option<PathBuf> {
    Some(exe.parent()?.join("../../apps/prover/target/release/aether-prover"))
}

struct Io {
    child: Child,
    stdin: ChildStdin,
    /// Reply lines, read by a thread so a request can give up waiting.
    lines: std::sync::mpsc::Receiver<String>,
}

/// How long a request may take before the sidecar is presumed hung and killed.
const VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const PROVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1800);
const SPAWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The verify timeout, overridable for the fault tests (hidden, like
/// `AETHER_STORE_RECOVERY`): `AETHER_VERIFY_TIMEOUT_MS=<ms>`.
fn verify_timeout() -> std::time::Duration {
    std::env::var("AETHER_VERIFY_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(VERIFY_TIMEOUT, std::time::Duration::from_millis)
}

/// The sidecar's verdict on one proof (red team #20).
#[derive(Debug, PartialEq, Eq)]
pub enum Verified {
    /// Checked: the proof holds for the commitment.
    Valid,
    /// Checked: it does not.
    Invalid,
    /// No verdict — the sidecar died, hung, could not be talked to, or could
    /// not read the proof. Nothing may be remembered from this: not a
    /// rejection, not an acceptance.
    Unavailable(String),
}

/// One `aether-prover serve` process.
pub struct Sidecar {
    io: Mutex<Io>,
    dir: PathBuf,
    /// Killed by pid: a request may hold the io lock for a whole proof, so the
    /// memory watchdog never waits for that lock.
    pid: u32,
    /// SHA-256 of the guest ELF it proves and verifies against.
    pub program: String,
}

/// Never anything about the child it holds (`unwrap_err` in tests wants Debug).
impl std::fmt::Debug for Sidecar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sidecar(..)")
    }
}

impl Sidecar {
    /// The consensus verifier's sidecar: normal priority (a slow answer holds
    /// up a vote).
    pub fn spawn(bin: &Path, dir: &Path) -> Result<Self, String> {
        Self::start(bin, dir, false)
    }

    /// The proving sidecar, tuned to stay out of the machine's way: a low
    /// scheduler priority (CPU contention only — nice does not throttle disk
    /// or QoS) and at most half the cores of rayon workers, per the resource
    /// limits. The node process keeps normal priority (it may be a validator).
    pub fn spawn_prover(bin: &Path, dir: &Path) -> Result<Self, String> {
        Self::start(bin, dir, true)
    }

    fn start(bin: &Path, dir: &Path, tuned: bool) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(bin);
        cmd.arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if tuned {
            let threads = crate::resources::monitor()
                .map(|m| m.limits.prover_threads)
                .unwrap_or_else(|| crate::resources::Limits::default().prover_threads);
            cmd.env("RAYON_NUM_THREADS", threads.to_string());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt as _;
                // Between fork and exec: one syscall, async-signal-safe.
                unsafe {
                    cmd.pre_exec(|| {
                        libc::setpriority(libc::PRIO_PROCESS, 0, 15);
                        Ok(())
                    });
                }
            }
        }
        let mut child = cmd.spawn().map_err(|e| format!("start {}: {e}", bin.display()))?;
        let pid = child.id();
        let stdin = child.stdin.take().ok_or("no sidecar stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("no sidecar stdout")?);
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in stdout.lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        // Its first line reports the program; a sidecar that says nothing in time is not used.
        let first = match lines.recv_timeout(SPAWN_TIMEOUT) {
            Ok(line) => line,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the sidecar did not start in time".into());
            }
        };
        // A sidecar whose line is unusable is refused *and* stopped (red team
        // #20): an answer the node cannot read must not leave a stray process.
        let info: Value = match serde_json::from_str(&first) {
            Ok(v) => v,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("sidecar info: {e}"));
            }
        };
        let program = match info["guest_elf_sha256"].as_str() {
            Some(p) => p.to_string(),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("sidecar did not report its program".into());
            }
        };
        if let Some(pinned) = PROGRAM.filter(|p| *p != program) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("the sidecar proves program {program}, the protocol pins {pinned}"));
        }
        Ok(Sidecar { io: Mutex::new(Io { child, stdin, lines }), dir: dir.to_path_buf(), pid, program })
    }

    /// Stop the process now, by pid (the io lock may be held by a request for
    /// the whole proof). `Drop` still reaps it.
    pub fn kill_hard(&self) {
        unsafe { libc::kill(self.pid as libc::pid_t, libc::SIGKILL) };
    }

    fn request(&self, req: Value, timeout: std::time::Duration) -> Result<Value, String> {
        let mut io = self.io.lock().map_err(|_| "sidecar lock poisoned")?;
        writeln!(io.stdin, "{req}").and_then(|_| io.stdin.flush()).map_err(|e| format!("sidecar: {e}"))?;
        let line = match io.lines.recv_timeout(timeout) {
            Ok(line) => line,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Hung: stop it so the next request starts a fresh one.
                let _ = io.child.kill();
                let _ = io.child.wait();
                return Err("the sidecar exited (no answer in time)".into());
            }
            Err(_) => return Err("the sidecar exited".into()),
        };
        let v: Value = serde_json::from_str(&line).map_err(|e| format!("sidecar reply: {e}"))?;
        if v["ok"].as_bool() == Some(true) {
            Ok(v)
        } else {
            Err(v["error"].as_str().unwrap_or("sidecar error").to_string())
        }
    }

    /// Prove a postcard `BlockInput`: (proof bytes, commitment, seconds).
    pub fn prove(&self, input: &[u8]) -> Result<(Vec<u8>, [u8; 32], f64), String> {
        let tag = unique();
        let (inp, out) = (self.dir.join(format!("{tag}.input")), self.dir.join(format!("{tag}.proof")));
        std::fs::write(&inp, input).map_err(|e| e.to_string())?;
        let reply = self.request(json!({"cmd": "prove", "input": inp, "out": out}), PROVE_TIMEOUT);
        let _ = std::fs::remove_file(&inp);
        let reply = reply?;
        let proof = std::fs::read(&out).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&out);
        let commitment = from_hex32(reply["commitment"].as_str().unwrap_or_default())?;
        Ok((proof, commitment, reply["seconds"].as_f64().unwrap_or_default()))
    }

    /// What the sidecar said about a proof (red team #20): three values,
    /// never two. "Could not check" is not "checked and false" — a node that
    /// reads a dead sidecar as a refusal rejects proofs it never judged and
    /// splits itself from the network.
    pub fn verify(&self, proof: &[u8], commitment: [u8; 32]) -> Verified {
        let path = self.dir.join(format!("{}.verify", unique()));
        if let Err(e) = std::fs::write(&path, proof) {
            return Verified::Unavailable(format!("proof file: {e}"));
        }
        let reply = self.request(json!({"cmd": "verify", "proof": path, "commitment": hex::encode(commitment)}), verify_timeout());
        let _ = std::fs::remove_file(&path);
        match reply {
            Ok(v) if v["verified"].as_bool() == Some(true) => Verified::Valid,
            Ok(v) if v["verified"].as_bool() == Some(false) => Verified::Invalid,
            // An answer that carries no verdict judges nothing.
            Ok(_) => Verified::Unavailable("the sidecar answered without a verdict".into()),
            // This node could not talk to the sidecar, or read its answer:
            // every message `request` builds itself says "sidecar".
            Err(e) if e.starts_with("sidecar") || e.starts_with("the sidecar") => Verified::Unavailable(e),
            // The sidecar could not read the proof file this node just wrote —
            // a disk or transport problem, not a property of the proof.
            Err(e) if e.starts_with("read ") => Verified::Unavailable(e),
            // Only an explicit proof rejection is a verdict. An unexpected
            // sidecar error can be a panic, parse failure or I/O problem and
            // must not be turned into an invalid-proof decision.
            Err(e) if e == "proof rejected" => Verified::Invalid,
            Err(e) => Verified::Unavailable(e),
        }
    }
}

/// A file name no other request uses (requests on one sidecar may overlap).
fn unique() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!("{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

impl Drop for Sidecar {
    /// A replaced or dropped sidecar is stopped and reaped (no stray processes).
    fn drop(&mut self) {
        if let Ok(io) = self.io.get_mut() {
            let _ = io.child.kill();
            let _ = io.child.wait();
        }
    }
}

fn from_hex32(s: &str) -> Result<[u8; 32], String> {
    hex::decode(s).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| "bad commitment".to_string())
}

/// The consensus verifier: the verifying sidecar, with answers cached (a block's
/// proofs are checked when proposed, verified and finalized).
pub struct Verifier {
    sidecar: Mutex<Arc<Sidecar>>,
    bin: PathBuf,
    dir: PathBuf,
    seen: Mutex<HashMap<[u8; 32], bool>>,
}

impl Verifier {
    /// Start the verifying sidecar (before consensus starts: blocks replayed or
    /// delivered at start-up may carry proofs).
    pub fn start(bin: &Path, dir: &Path) -> Result<Self, String> {
        let sidecar = Arc::new(Sidecar::spawn(bin, dir)?);
        Ok(Verifier { sidecar: Mutex::new(sidecar), bin: bin.to_path_buf(), dir: dir.to_path_buf(), seen: Mutex::new(HashMap::new()) })
    }

    pub fn program(&self) -> String {
        self.sidecar.lock().map(|s| s.program.clone()).unwrap_or_default()
    }

    /// Ask the sidecar; when it could not answer, start a new one and ask
    /// again once. An invalid proof is an answer — it is never re-asked.
    fn ask(&self, proof: &[u8], commitment: [u8; 32]) -> Verified {
        let current = match self.sidecar.lock() {
            Ok(s) => s.clone(),
            Err(_) => return Verified::Unavailable("verifier lock poisoned".into()),
        };
        match current.verify(proof, commitment) {
            Verified::Unavailable(e) => {
                tracing::warn!(%e, "proof verifier stopped; starting a new one");
                let fresh = match Sidecar::spawn(&self.bin, &self.dir) {
                    Ok(f) => f,
                    Err(e) => return Verified::Unavailable(e),
                };
                let fresh = Arc::new(fresh);
                match self.sidecar.lock() {
                    Ok(mut slot) => *slot = fresh.clone(),
                    Err(_) => return Verified::Unavailable("verifier lock poisoned".into()),
                }
                fresh.verify(proof, commitment)
            }
            answered => answered,
        }
    }
}

impl ProofVerifier for Verifier {
    fn program_id(&self) -> Option<String> {
        Some(self.program())
    }

    fn decide(&self, proof: &[u8], commitment: [u8; 32]) -> Option<bool> {
        let mut h = blake3::Hasher::new();
        h.update(&commitment).update(proof);
        let key = *h.finalize().as_bytes();
        if let Some(v) = self.seen.lock().ok().and_then(|s| s.get(&key).copied()) {
            return Some(v);
        }
        match self.ask(proof, commitment) {
            // Only a verified proof is remembered: a refusal may have been a
            // transient failure, and caching it would split this node from the rest.
            Verified::Valid => {
                if let Ok(mut s) = self.seen.lock() {
                    if s.len() > 4096 {
                        s.clear();
                    }
                    s.insert(key, true);
                }
                Some(true)
            }
            Verified::Invalid => Some(false),
            Verified::Unavailable(e) => {
                tracing::error!(%e, "proof verifier unavailable; refusing to judge");
                None
            }
        }
    }

    fn verify(&self, proof: &[u8], commitment: [u8; 32]) -> bool {
        self.decide(proof, commitment).unwrap_or(false)
    }
}

/// What the proving side has done, for `aether_proverStatus` and the app's menu bar.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Status {
    pub running: bool,
    pub program: String,
    /// The block being proven now.
    pub proving: Option<u64>,
    pub last_height: Option<u64>,
    pub last_txs: usize,
    pub last_seconds: f64,
    pub proofs: u64,
    /// Proofs admitted to this node's pool or accepted by the upstream.
    pub accepted: u64,
    /// Definite cryptographic refusals (transient transport errors excluded).
    pub verification_failures: u64,
    /// Acceptance over the last 16 definite submission outcomes.
    pub acceptance_rate_percent: Option<u8>,
    /// At least eight recent outcomes, with fewer than one in four accepted.
    pub proofs_failing: bool,
    /// The validator's program differs from this prover.
    pub program_mismatch: bool,
    /// The validator's program could not be read yet.
    pub program_unknown: bool,
    /// Submission outcomes fell below the acceptance floor.
    pub acceptance_failing: bool,
    /// Guest program reported by the validator this follower submits to.
    pub network_program: Option<String>,
    pub error: Option<String>,
    /// Where rewards go.
    pub payout: Option<Address>,
    /// Why proving is paused right now ("memory", "pressure", "battery", "disk").
    pub paused: Option<String>,
    /// The sidecar's physical footprint at the last watchdog sample.
    pub memory_bytes: Option<u64>,
    /// The footprint cap the watchdog enforces.
    pub memory_cap: u64,
    /// Worker threads the sidecar proves with.
    pub threads: usize,
}

pub type SharedStatus = Arc<Mutex<Status>>;

#[derive(Default)]
struct SubmissionHealth {
    recent: VecDeque<bool>,
}

impl SubmissionHealth {
    /// Returns true only when the failing signal first turns on.
    fn observe(&mut self, result: &Result<(), String>, status: &mut Status) -> bool {
        let accepted = match result {
            Ok(()) => {
                status.accepted += 1;
                true
            }
            Err(e) if verifier_refusal(e) => {
                status.verification_failures += 1;
                false
            }
            Err(_) => return false,
        };
        if self.recent.len() == 16 {
            self.recent.pop_front();
        }
        self.recent.push_back(accepted);
        let successes = self.recent.iter().filter(|&&ok| ok).count();
        status.acceptance_rate_percent = Some((100 * successes / self.recent.len()) as u8);
        let was_failing = status.proofs_failing;
        status.acceptance_failing = self.recent.len() >= 8 && successes * 4 < self.recent.len();
        status.proofs_failing = status.program_mismatch || status.acceptance_failing;
        status.proofs_failing && !was_failing
    }
}

fn verifier_refusal(error: &str) -> bool {
    let mut message = error.to_string();
    // An HTTP follower may wrap a validator's JSON-RPC error in another
    // JSON-RPC error. Unwrap only bounded, well-formed server messages.
    for _ in 0..8 {
        if matches!(message.as_str(), "the proof does not verify" | "this proof was already refused") {
            return true;
        }
        let Ok(v) = serde_json::from_str::<Value>(&message) else { return false };
        let Some(next) = v["message"].as_str() else { return false };
        message = next.to_string();
    }
    false
}

/// A kill waits before the sidecar restarts: a minute, doubling, up to half
/// an hour (the incident prover ate 14 GB of a 64 GB Mac).
fn backoff_secs(kills: u32) -> std::time::Duration {
    std::time::Duration::from_secs((60u64 << kills.saturating_sub(1).min(5)).min(1800))
}

/// The proving sidecar under the memory watchdog: the service proves with it,
/// and every couple of seconds its physical footprint is sampled and, past the
/// cap, the process is killed (by pid — a request holds the sidecar's io lock
/// for up to half an hour) and restarted after a growing back-off.
struct Gate {
    bin: PathBuf,
    dir: PathBuf,
    sidecar: Mutex<Arc<Sidecar>>,
    /// The sidecar process is gone; the service restarts it when the back-off allows.
    dead: std::sync::atomic::AtomicBool,
    /// The footprint cap (0: no watching; the prover never runs then).
    cap: u64,
    /// Memory kills since the last proof that came back.
    kills: std::sync::atomic::AtomicU32,
    backoff_until: Mutex<Option<std::time::Instant>>,
}

impl Gate {
    fn current(&self) -> Arc<Sidecar> {
        self.sidecar.lock().expect("prover gate").clone()
    }

    /// A memory kill is counted and its back-off set; the process dies now.
    fn check(&self, status: &SharedStatus) {
        let sidecar = self.current();
        let memory = crate::resources::footprint(sidecar.pid);
        status.lock().map(|mut s| s.memory_bytes = memory).ok();
        let Some(memory) = memory else { return };
        if self.cap == 0 || memory <= self.cap || self.dead.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let kills = self.kills.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let wait = backoff_secs(kills);
        *self.backoff_until.lock().expect("prover gate") = Some(std::time::Instant::now() + wait);
        tracing::warn!(
            memory_bytes = memory,
            cap = self.cap,
            kills,
            backoff_secs = wait.as_secs(),
            "prover over its memory cap: killed; it restarts after a growing back-off"
        );
        sidecar.kill_hard();
        self.dead.store(true, std::sync::atomic::Ordering::Relaxed);
        status
            .lock()
            .map(|mut s| {
                s.proving = None;
                s.paused = Some("memory".into());
                s.error = Some(format!("prover killed at {} bytes (cap {})", gbytes(memory), gbytes(self.cap)));
            })
            .ok();
    }

    /// Critical system pressure: kill the running proof too, without a memory
    /// back-off (the monitor's own pause keeps new jobs from starting).
    fn kill_running(&self) {
        let sidecar = self.current();
        if self.dead.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        tracing::warn!("critical memory pressure: killing the running proof");
        sidecar.kill_hard();
    }

    /// A proof came back: the next memory kill backs off from a minute again.
    fn proved(&self) {
        self.kills.store(0, std::sync::atomic::Ordering::Relaxed);
        *self.backoff_until.lock().expect("prover gate") = None;
    }

    fn blocked(&self) -> bool {
        self.backoff_until
            .lock()
            .expect("prover gate")
            .is_some_and(|t| std::time::Instant::now() < t)
    }

    /// Restart a dead sidecar once the back-off allows.
    fn ensure(&self) {
        if !self.dead.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        match Sidecar::spawn_prover(&self.bin, &self.dir) {
            Ok(fresh) => {
                *self.sidecar.lock().expect("prover gate") = Arc::new(fresh);
                self.dead.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            Err(e) => tracing::warn!(%e, "prover restart failed; trying again"),
        }
    }
}

fn gbytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / crate::resources::GB as f64)
}

/// Prove the newest finalized block nobody has proven yet, again and again,
/// and hand each proof to `submit` (this node's proof pool, or its upstream).
pub fn spawn_service(
    chain: Chain,
    bin: PathBuf,
    dir: PathBuf,
    sidecar: Sidecar,
    prover: Address,
    status: SharedStatus,
    network_program: impl Fn() -> Result<String, String> + Send + 'static,
    submit: impl Fn(ProofClaim) -> Result<(), String> + Send + 'static,
) {
    let program = sidecar.program.clone();
    let gate = Arc::new(Gate {
        cap: crate::resources::monitor()
            .map(|m| m.limits.prover_max_memory)
            .unwrap_or_else(|| crate::resources::default_prover_cap()),
        bin,
        dir,
        sidecar: Mutex::new(Arc::new(sidecar)),
        dead: std::sync::atomic::AtomicBool::new(false),
        kills: std::sync::atomic::AtomicU32::new(0),
        backoff_until: Mutex::new(None),
    });
    let threads = crate::resources::monitor()
        .map(|m| m.limits.prover_threads)
        .unwrap_or_else(|| crate::resources::Limits::default().prover_threads);
    status
        .lock()
        .map(|mut s| {
            s.running = true;
            s.program = program;
            s.payout = Some(prover);
            s.memory_cap = gate.cap;
            s.threads = threads;
        })
        .ok();
    // The watchdog: sample the sidecar's footprint every 2 s, kill it past the
    // cap, and kill a running proof when the system hits critical pressure.
    {
        let (gate, status) = (gate.clone(), status.clone());
        std::thread::Builder::new()
            .name("prover-watchdog".into())
            .spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if crate::resources::monitor().is_some_and(|m| m.critical()) {
                    gate.kill_running();
                }
                gate.check(&status);
            })
            .expect("start the prover watchdog");
    }
    // Proofs not yet accepted, retried until they are or their block is no longer open.
    let mut unsent: Vec<ProofClaim> = Vec::new();
    let mut health = SubmissionHealth::default();
    let mut last_program_check: Option<std::time::Instant> = None;
    let mut compatible = false;
    std::thread::spawn(move || loop {
        let interval = std::time::Duration::from_secs(if compatible { 30 } else { 5 });
        if last_program_check.is_none_or(|checked| checked.elapsed() >= interval) {
            let program = network_program();
            compatible = status.lock().map(|mut s| program_health(&mut s, program)).unwrap_or(false);
            last_program_check = Some(std::time::Instant::now());
        }
        if !compatible {
            std::thread::sleep(std::time::Duration::from_secs(5));
            continue;
        }
        unsent.retain(|c| chain.proof_open(c.height) && submit(c.clone()).is_err());
        // The system first: memory pressure, swap, battery, or a full disk.
        if let Some(reason) = crate::resources::monitor().and_then(|m| m.proving_pause()) {
            pause(&status, reason);
            std::thread::sleep(std::time::Duration::from_secs(5));
            continue;
        }
        if gate.blocked() {
            pause(&status, "memory");
            std::thread::sleep(std::time::Duration::from_secs(5));
            continue;
        }
        status.lock().map(|mut s| s.paused = None).ok();
        gate.ensure();
        let Some((height, txs, input)) = next_job(&chain, prover) else {
            std::thread::sleep(std::time::Duration::from_secs(if unsent.is_empty() { 1 } else { 5 }));
            continue;
        };
        status.lock().map(|mut s| s.proving = Some(height)).ok();
        let bytes = postcard::to_allocvec(&input).expect("input encodes");
        match gate.current().prove(&bytes) {
            Ok((proof, _, seconds)) => {
                gate.proved();
                let claim = ProofClaim { height, prover, proof: hex::encode(proof) };
                let submission = submit(claim.clone());
                let alert = status.lock().map(|mut s| health.observe(&submission, &mut s)).unwrap_or(false);
                if alert {
                    tracing::warn!(
                        height,
                        acceptance_rate_percent = status.lock().ok().and_then(|s| s.acceptance_rate_percent),
                        "proofs failing: recent proof acceptance rate fell below 25%"
                    );
                }
                if let Err(e) = submission {
                    if verifier_refusal(&e) {
                        tracing::warn!(height, %e, "proof rejected by verifier; will retry while the claim is open");
                    } else {
                        tracing::warn!(height, %e, "proof not accepted yet; will retry");
                    }
                    unsent.push(claim);
                }
                status
                    .lock()
                    .map(|mut s| {
                        s.proving = None;
                        s.last_height = Some(height);
                        s.last_txs = txs;
                        s.last_seconds = seconds;
                        s.proofs += 1;
                        s.error = None;
                    })
                    .ok();
                tracing::info!(height, txs, seconds, "proved block");
            }
            Err(e) => {
                status.lock().map(|mut s| (s.proving, s.error) = (None, Some(e.clone()))).ok();
                tracing::warn!(height, %e, "proving failed");
                if e.contains("exited") {
                    // The loop restarts it — after the back-off, if the watchdog killed it.
                    gate.dead.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
    });
}

/// A local pin only says the node and its own sidecar match. Followers must
/// also agree with the validators' verifier program before spending work on a
/// proof. Missing/old RPC answers fail closed until compatibility is known.
fn program_health(status: &mut Status, network: Result<String, String>) -> bool {
    let was_program_paused = status.paused.as_deref() == Some("program");
    status.network_program = network.as_ref().ok().cloned();
    status.program_mismatch = network.as_ref().is_ok_and(|program| program != &status.program);
    status.program_unknown = network.is_err();
    let problem = match network {
        Ok(ref program) if program == &status.program => None,
        Ok(program) => Some(format!("proof program mismatch: local {}, validator {program}", status.program)),
        Err(e) => Some(format!("cannot confirm validator proof program: {e}")),
    };
    if let Some(problem) = problem {
        if !was_program_paused {
            tracing::warn!(%problem, "proving paused until validator program is confirmed compatible");
        }
        status.paused = Some("program".into());
        status.error = Some(problem);
        status.proofs_failing = status.program_mismatch || status.acceptance_failing;
        return false;
    }
    if status.paused.as_deref() == Some("program") {
        status.paused = None;
        status.error = None;
        tracing::info!(program = %status.program, "validator proof program matches; proving resumed");
    }
    status.program_mismatch = false;
    status.program_unknown = false;
    status.proofs_failing = status.acceptance_failing;
    true
}

/// Mark proving paused for `reason`, logged when the reason changes.
fn pause(status: &SharedStatus, reason: &str) {
    status
        .lock()
        .map(|mut s| {
            if s.paused.as_deref() != Some(reason) {
                tracing::warn!(reason, "proving paused");
            }
            s.paused = Some(reason.to_string());
        })
        .ok();
}

/// The newest finalized block still unproven whose parent state this node holds.
fn next_job(chain: &Chain, prover: Address) -> Option<(u64, usize, aether_proving::block::BlockInput)> {
    let (exec, parent, block) = chain.provable()?;
    let payload = block.payload()?;
    let (pre, _) = chain.pre_state_with(&parent, payload.version, &payload.proofs, &payload.beacons, &payload.registrations, payload.seed.as_ref(), true).ok()?;
    let ctx = Chain::block_context(&chain.cfg(), &block, &parent);
    let input = aether_proving::block::input(&pre, &ctx, &payload.txs, &[], prover).ok()?;
    debug_assert_eq!(aether_proving::block::execute(&input).ok()?.commitment(), exec.statement.commitment);
    Some((exec.height, payload.txs.len(), input))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_verifier_refusals_make_proving_unhealthy_until_acceptance_recovers() {
        let mut health = SubmissionHealth::default();
        let mut status = Status::default();
        for _ in 0..7 {
            assert!(!health.observe(&Err("the proof does not verify".into()), &mut status));
        }
        assert!(!status.proofs_failing);
        assert!(health.observe(&Err("the proof does not verify".into()), &mut status));
        assert!(status.proofs_failing);
        assert_eq!(status.acceptance_rate_percent, Some(0));
        assert_eq!(status.verification_failures, 8);
        for _ in 0..8 {
            health.observe(&Ok(()), &mut status);
        }
        assert!(!status.proofs_failing);
        assert_eq!(status.accepted, 8);
        assert_eq!(status.acceptance_rate_percent, Some(50));
    }

    #[test]
    fn transient_submission_errors_do_not_count_as_verifier_refusals() {
        let mut health = SubmissionHealth::default();
        let mut status = Status::default();
        for _ in 0..20 {
            health.observe(&Err("busy; try again shortly".into()), &mut status);
        }
        assert_eq!(status.acceptance_rate_percent, None);
        assert_eq!(status.verification_failures, 0);
        assert!(!status.proofs_failing);
    }

    #[test]
    fn follower_rpc_refusal_counts_as_a_verification_failure() {
        let wrapped = r#"{"code":-32000,"message":"the proof does not verify"}"#;
        assert!(verifier_refusal(wrapped));
        let relayed = serde_json::json!({"code": -32000, "message": wrapped}).to_string();
        assert!(verifier_refusal(&relayed));
        assert!(!verifier_refusal(r#"{"code":-32000,"message":"busy; try again shortly"}"#));
        let mut health = SubmissionHealth::default();
        let mut status = Status::default();
        for _ in 0..8 {
            health.observe(&Err(wrapped.into()), &mut status);
        }
        assert!(status.proofs_failing);
        assert_eq!(status.verification_failures, 8);
    }

    #[test]
    fn follower_pauses_on_a_different_or_unknown_validator_program() {
        let mut status = Status { program: "local".into(), running: true, ..Status::default() };
        assert!(!program_health(&mut status, Ok("validator".into())));
        assert_eq!(status.paused.as_deref(), Some("program"));
        assert!(status.proofs_failing);
        assert_eq!(status.network_program.as_deref(), Some("validator"));
        assert!(!program_health(&mut status, Err("old validator RPC".into())));
        assert_eq!(status.network_program, None);
        assert!(status.program_unknown);
        assert!(!status.program_mismatch);
        assert!(program_health(&mut status, Ok("local".into())));
        assert_eq!(status.paused, None);
        assert!(!status.proofs_failing);
    }

    /// A fake prover that speaks the sidecar handshake and then balloons past
    /// any small cap — enough like the 14 GB incident to test the watchdog.
    /// It names the pinned program when this build pins one (`option_env!` is
    /// set by the release env even for the test binary).
    fn hog(dir: &Path) -> PathBuf {
        let p = dir.join("hog.sh");
        let program = PROGRAM.unwrap_or("00");
        std::fs::write(
            &p,
            format!(
                concat!(
                    "#!/bin/sh\n",
                    "echo '{{\"guest_elf_sha256\":\"{program}\"}}'\n",
                    // exec: the memory lands in this pid, not in a child the
                    // watchdog's footprint sample would never see.
                    "exec awk 'BEGIN{{x=\"A\"; while(length(x)<268435456) x=x x; sleep 300}}'\n",
                ),
                program = program,
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    fn gate(bin: PathBuf, dir: PathBuf, sidecar: Arc<Sidecar>, cap: u64) -> Gate {
        Gate {
            bin,
            dir,
            sidecar: Mutex::new(sidecar),
            dead: std::sync::atomic::AtomicBool::new(false),
            cap,
            kills: std::sync::atomic::AtomicU32::new(0),
            backoff_until: Mutex::new(None),
        }
    }

    /// The development sidecar is looked up from the running executable, so a
    /// node built in another directory looks next to itself. It used to come
    /// from `env!("CARGO_MANIFEST_DIR")`, which put the builder's absolute path
    /// in the binary: same source, different bytes (gap G5).
    #[test]
    fn the_development_sidecar_is_found_from_the_executable_not_the_checkout() {
        assert_eq!(
            dev_binary(Path::new("/builds/one/target/release/aether")).unwrap(),
            Path::new("/builds/one/target/release/../../apps/prover/target/release/aether-prover")
        );
        assert_eq!(
            dev_binary(Path::new("/somewhere/else/bbbbbbbbbb/target/release/aether")).unwrap(),
            Path::new("/somewhere/else/bbbbbbbbbb/target/release/../../apps/prover/target/release/aether-prover")
        );
    }

    #[test]
    fn the_backoff_grows_a_minute_at_a_time_to_half_an_hour() {
        let secs = |k: u32| backoff_secs(k).as_secs();
        assert_eq!([1, 2, 3, 4, 5, 6, 7, 50].map(secs), [60, 120, 240, 480, 960, 1800, 1800, 1800]);
    }

    /// The watchdog kills a sidecar past its memory cap by pid (without the io
    /// lock), reports it in the status, and backs off before restarting.
    #[test]
    #[cfg(target_os = "macos")]
    fn the_watchdog_kills_a_sidecar_over_its_cap_and_backs_off() {
        const CAP: u64 = 96 * 1024 * 1024;
        let dir = std::env::temp_dir().join(format!("aether-prover-watchdog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = hog(&dir);
        let sc = Arc::new(Sidecar::spawn(&bin, &dir).expect("the fake sidecar starts"));
        let pid = sc.pid;
        let status = SharedStatus::default();
        // A second, uncapped gate over the same sidecar: the under-cap path
        // (sample and no kill) without racing the hog's growth.
        let lenient = gate(bin.clone(), dir.clone(), sc.clone(), u64::MAX);
        let g = gate(bin.clone(), dir.clone(), sc, CAP);
        // Wait for the hog to actually allocate (string doubling to 256 MiB).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while crate::resources::footprint(pid).is_none_or(|m| m < CAP) {
            assert!(std::time::Instant::now() < deadline, "the hog never allocated");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        // With room to spare it stays alive; the sample lands in the status.
        lenient.check(&status);
        assert!(!lenient.dead.load(std::sync::atomic::Ordering::Relaxed));
        assert!(status.lock().unwrap().memory_bytes.is_some_and(|m| m > CAP));

        // The kill: past the cap the process dies, the status says why, and a
        // back-off blocks the next start.
        g.check(&status);
        assert!(g.dead.load(std::sync::atomic::Ordering::Relaxed));
        assert!(g.blocked(), "a memory kill waits out its back-off");
        let s = status.lock().unwrap();
        assert_eq!(s.paused.as_deref(), Some("memory"));
        assert!(s.memory_bytes.unwrap() > CAP);
        drop(s);
        assert_eq!(g.kills.load(std::sync::atomic::Ordering::Relaxed), 1);
        // Really gone: the footprint falls away (a zombie holds nothing).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while crate::resources::footprint(pid).is_some_and(|m| m > CAP) {
            assert!(std::time::Instant::now() < deadline, "the hog survived the kill");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // A proof that comes back resets the back-off ladder...
        g.proved();
        assert!(!g.blocked());
        // ...and `ensure` restarts a dead sidecar once the back-off allows.
        g.ensure();
        assert!(!g.dead.load(std::sync::atomic::Ordering::Relaxed));
        assert!(crate::resources::footprint(g.current().pid).is_some(), "a fresh sidecar is running");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod sidecar_fault_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};

    /// An executable stand-in for `aether-prover serve`: prints the info line
    /// (echoing the pinned program when the build pins one), then answers
    /// every verify request with `answer` — a shell snippet, so a test plays
    /// the sidecar faults: a verdict, a refusal, a hang, an exit.
    fn fake_prover(dir: &Path, answer: &str) -> PathBuf {
        let program = PROGRAM.unwrap_or("any-program");
        let path = dir.join(format!("fake-{}", unique()));
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\necho '{{\"guest_elf_sha256\":\"{program}\"}}'\n\
                 while IFS= read -r line; do\n  case \"$line\" in\n    *verify*) {answer} ;;\n    \
                 *) echo '{{\"ok\":false,\"error\":\"no such command\"}}' ;;\n  esac\ndone\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-prover-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Red team #20: "could not check" is never "checked and false". A
    /// verdict and a refusal are answers; a sidecar that hangs, dies, or
    /// cannot read the proof leaves no verdict at all — the caller refuses to
    /// judge instead of rejecting.
    #[test]
    fn a_proof_check_has_three_answers_not_two() {
        let dir = scratch("3v");
        std::env::set_var("AETHER_VERIFY_TIMEOUT_MS", "200");
        let commit = [7u8; 32];

        let valid = Sidecar::spawn(&fake_prover(&dir, r#"echo '{"ok":true,"verified":true,"seconds":0.1}'"#), &dir).unwrap();
        assert_eq!(valid.verify(b"a proof", commit), Verified::Valid);

        let invalid = Sidecar::spawn(&fake_prover(&dir, r#"echo '{"ok":true,"verified":false}'"#), &dir).unwrap();
        assert_eq!(invalid.verify(b"a proof", commit), Verified::Invalid);

        // The sidecar could not read what it was asked to judge: not a verdict.
        let unreadable = Sidecar::spawn(&fake_prover(&dir, r#"echo '{"ok":false,"error":"read /tmp/1-0.verify: No such file or directory"}'"#), &dir).unwrap();
        assert!(matches!(unreadable.verify(b"a proof", commit), Verified::Unavailable(_)));
        let unknown = Sidecar::spawn(&fake_prover(&dir, r#"echo '{"ok":false,"error":"request panicked"}'"#), &dir).unwrap();
        assert!(matches!(unknown.verify(b"a proof", commit), Verified::Unavailable(_)));

        // A hung sidecar is not waited on past the timeout, and not read as a refusal.
        let hung = Sidecar::spawn(&fake_prover(&dir, "sleep 5"), &dir).unwrap();
        assert!(matches!(hung.verify(b"a proof", commit), Verified::Unavailable(_)));

        // A dead one — it exits instead of answering.
        let dead = Sidecar::spawn(&fake_prover(&dir, "exit 0"), &dir).unwrap();
        assert!(matches!(dead.verify(b"a proof", commit), Verified::Unavailable(_)));

        std::env::remove_var("AETHER_VERIFY_TIMEOUT_MS");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A sidecar whose first line is unusable is refused *and* stopped: a
    /// sidecar the node will not talk to must not keep running.
    #[test]
    fn a_sidecar_that_reports_nonsense_is_refused_and_not_left_running() {
        let dir = scratch("spawn");
        for (name, line, why) in [
            ("garbled", "not json", "sidecar info"),
            ("mute-program", r#"{"ready":true}"#, "did not report its program"),
        ] {
            let pidfile = dir.join(format!("{name}.pid"));
            let script = dir.join(name);
            std::fs::write(
                &script,
                format!("#!/bin/sh\necho $$ > {}\necho '{line}'\nsleep 5\n", pidfile.display()),
            )
            .unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

            let err = Sidecar::spawn(&script, &dir).unwrap_err();
            assert!(err.contains(why), "{name}: {err}");
            let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while unsafe { libc::kill(pid, 0) == 0 } && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            assert!(unsafe { libc::kill(pid, 0) == -1 }, "{name}: the refused sidecar was stopped, not left running");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A refusal is an answer, asked once; an unavailability is retried on a
    /// fresh sidecar, once — and only the second ask is a second request.
    #[test]
    fn an_invalid_proof_is_answered_once_and_an_unavailable_one_retries() {
        let dir = scratch("retry");
        let commit = [7u8; 32];
        // One counter per stand-in: its lines are the requests this bin served.
        let fake = |name: &str, reply: &str| {
            let counter = dir.join(format!("{name}.asks"));
            fake_prover(&dir, &format!("echo ask >> {}; {reply}", counter.display()))
        };

        let v = Verifier::start(
            &fake("refused", r#"echo '{"ok":true,"verified":false}'"#),
            &dir,
        )
        .unwrap();
        assert_eq!(v.decide(b"a proof", commit), Some(false));
        assert_eq!(std::fs::read_to_string(dir.join("refused.asks")).unwrap(), "ask\n", "a refusal is never re-asked");

        let v = Verifier::start(
            &fake("unreadable", r#"echo '{"ok":false,"error":"read /tmp/9-9.verify: No such file"}'"#),
            &dir,
        )
        .unwrap();
        assert_eq!(v.decide(b"another proof", commit), None, "no verdict after the retry");
        assert_eq!(
            std::fs::read_to_string(dir.join("unreadable.asks")).unwrap(),
            "ask\nask\n",
            "one ask on the dead sidecar, one on its replacement"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
