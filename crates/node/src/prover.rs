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

/// One `aether-prover serve` process.
pub struct Sidecar {
    io: Mutex<Io>,
    dir: PathBuf,
    /// SHA-256 of the guest ELF it proves and verifies against.
    pub program: String,
}

impl Sidecar {
    pub fn spawn(bin: &Path, dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut child = Command::new(bin)
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("start {}: {e}", bin.display()))?;
        let stdin = child.stdin.take().ok_or("no sidecar stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or("no sidecar stdout")?);
        let mut first = String::new();
        stdout.read_line(&mut first).map_err(|e| e.to_string())?;
        let info: Value = serde_json::from_str(&first).map_err(|e| format!("sidecar info: {e}"))?;
        let program = info["guest_elf_sha256"].as_str().ok_or("sidecar did not report its program")?.to_string();
        if let Some(pinned) = PROGRAM.filter(|p| *p != program) {
            return Err(format!("the sidecar proves program {program}, the protocol pins {pinned}"));
        }
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in stdout.lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Sidecar { io: Mutex::new(Io { child, stdin, lines }), dir: dir.to_path_buf(), program })
    }

    fn request(&self, req: Value, timeout: std::time::Duration) -> Result<Value, String> {
        let mut io = self.io.lock().map_err(|_| "sidecar lock poisoned")?;
        writeln!(io.stdin, "{req}").and_then(|_| io.stdin.flush()).map_err(|e| format!("sidecar: {e}"))?;
        let line = match io.lines.recv_timeout(timeout) {
            Ok(line) => line,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Hung: stop it so the next request starts a fresh one.
                let _ = io.child.kill();
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
    fn verify(&self, proof: &[u8], commitment: [u8; 32]) -> bool {
        let mut h = blake3::Hasher::new();
        h.update(&commitment).update(proof);
        let key = *h.finalize().as_bytes();
        if let Some(v) = self.seen.lock().ok().and_then(|s| s.get(&key).copied()) {
            return v;
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
                true
            }
            Ok(false) => false,
            Err(e) => {
                tracing::error!(%e, "proof verifier unavailable; refusing blocks with proofs");
                false
            }
        }
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
}

pub type SharedStatus = Arc<Mutex<Status>>;

/// Prove the newest finalized block nobody has proven yet, again and again,
/// and hand each proof to `submit` (this node's proof pool, or its upstream).
pub fn spawn_service(chain: Chain, sidecar: Sidecar, prover: Address, status: SharedStatus, submit: impl Fn(ProofClaim) + Send + 'static) {
    status
        .lock()
        .map(|mut s| {
            s.running = true;
            s.program = sidecar.program.clone();
            s.payout = Some(prover);
        })
        .ok();
    std::thread::spawn(move || loop {
        let Some((height, txs, input)) = next_job(&chain, prover) else {
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        };
        status.lock().map(|mut s| s.proving = Some(height)).ok();
        let bytes = postcard::to_allocvec(&input).expect("input encodes");
        match sidecar.prove(&bytes) {
            Ok((proof, _, seconds)) => {
                submit(ProofClaim { height, prover, proof: hex::encode(proof) });
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
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
    });
}

/// The newest finalized block still unproven whose parent state this node holds.
fn next_job(chain: &Chain, prover: Address) -> Option<(u64, usize, aether_proving::block::BlockInput)> {
    let (exec, parent, block) = chain.provable()?;
    let payload = block.payload()?;
    let (pre, _) = chain.pre_state(&parent, payload.version, &payload.proofs, true).ok()?;
    let ctx = Chain::block_context(&chain.cfg(), &block, &parent);
    let input = aether_proving::block::input(&pre, &ctx, &payload.txs, &[], prover).ok()?;
    debug_assert_eq!(aether_proving::block::execute(&input).ok()?.commitment(), exec.statement.commitment);
    Some((exec.height, payload.txs.len(), input))
}
