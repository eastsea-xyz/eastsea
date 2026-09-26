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
}

/// A background reshare for one registry epoch's proposal.
struct Reshare {
    child: Child,
    started: Instant,
}

impl Supervisor {
    fn network_path(&self) -> PathBuf {
        self.data.join("network.json")
    }

    fn rpc(&self) -> String {
        format!("http://127.0.0.1:{}", self.rpc_port)
    }

    fn my_key(&self) -> Result<String, String> {
        let keys = crate::roster::LocalKeys::load(&self.data)?;
        Ok(hex::encode(commonware_cryptography::Signer::public_key(&keys.signer).as_ref()))
    }

    fn role(&self, me: &str) -> Result<Role, String> {
        let net = NetworkFile::load(&self.network_path())?;
        let member = net.validators.iter().any(|m| m.key == me);
        Ok(if member && self.data.join("threshold.json").exists() { Role::Validator } else { Role::Candidate })
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
                let mut it = ports.split_whitespace().filter_map(|p| p.parse::<u16>().ok());
                let (p2p, resh) = (it.next()?, it.next()?);
                Some(format!("{}@127.0.0.1:{}", i + 1, if reshare { resh } else { p2p }))
            })
            .collect();
        Some(list.join(","))
    }

    fn spawn(&self, role: Role, me: &str) -> Result<Child, String> {
        let mut cmd = Command::new(&self.exe);
        let net = self.network_path();
        match role {
            Role::Validator => {
                cmd.args(["node", "--exit-with-parent", "--network", &path_str(&net), "--data", &path_str(&self.data)]).args([
                    "--port",
                    &self.port.to_string(),
                    "--rpc-port",
                    &self.rpc_port.to_string(),
                ]);
                if let Some(peers) = self.tcp_peers(&NetworkFile::load(&net)?.validators, me, false) {
                    cmd.args(["--peers", &peers, "--offline"]);
                }
                cmd.args(&self.node_args);
            }
            Role::Candidate => {
                cmd.args(["follow", "--exit-with-parent", "--network", &path_str(&net), "--data", &path_str(&self.data.join("follow"))])
                    .args(["--keys", &path_str(&self.data), "--rpc-port", &self.rpc_port.to_string(), "--candidate"])
                    .args(&self.follow_args);
            }
        }
        tracing::info!(?role, "aether run: starting");
        cmd.spawn().map_err(|e| format!("spawn {}: {e}", self.exe.display()))
    }

    /// Runs forever: (re)start the child for the current role; follow rotations.
    pub fn run(&self) -> Result<(), String> {
        let me = self.my_key()?;
        if let Some(dir) = &self.dev_peer_dir {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join(&me), format!("{} {}", self.port, self.reshare_port)).map_err(|e| e.to_string())?;
        }
        loop {
            let role = self.role(&me)?;
            let mut child = self.spawn(role, &me)?;
            let switched = self.watch(&mut child, role, &me);
            let _ = child.kill();
            let _ = child.wait();
            if !switched {
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }

    /// Watch the child; returns true after installing a handoff (restart in the
    /// new role), false when the child exited on its own.
    fn watch(&self, child: &mut Child, role: Role, me: &str) -> bool {
        let rpc = self.rpc();
        let mut reshare: Option<Reshare> = None;
        let mut attempted: Option<u64> = None;
        let mut last_sign = Instant::now() - Duration::from_secs(60);
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if child.try_wait().ok().flatten().is_some() {
                tracing::warn!("aether run: child exited; restarting");
                stop(&mut reshare);
                return false;
            }
            // 1. A proposed voting set: reshare to it in the background, once per registry epoch.
            if reshare.is_none() {
                if let Ok(rot) = rpc_call(&rpc, "aether_rotation", json!([])) {
                    let epoch = rot["epoch"].as_u64();
                    let involved = role == Role::Validator || rot["next"].as_array().is_some_and(|n| n.iter().any(|m| m["key"] == me));
                    if !rot.is_null() && involved && epoch != attempted {
                        attempted = epoch;
                        match self.start_reshare(role, me, &rot) {
                            Ok(c) => reshare = Some(Reshare { child: c, started: Instant::now() }),
                            Err(e) => tracing::warn!(%e, "aether run: cannot reshare to the proposed set"),
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
                        tracing::warn!("aether run: background reshare timed out; the running set carries on");
                        stop(&mut reshare);
                    }
                    _ => {}
                }
            }
            // 2. A staged reshare: sign its handoff (again now and then, for peers that missed it).
            if role == Role::Validator && self.data.join(STAGED_THRESHOLD).exists() && last_sign.elapsed() > Duration::from_secs(5) {
                last_sign = Instant::now();
                if let Err(e) = rpc_call(&rpc, "aether_signHandoff", json!([])) {
                    tracing::debug!(%e, "handoff not signed");
                }
            }
            // 3. A finalized handoff whose switch height the chain reached: install the new role.
            if let Ok(h) = rpc_call(&rpc, "aether_handoff", json!([])) {
                if self.handoff_due(&h) {
                    stop(&mut reshare);
                    match self.install(role, me, &h) {
                        Ok(()) => return true,
                        Err(e) => tracing::warn!(%e, "aether run: could not install the handoff"),
                    }
                }
            }
        }
    }

    /// The handoff is new to this Mac and the chain finalized its last block before the switch.
    fn handoff_due(&self, h: &Value) -> bool {
        let (Some(round), Some(switch), Some(finalized)) = (h["round"].as_u64(), h["switch"].as_u64(), h["finalized"].as_u64()) else { return false };
        let ours = NetworkFile::load(&self.network_path()).map(|n| n.round).unwrap_or(0);
        round > ours && finalized + 1 >= switch
    }

    fn start_reshare(&self, role: Role, me: &str, rot: &Value) -> Result<Child, String> {
        let ours = NetworkFile::load(&self.network_path())?;
        let from: NetworkFile = match role {
            Role::Validator => ours.clone(),
            Role::Candidate => {
                // The running set's public file, checked against the identity this Mac pins.
                let f: NetworkFile = serde_json::from_value(rot["network"].clone()).map_err(|e| format!("rotation has no network: {e}"))?;
                if f.identity != ours.identity || f.chain_id != ours.chain_id {
                    return Err("the running set's network file is for another committee".into());
                }
                let out = f.output.clone().ok_or("the running set's network file has no output")?;
                let identity = aether_light::ValidatorSet::from_hex(ours.identity.as_deref().unwrap_or_default()).map_err(|e| format!("{e:?}"))?;
                let decoded = crate::dkg::KeyFile { round: f.round, output: out, identity: String::new(), share: String::new() }
                    .decode_output(f.validators.len() as u32)?;
                if decoded.public().public() != identity.identity() {
                    return Err("the running set's output is for another identity".into());
                }
                f
            }
        };
        let next: Vec<Member> = serde_json::from_value(rot["next"].clone()).map_err(|e| format!("rotation next: {e}"))?;
        let to = NetworkFile { validators: next.clone(), identity: None, output: None, epochs: vec![], ..from.clone() };
        let (from_path, to_path) = (self.data.join("rotation-from.json"), self.data.join("rotation-to.json"));
        std::fs::write(&from_path, serde_json::to_vec_pretty(&from).expect("json")).map_err(|e| e.to_string())?;
        std::fs::write(&to_path, serde_json::to_vec_pretty(&to).expect("json")).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(self.data.join(STAGED_THRESHOLD));
        let _ = std::fs::remove_file(self.data.join(STAGED_NETWORK));
        let mut cmd = Command::new(&self.exe);
        cmd.args(["reshare", "--stage", "--exit-with-parent", "--from", &path_str(&from_path), "--to", &path_str(&to_path)]);
        if role == Role::Validator {
            cmd.arg("--via-node");
        }
        cmd.args(["--port", &self.reshare_port.to_string(), "--data", &path_str(&self.data)]);
        let union: Vec<Member> = from.validators.iter().cloned().chain(next.iter().filter(|m| !from.validators.contains(m)).cloned()).collect();
        if let Some(peers) = self.tcp_peers(&union, me, true) {
            cmd.args(["--peers", &peers, "--offline"]);
        }
        tracing::info!(members = next.len(), "aether run: resharing to the proposed voting set in the background");
        cmd.spawn().map_err(|e| e.to_string())
    }

    /// Take this Mac's role in the new voting set: files only; the caller restarts.
    fn install(&self, role: Role, me: &str, h: &Value) -> Result<(), String> {
        let switch = h["switch"].as_u64().ok_or("handoff switch")?;
        let round = h["round"].as_u64().ok_or("handoff round")?;
        let output = h["output"].as_str().ok_or("handoff output")?.to_string();
        let members: Vec<Member> = serde_json::from_value(h["members"].clone()).map_err(|e| format!("handoff members: {e}"))?;
        let end = switch - 1;
        let end_hash = rpc_call(&self.rpc(), "aether_getBlock", json!([end]))?["hash"].as_str().ok_or("no block before the switch")?.to_string();
        let ours = NetworkFile::load(&self.network_path())?;
        let mut next = NetworkFile { validators: members.clone(), identity: ours.identity.clone(), round, output: Some(output.clone()), ..ours.clone() };
        next.epochs.push(EpochStart { height: switch, parent: end_hash });
        let staged: Option<crate::dkg::KeyFile> = std::fs::read(self.data.join(STAGED_THRESHOLD))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .filter(|k: &crate::dkg::KeyFile| k.output == output && k.round == round);
        let joining = members.iter().any(|m| m.key == me);
        match (joining, staged) {
            (true, Some(key)) => {
                if role == Role::Candidate {
                    // No validator history: start from the block verified last as a follower.
                    let fin = rpc_call(&self.rpc(), "aether_getFinalized", json!([end]))?;
                    if fin.is_null() {
                        return Err("the follower has no proof of the block before the switch yet".into());
                    }
                    std::fs::write(self.data.join(crate::rotation::ANCHOR_FILE), fin.to_string()).map_err(|e| e.to_string())?;
                    self.adopt_follower_state()?;
                }
                write_secret(&self.data.join("threshold.json"), &serde_json::to_vec_pretty(&key).expect("json"))?;
                tracing::info!(switch, "aether run: this Mac votes from the switch height");
            }
            (true, None) => tracing::warn!("aether run: in the new voting set without its share (the reshare did not finish here); following"),
            (false, _) => {
                // Erase the old share: the new sharing has the same secret, so a
                // quorum of old shares kept anywhere could still sign. Safety
                // rests on honest members deleting theirs when they leave.
                erase(&self.data.join("threshold.json"))?;
                tracing::info!(switch, "aether run: left the voting set; following");
            }
        }
        self.write_network(&next)?;
        let _ = std::fs::remove_file(self.data.join(STAGED_THRESHOLD));
        let _ = std::fs::remove_file(self.data.join(STAGED_NETWORK));
        Ok(())
    }

    /// Move a previous validator's storage aside and start from the follower's
    /// verified state (its node state and marshal archives would have a gap).
    fn adopt_follower_state(&self) -> Result<(), String> {
        let prefix = std::fs::read_to_string(self.data.join("partition")).map(|p| p.trim().to_string()).unwrap_or_else(|_| "aether".into());
        let stale: Vec<PathBuf> = std::fs::read_dir(&self.data)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n == "state.redb" || n.starts_with(&format!("{prefix}-"))))
            .collect();
        if !stale.is_empty() {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
            let aside = self.data.join(format!("stale-{secs}"));
            std::fs::create_dir_all(&aside).map_err(|e| e.to_string())?;
            for p in stale {
                let name = p.file_name().expect("listed entry has a name").to_owned();
                std::fs::rename(&p, aside.join(name)).map_err(|e| e.to_string())?;
            }
        }
        std::fs::copy(self.data.join("follow").join("state.redb"), self.data.join("state.redb")).map_err(|e| format!("follower state: {e}"))?;
        Ok(())
    }

    fn write_network(&self, f: &NetworkFile) -> Result<(), String> {
        std::fs::write(self.network_path(), serde_json::to_vec_pretty(f).expect("json")).map_err(|e| e.to_string())
    }
}

fn stop(reshare: &mut Option<Reshare>) {
    if let Some(mut r) = reshare.take() {
        let _ = r.child.kill();
        let _ = r.child.wait();
    }
}

/// Overwrite a secret file, then remove it.
fn erase(path: &Path) -> Result<(), String> {
    let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else { return Ok(()) };
    write_secret(path, &vec![0u8; len as usize])?;
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

/// Secret files are written 0600.
fn write_secret(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path).map_err(|e| e.to_string())?;
    f.write_all(bytes).map_err(|e| e.to_string())
}

/// Keys that identify this Mac; everything else in <data> belongs to one network.
const KEEP_ACROSS_NETWORKS: [&str; 3] = ["validator.key", "validator.pub.json", "node-account.key"];

/// Put `network` in `<data>/network.json`: the first time, or when it is a
/// different network (a testnet reset: other chain id or committee identity).
/// The old network's data is moved to `<data>/stale-<time>`, never deleted.
/// The same network with newer epochs (written by a reshare) is kept.
pub fn adopt_network(data: &Path, network: Option<&Path>) -> Result<(), String> {
    let ours = data.join("network.json");
    let Some(src) = network else {
        return if ours.exists() { Ok(()) } else { Err("first run: pass --network <network.json>".into()) };
    };
    let theirs = NetworkFile::load(src)?;
    if let Ok(current) = NetworkFile::load(&ours) {
        if current.chain_id == theirs.chain_id && current.identity == theirs.identity {
            return Ok(());
        }
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let aside = data.join(format!("stale-{secs}"));
        std::fs::create_dir_all(&aside).map_err(|e| e.to_string())?;
        for e in std::fs::read_dir(data).map_err(|e| e.to_string())?.flatten() {
            let name = e.file_name();
            let n = name.to_string_lossy();
            if KEEP_ACROSS_NETWORKS.contains(&n.as_ref()) || n.starts_with("stale-") {
                continue;
            }
            std::fs::rename(e.path(), aside.join(&name)).map_err(|e| e.to_string())?;
        }
        tracing::warn!(moved_to = %aside.display(), "a different network: the previous one's data was moved aside");
    }
    std::fs::copy(src, &ours).map(|_| ()).map_err(|e| format!("{}: {e}", src.display()))
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
        assert_eq!(NetworkFile::load(&data.join("network.json")).unwrap().identity.as_deref(), Some("bb"));
        let aside = std::fs::read_dir(&data).unwrap().flatten().find(|e| e.file_name().to_string_lossy().starts_with("stale-")).unwrap();
        assert!(aside.path().join("state.redb").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
