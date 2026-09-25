//! Multi-process devnet test: 4 validators on loopback.
//! Checks consensus progress, tx finality through different nodes, identical
//! block hashes and state roots everywhere, liveness with one validator down,
//! and restart recovery from the finalized archive.

use serde_json::{json, Value};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_aether");
const COUNTER_INIT: &str = "600a600c600039600a6000f360005460010160005500";

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

struct Net {
    dir: PathBuf,
    p2p: Vec<u16>,
    rpc: Vec<u16>,
    procs: Vec<Option<Child>>,
}

impl Net {
    fn start(n: usize) -> Net {
        let dir = std::env::temp_dir().join(format!("aether-devnet-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p2p: Vec<u16> = (0..n).map(|_| free_port()).collect();
        let rpc: Vec<u16> = (0..n).map(|_| free_port()).collect();
        let mut net = Net { dir, p2p, rpc, procs: (0..n).map(|_| None).collect() };
        for i in 0..n {
            net.spawn(i);
        }
        net
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
        if i > 0 {
            cmd.args(["--bootstrap", &format!("1@127.0.0.1:{}", self.p2p[0])]);
        }
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
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn block(net: &Net, i: usize, h: u64) -> Value {
    net.rpc(i, "aether_getBlock", json!([h])).unwrap_or(Value::Null)
}

fn assert_agree(net: &Net, nodes: &[usize], h: u64) {
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
    let mut net = Net::start(4);
    for i in 0..4 {
        net.wait_height(i, 3, 60);
    }

    // Transfer through node 0, verify through node 2.
    let bob = "0x00000000000000000000000000000000000b0b00";
    let out = net.cli(&["send", "--rpc", &net.url(0), "--from-dev", "1", "--to", bob, "--value", "777", "--wait"]);
    assert!(out.contains("success=true"), "{out}");
    let bal = net.cli(&["balance", bob, "--rpc", &net.url(2)]);
    assert!(bal.contains("balance   777 wei") && bal.contains("verified  ✓"), "{bal}");

    // Deploy through node 1, call through node 3, read with proof through node 0.
    let out = net.cli(&["deploy", "--rpc", &net.url(1), "--from-dev", "2", "--code", COUNTER_INIT]);
    let contract = out.lines().find_map(|l| l.strip_prefix("contract: ")).expect("contract address").trim().to_string();
    for _ in 0..2 {
        let out = net.cli(&["call", "--rpc", &net.url(3), "--from-dev", "3", "--to", &contract, "--wait"]);
        assert!(out.contains("success=true"), "{out}");
    }
    let st = net.cli(&["storage", &contract, "0", "--rpc", &net.url(0)]);
    assert!(st.contains("] = 2") && st.contains("verified  ✓"), "{st}");

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
