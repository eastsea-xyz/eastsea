//! `aether run`: one Mac, one process to keep running. It runs this Mac as a
//! validator while it is in the voting set and as a verifying follower (and
//! voting-node candidate) otherwise, and moves between the two on its own: when
//! the running set stops at a rotation boundary (`rotation.rs`), old and new
//! members reshare the committee key and restart in their new roles. If the
//! reshare fails, the running set carries on for that registry epoch, so the
//! chain never waits on a person.

use crate::roster::{Member, NetworkFile};
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
    pub port: u16,
    pub rpc_port: u16,
    /// Extra args for `aether node` (faucet, DeviceCheck, block time, …).
    pub node_args: Vec<String>,
    /// Extra args for `aether follow` (e.g. `--from-rpc`).
    pub follow_args: Vec<String>,
    /// Local tests: validators talk plain TCP on loopback; each writes its
    /// port to `<dir>/<voting key hex>` so the others can find it.
    pub dev_peer_dir: Option<PathBuf>,
    /// How long a reshare may take before the running set carries on.
    pub reshare_timeout: Duration,
}

#[derive(Debug, PartialEq, Eq)]
enum Role {
    Validator,
    Candidate,
}

impl Supervisor {
    fn network_path(&self) -> PathBuf {
        self.data.join("network.json")
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
    fn tcp_peers(&self, validators: &[Member], me: &str) -> Option<String> {
        let dir = self.dev_peer_dir.as_ref()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while validators.iter().any(|m| !dir.join(&m.key).exists()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
        }
        let list: Vec<String> = validators
            .iter()
            .enumerate()
            .filter(|(_, m)| m.key != me)
            .filter_map(|(i, m)| std::fs::read_to_string(dir.join(&m.key)).ok().map(|p| format!("{}@127.0.0.1:{}", i + 1, p.trim())))
            .collect();
        Some(list.join(","))
    }

    fn spawn(&self, role: &Role, me: &str) -> Result<Child, String> {
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
                if let Some(peers) = self.tcp_peers(&NetworkFile::load(&net)?.validators, me) {
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

    /// Runs forever: (re)start the child for the current role, watch for rotations.
    pub fn run(&self) -> Result<(), String> {
        let me = self.my_key()?;
        if let Some(dir) = &self.dev_peer_dir {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join(&me), self.port.to_string()).map_err(|e| e.to_string())?;
        }
        loop {
            let role = self.role(&me)?;
            let mut child = self.spawn(&role, &me)?;
            let outcome = self.watch(&mut child, &role, &me);
            let _ = child.kill();
            let _ = child.wait();
            match outcome {
                Watch::Exited => std::thread::sleep(Duration::from_secs(2)),
                Watch::Rotate(rot) => self.rotate(&role, &me, &rot),
            }
        }
    }

    fn watch(&self, child: &mut Child, role: &Role, me: &str) -> Watch {
        let rpc = format!("http://127.0.0.1:{}", self.rpc_port);
        let mut last_refresh = Instant::now();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if child.try_wait().ok().flatten().is_some() {
                tracing::warn!("aether run: child exited; restarting");
                return Watch::Exited;
            }
            if let Ok(rot) = rpc_call(&rpc, "aether_rotation", json!([])) {
                if !rot.is_null() {
                    let involved = *role == Role::Validator || rot["next"].as_array().is_some_and(|n| n.iter().any(|m| m["key"] == me));
                    if involved && (*role == Role::Validator || self.save_anchor(&rpc, &rot)) {
                        return Watch::Rotate(rot);
                    }
                }
            }
            // A follower keeps its view of who the validators are current.
            if *role == Role::Candidate && last_refresh.elapsed() > Duration::from_secs(30) {
                last_refresh = Instant::now();
                if self.refresh_network(&rpc) {
                    return Watch::Exited;
                }
            }
        }
    }

    /// A follower joining the voting set: once it verified the old set's last
    /// block, keep that block and its finalization to start from (false = not yet).
    fn save_anchor(&self, rpc: &str, rot: &Value) -> bool {
        let Some(end) = rot["end_height"].as_u64() else { return false };
        let Ok(fin) = rpc_call(rpc, "aether_getFinalized", json!([end])) else { return false };
        if fin.is_null() {
            return false;
        }
        std::fs::write(self.data.join(crate::rotation::ANCHOR_FILE), fin.to_string()).is_ok()
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

    /// Adopt the validators' network.json if its voting set changed (true = restart).
    fn refresh_network(&self, rpc: &str) -> bool {
        let Ok(v) = rpc_call(rpc, "aether_network", json!([])) else { return false };
        let Ok(theirs) = serde_json::from_value::<NetworkFile>(v) else { return false };
        let Ok(ours) = NetworkFile::load(&self.network_path()) else { return false };
        if theirs.chain_id != ours.chain_id || theirs.identity != ours.identity || theirs.validators == ours.validators {
            return false;
        }
        self.write_network(&theirs).is_ok()
    }

    fn write_network(&self, f: &NetworkFile) -> Result<(), String> {
        std::fs::write(self.network_path(), serde_json::to_vec_pretty(f).expect("json")).map_err(|e| e.to_string())
    }

    fn rotate(&self, role: &Role, me: &str, rot: &Value) {
        match self.try_rotate(role, me, rot) {
            Ok(()) => tracing::info!(end = %rot["end_height"], "aether run: voting set rotated"),
            Err(e) => {
                tracing::warn!(%e, "aether run: reshare failed");
                // The running set keeps the chain going until the next registry epoch.
                if *role == Role::Validator {
                    let _ = std::fs::write(self.data.join(crate::rotation::DEFERRED_FILE), rot["epoch"].to_string());
                }
            }
        }
    }

    fn try_rotate(&self, role: &Role, me: &str, rot: &Value) -> Result<(), String> {
        let from: NetworkFile = match role {
            Role::Validator => NetworkFile::load(&self.network_path())?,
            Role::Candidate => serde_json::from_value(rot["network"].clone()).map_err(|e| format!("rotation has no network: {e}"))?,
        };
        let next: Vec<Member> = serde_json::from_value(rot["next"].clone()).map_err(|e| format!("rotation next: {e}"))?;
        let to = NetworkFile { validators: next.clone(), identity: None, output: None, epochs: vec![], ..from.clone() };
        let (from_path, to_path) = (self.data.join("rotation-from.json"), self.data.join("rotation-to.json"));
        std::fs::write(&from_path, serde_json::to_vec_pretty(&from).expect("json")).map_err(|e| e.to_string())?;
        std::fs::write(&to_path, serde_json::to_vec_pretty(&to).expect("json")).map_err(|e| e.to_string())?;
        let end = rot["end_height"].as_u64().ok_or("rotation end_height")?;
        let hash = rot["end_hash"].as_str().ok_or("rotation end_hash")?;
        let mut cmd = Command::new(&self.exe);
        cmd.args(["reshare", "--exit-with-parent", "--from", &path_str(&from_path), "--to", &path_str(&to_path)])
            .args(["--epoch-end", &end.to_string(), "--epoch-end-hash", hash])
            .args(["--port", &self.port.to_string(), "--data", &path_str(&self.data)]);
        let union: Vec<Member> = from.validators.iter().cloned().chain(next.iter().filter(|m| !from.validators.contains(m)).cloned()).collect();
        if let Some(peers) = self.tcp_peers(&union, me) {
            cmd.args(["--peers", &peers, "--offline"]);
        }
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let deadline = Instant::now() + self.reshare_timeout;
        let status = loop {
            if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
                break s;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("reshare timed out".into());
            }
            std::thread::sleep(Duration::from_millis(500));
        };
        if !status.success() {
            return Err(format!("reshare exited with {status}"));
        }
        if *role == Role::Candidate && next.iter().any(|m| m.key == me) {
            self.adopt_follower_state()?;
        }
        if !next.iter().any(|m| m.key == me) {
            // Left the voting set: follow the new one (same identity, one more epoch).
            let mut left = to;
            left.keep_genesis(&from);
            left.epochs = from.epochs.clone();
            left.epochs.push(crate::roster::EpochStart { height: end + 1, parent: hash.to_string() });
            left.identity = from.identity.clone();
            left.round = from.round + 1;
            self.write_network(&left)?;
        }
        let _ = std::fs::remove_file(self.data.join(crate::rotation::DEFERRED_FILE));
        Ok(())
    }
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

enum Watch {
    Exited,
    Rotate(Value),
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
