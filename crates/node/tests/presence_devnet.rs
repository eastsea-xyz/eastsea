//! Six real processes expose only frozen, thresholded aggregate observations
//! across a local-only overlay. Distinct-node census claims are intentionally
//! absent: anonymous forwarded cohorts cannot be deduplicated or added.

use aether_node::presence::{MIN_BUCKET_SIZE, TIME_BUCKET_SECONDS};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::net::{TcpListener, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_aether");
const VALIDATORS: usize = 4;
const NODES: usize = 6;

struct LocalDevnet {
    dir: PathBuf,
    rpc: Vec<u16>,
    logs: Vec<PathBuf>,
    children: Vec<Child>,
    client: reqwest::blocking::Client,
}

impl LocalDevnet {
    fn start() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("resolve worktree root");
        let dir = root
            .join("tmp")
            .join(format!("aether-presence-devnet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create isolated presence devnet directory");
        // Hold reservations until each child starts rather than releasing a
        // port and asking the OS for another one that may return the same port.
        let mut consensus: Vec<_> = (0..VALIDATORS)
            .map(|_| Some(TcpListener::bind("127.0.0.1:0").unwrap()))
            .collect();
        let mut http: Vec<_> = (0..NODES)
            .map(|_| Some(TcpListener::bind("127.0.0.1:0").unwrap()))
            .collect();
        let mut quic: Vec<_> = (0..NODES)
            .map(|_| Some(UdpSocket::bind("127.0.0.1:0").unwrap()))
            .collect();
        let consensus_ports: Vec<_> = consensus
            .iter()
            .map(|p| p.as_ref().unwrap().local_addr().unwrap().port())
            .collect();
        let rpc: Vec<_> = http
            .iter()
            .map(|p| p.as_ref().unwrap().local_addr().unwrap().port())
            .collect();
        let presence_ports: Vec<_> = quic
            .iter()
            .map(|p| p.as_ref().unwrap().local_addr().unwrap().port())
            .collect();
        let logs = (0..NODES)
            .map(|i| dir.join(format!("node{}.log", i + 1)))
            .collect();
        let mut net = Self {
            dir,
            rpc,
            logs,
            children: Vec::new(),
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
        };
        let from_rpc = (0..VALIDATORS)
            .map(|i| net.url(i))
            .collect::<Vec<_>>()
            .join(",");
        for i in 0..NODES {
            let data = net.dir.join(if i < VALIDATORS {
                format!("validator{}", i + 1)
            } else {
                format!("follower{}", i + 1 - VALIDATORS)
            });
            std::fs::create_dir_all(&data).expect("create node data directory");
            let mut cmd = Command::new(BIN);
            if i < VALIDATORS {
                let peers = (0..VALIDATORS)
                    .filter(|j| *j != i)
                    .map(|j| format!("{}@127.0.0.1:{}", j + 1, consensus_ports[j]))
                    .collect::<Vec<_>>()
                    .join(",");
                cmd.args([
                    "node",
                    "--index",
                    &(i + 1).to_string(),
                    "--validators",
                    &VALIDATORS.to_string(),
                ])
                .args([
                    "--port",
                    &consensus_ports[i].to_string(),
                    "--block-time-ms",
                    "500",
                    "--peers",
                    &peers,
                    "--offline",
                ]);
            } else {
                // wallet_node_key reads a raw 32-byte secret. Each follower
                // uses a distinct, public devnet key; no real wallet is read.
                let key_path = data.join("wallet-node.key");
                std::fs::write(
                    &key_path,
                    aether_net::devnet_node_secret((i + 1) as u64).to_bytes(),
                )
                .expect("write devnet follower key");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
                        .unwrap();
                }
                cmd.args([
                    "follow",
                    "--validators",
                    &VALIDATORS.to_string(),
                    "--from-rpc",
                    &from_rpc,
                ])
                .arg("--node-key")
                .arg(&key_path);
            }
            cmd.arg("--data")
                .arg(&data)
                .args(["--rpc-port", &net.rpc[i].to_string(), "--min-free-disk=0"])
                .arg(format!(
                    "--dev-presence-bind=127.0.0.1:{}",
                    presence_ports[i]
                ));
            let presence_peers: Vec<_> = if i < VALIDATORS {
                (0..VALIDATORS).filter(|j| *j != i).collect()
            } else {
                vec![if i == VALIDATORS { 0 } else { VALIDATORS - 1 }]
            };
            for j in presence_peers {
                cmd.arg(format!(
                    "--dev-presence-peer={}@127.0.0.1:{}",
                    aether_net::devnet_node_id((j + 1) as u64),
                    presence_ports[j]
                ));
            }
            drop(http[i].take());
            drop(quic[i].take());
            if i < VALIDATORS {
                drop(consensus[i].take());
            }
            let log = std::fs::File::create(&net.logs[i]).expect("create node log");
            cmd.env("RUST_LOG", "warn")
                .stdout(log.try_clone().expect("clone node log"))
                .stderr(log);
            net.children
                .push(cmd.spawn().expect("spawn isolated devnet process"));
        }
        net
    }

    fn url(&self, i: usize) -> String {
        format!("http://127.0.0.1:{}", self.rpc[i])
    }

    fn rpc(&self, i: usize, method: &str, deadline: Instant) -> Option<Value> {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": [] });
        let answer: Value = self
            .client
            .post(self.url(i))
            .json(&body)
            .timeout(Duration::from_secs(2).min(remaining))
            .send()
            .ok()?
            .json()
            .ok()?;
        answer.get("result").cloned()
    }

    fn log_tails(&self) -> String {
        self.logs
            .iter()
            .map(|path| {
                let text = std::fs::read_to_string(path).unwrap_or_default();
                let lines: Vec<_> = text.lines().collect();
                format!(
                    "\n{}:\n{}",
                    path.display(),
                    lines[lines.len().saturating_sub(16)..].join("\n")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Drop for LocalDevnet {
    fn drop(&mut self) {
        // Always reap every child, including during a test assertion panic.
        for child in &mut self.children {
            let _ = child.kill();
        }
        for child in &mut self.children {
            let _ = child.wait();
        }
        if std::thread::panicking() {
            eprintln!("kept presence devnet logs in {}", self.dir.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

fn assert_presence_shape(value: &Value) {
    assert_eq!(value["schema"], 2);
    assert_eq!(value["available"], true);
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "schema",
            "available",
            "scope",
            "observed_at",
            "ttl_seconds",
            "minimum_bucket_size",
            "total",
            "by_role",
            "by_region",
            "by_version"
        ])
    );
    assert_eq!(value["ttl_seconds"], TIME_BUCKET_SECONDS);
    assert_eq!(value["minimum_bucket_size"], MIN_BUCKET_SIZE);
    assert_eq!(value["scope"], "unverified cohort observation");
    assert!(value["observed_at"]
        .as_u64()
        .is_some_and(|t| t > 0 && t % TIME_BUCKET_SECONDS == 0));
    let total = value["total"].as_u64();
    assert!(
        value["total"].is_null()
            || total.is_some_and(|t| t >= MIN_BUCKET_SIZE as u64 && t <= NODES as u64)
    );
    for field in ["by_role", "by_region", "by_version"] {
        let buckets = value[field].as_object().expect("aggregate partition");
        assert!(buckets
            .values()
            .all(|count| count.as_u64().is_some_and(|n| n >= MIN_BUCKET_SIZE as u64)));
        assert_eq!(
            buckets
                .values()
                .map(|count| count.as_u64().unwrap())
                .sum::<u64>(),
            total.unwrap_or_default(),
            "a total cannot reveal an unreported sub-k residual in {field}"
        );
    }
    let serialized = value.to_string();
    for i in 1..=NODES as u64 {
        assert!(!serialized.contains(&aether_net::devnet_node_id(i).to_string()));
    }
    for field in [
        "nodes",
        "node_id",
        "observer",
        "country",
        "last_seen",
        "timestamp",
        "signature",
        "sender",
        "pings",
    ] {
        assert!(
            !serialized.contains(field),
            "{field} absent from actual RPC JSON"
        );
    }
}

fn assert_peer_shape(value: &Value) {
    for peer in value.as_array().expect("peers array") {
        let fields = peer.as_object().expect("peer object");
        assert_eq!(
            fields.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "node_id",
                "role",
                "connected_since",
                "last_seen",
                "path",
                "version"
            ])
        );
        let node_id = peer["node_id"].as_str().expect("peer node id");
        assert_eq!(node_id.len(), 64);
        assert!(node_id.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(
            peer["role"].is_null()
                || matches!(
                    peer["role"].as_str(),
                    Some("validator" | "candidate" | "follower")
                )
        );
        assert!(peer["connected_since"].as_u64().is_some());
        assert!(peer["last_seen"].as_u64().is_some());
        assert!(matches!(
            peer["path"].as_str(),
            Some("direct" | "relay" | "unknown")
        ));
        assert!(peer["version"].is_null() || peer["version"].is_string());
    }
}

#[test]
fn six_nodes_export_safe_aggregate_json_and_keep_diagnostics_local() {
    let deadline = Instant::now() + Duration::from_secs(120);
    let net = LocalDevnet::start();
    let mut last = vec![None; NODES];
    loop {
        let mut all_ready = true;
        for (i, snapshot) in last.iter_mut().enumerate() {
            *snapshot = net.rpc(i, "aether_presence", deadline);
            all_ready &= snapshot
                .as_ref()
                .is_some_and(|v| v["schema"] == 2 && v["available"] == true);
        }
        if all_ready {
            let peer_deadline = Instant::now() + Duration::from_secs(15);
            for (i, value) in last.iter().enumerate() {
                assert_presence_shape(value.as_ref().unwrap());
                let peers = net
                    .rpc(i, "aether_peers", peer_deadline)
                    .expect("live peers RPC answers");
                assert_peer_shape(&peers);
            }
            return;
        }
        assert!(
            Instant::now() < deadline,
            "presence endpoints did not become ready in 120s: {last:?}{}",
            net.log_tails()
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}
