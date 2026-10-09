//! A devnet-only forged peer for the browser's real light-verifier test.
//!
//! Build with the lane's compile gate, then run the resulting example directly:
//! `AETHER_IROH_NO_DHT=1 AETHER_IROH_RELAY_URL=http://127.0.0.1:3340 \
//! target/debug/examples/public_read_forged --rpc http://127.0.0.1:28545 \
//! --hint ./tmp/p2p-read/forged-peer.json`
//! Only the status hash is forged; finality certificates stay genuine.

use aether_net::public_read::{MAX_READ_REQUEST, MAX_READ_RESPONSE};
use aether_net::{Connection, SecretKey};
use clap::Parser;
use serde_json::{json, Value};
use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;
use tokio::task::JoinSet;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

const ALPN: &[u8] = b"eastsea/read/1";
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CONNECTIONS: usize = 16;
const READ_METHODS: &[&str] = &[
    "aether_status",
    "aether_recentBlocks",
    "aether_candidates",
    "aether_proverStatus",
    "aether_getBlock",
    "aether_getReceipt",
    "aether_getReceiptProof",
    "aether_getAccount",
    "aether_getStorage",
    "aether_getCodeHash",
    "aether_getFinalized",
    "aether_readPeers",
    "aether_presence",
    "aether_history",
    "aether_historyProof",
    "aether_eraInfo",
    "aether_eraProof",
    "aether_rewards",
    "aether_accountHistory",
    "eth_blockNumber",
    "eth_call",
    "eth_getLogs",
];

#[derive(Parser)]
#[command(about = "Serve a deliberately forged status header over the public-read ALPN")]
struct Args {
    /// The running local devnet node's HTTP RPC endpoint.
    #[arg(long)]
    rpc: reqwest::Url,
    /// A task-owned ./tmp path for the browser's JSON node/relay hint.
    #[arg(long)]
    hint: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.rpc.scheme() != "http"
        || args.rpc.host_str() != Some("127.0.0.1")
        || args.rpc.port().is_none()
        || !args.rpc.username().is_empty()
        || args.rpc.password().is_some()
        || args.rpc.path() != "/"
        || args.rpc.query().is_some()
        || args.rpc.fragment().is_some()
    {
        return Err("--rpc must be http://127.0.0.1:<devnet-port>".into());
    }
    let relay = std::env::var("AETHER_IROH_RELAY_URL")?;
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()?;
    // The generated key lives only in memory; the hint exposes no key material.
    let endpoint = aether_net::bind(Some(SecretKey::generate()), vec![ALPN.to_vec()]).await?;
    let mut stop = Box::pin(tokio::signal::ctrl_c());
    tokio::select! {
        ready = tokio::time::timeout(TIMEOUT, endpoint.online()) => {
            if ready.is_err() {
                endpoint.close().await;
                return Err("the explicit relay did not become ready within 15 seconds".into());
            }
        }
        stopped = &mut stop => {
            endpoint.close().await;
            stopped?;
            return Ok(());
        }
    }
    let hint = json!({"node": endpoint.id().to_string(), "relay": relay});
    let hint_bytes = serde_json::to_vec(&hint)?;
    if let Err(error) = aether_node::atomic::replace(&args.hint, &hint_bytes, 0o600) {
        endpoint.close().await;
        return Err(error.into());
    }
    println!("{hint}");

    let mut connections = JoinSet::new();
    let stop_result = loop {
        tokio::select! {
            stopped = &mut stop => break stopped,
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break Ok(()) };
                if connections.len() >= MAX_CONNECTIONS {
                    let _ = incoming.refuse();
                    continue;
                }
                let client = client.clone();
                let rpc = args.rpc.clone();
                connections.spawn(async move {
                    match tokio::time::timeout(TIMEOUT, async { incoming.await }).await {
                        Ok(Ok(connection)) => serve(connection, client, rpc).await,
                        Ok(Err(error)) => eprintln!("forged peer handshake: {error}"),
                        Err(_) => eprintln!("forged peer handshake timed out"),
                    }
                });
            }
            _ = connections.join_next(), if !connections.is_empty() => {}
        }
    };
    endpoint.close().await;
    connections.shutdown().await;
    stop_result?;
    Ok(())
}

async fn serve(connection: Connection, client: reqwest::Client, rpc: reqwest::Url) {
    loop {
        let Ok(Ok((mut send, mut recv))) =
            tokio::time::timeout(TIMEOUT, connection.accept_bi()).await
        else {
            break;
        };
        // FIN-delimited JSON, identical to the actual service. The whole
        // read/proxy/write operation has one deadline and bounded buffers.
        let served = tokio::time::timeout(TIMEOUT, async {
            let response = match recv.read_to_end(MAX_READ_REQUEST).await {
                Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(request) => answer(&client, &rpc, request).await,
                    Err(_) => rpc_error(Value::Null, -32700, "invalid JSON-RPC request"),
                },
                Err(_) => {
                    let _ = recv.stop(1u32.into());
                    rpc_error(Value::Null, -32002, "request exceeds the 64 KiB read limit")
                }
            };
            let bytes = serde_json::to_vec(&response)?;
            if bytes.len() > MAX_READ_RESPONSE {
                return Err("response exceeds the 16 MiB read limit".into());
            }
            send.write_all(&bytes).await?;
            send.finish()?;
            Ok::<(), Box<dyn Error + Send + Sync>>(())
        })
        .await;
        if !matches!(served, Ok(Ok(()))) {
            let _ = recv.stop(1u32.into());
            let _ = send.reset(1u32.into());
        }
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

async fn answer(client: &reqwest::Client, rpc: &reqwest::Url, request: Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if !request.is_object()
        || request.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || !request.get("params").is_none_or(Value::is_array)
    {
        return rpc_error(
            id,
            -32600,
            "expected one JSON-RPC 2.0 request with array params",
        );
    }
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    if !READ_METHODS.contains(&method) {
        return rpc_error(id, -32601, "only public read methods are allowed");
    }
    let mut response = match proxy(client, rpc, &request).await {
        Ok(response) => response,
        Err(error) => {
            eprintln!("forged peer upstream {method}: {error}");
            return rpc_error(
                id,
                -32002,
                "local devnet RPC could not answer within the read limits",
            );
        }
    };
    if method == "aether_status" {
        if let Some(status) = response.get_mut("result").and_then(Value::as_object_mut) {
            status.insert("hash".into(), Value::String("00".repeat(32)));
        }
    } else if method == "aether_readPeers" && response["error"]["code"] == -32601 {
        return json!({"jsonrpc": "2.0", "id": id, "result": []});
    }
    response
}

async fn proxy(client: &reqwest::Client, rpc: &reqwest::Url, request: &Value) -> Result<Value> {
    let mut response = client
        .post(rpc.clone())
        .json(request)
        .send()
        .await?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|bytes| bytes > MAX_READ_RESPONSE as u64)
    {
        return Err("upstream response exceeds 16 MiB".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_READ_RESPONSE {
            return Err("upstream response exceeds 16 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}
