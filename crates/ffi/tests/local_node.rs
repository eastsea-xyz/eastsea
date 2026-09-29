//! The verification path against a node we control: a fake JSON-RPC node on
//! 127.0.0.1 serving the real devnet artifacts captured in aether-light's
//! fixture, so every guard (identity, chain, replay, staleness) is exercised
//! end to end without a network.

use aether_ffi::{account_history, use_devnet_keys, use_local_node, verified_account, verified_height};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

/// The captured 4-validator devnet: block 6 and 7 with their certificates,
/// over the state of height 5, and a proof for one account under it.
fn fixture() -> Value {
    serde_json::from_str(include_str!("../../light/tests/fixtures/devnet4.json")).unwrap()
}

/// What the fake node is serving right now: a node can lag, replay, sit on
/// another chain, or hand out the wrong block for a height.
struct Served {
    chain_id: u64,
    /// The state height it answers `aether_getAccount` with.
    height: u64,
    block: String,
    finalization: String,
}

/// A JSON-RPC node on 127.0.0.1 answering from `served`: one request per
/// connection, which is what the wallet's local-node client expects.
fn fake_node(served: Arc<Mutex<Served>>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fake node");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let mut s = match conn {
                Ok(s) => s,
                Err(_) => continue,
            };
            let Some(req) = read_request(&mut s) else { continue };
            let f = fixture();
            let (method, asked) = (req["method"].as_str().unwrap_or_default().to_string(), req["params"][0].as_u64().unwrap_or_default());
            let g = served.lock().expect("served lock");
            let result = match method.as_str() {
                "aether_status" => json!({
                    "chain_id": g.chain_id,
                    "height": g.height + 1,
                    "state_root": f["state_root"],
                    "mempool": 0,
                    "base_fee": { "exec": "1000000000", "prove": "0" },
                }),
                "aether_getAccount" => json!({
                    "address": f["address"],
                    "balance": f["balance"],
                    "nonce": 0,
                    "height": g.height,
                    "state_root": f["state_root"],
                    "proof": f["proof"],
                }),
                // Whatever block it is holding, honest or not.
                "aether_getFinalized" => json!({ "height": asked - 1, "block": g.block, "finalization": g.finalization, "links": [] }),
                "aether_accountHistory" => json!({ "entries": [], "next_cursor": null, "history_start": 5, "indexed_height": g.height }),
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
fn verification_guards_refuse_unconfigured_stale_and_foreign_state() {
    let f = fixture();
    let addr = f["address"].as_str().unwrap().to_string();
    let at = |height: u64, key: &str| Served {
        chain_id: 7_777,
        height,
        block: f[&format!("{key}_block")].as_str().unwrap().to_string(),
        finalization: f[&format!("{key}_finalization")].as_str().unwrap().to_string(),
    };
    let served = Arc::new(Mutex::new(at(5, "anchor")));
    let port = fake_node(served.clone());
    use_local_node(Some(port));
    let read = || {
        let err = verified_account(addr.clone(), 4).map(|_| ()).unwrap_err().to_string();
        println!("verified_account: {err}");
        err
    };

    // No identity pinned and no dev mode: the public devnet key is not a
    // fallback, so nothing is verified at all.
    let err = read();
    assert!(err.contains("identity"), "{err}");

    // Dev mode on: the devnet committee key really does verify the captured
    // certificate — and then the (long since captured) state is refused as
    // stale, with the height it was certified at becoming this process's floor.
    use_devnet_keys();
    for _ in 0..2 {
        let err = read();
        assert!(err.contains("stale"), "{err}");
    }
    assert_eq!(verified_height(), 6, "the certified height is exposed even when the state is too old");

    // A node answering a height with some other certified block it holds is refused.
    let next = f["next_block"].as_str().unwrap().to_string();
    let next_fin = f["next_finalization"].as_str().unwrap().to_string();
    {
        let mut g = served.lock().expect("served lock");
        (g.block, g.finalization) = (next.clone(), next_fin.clone());
    }
    let err = read();
    assert!(err.contains("asked for block 6"), "{err}");
    assert_eq!(verified_height(), 6);

    // A newer certified block moves the floor up (still too old to show).
    *served.lock().expect("served lock") = at(6, "next");
    let err = read();
    assert!(err.contains("stale"), "{err}");
    assert_eq!(verified_height(), 7);

    // A node then serving the older state again is replaying it, not reporting it.
    *served.lock().expect("served lock") = at(5, "anchor");
    let err = read();
    assert!(err.contains("never go back"), "{err}");

    // A node on another chain is refused before its state is even fetched.
    served.lock().expect("served lock").chain_id = 7_780;
    let err = read();
    assert!(err.contains("chain 7780"), "{err}");

    // History is a node-sourced read, independent of the verified-balance
    // guard above; its paging contract still validates address and limit.
    let page: Value = serde_json::from_str(&account_history(addr.clone(), None, 200).unwrap()).unwrap();
    assert_eq!(page["history_start"], 5);
    assert!(account_history(addr, None, 201).is_err());
    use_local_node(None);
}
