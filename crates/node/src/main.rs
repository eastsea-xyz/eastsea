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
use aether_node::p2p::{loopback, validator_key, P2pArgs, Transport};
use aether_node::rpc::{self, RpcState};
use aether_state::Proof;
use aether_types::{Address, Bytes, GasVector, TxEnvelope, TxHash, U256};
use clap::{Parser, Subcommand};
use commonware_consensus::{marshal, types::ViewDelta};
use commonware_cryptography::{ed25519, Signer as _};
use commonware_p2p::{authenticated::lookup, AddressableManager as _, Receiver as _, Recipients, Sender as _};
use commonware_runtime::{tokio as cw_tokio, Quota, Runner as _, Supervisor as _};
use commonware_utils::{NZUsize, NZU32};
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

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
    /// Distributed key generation for the committee (run on every validator at
    /// once). Writes <data>/threshold.json with this validator's secret share
    /// and prints the committee identity that wallets pin.
    Dkg {
        #[arg(long)]
        index: u64,
        #[arg(long)]
        validators: u64,
        #[arg(long)]
        port: u16,
        #[arg(long)]
        data: String,
        #[arg(long, value_delimiter = ',')]
        peers: Vec<String>,
        #[arg(long)]
        link_base: Option<u16>,
        #[arg(long)]
        offline: bool,
        #[arg(long, default_value_t = 0)]
        round: u64,
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
        /// Committee identity to trust (hex, from `aether dkg`). Default: the devnet dealer's.
        #[arg(long)]
        identity: Option<String>,
    },
    /// Contract storage slot, verified locally with a proof.
    Storage {
        address: Address,
        slot: U256,
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        #[arg(long, default_value_t = 4)]
        validators: u64,
        #[arg(long)]
        identity: Option<String>,
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
            let p2p = p2p_args(index, validators, port, peers, link_base, offline);
            run_node(NodeArgs { p2p, rpc_port, data, block_time_ms, dev_censor, dev_deprioritize });
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
        Cmd::Balance { address, rpc, validators, identity } => trusted(validators, identity).and_then(|set| verified_balance(&rpc, address, &set)),
        Cmd::Storage { address, slot, rpc, validators, identity } => trusted(validators, identity).and_then(|set| verified_storage(&rpc, address, slot, &set)),
        Cmd::Dkg { index, validators, port, data, peers, link_base, offline, round } => {
            run_dkg(p2p_args(index, validators, port, peers, link_base, offline), data, round);
            Ok(())
        }
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

fn p2p_args(index: u64, n: u64, port: u16, peers: Vec<String>, link_base: Option<u16>, offline: bool) -> P2pArgs {
    let transport = if peers.iter().any(|p| !p.is_empty()) {
        Transport::Tcp(peers)
    } else {
        Transport::Iroh { link_base: link_base.unwrap_or(20_000 + 100 * index as u16) }
    };
    P2pArgs { index, n, port, transport, offline, max_message: MAX_BLOCK_BYTES + 1024 * 1024 }
}

struct NodeArgs {
    p2p: P2pArgs,
    rpc_port: u16,
    data: String,
    block_time_ms: u64,
    dev_censor: Option<Address>,
    dev_deprioritize: Option<Address>,
}

fn run_node(a: NodeArgs) {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,commonware=warn".into())).init();
    let NodeArgs { p2p, rpc_port, block_time_ms, dev_censor, dev_deprioritize, data } = a;
    let (index, n, port) = (p2p.index, p2p.n, p2p.port);
    assert!(!p2p.offline || matches!(p2p.transport, Transport::Tcp(_)), "--offline needs --peers");
    let signer = validator_key(index);
    let peers = aether_node::p2p::peer_addresses(&p2p);
    let p2p_cfg = aether_node::p2p::config(&p2p, b"_P2P");
    let links = matches!(p2p.transport, Transport::Iroh { .. });
    let executor = cw_tokio::Runner::new(cw_tokio::Config::new().with_storage_directory(&data));
    let cfg = chain_config();

    executor.start(async move |context| {
        // Public endpoint first: validator links and wallet RPC share it.
        let endpoint = aether_node::p2p::open_public(&p2p).await;
        if links && endpoint.is_none() {
            panic!("iroh transport needs the public endpoint");
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

        // BLS threshold certificates (one group signature per block) with a VRF
        // seed per round for leader election. Devnet shares come from a fixed
        // dealer seed; a real network derives them with a DKG.
        let (participants, polynomial, share) = committee_keys(&data, n, &signer.public_key());
        let scheme = aether_light::Scheme::signer(&aether_light::consensus_namespace(), participants, polynomial, share).expect("share matches polynomial");
        let store = aether_node::store::Store::open(&std::path::Path::new(&data).join("state.redb")).expect("open state store");
        let (chain, genesis) = Chain::open(cfg.clone(), store).expect("restore state (delete the data dir to resync)");
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
            let p2p_target = links.then(|| loopback(port));
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

fn run_dkg(p2p: P2pArgs, data: String, round: u64) {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,commonware=warn".into())).init();
    let dir = std::path::PathBuf::from(&data);
    std::fs::create_dir_all(&dir).expect("data dir");
    let out_path = dir.join("threshold.json");
    let executor = cw_tokio::Runner::new(cw_tokio::Config::new().with_storage_directory(dir.join("dkg-runtime")));
    let (p2p_n, dir_out) = (p2p.n, dir.clone());
    let result = executor.start(async move |context| {
        // Accept incoming validator links (the node's RPC is not needed here).
        let _router = aether_node::p2p::open_public(&p2p).await.map(|ep| aether_net::serve_p2p(ep, loopback(p2p.port)));
        let (mut network, mut oracle) = lookup::Network::new(context.child("network"), aether_node::p2p::config(&p2p, b"_DKG"));
        oracle.track(0, aether_node::p2p::peer_addresses(&p2p));
        let (sender, receiver) = network.register(0, Quota::per_second(NZU32!(256)));
        network.start();
        tracing::info!(index = p2p.index, n = p2p.n, round, "dkg: started");
        aether_node::dkg::run(validator_key(p2p.index), aether_node::p2p::validators(p2p.n), round, sender, receiver, Default::default()).await
    });
    match result {
        Ok((output, share)) => {
            let file = aether_node::dkg::KeyFile::new(round, &output, &share);
            write_secret(&out_path, &serde_json::to_vec_pretty(&file).expect("key file serializes"));
            let public = json!({ "validators": p2p_n, "round": round, "identity": file.identity });
            std::fs::write(dir_out.join("network.json"), serde_json::to_vec_pretty(&public).expect("json")).expect("write network.json");
            println!("committee identity: {}", file.identity);
            println!("secret share written to {} (mode 600)", out_path.display());
        }
        Err(e) => {
            eprintln!("dkg failed: {e}");
            std::process::exit(1);
        }
    }
}

/// This validator's threshold share: from `<data>/threshold.json` (DKG) when
/// present, else the devnet dealer's (insecure: the dealer knows every share).
fn committee_keys(
    data: &str,
    n: u64,
    me: &PublicKey,
) -> (
    commonware_utils::ordered::Set<PublicKey>,
    commonware_cryptography::bls12381::primitives::sharing::Sharing<commonware_cryptography::bls12381::primitives::variant::MinSig>,
    commonware_cryptography::bls12381::primitives::group::Share,
) {
    let path = std::path::Path::new(data).join("threshold.json");
    if let Ok(bytes) = std::fs::read(&path) {
        let file: aether_node::dkg::KeyFile = serde_json::from_slice(&bytes).expect("threshold.json");
        let (output, share) = file.decode(n as u32).expect("threshold.json decodes");
        assert_eq!(output.players(), &aether_node::p2p::validators(n), "threshold.json is for a different validator set");
        tracing::info!(identity = %file.identity, "committee key from DKG");
        return (output.players().clone(), output.public().clone(), share);
    }
    tracing::warn!("no threshold.json: using the devnet dealer's shares (every share is public knowledge)");
    let (participants, polynomial, shares) = aether_light::devnet_threshold(n);
    let share = shares.into_iter().find(|(pk, _)| pk == me).map(|(_, s)| s).expect("key is a validator");
    (participants, polynomial, share)
}

fn write_secret(path: &std::path::Path, bytes: &[u8]) {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path).expect("open key file");
    f.write_all(bytes).expect("write key file");
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
fn trusted(validators: u64, identity: Option<String>) -> Result<aether_light::ValidatorSet, String> {
    match identity {
        Some(hex) => aether_light::ValidatorSet::from_hex(&hex).map_err(|e| format!("identity: {e}")),
        None => Ok(aether_light::ValidatorSet::devnet(validators)),
    }
}

fn certified_anchor(rpc: &str, height: u64, set: &aether_light::ValidatorSet) -> Result<aether_light::VerifiedBlock, String> {
    for _ in 0..40 {
        let v = call(rpc, "aether_getFinalized", json!([height + 1]))?;
        if !v.is_null() {
            let block = aether_light::from_hex(v["block"].as_str().unwrap_or_default()).map_err(|e| e.to_string())?;
            let fin = aether_light::from_hex(v["finalization"].as_str().unwrap_or_default()).map_err(|e| e.to_string())?;
            return aether_light::verify_finalized(set, &block, &fin).map_err(|e| format!("CERTIFICATE REJECTED: {e} — do not trust this server"));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!("block {} not finalized yet", height + 1))
}

fn verified_balance(rpc: &str, a: Address, set: &aether_light::ValidatorSet) -> Result<(), String> {
    let v = call(rpc, "aether_getAccount", json!([a]))?;
    let proof: Proof = serde_json::from_value(v["proof"].clone()).map_err(|e| e.to_string())?;
    let height = v["height"].as_u64().unwrap_or_default();
    let anchor = certified_anchor(rpc, height, set)?;
    let data = aether_light::verify_account(&anchor, &a, &proof).map_err(|e| format!("PROOF REJECTED: {e}"))?.unwrap_or_default();
    let claimed: U256 = serde_json::from_value(v["balance"].clone()).map_err(|e| e.to_string())?;
    if U256::from(data.balance) != claimed {
        return Err(format!("server claimed {claimed} but the proof says {}", data.balance));
    }
    println!("address   {a}");
    println!("balance   {} wei", data.balance);
    println!("nonce     {}", data.nonce);
    println!("verified  ✓ finality certificate of block {}: one BLS threshold signature under committee key {}…", anchor.height, &set.identity_hex()[..16]);
    println!("          ✓ it commits state root {} (after block {height})", anchor.parent_state_root);
    println!("          ✓ EIP-7864 proof for this address verifies under that root");
    Ok(())
}

fn verified_storage(rpc: &str, a: Address, slot: U256, set: &aether_light::ValidatorSet) -> Result<(), String> {
    let v = call(rpc, "aether_getStorage", json!([a, slot]))?;
    let proof: Proof = serde_json::from_value(v["proof"].clone()).map_err(|e| e.to_string())?;
    let height = v["height"].as_u64().unwrap_or_default();
    let anchor = certified_anchor(rpc, height, set)?;
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
