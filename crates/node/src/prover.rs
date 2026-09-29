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
    pub error: Option<String>,
    /// Where rewards go.
    pub payout: Option<Address>,
}

pub type SharedStatus = Arc<Mutex<Status>>;

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
    status
        .lock()
        .map(|mut s| {
            s.running = true;
            s.program = sidecar.program.clone();
            s.payout = Some(prover);
        })
        .ok();
    let mut sidecar = sidecar;
    // Proofs not yet accepted, retried until they are or their block is no longer open.
    let mut unsent: Vec<ProofClaim> = Vec::new();
    std::thread::spawn(move || loop {
        unsent.retain(|c| chain.proof_open(c.height) && submit(c.clone()).is_err());
        let Some((height, txs, input)) = next_job(&chain, prover) else {
            std::thread::sleep(std::time::Duration::from_secs(if unsent.is_empty() { 1 } else { 5 }));
            continue;
        };
        status.lock().map(|mut s| s.proving = Some(height)).ok();
        let bytes = postcard::to_allocvec(&input).expect("input encodes");
        match sidecar.prove(&bytes) {
            Ok((proof, _, seconds)) => {
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
                    match Sidecar::spawn(&bin, &dir) {
                        Ok(fresh) => sidecar = fresh,
                        Err(e) => tracing::warn!(%e, "prover restart failed"),
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
    });
}

/// The newest finalized block still unproven whose parent state this node holds.
fn next_job(chain: &Chain, prover: Address) -> Option<(u64, usize, aether_proving::block::BlockInput)> {
    let (exec, parent, block) = chain.provable()?;
    let payload = block.payload()?;
    let (pre, _) = chain.pre_state_with(&parent, payload.version, &payload.proofs, &payload.beacons, payload.seed.as_ref(), true).ok()?;
    let ctx = Chain::block_context(&chain.cfg(), &block, &parent);
    let input = aether_proving::block::input(&pre, &ctx, &payload.txs, &[], prover).ok()?;
    debug_assert_eq!(aether_proving::block::execute(&input).ok()?.commitment(), exec.statement.commitment);
    Some((exec.height, payload.txs.len(), input))
}

#[cfg(test)]
mod tests {
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
