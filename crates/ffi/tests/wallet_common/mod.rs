//! Shared fake servers for the wallet-read-spread tests: iroh endpoints on
//! 127.0.0.1 answering from aether-light's captured devnet fixture, so the
//! spread, the politeness and the demotion all run over real QUIC without a
//! network. A directory module, not a test target of its own.

use aether_net::{Endpoint, SecretKey};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What a fake server serves. `Chain(7_777)` is an honest follower on the
/// devnet; `Chain(other)` serves the same fixture but reports another chain
/// (what a lying node would do); `Busy` is over its limits.
#[derive(Clone)]
pub enum Mode {
    Chain(u64),
    Busy,
    /// An honest devnet validator that admitted the one transaction this
    /// fixture submits (`HELD_TX`): it accepts the submission and answers
    /// its receipt as pending — what a follower, which never saw the
    /// validators' pending gossip, cannot do (B5 review round 2, finding 2).
    Holding,
}

/// The hash the `Holding` validator gives the submission it admits.
pub const HELD_TX: &str = "0x4b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b";

/// A fake follower or validator: where it landed and what it answered.
pub struct Fake {
    pub node: String,
    pub socket: String,
    served: Arc<Mutex<Vec<String>>>,
}

impl Fake {
    /// How many requests of `method` this server answered.
    pub fn served(&self, method: &str) -> usize {
        self.served
            .lock()
            .expect("served lock")
            .iter()
            .filter(|m| m.as_str() == method)
            .count()
    }

    /// How many requests this server answered in total.
    pub fn total(&self) -> usize {
        self.served.lock().expect("served lock").len()
    }

    /// This server as a pinned endpoint.
    pub fn pinned(&self) -> aether_ffi::PinnedServer {
        aether_ffi::PinnedServer {
            node: self.node.clone(),
            socket: self.socket.clone(),
        }
    }
}

/// The captured 4-validator devnet (aether-light's fixture): block 6 with its
/// certificate over the state of height 5, and a proof for one account.
fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../light/tests/fixtures/devnet4.json")).unwrap()
}

/// The account the fixture proves something about.
pub fn fixture_address() -> String {
    fixture()["address"].as_str().unwrap().to_string()
}

/// One JSON-RPC answer, as the `aether/rpc/1` transport expects it.
fn answer(mode: &Mode, req: Value, served: &Arc<Mutex<Vec<String>>>) -> Value {
    let f = fixture();
    let method = req["method"].as_str().unwrap_or_default().to_string();
    served.lock().expect("served lock").push(method.clone());
    if matches!(mode, Mode::Busy) {
        // The exact error an over-limit transport gives.
        return json!({ "jsonrpc": "2.0", "id": req["id"], "error": { "code": -32000, "message": "server busy: rpc concurrency or rate limit reached, retry later" } });
    }
    let chain = match mode {
        Mode::Chain(c) => *c,
        Mode::Holding => 7_777,
        Mode::Busy => unreachable!("answered above"),
    };
    if matches!(mode, Mode::Holding) {
        match method.as_str() {
            "aether_sendTransaction" => return json!({ "jsonrpc": "2.0", "id": req["id"], "result": { "hash": HELD_TX } }),
            "aether_getReceipt" if req["params"][0].as_str() == Some(HELD_TX) => {
                return json!({ "jsonrpc": "2.0", "id": req["id"], "result": {
                    "pending": true, "status": "pending",
                    "waiting": { "kind": "state_price_above_cap", "cap": "2000000000000", "price": "43000000000000", "blocks": 1200 } } });
            }
            _ => {}
        }
    }
    let result = match method.as_str() {
        "aether_status" => json!({
            "chain_id": chain, "height": 6, "state_root": f["state_root"], "mempool": 0,
            // A paid-state chain reports its state price (pre-audit 7, M1);
            // the legacy 7780 mode ignores it.
            "base_fee": { "exec": "1000000000", "state": "1000000000000", "prove": "0" },
        }),
        "aether_getAccount" => json!({
            "address": f["address"], "balance": f["balance"], "nonce": 0,
            "height": f["height"], "state_root": f["state_root"], "proof": f["proof"],
        }),
        "eth_getTransactionCount" => json!("0x0"),
        "aether_accountHistory" => json!({ "entries": [], "next_cursor": null, "history_start": 1, "indexed_height": 6 }),
        // The captured anchor, whatever height is asked (the wallet checks).
        "aether_getFinalized" => json!({
            "height": req["params"][0].as_u64().unwrap_or_default().saturating_sub(1),
            "block": f["anchor_block"], "finalization": f["anchor_finalization"], "links": [],
        }),
        _ => Value::Null,
    };
    json!({ "jsonrpc": "2.0", "id": req["id"], "result": result })
}

/// A fake server on its own thread and runtime. `None` binds no `aether/rpc/1`
/// (a Mac that went away): connects fail fast instead of timing out. Localhost
/// only: no relay, no DHT.
pub fn server(mode: Option<Mode>) -> Fake {
    let served: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let s = served.clone();
    let (tx, rx) = std::sync::mpsc::channel::<(String, String)>();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("fake server runtime");
        rt.block_on(async move {
            let secret = SecretKey::generate();
            let alpn = match mode.as_ref() {
                Some(_) => aether_net::ALPN_RPC.to_vec(),
                None => b"aether/none/1".to_vec(),
            };
            let endpoint = Endpoint::builder(iroh::endpoint::presets::Minimal)
                .relay_mode(iroh::RelayMode::Disabled)
                .alpns(vec![alpn])
                .secret_key(secret)
                .bind()
                .await
                .expect("bind fake server");
            let (id, port) = (
                endpoint.id().to_string(),
                endpoint
                    .bound_sockets()
                    .into_iter()
                    .find(|a| a.is_ipv4())
                    .expect("an IPv4 socket")
                    .port(),
            );
            tx.send((id, format!("127.0.0.1:{port}")))
                .expect("the test is waiting");
            // The router (and the endpoint in it) must outlive this setup:
            // bound here, not inside an if, so it lives until the test ends.
            let _router = mode.clone().map(|m| {
                aether_net::serve_rpc(endpoint, move |req| {
                    let (s, m) = (s.clone(), m.clone());
                    async move { answer(&m, req, &s) }
                })
            });
            std::future::pending::<()>().await
        });
    });
    let (node, socket) = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("fake server bound");
    Fake {
        node,
        socket,
        served,
    }
}

/// An honest follower on the devnet chain.
pub fn follower() -> Fake {
    server(Some(Mode::Chain(7_777)))
}

/// A validator holding `HELD_TX` in its mempool.
pub fn holding() -> Fake {
    server(Some(Mode::Holding))
}

/// A follower that is over its limits.
pub fn busy() -> Fake {
    server(Some(Mode::Busy))
}

/// A follower serving the devnet fixture under another chain id.
pub fn liar() -> Fake {
    server(Some(Mode::Chain(999)))
}

/// A Mac that is not serving wallets.
pub fn dead() -> Fake {
    server(None)
}
