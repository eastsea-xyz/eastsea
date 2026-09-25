//! Real-network test: a server publishes itself to the BitTorrent Mainline DHT;
//! a client that only knows the server's node id finds it there and calls RPC.
//! Needs internet access (UDP). Run: cargo test -p aether-net --test dht_rpc -- --ignored

use aether_net::{bind, serve_rpc, RpcClient, ALPN_RPC};
use iroh::SecretKey;
use serde_json::json;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "uses the public Mainline DHT"]
async fn client_finds_server_through_mainline_dht() {
    let secret = SecretKey::generate();
    let id = secret.public();
    let server = bind(Some(secret), vec![ALPN_RPC.to_vec()]).await.unwrap();
    let _router = serve_rpc(server, |req| async move { json!({ "jsonrpc": "2.0", "id": req["id"], "result": { "echo": req["method"] } }) });

    let start = Instant::now();
    let client = RpcClient::new(vec![id]).await.unwrap();
    let mut last = String::new();
    while start.elapsed() < Duration::from_secs(120) {
        match client.call("ping", json!([])).await {
            Ok(v) => {
                assert_eq!(v["echo"], "ping");
                println!("resolved via Mainline DHT and answered after {:?} ({})", start.elapsed(), client.describe().await);
                return;
            }
            Err(e) => last = e.to_string(),
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    panic!("not reachable via DHT within 120s: {last}");
}
