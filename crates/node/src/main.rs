//! `aether` — validator node and client.
//!
//!   aether node --index 1 --validators 4 --port 9001 --rpc-port 8545 --data /tmp/aether/1
//!   aether node --index 2 --validators 4 --port 9002 --rpc-port 8546 --data /tmp/aether/2 --bootstrap 1@127.0.0.1:9001
//!   aether send --from-dev 1 --to 0x… --value 1000 --wait
//!   aether balance 0x…              # fetches an EIP-7864 proof and verifies it locally

use aether_crypto::{P256Signer, Signer};
use aether_execution::{sign_call, EvmCall};
use aether_node::application::Application;
use aether_node::block::PublicKey;
use aether_node::chain::{dev_accounts, dev_seed, Chain, ChainConfig};
use aether_node::engine::{self, MAX_BLOCK_BYTES};
use aether_node::rpc::{self, RpcState};
use aether_state::Proof;
use aether_types::{Address, Bytes, GasVector, TxEnvelope, TxHash, U256};
use clap::{Parser, Subcommand};
use commonware_consensus::{marshal, simplex::scheme::ed25519::Scheme, types::ViewDelta};
use commonware_cryptography::{ed25519, Signer as _};
use commonware_p2p::{
    authenticated::{self, lookup},
    Address as PeerAddress, AddressableManager as _, Receiver as _, Recipients, Sender as _,
};
use commonware_runtime::{tokio as cw_tokio, Quota, Runner as _, Supervisor as _};
use commonware_utils::{
    ordered::{Map, Set},
    union, NZUsize, TryCollect, NZU32,
};
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use aether_light::NAMESPACE;
const DEFAULT_CHAIN_ID: u64 = 7_777;
const DEV_ACCOUNTS: u8 = 10;
const DEV_BALANCE: u128 = 1_000_000 * 10u128.pow(18);

#[derive(Parser)]
#[command(name = "aether", about = "Aether devnet node and client")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a validator.
    Node {
        #[arg(long)]
        index: u64,
        #[arg(long)]
        validators: u64,
        #[arg(long)]
        port: u16,
        #[arg(long)]
        rpc_port: u16,
        #[arg(long)]
        data: String,
        /// Plain-TCP transport (LAN/tests): every other validator as
        /// `<index>@<host:port>`, comma separated. Without it, validators reach
        /// each other over iroh (Mainline DHT discovery, hole punching, relays).
        #[arg(long, value_delimiter = ',')]
        peers: Vec<String>,
        /// First loopback port for iroh links (link to validator j = base + j).
        /// Default: 20000 + 100 * index.
        #[arg(long)]
        link_base: Option<u16>,
        /// No public iroh endpoint (no DHT publishing, no public RPC). TCP peers only.
        #[arg(long)]
        offline: bool,
        #[arg(long, default_value_t = 1000)]
        block_time_ms: u64,
        /// Devnet fault injection: propose blocks without this sender's txs and
        /// ignore inclusion lists (tests censorship resistance).
        #[arg(long, hide = true)]
        dev_censor: Option<Address>,
        /// Devnet: skip this sender in mempool ordering; its txs land only via inclusion lists.
        #[arg(long, hide = true)]
        dev_deprioritize: Option<Address>,
    },
    /// List the public development accounts (funded at genesis; never use for value).
    DevAccounts,
    /// Chain status.
    Status {
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
    },
    /// Recent finalized blocks.
    Blocks {
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        #[arg(default_value_t = 10)]
        n: usize,
    },
    /// Transfer native tokens.
    Send {
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        #[arg(long)]
        from_dev: u8,
        #[arg(long)]
        to: Address,
        #[arg(long)]
        value: U256,
        #[arg(long)]
        nonce: Option<u64>,
        #[arg(long)]
        wait: bool,
    },
    /// Deploy contract init code (hex).
    Deploy {
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        #[arg(long)]
        from_dev: u8,
        #[arg(long)]
        code: String,
    },
    /// Call a contract with calldata (hex).
    Call {
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        #[arg(long)]
        from_dev: u8,
        #[arg(long)]
        to: Address,
        #[arg(long, default_value = "")]
        data: String,
        #[arg(long)]
        wait: bool,
    },
    /// Account balance, verified locally with an EIP-7864 proof.
    Balance {
        address: Address,
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        /// Size of the trusted devnet validator set.
        #[arg(long, default_value_t = 4)]
        validators: u64,
    },
    /// Contract storage slot, verified locally with a proof.
    Storage {
        address: Address,
        slot: U256,
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        #[arg(long, default_value_t = 4)]
        validators: u64,
    },
    /// Transaction receipt.
    Receipt {
        hash: TxHash,
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
    },
}

fn main() {
    let cli = Cli::parse();
    let res = match cli.cmd {
        Cmd::Node { index, validators, port, rpc_port, data, peers, link_base, offline, block_time_ms, dev_censor, dev_deprioritize } => {
            let transport = if peers.iter().any(|p| !p.is_empty()) {
                Transport::Tcp(peers)
            } else {
                Transport::Iroh { link_base: link_base.unwrap_or(20_000 + 100 * index as u16) }
            };
            run_node(NodeArgs { index, n: validators, port, rpc_port, data, transport, offline, block_time_ms, dev_censor, dev_deprioritize });
            Ok(())
        }
        Cmd::DevAccounts => {
            for (i, a) in dev_accounts(DEV_ACCOUNTS) {
                println!("dev {i:>2}  {a}");
            }
            Ok(())
        }
        Cmd::Status { rpc } => call(&rpc, "aether_status", json!([])).map(|v| println!("{}", pretty(&v))),
        Cmd::Blocks { rpc, n } => call(&rpc, "aether_recentBlocks", json!([n])).map(|v| print_blocks(&v)),
        Cmd::Send { rpc, from_dev, to, value, nonce, wait } => {
            submit(&rpc, from_dev, nonce, EvmCall { to: Some(to), value, input: Bytes::new(), gas_limit: 21_000 }, wait).map(|_| ())
        }
        Cmd::Deploy { rpc, from_dev, code } => (|| {
            let input = Bytes::from(hex::decode(code.trim_start_matches("0x")).map_err(|e| e.to_string())?);
            let r = submit(&rpc, from_dev, None, EvmCall { to: None, value: U256::ZERO, input, gas_limit: 3_000_000 }, true)?;
            if let Some(a) = r.pointer("/receipt/contract_address") {
                println!("contract: {}", a.as_str().unwrap_or_default());
            }
            Ok(())
        })(),
        Cmd::Call { rpc, from_dev, to, data, wait } => (|| {
            let input = Bytes::from(hex::decode(data.trim_start_matches("0x")).map_err(|e| e.to_string())?);
            submit(&rpc, from_dev, None, EvmCall { to: Some(to), value: U256::ZERO, input, gas_limit: 1_000_000 }, wait).map(|_| ())
        })(),
        Cmd::Balance { address, rpc, validators } => verified_balance(&rpc, address, validators),
        Cmd::Storage { address, slot, rpc, validators } => verified_storage(&rpc, address, slot, validators),
        Cmd::Receipt { hash, rpc } => call(&rpc, "aether_getReceipt", json!([hash])).map(|v| println!("{}", pretty(&v))),
    };
    if let Err(e) = res {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: DEFAULT_CHAIN_ID,
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        alloc: dev_accounts(DEV_ACCOUNTS).into_iter().map(|(_, a)| (a, U256::from(DEV_BALANCE))).collect(),
    }
}

fn validator_key(i: u64) -> ed25519::PrivateKey {
    aether_light::devnet_validator_key(i)
}

enum Transport {
    Tcp(Vec<String>),
    Iroh { link_base: u16 },
}

struct NodeArgs {
    index: u64,
    n: u64,
    port: u16,
    rpc_port: u16,
    data: String,
    transport: Transport,
    offline: bool,
    block_time_ms: u64,
    dev_censor: Option<Address>,
    dev_deprioritize: Option<Address>,
}

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

/// Socket address Commonware dials for every validator. Over iroh, each remote
/// validator is a local link port; the link carries the TCP stream over QUIC.
fn peer_addresses(a: &NodeArgs) -> Map<PublicKey, PeerAddress> {
    let mut peers: Vec<(PublicKey, PeerAddress)> = vec![(validator_key(a.index).public_key(), PeerAddress::Symmetric(loopback(a.port)))];
    match &a.transport {
        Transport::Tcp(list) => {
            for p in list.iter().filter(|s| !s.is_empty()) {
                let (i, addr) = p.split_once('@').expect("peer is <index>@<host:port>");
                let addr: SocketAddr = addr.parse().expect("peer address");
                peers.push((validator_key(i.parse().expect("peer index")).public_key(), PeerAddress::Symmetric(addr)));
            }
        }
        Transport::Iroh { link_base } => {
            for j in (1..=a.n).filter(|j| *j != a.index) {
                let ingress = loopback(link_base + j as u16);
                peers.push((validator_key(j).public_key(), PeerAddress::Asymmetric { ingress: ingress.into(), egress: loopback(0) }));
            }
        }
    }
    peers.try_into().expect("unique validators")
}

fn run_node(a: NodeArgs) {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,commonware=warn".into())).init();
    let NodeArgs { index, n, port, rpc_port, block_time_ms, offline, dev_censor, dev_deprioritize, .. } = a;
    assert!(!offline || matches!(a.transport, Transport::Tcp(_)), "--offline needs --peers");
    let signer = validator_key(index);
    let validators: Set<PublicKey> = (1..=n).map(|i| validator_key(i).public_key()).try_collect().expect("unique validator keys");
    let peers = peer_addresses(&a);
    let max_peers = authenticated::peer_set_limit(&validators, &signer.public_key());
    // Validators listen on loopback; the outside world reaches them only via iroh
    // links (or explicitly configured TCP peers).
    let listen = match a.transport {
        Transport::Tcp(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port),
        Transport::Iroh { .. } => loopback(port),
    };
    let mut p2p_cfg = lookup::Config::local(signer.clone(), &union(NAMESPACE, b"_P2P"), listen, max_peers, MAX_BLOCK_BYTES + 1024 * 1024);
    // Link traffic arrives from 127.0.0.1; identity is proven by the handshake.
    p2p_cfg.bypass_ip_check = true;
    let links: Vec<u64> = match a.transport {
        Transport::Iroh { .. } => (1..=n).filter(|j| *j != index).collect(),
        Transport::Tcp(_) => vec![],
    };
    let link_base = match a.transport {
        Transport::Iroh { link_base } => link_base,
        Transport::Tcp(_) => 0,
    };
    let data = a.data;
    let executor = cw_tokio::Runner::new(cw_tokio::Config::new().with_storage_directory(&data));
    let cfg = chain_config();

    executor.start(async move |context| {
        // Public endpoint first: validator links and wallet RPC share it.
        let endpoint = if offline {
            None
        } else {
            match aether_net::bind(Some(aether_net::devnet_node_secret(index)), vec![aether_net::ALPN_RPC.to_vec(), aether_net::ALPN_P2P.to_vec()]).await {
                Ok(ep) => Some(ep),
                Err(e) => {
                    tracing::warn!(?e, "public endpoint unavailable");
                    None
                }
            }
        };
        let mut outbound = Vec::new();
        if let Some(ep) = &endpoint {
            for j in &links {
                let link = aether_net::tunnel::Outbound::spawn(ep.clone(), aether_net::devnet_node_id(*j), loopback(link_base + *j as u16))
                    .await
                    .expect("bind link port");
                outbound.push((*j, link));
            }
        } else if !links.is_empty() {
            panic!("iroh transport needs the public endpoint");
        }
        if !outbound.is_empty() {
            tokio::spawn(async move {
                let mut last = String::new();
                loop {
                    let mut line = Vec::new();
                    for (j, l) in &outbound {
                        line.push(format!("v{j}={}", l.path().await));
                    }
                    let line = line.join(" ");
                    if line != last {
                        tracing::info!(links = %line, "validator links");
                        last = line;
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });
        }

        let (mut network, mut oracle) = lookup::Network::new(context.child("network"), p2p_cfg);
        oracle.track(0, peers);
        let quota = Quota::per_second(NZU32!(256));
        let pending = network.register(0, quota);
        let recovered = network.register(1, quota);
        let resolver = network.register(2, quota);
        let broadcast = network.register(3, quota);
        let backfill = network.register(4, quota);
        let (mut tx_out, mut tx_in) = network.register(5, Quota::per_second(NZU32!(1024)));
        let (il_out, il_in) = network.register(6, Quota::per_second(NZU32!(256)));

        let scheme = Scheme::signer(&union(NAMESPACE, b"_CONSENSUS"), validators.clone(), signer.clone()).expect("key is a validator");
        let (chain, genesis) = Chain::new(cfg.clone());
        if let Some(a) = dev_censor {
            tracing::warn!(censored = %a, "DEVNET FAULT INJECTION: this validator censors a sender and ignores inclusion lists");
            chain.lock().censor = Some(a);
        }
        chain.lock().deprioritize = dev_deprioritize;
        tracing::info!(index, genesis_root = %chain.lock().finalized.state.root(), "starting validator");

        let marshal_resolver = marshal::resolver::p2p::init(
            context.child("backfill"),
            marshal::resolver::p2p::Config {
                public_key: signer.public_key(),
                peer_provider: oracle.clone(),
                blocker: oracle.clone(),
                mailbox_size: NZUsize!(1024),
                timeout: Duration::from_secs(2),
                fetch_retry_timeout: Duration::from_millis(100),
                priority_requests: false,
                priority_responses: false,
            },
            backfill,
        );
        let engine = engine::Engine::new(
            context.child("engine"),
            engine::Config {
                blocker: oracle.clone(),
                provider: oracle.clone(),
                partition_prefix: format!("aether-{index}"),
                me: signer.public_key(),
                scheme,
                genesis,
                application: Application::new(chain.clone(), block_time_ms),
                mailbox_size: 1024,
                leader_timeout: Duration::from_secs(2),
                certification_timeout: Duration::from_secs(3),
                nullify_retry: Duration::from_secs(4),
                fetch_timeout: Duration::from_secs(2),
                activity_timeout: ViewDelta::new(20),
                skip_timeout: Duration::from_secs(5),
            },
        )
        .await;
        let marshal_mailbox = engine.mailbox.clone();
        engine.start(pending, recovered, resolver, broadcast, marshal_resolver);
        network.start();

        // Mempool gossip: RPC-accepted txs go out, peers' txs come in.
        let (gossip_tx, mut gossip_rx) = tokio::sync::mpsc::unbounded_channel::<TxEnvelope>();
        tokio::spawn(async move {
            while let Some(tx) = gossip_rx.recv().await {
                let bytes = serde_json::to_vec(&tx).expect("tx serializes");
                let _ = tx_out.send(Recipients::All, bytes, false);
            }
        });
        let gossip_chain = chain.clone();
        let chain_id = cfg.chain_id;
        tokio::spawn(async move {
            while let Ok((_peer, msg)) = tx_in.recv().await {
                let Ok(tx) = serde_json::from_slice::<TxEnvelope>(msg.as_ref()) else { continue };
                if aether_execution::validate_stateless(&tx, chain_id).is_ok() {
                    gossip_chain.add_to_mempool(tx);
                }
            }
        });

        spawn_inclusion_lists(chain.clone(), signer.clone(), index, n, cfg.chain_id, Duration::from_millis(block_time_ms), il_out, il_in);

        let rpc_state = RpcState { chain, marshal: marshal_mailbox, gossip: gossip_tx };

        // Public access: iroh endpoint published to the BitTorrent Mainline DHT.
        // Wallets find this node by its id alone and verify everything they get;
        // validators tunnel consensus traffic over the same endpoint.
        let _router = endpoint.map(|ep| {
            tracing::info!(node_id = %ep.id(), "public endpoint on iroh; address published to Mainline DHT");
            let st = rpc_state.clone();
            let p2p_target = (!links.is_empty()).then(|| loopback(port));
            aether_net::serve(
                ep,
                move |req| {
                    let st = st.clone();
                    async move { rpc::handle_value(&st, req).await }
                },
                p2p_target,
            )
        });

        let rpc_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), rpc_port);
        tracing::info!(%rpc_addr, "rpc listening");
        if let Err(e) = rpc::serve(rpc_addr, rpc_state).await {
            tracing::error!(?e, "rpc server stopped");
        }
    });
}

/// FOCIL gossip: as a committee member, sign and publish the oldest waiting
/// mempool txs every block interval; verify, pool and re-gossip (once) lists
/// from other members.
#[allow(clippy::too_many_arguments)]
fn spawn_inclusion_lists<S, R>(chain: Chain, key: ed25519::PrivateKey, index: u64, n: u64, chain_id: u64, period: Duration, mut out: S, mut inbox: R)
where
    S: commonware_p2p::Sender<PublicKey = PublicKey> + 'static,
    R: commonware_p2p::Receiver<PublicKey = PublicKey> + 'static,
{
    use aether_node::inclusion::{committee, InclusionList};
    let validators: Vec<PublicKey> = (1..=n).map(|i| validator_key(i).public_key()).collect();
    let (send_tx, mut send_rx) = tokio::sync::mpsc::unbounded_channel::<InclusionList>();
    tokio::spawn(async move {
        while let Some(il) = send_rx.recv().await {
            let _ = out.send(Recipients::All, serde_json::to_vec(&il).expect("list serializes"), false);
        }
    });
    let (pub_chain, pub_send) = (chain.clone(), send_tx.clone());
    tokio::spawn(async move {
        let mut last = Vec::new();
        loop {
            tokio::time::sleep(period).await;
            let height = pub_chain.lock().finalized.height + 1;
            if !committee(height, n).contains(&index) || pub_chain.lock().censor.is_some() {
                continue;
            }
            let txs = pub_chain.inclusion_candidates(period, std::time::Instant::now());
            let hashes: Vec<_> = txs.iter().map(aether_execution::tx_hash).collect();
            if txs.is_empty() || hashes == last {
                continue;
            }
            last = hashes;
            let il = InclusionList::sign(&key, index, height, txs);
            pub_chain.lock().inclusion.accept(&il, std::time::Instant::now());
            tracing::info!(height, txs = il.txs.len(), "published inclusion list");
            let _ = pub_send.send(il);
        }
    });
    tokio::spawn(async move {
        while let Ok((_peer, msg)) = inbox.recv().await {
            let Ok(il) = serde_json::from_slice::<InclusionList>(msg.as_ref()) else { continue };
            let fin = chain.lock().finalized.height;
            if il.height + 16 < fin || il.height > fin + 16 {
                continue;
            }
            if let Err(e) = il.verify(&validators, chain_id) {
                tracing::debug!(member = il.member, ?e, "rejected inclusion list");
                continue;
            }
            let fresh = chain.lock().inclusion.accept(&il, std::time::Instant::now());
            if fresh {
                let _ = send_tx.send(il);
            }
        }
    });
}

// ---------------- client ----------------

fn call(rpc: &str, method: &str, params: Value) -> Result<Value, String> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let resp: Value =
        reqwest::blocking::Client::new().post(rpc).json(&body).send().map_err(|e| format!("rpc {rpc}: {e}"))?.json().map_err(|e| e.to_string())?;
    if let Some(err) = resp.get("error") {
        return Err(err.to_string());
    }
    Ok(resp.get("result").cloned().unwrap_or(Value::Null))
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

fn submit(rpc: &str, dev: u8, nonce: Option<u64>, c: EvmCall, wait: bool) -> Result<Value, String> {
    let signer = P256Signer::from_seed(&dev_seed(dev)).map_err(|e| e.to_string())?;
    let from = aether_crypto::address_of(&signer.public_key()).map_err(|e| e.to_string())?;
    let status = call(rpc, "aether_status", json!([]))?;
    let chain_id = status["chain_id"].as_u64().ok_or("no chain id")?;
    let nonce = match nonce {
        Some(n) => n,
        None => {
            let hex = call(rpc, "eth_getTransactionCount", json!([from]))?;
            u64::from_str_radix(hex.as_str().unwrap_or("0x0").trim_start_matches("0x"), 16).map_err(|e| e.to_string())?
        }
    };
    let tx = sign_call(&signer, chain_id, nonce, 1, &c).map_err(|e| e.to_string())?;
    let r = call(rpc, "aether_sendTransaction", json!([tx]))?;
    let hash: TxHash = serde_json::from_value(r["hash"].clone()).map_err(|e| e.to_string())?;
    println!("tx {hash}  from {from}  nonce {nonce}  (signed with P-256)");
    if !wait {
        return Ok(Value::Null);
    }
    for _ in 0..60 {
        let r = call(rpc, "aether_getReceipt", json!([hash]))?;
        if r.get("receipt").is_some() {
            let rc = &r["receipt"];
            println!("finalized in block {}  success={}  gas={}  prove_gas={}", r["height"], rc["success"], rc["gas_used"], rc["prove_gas"]);
            return Ok(r);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("timed out waiting for finalization".into())
}

/// Fetch the finalized child block H+1 that commits to the state root after H,
/// and verify its certificate against the validator set.
fn certified_anchor(rpc: &str, height: u64, validators: u64) -> Result<aether_light::VerifiedBlock, String> {
    let set = aether_light::ValidatorSet::devnet(validators);
    for _ in 0..40 {
        let v = call(rpc, "aether_getFinalized", json!([height + 1]))?;
        if !v.is_null() {
            let block = aether_light::from_hex(v["block"].as_str().unwrap_or_default()).map_err(|e| e.to_string())?;
            let fin = aether_light::from_hex(v["finalization"].as_str().unwrap_or_default()).map_err(|e| e.to_string())?;
            return aether_light::verify_finalized(&set, &block, &fin).map_err(|e| format!("CERTIFICATE REJECTED: {e} — do not trust this server"));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!("block {} not finalized yet", height + 1))
}

fn verified_balance(rpc: &str, a: Address, validators: u64) -> Result<(), String> {
    let v = call(rpc, "aether_getAccount", json!([a]))?;
    let proof: Proof = serde_json::from_value(v["proof"].clone()).map_err(|e| e.to_string())?;
    let height = v["height"].as_u64().unwrap_or_default();
    let anchor = certified_anchor(rpc, height, validators)?;
    let data = aether_light::verify_account(&anchor, &a, &proof).map_err(|e| format!("PROOF REJECTED: {e}"))?.unwrap_or_default();
    let claimed: U256 = serde_json::from_value(v["balance"].clone()).map_err(|e| e.to_string())?;
    if U256::from(data.balance) != claimed {
        return Err(format!("server claimed {claimed} but the proof says {}", data.balance));
    }
    println!("address   {a}");
    println!("balance   {} wei", data.balance);
    println!("nonce     {}", data.nonce);
    println!("verified  ✓ finality certificate of block {} checked against {} validator keys", anchor.height, validators);
    println!("          ✓ it commits state root {} (after block {height})", anchor.parent_state_root);
    println!("          ✓ EIP-7864 proof for this address verifies under that root");
    Ok(())
}

fn verified_storage(rpc: &str, a: Address, slot: U256, validators: u64) -> Result<(), String> {
    let v = call(rpc, "aether_getStorage", json!([a, slot]))?;
    let proof: Proof = serde_json::from_value(v["proof"].clone()).map_err(|e| e.to_string())?;
    let height = v["height"].as_u64().unwrap_or_default();
    let anchor = certified_anchor(rpc, height, validators)?;
    let value = aether_light::verify_storage(&anchor, &a, slot, &proof).map_err(|e| format!("PROOF REJECTED: {e}"))?;
    println!("{a}[{slot}] = {value}");
    println!("verified  ✓ finality certificate of block {} + proof under committed root {}", anchor.height, anchor.parent_state_root);
    Ok(())
}

fn print_blocks(v: &Value) {
    println!("{:>7}  {:>4}  {:>9}  {:<66}  proposer", "height", "txs", "gas", "state root after block");
    for b in v.as_array().into_iter().flatten() {
        println!(
            "{:>7}  {:>4}  {:>9}  {:<66}  {}",
            b["height"],
            b["txs"].as_array().map(|t| t.len()).unwrap_or(0),
            b["gas_used"],
            b["state_root"].as_str().unwrap_or_default(),
            b["proposer"].as_str().unwrap_or_default()
        );
    }
}
