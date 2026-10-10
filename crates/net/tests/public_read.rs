//! Local-only real iroh tests: no public relay/DHT and no node process.

use aether_net::{
    ALPN_READ, ALPN_RPC, Endpoint, EndpointAddr, PublicRead, ReadBudget, Reservation, SecretKey,
    serve_with_public_read,
};
use iroh::endpoint::presets;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Budget(Mutex<(bool, usize, usize)>);
impl Budget {
    fn new(cap: usize) -> Self {
        Self(Mutex::new((true, cap, 0)))
    }
    fn used(&self) -> usize {
        self.0.lock().unwrap().2
    }
}
impl ReadBudget for Budget {
    fn reserve(&self, maximum: usize) -> Result<Reservation, String> {
        let mut state = self.0.lock().unwrap();
        if !state.0 {
            return Err("disabled".into());
        }
        let bytes = maximum.min(state.1.saturating_sub(state.2));
        if bytes == 0 {
            return Err("daily cap".into());
        }
        state.2 += bytes;
        Ok(Reservation { period: 1, bytes })
    }
    fn refund(&self, reservation: Reservation, unused: usize) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state.2 = state.2.saturating_sub(unused.min(reservation.bytes));
        Ok(())
    }
}

async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .relay_mode(iroh::RelayMode::Disabled)
        .secret_key(SecretKey::generate())
        .bind()
        .await
        .unwrap()
}

fn address(endpoint: &Endpoint) -> EndpointAddr {
    let port = endpoint
        .bound_sockets()
        .into_iter()
        .find(|addr| addr.is_ipv4())
        .unwrap()
        .port();
    EndpointAddr::from(endpoint.id()).with_ip_addr((std::net::Ipv4Addr::LOCALHOST, port).into())
}

async fn call(conn: &aether_net::Connection, method: &str) -> Result<Value, String> {
    request(
        conn,
        json!({"jsonrpc":"2.0", "id":7, "method":method,"params":[]}),
    )
    .await
}

async fn request(conn: &aether_net::Connection, req: Value) -> Result<Value, String> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(&req).unwrap();
        // Deliberately split the bytes: QUIC write/chunk boundaries are not framing.
        send.write_all(&bytes[..11])
            .await
            .map_err(|e| e.to_string())?;
        send.write_all(&bytes[11..])
            .await
            .map_err(|e| e.to_string())?;
        send.finish().map_err(|e| e.to_string())?;
        let answer = recv.read_to_end(1 << 20).await.map_err(|e| e.to_string())?;
        serde_json::from_slice(&answer).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "timed out".to_string())?
}

fn response(req: Value) -> Value {
    json!({"jsonrpc":"2.0","id":req["id"],"result":{"height":42,"certified":true}})
}

#[tokio::test]
async fn public_read_alpn_frames_answers_and_legacy_rpc_shares_the_byte_cap() {
    let server = endpoint().await;
    let addr = address(&server);
    let budget = Arc::new(Budget::new(1024));
    let read = PublicRead::new(|req| async move { response(req) }, budget.clone(), vec![]);
    let router =
        serve_with_public_read(server, |req| async move { response(req) }, None, None, read);
    let client = endpoint().await;
    let read = client.connect(addr.clone(), ALPN_READ).await.unwrap();
    assert_eq!(
        call(&read, "aether_status").await.unwrap()["result"]["height"],
        42
    );
    let after_read = budget.used();
    assert!(after_read > 0 && after_read < 1024);
    let native = client.connect(addr, ALPN_RPC).await.unwrap();
    assert_eq!(call(&native, "aether_status").await.unwrap()["id"], 7);
    assert!(
        budget.used() > after_read,
        "the old ALPN cannot bypass the public read cap"
    );
    budget.0.lock().unwrap().0 = false;
    assert!(
        call(&read, "aether_status").await.is_err(),
        "a connected reader observes the operator toggle"
    );
    assert!(
        call(&native, "aether_status").await.is_err(),
        "legacy public RPC observes the same toggle"
    );
    router.shutdown().await.unwrap();
    client.close().await;
}

#[tokio::test]
async fn one_reader_cannot_starve_another_and_denials_are_charged() {
    let server = endpoint().await;
    let addr = address(&server);
    let budget = Arc::new(Budget::new(4 * 1024 * 1024));
    let release = Arc::new(tokio::sync::Notify::new());
    let (held_tx, mut held_rx) = tokio::sync::mpsc::channel::<()>(4);
    let release_handler = release.clone();
    let read = PublicRead::new(
        move |req| {
            let (release, held) = (release_handler.clone(), held_tx.clone());
            async move {
                if req["method"] == "hold" {
                    held.send(()).await.unwrap();
                    release.notified().await;
                }
                response(req)
            }
        },
        budget.clone(),
        vec![],
    );
    let router =
        serve_with_public_read(server, |req| async move { response(req) }, None, None, read);
    let client = endpoint().await;
    let other = endpoint().await;
    let conn = client.connect(addr.clone(), ALPN_READ).await.unwrap();
    let mut held = Vec::new();
    for _ in 0..4 {
        let conn = conn.clone();
        held.push(tokio::spawn(async move { call(&conn, "hold").await }));
    }
    for _ in 0..4 {
        tokio::time::timeout(Duration::from_secs(10), held_rx.recv())
            .await
            .unwrap()
            .unwrap();
    }
    let before = budget.used();
    let denied = call(&conn, "aether_status").await.unwrap();
    assert_eq!(denied["error"]["code"], -32002);
    assert!(
        budget.used() > before,
        "error request and answer consume daily bytes"
    );
    let conn2 = other.connect(addr, ALPN_READ).await.unwrap();
    assert_eq!(
        call(&conn2, "aether_status").await.unwrap()["result"]["height"],
        42
    );
    release.notify_waiters();
    for task in held {
        assert!(task.await.unwrap().is_ok());
    }
    router.shutdown().await.unwrap();
    client.close().await;
    other.close().await;
}

#[tokio::test]
async fn a_tight_daily_cap_never_sends_an_uncounted_answer() {
    let server = endpoint().await;
    let addr = address(&server);
    let budget = Arc::new(Budget::new(100));
    let read = PublicRead::new(|req| async move { response(req) }, budget.clone(), vec![]);
    let router =
        serve_with_public_read(server, |req| async move { response(req) }, None, None, read);
    let client = endpoint().await;
    let conn = client.connect(addr, ALPN_READ).await.unwrap();
    assert!(call(&conn, "aether_status").await.is_err());
    assert!(budget.used() <= 100);
    assert!(call(&conn, "aether_status").await.is_err());
    assert!(
        budget.used() <= 100,
        "refused requests and answers never escape the cap"
    );
    router.shutdown().await.unwrap();
    client.close().await;
}

#[tokio::test]
async fn rate_limited_reader_does_not_spend_another_readers_tokens() {
    let server = endpoint().await;
    let addr = address(&server);
    let budget = Arc::new(Budget::new(4 * 1024 * 1024));
    let read = PublicRead::new(|req| async move { response(req) }, budget, vec![]);
    let router =
        serve_with_public_read(server, |req| async move { response(req) }, None, None, read);
    let client = endpoint().await;
    let other = endpoint().await;
    let conn = client.connect(addr.clone(), ALPN_READ).await.unwrap();
    let mut refused = 0;
    // Sequential requests cannot hit the concurrency cap; refusals here are
    // the burst/rate budget. The fixture uses memory policy, no disk fsync.
    for _ in 0..64 {
        let result = call(&conn, "aether_status").await.unwrap();
        if result["error"]["code"] == -32002 {
            refused += 1;
        }
    }
    assert!(
        refused > 0,
        "a burst above the service's per-peer rate was accepted"
    );
    let conn2 = other.connect(addr, ALPN_READ).await.unwrap();
    assert_eq!(
        call(&conn2, "aether_status").await.unwrap()["result"]["height"],
        42
    );
    router.shutdown().await.unwrap();
    client.close().await;
    other.close().await;
}

#[tokio::test]
async fn peer_exchange_includes_admitted_followers_but_never_unregistered_announcements() {
    use commonware_codec::Encode as _;
    use commonware_cryptography::{Signer as _, ed25519};
    let server = endpoint().await;
    let addr = address(&server);
    let wallet = endpoint().await;
    let unknown = endpoint().await;
    let client = endpoint().await;
    let key = ed25519::PrivateKey::from_seed(7);
    let listed = key.public_key().encode().to_vec();
    let registered: aether_net::RegisteredCandidate =
        Arc::new(move |key| (key.as_slice() == listed.as_slice()).then_some([7; 20]));
    let own = server.id().to_string();
    let read = PublicRead::new(
        move |req| {
            let own = own.clone();
            async move { json!({"jsonrpc":"2.0", "id":req["id"],"result":[own]}) }
        },
        Arc::new(Budget::new(4 * 1024 * 1024)),
        vec![],
    );
    let router = serve_with_public_read(
        server,
        |req| async move { response(req) },
        None,
        Some(registered),
        read,
    );
    let conn = wallet.connect(addr.clone(), ALPN_RPC).await.unwrap();
    let announce = |key: &ed25519::PrivateKey, node: &aether_net::EndpointId| {
        json!({
            "jsonrpc":"2.0", "id":1,"method":"aether_announceWalletServer","params":[
                hex::encode(key.public_key().encode()),
                hex::encode(key.sign(aether_net::WALLET_SERVER_NAMESPACE, node.as_bytes()).encode()),
            ]
        })
    };
    let accepted = request(&conn, announce(&key, &wallet.id())).await.unwrap();
    assert_eq!(accepted["result"]["ok"], true);
    let unknown_conn = unknown.connect(addr.clone(), ALPN_RPC).await.unwrap();
    let rejected = request(
        &unknown_conn,
        announce(&ed25519::PrivateKey::from_seed(8), &unknown.id()),
    )
    .await
    .unwrap();
    assert!(rejected["error"].is_object());
    let probe = client.connect(addr, ALPN_READ).await.unwrap();
    let hints = call(&probe, "aether_readPeers").await.unwrap();
    let hints = hints["result"].as_array().unwrap();
    assert_eq!(hints.len(), 2);
    assert_eq!(hints[0], json!({"node": wallet.id().to_string(), "operator": hex::encode([7; 20])}),
        "admitted operator diversity must reach browser discovery before truncation");
    assert!(!hints.iter().any(|hint| hint.as_str().or_else(|| hint["node"].as_str()) == Some(unknown.id().to_string().as_str())));
    router.shutdown().await.unwrap();
    wallet.close().await;
    unknown.close().await;
    client.close().await;
}
