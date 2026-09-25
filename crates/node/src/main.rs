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
use aether_state::layout::BasicData;
use aether_state::Proof;
use aether_types::{Address, Bytes, GasVector, TxEnvelope, TxHash, B256, U256};
use clap::{Parser, Subcommand};
use commonware_consensus::{marshal, simplex::scheme::ed25519::Scheme, types::ViewDelta};
use commonware_cryptography::{ed25519, Signer as _};
use commonware_p2p::{authenticated::{self, discovery}, Manager as _, Receiver as _, Recipients, Sender as _};
use commonware_runtime::{tokio as cw_tokio, Quota, Runner as _, Supervisor as _};
use commonware_utils::{ordered::Set, union, NZUsize, TryCollect, NZU32};
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

const NAMESPACE: &[u8] = b"_AETHER_DEVNET_V1";
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
        /// Other validators: `<index>@<host:port>`, comma separated.
        #[arg(long, value_delimiter = ',')]
        bootstrap: Vec<String>,
        #[arg(long, default_value_t = 1000)]
        block_time_ms: u64,
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
    },
    /// Contract storage slot, verified locally with a proof.
    Storage {
        address: Address,
        slot: U256,
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
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
        Cmd::Node { index, validators, port, rpc_port, data, bootstrap, block_time_ms } => {
            run_node(index, validators, port, rpc_port, data, bootstrap, block_time_ms);
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
        Cmd::Balance { address, rpc } => verified_balance(&rpc, address),
        Cmd::Storage { address, slot, rpc } => verified_storage(&rpc, address, slot),
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
    ed25519::PrivateKey::from_seed(i)
}

#[allow(clippy::too_many_arguments)]
fn run_node(index: u64, n: u64, port: u16, rpc_port: u16, data: String, bootstrap: Vec<String>, block_time_ms: u64) {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,commonware=warn".into()))
        .init();
    let signer = validator_key(index);
    let validators: Set<PublicKey> = (1..=n).map(|i| validator_key(i).public_key()).try_collect().expect("unique validator keys");
    let bootstrappers: Vec<_> = bootstrap
        .iter()
        .filter(|s| !s.is_empty())
        .map(|b| {
            let (i, addr) = b.split_once('@').expect("bootstrap is <index>@<host:port>");
            let addr: SocketAddr = addr.parse().expect("bootstrap address");
            (validator_key(i.parse().expect("bootstrap index")).public_key(), addr.into())
        })
        .collect();
    let max_peers = authenticated::peer_set_limit(&validators, &signer.public_key());
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let p2p_cfg = discovery::Config::local(
        signer.clone(),
        &union(NAMESPACE, b"_P2P"),
        listen,
        listen,
        bootstrappers,
        max_peers,
        MAX_BLOCK_BYTES + 1024 * 1024,
    );
    let executor = cw_tokio::Runner::new(cw_tokio::Config::new().with_storage_directory(&data));
    let cfg = chain_config();

    executor.start(async move |context| {
        let (mut network, mut oracle) = discovery::Network::new(context.child("network"), p2p_cfg);
        oracle.track(0, validators.clone());
        let quota = Quota::per_second(NZU32!(256));
        let pending = network.register(0, quota);
        let recovered = network.register(1, quota);
        let resolver = network.register(2, quota);
        let broadcast = network.register(3, quota);
        let backfill = network.register(4, quota);
        let (mut tx_out, mut tx_in) = network.register(5, Quota::per_second(NZU32!(1024)));

        let scheme = Scheme::signer(&union(NAMESPACE, b"_CONSENSUS"), validators.clone(), signer.clone()).expect("key is a validator");
        let (chain, genesis) = Chain::new(cfg.clone());
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

        let rpc_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), rpc_port);
        tracing::info!(%rpc_addr, "rpc listening");
        if let Err(e) = rpc::serve(rpc_addr, RpcState { chain, gossip: gossip_tx }).await {
            tracing::error!(?e, "rpc server stopped");
        }
    });
}

// ---------------- client ----------------

fn call(rpc: &str, method: &str, params: Value) -> Result<Value, String> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let resp: Value = reqwest::blocking::Client::new()
        .post(rpc)
        .json(&body)
        .send()
        .map_err(|e| format!("rpc {rpc}: {e}"))?
        .json()
        .map_err(|e| e.to_string())?;
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
            println!(
                "finalized in block {}  success={}  gas={}  prove_gas={}",
                r["height"], rc["success"], rc["gas_used"], rc["prove_gas"]
            );
            return Ok(r);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("timed out waiting for finalization".into())
}

fn verify_proof(v: &Value) -> Result<(Proof, B256, u64), String> {
    let proof: Proof = serde_json::from_value(v["proof"].clone()).map_err(|e| e.to_string())?;
    let root: B256 = serde_json::from_value(v["state_root"].clone()).map_err(|e| e.to_string())?;
    let height = v["height"].as_u64().unwrap_or_default();
    proof
        .verify(&aether_execution::ChainHasher::new(), &root.0)
        .map_err(|e| format!("PROOF REJECTED: {e:?} — do not trust this server"))?;
    Ok((proof, root, height))
}

fn verified_balance(rpc: &str, a: Address) -> Result<(), String> {
    let v = call(rpc, "aether_getAccount", json!([a]))?;
    let (proof, root, height) = verify_proof(&v)?;
    let data = proof.value.map(|x| BasicData::decode(&x)).unwrap_or_default();
    let claimed: U256 = serde_json::from_value(v["balance"].clone()).map_err(|e| e.to_string())?;
    if U256::from(data.balance) != claimed {
        return Err(format!("server claimed {claimed} but the proof says {}", data.balance));
    }
    println!("address   {a}");
    println!("balance   {} wei", data.balance);
    println!("nonce     {}", data.nonce);
    println!("verified  ✓ EIP-7864 proof ({} path nodes) against state root {root} at height {height}", proof.stem_path.len());
    Ok(())
}

fn verified_storage(rpc: &str, a: Address, slot: U256) -> Result<(), String> {
    let v = call(rpc, "aether_getStorage", json!([a, slot]))?;
    let (proof, root, height) = verify_proof(&v)?;
    let value = proof.value.map(U256::from_be_bytes).unwrap_or_default();
    println!("{a}[{slot}] = {value}");
    println!("verified  ✓ proof against state root {root} at height {height}");
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
