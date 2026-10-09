//! Multi-process devnet test: 4 validators on loopback.
//! Checks consensus progress, tx finality through different nodes, identical
//! block hashes and state roots everywhere, liveness with one validator down,
//! and restart recovery from the finalized archive. Also: FOCIL inclusion
//! lists get a censored sender's tx into a block.

use aether_test_support::{Port, TestChild};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_aether");
const COUNTER_INIT: &str = "600a600c600039600a6000f360005460010160005500";

/// These tests each run 4-5 validator processes; run one at a time so they do
/// not starve each other of CPU. Port leases coordinate other processes.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

struct Net {
    dir: PathBuf,
    p2p: Vec<Port>,
    rpc: Vec<Port>,
    procs: Vec<Option<TestChild>>,
    extra: Vec<Vec<String>>,
    /// Where node `i`'s stdout and stderr are captured, so a node that never
    /// answers can show why in the panic (a startup refusal prints to stderr).
    logs: Vec<PathBuf>,
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
        let p2p: Vec<Port> = (0..n).map(|_| Port::reserve().expect("reserve test port")).collect();
        let rpc: Vec<Port> = (0..n).map(|_| Port::reserve().expect("reserve test port")).collect();
        let mut net = Net::prepared(dir, p2p, rpc, extra);
        for i in 0..n {
            net.spawn(i);
        }
        net
    }

    fn prepared(dir: PathBuf, p2p: Vec<Port>, rpc: Vec<Port>, extra: Vec<Vec<String>>) -> Net {
        let n = extra.len();
        let logs = (0..n).map(|i| dir.join((i + 1).to_string()).join("node.log")).collect();
        Net { dir, p2p, rpc, procs: (0..n).map(|_| None).collect(), extra, logs }
    }

    /// Node `i`'s data dir (validators are numbered 1..n; a later-joined
    /// follower may override `logs[i]` with its own file).
    fn data(&self, i: usize) -> PathBuf {
        self.dir.join((i + 1).to_string())
    }

    /// Spawn a node with both output streams captured into its node.log: a
    /// refusal to start (e.g. the ceremony bind) prints on stderr, and the
    /// panic of a node that never answers shows its last lines.
    fn capture(&self, i: usize, mut cmd: Command) -> TestChild {
        let log = self.logs.get(i).cloned().unwrap_or_else(|| self.data(i).join("node.log"));
        cmd.env("RUST_LOG", "warn");
        TestChild::spawn(cmd, log).expect("spawn validator")
    }

    fn peers(&self, i: usize) -> String {
        (0..self.p2p.len()).filter(|j| *j != i).map(|j| format!("{}@127.0.0.1:{}", j + 1, self.p2p[j])).collect::<Vec<_>>().join(",")
    }

    fn spawn(&mut self, i: usize) {
        let mut cmd = Command::new(BIN);
        cmd.args(["node", "--index", &(i + 1).to_string(), "--validators", &self.p2p.len().to_string()])
            .args(["--port", &self.p2p[i].to_string(), "--rpc-port", &self.rpc[i].to_string()])
            .args(["--data", self.data(i).to_str().unwrap()])
            .args(["--block-time-ms", "500"]);
        // Plain TCP between validators and no public endpoint: offline, and
        // never publishes devnet node ids to the DHT.
        let peers: Vec<String> = (0..self.p2p.len()).filter(|j| *j != i).map(|j| format!("{}@127.0.0.1:{}", j + 1, self.p2p[j])).collect();
        cmd.args(["--peers", &peers.join(","), "--offline"]).args(&self.extra[i]);
        self.procs[i] = Some(self.capture(i, cmd));
    }

    /// Like `spawn`, but identity comes from --network in `extra` and <data>/validator.key.
    fn spawn_with_network(&mut self, i: usize) {
        let mut cmd = Command::new(BIN);
        cmd.args(["node", "--port", &self.p2p[i].to_string(), "--rpc-port", &self.rpc[i].to_string()])
            .args(["--data", self.data(i).to_str().unwrap()])
            .args(["--block-time-ms", "500", "--peers", &self.peers(i), "--offline"])
            .args(&self.extra[i]);
        self.procs[i] = Some(self.capture(i, cmd));
    }

    fn kill(&mut self, i: usize) {
        if let Some(c) = self.procs[i].take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    fn url(&self, i: usize) -> String {
        format!("http://127.0.0.1:{}", self.rpc[i])
    }

    /// The last lines of node `i`'s captured log, for a panic message: a node
    /// that refused to start (or stalled) says why there.
    fn log_tail(&self, i: usize) -> String {
        let path = self.logs.get(i).cloned().unwrap_or_else(|| self.data(i).join("node.log"));
        match std::fs::read_to_string(&path) {
            Ok(log) if log.trim().is_empty() => {
                format!("\n--- {} is empty: the node printed nothing ---", path.display())
            }
            Ok(log) => {
                let lines: Vec<&str> = log.lines().collect();
                let tail: Vec<&str> = lines.iter().skip(lines.len().saturating_sub(24)).copied().collect();
                format!("\n--- last lines of {} ---\n{}", path.display(), tail.join("\n"))
            }
            Err(_) => format!("\n--- no captured log at {} ---", path.display()),
        }
    }

    fn rpc(&self, i: usize, method: &str, params: Value) -> Option<Value> {
        // Poll every startup, including nodes we have not queried yet. A
        // listener actor can fail while the rest of the process stays alive.
        for (node, child) in self.procs.iter().enumerate() {
            if let Some(child) = child {
                if let Some(status) = child.try_wait().unwrap_or_else(|e| panic!("node {node}: {e}{}", self.log_tail(node))) {
                    panic!("node {node} exited {status}{}", self.log_tail(node));
                }
            }
        }
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let v: Value = reqwest::blocking::Client::new().post(self.url(i)).json(&body).timeout(Duration::from_secs(3)).send().ok()?.json().ok()?;
        let result = v.get("result").cloned();
        if result.is_some() {
            if let Some(child) = &self.procs[i] {
                child.mark_started();
            }
        }
        result
    }

    fn height(&self, i: usize) -> u64 {
        self.rpc(i, "aether_status", json!([])).and_then(|v| v["height"].as_u64()).unwrap_or(0)
    }

    /// Wait until node `i` reports height `h`. A started-but-not-yet-answering
    /// RPC is startup, not a stall: on a loaded machine it can take a minute or
    /// more, so the budget is counted from the first answer. From there a node
    /// that stops making progress still fails the test; a node that never
    /// answers at all gets the generous overall limit, and no wait exceeds it.
    fn wait_height(&self, i: usize, h: u64, secs: u64) {
        let start = Instant::now();
        let overall = Duration::from_secs(secs + 240);
        let mut answered: Option<Instant> = None;
        loop {
            let status = self.rpc(i, "aether_status", json!([]));
            let height = status.as_ref().and_then(|v| v["height"].as_u64());
            if height.is_some_and(|got| got >= h) {
                return;
            }
            let now = Instant::now();
            if answered.is_none() && status.is_some() {
                answered = Some(now);
            }
            // An answering node gets `secs` from that first answer; a silent
            // one gets the whole overall limit instead (so a slow start is
            // never mistaken for a stall), and `overall` caps both.
            let deadline = match answered {
                Some(first) => (first + Duration::from_secs(secs)).min(start + overall),
                None => start + overall,
            };
            if now >= deadline {
                panic!(
                    "node {i} did not reach height {h} (at {}) after {}s{}{}",
                    height.unwrap_or(0),
                    now.duration_since(start).as_secs(),
                    if answered.is_none() { "; its RPC never answered" } else { "" },
                    self.log_tail(i)
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Run the CLI expecting failure; returns stderr.
    fn cli_fails(&self, args: &[&str]) -> String {
        let out = Command::new(BIN).args(args).output().expect("run cli");
        assert!(!out.status.success(), "cli {:?} should have failed", args);
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    /// Whether validator `i`'s process is still running.
    fn alive(&mut self, i: usize) -> bool {
        match self.procs[i].as_mut() {
            Some(c) => c.try_wait().ok().flatten().is_none(),
            None => false,
        }
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

    // Recovery: dev 6 names devs 7 and 8 as recovery devices (both must sign,
    // 10 min delay). They propose moving its funds; nothing moves before the
    // delay, and dev 6 (not actually lost) cancels. The post-delay run is covered
    // with virtual time in crates/execution/tests/delegation.rs.
    let out = net.cli(&["set-guardian", "--rpc", &net.url(0), "--from-dev", "6", "--guardian-dev", "7,8", "--threshold", "2", "--delay", "600"]);
    assert!(out.contains("success=true"), "{out}");
    let lost = net.cli(&["dev-accounts"]).lines().find(|l| l.split_whitespace().nth(1) == Some("6")).unwrap().split_whitespace().nth(2).unwrap().to_string();
    net.wait_height(3, net.height(0), 20);
    let before = net.cli(&["balance", &lost, "--rpc", &net.url(3)]);
    let out = net.cli(&["recover", "--rpc", &net.url(3), "--guardian-dev", "7,8", "--lost", &lost]);
    assert!(out.contains("success=true"), "two recovery devices propose: {out}");
    let finish = out.lines().find_map(|l| l.split("--finish ").nth(1)).expect("finish command").trim().to_string();
    let out = net.cli(&["recover", "--rpc", &net.url(3), "--guardian-dev", "7", "--lost", &lost, "--finish", &finish]);
    assert!(out.contains("success=false"), "not before the delay: {out}");
    let out = net.cli(&["cancel-recovery", "--rpc", &net.url(0), "--from-dev", "6"]);
    assert!(out.contains("success=true"), "the owner cancels: {out}");
    net.wait_height(2, net.height(0), 20);
    let after = net.cli(&["balance", &lost, "--rpc", &net.url(2)]);
    let wei = |s: &str| s.lines().nth(1).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u128>().ok()).expect("balance");
    assert!(wei(&before) - wei(&after) < 10u128.pow(15), "funds did not move (only the cancel's gas): {before} -> {after}");

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

/// Grant `addr` test tokens through the faucet on node `i` and wait for the
/// grant to finalize. The faucet signs at most one grant per second, so a
/// refused request is retried rather than treated as a verdict.
fn faucet_grant(net: &Net, i: usize, addr: &str) {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(g) = net.rpc(i, "aether_faucet", json!([addr])) {
            wait_receipt(net, i, g["hash"].as_str().unwrap(), 30);
            return;
        }
        assert!(Instant::now() < end, "the faucet on node {i} would not grant {addr}");
        std::thread::sleep(Duration::from_millis(1_100));
    }
}

/// A loopback WebSocket client without another test dependency. Keep received
/// frames buffered across timeout boundaries; notifications can precede the
/// response to a request on the same socket.
struct RpcSocket {
    stream: TcpStream,
    input: Vec<u8>,
    pending: Vec<Value>,
    id: u64,
    closed: bool,
}

impl RpcSocket {
    fn connect(net: &Net, node: usize) -> Self {
        let mut stream = TcpStream::connect(("127.0.0.1", net.rpc[node].port())).expect("connect RPC websocket");
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
        let request = format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
            net.rpc[node]
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = Vec::new();
        while !response.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).expect("RPC websocket handshake response");
            response.push(byte[0]);
            assert!(response.len() <= 16 * 1024, "bounded websocket handshake");
        }
        let headers = String::from_utf8(response).unwrap();
        assert!(headers.starts_with("HTTP/1.1 101 "), "RPC must upgrade to websocket: {headers}");
        assert!(
            headers.to_ascii_lowercase().contains("sec-websocket-accept: s3pplmbitxaq9kygzzhzrbk+xoo="),
            "valid websocket handshake: {headers}"
        );
        Self { stream, input: Vec::new(), pending: Vec::new(), id: 0, closed: false }
    }

    fn receive_buffer(&self, bytes: i32) -> i32 {
        use std::os::fd::AsRawFd;
        let result = unsafe {
            libc::setsockopt(
                self.stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF,
                (&bytes as *const i32).cast(), std::mem::size_of_val(&bytes) as libc::socklen_t,
            )
        };
        assert_eq!(result, 0, "set RPC client receive buffer: {}", std::io::Error::last_os_error());
        let mut actual = 0i32;
        let mut len = std::mem::size_of_val(&actual) as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                self.stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF,
                (&mut actual as *mut i32).cast(), &mut len,
            )
        };
        assert_eq!(result, 0, "read RPC client receive buffer: {}", std::io::Error::last_os_error());
        actual
    }

    fn frame(&mut self, opcode: u8, payload: &[u8]) {
        let mut frame = vec![0x80 | opcode];
        if payload.len() < 126 {
            frame.push(0x80 | payload.len() as u8);
        } else if payload.len() <= u16::MAX as usize {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
        let mask = [0x12, 0x34, 0x56, 0x78];
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, byte)| byte ^ mask[i % 4]));
        self.stream.write_all(&frame).expect("write masked websocket frame");
    }

    fn receive(&mut self, deadline: Instant) -> Option<Value> {
        loop {
            if self.input.len() >= 2 {
                let opcode = self.input[0] & 0x0f;
                assert_ne!(self.input[0] & 0x80, 0, "server sends complete JSON frames");
                assert_eq!(self.input[1] & 0x80, 0, "server frames are unmasked");
                let short_len = self.input[1] & 0x7f;
                let header = match short_len { 126 => 4, 127 => 10, _ => 2 };
                if self.input.len() >= header {
                    let len = match short_len {
                        126 => u16::from_be_bytes(self.input[2..4].try_into().unwrap()) as usize,
                        127 => usize::try_from(u64::from_be_bytes(self.input[2..10].try_into().unwrap())).unwrap(),
                        other => other as usize,
                    };
                    assert!(len <= 256 * 1024, "subscription frame exceeds 256 KiB");
                    if self.input.len() >= header + len {
                        let payload = self.input[header..header + len].to_vec();
                        self.input.drain(..header + len);
                        match opcode {
                            1 => return Some(serde_json::from_slice(&payload).expect("JSON websocket frame")),
                            8 => { self.closed = true; return None; }
                            9 => self.frame(10, &payload),
                            10 => {},
                            other => panic!("unexpected server websocket opcode {other}"),
                        }
                        continue;
                    }
                }
            }
            if self.closed || Instant::now() >= deadline { return None; }
            self.stream.set_read_timeout(Some(deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1)))).unwrap();
            let mut chunk = [0; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => { self.closed = true; return None; }
                Ok(n) => self.input.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return None,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted) => {
                    self.closed = true;
                    return None;
                },
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {},
                Err(e) => panic!("read RPC websocket: {e}"),
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let id = self.id;
        self.frame(1, &serde_json::to_vec(&json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})).unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let message = self.receive(deadline).expect("websocket RPC response before timeout");
            if message["id"] == id { return message; }
            self.pending.push(message);
            assert!(self.pending.len() <= 1024, "bounded test notification buffer");
        }
    }

    fn result(&mut self, method: &str, params: Value) -> Value {
        let answer = self.request(method, params);
        assert!(answer.get("error").is_none(), "{method}: {answer}");
        answer.get("result").cloned().expect("websocket RPC result")
    }

    fn subscribe(&mut self, method: &str, params: Value) -> String {
        let result = self.result(method, params);
        let id = result.as_str().expect("hex subscription identifier");
        assert!(id.starts_with("0x") && id.len() > 2 && id[2..].bytes().all(|b| b.is_ascii_hexdigit()), "{id}");
        id.to_string()
    }

    fn notification(&mut self, subscription: &str, wait: Duration) -> Option<Value> {
        let matches = |v: &Value| v["params"]["subscription"].as_str() == Some(subscription);
        if let Some(i) = self.pending.iter().position(matches) { return Some(self.pending.remove(i)); }
        let deadline = Instant::now() + wait;
        while let Some(v) = self.receive(deadline) {
            if matches(&v) { return Some(v); }
            self.pending.push(v);
            assert!(self.pending.len() <= 1024, "bounded test notification buffer");
        }
        None
    }

    fn close(&mut self) {
        self.frame(8, &1000u16.to_be_bytes());
        let _ = self.stream.shutdown(Shutdown::Both);
        self.closed = true;
    }
}

impl Drop for RpcSocket {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

fn hex_quantity(value: &Value) -> u64 {
    u64::from_str_radix(value.as_str().expect("hex quantity").strip_prefix("0x").expect("0x prefix"), 16).unwrap()
}

fn slow_websocket_reader_is_evicted_while_validators_finalize(net: &Net) {
    const CALLS_PER_BURST: u64 = 12;
    const BURSTS: u64 = 2;
    const LOGS_PER_CALL: usize = 4;
    const DATA_BYTES: usize = 32 * 1024;
    let started = Instant::now();
    let deadline = started + Duration::from_secs(60);
    // Four LOG0 instructions reuse the same zeroed 32 KiB memory region.
    // Gas: 21,000 intrinsic + 4*(375+8*32,768) + 5,120 memory + 24 pushes
    // = 1,076,220. The 2,000,000 cap leaves room without filling a whole block.
    let runtime = format!("{}00", "6180006000a0".repeat(LOGS_PER_CALL));
    let len = runtime.len() / 2;
    let init = format!("60{len:02x}600c60003960{len:02x}6000f3{runtime}");
    let out = net.cli(&["deploy", "--rpc", &net.url(1), "--from-dev", "2", "--code", &init]);
    let emitter = out.lines().find_map(|line| line.strip_prefix("contract: ")).expect("backpressure log emitter").trim().to_string();
    let deploy_hash = out.split_whitespace().nth(1).expect("deployment transaction hash");
    wait_receipt(net, 0, deploy_hash, 15);
    let mut slow = RpcSocket::connect(net, 0);
    let slow_buffer = slow.receive_buffer(1024);
    assert!(slow_buffer <= 4096, "tiny slow-reader TCP receive window: {slow_buffer}");
    let slow_id = slow.subscribe("eth_subscribe", json!(["logs", {"address":emitter}]));
    let mut fast = RpcSocket::connect(net, 0);
    fast.receive_buffer(1024 * 1024);
    let fast_id = fast.subscribe("eth_subscribe", json!(["logs", {"address":emitter}]));
    let expected_logs = CALLS_PER_BURST as usize * BURSTS as usize * LOGS_PER_CALL;
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut received = std::collections::BTreeMap::<String, usize>::new();
        ready_tx.send(()).unwrap();
        for _ in 0..expected_logs {
            let notification = fast.notification(&fast_id, deadline.saturating_duration_since(Instant::now())).expect("healthy websocket reader receives every emitted log");
            let log = &notification["params"]["result"];
            assert_eq!(notification["method"], "eth_subscription");
            assert!(log.get("kind").is_none(), "healthy reader must not receive a gap: {log}");
            assert_eq!(log["data"].as_str().unwrap().len(), DATA_BYTES * 2 + 2);
            assert_eq!(log["removed"], false);
            let hash = log["transactionHash"].as_str().unwrap().to_string();
            *received.entry(hash).or_default() += 1;
        }
        assert_eq!(fast.result("eth_unsubscribe", json!([fast_id])), true);
        fast.close();
        received
    });
    ready_rx.recv_timeout(Duration::from_secs(2)).expect("healthy reader active before emissions");
    let status = net.rpc(0, "aether_status", json!([])).unwrap();
    let chain_id = status["chain_id"].as_u64().unwrap();
    let signer = aether_crypto::P256Signer::from_seed(&aether_node::chain::dev_seed(1)).unwrap();
    let sender = dev_address(1);
    let nonce = hex_quantity(&net.rpc(0, "eth_getTransactionCount", json!([sender])).unwrap());
    let call = aether_execution::EvmCall {
        to: Some(emitter.parse().unwrap()), value: aether_types::U256::ZERO,
        input: aether_types::Bytes::new(), gas_limit: 2_000_000, delegate: None,
    };
    let mut hashes = Vec::new();
    let mut finalized_height = 0;
    for burst in 0..BURSTS {
        let begin = hashes.len();
        for i in 0..CALLS_PER_BURST {
            assert!(Instant::now() < deadline, "bounded backpressure submission phase");
            let tx = aether_execution::sign_call(&signer, chain_id, nonce + burst * CALLS_PER_BURST + i, 100_000_000_000, &call).unwrap();
            let hash = format!("{:#x}", aether_execution::tx_hash(&tx));
            let sent = net.rpc(3, "aether_sendTransaction", json!([tx])).expect("submit bounded backpressure call");
            assert_eq!(sent["hash"], hash);
            hashes.push(hash);
        }
        // Separate the bursts at finality: 48 logs (~3 MiB JSON) fit beneath
        // the per-height aggregate cap, while both bursts exceed ordinary TCP
        // send buffers. The slow client performs no reads after its ack.
        let mut waiting: std::collections::BTreeSet<_> = hashes[begin..].iter().cloned().collect();
        while !waiting.is_empty() {
            let mut included = Vec::new();
            for hash in &waiting {
                assert!(Instant::now() < deadline, "bounded backpressure finality phase");
                if let Some(receipt) = net.rpc(0, "aether_getReceipt", json!([hash])) {
                    if let Some(height) = receipt["height"].as_u64() {
                        assert_eq!(receipt["receipt"]["success"], true, "log fixture must execute: {receipt}");
                        assert_eq!(receipt["receipt"]["events"].as_array().unwrap().len(), LOGS_PER_CALL);
                        finalized_height = finalized_height.max(height);
                        included.push(hash.clone());
                    }
                }
            }
            for hash in included { waiting.remove(&hash); }
            if !waiting.is_empty() { std::thread::sleep(Duration::from_millis(50)); }
        }
    }
    let received = reader.join().expect("healthy websocket reader thread");
    assert_eq!(received.len(), hashes.len());
    for hash in hashes { assert_eq!(received.get(&hash), Some(&LOGS_PER_CALL), "healthy reader receives all four logs of {hash}"); }

    // Keep the slow reader paused through the writer's two-second deadline,
    // and prove all four nodes make progress before draining its receive side.
    let reader_done = Instant::now();
    let target = finalized_height + 3;
    loop {
        assert!(Instant::now() < deadline, "node consensus continues under client backpressure");
        let progressed = (0..4).all(|i| net.height(i) >= target);
        if progressed && reader_done.elapsed() >= Duration::from_secs(3) { break; }
        std::thread::sleep(Duration::from_millis(50));
    }
    for node in 0..4 {
        let b = block(net, node, target);
        assert_eq!(b["hash"], block(net, 0, target)["hash"], "all validators agree while slow reader is paused");
    }
    slow.receive_buffer(256 * 1024);
    let mut gap_seen = false;
    let mut slow_logs = 0;
    while let Some(notification) = slow.notification(&slow_id, deadline.saturating_duration_since(Instant::now())) {
        let result = &notification["params"]["result"];
        if result["kind"] == "gap" {
            gap_seen = true;
            assert!(matches!(result["reason"].as_str(), Some("slow_reader" | "queue_overflow")), "backpressure emits an explicit gap: {result}");
        } else {
            slow_logs += 1;
        }
    }
    assert!(slow.closed, "slow TCP client is evicted with a gap or EOF within 60 seconds");
    assert!(gap_seen || slow_logs < expected_logs, "the slow reader does not silently remain subscribed after missing deliveries");
    eprintln!("backpressure: {expected_logs} healthy logs, {slow_logs} buffered slow logs, receive_buffer={slow_buffer}, gap={gap_seen}, validators agree at {target}, elapsed={:?}", started.elapsed());
}

#[test]
fn four_validators_websocket_heads_unsubscribe_and_subscriber_limits() {
    let _serial = serial();
    let net = Net::start_with("rpc-push-heads", vec![vec![]; 4]);
    for i in 0..4 { net.wait_height(i, 3, 60); }
    let mut first = RpcSocket::connect(&net, 0);
    let first_id = first.subscribe("eth_subscribe", json!(["newHeads"]));
    let mut other = RpcSocket::connect(&net, 0);
    let other_id = other.subscribe("eth_subscribe", json!(["newHeads"]));
    aether_node::rpc_push::assert_backpressure_isolated();
    assert_ne!(first_id, other_id);
    assert_eq!(other.result("eth_unsubscribe", json!([first_id])), false, "a connection cannot cancel another's subscription");
    let head = first.notification(&first_id, Duration::from_secs(30)).expect("finalized newHeads notification");
    assert_eq!(head["method"], "eth_subscription");
    let height = hex_quantity(&head["params"]["result"]["number"]);
    let finalized = block(&net, 0, height);
    assert_eq!(head["params"]["result"]["hash"].as_str().unwrap().trim_start_matches("0x"), finalized["hash"].as_str().unwrap().trim_start_matches("0x"));
    assert_eq!(head["params"]["result"]["stateRoot"], finalized["state_root"]);
    let inactive = json!(["logs", {"address":"0x0000000000000000000000000000000000000000"}]);
    let log_id = first.subscribe("eth_subscribe", inactive.clone());
    let excess = first.request("eth_subscribe", inactive.clone());
    assert_eq!(excess["error"]["code"], -32002, "two subscriptions per connection: {excess}");
    assert_eq!(first.result("eth_unsubscribe", json!([first_id])), true);
    assert_eq!(first.result("eth_unsubscribe", json!([first_id])), false);
    first.pending.retain(|v| v["params"]["subscription"] != first_id);
    assert!(first.notification(&first_id, Duration::from_millis(750)).is_none(), "no notifications after unsubscribe acknowledges");
    assert!(other.notification(&other_id, Duration::from_secs(30)).is_some(), "other connection stays subscribed");
    assert_eq!(first.result("eth_unsubscribe", json!([log_id])), true);
    first.close();
    assert_eq!(other.result("eth_unsubscribe", json!([other_id])), true);
    other.close();

    slow_websocket_reader_is_evicted_while_validators_finalize(&net);

    // Quiet log filters make the exact node-wide cap deterministic: no socket
    // is evicted by a queue filled with notifications while slots are counted.
    let mut sockets = Vec::new();
    let mut ids = Vec::new();
    for _ in 0..64 {
        let mut socket = RpcSocket::connect(&net, 0);
        ids.push(socket.subscribe("eth_subscribe", inactive.clone()));
        socket.subscribe("eth_subscribe", inactive.clone());
        sockets.push(socket);
    }
    let mut overflow = RpcSocket::connect(&net, 0);
    let refused = overflow.request("eth_subscribe", inactive.clone());
    assert_eq!(refused["error"]["code"], -32002, "128 subscriptions per node: {refused}");
    let mut independent = RpcSocket::connect(&net, 1);
    independent.subscribe("eth_subscribe", inactive.clone());
    assert_eq!(sockets[0].result("eth_unsubscribe", json!([ids[0]])), true);
    overflow.subscribe("eth_subscribe", inactive.clone());
    sockets[0].close();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let reply = overflow.request("eth_subscribe", inactive.clone());
        if reply.get("result").is_some() { break; }
        assert_eq!(reply["error"]["code"], -32002, "{reply}");
        assert!(Instant::now() < deadline, "closing a connection must release its remaining subscription");
        std::thread::sleep(Duration::from_millis(20));
    }
    let next = net.height(0) + 2;
    for i in 0..4 { net.wait_height(i, next, 30); }
    assert_agree(&net, &[0, 1, 2, 3], next);
}

#[test]
fn four_validators_websocket_logs_match_filters_and_finalized_receipts() {
    let _serial = serial();
    let net = Net::start_with("rpc-push-logs", vec![vec![]; 4]);
    for i in 0..4 { net.wait_height(i, 3, 60); }
    // This handshake is also the old-code RED gate, before fixture deployment.
    let mut socket = RpcSocket::connect(&net, 0);
    let topic_a = format!("0x{}", "11".repeat(32));
    let topic_b = format!("0x{}", "22".repeat(32));
    let noise = format!("0x{}", "aa".repeat(32));
    let indexed = "bb".repeat(32);
    // Emit a noise LOG2 first, then LOG2(calldata[0], calldata[32]). This makes
    // the matched log's block index independent of its filtered result index.
    let runtime = format!("6020357f{}60006000a260203560003560006000a200", "aa".repeat(32));
    let len = runtime.len() / 2;
    let init = format!("60{len:02x}600c60003960{len:02x}6000f3{runtime}");
    let deploy = |dev: &str| {
        let out = net.cli(&["deploy", "--rpc", &net.url(1), "--from-dev", dev, "--code", &init]);
        out.lines().find_map(|l| l.strip_prefix("contract: ")).expect("log emitter deployed").trim().to_string()
    };
    let emitter = deploy("2");
    let other_emitter = deploy("3");
    net.wait_height(0, net.height(1), 30);
    let filter = json!({"address":[emitter], "topics":[[topic_a, topic_b], null]});
    let filtered = socket.subscribe("eth_subscribe", json!(["logs", filter]));
    let noisy = socket.subscribe("eth_subscribe", json!(["logs", {"address":emitter, "topics":[noise]}]));
    let mut excluded = RpcSocket::connect(&net, 0);
    let absent = excluded.subscribe("eth_subscribe", json!(["logs", {"address":"0x0000000000000000000000000000000000000000"}]));
    let from = net.height(0) + 1;
    let mut matching = Vec::new();
    let mut all_hashes = Vec::new();
    for (address, topic, expected) in [(&emitter, &topic_a, true), (&emitter, &topic_b, true), (&emitter, &format!("0x{}", "99".repeat(32)), false), (&other_emitter, &topic_a, false)] {
        let data = format!("{}{indexed}", topic.trim_start_matches("0x"));
        let out = net.cli(&["call", "--rpc", &net.url(3), "--from-dev", "4", "--to", address, "--data", &data, "--wait"]);
        assert!(out.contains("success=true"), "{out}");
        let hash = out.split_whitespace().nth(1).expect("call transaction hash").to_string();
        let receipt = wait_receipt(&net, 0, &hash, 30);
        assert_eq!(receipt["receipt"]["events"].as_array().unwrap().len(), 2);
        if expected { matching.push((hash.clone(), receipt)); }
        all_hashes.push(hash);
    }
    let to = net.height(0);
    let mut query = filter;
    query["fromBlock"] = json!(format!("0x{from:x}"));
    query["toBlock"] = json!(format!("0x{to:x}"));
    let expected = net.rpc(0, "eth_getLogs", json!([query])).unwrap();
    let expected = expected.as_array().unwrap();
    assert_eq!(expected.len(), 2, "OR topics match both target logs and exclude another topic/emitter");
    for (i, (hash, receipt)) in matching.iter().enumerate() {
        let notification = socket.notification(&filtered, Duration::from_secs(30)).expect("matching finalized log notification");
        assert_eq!(notification["method"], "eth_subscription");
        let log = &notification["params"]["result"];
        assert_eq!(log, &expected[i], "subscription and historical log RPC agree");
        assert_eq!(log["transactionHash"], *hash);
        assert_eq!(hex_quantity(&log["blockNumber"]), receipt["height"].as_u64().unwrap());
        let b = block(&net, 0, receipt["height"].as_u64().unwrap());
        let tx_index = b["txs"].as_array().unwrap().iter().position(|tx| tx == hash).unwrap() as u64;
        assert_eq!(hex_quantity(&log["transactionIndex"]), tx_index);
        assert_eq!(log["logIndex"], "0x1", "noise log consumes block log index zero");
        assert_eq!(log["removed"], false, "only finalized logs are pushed");
        assert_eq!(log["topics"][1], format!("0x{indexed}"), "null positional filter preserves the indexed topic");
        for node in 1..4 { wait_receipt(&net, node, hash, 30); }
    }
    for hash in &all_hashes[..3] {
        let notification = socket.notification(&noisy, Duration::from_secs(30)).expect("independent topic filter");
        assert_eq!(notification["params"]["result"]["transactionHash"], *hash);
        assert_eq!(notification["params"]["result"]["logIndex"], "0x0");
    }
    assert!(socket.notification(&filtered, Duration::from_millis(750)).is_none(), "excluded topic and emitter do not leak into the subscription");
    assert!(excluded.notification(&absent, Duration::from_millis(750)).is_none(), "address filter excludes other emitters");
}

fn has_wallet_topic(notification: &Value, topic: &str) -> bool {
    notification["params"]["result"]["topics"].as_array().is_some_and(|topics| topics.iter().any(|t| t == topic))
}

#[test]
fn four_validators_own_node_wallet_push_replays_and_reports_gap() {
    let _serial = serial();
    let net = Net::start_with("rpc-push-wallet", vec![vec![]; 4]);
    for i in 0..4 { net.wait_height(i, 3, 60); }
    let mut socket = RpcSocket::connect(&net, 0);
    let address = dev_address(2);
    let chain_id = net.rpc(0, "aether_status", json!([])).unwrap()["chain_id"].as_u64().unwrap();
    let signer = aether_crypto::P256Signer::from_seed(&aether_node::chain::dev_seed(1)).unwrap();
    let transfer = |nonce, value| {
        aether_execution::sign_call(&signer, chain_id, nonce, 100_000_000_000, &aether_execution::EvmCall {
            to: Some(address.parse().unwrap()), value: aether_types::U256::from(value),
            input: aether_types::Bytes::new(), gas_limit: 21_000, delegate: None,
        }).unwrap()
    };
    // Know both hashes before subscription, including the transaction that
    // will finalize while this wallet is disconnected.
    let tx = transfer(0, 777u64);
    let offline_tx = transfer(1, 888u64);
    let hash = format!("{:#x}", aether_execution::tx_hash(&tx));
    let offline_hash = format!("{:#x}", aether_execution::tx_hash(&offline_tx));
    let subscription = socket.subscribe("aether_subscribe", json!(["wallet", {"address":address, "transactions":[hash, offline_hash]}]));
    let initial = socket.notification(&subscription, Duration::from_secs(5)).expect("wallet initial snapshot");
    assert_eq!(initial["method"], "aether_subscription");
    assert_eq!(initial["params"]["result"]["kind"], "wallet");
    for topic in ["head", "balance", "tx_status", "release"] {
        assert!(has_wallet_topic(&initial, topic), "initial wallet view includes {topic}: {initial}");
    }
    let initial_height = initial["params"]["result"]["height"].as_u64().unwrap();
    let sent = net.rpc(3, "aether_sendTransaction", json!([tx])).expect("submit tracked transfer");
    assert_eq!(sent["hash"], hash);
    let receipt = wait_receipt(&net, 0, &hash, 30);
    assert_eq!(receipt["receipt"]["success"], true);
    let height = receipt["height"].as_u64().unwrap();
    assert!(height > initial_height, "subscribed before transaction finality");
    let mut last = initial_height;
    loop {
        let notification = socket.notification(&subscription, Duration::from_secs(30)).expect("live own-node wallet watermark");
        let current = notification["params"]["result"]["height"].as_u64().unwrap();
        assert!(current == last || current == last + 1, "every finalized height advances the wallet, including empty blocks");
        assert!(has_wallet_topic(&notification, "head"));
        last = current;
        if current == height {
            assert!(has_wallet_topic(&notification, "balance"), "recipient balance change is pushed at finality: {notification}");
            assert!(has_wallet_topic(&notification, "tx_status"), "watched transaction finality is pushed: {notification}");
            break;
        }
        assert!(current < height, "must not skip the finalized transfer height");
    }

    // Legacy devnet release notices already carry the committee threshold
    // signature. Announce one far beyond this test's lifetime, so no protocol
    // transition or update installer is triggered by the fixture.
    let (_, sharing, shares) = aether_light::devnet_threshold(4);
    let upgrade = aether_node::upgrade::Upgrade {
        chain_id, protocol: 2, activate_at: net.height(0) + aether_node::upgrade::MAINNET_NOTICE_BLOCKS + 10_000, emergency: false,
        releases: vec![aether_node::upgrade::Release {
            platform: "macos-arm64-dmg".to_string(), version: "0.0.0-rpc-push-fixture".to_string(),
            blake3: "ab".repeat(32), url: "https://example.invalid/rpc-push-fixture.dmg".to_string(),
        }],
        notes: "isolated wallet push integration fixture".to_string(), registrar: None,
    };
    let partials = shares.iter().take(3).map(|(_, share)| aether_node::upgrade::sign_partial(&upgrade, share)).collect::<Vec<_>>();
    let signed = aether_node::upgrade::combine(&sharing, &partials).unwrap();
    let signed_bytes = serde_json::to_vec(&signed).unwrap();
    for node in 0..4 {
        let dir = net.data(node).join("upgrades");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rpc-push-fixture.json"), &signed_bytes).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let notification = socket.notification(&subscription, deadline.saturating_duration_since(Instant::now())).expect("finalized release notice is pushed");
        let current = notification["params"]["result"]["height"].as_u64().unwrap();
        assert!(current == last || current == last + 1, "release notice stream has complete finalized head watermarks");
        if current > height {
            assert!(!has_wallet_topic(&notification, "balance"), "unchanged balances must not trigger repeated wallet refreshes");
            assert!(!has_wallet_topic(&notification, "tx_status"), "unchanged watched receipts must not trigger repeated wallet refreshes");
        }
        last = current;
        if has_wallet_topic(&notification, "release") {
            let status = net.rpc(0, "aether_status", json!([])).unwrap();
            assert!(status["upcoming_upgrades"].as_array().unwrap().iter().any(|notice| notice == &json!(signed)), "release hint comes from a finalized committee-signed notice: {status}");
            break;
        }
    }
    socket.close();
    net.rpc(2, "aether_sendTransaction", json!([offline_tx])).expect("transfer while wallet disconnected");
    let offline_receipt = wait_receipt(&net, 0, &offline_hash, 30);
    assert_eq!(offline_receipt["receipt"]["success"], true);
    let offline_height = offline_receipt["height"].as_u64().unwrap();
    net.wait_height(0, offline_height + 2, 30);
    let watermark = net.height(0);
    let mut resumed = RpcSocket::connect(&net, 0);
    let resumed_id = resumed.subscribe("aether_subscribe", json!(["wallet", {"address":address, "transactions":[hash, offline_hash], "after":last}]));
    let mut received = std::collections::BTreeSet::new();
    let mut previous = last;
    let mut offline_hint = false;
    loop {
        let notification = resumed.notification(&resumed_id, Duration::from_secs(30)).expect("bounded reconnect replay");
        assert_eq!(notification["params"]["result"]["kind"], "wallet", "retained history resumes without a gap: {notification}");
        let current = notification["params"]["result"]["height"].as_u64().unwrap();
        assert!(current >= previous, "replay stays ordered across initial snapshot and live registration");
        assert!(current >= last, "a durable cursor is never rolled backwards");
        previous = current;
        received.insert(current);
        if current == offline_height {
            offline_hint |= has_wallet_topic(&notification, "balance") && has_wallet_topic(&notification, "tx_status");
        }
        if current >= watermark { break; }
    }
    for expected in last + 1..=watermark {
        assert!(received.contains(&expected), "no replay-to-live height gap at {expected}");
    }
    assert!(offline_hint, "offline transaction is present in the own-node catch-up view");
    assert_eq!(resumed.result("eth_unsubscribe", json!([resumed_id])), true);
    resumed.close();

    // A cursor outside the node's retained finalized history cannot look like
    // an empty inbox. The explicit gap is followed by stream termination.
    let mut gap_socket = RpcSocket::connect(&net, 0);
    let gap_id = gap_socket.subscribe("aether_subscribe", json!(["wallet", {"address":address, "transactions":[], "after":u64::MAX}]));
    let gap = gap_socket.notification(&gap_id, Duration::from_secs(5)).expect("explicit history-unavailable gap");
    assert_eq!(gap["params"]["result"]["kind"], "gap");
    assert_eq!(gap["params"]["result"]["reason"], "history_unavailable");
    assert!(gap_socket.receive(Instant::now() + Duration::from_secs(5)).is_none());
    assert!(gap_socket.closed, "gap terminates the stream");
    assert_agree(&net, &[0, 1, 2, 3], offline_height);
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
    let p2p: Vec<Port> = (0..n).map(|_| Port::reserve().expect("reserve test port")).collect();
    let rpc: Vec<Port> = (0..n).map(|_| Port::reserve().expect("reserve test port")).collect();
    let mut net = Net::prepared(dir.clone(), p2p, rpc, vec![vec![]; n]);

    let dkg: Vec<TestChild> = (0..n)
        .map(|i| {
            let mut cmd = Command::new(BIN);
            cmd.args(["dkg", "--index", &(i + 1).to_string(), "--validators", &n.to_string(), "--port", &net.p2p[i].to_string()])
                .args(["--data", dir.join((i + 1).to_string()).to_str().unwrap(), "--peers", &net.peers(i), "--offline"]);
            net.capture(i, cmd)
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

/// `aether keygen` is the documented first install of a validator ("Run a real
/// network" in the README), so the directory it writes must be one a node can
/// load. An identity holds two secrets — the voting key and the node account
/// that pays for and sends beacons — and a directory with the voting key but
/// no account key is read as a *lost* identity and never runs (candidate.rs,
/// red team #5: no replacement signer is ever minted). keygen is that first
/// install, so it writes both, exactly as `aether run` does on a fresh
/// directory; it still refuses to overwrite either one.
#[test]
fn keygen_writes_the_whole_identity_a_node_loads() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("aether-devnet-test-{}-keygen", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let d = dir.to_str().unwrap();
    run_ok(&["keygen", "--data", d]);

    let account = dir.join("node-account.key");
    assert!(account.exists(), "keygen installs the node account key with the voting key");
    assert_eq!(std::fs::metadata(&account).unwrap().permissions().mode() & 0o777, 0o600, "the account secret is 0600");

    // The node loads the directory instead of reporting "node-account.key is
    // missing from an existing identity" and starting keyless.
    let info: Value = serde_json::from_str(&run_ok(&["candidate-info", "--data", d])).unwrap();
    assert!(!info["validator_key"].as_str().unwrap().is_empty() && !info["beaconer"].as_str().unwrap().is_empty(), "{info}");

    // Neither secret is ever replaced: keygen refuses a directory it made.
    let (key, acct) = (std::fs::read(dir.join("validator.key")).unwrap(), std::fs::read(&account).unwrap());
    let again = Command::new(BIN).args(["keygen", "--data", d]).output().unwrap();
    assert!(!again.status.success(), "keygen must not overwrite: {}", String::from_utf8_lossy(&again.stderr));
    assert_eq!(std::fs::read(dir.join("validator.key")).unwrap(), key, "the voting key is kept");
    assert_eq!(std::fs::read(&account).unwrap(), acct, "the account key is kept");
    let _ = std::fs::remove_dir_all(&dir);
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
    let again = Command::new(BIN).args(["keygen", "--data", data(0).to_str().unwrap()]).output().unwrap();
    assert!(!again.status.success(), "keygen must not overwrite: {}", String::from_utf8_lossy(&again.stderr));

    // 2. a faucet key on validator 1's machine, then network.json from the public
    //    halves: genesis funds only the faucet (no public dev keys).
    let out = Command::new(BIN).args(["faucet-key", "--data", data(0).to_str().unwrap()]).output().unwrap();
    assert!(out.status.success());
    let faucet_addr = String::from_utf8_lossy(&out.stdout).split_whitespace().nth(2).unwrap().to_string();
    let pubs: Vec<String> = (0..n).map(|i| data(i).join("validator.pub.json").to_str().unwrap().to_string()).collect();
    let out = Command::new(BIN).args(["network", "--faucet", &faucet_addr, "--epoch-blocks", "10"]).args(&pubs).output().unwrap();
    assert!(out.status.success());
    let network = dir.join("network.json");
    std::fs::write(&network, &out.stdout).unwrap();
    let net_arg = network.to_str().unwrap().to_string();

    // 3. DKG on those keys (no --index: each process finds itself by its key).
    let p2p: Vec<Port> = (0..n).map(|_| Port::reserve().expect("reserve test port")).collect();
    let rpc: Vec<Port> = (0..n).map(|_| Port::reserve().expect("reserve test port")).collect();
    // Nodes run from the network.json each DKG wrote (it adds the identity).
    let extra = (0..n)
        .map(|i| {
            let mut a = vec!["--network".to_string(), data(i).join("network.json").to_str().unwrap().to_string()];
            if i == 0 {
                a.extend(["--faucet-key".to_string(), data(0).join("faucet.key").to_str().unwrap().to_string()]);
            }
            a
        })
        .collect();
    let mut net = Net::prepared(dir.clone(), p2p, rpc, extra);
    let dkg: Vec<TestChild> = (0..n)
        .map(|i| {
            let mut cmd = Command::new(BIN);
            cmd.args(["dkg", "--network", &net_arg, "--port", &net.p2p[i].to_string(), "--data", data(i).to_str().unwrap()])
                .args(["--peers", &net.peers(i), "--offline"]);
            net.capture(i, cmd)
        })
        .collect();
    for c in dkg {
        assert!(c.wait_with_output().unwrap().status.success(), "dkg failed");
    }
    let written: Value = serde_json::from_slice(&std::fs::read(data(0).join("network.json")).unwrap()).unwrap();
    let identity = written["identity"].as_str().expect("identity in network.json").to_string();
    assert_eq!(written["validators"].as_array().unwrap().len(), n);

    // 3.5 The ceremony's record, made the way the real ceremony makes it: the
    // coordinator's check writes ceremony-check.json over validator 1's final
    // file, and every Mac binds to it (verify-local) before its node starts
    // voting — no validator votes from a genesis the ceremony did not check.
    let final_net = data(0).join("network.json");
    let record = coordinator_check(final_net.to_str().unwrap());
    for i in 0..n {
        verify_local(final_net.to_str().unwrap(), data(i).to_str().unwrap(), &record);
    }

    // 4. consensus from the network file (spawn passes only --network, no --index).
    for i in 0..n {
        net.spawn_with_network(i);
    }
    for i in 0..n {
        net.wait_height(i, 3, 60);
    }
    // 5. public dev keys hold nothing here; test tokens come only from the faucet.
    let dev = dev_address(3);
    let dev_bal = net.cli(&["balance", &dev, "--rpc", &net.url(2), "--identity", &identity]);
    assert!(dev_bal.contains("balance   0 wei"), "a public dev key is funded on a public network: {dev_bal}");
    let bob = "0x000000000000000000000000000000000000cafe";
    assert!(net.rpc(1, "aether_faucet", json!([bob])).is_none(), "only the node with the faucet key answers");
    let grant = net.rpc(0, "aether_faucet", json!([bob])).expect("faucet grant");
    wait_receipt(&net, 0, grant["hash"].as_str().unwrap(), 30);
    net.wait_height(3, net.height(0), 20);
    let bal = net.cli(&["balance", bob, "--rpc", &net.url(3), "--identity", &identity]);
    assert!(bal.contains("balance   10000000000000000000 wei") && bal.contains("verified  ✓"), "{bal}");
    std::thread::sleep(Duration::from_millis(1_100));
    assert!(net.rpc(0, "aether_faucet", json!([bob])).is_none(), "second grant to the same address within the cooldown");
    // 6. A protocol upgrade signed by 3 of the 4 validators' key shares goes on
    //    chain (at least one epoch of notice); nodes that do not run the new
    //    protocol stop before it activates instead of forking.
    let target = net.height(0) + 40;
    let next = aether_node::upgrade::PROTOCOL + 1;
    let upgrade = json!({ "chain_id": written["chain_id"], "protocol": next, "activate_at": target, "releases": [], "notes": "test" });
    let up_path = dir.join("upgrade.json");
    std::fs::write(&up_path, upgrade.to_string()).unwrap();
    let net_file = data(0).join("network.json");
    let partials: Vec<String> = (0..3)
        .map(|i| {
            let out = net.cli(&["upgrade-sign", "--data", data(i).to_str().unwrap(), "--network", net_file.to_str().unwrap(), up_path.to_str().unwrap()]);
            let p = dir.join(format!("partial{i}.json"));
            std::fs::write(&p, out).unwrap();
            p.to_str().unwrap().to_string()
        })
        .collect();
    let two = net.cli_fails(&["upgrade-combine", "--network", net_file.to_str().unwrap(), &partials[0], &partials[1]]);
    assert!(two.contains("need 3"), "two validators alone cannot sign: {two}");
    let mut args = vec!["upgrade-combine", "--network", net_file.to_str().unwrap()];
    args.extend(partials.iter().map(String::as_str));
    let signed = net.cli(&args);
    let signed_path = dir.join("signed.json");
    std::fs::write(&signed_path, &signed).unwrap();
    assert!(net.cli(&["upgrade-verify", "--network", net_file.to_str().unwrap(), signed_path.to_str().unwrap()]).contains("signed by the committee"));
    // A forged copy (protocol changed) is ignored; the signed one stops the nodes.
    let mut forged: Value = serde_json::from_str(&signed).unwrap();
    forged["upgrade"]["protocol"] = json!(1);
    forged["upgrade"]["activate_at"] = json!(1);
    for i in 0..n {
        let d = data(i).join("upgrades");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("forged.json"), forged.to_string()).unwrap();
        std::fs::write(d.join("v2.json"), &signed).unwrap();
    }
    let end = Instant::now() + Duration::from_secs(120);
    while Instant::now() < end && (0..n).any(|i| net.alive(i)) {
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!((0..n).all(|i| !net.alive(i)), "nodes without the new protocol must stop before the upgrade activates");
    let last: u64 = (0..n)
        .map(|i| {
            let out = Command::new(BIN).args(["head", "--data", data(i).to_str().unwrap()]).output().unwrap();
            String::from_utf8_lossy(&out.stdout).split_whitespace().next().and_then(|h| h.parse().ok()).unwrap_or(0)
        })
        .max()
        .unwrap();
    assert!(last < target, "finalized {last}, but the upgrade activates at {target}");
}

fn run_ok(args: &[&str]) -> String {
    let out = Command::new(BIN).args(args).output().expect("run");
    assert!(out.status.success(), "{:?}: {}", args, String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// The coordinator's step after the DKG (scripts/mainnet-genesis.sh `check`
/// on PASS): write ceremony-check.json next to the final network.json — the
/// pin over the exact bytes that passed — with the same `aether
/// ceremony-record` command the script runs. Returns the record's path.
fn coordinator_check(final_net: &str) -> String {
    let record = std::path::Path::new(final_net)
        .parent()
        .unwrap()
        .join("ceremony-check.json");
    run_ok(&["ceremony-record", "--network", final_net, "--out", record.to_str().unwrap()]);
    record.to_str().unwrap().to_string()
}

/// Each validator Mac's step before it votes (scripts/mainnet-genesis.sh
/// `verify-local`): the same `aether mainnet-bind` the script runs — the
/// record against the final file's bytes and this Mac's network.json and
/// threshold.json — which also stores the record in the data dir, where the
/// node's startup bind finds it (audit 6, A6-3/A6-4).
fn verify_local(final_net: &str, data: &str, record: &str) {
    run_ok(&["mainnet-bind", "--network", final_net, "--data", data, "--ceremony", record]);
    assert!(
        std::path::Path::new(data).join("ceremony-check.json").exists(),
        "verify-local must store the record in {data}'s data dir"
    );
}

fn spawn_quiet(args: &[String]) -> TestChild {
    static NEXT_LOG: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let index = NEXT_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let log = std::env::temp_dir().join(format!("aether-devnet-child-{}-{index}.log", std::process::id()));
    spawn_logged(log, args)
}

fn spawn_logged(log: impl AsRef<std::path::Path>, args: &[String]) -> TestChild {
    let mut cmd = Command::new(BIN);
    cmd.args(args).env("RUST_LOG", "warn");
    TestChild::spawn(cmd, log).expect("spawn")
}

fn tcp_peers(ports: &[Port], me: usize) -> String {
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
    // A network.json genesis funds only the faucet (mainnet funds nobody), so
    // this chain gets one, as the testnet does: machine 1 holds the key.
    let faucet = run_ok(&["faucet-key", "--data", &d(1)]);
    let faucet_addr = faucet.split_whitespace().nth(2).unwrap().to_string();
    let pubs = |ids: &[usize]| ids.iter().map(|i| format!("{}/validator.pub.json", d(*i))).collect::<Vec<_>>();
    let network = |ids: &[usize]| {
        let mut a = vec!["network".to_string(), "--faucet".to_string(), faucet_addr.clone()];
        a.extend(pubs(ids));
        run_ok(&a.iter().map(String::as_str).collect::<Vec<_>>())
    };
    let net_a = network(&[1, 2, 3, 4]);
    let net_b = network(&[2, 3, 4, 5]);
    std::fs::write(path("A.json"), net_a).unwrap();
    std::fs::write(path("B.json"), net_b).unwrap();

    // DKG for A.
    let ports: Vec<Port> = (0..4).map(|_| Port::reserve().expect("reserve test port")).collect();
    let dkg: Vec<TestChild> = (0..4)
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
        let out = c.wait_with_output().unwrap();
        assert!(out.status.success(), "dkg failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    std::fs::copy(format!("{}/network.json", d(1)), path("A-final.json")).unwrap();
    let identity = serde_json::from_slice::<Value>(&std::fs::read(path("A-final.json")).unwrap()).unwrap()["identity"].as_str().unwrap().to_string();
    // The ceremony's record over committee A's final file, and each member's
    // verify-local before it votes (the node makes the same bind at startup).
    let record = coordinator_check(&path("A-final.json"));
    for i in 1..=4 {
        verify_local(&path("A-final.json"), &d(i), &record);
    }

    // Committee A runs; pay 0xaa.
    let p2p: Vec<Port> = (0..4).map(|_| Port::reserve().expect("reserve test port")).collect();
    let rpc: Vec<Port> = (0..4).map(|_| Port::reserve().expect("reserve test port")).collect();
    let extra = (1..=4)
        .map(|i| {
            let mut a = vec!["--network".to_string(), format!("{}/network.json", d(i))];
            if i == 1 {
                a.extend(["--faucet-key".to_string(), format!("{}/faucet.key", d(i))]);
            }
            a
        })
        .collect::<Vec<_>>();
    let mut a = Net::prepared(dir.clone(), p2p, rpc, extra);
    for k in 0..4 {
        a.spawn_with_network(k);
    }
    a.wait_height(0, 3, 60);
    // Dev 1 is funded only through the faucet (the genesis funds nobody else).
    faucet_grant(&a, 0, &dev_address(1));
    let aa = "0x00000000000000000000000000000000000000aa";
    a.cli(&["send", "--rpc", &a.url(0), "--from-dev", "1", "--to", aa, "--value", "11", "--wait"]);
    for k in 0..4 {
        a.kill(k);
    }

    // Reshare A -> B at the old committee's last finalized block.
    let heads: Vec<String> = (1..=4).map(|i| run_ok(&["head", "--data", &d(i)]).trim().to_string()).collect();
    let end = heads.iter().max_by_key(|h| h.split(' ').next().unwrap().parse::<u64>().unwrap()).unwrap().clone();
    let (end_h, end_hash) = end.split_once(' ').unwrap();
    let rports: Vec<Port> = (0..5).map(|_| Port::reserve().expect("reserve test port")).collect();
    let rs: Vec<TestChild> = (0..5)
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
    let outs: Vec<String> = rs.into_iter().map(|c| {
        let out = c.wait_with_output().expect("reshare exits");
        assert!(out.status.success(), "reshare failed: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }).collect();
    assert!(outs[0].contains("has left"), "validator 1 leaves: {}", outs[0]);
    for o in &outs[1..] {
        assert!(o.contains(&identity), "identity unchanged: {o}");
    }

    // Committee B continues the chain (validator 5 starts empty and catches up).
    let p2p: Vec<Port> = (0..4).map(|_| Port::reserve().expect("reserve test port")).collect();
    let rpc: Vec<Port> = (0..4).map(|_| Port::reserve().expect("reserve test port")).collect();
    let mut b = Net::prepared(dir.clone(), p2p, rpc, vec![vec!["--network".into(), path("B.json")]; 4]);
    b.logs = (0..4).map(|k| dir.join(format!("nodeB{k}.log"))).collect();
    // B's validator k is machine k + 2: point spawn at those data dirs.
    let mut procs = Vec::new();
    for k in 0..4 {
        let log = dir.join(format!("nodeB{k}.log"));
        procs.push(spawn_logged(
            log,
            &[
                "node".into(),
                "--network".into(),
                // The network.json reshare wrote: epoch schedule and identity.
                format!("{}/network.json", d(k + 2)),
                // The round-0 record still binds: a reshare's file is an
                // evolution of the checked genesis, pinned by the immutable
                // genesis the record carries.
                "--ceremony".into(),
                record.clone(),
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
    b.logs = (0..4).map(|k| dir.join(format!("nodeB{k}.log"))).collect();
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

#[test]
fn a_follower_verifies_everything_and_serves_a_wallet() {
    let _serial = serial();
    let mut net = Net::start(4);
    for i in 0..4 {
        net.wait_height(i, 3, 60);
    }
    let bob = "0x00000000000000000000000000000000000f0110";
    let out = net.cli(&["send", "--rpc", &net.url(0), "--from-dev", "2", "--to", bob, "--value", "777", "--wait"]);
    assert!(out.contains("success=true"), "{out}");

    // A Mac that is not a validator follows from genesis, pulling from two validators.
    let port = Port::reserve().expect("reserve test port");
    let data = net.dir.join("follower");
    let from = format!("{},{}", net.url(1), net.url(2));
    let log = net.dir.join("follower.log");
    let child =
        spawn_logged(log, &["follow".into(), "--from-rpc".into(), from, "--data".into(), data.to_str().unwrap().into(), "--rpc-port".into(), port.to_string()]);
    net.rpc.push(port);
    net.procs.push(Some(child));
    net.logs.push(net.dir.join("follower.log"));
    let f = net.rpc.len() - 1;
    let target = net.height(0) + 2;
    net.wait_height(f, target, 60);
    for h in (1..=target).step_by(3) {
        assert_agree(&net, &[0, f], h);
    }

    // A wallet talking only to the follower verifies balances with its certificates.
    let bal = net.cli(&["balance", bob, "--rpc", &net.url(f)]);
    assert!(bal.contains("balance   777 wei") && bal.contains("verified  ✓"), "{bal}");
    // Transactions sent to the follower reach the validators.
    let out = net.cli(&["send", "--rpc", &net.url(f), "--from-dev", "3", "--to", bob, "--value", "1", "--wait"]);
    assert!(out.contains("success=true"), "{out}");

    // History: a certified block proves an old block through its MMR history root.
    let anchor_h = net.height(0);
    let fin = net.rpc(0, "aether_getFinalized", json!([anchor_h])).expect("anchor");
    let hex = |v: &Value| aether_light::from_hex(v.as_str().unwrap()).unwrap();
    let links: Vec<Vec<u8>> = fin["links"].as_array().map(|a| a.iter().map(hex).collect()).unwrap_or_default();
    let anchor = aether_light::verify_finalized_chain(&aether_light::ValidatorSet::devnet(4), &hex(&fin["block"]), &hex(&fin["finalization"]), &links).unwrap();
    let hp = net.rpc(0, "aether_historyProof", json!([2, anchor_h])).expect("history proof");
    let proof: aether_state::mmr::MmrProof = serde_json::from_value(hp["proof"].clone()).unwrap();
    let hash: aether_types::B256 = format!("0x{}", hp["hash"].as_str().unwrap()).parse().unwrap();
    aether_light::verify_history(&anchor, 2, &hash, &proof).expect("block 2 is in the certified history");
    let wrong: aether_types::B256 = [7u8; 32].into();
    assert!(aether_light::verify_history(&anchor, 2, &wrong, &proof).is_err(), "another hash at height 2");
    // Its full contents too, from bytes any archive could serve.
    let old = net.rpc(0, "aether_getFinalized", json!([2])).expect("block 2");
    let (b2, _) = aether_light::verify_old_block(&anchor, &hex(&old["block"]), &proof).expect("block 2's bytes are certified");
    assert_eq!(b2.height, 2);
}

/// A valid certificate proves a block, not that a single path's replay is
/// today's head. Replay one stale certified head while only one source has
/// the current head; a follower must wait for a second current witness.
#[test]
fn rel24_follower_rejects_a_stale_path_and_waits_for_two_current_sources() {
    let _serial = serial();
    let mut net = Net::start_with("rel24", vec![vec![]; 4]);
    for i in 0..4 { net.wait_height(i, 8, 90); }
    // A real finalized large-read fixture: valid one-byte runtime, with padded
    // init code. Its canonical finalized RPC includes the actual transaction.
    let init = format!("6001600c60003960016000f300{}", "00".repeat(16_000));
    let deployed = net.cli(&["deploy", "--rpc", &net.url(0), "--from-dev", "2", "--code", &init]);
    assert!(deployed.contains("contract: "), "{deployed}");
    let target = net.height(0) + 2;
    for i in 0..4 { net.wait_height(i, target, 60); }
    for i in 0..4 { net.kill(i); }
    // Restore one validator for reads only: the other three remain stopped,
    // so certificates and the DB fixture cannot change during this test.
    net.spawn(0);
    net.wait_height(0, 8, 60);
    let head = net.height(0);
    let mut proofs = std::collections::BTreeMap::new();
    for h in 1..=head {
        proofs.insert(h, net.rpc(0, "aether_getFinalized", json!([h])).unwrap());
    }
    net.kill(0);
    let proofs = std::sync::Arc::new(proofs);

    struct Source {
        url: String,
        enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        stop: Option<tokio::sync::oneshot::Sender<()>>,
        task: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for Source {
        fn drop(&mut self) {
            if let Some(stop) = self.stop.take() { let _ = stop.send(()); }
            if let Some(task) = self.task.take() { let _ = task.join(); }
        }
    }
    let source = |height: u64, enabled: bool| {
        let enabled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(enabled));
        let flag = enabled.clone();
        let proofs = proofs.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let task = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                started_tx.send(format!("http://{}", listener.local_addr().unwrap())).unwrap();
                let app = axum::Router::new().route("/", axum::routing::post(move |axum::Json(req): axum::Json<Value>| {
                    let proofs = proofs.clone();
                    let flag = flag.clone();
                    async move {
                        let result = if !flag.load(std::sync::atomic::Ordering::SeqCst) {
                            Value::Null
                        } else if req["method"] == "aether_status" {
                            json!({"height":height})
                        } else if req["method"] == "aether_getFinalized" {
                            req["params"][0].as_u64().filter(|h| *h <= height)
                                .and_then(|h| proofs.get(&h).cloned()).unwrap_or(Value::Null)
                        } else { Value::Null };
                        axum::Json(json!({"jsonrpc":"2.0", "id":req["id"], "result":result}))
                    }
                }));
                axum::serve(listener, app).with_graceful_shutdown(async { let _ = stopped.await; }).await.unwrap();
            });
        });
        Source {url:started_rx.recv().unwrap(), enabled, stop:Some(stop), task:Some(task)}
    };
    let stale = source(3, true);
    let fresh = source(head, true);
    let corroborator = source(head, false);
    let port = Port::reserve().expect("reserve test port");
    let data = net.dir.join("follower");
    let log = net.dir.join("rel24-follower.log");
    let from = format!("{},{},{}", stale.url, fresh.url, corroborator.url);
    let child = spawn_logged(log, &["follow".into(), "--from-rpc".into(), from,
        "--data".into(), data.to_str().unwrap().into(), "--rpc-port".into(), port.to_string()]);
    net.rpc.push(port);
    net.procs.push(Some(child));
    net.logs.push(net.dir.join("rel24-follower.log"));
    let f = net.rpc.len() - 1;
    let deadline = Instant::now() + Duration::from_secs(45);
    while net.rpc(f, "aether_status", json!([])).is_none() {
        assert!(Instant::now() < deadline, "follower did not start{}", net.log_tail(f));
        std::thread::sleep(Duration::from_millis(100));
    }
    // Wait long enough for several polling rounds. Old code accepts the stale
    // path's three valid blocks, then the sole current source's head.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(net.height(f), 0, "a stale certificate and one current witness must not establish the head{}", net.log_tail(f));
    let status = net.rpc(f, "aether_status", json!([])).unwrap();
    assert_eq!(status["follower_network"]["alert"], true, "HTTP-only devnet paths are explicitly degraded");
    corroborator.enabled.store(true, std::sync::atomic::Ordering::SeqCst);
    net.wait_height(f, head, 60);
    let fin = net.rpc(f, "aether_getFinalized", json!([head])).unwrap();
    assert_eq!(fin["block"], proofs[&head]["block"], "the accepted head is the corroborated certified block");
    fresh.enabled.store(false, std::sync::atomic::Ordering::SeqCst);
    corroborator.enabled.store(false, std::sync::atomic::Ordering::SeqCst);
    std::thread::sleep(Duration::from_secs(2));
    let status = net.rpc(f, "aether_status", json!([])).unwrap();
    assert_eq!(status["follower_network"]["head_confirmed"], false, "cached agreement cannot survive lost sources");
    assert_eq!(net.height(f), head, "a later stale head never rolls state back");
    if let Ok(copy) = std::env::var("AETHER_REL_DB_FIXTURE") {
        let copy = std::path::Path::new(&copy);
        std::fs::create_dir_all(copy).unwrap();
        std::fs::copy(net.data(0).join("state.redb"), copy.join("state.redb")).unwrap();
        let store = aether_node::store::Store::open_for_maintenance(&copy.join("state.redb")).unwrap();
        for (h, proof) in proofs.iter() {
            store.put_proof(*h, proof.to_string().as_bytes()).unwrap();
        }
        drop(store);
        let measurement_height = proofs.iter().max_by_key(|(_, p)| p["block"].as_str().map(str::len).unwrap_or(0)).unwrap().0;
        std::fs::write(copy.join("certificates.json"), serde_json::to_vec(&*proofs).unwrap()).unwrap();
        std::fs::write(copy.join("fixture.json"), json!({"head":head, "measurement_height":measurement_height, "validators":4, "origin":"isolated loopback devnet"}).to_string()).unwrap();
    }
}

/// Open voting nodes, part 2: a follower Mac becomes a candidate. Its owner
/// registers it once (registrar attestation → registry); from then on the node
/// sends a liveness beacon every epoch by itself and its streak grows.
#[test]
fn a_candidate_registers_once_and_beacons_every_epoch() {
    let _serial = serial();
    let epoch = ["--dev-epoch-blocks".to_string(), "20".to_string()];
    let mut extra: Vec<Vec<String>> = (0..4).map(|_| epoch.to_vec()).collect();
    extra[0].push("--dev-registrar".into());
    let mut net = Net::start_with("candidate", extra);
    for i in 0..4 {
        net.wait_height(i, 3, 60);
    }
    let port = Port::reserve().expect("reserve test port");
    let data = net.dir.join("candidate");
    let log = net.dir.join("candidate.log");
    let args: Vec<String> = vec![
        "follow".into(),
        "--from-rpc".into(),
        format!("{},{}", net.url(1), net.url(2)),
        "--data".into(),
        data.to_str().unwrap().into(),
        "--rpc-port".into(),
        port.to_string(),
        "--candidate".into(),
        epoch[0].clone(),
        epoch[1].clone(),
    ];
    net.procs.push(Some(spawn_logged(log, &args)));
    net.rpc.push(port);
    net.logs.push(net.dir.join("candidate.log"));
    let f = net.rpc.len() - 1;
    net.wait_height(f, 3, 60);

    // This legacy devnet has no free lane; its genesis funds dev account 4,
    // so the contract registration pays normally.
    assert_eq!(net.rpc(0, "aether_status", json!([])).unwrap()["free_registration"], false);
    let out = net.cli(&["candidate-register", "--devnet", "--data", data.to_str().unwrap(), "--registrar-rpc", &net.url(0), "--rpc", &net.url(0), "--from-dev", "4"]);
    assert!(out.contains("candidate") && out.contains("success=true"), "{out}");
    let mine = |net: &Net| net.rpc(0, "aether_candidates", json!([])).expect("candidates");
    let c = mine(&net);
    assert_eq!(c["candidates"].as_array().unwrap().len(), 1, "{c}");
    assert_eq!(c["candidates"][0]["operator"].as_str().unwrap().to_lowercase(), dev_address(4).to_lowercase());
    let current_epoch = c["epoch"].as_u64().unwrap();
    assert_eq!(c["next_draw_epoch"], (current_epoch / 24 + 1) * 24, "{c}");
    assert_eq!(c["open_seats"], 1, "a four-validator draw has one seat: {c}");
    let candidate = &c["candidates"][0];
    assert!(candidate["missed"].is_u64(), "missed epochs must be visible: {c}");
    assert_eq!(candidate["eligible_next_draw"], false, "this Mac is still warming up: {c}");
    assert_eq!(candidate["why_not"], "streak", "{c}");
    assert!(candidate["hours_to_eligible"].as_f64().is_some_and(|h| h > 0.0), "a live devnet has a measured ETA: {c}");
    // Registering the same Mac again is refused by the registry.
    assert!(!net
        .cli_fails(&["candidate-register", "--devnet", "--data", data.to_str().unwrap(), "--registrar-rpc", &net.url(0), "--rpc", &net.url(0), "--from-dev", "4"])
        .is_empty());

    // Two more epochs pass: the node beacons by itself and the streak grows.
    let start = c["candidates"][0]["streak"].as_u64().unwrap();
    let e0 = c["epoch"].as_u64().unwrap();
    net.wait_height(0, (e0 + 3) * 20 + 3, 120);
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let c = mine(&net);
        let cand = &c["candidates"][0];
        if cand["streak"].as_u64().unwrap() >= start + 2 && cand["last_epoch"].as_u64() >= Some(e0 + 2) {
            break;
        }
        assert!(Instant::now() < end, "no beacons: {c}");
        std::thread::sleep(Duration::from_millis(500));
    }

    // The new diagnostics are reads. Keep an already-finalized root, poll the
    // candidate view, then replay from genesis in a fresh follower directory.
    let checkpoint = net.height(0);
    let root = block(&net, 0, checkpoint)["state_root"].clone();
    assert!(!root.is_null());
    for _ in 0..8 { mine(&net); }
    let port = Port::reserve().expect("reserve test port");
    let data = net.dir.join("candidate-replay");
    let log_path = net.dir.join("candidate-replay.log");
    let args = vec![
        "follow".into(), "--from-rpc".into(), net.url(0),
        "--data".into(), data.to_str().unwrap().into(),
        "--rpc-port".into(), port.port().to_string(), epoch[0].clone(), epoch[1].clone(),
    ];
    net.procs.push(Some(spawn_logged(&log_path, &args)));
    net.rpc.push(port);
    net.logs.push(log_path);
    let replay = net.rpc.len() - 1;
    net.wait_height(replay, checkpoint + 2, 120);
    assert!(!block(&net, replay, 1).is_null(), "the fresh follower replayed history instead of snapshot-jumping");
    assert_eq!(block(&net, replay, checkpoint)["state_root"], root, "candidate reads do not change the state root on devnet replay");
    for h in [checkpoint, checkpoint + 1, checkpoint + 2] { assert_agree(&net, &[0, 1, 2, 3, replay], h); }
    eprintln!("candidate observability replay: height={checkpoint} unchanged state_root={root}");
}

/// Open voting nodes: nobody runs a ceremony by hand. Four Macs run `aether
/// run` as the genesis voting set, four more as candidates. Once candidates are
/// registered and alive, the chain draws a new voting set with the committee's
/// seed (the seed-drawn candidate takes a seat); old and new
/// members reshare the key in the background while blocks keep coming, the
/// running committee signs the handoff, and at the switch height a candidate
/// takes a seat (one per epoch at this size) under the same identity. It had no
/// validator history: it starts from the block it verified as a follower.
#[test]
fn open_voting_nodes_take_over_the_chain_by_themselves() {
    let _serial = serial();
    let dir = std::env::temp_dir().join(format!("aether-devnet-test-{}-open", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let d = |name: &str| dir.join(name).to_str().unwrap().to_string();
    let genesis_set = ["g1", "g2", "g3", "g4"];
    let candidates = ["c1", "c2", "c3", "c4"];
    for g in genesis_set {
        run_ok(&["keygen", "--data", &d(g)]);
    }
    // A network.json genesis funds only the faucet: g1 holds its key, and the
    // senders below get test tokens through it. It must also name the dev
    // registrar: without one the registry is not predeployed, registrations
    // call empty code (no candidate ever exists) and the draw machinery runs
    // at epoch_blocks 1 with an empty pool — no handoff, ever.
    let faucet = run_ok(&["faucet-key", "--data", &d("g1")]);
    let faucet_addr = faucet.split_whitespace().nth(2).unwrap().to_string();
    let mut args = vec!["network".to_string(), "--faucet".to_string(), faucet_addr.clone(), "--dev-registrar".into(), "--epoch-blocks".into(), "40".into(), "--min-streak".into(), "0".into(), "--draw-epochs".into(), "1".into()];
    args.extend(genesis_set.iter().map(|g| format!("{}/validator.pub.json", d(g))));
    std::fs::write(d("A.json"), run_ok(&args.iter().map(String::as_str).collect::<Vec<_>>())).unwrap();
    let ports: Vec<Port> = (0..4).map(|_| Port::reserve().expect("reserve test port")).collect();
    let dkg: Vec<TestChild> = (0..4)
        .map(|k| {
            spawn_quiet(&[
                "dkg".into(),
                "--network".into(),
                d("A.json"),
                "--port".into(),
                ports[k].to_string(),
                "--data".into(),
                d(genesis_set[k]),
                "--peers".into(),
                tcp_peers(&ports, k),
                "--offline".into(),
            ])
        })
        .collect();
    for c in dkg {
        let out = c.wait_with_output().unwrap();
        assert!(out.status.success(), "dkg failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    std::fs::copy(format!("{}/network.json", d("g1")), d("A-final.json")).unwrap();
    let a_final = serde_json::from_slice::<Value>(&std::fs::read(d("A-final.json")).unwrap()).unwrap();
    assert_eq!(a_final["epoch_blocks"], json!(40), "the ceremony keeps the genesis epoch length");
    let identity = a_final["identity"].as_str().unwrap().to_string();
    // The ceremony's record over the final file; every Mac's `aether run`
    // binds to it (the supervisor hands the same record to the node it
    // starts, and each voting Mac binds before it votes).
    let record = coordinator_check(&d("A-final.json"));

    // Eight Macs, each running only `aether run`.
    let names: Vec<&str> = genesis_set.iter().chain(candidates.iter()).copied().collect();
    let p2p: Vec<Port> = (0..8).map(|_| Port::reserve().expect("reserve test port")).collect();
    let rpc: Vec<Port> = (0..8).map(|_| Port::reserve().expect("reserve test port")).collect();
    let reshare: Vec<Port> = (0..8).map(|_| Port::reserve().expect("reserve test port")).collect();
    let mut net = Net::prepared(dir.clone(), p2p.clone(), rpc.clone(), vec![vec![]; 8]);
    net.logs = names.iter().map(|name| dir.join(format!("{name}.log"))).collect();
    for (k, name) in names.iter().enumerate() {
        let others: Vec<String> = (0..8).filter(|j| *j != k).map(|j| format!("http://127.0.0.1:{}", rpc[j])).collect();
        let mut a: Vec<String> = vec![
            "run".into(),
            "--data".into(),
            d(name),
            "--network".into(),
            d("A-final.json"),
            "--ceremony".into(),
            record.clone(),
            "--port".into(),
            p2p[k].to_string(),
            "--rpc-port".into(),
            rpc[k].to_string(),
            "--reshare-port".into(),
            reshare[k].to_string(),
            "--dev-peer-dir".into(),
            d("peers"),
            "--node-arg=--block-time-ms=500".into(),
            format!("--follow-arg=--from-rpc={}", others.join(",")),
            "--reshare-timeout".into(),
            "120".into(),
        ];
        if k == 0 {
            // The dev registrar registers any device without Apple, and g1's
            // node also answers the faucet (the two may share a test chain;
            // only the 7780 testnet refuses this).
            a.push("--node-arg=--dev-registrar".into());
            a.push(format!("--node-arg=--faucet-key={}/faucet.key", d("g1")));
        }
        let mut cmd = Command::new(BIN);
        cmd.args(&a).env("RUST_LOG", "info,commonware=warn")
            .env("AETHER_TEST_INTERNAL_KEY_DIR", d(name));
        let child = TestChild::spawn(cmd, dir.join(format!("{name}.log"))).expect("spawn run");
        net.procs[k] = Some(child);
    }
    net.wait_height(0, 3, 90);
    // Nobody is funded at genesis: the faucet on g1 funds the sender and the
    // four operators-to-be (this legacy chain has no free lane, so their
    // contract registrations and tips come out of it).
    for dev in 1..=5u8 {
        faucet_grant(&net, 0, &dev_address(dev));
    }
    assert_eq!(net.rpc(0, "aether_status", json!([])).unwrap()["free_registration"], false);
    let aa = "0x00000000000000000000000000000000000000aa";
    net.cli(&["send", "--rpc", &net.url(0), "--from-dev", "1", "--to", aa, "--value", "11", "--wait"]);

    // Four owners register their Macs (four operators: no one holds a third).
    for (i, c) in candidates.iter().enumerate() {
        let out = net.cli(&["candidate-register", "--data", &d(c), "--registrar-rpc", &net.url(0), "--rpc", &net.url(0), "--from-dev", &(i + 2).to_string()]);
        assert!(out.contains("success=true"), "{out}");
    }
    let keys: std::collections::BTreeSet<String> = candidates
        .iter()
        .map(|c| serde_json::from_slice::<Value>(&std::fs::read(format!("{}/validator.pub.json", d(c))).unwrap()).unwrap()["key"].as_str().unwrap().to_string())
        .collect();

    // No one acts from here. The chain never stops: heights keep rising throughout.
    let g1 = keys_of(&d("g1"));
    let end = Instant::now() + Duration::from_secs(300);
    let mut last_height = 0;
    let mut stalled_since = Instant::now();
    let handoff = loop {
        let h = net.height(1);
        if h > last_height {
            last_height = h;
            stalled_since = Instant::now();
        }
        assert!(stalled_since.elapsed() < Duration::from_secs(30), "the chain stopped at {h} (see {}/*.log)", dir.display());
        if let Some(v) = net.rpc(1, "aether_handoff", json!([])).filter(|v| !v.is_null()) {
            break v;
        }
        assert!(Instant::now() < end, "no handoff (see {}/*.log)", dir.display());
        std::thread::sleep(Duration::from_millis(500));
    };
    let members: Vec<String> = handoff["members"].as_array().unwrap().iter().map(|m| m["key"].as_str().unwrap().to_string()).collect();
    assert_eq!(members.len(), 4);
    assert!(!members.contains(&g1), "a genesis member that never registered leaves first: {members:?}");
    let joined: Vec<&String> = members.iter().filter(|k| keys.contains(*k)).collect();
    assert_eq!(joined.len(), 1, "one seat per draw at four seats: {members:?}");
    // The seed-drawn candidate: its Mac is net index 4 + its position among c1..c4.
    let j = 4 + candidates.iter().position(|c| &keys_of(&d(c)) == joined[0]).unwrap();
    let switch = handoff["switch"].as_u64().unwrap();
    let round = handoff["round"].as_u64().unwrap();

    // Strict reshares derive their round from the draw, so the first handoff
    // can be round 3. Check the candidate's installed files too: a follower's
    // aether_network RPC proxies upstream and cannot prove that it is voting.
    let end = Instant::now() + Duration::from_secs(120);
    let candidate = d(candidates[j - 4]);
    let installed = || {
        let network = std::fs::read(format!("{candidate}/network.json")).ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        let threshold = std::fs::read(format!("{candidate}/threshold.json")).ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        network.as_ref().and_then(|v| v["round"].as_u64()) == Some(round)
            && network.as_ref().map(|v| &v["output"]) == Some(&handoff["output"])
            && threshold.as_ref().and_then(|v| v["round"].as_u64()) == Some(round)
            && threshold.as_ref().map(|v| &v["output"]) == Some(&handoff["output"])
            && !std::path::Path::new(&candidate).join("no-vote").exists()
    };
    while !installed() || net.rpc(j, "aether_network", json!([])).and_then(|v| v["round"].as_u64()) != Some(round) {
        assert!(Instant::now() < end, "the drawn candidate did not start voting (see {}/*.log)", dir.display());
        std::thread::sleep(Duration::from_millis(500));
    }
    let target = switch + 10;
    for k in [1, 2, 3, j, 0] {
        net.wait_height(k, target, 120);
    }
    assert_agree(&net, &[1, 2, 3, j, 0], target);
    // History from before the handoff verifies under the same identity on the new voting node.
    let bal = net.cli(&["balance", aa, "--rpc", &net.url(j), "--identity", &identity]);
    assert!(bal.contains("balance   11 wei") && bal.contains("verified  ✓"), "{bal}");
}

/// Four genesis voters and three unseated reserves, all supervised by `run`.
/// The mainnet variant uses the free registration lane and registry-v3 uptime.
fn founder_reserve_net(tag: &str, epoch_blocks: u64, mainnet: bool) -> Net {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp")
        .join(format!("aether-devnet-test-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let d = |name: &str| dir.join(name).to_str().unwrap().to_string();
    let genesis_set = ["g1", "g2", "g3", "g4"];
    let reserve = ["r1", "r2", "r3"];
    for k in genesis_set.iter().chain(reserve.iter()) {
        run_ok(&["keygen", "--data", &d(k)]);
    }
    let founder = "0x00000000000000000000000000000000000000f0";
    let mut args = vec!["network".into(), "--epoch-blocks".into(), epoch_blocks.to_string(),
        "--min-streak".into(), "0".into(), "--draw-epochs".into(), "1".into(),
        "--node-rewards".into(), "--reserve-operator".into(), founder.into()];
    if mainnet {
        args.extend(["--dev-registrar", "--history", "2", "--protocol", "4"].map(str::to_string));
    } else {
        let reg = run_ok(&["registrar-key", "--data", &d("reg")]);
        args.extend(["--registrar".to_string(), reg.split_whitespace().nth(2).unwrap().to_string()]);
    }
    for r in reserve {
        args.extend(["--reserve".to_string(), format!("{}/validator.pub.json", d(r))]);
    }
    args.extend(genesis_set.iter().map(|g| format!("{}/validator.pub.json", d(g))));
    std::fs::write(d("A.json"), run_ok(&args.iter().map(String::as_str).collect::<Vec<_>>())).unwrap();
    let ports: Vec<Port> = (0..4).map(|_| Port::reserve().expect("reserve test port")).collect();
    let dkg: Vec<TestChild> = (0..4)
        .map(|k| {
            spawn_quiet(&[
                "dkg".into(),
                "--network".into(),
                d("A.json"),
                "--port".into(),
                ports[k].to_string(),
                "--data".into(),
                d(genesis_set[k]),
                "--peers".into(),
                tcp_peers(&ports, k),
                "--offline".into(),
            ])
        })
        .collect();
    for c in dkg {
        let out = c.wait_with_output().unwrap();
        assert!(out.status.success(), "dkg failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    std::fs::copy(format!("{}/network.json", d("g1")), d("A-final.json")).unwrap();
    let a_final = serde_json::from_slice::<Value>(&std::fs::read(d("A-final.json")).unwrap()).unwrap();
    assert_eq!(a_final["reserve"]["validators"].as_array().map(Vec::len), Some(3), "the ceremony keeps the reserve keys");
    // The ceremony's record over the final file; every Mac's `aether run`
    // binds to it before voting or following.
    let record = coordinator_check(&d("A-final.json"));

    // Seven processes on "one Mac", each only `aether run` (the reserve keys as in scripts/reserve-keys.sh).
    let names: Vec<&str> = genesis_set.iter().chain(reserve.iter()).copied().collect();
    let n = names.len();
    let p2p: Vec<Port> = (0..n).map(|_| Port::reserve().expect("reserve test port")).collect();
    let rpc: Vec<Port> = (0..n).map(|_| Port::reserve().expect("reserve test port")).collect();
    let resh: Vec<Port> = (0..n).map(|_| Port::reserve().expect("reserve test port")).collect();
    let mut net = Net::prepared(dir.clone(), p2p.clone(), rpc.clone(), vec![vec![]; n]);
    net.logs = names.iter().map(|name| dir.join(format!("{name}.log"))).collect();
    for (k, name) in names.iter().enumerate() {
        let others: Vec<String> = (0..n).filter(|j| *j != k).map(|j| format!("http://127.0.0.1:{}", rpc[j])).collect();
        let mut a: Vec<String> = vec![
            "run".into(),
            "--data".into(),
            d(name),
            "--network".into(),
            d("A-final.json"),
            "--ceremony".into(),
            record.clone(),
            "--port".into(),
            p2p[k].to_string(),
            "--rpc-port".into(),
            rpc[k].to_string(),
            "--reshare-port".into(),
            resh[k].to_string(),
            "--dev-peer-dir".into(),
            d("peers"),
            "--node-arg=--block-time-ms=1000".into(),
            format!("--follow-arg=--from-rpc={}", others.join(",")),
            "--reshare-timeout".into(),
            "120".into(),
        ];
        if mainnet && k == 0 {
            a.push("--node-arg=--dev-registrar".into());
        }
        let mut cmd = Command::new(BIN);
        cmd.args(&a).env("RUST_LOG", "info,commonware=warn")
            .env("AETHER_TEST_INTERNAL_KEY_DIR", d(name)).env("TMPDIR", &dir);
        let child = TestChild::spawn(cmd, dir.join(format!("{name}.log"))).expect("spawn run");
        net.procs[k] = Some(child);
    }
    net.wait_height(0, 3, 90);
    net
}

/// Founder reserve keys run as `aether run` like any Mac (docs/ops/reserve-keys.md):
/// three keys that are not in the genesis voting set and never register follow
/// the chain. With the genesis set holding four seats, not one of them joins —
/// reserve keys only fill a committee that is short of four seats, because
/// growing four to seven would put three seats on the founder's one Mac and let
/// its outage stall the quorum. They stay followers with no share and no vote,
/// and the chain never stops.
#[test]
fn founder_reserve_keys_stay_followers_over_a_full_committee() {
    let _serial = serial();
    let net = founder_reserve_net("reserve", 40, false);
    let dir = &net.dir;
    let d = |name: &str| dir.join(name).to_str().unwrap().to_string();
    let reserve = ["r1", "r2", "r3"];
    let n = net.procs.len();
    let reserve_keys: Vec<String> = reserve.iter().map(|r| keys_of(&d(r))).collect();
    // The reserve keys follow (no share, not voting), and stay that way.

    // Nobody registers, so the committee never falls below its four seats: no
    // handoff is ever proposed, and the chain keeps finalizing on its own.
    let target = 4 * 40;
    net.wait_height(0, target, 300);
    assert!(
        net.rpc(0, "aether_handoff", json!([])).filter(|v| !v.is_null()).is_none(),
        "a full committee seats no reserve key (see {}/*.log)",
        dir.display()
    );
    for r in reserve {
        assert!(!dir.join(r).join("threshold.json").exists(), "{r} stays a follower with no share");
    }
    // The followers keep up with the validators and agree on every block.
    for k in 0..n {
        net.wait_height(k, target, 120);
    }
    assert_agree(&net, &(0..n).collect::<Vec<_>>(), target);
    // The four genesis members propose; the reserve keys never do.
    let blocks = net.rpc(0, "aether_recentBlocks", json!([20])).unwrap();
    let proposers: std::collections::BTreeSet<String> = blocks
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["height"].as_u64().unwrap() >= 4 * 40)
        .map(|b| b["proposer"].as_str().unwrap().to_string())
        .collect();
    assert!(!proposers.is_empty());
    assert!(reserve_keys.iter().all(|k| !proposers.contains(k)), "a reserve key proposed: {proposers:?}");
}

/// A consistent RPC snapshot lets this black-box test read the same public
/// eligibility and silence records as rotation, without duplicating slot tags.
fn devnet_state(net: &Net, node: usize) -> Option<(u64, aether_execution::WorldState)> {
    let manifest = net.rpc(node, "aether_snapshot", json!([]))?;
    let height = manifest["height"].as_u64()?;
    let size = manifest["size"].as_u64()?;
    let chunk = manifest["chunk"].as_u64()?;
    let mut bytes = Vec::new();
    for index in 0..size.div_ceil(chunk) {
        let part = net.rpc(node, "aether_snapshotChunk", json!([height, index]))?;
        bytes.extend(hex::decode(part["data"].as_str()?).ok()?);
    }
    assert_eq!(bytes.len() as u64, size, "snapshot length");
    assert_eq!(blake3::hash(&bytes).to_hex().to_string(), manifest["blake3"].as_str()?, "snapshot digest");
    let snapshot = aether_node::snapshot::Snapshot::from_bytes(&bytes).expect("decode snapshot");
    assert_eq!(snapshot.summary.height, height);
    let root = snapshot.summary.state_root;
    let state = aether_execution::WorldState::from_parts(snapshot.entries, snapshot.codes.into_iter().collect());
    assert_eq!(state.root(), root, "snapshot state root");
    Some((height, state))
}

/// Mainnet standby + auto-join, with real supervised nodes and a real reshare.
/// About 20–25 minutes at the mainnet one-second block floor; run separately:
/// cargo test -p aether-node --test devnet founder_reserve_fills_one_silent_seat_without_halting -- --ignored --nocapture
#[test]
#[ignore = "seven supervised processes, registry-v3 warmup and two silent epochs"]
fn founder_reserve_fills_one_silent_seat_without_halting() {
    let _serial = serial();
    // The 64-block handoff delay alone exceeds the old fixture's 40-block
    // epoch. This leaves room for the offline dealer's quorum-log wait too.
    const E: u64 = 192;
    let mut net = founder_reserve_net("reserve-autojoin", E, true);
    let dir = net.dir.clone();
    let d = |name: &str| dir.join(name).to_str().unwrap().to_string();
    let operators = ["g1", "g2", "g3", "g4"];
    let reserves = ["r1", "r2", "r3"];
    let operator_keys: Vec<String> = operators.iter().map(|name| keys_of(&d(name))).collect();
    let reserve_keys: Vec<String> = reserves.iter().map(|name| keys_of(&d(name))).collect();
    let expected: std::collections::BTreeSet<String> = operator_keys.iter().cloned().collect();
    let initial: Value = serde_json::from_slice(&std::fs::read(d("A-final.json")).unwrap()).unwrap();
    assert_eq!(net.rpc(0, "aether_status", json!([])).unwrap()["free_registration"], true);
    for (i, name) in operators.iter().enumerate() {
        net.cli(&["candidate-register", "--data", &d(name), "--registrar-rpc", &net.url(0),
            "--rpc", &net.url(0), "--from-dev", &(i + 1).to_string()]);
    }
    eprintln!("reserve devnet: four operators registered; waiting for registry-v3 qualification ({E}-block epochs)");

    let mut last_height = net.height(0);
    let mut progressed = Instant::now();
    let mut observe = |net: &Net| {
        let h = net.height(0);
        if h > last_height {
            last_height = h;
            progressed = Instant::now();
        }
        assert!(progressed.elapsed() < Duration::from_secs(30),
            "finalization stalled at {last_height} (see {}/*.log){}", dir.display(), net.log_tail(0));
        h
    };
    // Registry v3 needs three good full epochs, not merely four registrations.
    // Read at a boundary, before this epoch's answers replace last_epoch.
    let end = Instant::now() + Duration::from_secs(8 * E + 240);
    let mut checked_epoch = 0;
    let (ready_height, state) = loop {
        let h = observe(&net);
        let epoch = h / E;
        if h >= 4 * E && epoch != checked_epoch && h % E <= 1 {
            if let Some((at, state)) = devnet_state(&net, 0) {
                checked_epoch = epoch;
                let reserve = aether_node::chain::Reserve::of(&state).expect("registered reserves");
                let pool = aether_node::rotation::eligible(&state, at / E, 0);
                let ops = aether_node::rotation::operators(&state);
                let keys: std::collections::BTreeSet<String> = pool.iter().map(|(k, _)| k.clone()).collect();
                if keys == expected && aether_node::rotation::independent(&pool, |k| ops.get(k).cloned(), &reserve) == 4 {
                    break (at, state);
                }
            }
        }
        assert!(Instant::now() < end, "four independent operators never qualified (see {}/*.log)", dir.display());
        std::thread::sleep(Duration::from_millis(200));
    };
    let candidates = aether_execution::registry::candidates(&state);
    assert_eq!(candidates.len(), 4);
    assert_eq!(candidates.iter().map(|c| c.operator).collect::<std::collections::BTreeSet<_>>().len(), 4);
    let reserve = aether_node::chain::Reserve::of(&state).unwrap();
    assert_eq!(reserve.members.len(), 3, "four operators keep the reserve set registered");
    assert_eq!(aether_rewards::committee(&state).iter().map(|(k, _)| k.clone()).collect::<std::collections::BTreeSet<_>>(), expected);
    assert!(net.rpc(0, "aether_handoff", json!([])).is_some_and(|v| v.is_null()), "no reserve joins a full four-seat committee");
    for name in reserves {
        assert!(!dir.join(name).join("threshold.json").exists(), "{name} waits without a voting share");
    }
    eprintln!("reserve devnet: four independent operators qualified at height {ready_height}; all reserves are followers");
    assert_agree(&net, &(0..7).collect::<Vec<_>>(), ready_height);
    let dead_key = &operator_keys[3];
    let dead_index = candidates.iter().find(|c| hex::encode(c.validator_key) == *dead_key).unwrap().index;

    // No departure announcement, hand-made reshare or intervention: kill g4's
    // supervisor; its leased node child exits with it. The other three still
    // have the old 3-of-4 quorum throughout the automatic replacement.
    net.kill(3);
    let stopped = Instant::now() + Duration::from_secs(15);
    while net.rpc(3, "aether_status", json!([])).is_some() {
        observe(&net);
        assert!(Instant::now() < stopped, "the stopped operator's node child is still running");
        std::thread::sleep(Duration::from_millis(100));
    }
    let down_epoch = net.height(0) / E;
    eprintln!("reserve devnet: g4 stopped in epoch {down_epoch}; waiting for two confirmed silent epochs");
    let end = Instant::now() + Duration::from_secs(4 * E + 240);
    let mut checked_epoch = down_epoch;
    let mut confirmed_boundary = None;
    let handoff = loop {
        let h = observe(&net);
        if confirmed_boundary.is_none() && h / E >= down_epoch + 2 && h / E != checked_epoch {
            if let Some((at, state)) = devnet_state(&net, 0) {
                if at / E == h / E {
                    checked_epoch = at / E;
                    if let Some((epoch, last, prev)) = aether_rewards::beacons::recent(&state, dead_index) {
                        if epoch + 1 == at / E && last < aether_node::rotation::SILENT_BELOW && prev < aether_node::rotation::SILENT_BELOW {
                            confirmed_boundary = Some((epoch + 1) * E);
                            eprintln!("reserve devnet: two silent epochs confirmed at boundary {} (answers {prev}, {last})", (epoch + 1) * E);
                        }
                    }
                }
            }
        }
        if let (Some(boundary), Some(handoff)) = (confirmed_boundary,
            net.rpc(0, "aether_handoff", json!([])).filter(|v| !v.is_null())) {
            assert!(handoff["at"].as_u64().unwrap() >= boundary, "no replacement before two confirmed silent epochs");
            assert!(handoff["switch"].as_u64().unwrap() <= boundary + E, "the reserve takes the missing seat within one epoch");
            break handoff;
        }
        if let Some(boundary) = confirmed_boundary {
            assert!(h <= boundary + E, "no automatic reserve handoff within one epoch (see {}/*.log)", dir.display());
        }
        assert!(Instant::now() < end, "no automatic reserve handoff (see {}/*.log)", dir.display());
        std::thread::sleep(Duration::from_millis(250));
    };
    let members: std::collections::BTreeSet<String> = handoff["members"].as_array().unwrap().iter()
        .map(|m| m["key"].as_str().unwrap().to_string()).collect();
    assert_eq!(members.len(), 4, "fill only the missing seat, never grow four to seven");
    assert!(!members.contains(dead_key), "the silent operator's seat is replaced");
    assert!(operator_keys[..3].iter().all(|k| members.contains(k)), "all surviving operators retain their seats");
    let joined: Vec<usize> = reserve_keys.iter().enumerate().filter_map(|(i, k)| members.contains(k).then_some(i)).collect();
    assert_eq!(joined.len(), 1, "exactly one standby reserve joins");
    let j = 4 + joined[0];
    let round = handoff["round"].as_u64().unwrap();
    let switch = handoff["switch"].as_u64().unwrap();
    let target = switch + 12;
    eprintln!("reserve devnet: one reserve joins in round {round} at switch {switch}; checking local installation and height {target}");
    let installed = || {
        let read = |file: &str| std::fs::read(dir.join(reserves[j - 4]).join(file)).ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let (Some(network), Some(threshold)) = (read("network.json"), read("threshold.json")) else { return false };
        network["round"].as_u64() == Some(round) && threshold["round"].as_u64() == Some(round)
            && network["output"] == handoff["output"] && threshold["output"] == handoff["output"]
            && network["identity"] == initial["identity"]
    };
    let end = Instant::now() + Duration::from_secs(E + 180);
    while !installed() || [0, 1, 2, 4, 5, 6].iter().any(|&i| net.height(i) < target) {
        observe(&net);
        assert!(Instant::now() < end, "the automatic reserve did not install and finalize (see {}/*.log)", dir.display());
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_agree(&net, &[0, 1, 2, 4, 5, 6], target);
    for (i, name) in reserves.iter().enumerate() {
        assert_eq!(dir.join(name).join("threshold.json").exists(), i == joined[0], "only the seated reserve receives a share");
    }
}

fn keys_of(dir: &str) -> String {
    serde_json::from_slice::<Value>(&std::fs::read(format!("{dir}/validator.pub.json")).unwrap()).unwrap()["key"].as_str().unwrap().to_string()
}

/// Regenerates `crates/light/tests/fixtures/devnet4.json` (run when the state
/// hash or block format changes):
///   cargo test -p aether-node --test devnet regenerate_light_fixture -- --ignored
/// A 4-validator devnet pays 4242 wei to 0x…b0b00; the fixture keeps its
/// account proof, another account's proof, and the two certified blocks that
/// follow (the first commits to the proven state root).
#[test]
#[ignore]
fn regenerate_light_fixture() {
    let _serial = serial();
    let net = Net::start(4);
    for i in 0..4 {
        net.wait_height(i, 3, 60);
    }
    let bob = "0x00000000000000000000000000000000000b0b00";
    let out = net.cli(&["send", "--rpc", &net.url(0), "--from-dev", "2", "--to", bob, "--value", "4242", "--wait"]);
    assert!(out.contains("success=true"), "{out}");
    let other = dev_address(1);
    let end = Instant::now() + Duration::from_secs(120);
    loop {
        assert!(Instant::now() < end, "no height with directly certified successors");
        let (Some(a), Some(b)) = (net.rpc(0, "aether_getAccount", json!([bob])), net.rpc(0, "aether_getAccount", json!([other]))) else { continue };
        let h = a["height"].as_u64().unwrap();
        if b["height"].as_u64() != Some(h) {
            continue;
        }
        net.wait_height(0, h + 3, 60);
        let (Some(anchor), Some(next)) = (net.rpc(0, "aether_getFinalized", json!([h + 1])), net.rpc(0, "aether_getFinalized", json!([h + 2]))) else {
            continue;
        };
        // The fixture uses blocks with their own certificates (no links).
        let direct = |v: &Value| v["links"].as_array().is_none_or(|l| l.is_empty());
        if !direct(&anchor) || !direct(&next) {
            continue;
        }
        let fixture = json!({
            "validators": 4,
            "address": bob,
            "balance": a["balance"],
            "height": h,
            "state_root": a["state_root"],
            "proof": a["proof"],
            "other_address": other,
            "other_proof": b["proof"],
            "anchor_block": anchor["block"],
            "anchor_finalization": anchor["finalization"],
            "next_block": next["block"],
            "next_finalization": next["finalization"],
        });
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../light/tests/fixtures/devnet4.json");
        std::fs::write(&path, serde_json::to_string_pretty(&fixture).unwrap() + "\n").unwrap();
        eprintln!("wrote {} (height {h})", path.display());
        return;
    }
}

/// Checkpoint sync: a Mac joining late takes a certified snapshot (checked
/// against the next certified block) and follows from there, without
/// replaying history; balances still verify against certified roots.
#[test]
fn a_late_mac_starts_from_a_certified_snapshot() {
    let _serial = serial();
    let mut net = Net::start(4);
    for i in 0..4 {
        net.wait_height(i, 3, 60);
    }
    let bob = "0x00000000000000000000000000000000000c0c00";
    let out = net.cli(&["send", "--rpc", &net.url(0), "--from-dev", "2", "--to", bob, "--value", "321", "--wait"]);
    assert!(out.contains("success=true"), "{out}");
    let joined_at = net.height(0);

    let port = Port::reserve().expect("reserve test port");
    let data = net.dir.join("late");
    let log = net.dir.join("late.log");
    let from = format!("{},{}", net.url(1), net.url(2));
    let args: Vec<String> = vec![
        "follow".into(),
        "--checkpoint".into(),
        "--from-rpc".into(),
        from,
        "--data".into(),
        data.to_str().unwrap().into(),
        "--rpc-port".into(),
        port.to_string(),
    ];
    net.procs.push(Some(spawn_logged(log, &args)));
    net.rpc.push(port);
    net.logs.push(net.dir.join("late.log"));
    let f = net.rpc.len() - 1;
    net.wait_height(f, joined_at + 5, 60);
    assert_agree(&net, &[0, f], joined_at + 5);
    // It did not replay: early blocks are not on this Mac.
    assert!(net.rpc(f, "aether_getBlock", json!([1])).is_none_or(|b| b.is_null()), "the late Mac replayed history");
    let bal = net.cli(&["balance", bob, "--rpc", &net.url(f)]);
    assert!(bal.contains("balance   321 wei") && bal.contains("verified  ✓"), "{bal}");
}

/// The vote journal keeps one section file per view, pruned only below the
/// last finalization minus view retention (~20 views). A healthy chain
/// therefore holds a couple dozen files; a stalled one piles up one per burned
/// view — 190 on 2026-09-29 — and the journal opens every one at startup,
/// which is what exhausted launchd's 256-file limit and crash-looped the four
/// validators. Run a chain well past view retention and check every node's
/// journal stays small (unpruned it would hold one file per view: 80+ here).
#[test]
fn the_vote_journal_keeps_a_bounded_number_of_section_files() {
    let _serial = serial();
    let net = Net::start_with("journal", vec![vec![]; 4]);
    net.wait_height(0, 80, 180);
    let h = net.height(0);
    for i in 0..4 {
        net.wait_height(i, h, 30);
    }
    for i in 0..4 {
        let dir = net.dir.join((i + 1).to_string()).join("aether-consensus");
        let sections = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("node {i}: no vote journal at {}: {e}", dir.display()))
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .count();
        assert!(sections > 1, "node {i}: no vote journal sections in {}", dir.display());
        assert!(
            sections <= 40,
            "node {i} holds {sections} vote journal sections at height {h}: pruned only below the last finalization?"
        );
    }
}
