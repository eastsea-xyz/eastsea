//! Phase-0 security regression tests. Each test boots a real `aether-node`
//! process on a free loopback port with an isolated HOME.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Node {
    child: Child,
    port: u16,
    home: PathBuf,
}

impl Node {
    fn start() -> Node {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let home = std::env::temp_dir().join(format!("aether-sec-{}-{}", std::process::id(), port));
        std::fs::create_dir_all(&home).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_aether-node"))
            .args(["--port", &port.to_string(), "--no-open"])
            .env("HOME", &home)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn aether-node");
        let node = Node { child, port, home };
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() && node.home.join(".aether/token").exists() {
                return node;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("node did not start on port {port}");
    }

    fn token(&self) -> String {
        std::fs::read_to_string(self.home.join(".aether/token")).unwrap().trim().to_string()
    }

    fn host(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// Raw HTTP/1.1 exchange; returns (status code, full response text).
    fn send(&self, method: &str, path: &str, headers: &[(&str, String)], body: &str) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut req = format!("{method} {path} HTTP/1.1\r\n");
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str(&format!("Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()));
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        let code = out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        (code, out)
    }

    fn authed(&self) -> Vec<(&'static str, String)> {
        vec![("Host", self.host()), ("X-Aether-Token", self.token())]
    }

    fn my_balance(&self) -> (String, u64) {
        let (code, resp) = self.send("GET", "/api/status", &self.authed(), "");
        assert_eq!(code, 200, "{resp}");
        let body = resp.split("\r\n\r\n").nth(1).unwrap();
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        (v["address"].as_str().unwrap().to_string(), v["balance"].as_u64().unwrap())
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

#[test]
fn control_api_requires_token() {
    let n = Node::start();
    let (code, _) = n.send("POST", "/api/tx", &[("Host", n.host())], r#"{"amount":1}"#);
    assert_eq!(code, 401);
    let (code, _) = n.send("GET", "/api/status", &[("Host", n.host())], "");
    assert_eq!(code, 401);
}

#[test]
fn control_api_rejects_foreign_origin_and_host() {
    let n = Node::start();
    let mut h = n.authed();
    h.push(("Origin", "https://evil.example".into()));
    assert_eq!(n.send("POST", "/api/tx", &h, r#"{"amount":1}"#).0, 403);

    let h = vec![("Host", format!("evil.example:{}", n.port)), ("X-Aether-Token", n.token())];
    assert_eq!(n.send("GET", "/api/status", &h, "").0, 403);
}

#[test]
fn responses_carry_no_wildcard_cors() {
    let n = Node::start();
    let (code, resp) = n.send("GET", "/api/status", &n.authed(), "");
    assert_eq!(code, 200);
    assert!(!resp.to_ascii_lowercase().contains("access-control-allow-origin"), "{resp}");
}

#[test]
fn gossip_cannot_move_funds() {
    let n = Node::start();
    let (addr, before) = n.my_balance();
    assert!(before >= 500_000);
    // Unsigned transfer "from" the node's own address to an attacker.
    let tx = serde_json::json!({ "NewTx": {
        "id": 1, "sender": addr_bytes(&addr), "nonce": 0, "gas_limit": 21000,
        "payload": { "Transfer": { "to": addr_bytes("0x00000000000000000000000000000000000003e7"), "amount": 500_000 } }
    }});
    let (code, _) = n.send("POST", "/api/p2p/gossip", &[("Host", n.host())], &tx.to_string());
    assert_eq!(code, 200);
    let (_, after) = n.my_balance();
    assert!(after >= before, "gossip moved funds: {before} -> {after}");
}

#[test]
fn removed_fake_endpoints_are_gone() {
    let n = Node::start();
    assert_eq!(n.send("POST", "/api/mev_attack", &n.authed(), "").0, 404);
}

#[test]
fn binds_loopback_only_by_default() {
    let n = Node::start();
    let lan = aether_core::p2p::get_local_ip();
    if lan == "127.0.0.1" {
        return; // no non-loopback interface in this environment
    }
    let reachable = TcpStream::connect_timeout(
        &format!("{lan}:{}", n.port).parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok();
    assert!(!reachable, "node reachable on {lan}:{}", n.port);
}

#[test]
fn dashboard_is_served_with_token_and_escapes_remote_data() {
    let n = Node::start();
    let (code, page) = n.send("GET", "/", &[("Host", n.host())], "");
    assert_eq!(code, 200);
    assert!(page.contains(&n.token()), "token meta not injected");
    for raw in ["<span>${c.name}</span>", "aether://${p.node_id}</div>", "(${data.nat.external_ip})"] {
        assert!(!page.contains(raw), "unescaped remote value in dashboard: {raw}");
    }
    let (code, _) = n.send("GET", "/", &[("Host", "evil.example".into())], "");
    assert_eq!(code, 403);
}

fn addr_bytes(hex: &str) -> Vec<u8> {
    let h = hex.trim_start_matches("0x");
    (0..20).map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap()).collect()
}
