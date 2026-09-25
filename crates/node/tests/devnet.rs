//! Multi-process devnet test: 4 validators on loopback.
//! Checks consensus progress, tx finality through different nodes, identical
//! block hashes and state roots everywhere, liveness with one validator down,
//! and restart recovery from the finalized archive. Also: FOCIL inclusion
//! lists get a censored sender's tx into a block.

use serde_json::{json, Value};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_aether");
const COUNTER_INIT: &str = "600a600c600039600a6000f360005460010160005500";

/// These tests each run 4-5 validator processes; run one at a time so they do
/// not starve each other of CPU or race for the same free ports.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

struct Net {
    dir: PathBuf,
    p2p: Vec<u16>,
    rpc: Vec<u16>,
    procs: Vec<Option<Child>>,
    extra: Vec<Vec<String>>,
}

impl Net {
    fn start(n: usize) -> Net {
        Self::start_with("basic", vec![vec![]; n])
    }

    /// `extra[i]` = additional CLI args for validator `i`.
    fn start_with(tag: &str, extra: Vec<Vec<String>>) -> Net {
        let n = extra.len();
        let dir = std::env::temp_dir().join(format!("aether-devnet-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p2p: Vec<u16> = (0..n).map(|_| free_port()).collect();
        let rpc: Vec<u16> = (0..n).map(|_| free_port()).collect();
        let mut net = Net::prepared(dir, p2p, rpc, extra);
        for i in 0..n {
            net.spawn(i);
        }
        net
    }

    fn prepared(dir: PathBuf, p2p: Vec<u16>, rpc: Vec<u16>, extra: Vec<Vec<String>>) -> Net {
        let n = extra.len();
        Net { dir, p2p, rpc, procs: (0..n).map(|_| None).collect(), extra }
    }

    fn peers(&self, i: usize) -> String {
        (0..self.p2p.len()).filter(|j| *j != i).map(|j| format!("{}@127.0.0.1:{}", j + 1, self.p2p[j])).collect::<Vec<_>>().join(",")
    }

    fn spawn(&mut self, i: usize) {
        let mut cmd = Command::new(BIN);
        cmd.args(["node", "--index", &(i + 1).to_string(), "--validators", &self.p2p.len().to_string()])
            .args(["--port", &self.p2p[i].to_string(), "--rpc-port", &self.rpc[i].to_string()])
            .args(["--data", self.dir.join((i + 1).to_string()).to_str().unwrap()])
            .args(["--block-time-ms", "500"])
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Plain TCP between validators and no public endpoint: offline, and
        // never publishes devnet node ids to the DHT.
        let peers: Vec<String> = (0..self.p2p.len()).filter(|j| *j != i).map(|j| format!("{}@127.0.0.1:{}", j + 1, self.p2p[j])).collect();
        cmd.args(["--peers", &peers.join(","), "--offline"]).args(&self.extra[i]);
        self.procs[i] = Some(cmd.spawn().expect("spawn validator"));
    }

    /// Like `spawn`, but identity comes from --network in `extra` and <data>/validator.key.
    fn spawn_with_network(&mut self, i: usize) {
        let mut cmd = Command::new(BIN);
        cmd.args(["node", "--port", &self.p2p[i].to_string(), "--rpc-port", &self.rpc[i].to_string()])
            .args(["--data", self.dir.join((i + 1).to_string()).to_str().unwrap()])
            .args(["--block-time-ms", "500", "--peers", &self.peers(i), "--offline"])
            .args(&self.extra[i])
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        self.procs[i] = Some(cmd.spawn().expect("spawn validator"));
    }

    fn kill(&mut self, i: usize) {
        if let Some(mut c) = self.procs[i].take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    fn url(&self, i: usize) -> String {
        format!("http://127.0.0.1:{}", self.rpc[i])
    }

    fn rpc(&self, i: usize, method: &str, params: Value) -> Option<Value> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let v: Value = reqwest::blocking::Client::new().post(self.url(i)).json(&body).timeout(Duration::from_secs(3)).send().ok()?.json().ok()?;
        v.get("result").cloned()
    }

    fn height(&self, i: usize) -> u64 {
        self.rpc(i, "aether_status", json!([])).and_then(|v| v["height"].as_u64()).unwrap_or(0)
    }

    fn wait_height(&self, i: usize, h: u64, secs: u64) {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if self.height(i) >= h {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        panic!("node {i} did not reach height {h} (at {})", self.height(i));
    }

    fn cli(&self, args: &[&str]) -> String {
        let out = Command::new(BIN).args(args).output().expect("run cli");
        assert!(out.status.success(), "cli {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }
}

impl Drop for Net {
    fn drop(&mut self) {
        for i in 0..self.procs.len() {
            self.kill(i);
        }
        // Keep logs and data of a failed run for inspection.
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.dir);
        } else {
            eprintln!("kept test data in {}", self.dir.display());
        }
    }
}

fn block(net: &Net, i: usize, h: u64) -> Value {
    net.rpc(i, "aether_getBlock", json!([h])).unwrap_or(Value::Null)
}

fn assert_agree(net: &Net, nodes: &[usize], h: u64) {
    // A node may be a block or two behind the one we read `h` from.
    for &i in nodes {
        net.wait_height(i, h, 30);
    }
    let first = block(net, nodes[0], h);
    assert!(!first.is_null(), "block {h} missing on node {}", nodes[0]);
    for &i in &nodes[1..] {
        let b = block(net, i, h);
        assert_eq!(b["hash"], first["hash"], "block hash differs at {h} on node {i}");
        assert_eq!(b["state_root"], first["state_root"], "state root differs at {h} on node {i}");
    }
}

#[test]
fn four_validators_agree_execute_survive_and_recover() {
    let _serial = serial();
    let mut net = Net::start(4);
    for i in 0..4 {
        net.wait_height(i, 3, 60);
    }

    // Transfer through node 0, verify through node 2.
    let bob = "0x00000000000000000000000000000000000b0b00";
    let out = net.cli(&["send", "--rpc", &net.url(0), "--from-dev", "1", "--to", bob, "--value", "777", "--wait"]);
    assert!(out.contains("success=true"), "{out}");
    // Finalized on the submitting node; the reading node may be a block behind.
    net.wait_height(2, net.height(0), 20);
    let bal = net.cli(&["balance", bob, "--rpc", &net.url(2)]);
    assert!(bal.contains("balance   777 wei") && bal.contains("verified  ✓"), "{bal}");

    // Deploy through node 1, call through node 3, read with proof through node 0.
    let out = net.cli(&["deploy", "--rpc", &net.url(1), "--from-dev", "2", "--code", COUNTER_INIT]);
    let contract = out.lines().find_map(|l| l.strip_prefix("contract: ")).expect("contract address").trim().to_string();
    for _ in 0..2 {
        let out = net.cli(&["call", "--rpc", &net.url(3), "--from-dev", "3", "--to", &contract, "--wait"]);
        assert!(out.contains("success=true"), "{out}");
    }
    net.wait_height(0, net.height(3), 20);
    let st = net.cli(&["storage", &contract, "0", "--rpc", &net.url(0)]);
    assert!(st.contains("] = 2") && st.contains("verified  ✓"), "{st}");

    // One P-256 signature pays three addresses (EIP-7702 delegation + batch).
    let (x, y, z) = ("0x00000000000000000000000000000000000000a1", "0x00000000000000000000000000000000000000a2", "0x00000000000000000000000000000000000000a3");
    let out = net.cli(&["batch", "--rpc", &net.url(1), "--from-dev", "5", "--to", &format!("{x},{y},{z}"), "--value", "9", "--wait"]);
    assert!(out.contains("success=true"), "{out}");
    net.wait_height(2, net.height(1), 20);
    for a in [x, y, z] {
        let bal = net.cli(&["balance", a, "--rpc", &net.url(2)]);
        assert!(bal.contains("balance   9 wei") && bal.contains("verified  ✓"), "{bal}");
    }
    // Second batch: already delegated, no delegation field needed.
    let out = net.cli(&["batch", "--rpc", &net.url(0), "--from-dev", "5", "--to", x, "--value", "1", "--wait"]);
    assert!(out.contains("success=true"), "{out}");

    // Every node has the same chain.
    let h = net.height(0);
    for i in 1..4 {
        net.wait_height(i, h, 30);
    }
    for height in 1..=h {
        assert_agree(&net, &[0, 1, 2, 3], height);
    }

    // One validator down: 3 of 4 still finalize and include txs.
    net.kill(3);
    let h_down = net.height(0);
    let out = net.cli(&["send", "--rpc", &net.url(1), "--from-dev", "4", "--to", bob, "--value", "1", "--wait"]);
    assert!(out.contains("success=true"), "{out}");
    net.wait_height(0, h_down + 5, 60);

    // Restart: replays its archive, rejoins with identical history.
    net.spawn(3);
    let target = net.height(0) + 3;
    net.wait_height(3, target, 120);
    for height in (1..=target).step_by(7) {
        assert_agree(&net, &[0, 3], height);
    }
    assert_agree(&net, &[0, 1, 2, 3], target);
}

fn dev_address(dev: u8) -> String {
    let out = Command::new(BIN).arg("dev-accounts").output().expect("dev-accounts");
    let text = String::from_utf8(out.stdout).unwrap();
    let line = text.lines().find(|l| l.split_whitespace().nth(1) == Some(&dev.to_string())).expect("dev account");
    line.split_whitespace().nth(2).unwrap().to_string()
}

fn wait_receipt(net: &Net, i: usize, hash: &str, secs: u64) -> Value {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if let Some(r) = net.rpc(i, "aether_getReceipt", json!([hash])) {
            if r.get("height").is_some() {
                return r;
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    panic!("tx {hash} not finalized within {secs}s");
}

/// Every validator leaves dev 3 out of its own ordering, and validator 1 also
/// ignores inclusion lists. Dev 3's tx can then only land through a list:
/// published by a committee member, gossiped, put first by an honest proposer.
#[test]
fn inclusion_lists_get_censored_txs_in() {
    let _serial = serial();
    let censored = dev_address(3);
    let extra = (0..4)
        .map(|i| {
            let mut a = vec!["--dev-deprioritize".to_string(), censored.clone()];
            if i == 0 {
                a.extend(["--dev-censor".to_string(), censored.clone()]);
            }
            a
        })
        .collect();
    let net = Net::start_with("focil", extra);
    for i in 0..4 {
        net.wait_height(i, 3, 60);
    }
    let censor_addr = format!("{}", aether_node::chain::leader_address(&commonware_cryptography::Signer::public_key(&aether_light::devnet_validator_key(1))));

    let bob = "0x000000000000000000000000000000000000b0b1";
    let out = net.cli(&["send", "--rpc", &net.url(1), "--from-dev", "3", "--to", bob, "--value", "42"]);
    let hash = out.split_whitespace().nth(1).expect("tx hash").to_string();
    let r = wait_receipt(&net, 2, &hash, 40);
    assert_eq!(r["receipt"]["success"], true);
    let b = block(&net, 2, r["height"].as_u64().unwrap());
    assert!(!b["proposer"].as_str().unwrap().eq_ignore_ascii_case(&censor_addr), "the censoring validator never includes it");

    // Normal senders are unaffected.
    let out = net.cli(&["send", "--rpc", &net.url(3), "--from-dev", "4", "--to", bob, "--value", "1"]);
    wait_receipt(&net, 0, out.split_whitespace().nth(1).unwrap(), 20);
    let h = net.height(0);
    for i in 1..4 {
        net.wait_height(i, h, 20);
    }
    assert_agree(&net, &[0, 1, 2, 3], h);
}

/// Genesis ceremony: 4 processes run the DKG over p2p (no dealer), then run
/// consensus on the resulting shares; a client trusting only the printed
/// identity verifies a balance, and the devnet dealer's identity is rejected.
#[test]
fn dkg_ceremony_then_consensus_under_its_identity() {
    let _serial = serial();
    let n = 4;
    let dir = std::env::temp_dir().join(format!("aether-devnet-test-{}-dkg", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p2p: Vec<u16> = (0..n).map(|_| free_port()).collect();
    let rpc: Vec<u16> = (0..n).map(|_| free_port()).collect();
    let mut net = Net::prepared(dir.clone(), p2p, rpc, vec![vec![]; n]);

    let dkg: Vec<Child> = (0..n)
        .map(|i| {
            Command::new(BIN)
                .args(["dkg", "--index", &(i + 1).to_string(), "--validators", &n.to_string(), "--port", &net.p2p[i].to_string()])
                .args(["--data", dir.join((i + 1).to_string()).to_str().unwrap(), "--peers", &net.peers(i), "--offline"])
                .env("RUST_LOG", "warn")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn dkg")
        })
        .collect();
    let ids: Vec<String> = dkg
        .into_iter()
        .map(|c| {
            let out = c.wait_with_output().expect("dkg exits");
            assert!(out.status.success(), "dkg failed");
            let text = String::from_utf8(out.stdout).unwrap();
            text.lines().find_map(|l| l.strip_prefix("committee identity: ")).expect("identity printed").to_string()
        })
        .collect();
    assert!(ids.iter().all(|id| *id == ids[0]), "every validator derived the same identity");

    for i in 0..n {
        net.spawn(i);
    }
    for i in 0..n {
        net.wait_height(i, 3, 60);
    }
    let bob = "0x000000000000000000000000000000000000d1c9";
    net.cli(&["send", "--rpc", &net.url(0), "--from-dev", "1", "--to", bob, "--value", "99", "--wait"]);
    net.wait_height(2, net.height(0), 20);
    let bal = net.cli(&["balance", bob, "--rpc", &net.url(2), "--identity", &ids[0]]);
    assert!(bal.contains("balance   99 wei") && bal.contains("verified  ✓"), "{bal}");
    let out = Command::new(BIN).args(["balance", bob, "--rpc", &net.url(2)]).output().unwrap();
    assert!(!out.status.success(), "the dealer's devnet identity must not verify DKG certificates");
}

/// Real-network setup: every validator generates its own keys, the public
/// entries become network.json, the DKG runs on those keys, and consensus
/// starts from the file. Nothing uses the public devnet keys.
#[test]
fn locally_generated_keys_network_file_dkg_and_consensus() {
    let _serial = serial();
    let n = 4;
    let dir = std::env::temp_dir().join(format!("aether-devnet-test-{}-keys", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let data = |i: usize| dir.join((i + 1).to_string());

    // 1. keygen on each machine; secrets are 0600 and never overwritten.
    for i in 0..n {
        let out = Command::new(BIN).args(["keygen", "--data", data(i).to_str().unwrap()]).output().unwrap();
        assert!(out.status.success());
    }
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(data(0).join("validator.key")).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!Command::new(BIN).args(["keygen", "--data", data(0).to_str().unwrap()]).status().unwrap().success(), "keygen must not overwrite");

    // 2. assemble network.json from the public halves.
    let pubs: Vec<String> = (0..n).map(|i| data(i).join("validator.pub.json").to_str().unwrap().to_string()).collect();
    let out = Command::new(BIN).arg("network").args(&pubs).output().unwrap();
    assert!(out.status.success());
    let network = dir.join("network.json");
    std::fs::write(&network, &out.stdout).unwrap();
    let net_arg = network.to_str().unwrap().to_string();

    // 3. DKG on those keys (no --index: each process finds itself by its key).
    let p2p: Vec<u16> = (0..n).map(|_| free_port()).collect();
    let rpc: Vec<u16> = (0..n).map(|_| free_port()).collect();
    // Nodes run from the network.json each DKG wrote (it adds the identity).
    let extra = (0..n).map(|i| vec!["--network".to_string(), data(i).join("network.json").to_str().unwrap().to_string()]).collect();
    let mut net = Net::prepared(dir.clone(), p2p, rpc, extra);
    let dkg: Vec<Child> = (0..n)
        .map(|i| {
            Command::new(BIN)
                .args(["dkg", "--network", &net_arg, "--port", &net.p2p[i].to_string(), "--data", data(i).to_str().unwrap()])
                .args(["--peers", &net.peers(i), "--offline"])
                .env("RUST_LOG", "warn")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for c in dkg {
        assert!(c.wait_with_output().unwrap().status.success(), "dkg failed");
    }
    let written: Value = serde_json::from_slice(&std::fs::read(data(0).join("network.json")).unwrap()).unwrap();
    let identity = written["identity"].as_str().expect("identity in network.json").to_string();
    assert_eq!(written["validators"].as_array().unwrap().len(), n);

    // 4. consensus from the network file (spawn passes only --network, no --index).
    for i in 0..n {
        net.spawn_with_network(i);
    }
    for i in 0..n {
        net.wait_height(i, 3, 60);
    }
    let bob = "0x000000000000000000000000000000000000cafe";
    net.cli(&["send", "--rpc", &net.url(1), "--from-dev", "3", "--to", bob, "--value", "5", "--wait"]);
    net.wait_height(3, net.height(1), 20);
    let bal = net.cli(&["balance", bob, "--rpc", &net.url(3), "--identity", &identity]);
    assert!(bal.contains("balance   5 wei") && bal.contains("verified  ✓"), "{bal}");
}

fn run_ok(args: &[&str]) -> String {
    let out = Command::new(BIN).args(args).output().expect("run");
    assert!(out.status.success(), "{:?}: {}", args, String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn spawn_quiet(args: &[String]) -> Child {
    Command::new(BIN).args(args).env("RUST_LOG", "warn").stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn")
}

fn spawn_logged(log: std::fs::File, args: &[String]) -> Child {
    Command::new(BIN).args(args).env("RUST_LOG", "warn").stdout(log).stderr(Stdio::null()).spawn().expect("spawn")
}

fn tcp_peers(ports: &[u16], me: usize) -> String {
    (0..ports.len()).filter(|j| *j != me).map(|j| format!("{}@127.0.0.1:{}", j + 1, ports[j])).collect::<Vec<_>>().join(",")
}

/// Validator rotation: committee A = {1,2,3,4} runs a chain; it stops, the key
/// is reshared to B = {2,3,4,5} (1 leaves, 5 joins with no data), and B
/// continues the SAME chain in a new epoch under the SAME committee identity.
#[test]
fn validator_rotation_continues_the_chain_under_the_same_identity() {
    let _serial = serial();
    let dir = std::env::temp_dir().join(format!("aether-devnet-test-{}-rotate", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let d = |i: usize| dir.join(i.to_string()).to_str().unwrap().to_string();
    let path = |name: &str| dir.join(name).to_str().unwrap().to_string();
    for i in 1..=5 {
        run_ok(&["keygen", "--data", &d(i)]);
    }
    let pubs = |ids: &[usize]| ids.iter().map(|i| format!("{}/validator.pub.json", d(*i))).collect::<Vec<_>>();
    let net_a = run_ok(&[&["network".to_string()][..], &pubs(&[1, 2, 3, 4])].concat().iter().map(String::as_str).collect::<Vec<_>>());
    let net_b = run_ok(&[&["network".to_string()][..], &pubs(&[2, 3, 4, 5])].concat().iter().map(String::as_str).collect::<Vec<_>>());
    std::fs::write(path("A.json"), net_a).unwrap();
    std::fs::write(path("B.json"), net_b).unwrap();

    // DKG for A.
    let ports: Vec<u16> = (0..4).map(|_| free_port()).collect();
    let dkg: Vec<Child> = (0..4)
        .map(|k| {
            spawn_quiet(&[
                "dkg".into(),
                "--network".into(),
                path("A.json"),
                "--port".into(),
                ports[k].to_string(),
                "--data".into(),
                d(k + 1),
                "--peers".into(),
                tcp_peers(&ports, k),
                "--offline".into(),
            ])
        })
        .collect();
    for c in dkg {
        assert!(c.wait_with_output().unwrap().status.success());
    }
    std::fs::copy(format!("{}/network.json", d(1)), path("A-final.json")).unwrap();
    let identity = serde_json::from_slice::<Value>(&std::fs::read(path("A-final.json")).unwrap()).unwrap()["identity"].as_str().unwrap().to_string();

    // Committee A runs; pay 0xaa.
    let p2p: Vec<u16> = (0..4).map(|_| free_port()).collect();
    let rpc: Vec<u16> = (0..4).map(|_| free_port()).collect();
    let mut a = Net::prepared(dir.clone(), p2p, rpc, (1..=4).map(|i| vec!["--network".to_string(), format!("{}/network.json", d(i))]).collect());
    for k in 0..4 {
        a.spawn_with_network(k);
    }
    a.wait_height(0, 3, 60);
    let aa = "0x00000000000000000000000000000000000000aa";
    a.cli(&["send", "--rpc", &a.url(0), "--from-dev", "1", "--to", aa, "--value", "11", "--wait"]);
    for k in 0..4 {
        a.kill(k);
    }

    // Reshare A -> B at the old committee's last finalized block.
    let heads: Vec<String> = (1..=4).map(|i| run_ok(&["head", "--data", &d(i)]).trim().to_string()).collect();
    let end = heads.iter().max_by_key(|h| h.split(' ').next().unwrap().parse::<u64>().unwrap()).unwrap().clone();
    let (end_h, end_hash) = end.split_once(' ').unwrap();
    let rports: Vec<u16> = (0..5).map(|_| free_port()).collect();
    let rs: Vec<Child> = (0..5)
        .map(|k| {
            spawn_quiet(&[
                "reshare".into(),
                "--from".into(),
                path("A-final.json"),
                "--to".into(),
                path("B.json"),
                "--epoch-end".into(),
                end_h.into(),
                "--epoch-end-hash".into(),
                end_hash.into(),
                "--port".into(),
                rports[k].to_string(),
                "--data".into(),
                d(k + 1),
                "--peers".into(),
                tcp_peers(&rports, k),
                "--offline".into(),
            ])
        })
        .collect();
    let outs: Vec<String> = rs.into_iter().map(|c| String::from_utf8(c.wait_with_output().unwrap().stdout).unwrap()).collect();
    assert!(outs[0].contains("has left"), "validator 1 leaves: {}", outs[0]);
    for o in &outs[1..] {
        assert!(o.contains(&identity), "identity unchanged: {o}");
    }

    // Committee B continues the chain (validator 5 starts empty and catches up).
    let p2p: Vec<u16> = (0..4).map(|_| free_port()).collect();
    let rpc: Vec<u16> = (0..4).map(|_| free_port()).collect();
    let mut b = Net::prepared(dir.clone(), p2p, rpc, vec![vec!["--network".into(), path("B.json")]; 4]);
    // B's validator k is machine k + 2: point spawn at those data dirs.
    let mut procs = Vec::new();
    for k in 0..4 {
        let log = std::fs::File::create(dir.join(format!("nodeB{k}.log"))).unwrap();
        procs.push(spawn_logged(
            log,
            &[
                "node".into(),
                "--network".into(),
                // The network.json reshare wrote: epoch schedule and identity.
                format!("{}/network.json", d(k + 2)),
                "--port".into(),
                b.p2p[k].to_string(),
                "--rpc-port".into(),
                b.rpc[k].to_string(),
                "--data".into(),
                d(k + 2),
                "--block-time-ms".into(),
                "500".into(),
                "--peers".into(),
                b.peers(k),
                "--offline".into(),
            ],
        ));
    }
    b.procs = procs.into_iter().map(Some).collect();
    let boundary: u64 = end_h.parse().unwrap();
    for k in 0..4 {
        b.wait_height(k, boundary + 5, 90);
    }
    let bal = b.cli(&["balance", aa, "--rpc", &b.url(3), "--identity", &identity]);
    assert!(bal.contains("balance   11 wei") && bal.contains("verified  ✓"), "history from committee A, verified on new member: {bal}");
    let h = b.height(0);
    for k in 1..4 {
        b.wait_height(k, h, 30);
    }
    assert_agree(&b, &[0, 1, 2, 3], h);
    let old = block(&b, 3, boundary);
    assert_eq!(old["hash"].as_str().unwrap(), end_hash, "B built on A's last block");
}
