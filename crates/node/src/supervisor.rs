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
//! registry epoch tries again. No person acts at any step.

use crate::roster::{EpochStart, Member, NetworkFile};
use crate::rotation::{STAGED_NETWORK, STAGED_THRESHOLD};
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
    /// How long a background reshare may take.
    pub reshare_timeout: Duration,
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

/// A background reshare for one registry epoch's proposal.
struct Reshare {
    child: Child,
    started: Instant,
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
    let quick = exits
        .iter()
        .rev()
        .take_while(|e| e.at_ms.saturating_sub(e.started_ms) < PROGRESS_MS)
        .count();
    Next::Again(Duration::from_millis((FIRST_BACKOFF_MS << quick.saturating_sub(1).min(6)).min(MAX_BACKOFF_MS)))
}

/// `<data>/run-state.json`: an unreadable history cannot be treated as a
/// fresh one, since that would let a crash loop reset its restart budget.
fn load_exits(data: &Path) -> Result<Vec<ExitNote>, String> {
    match std::fs::read(data.join("run-state.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("run-state.json: {e}")),
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

impl Supervisor {
    fn network_path(&self) -> PathBuf {
        self.data.join("network.json")
    }

    fn rpc(&self) -> String {
        format!("http://127.0.0.1:{}", self.rpc_port)
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
                if let Some(peers) = self.tcp_peers(&NetworkFile::load(&net)?.validators, me.expect("a validator has its key"), false)
                {
                    cmd.args(["--peers", &peers, "--offline"]);
                }
                cmd.args(&self.node_args);
            }
            Role::Candidate | Role::Paused | Role::Keyless => {
                cmd.args([
                    "follow",
                    "--exit-with-parent",
                    "--network",
                    &path_str(&net),
                    "--data",
                    &path_str(&self.data.join("follow")),
                ])
                .args(["--rpc-port", &self.rpc_port.to_string(), "--checkpoint"]);
                if role == Role::Candidate {
                    // The candidate's beacon keys are the Mac's own; the keyless
                    // follower has none to send.
                    cmd.args(["--keys", &path_str(&self.data), "--candidate"]);
                }
                cmd.args(&self.follow_args);
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
            // A completed handoff must be installed before any child reads
            // network.json or its share. If disk failure prevents that, keep
            // the validator stopped rather than starting with a mixed pair.
            if let Err(e) = finish_incomplete(&self.data) {
                tracing::error!(%e, "a completed handoff cannot be installed; keeping the signer stopped");
                std::process::exit(crate::store::EXIT_STORAGE);
            }
            let role = self.role(me.as_deref())?;
            let started_ms = now_ms();
            let mut child = self.spawn(role, me.as_deref())?;
            match self.watch(&mut child, role, me.as_deref()) {
                Watched::Switched => {
                    // The restart is the role change itself, not a crash.
                    me = self.my_key();
                    exits.clear();
                    let _ = std::fs::remove_file(self.data.join("run-state.json"));
                }
                Watched::Exited(status) => {
                    exits.push(ExitNote { started_ms, at_ms: now_ms(), code: status.code() });
                    exits.retain(|e| now_ms().saturating_sub(e.at_ms) <= WINDOW_MS);
                    if let Err(e) = crate::atomic::replace(
                        &self.data.join("run-state.json"),
                        &serde_json::to_vec(&exits).unwrap_or_default(),
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
            // 1. A proposed voting set: reshare to it in the background, once per registry epoch.
            if reshare.is_none() {
                if let Ok(rot) = rpc_call(&rpc, "aether_rotation", json!([])) {
                    let epoch = rot["epoch"].as_u64();
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
                    if !rot.is_null() && involved && epoch != attempted {
                        attempted = epoch;
                        match self.start_reshare(role, me.expect("involved means keyed"), &rot) {
                            Ok(c) => {
                                reshare = Some(Reshare {
                                    child: c,
                                    started: Instant::now(),
                                })
                            }
                            Err(e) => {
                                tracing::warn!(%e, "aether run: cannot reshare to the proposed set")
                            }
                        }
                    }
                }
            }
            if let Some(r) = reshare.as_mut() {
                match r.child.try_wait() {
                    Ok(Some(status)) => {
                        tracing::info!(%status, "aether run: background reshare finished");
                        reshare = None;
                    }
                    _ if r.started.elapsed() > self.reshare_timeout => {
                        tracing::warn!(
                            "aether run: background reshare timed out; the running set carries on"
                        );
                        stop(&mut reshare);
                    }
                    _ => {}
                }
            }
            // 2. A staged reshare: sign its handoff (again now and then, for peers that missed it).
            if role == Role::Validator
                && self.data.join(STAGED_THRESHOLD).exists()
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
                    stop(&mut reshare);
                    // The old validator must not keep signing while its share
                    // and network pointer are changed on disk.
                    let _ = child.kill();
                    let status = child.wait().expect("a stopped child can be reaped");
                    match self.install(role, me, &h) {
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

    fn start_reshare(&self, role: Role, me: &str, rot: &Value) -> Result<Child, String> {
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
    /// file changes, so no signer can observe an incomplete pair.
    fn install(&self, role: Role, me: &str, h: &Value) -> Result<(), String> {
        let switch = h["switch"].as_u64().ok_or("handoff switch")?;
        let round = h["round"].as_u64().ok_or("handoff round")?;
        let output = h["output"].as_str().ok_or("handoff output")?.to_string();
        let members: Vec<Member> = serde_json::from_value(h["members"].clone())
            .map_err(|e| format!("handoff members: {e}"))?;
        let end = switch - 1;
        let end_hash = rpc_call(&self.rpc(), "aether_getBlock", json!([end]))?["hash"]
            .as_str()
            .ok_or("no block before the switch")?
            .to_string();
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
        //    that can fail (the anchor proof above all) fails here, with the
        //    old world still in place.
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
            let fin = rpc_call(&self.rpc(), "aether_getFinalized", json!([end]))?;
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
            self.data.join("follow").join("state.redb"),
            self.data.join("state.redb"),
        )
        .map_err(|e| format!("follower state: {e}"))?;
        Ok(())
    }

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
    let theirs = NetworkFile::load(src)?;
    if let Ok(current) = NetworkFile::load(&ours) {
        if current.chain_id == theirs.chain_id && current.identity == theirs.identity {
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
            genesis_validators: None,
        }
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
            reshare_timeout: Duration::from_secs(1),
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

    /// The history persists, and a damaged history file is a fresh one.
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
