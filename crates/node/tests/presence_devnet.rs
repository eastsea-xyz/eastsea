//! Six real processes see signed presence across a local-only overlay. The
//! followers attach to different validators, so seeing all six also checks
//! forwarding beyond this node's immediate connections.

use aether_node::presence::NODE_VERSION;
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
        let dir =
            std::env::temp_dir().join(format!("aether-presence-devnet-{}", std::process::id()));
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

fn assert_presence_shape(value: &Value, observer: usize) {
    assert_eq!(value["schema"], 1);
    assert_eq!(value["available"], true);
    assert_eq!(value["total"], NODES);
    assert_eq!(
        value["by_role"],
        json!({ "validator": 4, "candidate": 0, "follower": 2 })
    );
    assert_eq!(value["by_version"], json!({ (NODE_VERSION): NODES }));
    assert_eq!(
        value["by_region"],
        json!({
            "asia": 0, "europe": 0, "north_america": 0, "south_america": 0,
            "africa": 0, "oceania": 0, "unknown": NODES,
        })
    );
    assert_eq!(value["by_country"], json!({}));
    assert_eq!(value["ttl_seconds"], 180);
    assert_eq!(value["scope"], "what this node can see");
    assert_eq!(
        value["observer"],
        aether_net::devnet_node_id((observer + 1) as u64).to_string()
    );
    assert!(value["observed_at"].as_u64().is_some_and(|t| t > 0));
    let nodes = value["nodes"].as_array().expect("presence nodes array");
    assert_eq!(nodes.len(), NODES);
    let expected_ids: BTreeSet<_> = (1..=NODES as u64)
        .map(|i| aether_net::devnet_node_id(i).to_string())
        .collect();
    let seen_ids: BTreeSet<_> = nodes
        .iter()
        .map(|node| {
            let fields = node.as_object().expect("presence node object");
            assert_eq!(
                fields.keys().map(String::as_str).collect::<BTreeSet<_>>(),
                BTreeSet::from([
                    "node_id",
                    "role",
                    "version",
                    "timestamp",
                    "last_seen",
                    "region"
                ])
            );
            assert!(node["timestamp"].as_u64().is_some());
            assert!(node["last_seen"].as_u64().is_some());
            assert_eq!(node["version"], NODE_VERSION);
            assert_eq!(node["region"], "unknown");
            let node_id = node["node_id"].as_str().expect("node id");
            let validator = (1..=VALIDATORS as u64)
                .any(|i| aether_net::devnet_node_id(i).to_string() == node_id);
            assert_eq!(
                node["role"],
                if validator { "validator" } else { "follower" }
            );
            node_id.to_owned()
        })
        .collect();
    assert_eq!(
        seen_ids, expected_ids,
        "one count per authenticated node identity"
    );
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
fn four_validators_and_two_followers_see_six_macs_within_two_minutes() {
    let deadline = Instant::now() + Duration::from_secs(120);
    let net = LocalDevnet::start();
    let mut last = vec![None; NODES];
    loop {
        let mut all_six = true;
        for (i, snapshot) in last.iter_mut().enumerate() {
            *snapshot = net.rpc(i, "aether_presence", deadline);
            all_six &= snapshot.as_ref().is_some_and(|v| {
                v["total"] == NODES
                    && v["by_role"]["validator"] == 4
                    && v["by_role"]["follower"] == 2
            });
        }
        if all_six {
            let peer_deadline = Instant::now() + Duration::from_secs(15);
            for (i, value) in last.iter().enumerate() {
                assert_presence_shape(value.as_ref().unwrap(), i);
                let peers = net
                    .rpc(i, "aether_peers", peer_deadline)
                    .expect("live peers RPC answers");
                assert_peer_shape(&peers);
            }
            return;
        }
        assert!(
            Instant::now() < deadline,
            "presence did not converge to four validators and two followers in 120s: {last:?}{}",
            net.log_tails()
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}
