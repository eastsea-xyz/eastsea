//! `verified_block` against a node we control: the same captured devnet
//! artifacts as local_node.rs (its own test binary — the FFI's network state
//! is process-global, so one mutating test per binary), proving the block
//! checks an anchor passes hold for history queries while the state-only
//! rules (staleness, the replay floor) deliberately do not.

use aether_ffi::{use_devnet_keys, use_local_node, verified_block};
use aether_test_support::Port;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

/// The captured 4-validator devnet: block 6 and 7 with their certificates,
/// over the state of height 5.
fn fixture() -> Value {
    serde_json::from_str(include_str!("../../light/tests/fixtures/devnet4.json")).unwrap()
}

/// What the fake node is holding as "the block you asked for".
struct Served {
    chain_id: u64,
    block: String,
    finalization: String,
}

/// A JSON-RPC node on 127.0.0.1 answering from `served` (local_node.rs's
/// fake, trimmed to the two methods `verified_block` asks).
fn fake_node(served: Arc<Mutex<Served>>) -> u16 {
    let port_guard = Port::reserve().expect("reserve fake node port");
    let listener = port_guard.bind_tcp().expect("bind fake node");
    let port = port_guard.port();
    std::thread::spawn(move || {
        let _port_guard = port_guard;
        for conn in listener.incoming() {
            let mut s = match conn {
                Ok(s) => s,
                Err(_) => continue,
            };
            let Some(req) = read_request(&mut s) else { continue };
            let method = req["method"].as_str().unwrap_or_default().to_string();
            let g = served.lock().expect("served lock");
            let result = match method.as_str() {
                "aether_status" => json!({ "chain_id": g.chain_id, "height": 7, "state_root": fixture()["state_root"], "mempool": 0, "base_fee": { "exec": "1000000000", "prove": "0" } }),
                // Whatever block it is holding, honest or not.
                "aether_getFinalized" => json!({ "height": 6, "block": g.block, "finalization": g.finalization, "links": [] }),
                _ => Value::Null,
            };
            let body = json!({ "jsonrpc": "2.0", "id": 1, "result": result }).to_string();
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    port
}

/// One request: headers up to the blank line, then exactly Content-Length bytes.
fn read_request(s: &mut std::net::TcpStream) -> Option<Value> {
    let (mut head, mut byte) = (Vec::new(), [0u8; 1]);
    while !head.ends_with(b"\r\n\r\n") {
        if s.read(&mut byte).ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    let headers = String::from_utf8(head).ok()?;
    let len: usize = headers
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length"))?
        .split_once(':')?
        .1
        .trim()
        .parse()
        .ok()?;
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

#[test]
fn block_history_is_certified_without_the_state_only_guards() {
    let f = fixture();
    let served = Arc::new(Mutex::new(Served {
        chain_id: 7_777,
        block: f["anchor_block"].as_str().unwrap().to_string(),
        finalization: f["anchor_finalization"].as_str().unwrap().to_string(),
    }));
    let port = fake_node(served.clone());
    use_local_node(Some(port));

    // No identity pinned and no dev mode: nothing is verified at all.
    let err = verified_block(6).map(|_| ()).unwrap_err().to_string();
    assert!(err.contains("identity"), "{err}");

    use_devnet_keys();

    // The captured certificate is years old and its state long since superseded
    // — for history that is fine, so the block verifies where an anchor would
    // have refused it as stale.
    let b = verified_block(6).expect("the captured certificate verifies");
    assert_eq!(b.height, 6);
    assert_eq!(b.digest.len(), 64, "the digest is a bare hex hash: {digest}", digest = b.digest);
    assert!(b.digest.chars().all(|c| c.is_ascii_hexdigit()));

    // A node answering a height with some other certified block it holds is refused.
    {
        let mut g = served.lock().expect("served lock");
        g.block = f["next_block"].as_str().unwrap().to_string();
        g.finalization = f["next_finalization"].as_str().unwrap().to_string();
    }
    let err = verified_block(6).map(|_| ()).unwrap_err().to_string();
    assert!(err.contains("asked for block 6"), "{err}");

    // Serving the past after the newer block is history, not replay: the query
    // for the older height still succeeds (an anchor here would refuse with
    // "finalized blocks never go back").
    assert!(verified_block(7).is_ok(), "the next block verifies");
    {
        let mut g = served.lock().expect("served lock");
        g.block = f["anchor_block"].as_str().unwrap().to_string();
        g.finalization = f["anchor_finalization"].as_str().unwrap().to_string();
    }
    assert!(verified_block(6).is_ok(), "an older height stays verifiable");

    // A node on another chain is refused.
    served.lock().expect("served lock").chain_id = 7_780;
    let err = verified_block(6).map(|_| ()).unwrap_err().to_string();
    assert!(err.contains("chain 7780"), "{err}");
    use_local_node(None);
}
