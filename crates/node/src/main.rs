//! `aether` — validator node and client.
//!
//!   aether node --index 1 --validators 4 --port 9001 --rpc-port 8545 --data /tmp/aether/1
//!   aether node --index 2 --validators 4 --port 9002 --rpc-port 8546 --data /tmp/aether/2 --bootstrap 1@127.0.0.1:9001
//!   aether send --from-dev 1 --to 0x… --value 1000 --wait
//!   aether balance 0x…              # fetches an EIP-7864 proof and verifies it locally

use aether_crypto::{P256Signer, Signer};
use aether_execution::{sign_call_with, EvmCall};
use aether_node::application::Application;
use aether_node::block::PublicKey;
use aether_node::chain::{dev_accounts, dev_seed, Chain, ChainConfig};
use aether_node::engine::{self, MAX_BLOCK_BYTES};
use aether_node::p2p::{loopback, P2pArgs, Transport};
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
        /// Devnet: this validator's index (1-based). With --network, derived from the local key.
        #[arg(long)]
        index: Option<u64>,
        /// Devnet: number of validators. With --network, taken from the file.
        #[arg(long)]
        validators: Option<u64>,
        /// network.json (validator keys and node ids). Keys come from <data>/validator.key.
        #[arg(long)]
        network: Option<String>,
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
        index: Option<u64>,
        #[arg(long)]
        validators: Option<u64>,
        #[arg(long)]
        network: Option<String>,
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
    /// Move the committee key to a new validator set (run on every old and new
    /// validator at once). Identity is unchanged, shares are fresh; departing
    /// validators end with no share. Needs a quorum of the old validators online.
    Reshare {
        /// Current network.json (with identity and output, as written by dkg/reshare).
        #[arg(long)]
        from: String,
        /// The next validator set (network.json without identity).
        #[arg(long)]
        to: String,
        /// Last height the current committee finalized (it stops there); the new
        /// committee's epoch starts at the next height.
        #[arg(long)]
        epoch_end: u64,
        /// Hash of that block (`aether blocks`), which the new epoch builds on.
        #[arg(long)]
        epoch_end_hash: String,
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
    },
    /// Last finalized height and block hash in a (stopped) node's data dir.
    Head {
        #[arg(long)]
        data: String,
    },
    /// Generate this validator's keys in <data> (never overwrites). Prints the public entry.
    Keygen {
        #[arg(long)]
        data: String,
    },
    /// Assemble network.json from validators' validator.pub.json files (in validator order).
    Network {
        #[arg(long, default_value_t = DEFAULT_CHAIN_ID)]
        chain_id: u64,
        members: Vec<String>,
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
    /// Pay several addresses in ONE signed tx (EIP-7702 delegation to AetherAccount).
    Batch {
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        #[arg(long)]
        from_dev: u8,
        /// Recipients, comma separated; each receives --value.
        #[arg(long, value_delimiter = ',')]
        to: Vec<Address>,
        #[arg(long)]
        value: U256,
        #[arg(long)]
        wait: bool,
    },
    /// Make dev account `guardian_dev`'s key the recovery key of `from_dev`.
    SetGuardian {
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        #[arg(long)]
        from_dev: u8,
        #[arg(long)]
        guardian_dev: u8,
    },
    /// As `guardian_dev`, sweep the lost account's verified balance to itself
    /// (guardian signs with P-256; it also relays and pays gas).
    Recover {
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        #[arg(long)]
        guardian_dev: u8,
        #[arg(long)]
        lost: Address,
        #[arg(long, default_value_t = 4)]
        validators: u64,
        #[arg(long)]
        identity: Option<String>,
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
        Cmd::Node { index, validators, network, port, rpc_port, data, peers, link_base, offline, block_time_ms, dev_censor, dev_deprioritize } => {
            let with_file = network.is_some();
            p2p_args(index, validators, network, &data, port, peers, link_base, offline)
                .and_then(|args| {
                    if with_file && args.3.is_none() {
                        return Err("network.json has no committee identity: run the node with the network.json written by dkg/reshare".into());
                    }
                    Ok(args)
                })
                .map(|(p2p, chain_id, epochs, key_round)| {
                    run_node(NodeArgs { p2p, chain_id, epochs, key_round, rpc_port, data, block_time_ms, dev_censor, dev_deprioritize });
                })
        }
        Cmd::Keygen { data } => keygen(&data),
        Cmd::Head { data } => (|| {
            let store = aether_node::store::Store::open(&std::path::Path::new(&data).join("state.redb")).map_err(|e| e.to_string())?;
            let (h, d) = store.head().map_err(|e| e.to_string())?.ok_or("no finalized state")?;
            println!("{h} {}", hex::encode(d));
            Ok(())
        })(),
        Cmd::Reshare { from, to, epoch_end, epoch_end_hash, port, data, peers, link_base, offline } => {
            let boundary = aether_node::roster::EpochStart { height: epoch_end + 1, parent: epoch_end_hash };
            reshare(&from, &to, boundary, port, data, peers, link_base, offline)
        }
        Cmd::Network { chain_id, members } => assemble_network(chain_id, &members),
        Cmd::DevAccounts => {
            for (i, a) in dev_accounts(DEV_ACCOUNTS) {
                println!("dev {i:>2}  {a}");
            }
            Ok(())
        }
        Cmd::Status { rpc } => call(&rpc, "aether_status", json!([])).map(|v| println!("{}", pretty(&v))),
        Cmd::Blocks { rpc, n } => call(&rpc, "aether_recentBlocks", json!([n])).map(|v| print_blocks(&v)),
        Cmd::Send { rpc, from_dev, to, value, nonce, wait } => {
            submit(&rpc, from_dev, nonce, EvmCall { to: Some(to), value, input: Bytes::new(), gas_limit: 21_000, delegate: None }, wait).map(|_| ())
        }
        Cmd::Batch { rpc, from_dev, to, value, wait } => (|| {
            let signer = P256Signer::from_seed(&dev_seed(from_dev)).map_err(|e| e.to_string())?;
            let from = aether_crypto::address_of(&signer.public_key()).map_err(|e| e.to_string())?;
            let code = call(&rpc, "eth_getCode", json!([from]))?;
            let designator = format!("0xef0100{}", hex::encode(aether_execution::AETHER_ACCOUNT.as_slice()));
            let delegated = code.as_str().is_some_and(|c| c.eq_ignore_ascii_case(&designator));
            let calls: Vec<_> = to.iter().map(|a| (*a, value, Bytes::new())).collect();
            let c = EvmCall {
                to: Some(from),
                value: U256::ZERO,
                input: aether_execution::encode_execute(&calls),
                gas_limit: 60_000 + 40_000 * calls.len() as u64,
                delegate: (!delegated).then_some(aether_execution::AETHER_ACCOUNT),
            };
            submit(&rpc, from_dev, None, c, wait).map(|_| ())
        })(),
        Cmd::SetGuardian { rpc, from_dev, guardian_dev } => (|| {
            let guardian = P256Signer::from_seed(&dev_seed(guardian_dev)).map_err(|e| e.to_string())?;
            let (x, y) = aether_crypto::p256_xy(&guardian.public_key().bytes).map_err(|e| format!("{e:?}"))?;
            let me = dev_address(from_dev)?;
            let c = EvmCall {
                to: Some(me),
                value: U256::ZERO,
                input: aether_execution::encode_execute(&[(me, U256::ZERO, aether_execution::encode_set_guardian(x, y))]),
                gas_limit: 200_000,
                delegate: (!is_delegated(&rpc, me)?).then_some(aether_execution::AETHER_ACCOUNT),
            };
            submit(&rpc, from_dev, None, c, true).map(|_| ())
        })(),
        Cmd::Recover { rpc, guardian_dev, lost, validators, identity } => (|| {
            let set = trusted(validators, identity)?;
            let guardian = P256Signer::from_seed(&dev_seed(guardian_dev)).map_err(|e| e.to_string())?;
            let me = dev_address(guardian_dev)?;
            // Balance and guardian nonce, both proven against a certified root.
            let v = call(&rpc, "aether_getAccount", json!([lost]))?;
            let proof: Proof = serde_json::from_value(v["proof"].clone()).map_err(|e| e.to_string())?;
            let anchor = certified_anchor(&rpc, v["height"].as_u64().unwrap_or_default(), &set)?;
            let balance = U256::from(aether_light::verify_account(&anchor, &lost, &proof).map_err(|e| e.to_string())?.unwrap_or_default().balance);
            let slot = U256::from_str_radix(GUARDIAN_SLOT, 16).expect("slot") + U256::from(2u64);
            let v = call(&rpc, "aether_getStorage", json!([lost, slot]))?;
            let proof: Proof = serde_json::from_value(v["proof"].clone()).map_err(|e| e.to_string())?;
            let anchor = certified_anchor(&rpc, v["height"].as_u64().unwrap_or_default(), &set)?;
            let nonce = aether_light::verify_storage(&anchor, &lost, slot, &proof).map_err(|e| e.to_string())?.to::<u64>();
            let chain_id = call(&rpc, "aether_status", json!([]))?["chain_id"].as_u64().unwrap_or_default();
            let calls = [(me, balance, Bytes::new())];
            let sig = guardian.sign(&aether_execution::guardian_message(chain_id, lost, nonce, &calls)).map_err(|e| format!("{e:?}"))?;
            let (r, s): ([u8; 32], [u8; 32]) = (sig[..32].try_into().expect("32"), sig[32..64].try_into().expect("32"));
            println!("recovering {balance} wei from {lost} (guardian nonce {nonce})");
            let c = EvmCall {
                to: Some(lost),
                value: U256::ZERO,
                input: aether_execution::encode_guardian_execute(&calls, r, s),
                gas_limit: 250_000,
                delegate: None,
            };
            submit(&rpc, guardian_dev, None, c, true).map(|_| ())
        })(),
        Cmd::Deploy { rpc, from_dev, code } => (|| {
            let input = Bytes::from(hex::decode(code.trim_start_matches("0x")).map_err(|e| e.to_string())?);
            let r = submit(&rpc, from_dev, None, EvmCall { to: None, value: U256::ZERO, input, gas_limit: 3_000_000, delegate: None }, true)?;
            if let Some(a) = r.pointer("/receipt/contract_address") {
                println!("contract: {}", a.as_str().unwrap_or_default());
            }
            Ok(())
        })(),
        Cmd::Call { rpc, from_dev, to, data, wait } => (|| {
            let input = Bytes::from(hex::decode(data.trim_start_matches("0x")).map_err(|e| e.to_string())?);
            submit(&rpc, from_dev, None, EvmCall { to: Some(to), value: U256::ZERO, input, gas_limit: 1_000_000, delegate: None }, wait).map(|_| ())
        })(),
        Cmd::Balance { address, rpc, validators, identity } => trusted(validators, identity).and_then(|set| verified_balance(&rpc, address, &set)),
        Cmd::Storage { address, slot, rpc, validators, identity } => trusted(validators, identity).and_then(|set| verified_storage(&rpc, address, slot, &set)),
        Cmd::Dkg { index, validators, network, port, data, peers, link_base, offline, round } => {
            p2p_args(index, validators, network, &data, port, peers, link_base, offline).map(|(p2p, chain_id, _, _)| run_dkg(p2p, chain_id, data, round))
        }
        Cmd::Receipt { hash, rpc } => call(&rpc, "aether_getReceipt", json!([hash])).map(|v| println!("{}", pretty(&v))),
    };
    if let Err(e) = res {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn chain_config(chain_id: u64) -> ChainConfig {
    ChainConfig {
        chain_id,
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        alloc: dev_accounts(DEV_ACCOUNTS).into_iter().map(|(_, a)| (a, U256::from(DEV_BALANCE))).collect(),
        fees: true,
    }
}

/// Who we are and who the others are: from --network + <data>/validator.key,
/// or the public devnet keys (--index/--validators).
#[allow(clippy::too_many_arguments)]
fn p2p_args(
    index: Option<u64>,
    n: Option<u64>,
    network: Option<String>,
    data: &str,
    port: u16,
    peers: Vec<String>,
    link_base: Option<u16>,
    offline: bool,
) -> Result<(P2pArgs, u64, Vec<aether_node::roster::EpochStart>, Option<u64>), String> {
    use aether_node::roster::{LocalKeys, NetworkFile, Roster};
    let (roster, keys, index, chain_id, epochs, round) = match network {
        Some(path) => {
            let file = NetworkFile::load(std::path::Path::new(&path))?;
            let roster = Roster::from_file(&file)?;
            let keys = LocalKeys::load(std::path::Path::new(data))?;
            let index = roster.index_of(&keys.signer.public_key()).ok_or("this machine's validator key is not in network.json")?;
            // A network file with an identity names the key round its shares must be from.
            let round = file.identity.as_ref().map(|_| file.round);
            (roster, keys, index, file.chain_id, file.epochs, round)
        }
        None => {
            let (index, n) = (index.ok_or("--index (or --network)")?, n.ok_or("--validators (or --network)")?);
            (Roster::devnet(n), LocalKeys::devnet(index), index, DEFAULT_CHAIN_ID, vec![], None)
        }
    };
    let transport = if peers.iter().any(|p| !p.is_empty()) {
        Transport::Tcp(peers)
    } else {
        Transport::Iroh { link_base: link_base.unwrap_or(20_000 + 100 * index as u16) }
    };
    let n = roster.len();
    Ok((P2pArgs { index, n, roster, keys, port, transport, offline, max_message: MAX_BLOCK_BYTES + 1024 * 1024 }, chain_id, epochs, round))
}

/// Reshare: p2p over the union of both validator sets; old members deal with
/// their current share, new members receive.
#[allow(clippy::too_many_arguments)]
fn reshare(
    from: &str,
    to: &str,
    boundary: aether_node::roster::EpochStart,
    port: u16,
    data: String,
    peers: Vec<String>,
    link_base: Option<u16>,
    offline: bool,
) -> Result<(), String> {
    use aether_node::roster::{LocalKeys, NetworkFile, Roster};
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,commonware=warn".into())).init();
    let old_file = NetworkFile::load(std::path::Path::new(from))?;
    let new_file = NetworkFile::load(std::path::Path::new(to))?;
    let (old, new) = (Roster::from_file(&old_file)?, Roster::from_file(&new_file)?);
    let output_hex = old_file.output.clone().ok_or("--from has no committee output: use the network.json written by dkg/reshare")?;
    let n_old = old.len() as u32;
    let previous = aether_node::dkg::KeyFile { round: old_file.round, output: output_hex, identity: String::new(), share: String::new() };
    let previous = previous.decode_output(n_old)?;
    let dir = std::path::PathBuf::from(&data);
    let keys = LocalKeys::load(&dir)?;
    let share = match std::fs::read(dir.join("threshold.json")) {
        Ok(b) if old.index_of(&keys.signer.public_key()).is_some() => {
            let f: aether_node::dkg::KeyFile = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
            Some(f.decode(n_old)?.1)
        }
        _ => None,
    };
    let union = old.union(&new);
    let index = union.index_of(&keys.signer.public_key()).ok_or("this machine's key is in neither validator set")?;
    let transport = if peers.iter().any(|p| !p.is_empty()) {
        Transport::Tcp(peers)
    } else {
        Transport::Iroh { link_base: link_base.unwrap_or(20_000 + 100 * index as u16) }
    };
    let p2p = P2pArgs { index, n: union.len(), roster: union, keys: keys.clone(), port, transport, offline, max_message: MAX_BLOCK_BYTES + 1024 * 1024 };
    let round = aether_node::dkg::Round::reshare(previous, new.validators(), old_file.round + 1);
    let next_round = round.round;
    let executor = cw_tokio::Runner::new(cw_tokio::Config::new().with_storage_directory(dir.join("reshare-runtime")));
    let result = executor.start(async move |context| {
        let _router = aether_node::p2p::open_public(&p2p).await.map(|ep| aether_net::serve_p2p(ep, loopback(p2p.port)));
        let (mut network, mut oracle) = lookup::Network::new(context.child("network"), aether_node::p2p::config(&p2p, b"_DKG"));
        oracle.track(0, aether_node::p2p::peer_addresses(&p2p));
        let (sender, receiver) = network.register(0, Quota::per_second(NZU32!(256)));
        network.start();
        tracing::info!(index = p2p.index, round = next_round, "reshare: started");
        aether_node::dkg::run(p2p.keys.signer.clone(), round, share, sender, receiver, Default::default()).await
    });
    match result.map_err(|e| format!("reshare failed: {e}"))? {
        Some((output, share)) => {
            let file = aether_node::dkg::KeyFile::new(next_round, &output, &share);
            write_secret(&dir.join("threshold.json"), &serde_json::to_vec_pretty(&file).expect("json"));
            let mut public = new_file.clone();
            public.epochs = old_file.epochs.clone();
            public.epochs.push(boundary);
            public.identity = Some(file.identity.clone());
            public.round = next_round;
            public.output = Some(file.output.clone());
            std::fs::write(dir.join("network.json"), serde_json::to_vec_pretty(&public).expect("json")).map_err(|e| e.to_string())?;
            println!("committee identity: {} (unchanged)", file.identity);
            println!("new share written to {} (mode 600)", dir.join("threshold.json").display());
        }
        None => {
            // A departing validator: its old share is now useless; remove it.
            let _ = std::fs::remove_file(dir.join("threshold.json"));
            println!("dealt our share to the new committee; this validator has left it");
        }
    }
    Ok(())
}

/// Storage partition prefix for this data dir, fixed at first use so it does
/// not change when the validator's index in a new committee changes. Older
/// data dirs (named by index) keep their existing prefix.
fn partition_prefix(data: &str) -> String {
    let dir = std::path::Path::new(data);
    let marker = dir.join("partition");
    if let Ok(p) = std::fs::read_to_string(&marker) {
        return p.trim().to_string();
    }
    let existing = std::fs::read_dir(dir).ok().and_then(|entries| {
        entries.filter_map(|e| e.ok()?.file_name().into_string().ok()).find_map(|name| name.strip_suffix("-blocks-metadata").map(str::to_string))
    });
    let prefix = existing.unwrap_or_else(|| "aether".to_string());
    let _ = std::fs::write(&marker, &prefix);
    prefix
}

fn keygen(data: &str) -> Result<(), String> {
    let dir = std::path::Path::new(data);
    let keys = aether_node::roster::LocalKeys::generate();
    keys.save(dir)?;
    println!("{}", serde_json::to_string_pretty(&keys.public()).expect("json"));
    println!(
        "secret keys in {} (mode 600); share only {}",
        dir.join(aether_node::roster::KEY_FILE).display(),
        dir.join(aether_node::roster::PUBLIC_FILE).display()
    );
    Ok(())
}

/// Combine validators' public entries (validator.pub.json files) into network.json on stdout.
fn assemble_network(chain_id: u64, members: &[String]) -> Result<(), String> {
    let validators = members
        .iter()
        .map(|p| std::fs::read(p).map_err(|e| format!("{p}: {e}")).and_then(|b| serde_json::from_slice(&b).map_err(|e| format!("{p}: {e}"))))
        .collect::<Result<Vec<aether_node::roster::Member>, String>>()?;
    let file = aether_node::roster::NetworkFile { chain_id, validators, identity: None, round: 0, output: None, epochs: vec![] };
    aether_node::roster::Roster::from_file(&file)?;
    println!("{}", serde_json::to_string_pretty(&file).expect("json"));
    Ok(())
}

struct NodeArgs {
    p2p: P2pArgs,
    chain_id: u64,
    epochs: Vec<aether_node::roster::EpochStart>,
    /// Key round the network file expects (from its identity), if any.
    key_round: Option<u64>,
    rpc_port: u16,
    data: String,
    block_time_ms: u64,
    dev_censor: Option<Address>,
    dev_deprioritize: Option<Address>,
}

fn run_node(a: NodeArgs) {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,commonware=warn".into())).init();
    let NodeArgs { p2p, chain_id, epochs, key_round, rpc_port, block_time_ms, dev_censor, dev_deprioritize, data } = a;
    let (index, port) = (p2p.index, p2p.port);
    assert!(!p2p.offline || matches!(p2p.transport, Transport::Tcp(_)), "--offline needs --peers");
    let signer = p2p.keys.signer.clone();
    let (roster_keys, validator_set) = (p2p.roster.keys.clone(), p2p.validators());
    let peers = aether_node::p2p::peer_addresses(&p2p);
    let p2p_cfg = aether_node::p2p::config(&p2p, b"_P2P");
    let links = matches!(p2p.transport, Transport::Iroh { .. });
    let executor = cw_tokio::Runner::new(cw_tokio::Config::new().with_storage_directory(&data));
    let cfg = chain_config(chain_id);

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
        let (participants, polynomial, share) = committee_keys(&data, &validator_set, &signer.public_key(), key_round);
        let polynomial_identity = &polynomial.public().clone();
        let scheme = aether_light::Scheme::signer(&aether_light::consensus_namespace(), participants, polynomial, share).expect("share matches polynomial");
        let store = aether_node::store::Store::open(&std::path::Path::new(&data).join("state.redb")).expect("open state store");
        let (chain, genesis) = Chain::open(cfg.clone(), store).expect("restore state (delete the data dir to resync)");
        // A later epoch starts on the old committee's last block; it must be ours too.
        let epoch_floor = match epochs.last() {
            None => None,
            Some(e) => {
                use commonware_codec::DecodeExt;
                let d =
                    hex::decode(&e.parent).ok().and_then(|b| commonware_cryptography::sha256::Digest::decode(b.as_slice()).ok()).expect("epoch parent hash");
                let g = chain.lock();
                if let Some(ours) = g.blocks.get(&(e.height - 1)) {
                    assert_eq!(ours.hash, format!("{d}"), "epoch parent differs from our finalized block {}", e.height - 1);
                }
                assert!(g.finalized.height < e.height, "this node finalized past the epoch boundary {}: wrong --epoch-end", e.height - 1);
                Some(d)
            }
        };
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
                partition_prefix: partition_prefix(&data),
                me: signer.public_key(),
                scheme,
                identity: *polynomial_identity,
                epocher: aether_node::epochs::ScheduleEpocher::new(epochs.iter().map(|e| e.height).collect()),
                epoch_floor,
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

        spawn_inclusion_lists(chain.clone(), signer.clone(), index, roster_keys, cfg.chain_id, Duration::from_millis(block_time_ms), il_out, il_in);

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

fn run_dkg(p2p: P2pArgs, chain_id: u64, data: String, round: u64) {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,commonware=warn".into())).init();
    let dir = std::path::PathBuf::from(&data);
    std::fs::create_dir_all(&dir).expect("data dir");
    let out_path = dir.join("threshold.json");
    let executor = cw_tokio::Runner::new(cw_tokio::Config::new().with_storage_directory(dir.join("dkg-runtime")));
    let (mut public, dir_out) = (p2p.roster.to_file(chain_id), dir.clone());
    let result = executor.start(async move |context| {
        // Accept incoming validator links (the node's RPC is not needed here).
        let _router = aether_node::p2p::open_public(&p2p).await.map(|ep| aether_net::serve_p2p(ep, loopback(p2p.port)));
        let (mut network, mut oracle) = lookup::Network::new(context.child("network"), aether_node::p2p::config(&p2p, b"_DKG"));
        oracle.track(0, aether_node::p2p::peer_addresses(&p2p));
        let (sender, receiver) = network.register(0, Quota::per_second(NZU32!(256)));
        network.start();
        tracing::info!(index = p2p.index, n = p2p.n, round, "dkg: started");
        aether_node::dkg::run(p2p.keys.signer.clone(), aether_node::dkg::Round::dkg(p2p.validators(), round), None, sender, receiver, Default::default()).await
    });
    match result {
        Ok(None) => unreachable!("every DKG participant is a player"),
        Ok(Some((output, share))) => {
            let file = aether_node::dkg::KeyFile::new(round, &output, &share);
            write_secret(&out_path, &serde_json::to_vec_pretty(&file).expect("key file serializes"));
            // network.json now also carries the committee identity wallets pin.
            public.identity = Some(file.identity.clone());
            public.round = round;
            public.output = Some(file.output.clone());
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
    validators: &commonware_utils::ordered::Set<PublicKey>,
    me: &PublicKey,
    expected_round: Option<u64>,
) -> (
    commonware_utils::ordered::Set<PublicKey>,
    commonware_cryptography::bls12381::primitives::sharing::Sharing<commonware_cryptography::bls12381::primitives::variant::MinSig>,
    commonware_cryptography::bls12381::primitives::group::Share,
) {
    let path = std::path::Path::new(data).join("threshold.json");
    if let Ok(bytes) = std::fs::read(&path) {
        let file: aether_node::dkg::KeyFile = serde_json::from_slice(&bytes).expect("threshold.json");
        if let Some(r) = expected_round {
            assert_eq!(
                file.round, r,
                "threshold.json is from key round {} but network.json expects {r}: use the network.json written by the last dkg/reshare",
                file.round
            );
        }
        let (output, share) = file.decode(validators.len() as u32).expect("threshold.json decodes");
        assert_eq!(output.players(), validators, "threshold.json is for a different validator set");
        tracing::info!(identity = %file.identity, "committee key from DKG");
        return (output.players().clone(), output.public().clone(), share);
    }
    let n = validators.len() as u64;
    assert_eq!(validators, &aether_node::p2p::validators(n), "no threshold.json for this network: run `aether dkg --network …` first");
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
fn spawn_inclusion_lists<S, R>(
    chain: Chain,
    key: ed25519::PrivateKey,
    index: u64,
    validators: Vec<PublicKey>,
    chain_id: u64,
    period: Duration,
    mut out: S,
    mut inbox: R,
) where
    S: commonware_p2p::Sender<PublicKey = PublicKey> + 'static,
    R: commonware_p2p::Receiver<PublicKey = PublicKey> + 'static,
{
    use aether_node::inclusion::{committee, InclusionList};
    let n = validators.len() as u64;
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

/// Guardian storage slot in AetherAccount (ERC-7201 namespace "aether.account.guardian").
const GUARDIAN_SLOT: &str = "814c365e7a9c4fa1d0da41caf4c2bc2af8cb172a9b60ca9932fe088002417e00";

fn dev_address(dev: u8) -> Result<Address, String> {
    let s = P256Signer::from_seed(&dev_seed(dev)).map_err(|e| e.to_string())?;
    aether_crypto::address_of(&s.public_key()).map_err(|e| e.to_string())
}

fn is_delegated(rpc: &str, a: Address) -> Result<bool, String> {
    let code = call(rpc, "eth_getCode", json!([a]))?;
    Ok(code.as_str().is_some_and(|c| c.eq_ignore_ascii_case(&format!("0xef0100{}", hex::encode(aether_execution::AETHER_ACCOUNT.as_slice())))))
}

/// Fee caps from the node's next base fees: 2x headroom (~70 full blocks of
/// growth) plus a 1 gwei tip; only the actual base + tip is charged.
const TIP: u128 = 1_000_000_000;

fn fee_caps(status: &Value) -> Result<aether_types::FeeVector, String> {
    let get = |k: &str| status["base_fee"][k].as_str().and_then(|v| v.parse::<u128>().ok()).ok_or(format!("status has no base_fee.{k}"));
    Ok(aether_types::FeeVector { exec: get("exec")? * 2 + TIP, state: 0, prove: get("prove")? * 2 })
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
    let tx = sign_call_with(&signer, chain_id, nonce, fee_caps(&status)?, TIP, &c).map_err(|e| e.to_string())?;
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
