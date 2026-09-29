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
use std::collections::HashMap;
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
    let beside = std::env::current_exe().ok()?.with_file_name("aether-prover");
    if beside.exists() {
        return Some(beside);
    }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/prover/target/release/aether-prover");
    dev.exists().then_some(dev)
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
        let info: Value = serde_json::from_str(&first).map_err(|e| format!("sidecar info: {e}"))?;
        let program = info["guest_elf_sha256"].as_str().ok_or("sidecar did not report its program")?.to_string();
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

    pub fn verify(&self, proof: &[u8], commitment: [u8; 32]) -> Result<bool, String> {
        let path = self.dir.join(format!("{}.verify", unique()));
        std::fs::write(&path, proof).map_err(|e| e.to_string())?;
        let reply = self.request(json!({"cmd": "verify", "proof": path, "commitment": hex::encode(commitment)}), VERIFY_TIMEOUT);
        let _ = std::fs::remove_file(&path);
        match reply {
            Ok(v) => Ok(v["verified"].as_bool() == Some(true)),
            // A rejected proof is an answer; a dead sidecar is not.
            Err(e) if e.contains("exited") || e.contains("sidecar:") => Err(e),
            Err(_) => Ok(false),
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

    /// Ask the sidecar; if it died, start a new one and ask again once.
    fn ask(&self, proof: &[u8], commitment: [u8; 32]) -> Result<bool, String> {
        let current = self.sidecar.lock().map_err(|_| "verifier lock poisoned")?.clone();
        match current.verify(proof, commitment) {
            Err(e) => {
                tracing::warn!(%e, "proof verifier stopped; starting a new one");
                let fresh = Arc::new(Sidecar::spawn(&self.bin, &self.dir)?);
                *self.sidecar.lock().map_err(|_| "verifier lock poisoned")? = fresh.clone();
                fresh.verify(proof, commitment)
            }
            ok => ok,
        }
    }
}

impl ProofVerifier for Verifier {
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
            Ok(true) => {
                if let Ok(mut s) = self.seen.lock() {
                    if s.len() > 4096 {
                        s.clear();
                    }
                    s.insert(key, true);
                }
                Some(true)
            }
            Ok(false) => Some(false),
            Err(e) => {
                tracing::error!(%e, "proof verifier unavailable; refusing blocks with proofs");
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
    std::thread::spawn(move || loop {
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
                if let Err(e) = submit(claim.clone()) {
                    tracing::warn!(height, %e, "proof not accepted yet; will retry");
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
