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
use commonware_p2p::{
    authenticated::lookup, AddressableManager as _, Receiver as _, Recipients, Sender as _,
};
use commonware_runtime::{tokio as cw_tokio, Quota, Runner as _, Supervisor as _};
use commonware_utils::{NZUsize, NZU32};
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

const DEFAULT_CHAIN_ID: u64 = 7_777;
/// The public testnet: the one chain with a faucet that must never run the
/// dev registrar (its genesis registrar key is what actually decides on chain;
/// local test networks may combine a faucet with the dev registrar).
const TESTNET_CHAIN_ID: u64 = 7_780;
const DEV_ACCOUNTS: u8 = 10;
/// Seed index of the public devnet registrar key (local devnets only).
const DEV_REGISTRAR: u8 = 11;
const DEV_BALANCE: u128 = 1_000_000 * 10u128.pow(18);

#[derive(Parser)]
#[command(name = "aether", about = "Aether devnet node and client")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// What history a node keeps (roadmap B4).
#[derive(clap::Args, Clone, Debug)]
struct HistoryArgs {
    /// `archive` keeps every block; `prune` keeps the last --retain-days and
    /// the era files. Default: prune on history v2 networks, archive otherwise
    /// (the 7780 testnet has no era files, so it always keeps everything).
    #[arg(long)]
    history: Option<String>,
    /// Days of blocks, certificates, summaries and receipts a pruning node keeps.
    #[arg(long, default_value_t = aether_node::prune::DEFAULT_RETAIN_DAYS)]
    retain_days: u64,
    /// Also delete the era files of pruned eras (keep only their roots).
    #[arg(long)]
    drop_era_files: bool,
    /// Era shards this Mac holds at most (roadmap B5 phase 1; the disk budget,
    /// 64 shards ≈ the default 50 GB setting of docs/design/15-node-rewards.md).
    #[arg(long, default_value_t = aether_node::shards::DEFAULT_MAX_SHARDS)]
    max_shards: usize,
}

impl HistoryArgs {
    fn mode(&self, history_v2: bool, block_time_ms: u64) -> Result<aether_node::prune::HistoryMode, String> {
        aether_node::prune::HistoryMode::resolve(self.history.as_deref(), history_v2, self.retain_days, block_time_ms, self.drop_era_files)
    }
}

/// Resource limits (docs/ops/resource-limits.md): a Mac running Aether never
/// runs away with it — the prover sidecar's memory cap and worker threads, the
/// node's own history-cache budget, and the data volume's free-space floor.
#[derive(clap::Args, Clone, Debug, Default)]
struct ResourceArgs {
    /// The prover sidecar's memory cap (physical footprint): a plain number is
    /// GB, 512M is exact. Default: a quarter of the RAM, at least 4 GB.
    /// 0 = the prover never runs.
    #[arg(long = "prover-max-memory", value_name = "SIZE")]
    prover_max_memory: Option<String>,
    /// Worker threads the prover may use. Default: half the cores.
    #[arg(long = "prover-threads")]
    prover_threads: Option<usize>,
    /// Let proving run on battery power (it pauses otherwise).
    #[arg(long = "prover-on-battery")]
    prover_on_battery: bool,
    /// Budget for this node's own in-memory history caches (summaries,
    /// receipts). Default: a quarter of the RAM, at least 2 GB.
    #[arg(long = "max-memory", value_name = "SIZE")]
    max_memory: Option<String>,
    /// Below this much free space on the data volume, no new era files or
    /// shards are written and proving pauses. Default: 5 GB. 0 = off.
    #[arg(long = "min-free-disk", value_name = "SIZE")]
    min_free_disk: Option<String>,
}

impl ResourceArgs {
    fn limits(&self) -> Result<aether_node::resources::Limits, String> {
        aether_node::resources::Limits::resolve(
            self.prover_max_memory.as_deref(),
            self.prover_threads,
            self.prover_on_battery,
            self.max_memory.as_deref(),
            self.min_free_disk.as_deref(),
        )
    }

    /// The same settings as flags for a child process (`aether run` forwards
    /// them to its `aether node`/`aether follow` children).
    fn forward(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(v) = &self.prover_max_memory {
            out.push(format!("--prover-max-memory={v}"));
        }
        if let Some(v) = self.prover_threads {
            out.push(format!("--prover-threads={v}"));
        }
        if self.prover_on_battery {
            out.push("--prover-on-battery".into());
        }
        if let Some(v) = &self.max_memory {
            out.push(format!("--max-memory={v}"));
        }
        if let Some(v) = &self.min_free_disk {
            out.push(format!("--min-free-disk={v}"));
        }
        out
    }
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
        /// Answer `aether_faucet` with this key (from `aether faucet-key`). A local
        /// devnet (no --network) uses its public dev account 10 instead.
        #[arg(long)]
        faucet_key: Option<String>,
        /// Register Macs (`aether_registerDevice`) with this Apple DeviceCheck key (.p8);
        /// attestations are signed with <data>/registrar.key.
        #[arg(long)]
        devicecheck_key: Option<String>,
        #[arg(long, requires = "devicecheck_key")]
        devicecheck_key_id: Option<String>,
        #[arg(long, default_value = "45WU468FZE")]
        devicecheck_team: String,
        /// Test chains (no faucet): register every device without Apple (public dev registrar key).
        #[arg(long, hide = true)]
        dev_registrar: bool,
        /// Local devnet: blocks per voting-node epoch.
        #[arg(long, hide = true, conflicts_with = "network")]
        dev_epoch_blocks: Option<u64>,
        /// Exit when the launching process does (`aether run`, the Mac app).
        #[arg(long)]
        exit_with_parent: bool,
        #[command(flatten)]
        history: HistoryArgs,
        #[command(flatten)]
        resources: ResourceArgs,
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
        #[arg(long, required_unless_present = "stage")]
        epoch_end: Option<u64>,
        /// Hash of that block (`aether blocks`), which the new epoch builds on.
        #[arg(long, required_unless_present = "stage")]
        epoch_end_hash: Option<String>,
        /// Background reshare while the current committee keeps running (`aether
        /// run`): write threshold-next.json and network-next.json only; the switch
        /// height comes later from the committee-signed handoff.
        #[arg(long)]
        stage: bool,
        /// With --stage on a running validator: its node owns this Mac's public
        /// node id and forwards reshare links here.
        #[arg(long, requires = "stage")]
        via_node: bool,
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
        #[arg(long)]
        exit_with_parent: bool,
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
    /// Follow the chain without being a validator: verify every certificate,
    /// re-execute every block, and serve wallets on this machine.
    Follow {
        /// network.json of the network to follow (default: the public devnet keys).
        #[arg(long)]
        network: Option<String>,
        /// Validators' RPC URLs to pull from (default: find them on the Mainline DHT).
        #[arg(long, value_delimiter = ',')]
        from_rpc: Vec<String>,
        #[arg(long)]
        data: String,
        #[arg(long, default_value_t = 8545)]
        rpc_port: u16,
        /// Devnet validator count (without --network).
        #[arg(long, default_value_t = 4)]
        validators: u64,
        /// Exit when the launching app does (the Mac app's node switch).
        #[arg(long)]
        exit_with_parent: bool,
        /// Also be a voting-node candidate: keep keys in <data> and send a liveness
        /// beacon every epoch once registered (`aether candidate-register`).
        #[arg(long)]
        candidate: bool,
        /// Local devnet: blocks per voting-node epoch (must match the validators).
        #[arg(long, hide = true, conflicts_with = "network")]
        dev_epoch_blocks: Option<u64>,
        /// Where the candidate keys are (default: <data>).
        #[arg(long)]
        keys: Option<String>,
        /// Start from a certified state snapshot (checked against the next
        /// certified block) instead of replaying history from genesis.
        #[arg(long)]
        checkpoint: bool,
        /// Dev only (hidden): the disk fills `<ms>` after start, so the node
        /// hits the storage path of docs/design/24-self-healing.md.
        #[arg(long, hide = true)]
        dev_storage_fault: Option<u64>,
        #[command(flatten)]
        history: HistoryArgs,
        #[command(flatten)]
        resources: ResourceArgs,
    },
    /// Keep this Mac in the network: validator while in the voting set, verifying
    /// follower and candidate otherwise; rotations are followed automatically.
    Run {
        #[arg(long)]
        data: String,
        /// network.json to start from (copied into <data> the first time).
        #[arg(long)]
        network: Option<String>,
        #[arg(long, default_value_t = 9000)]
        port: u16,
        #[arg(long, default_value_t = 8545)]
        rpc_port: u16,
        /// Background reshare port (default: --port + 1, where a running
        /// validator's node forwards reshare links).
        #[arg(long)]
        reshare_port: Option<u16>,
        /// Extra argument for `aether node` (repeatable), e.g. --node-arg=--faucet-key=…
        #[arg(long = "node-arg", allow_hyphen_values = true)]
        node_args: Vec<String>,
        /// Extra argument for `aether follow` (repeatable).
        #[arg(long = "follow-arg", allow_hyphen_values = true)]
        follow_args: Vec<String>,
        /// Seconds a reshare may take before the running set carries on.
        #[arg(long, default_value_t = 300)]
        reshare_timeout: u64,
        #[arg(long, hide = true)]
        dev_peer_dir: Option<String>,
        /// Exit when the launching app does (the Mac app's node switch).
        #[arg(long)]
        exit_with_parent: bool,
        #[command(flatten)]
        resources: ResourceArgs,
    },
    /// This Mac's voting-node identity in <data> (created the first time), as JSON.
    CandidateInfo {
        #[arg(long)]
        data: String,
        /// Also print the voting key's ownership signature for registering it
        /// under this operator (the wallet account) on this chain.
        #[arg(long, requires = "chain_id")]
        operator: Option<Address>,
        #[arg(long)]
        chain_id: Option<u64>,
    },
    /// Register this Mac's candidate (keys in <data>) with the registrar and the registry.
    CandidateRegister {
        #[arg(long)]
        data: String,
        /// The registrar node's RPC.
        #[arg(long)]
        registrar_rpc: String,
        /// Where to submit the registration.
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        /// Operator (the owner's account): a dev account number on a devnet.
        #[arg(long)]
        from_dev: u8,
        /// DeviceCheck token (base64) from the Mac app; any text on a dev registrar.
        #[arg(long, default_value = "dev")]
        device_token: String,
        /// Priority fee (wei) on the registration tx: 0 lets a zero-balance operator
        /// register on a network with no faucet (mainnet).
        #[arg(long, default_value_t = TIP)]
        tip: u128,
    },
    /// Validator: sign a protocol upgrade with this validator's key share (prints a partial).
    UpgradeSign {
        #[arg(long)]
        data: String,
        #[arg(long)]
        network: String,
        /// Upgrade JSON: {chain_id, protocol, activate_at, releases: [{platform, version, blake3, url}], notes}.
        upgrade: String,
    },
    /// Combine at least a threshold of partials into the committee-signed upgrade.
    UpgradeCombine {
        #[arg(long)]
        network: String,
        partials: Vec<String>,
    },
    /// Check a signed upgrade against the committee identity in network.json.
    UpgradeVerify {
        #[arg(long)]
        network: String,
        signed: String,
    },
    /// Create the DeviceCheck registrar's attestation key at <data>/registrar.key.
    RegistrarKey {
        #[arg(long)]
        data: String,
    },
    /// Create the testnet faucet key at <data>/faucet.key and print its address
    /// (put it in network.json with `aether network --faucet`).
    FaucetKey {
        #[arg(long)]
        data: String,
    },
    /// Assemble network.json from validators' validator.pub.json files (in validator order).
    Network {
        #[arg(long, default_value_t = DEFAULT_CHAIN_ID)]
        chain_id: u64,
        /// Faucet address (from `aether faucet-key`): the only account funded at genesis.
        #[arg(long)]
        faucet: Option<Address>,
        /// Registrar public key (x‖y hex, from `aether registrar-key`): predeploys the voting-node registry.
        #[arg(long)]
        registrar: Option<String>,
        /// Blocks per voting-node epoch (default 3600: an hour of 1 s blocks).
        #[arg(long)]
        epoch_blocks: Option<u64>,
        /// Epochs of unbroken liveness before a Mac can be drawn (default 24).
        #[arg(long)]
        min_streak: Option<u64>,
        /// Epochs between voting-set draws (default 24).
        #[arg(long)]
        draw_epochs: Option<u64>,
        /// Node rewards from genesis (docs/design/15-node-rewards.md): a new network only, needs --registrar.
        #[arg(long)]
        node_rewards: bool,
        /// History v2 (a new genesis only, roadmap B): quiet empty blocks, era files, prune by default.
        #[arg(long)]
        history: Option<u32>,
        /// Protocol this genesis starts under (1 to the newest this binary runs): the mainnet names
        /// the newest (3: proof market, registry v2 with its registration cap, 16-seat growth) and
        /// never needs an upgrade (docs/design/15-node-rewards.md, gap G1). Absent: 1, upgrades
        /// turn later protocols on — 7780's genesis stays byte-identical.
        #[arg(long)]
        protocol: Option<u32>,
        /// Write the public dev registrar key as the registrar: a local or rehearsal network whose
        /// registrar node runs `aether run --dev-registrar` (no Apple DeviceCheck).
        #[arg(long, conflicts_with = "registrar")]
        dev_registrar: bool,
        /// Founder reserve keys (validator.pub.json, up to 3, one Mac): seated only while fewer
        /// than four independent operators qualify. Needs --node-rewards and --reserve-operator.
        #[arg(long = "reserve", requires = "reserve_operator")]
        reserve: Vec<String>,
        /// The founder's operator address (its own registered Macs are not independent).
        #[arg(long)]
        reserve_operator: Option<Address>,
        members: Vec<String>,
    },
    /// List the public development accounts (funded at genesis; never use for value).
    DevAccounts,
    /// Check a network.json against the mainnet rule set: every rule the mainnet
    /// must have active at height 1 (docs/ops/mainnet-launch.md §2), built from
    /// the file's genesis. Prints one line per rule; fails listing what is off.
    MainnetRules {
        /// network.json to check.
        #[arg(long)]
        network: String,
    },
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
        /// Recovery devices (dev account numbers), comma separated.
        #[arg(long, value_delimiter = ',')]
        guardian_dev: Vec<u8>,
        /// How many of them must sign a recovery.
        #[arg(long, default_value_t = 1)]
        threshold: u8,
        /// Seconds between a proposal and when it may run (the owner can cancel meanwhile).
        #[arg(long, default_value_t = aether_execution::account::DEFAULT_DELAY)]
        delay: u64,
    },
    /// Stop a pending recovery of your account.
    CancelRecovery {
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        #[arg(long)]
        from_dev: u8,
    },
    /// Guardians propose moving the lost account's verified balance to the first
    /// guardian's account; it runs after the delay with `--finish`.
    Recover {
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc: String,
        /// Signing recovery devices (dev account numbers), comma separated; the first relays.
        #[arg(long, value_delimiter = ',')]
        guardian_dev: Vec<u8>,
        #[arg(long)]
        lost: Address,
        /// Run a proposal whose delay has passed: `--finish <to>:<value wei>` as printed by the proposal.
        #[arg(long)]
        finish: Option<String>,
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
        Cmd::Node {
            index,
            validators,
            network,
            port,
            rpc_port,
            data,
            peers,
            link_base,
            offline,
            block_time_ms,
            dev_censor,
            dev_deprioritize,
            faucet_key,
            devicecheck_key,
            devicecheck_key_id,
            devicecheck_team,
            dev_registrar,
            dev_epoch_blocks,
            exit_with_parent,
            history,
            resources,
        } => {
            if exit_with_parent {
                exit_with_parent_process();
            }
            let with_file = network.is_some();
            let network_file = network.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            let max_shards = history.max_shards;
            p2p_args(index, validators, network, &data, port, peers, link_base, offline)
                .and_then(|args| {
                    if with_file && args.3.is_none() {
                        return Err("network.json has no committee identity: run the node with the network.json written by dkg/reshare".into());
                    }
                    Ok(args)
                })
                .and_then(|args| {
                    if dev_registrar && args.4.faucet.is_some() {
                        if args.1 == TESTNET_CHAIN_ID {
                            return Err("--dev-registrar is only for test chains without a faucet".into());
                        }
                        // A local network that funds through a faucet (devnet tests):
                        // allowed, but say it — the dev registrar registers any device.
                        eprintln!("--dev-registrar on a chain with a faucet ({})", args.1);
                    }
                    Ok(args)
                })
                .and_then(|args| {
                    let mode = history.mode(args.4.history >= 2, block_time_ms)?;
                    Ok((args, mode))
                })
                .map(|((p2p, chain_id, epochs, key_round, mut genesis), history)| {
                    if let Some(e) = dev_epoch_blocks {
                        genesis.epoch_blocks = e;
                    }
                    run_node(NodeArgs {
                        history,
                        max_shards,
                        p2p,
                        chain_id,
                        epochs,
                        key_round,
                        rpc_port,
                        data,
                        block_time_ms,
                        dev_censor,
                        dev_deprioritize,
                        genesis,
                        faucet_key,
                        devicecheck: devicecheck_key.zip(devicecheck_key_id).map(|(k, id)| (k, id, devicecheck_team)),
                        dev_registrar,
                        network_file,
                        resources,
                    });
                })
        }
        Cmd::Keygen { data } => keygen(&data),
        Cmd::UpgradeSign { data, network, upgrade } => (|| {
            use aether_node::upgrade::{sign_partial, Upgrade};
            let file = aether_node::roster::NetworkFile::load(std::path::Path::new(&network))?;
            let key: aether_node::dkg::KeyFile =
                serde_json::from_slice(&std::fs::read(std::path::Path::new(&data).join("threshold.json")).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let (_, share) = key.decode(file.validators.len() as u32)?;
            let u: Upgrade = serde_json::from_slice(&std::fs::read(&upgrade).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            if u.chain_id != file.chain_id {
                return Err(format!("upgrade is for chain {}, network.json for {}", u.chain_id, file.chain_id));
            }
            println!("{}", serde_json::to_string_pretty(&sign_partial(&u, &share)).expect("json"));
            Ok(())
        })(),
        Cmd::UpgradeCombine { network, partials } => (|| {
            let file = aether_node::roster::NetworkFile::load(std::path::Path::new(&network))?;
            let output = file.output.clone().ok_or("network.json has no committee output")?;
            let key = aether_node::dkg::KeyFile { round: file.round, output, identity: String::new(), share: String::new() };
            let dkg = key.decode_output(file.validators.len() as u32)?;
            let parts = partials
                .iter()
                .map(|p| std::fs::read(p).map_err(|e| format!("{p}: {e}")).and_then(|b| serde_json::from_slice(&b).map_err(|e| format!("{p}: {e}"))))
                .collect::<Result<Vec<aether_node::upgrade::PartialUpgrade>, String>>()?;
            let signed = aether_node::upgrade::combine(dkg.public(), &parts)?;
            println!("{}", serde_json::to_string_pretty(&signed).expect("json"));
            Ok(())
        })(),
        Cmd::UpgradeVerify { network, signed } => (|| {
            let file = aether_node::roster::NetworkFile::load(std::path::Path::new(&network))?;
            let set = aether_light::ValidatorSet::from_hex(file.identity.as_deref().ok_or("network.json has no identity")?).map_err(|e| format!("{e:?}"))?;
            let s: aether_node::upgrade::SignedUpgrade =
                serde_json::from_slice(&std::fs::read(&signed).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            aether_node::upgrade::verify(set.identity(), &s)?;
            println!("signed by the committee: protocol {} at height {} on chain {}", s.upgrade.protocol, s.upgrade.activate_at, s.upgrade.chain_id);
            Ok(())
        })(),
        Cmd::Follow { network, from_rpc, data, rpc_port, validators, exit_with_parent, candidate, dev_epoch_blocks, keys, checkpoint, dev_storage_fault, history, resources } => {
            if exit_with_parent {
                exit_with_parent_process();
            }
            let keys = candidate.then(|| keys.unwrap_or_else(|| data.clone()));
            run_follow(network, from_rpc, data, rpc_port, validators, keys, dev_epoch_blocks, checkpoint, dev_storage_fault, history, resources)
        }
        Cmd::CandidateInfo { data, operator, chain_id } => aether_node::candidate::CandidateKeys::load_or_create(std::path::Path::new(&data)).map(|k| {
            let ownership = operator.zip(chain_id).map(|(op, id)| hex::encode(k.ownership(id, op)));
            println!(
                "{}",
                json!({ "validator_key": hex::encode(k.validator_key()), "node_id": hex::encode(k.node_id()), "beaconer": k.beaconer(), "ownership": ownership })
            );
        }),
        Cmd::Run { data, network, port, rpc_port, reshare_port, node_args, follow_args, reshare_timeout, dev_peer_dir, exit_with_parent, resources } => {
            if exit_with_parent {
                exit_with_parent_process();
            }
            tracing_subscriber::fmt()
                .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,commonware=warn".into()))
                .init();
            (|| {
                let dir = std::path::PathBuf::from(&data);
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                aether_node::candidate::CandidateKeys::load_or_create(&dir)?;
                aether_node::supervisor::adopt_network(&dir, network.as_deref().map(std::path::Path::new))?;
                // The same resource limits for whichever child runs (the
                // supervisor adds them to both `aether node` and `aether follow`).
                let forwarded = resources.forward();
                let (mut node_args, mut follow_args) = (node_args, follow_args);
                node_args.extend(forwarded.iter().cloned());
                follow_args.extend(forwarded);
                aether_node::supervisor::Supervisor {
                    exe: std::env::current_exe().map_err(|e| e.to_string())?,
                    data: dir,
                    port,
                    rpc_port,
                    reshare_port: reshare_port.unwrap_or(port + 1),
                    node_args,
                    follow_args,
                    dev_peer_dir: dev_peer_dir.map(Into::into),
                    reshare_timeout: Duration::from_secs(reshare_timeout),
                }
                .run()
            })()
        }
        Cmd::CandidateRegister { data, registrar_rpc, rpc, from_dev, device_token, tip } => (|| {
            let keys = aether_node::candidate::CandidateKeys::load_or_create(std::path::Path::new(&data))?;
            let operator = dev_address(from_dev)?;
            let (vk, nid, beaconer) = (keys.validator_key(), keys.node_id(), keys.beaconer());
            let chain_id = call(&registrar_rpc, "aether_status", json!([]))?["chain_id"].as_u64().ok_or("registrar has no chain id")?;
            let ownership = hex::encode(keys.ownership(chain_id, operator));
            let a = call(&registrar_rpc, "aether_registerDevice", json!([device_token, operator, hex::encode(vk), hex::encode(nid), beaconer, ownership]))?;
            let part = |k: &str| -> Result<[u8; 32], String> {
                hex::decode(a[k].as_str().unwrap_or_default()).ok().and_then(|b| b.try_into().ok()).ok_or(format!("registrar gave no {k}"))
            };
            let input = aether_execution::registry::encode_register(vk, nid, beaconer, part("r")?, part("s")?);
            println!("candidate {}  node {}  beacons from {beaconer}", hex::encode(vk), hex::encode(nid));
            let c = EvmCall { to: Some(aether_execution::registry::REGISTRY), value: U256::ZERO, input, gas_limit: 400_000, delegate: None };
            let r = submit_with_tip(&rpc, from_dev, None, c, true, tip)?;
            if r["receipt"]["success"] != json!(true) {
                return Err("registration reverted (this Mac or voting key is already registered?)".into());
            }
            Ok(())
        })(),
        Cmd::FaucetKey { data } => (|| {
            // Idempotent: an existing key is kept (and its address printed).
            let path = std::path::Path::new(&data).join("faucet.key");
            if !path.exists() {
                aether_node::faucet::Faucet::generate(&path)?;
            }
            let a = aether_node::faucet::Faucet::load(&path)?.address;
            println!("faucet address {a}\nkey in {data}/faucet.key (keep it on this machine; run the node with --faucet-key)");
            Ok(())
        })(),
        Cmd::Head { data } => (|| {
            let store = aether_node::store::Store::open(&std::path::Path::new(&data).join("state.redb")).map_err(|e| e.to_string())?;
            let (h, d) = store.head().map_err(|e| e.to_string())?.ok_or("no finalized state")?;
            println!("{h} {}", hex::encode(d));
            Ok(())
        })(),
        Cmd::Reshare { from, to, epoch_end, epoch_end_hash, stage, via_node, port, data, peers, link_base, offline, exit_with_parent } => {
            if exit_with_parent {
                exit_with_parent_process();
            }
            let boundary = match (stage, epoch_end, epoch_end_hash) {
                (true, _, _) => None,
                (false, Some(h), Some(parent)) => Some(aether_node::roster::EpochStart { height: h + 1, parent }),
                _ => unreachable!("clap requires --epoch-end and --epoch-end-hash without --stage"),
            };
            reshare(&from, &to, boundary, port, data, peers, link_base, offline, via_node)
        }
        Cmd::Network { chain_id, faucet, registrar, dev_registrar, epoch_blocks, min_streak, draw_epochs, node_rewards, history, protocol, reserve, reserve_operator, members } => {
            let registrar = match (registrar, dev_registrar) {
                (None, true) => Some(dev_registrar_hex()),
                (r, _) => r,
            };
            let reserve = reserve_operator.map(|op| (op, reserve));
            assemble_network(chain_id, faucet, registrar, (epoch_blocks, min_streak, draw_epochs), history, protocol, (node_rewards, reserve), &members)
        }
        Cmd::RegistrarKey { data } => (|| {
            // Idempotent: an existing key is kept (and its public half printed).
            let path = std::path::Path::new(&data).join("registrar.key");
            if !path.exists() {
                aether_node::faucet::Faucet::generate(&path)?;
            }
            let k = aether_node::faucet::Faucet::load(&path)?;
            println!("registrar key {}\nin {data}/registrar.key (put the key in network.json with `aether network --registrar`)", k.public_hex());
            Ok(())
        })(),
        Cmd::DevAccounts => {
            for (i, a) in dev_accounts(DEV_ACCOUNTS) {
                println!("dev {i:>2}  {a}");
            }
            Ok(())
        }
        Cmd::MainnetRules { network } => (|| {
            let file = aether_node::roster::NetworkFile::load(std::path::Path::new(&network))?;
            let genesis = file.genesis()?;
            let chain_id = file.chain_id;
            let rules = aether_node::mainnet::check(&chain_config(chain_id, &genesis, false));
            for r in &rules {
                println!("{}  {}: {}", if r.ok { "ok" } else { "FAIL" }, r.name, r.detail);
            }
            let missing = aether_node::mainnet::missing(&rules);
            (rules.iter().all(|r| r.ok)).then_some(()).ok_or(missing)
        })(),
        Cmd::Status { rpc } => call(&rpc, "aether_status", json!([])).map(|v| println!("{}", pretty(&v))),
        Cmd::Blocks { rpc, n } => call(&rpc, "aether_recentBlocks", json!([n])).map(|v| print_blocks(&v)),
        Cmd::Send { rpc, from_dev, to, value, nonce, wait } => {
            submit(&rpc, from_dev, nonce, EvmCall { to: Some(to), value, input: Bytes::new(), gas_limit: 21_000, delegate: None }, wait).map(|_| ())
        }
        Cmd::Batch { rpc, from_dev, to, value, wait } => (|| {
            let signer = P256Signer::from_seed(&dev_seed(from_dev)).map_err(|e| e.to_string())?;
            let from = aether_crypto::address_of(&signer.public_key()).map_err(|e| e.to_string())?;
            let calls: Vec<_> = to.iter().map(|a| (*a, value, Bytes::new())).collect();
            let c = EvmCall {
                to: Some(from),
                value: U256::ZERO,
                input: aether_execution::encode_execute(&calls),
                gas_limit: 60_000 + 40_000 * calls.len() as u64,
                delegate: Some(aether_execution::AETHER_ACCOUNT),
            };
            submit(&rpc, from_dev, None, c, wait).map(|_| ())
        })(),
        Cmd::SetGuardian { rpc, from_dev, guardian_dev, threshold, delay } => (|| {
            let keys = guardian_dev
                .iter()
                .map(|d| {
                    let g = P256Signer::from_seed(&dev_seed(*d)).map_err(|e| e.to_string())?;
                    aether_crypto::p256_xy(&g.public_key().bytes).map_err(|e| format!("{e:?}"))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let me = dev_address(from_dev)?;
            let set = aether_execution::account::encode_set_guardians(&keys, threshold, delay);
            let c = EvmCall {
                to: Some(me),
                value: U256::ZERO,
                input: aether_execution::encode_execute(&[(me, U256::ZERO, set)]),
                gas_limit: 500_000,
                delegate: Some(aether_execution::AETHER_ACCOUNT),
            };
            println!("{} recovery device(s), {threshold} must sign, {delay} s delay", keys.len());
            submit(&rpc, from_dev, None, c, true).map(|_| ())
        })(),
        Cmd::CancelRecovery { rpc, from_dev } => (|| {
            let me = dev_address(from_dev)?;
            let input = aether_execution::encode_execute(&[(me, U256::ZERO, aether_execution::account::encode_cancel_recovery())]);
            submit(&rpc, from_dev, None, EvmCall { to: Some(me), value: U256::ZERO, input, gas_limit: 200_000, delegate: None }, true).map(|_| ())
        })(),
        Cmd::Recover { rpc, guardian_dev, lost, finish, validators, identity } => (|| {
            use aether_execution::account::{self as acct, slots};
            let relay = *guardian_dev.first().ok_or("--guardian-dev")?;
            if let Some(spec) = finish {
                let (to, value) = spec.split_once(':').ok_or("--finish <to>:<value wei>")?;
                let calls = [(to.parse::<Address>().map_err(|e| e.to_string())?, value.parse::<U256>().map_err(|e| e.to_string())?, Bytes::new())];
                let c = EvmCall { to: Some(lost), value: U256::ZERO, input: acct::encode_execute_recovery(&calls), gas_limit: 300_000, delegate: None };
                return submit(&rpc, relay, None, c, true).map(|_| ());
            }
            let set = trusted(validators, identity)?;
            let slot = |slot: U256| -> Result<U256, String> {
                let v = call(&rpc, "aether_getStorage", json!([lost, slot]))?;
                let proof: Proof = serde_json::from_value(v["proof"].clone()).map_err(|e| e.to_string())?;
                let anchor = certified_anchor(&rpc, v["height"].as_u64().unwrap_or_default(), &set)?;
                aether_light::verify_storage(&anchor, &lost, slot, &proof).map_err(|e| e.to_string())
            };
            // Balance, guardian list and proposal nonce, all proven against certified roots.
            let v = call(&rpc, "aether_getAccount", json!([lost]))?;
            let proof: Proof = serde_json::from_value(v["proof"].clone()).map_err(|e| e.to_string())?;
            let anchor = certified_anchor(&rpc, v["height"].as_u64().unwrap_or_default(), &set)?;
            let balance = U256::from(aether_light::verify_account(&anchor, &lost, &proof).map_err(|e| e.to_string())?.unwrap_or_default().balance);
            let count = slot(slots::guardian_count())?.to::<u64>();
            let mut keys = Vec::new();
            for i in 0..count {
                keys.push((slot(slots::guardian(i))?.to_be_bytes::<32>(), slot(slots::guardian(i) + U256::from(1u64))?.to_be_bytes::<32>()));
            }
            let nonce = slot(slots::recovery_nonce())?.to::<u64>();
            let (_, delay) = slots::unpack_threshold_and_delay(slot(slots::threshold_and_delay())?);
            let chain_id = call(&rpc, "aether_status", json!([]))?["chain_id"].as_u64().unwrap_or_default();
            let me = dev_address(relay)?;
            let calls = [(me, balance, Bytes::new())];
            let message = acct::recovery_message(chain_id, lost, nonce, &calls);
            let mut sigs = Vec::new();
            for d in &guardian_dev {
                let g = P256Signer::from_seed(&dev_seed(*d)).map_err(|e| e.to_string())?;
                let k = aether_crypto::p256_xy(&g.public_key().bytes).map_err(|e| format!("{e:?}"))?;
                let i = keys.iter().position(|x| *x == k).ok_or(format!("dev {d} is not a recovery device of {lost}"))?;
                let sig = g.sign(&message).map_err(|e| format!("{e:?}"))?;
                sigs.push((i as u8, sig[..32].try_into().expect("32"), sig[32..64].try_into().expect("32")));
            }
            println!("proposing to move {balance} wei from {lost} to {me} (proposal {nonce}, runs after {delay} s unless the owner cancels)");
            let c = EvmCall { to: Some(lost), value: U256::ZERO, input: acct::encode_propose_recovery(&calls, &sigs), gas_limit: 500_000, delegate: None };
            submit(&rpc, relay, None, c, true)?;
            println!("then: aether recover --rpc {rpc} --guardian-dev {relay} --lost {lost} --finish {me}:{balance}");
            Ok(())
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
            p2p_args(index, validators, network, &data, port, peers, link_base, offline)
                .map(|(p2p, chain_id, _, _, genesis)| run_dkg(p2p, chain_id, data, round, genesis))
        }
        Cmd::Receipt { hash, rpc } => call(&rpc, "aether_getReceipt", json!([hash])).map(|v| println!("{}", pretty(&v))),
    };
    if let Err(e) = res {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// The public dev registrar key (x‖y hex): the key `aether run --dev-registrar`
/// signs attestations with, for `aether network --dev-registrar`.
fn dev_registrar_hex() -> String {
    aether_node::faucet::Faucet::from_seed(&dev_seed(DEV_REGISTRAR))
        .expect("dev registrar")
        .public_hex()
        .to_string()
}

/// Genesis: a public network funds only its faucet; a local devnet funds the public dev accounts.
/// A network.json without a faucet (mainnet: 사전 발행 0) funds nobody: all
/// tokens come from issuance (docs/design/12-launch-plan.md).
fn chain_config(chain_id: u64, genesis: &aether_node::roster::Genesis, dev_alloc: bool) -> ChainConfig {
    // A local devnet also gets the voting-node registry, with the public dev registrar key.
    let dev_registrar = dev_alloc.then(|| {
        let k = aether_node::faucet::Faucet::from_seed(&dev_seed(DEV_REGISTRAR))
            .expect("dev registrar");
        let h = hex::decode(k.public_hex()).expect("hex");
        (
            h[..32].try_into().expect("32"),
            h[32..].try_into().expect("32"),
        )
    });
    let alloc = match genesis.faucet {
        Some(f) => vec![(f, U256::from(aether_node::faucet::SUPPLY))],
        None if dev_alloc => dev_accounts(DEV_ACCOUNTS)
            .into_iter()
            .map(|(_, a)| (a, U256::from(DEV_BALANCE)))
            .collect(),
        None => vec![],
    };
    ChainConfig {
        chain_id,
        limits: GasVector {
            exec: 30_000_000,
            state: u64::MAX,
            prove: 200_000_000,
        },
        alloc,
        fees: true,
        registrar: genesis.registrar.or(dev_registrar),
        epoch_blocks: genesis.epoch_blocks,
        min_streak: genesis.min_streak,
        draw_epochs: genesis.draw_epochs,
        history_v2: genesis.history >= 2,
        protocol: genesis.protocol.max(1),
        node_rewards: genesis.node_rewards,
        committee: genesis.committee.clone(),
        reserve: genesis.reserve.clone(),
    }
}

/// p2p args, chain id, epoch starts, expected key round, genesis parameters.
type P2pSetup = (
    P2pArgs,
    u64,
    Vec<aether_node::roster::EpochStart>,
    Option<u64>,
    aether_node::roster::Genesis,
);

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
) -> Result<P2pSetup, String> {
    use aether_node::roster::{LocalKeys, NetworkFile, Roster};
    let (roster, keys, index, chain_id, epochs, round, genesis) = match network {
        Some(path) => {
            let file = NetworkFile::load(std::path::Path::new(&path))?;
            let roster = Roster::from_file(&file)?;
            let keys = LocalKeys::load(std::path::Path::new(data))?;
            let index = roster
                .index_of(&keys.signer.public_key())
                .ok_or("this machine's validator key is not in network.json")?;
            // A network file with an identity names the key round its shares must be from.
            let round = file.identity.as_ref().map(|_| file.round);
            let genesis = file.genesis()?;
            (
                roster,
                keys,
                index,
                file.chain_id,
                file.epochs,
                round,
                genesis,
            )
        }
        None => {
            let (index, n) = (
                index.ok_or("--index (or --network)")?,
                n.ok_or("--validators (or --network)")?,
            );
            (
                Roster::devnet(n),
                LocalKeys::devnet(index),
                index,
                DEFAULT_CHAIN_ID,
                vec![],
                None,
                Default::default(),
            )
        }
    };
    let transport = if peers.iter().any(|p| !p.is_empty()) {
        Transport::Tcp(peers)
    } else {
        Transport::Iroh {
            link_base: link_base.unwrap_or(20_000 + 100 * index as u16),
        }
    };
    let n = roster.len();
    Ok((
        P2pArgs {
            index,
            n,
            roster,
            keys,
            port,
            transport,
            offline,
            max_message: MAX_BLOCK_BYTES + 1024 * 1024,
        },
        chain_id,
        epochs,
        round,
        genesis,
    ))
}

/// Reshare: p2p over the union of both validator sets; old members deal with
/// their current share, new members receive.
#[allow(clippy::too_many_arguments)]
fn reshare(
    from: &str,
    to: &str,
    boundary: Option<aether_node::roster::EpochStart>,
    port: u16,
    data: String,
    peers: Vec<String>,
    link_base: Option<u16>,
    offline: bool,
    via_node: bool,
) -> Result<(), String> {
    use aether_node::roster::{LocalKeys, NetworkFile, Roster};
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,commonware=warn".into()),
        )
        .init();
    let old_file = NetworkFile::load(std::path::Path::new(from))?;
    let new_file = NetworkFile::load(std::path::Path::new(to))?;
    let (old, new) = (Roster::from_file(&old_file)?, Roster::from_file(&new_file)?);
    let output_hex = old_file
        .output
        .clone()
        .ok_or("--from has no committee output: use the network.json written by dkg/reshare")?;
    let n_old = old.len() as u32;
    let previous = aether_node::dkg::KeyFile {
        round: old_file.round,
        output: output_hex,
        identity: String::new(),
        share: String::new(),
    };
    let previous = previous.decode_output(n_old)?;
    let dir = std::path::PathBuf::from(&data);
    let keys = LocalKeys::load(&dir)?;
    let share = match std::fs::read(dir.join("threshold.json")) {
        Ok(b) if old.index_of(&keys.signer.public_key()).is_some() => {
            let f: aether_node::dkg::KeyFile =
                serde_json::from_slice(&b).map_err(|e| e.to_string())?;
            Some(f.decode(n_old)?.1)
        }
        _ => None,
    };
    let union = old.union(&new);
    let index = union
        .index_of(&keys.signer.public_key())
        .ok_or("this machine's key is in neither validator set")?;
    let transport = if peers.iter().any(|p| !p.is_empty()) {
        Transport::Tcp(peers)
    } else {
        // A staged reshare runs next to this Mac's node: its own link ports.
        let base = if boundary.is_none() { 30_000 } else { 20_000 };
        Transport::Iroh {
            link_base: link_base.unwrap_or(base + 100 * index as u16),
        }
    };
    let p2p = P2pArgs {
        index,
        n: union.len(),
        roster: union,
        keys: keys.clone(),
        port,
        transport,
        offline,
        max_message: MAX_BLOCK_BYTES + 1024 * 1024,
    };
    let round = aether_node::dkg::Round::reshare(previous, new.validators(), old_file.round + 1);
    let next_round = round.round;
    // A fresh runtime directory per attempt: a retried round never reads an older one's state.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let executor = cw_tokio::Runner::new(
        cw_tokio::Config::new()
            .with_storage_directory(dir.join("reshare-runtime").join(secs.to_string())),
    );
    let staged = boundary.is_none();
    let result = executor.start(async move |context| {
        let _public = if staged {
            aether_node::p2p::open_reshare(&p2p, via_node, loopback(p2p.port))
                .await
                .map(|(ep, r)| (ep, r.map(Some).unwrap_or(None)))
        } else {
            aether_node::p2p::open_public(&p2p).await.map(|ep| {
                (
                    ep.clone(),
                    Some(aether_net::serve_p2p(ep, loopback(p2p.port))),
                )
            })
        };
        let (mut network, mut oracle) = lookup::Network::new(
            context.child("network"),
            aether_node::p2p::config(&p2p, b"_DKG"),
        );
        oracle.track(0, aether_node::p2p::peer_addresses(&p2p));
        let (sender, receiver) = network.register(0, Quota::per_second(NZU32!(256)));
        network.start();
        tracing::info!(index = p2p.index, round = next_round, "reshare: started");
        aether_node::dkg::run(
            p2p.keys.signer.clone(),
            round,
            share,
            sender,
            receiver,
            Default::default(),
        )
        .await
    });
    match result.map_err(|e| format!("reshare failed: {e}"))? {
        Some((output, share)) => {
            let file = aether_node::dkg::KeyFile::new(next_round, &output, &share);
            let (threshold, network) = match boundary {
                Some(_) => ("threshold.json", "network.json"),
                None => (
                    aether_node::rotation::STAGED_THRESHOLD,
                    aether_node::rotation::STAGED_NETWORK,
                ),
            };
            write_secret(
                &dir.join(threshold),
                &serde_json::to_vec_pretty(&file).expect("json"),
            );
            let mut public = new_file.clone();
            // Same chain, same genesis: keep its faucet even if the new roster file omits it.
            public.keep_genesis(&old_file);
            public.epochs = old_file.epochs.clone();
            public.epochs.extend(boundary);
            public.identity = Some(file.identity.clone());
            public.round = next_round;
            public.output = Some(file.output.clone());
            std::fs::write(
                dir.join(network),
                serde_json::to_vec_pretty(&public).expect("json"),
            )
            .map_err(|e| e.to_string())?;
            println!("committee identity: {} (unchanged)", file.identity);
            println!(
                "new share written to {} (mode 600)",
                dir.join(threshold).display()
            );
        }
        None if boundary.is_none() => {
            // Staged: keep signing with the old share until the switch height.
            println!("dealt our share to the proposed voting set; this validator leaves it at the switch");
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
        entries
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .find_map(|name| name.strip_suffix("-blocks-metadata").map(str::to_string))
    });
    let prefix = existing.unwrap_or_else(|| "aether".to_string());
    let _ = std::fs::write(&marker, &prefix);
    prefix
}

/// `<data>/anchor.json` (block and finalization hex, as `aether_getFinalized`
/// returns them): where a voting node that was a follower starts.
fn load_anchor(
    data: &str,
) -> Option<(aether_node::block::Block, aether_node::engine::Finalization)> {
    use commonware_codec::Decode as _;
    use commonware_codec::DecodeExt as _;
    let v: Value = serde_json::from_slice(
        &std::fs::read(std::path::Path::new(data).join(aether_node::rotation::ANCHOR_FILE)).ok()?,
    )
    .ok()?;
    let hex = |k: &str| aether_light::from_hex(v[k].as_str()?).ok();
    let block = aether_node::block::Block::decode_cfg(
        hex("block")?.as_slice(),
        &aether_node::block::Block::codec_config(MAX_BLOCK_BYTES),
    )
    .ok()?;
    let fin = aether_node::engine::Finalization::decode(hex("finalization")?.as_slice()).ok()?;
    Some((block, fin))
}

fn keygen(data: &str) -> Result<(), String> {
    let dir = std::path::Path::new(data);
    let keys = aether_node::roster::LocalKeys::generate();
    keys.save(dir)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&keys.public()).expect("json")
    );
    println!(
        "secret keys in {} (mode 600); share only {}",
        dir.join(aether_node::roster::KEY_FILE).display(),
        dir.join(aether_node::roster::PUBLIC_FILE).display()
    );
    Ok(())
}

/// Combine validators' public entries (validator.pub.json files) into network.json on stdout.
/// (blocks per epoch, minimum streak, epochs per draw); None = the defaults.
type VotingParams = (Option<u64>, Option<u64>, Option<u64>);

fn assemble_network(
    chain_id: u64,
    faucet: Option<Address>,
    registrar: Option<String>,
    voting: VotingParams,
    history: Option<u32>,
    protocol: Option<u32>,
    rewards: (bool, Option<(Address, Vec<String>)>),
    members: &[String],
) -> Result<(), String> {
    let (epoch_blocks, min_streak, draw_epochs) = voting;
    let (node_rewards, reserve) = rewards;
    if node_rewards && registrar.is_none() {
        return Err("--node-rewards needs the voting-node registry (--registrar)".into());
    }
    let read_members = |paths: &[String]| {
        paths
            .iter()
            .map(|p| {
                std::fs::read(p)
                    .map_err(|e| format!("{p}: {e}"))
                    .and_then(|b| serde_json::from_slice(&b).map_err(|e| format!("{p}: {e}")))
            })
            .collect::<Result<Vec<aether_node::roster::Member>, String>>()
    };
    let validators = read_members(members)?;
    let reserve = match reserve {
        None => None,
        Some((operator, paths)) => Some(aether_node::roster::ReserveFile { operator, validators: read_members(&paths)? }),
    };
    let file = aether_node::roster::NetworkFile {
        chain_id,
        // Frozen here: after handoffs rewrite `validators`, this is still the
        // roster the genesis rewards words record (and re-syncs re-derive).
        genesis_validators: Some(validators.clone()),
        validators,
        identity: None,
        round: 0,
        output: None,
        epochs: vec![],
        faucet,
        registrar,
        epoch_blocks,
        min_streak,
        draw_epochs,
        history,
        protocol,
        node_rewards: node_rewards.then_some(true),
        reserve,
    };
    aether_node::roster::Roster::from_file(&file)?;
    file.genesis()?;
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
    /// Genesis parameters from network.json (default: local devnet).
    genesis: aether_node::roster::Genesis,
    faucet_key: Option<String>,
    /// (key path, key id, team) of the DeviceCheck key, if this node registers Macs.
    devicecheck: Option<(String, String, String)>,
    dev_registrar: bool,
    /// network.json (with the committee output) when run from one: enables rotation.
    network_file: Option<Value>,
    /// Archive or prune (roadmap B4).
    history: aether_node::prune::HistoryMode,
    /// Era shards this Mac holds at most (roadmap B5 phase 1).
    max_shards: usize,
    /// Memory, CPU and disk limits (docs/ops/resource-limits.md).
    resources: ResourceArgs,
}

/// The open-file limit a node asks for when its hard limit allows it.
const NOFILE_WANT: u64 = 65_536;

/// Raise this node's own soft open-file limit (RLIMIT_NOFILE) toward its hard
/// limit. A validator holds a few hundred open files at once — the vote journal
/// keeps one section file per view and opens every one at startup — but launchd
/// and GUI apps hand their children a 256-file soft limit, which on 2026-09-29
/// crash-looped all four validators ("Too many open files"). Never lowers the
/// limit and never raises it above the hard limit or the kernel's per-process
/// cap. Returns the soft limit now in effect (0 when it could not be read).
#[cfg(unix)]
fn raise_nofile_limit() -> u64 {
    let Some((soft, hard)) = nofile() else {
        tracing::warn!("could not read the open-file limit");
        return 0;
    };
    let want = NOFILE_WANT.min(hard).min(nofile_per_proc()).max(soft);
    if want > soft && set_nofile(want, hard).is_none() {
        tracing::warn!(from = soft, "could not raise the open-file limit");
        return soft;
    }
    want
}

#[cfg(not(unix))]
fn raise_nofile_limit() -> u64 {
    0
}

/// The soft and hard open-file limits (RLIM_INFINITY read back as u64::MAX).
#[cfg(unix)]
fn nofile() -> Option<(u64, u64)> {
    let mut lim: libc::rlimit = unsafe { std::mem::zeroed() };
    (unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) } == 0).then(|| {
        (
            lim.rlim_cur,
            if lim.rlim_max == libc::RLIM_INFINITY {
                u64::MAX
            } else {
                lim.rlim_max
            },
        )
    })
}

/// Set the soft open-file limit to `soft`, keeping `hard` as it was.
#[cfg(unix)]
fn set_nofile(soft: u64, hard: u64) -> Option<()> {
    let lim = libc::rlimit {
        rlim_cur: soft,
        rlim_max: if hard == u64::MAX {
            libc::RLIM_INFINITY
        } else {
            hard
        },
    };
    (unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &lim) } == 0).then_some(())
}

/// The most this process may ask for (macOS: kern.maxfilesperproc).
#[cfg(target_os = "macos")]
fn nofile_per_proc() -> u64 {
    let mut v: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>();
    (unsafe {
        libc::sysctlbyname(
            b"kern.maxfilesperproc\0".as_ptr().cast(),
            &mut v as *mut _ as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } == 0
        && v > 0)
        .then_some(v as u64)
        .unwrap_or(u64::MAX)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn nofile_per_proc() -> u64 {
    u64::MAX
}

fn run_node(a: NodeArgs) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,commonware=warn".into()),
        )
        .init();
    tracing::info!(
        files = raise_nofile_limit(),
        "open-file limit (the vote journal's section files are one fd each)"
    );
    let NodeArgs {
        p2p,
        chain_id,
        epochs,
        key_round,
        rpc_port,
        block_time_ms,
        dev_censor,
        dev_deprioritize,
        data,
        genesis,
        faucet_key,
        devicecheck,
        dev_registrar,
        network_file,
        history,
        max_shards,
        resources,
    } = a;
    // Resource limits (docs/ops/resource-limits.md), before the chain opens:
    // the open itself trims the history caches under the budget, and the
    // watchdog starts watching disk, pressure and battery from here on.
    let limits = resources.limits().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(2);
    });
    let m = aether_node::resources::install(limits, std::path::Path::new(&data).to_path_buf());
    tracing::info!(
        prover_max_memory_gb = m.limits.prover_max_memory / aether_node::resources::GB,
        prover_threads = m.limits.prover_threads,
        cache_budget_gb = m.limits.max_memory / aether_node::resources::GB,
        min_free_disk_gb = m.limits.min_free_disk / aether_node::resources::GB,
        "resource limits on"
    );
    let faucet = genesis.faucet;
    let registry = || {
        aether_node::devicecheck::Registry::open(
            std::path::Path::new(&data).join("registrations.json"),
        )
    };
    let registrar = if dev_registrar {
        let signer = aether_node::faucet::Faucet::from_seed(&dev_seed(DEV_REGISTRAR))
            .expect("dev registrar");
        Some(std::sync::Arc::new(aether_node::devicecheck::Registrar::new(
            None,
            registry(),
            signer,
            chain_id,
        )))
    } else {
        devicecheck.map(|(k, id, team)| {
            let apple =
                aether_node::devicecheck::DeviceCheck::load(std::path::Path::new(&k), &id, &team)
                    .expect("load --devicecheck-key");
            let signer = aether_node::faucet::Faucet::load(
                &std::path::Path::new(&data).join("registrar.key"),
            )
            .expect("<data>/registrar.key (aether registrar-key)");
            std::sync::Arc::new(aether_node::devicecheck::Registrar::new(
                Some(apple),
                registry(),
                signer,
                chain_id,
            ))
        })
    };
    let faucet_service = match (&faucet_key, faucet) {
        (Some(path), expected) => {
            let f = aether_node::faucet::Faucet::load(std::path::Path::new(path))
                .expect("load --faucet-key");
            if let Some(e) = expected {
                assert_eq!(
                    f.address, e,
                    "--faucet-key is not the faucet in network.json"
                );
            }
            Some(std::sync::Arc::new(f))
        }
        // Local devnet: public dev account 10 (funded at genesis) hands out test tokens.
        (None, None) if network_file.is_none() => Some(std::sync::Arc::new(
            aether_node::faucet::Faucet::from_seed(&dev_seed(DEV_ACCOUNTS)).expect("dev faucet"),
        )),
        _ => None,
    };
    let (index, port) = (p2p.index, p2p.port);
    assert!(
        !p2p.offline || matches!(p2p.transport, Transport::Tcp(_)),
        "--offline needs --peers"
    );
    let signer = p2p.keys.signer.clone();
    let (roster_keys, validator_set) = (p2p.roster.keys.clone(), p2p.validators());
    let roster_members: Vec<(String, String)> = p2p
        .roster
        .keys
        .iter()
        .zip(&p2p.roster.nodes)
        .map(|(k, n)| (hex::encode(k.as_ref()), n.to_string()))
        .collect();
    let peers = aether_node::p2p::peer_addresses(&p2p);
    let p2p_cfg = aether_node::p2p::config(&p2p, b"_P2P");
    let links = matches!(p2p.transport, Transport::Iroh { .. });
    let executor = cw_tokio::Runner::new(cw_tokio::Config::new().with_storage_directory(&data));
    let cfg = chain_config(chain_id, &genesis, network_file.is_none());

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
        let (mut handoff_out, mut handoff_in) = network.register(7, Quota::per_second(NZU32!(64)));

        // BLS threshold certificates (one group signature per block) with a VRF
        // seed per round for leader election. Devnet shares come from a fixed
        // dealer seed; a real network derives them with a DKG.
        let (participants, polynomial, share) = committee_keys(&data, &validator_set, &signer.public_key(), key_round);
        let (handoff_share, handoff_sharing) = (share.clone(), polynomial.clone());
        let polynomial_identity = &polynomial.public().clone();
        let scheme = aether_light::Scheme::signer(&aether_light::consensus_namespace(), participants, polynomial, share).expect("share matches polynomial");
        // Start-up integrity as a follower has it (docs/design/24-self-healing.md
        // layer 1): a database that does not verify is moved aside (never
        // deleted; the keys stay) and the catch-up below re-syncs it.
        let (store, _) = aether_node::follow::open_store(std::path::Path::new(&data)).expect("open state store");
        let (chain, genesis) = match Chain::open(cfg.clone(), store) {
            Ok(opened) => opened,
            Err(e) if aether_node::follow::is_corruption(&e) => {
                let store = aether_node::follow::reset_store(std::path::Path::new(&data), &e).expect("move a corrupt database aside");
                Chain::open(cfg.clone(), store).expect("restore state after moving a corrupt database aside")
            }
            Err(e) => panic!("restore state (delete the data dir to resync): {e:?}"),
        };
        install_verifier(&chain, &data, false);
        // Self-healing (2026-09-29, docs/design/24-self-healing.md): serve the
        // public endpoint BEFORE catching up, from the stored finalized state —
        // status, balances, snapshots, era reads; read-only answers while this
        // node does not vote. Peers restarting at the same moment learn heights
        // from each other instead of waiting for someone to start voting first.
        // The served state is swapped for the full one (marshal finality
        // answers, handoff signing, prover, shards) once voting starts.
        let (gossip_tx, mut gossip_rx) = tokio::sync::mpsc::unbounded_channel::<TxEnvelope>();
        let served_snapshot: rpc::SnapshotCache = Default::default();
        let served_state = std::sync::Arc::new(std::sync::RwLock::new(rpc::RpcState {
            chain: chain.clone(),
            finality: rpc::Finality::Archive(std::sync::Arc::new(aether_node::follow::FinalityArchive::new(chain.store()))),
            gossip: gossip_tx.clone(),
            faucet: faucet_service.clone(),
            registrar: registrar.clone(),
            network: network_file.clone(),
            upstream: None,
            handoff: None,
            snapshot: served_snapshot.clone(),
            prover: None,
            shards: None,
        }));
        // Public access: iroh endpoint published to the BitTorrent Mainline DHT.
        // Wallets find this node by its id alone and verify everything they get;
        // validators tunnel consensus traffic over the same endpoint.
        // Wallet-server announcements are listed only for keys the finalized
        // registry state knows (red-team 2026-09-29 §3).
        let _router = endpoint.clone().map(|ep| {
            tracing::info!(node_id = %ep.id(), "public endpoint on iroh; address published to Mainline DHT (serving read-only answers while catching up)");
            let st = served_state.clone();
            let registry = aether_node::announce::checker(st.read().expect("served state").chain.clone());
            let p2p_target = links.then(|| loopback(port));
            aether_net::serve(
                ep,
                move |req| {
                    let st = st.read().expect("served state").clone();
                    async move { rpc::handle_value(&st, req).await }
                },
                p2p_target,
                Some(registry),
            )
        });
        // Catch up before voting: a committee member that slept must not
        // propose or vote on views it cannot execute (the committee treats it
        // as offline until then). Follow the network with the follower
        // machinery — a certified snapshot jump included — and only then start
        // the consensus engine, so not one vote exists while behind. The
        // network's height is defined by the roster's answers (2026-09-29): a
        // census asks every peer, and voting starts when no reachable peer is
        // ahead — so a network where every validator restarts at once
        // recovers on its own (each answers the census from its stored state;
        // the tallest proceeds first, then serves the rest its blocks). Only
        // silence fails open, after a real wait; `AETHER_SKIP_CATCH_UP`
        // overrides the asking for runbook recoveries. The one network that
        // needs no asking is a single validator (a local devnet): it is the
        // network, so its own finalized height is the height.
        if links && network_file.is_some() {
            let me = p2p.keys.node_secret.public();
            let nodes: Vec<_> = p2p.roster.nodes.iter().copied().filter(|n| *n != me).collect();
            if nodes.is_empty() {
                tracing::warn!("the roster names no other node: single-validator network, taking our own height as the network's");
                let ours = chain.finalized_height();
                chain.lock().net_height = Some(ours);
            } else if std::env::var_os("AETHER_SKIP_CATCH_UP").is_some() {
                tracing::warn!("AETHER_SKIP_CATCH_UP is set: starting without a confirmed network height");
            } else {
                let set = aether_light::ValidatorSet::new(*polynomial_identity);
                let ep = endpoint.clone().expect("iroh links serve the public endpoint");
                let census = {
                    let (ep, nodes) = (ep.clone(), nodes.clone());
                    move || {
                        let (ep, nodes) = (ep.clone(), nodes.clone());
                        async move { aether_node::follow::roster_heights(&ep, &nodes).await }
                    }
                };
                let upstream_of = {
                    let ep = ep.clone();
                    move |n: &aether_net::EndpointId| {
                        aether_node::follow::Upstream::Iroh(
                            aether_net::RpcClient::with_endpoint(ep.clone(), vec![*n]),
                            Default::default(),
                        )
                    }
                };
                let caught = aether_node::follow::catch_up_before_voting(
                    &chain,
                    &set,
                    aether_node::follow::BEHIND_MARGIN,
                    aether_node::follow::STARTUP_PATIENCE,
                    census,
                    upstream_of,
                )
                .await;
                if caught > 0 {
                    tracing::info!(height = chain.finalized_height(), blocks = caught, "caught up before voting");
                }
            }
            // A catch-up that timed out mid-replay is dropped without clearing
            // its replay mode: blocks from here on (voting) commit durably.
            chain.set_relaxed(false);
        }
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
        // A DKG committee hands over to the registry's voting set (a devnet dealer set cannot).
        let epoch_end = chain.lock().epoch_end.clone();
        let handoff_service = network_file.is_some().then(|| {
            let mut g = chain.lock();
            g.committee = aether_node::rotation::Committee { members: roster_members.clone() };
            g.identity = Some(*polynomial_identity);
            g.epoch_start = epochs.last().map(|e| e.height).unwrap_or(0);
            drop(g);
            chain.resume();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<aether_node::handoff::CommitteeMsg>();
            tokio::spawn(async move {
                while let Some(m) = rx.recv().await {
                    let _ = handoff_out.send(Recipients::All, serde_json::to_vec(&m).expect("partial serializes"), false);
                }
            });
            std::sync::Arc::new(aether_node::handoff::Service::new(
                chain.clone(),
                cfg.chain_id,
                handoff_share.clone(),
                handoff_sharing.clone(),
                std::path::PathBuf::from(&data),
                tx,
            ))
        });
        // Draw seeds: sign while a draw's pool is frozen and its seed is not on chain yet.
        if let Some(svc) = handoff_service.clone() {
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    svc.tick();
                }
            });
        }
        if let Some(svc) = handoff_service.clone() {
            tokio::spawn(async move {
                while let Ok((_peer, msg)) = handoff_in.recv().await {
                    if let Ok(m) = serde_json::from_slice::<aether_node::handoff::CommitteeMsg>(msg.as_ref()) {
                        if let Err(e) = svc.receive(&m) {
                            tracing::debug!(%e, "handoff partial rejected");
                        }
                    }
                }
            });
        }
        watch_upgrades(chain.clone(), std::path::Path::new(&data).join("upgrades"), *polynomial_identity, cfg.chain_id);
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
        let layout = engine::Layout::choose(history.prunes(), std::path::Path::new(&data), &partition_prefix(&data));
        if history.prunes() && layout == engine::Layout::Immutable {
            tracing::warn!("this node already keeps an immutable block archive: it prunes summaries and receipts, not marshal's blocks (start from a fresh data dir to prune those too)");
        }
        let engine = engine::Engine::new(
            context.child("engine"),
            engine::Config {
                anchor: load_anchor(&data),
                layout,
                blocker: oracle.clone(),
                provider: oracle.clone(),
                partition_prefix: partition_prefix(&data),
                me: signer.public_key(),
                scheme,
                identity: *polynomial_identity,
                epocher: aether_node::epochs::ScheduleEpocher::new(epochs.iter().map(|e| e.height).collect()).with_end(epoch_end),
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
        match &history {
            aether_node::prune::HistoryMode::Prune(r) => {
                tracing::info!(retain_blocks = r.blocks, keep_era_files = r.keep_era_files, "pruning history older than the retention window");
                tokio::spawn(aether_node::prune::run(chain.clone(), r.clone(), Some(marshal_mailbox.clone())));
            }
            aether_node::prune::HistoryMode::Archive => tracing::info!("archive node: keeping every block"),
        }

        // Mempool gossip: RPC-accepted txs go out, peers' txs come in.
        // Beacon answers and free-lane registrations (node rewards networks)
        // ride the same channel as `{"beacon": answer}` /
        // `{"registration": item}`; nodes that do not know them skip them as non-txs.
        // (The channel was made before catch-up: txs accepted from wallets
        // while catching up wait in it until the network starts here.)
        let (beacon_tx, mut beacon_rx) = tokio::sync::mpsc::unbounded_channel::<aether_light::block::BeaconAnswer>();
        let (registration_tx, mut registration_rx) = tokio::sync::mpsc::unbounded_channel::<aether_light::block::NodeRegistration>();
        chain.lock().beacon_out = Some(beacon_tx);
        chain.lock().registration_out = Some(registration_tx);
        tokio::spawn(async move {
            loop {
                let bytes = tokio::select! {
                    Some(tx) = gossip_rx.recv() => serde_json::to_vec(&tx).expect("tx serializes"),
                    Some(a) = beacon_rx.recv() => serde_json::to_vec(&json!({ "beacon": a })).expect("answer serializes"),
                    Some(r) = registration_rx.recv() => serde_json::to_vec(&json!({ "registration": r })).expect("registration serializes"),
                    else => break,
                };
                let _ = tx_out.send(Recipients::All, bytes, false);
            }
        });
        let gossip_chain = chain.clone();
        let chain_id = cfg.chain_id;
        tokio::spawn(async move {
            while let Ok((_peer, msg)) = tx_in.recv().await {
                if let Ok(tx) = serde_json::from_slice::<TxEnvelope>(msg.as_ref()) {
                    if aether_execution::validate_stateless(&tx, chain_id).is_ok() {
                        let _ = gossip_chain.add_to_mempool(tx);
                    }
                } else if let Some(a) = serde_json::from_slice::<Value>(msg.as_ref())
                    .ok()
                    .and_then(|v| serde_json::from_value::<aether_light::block::BeaconAnswer>(v.get("beacon")?.clone()).ok())
                {
                    // Peers' answers are pooled, not relayed again (every validator hears every peer).
                    let _ = gossip_chain.add_beacon(a);
                } else if let Some(r) = serde_json::from_slice::<Value>(msg.as_ref())
                    .ok()
                    .and_then(|v| serde_json::from_value::<aether_light::block::NodeRegistration>(v.get("registration")?.clone()).ok())
                {
                    let _ = gossip_chain.add_registration(r);
                }
            }
        });

        spawn_inclusion_lists(chain.clone(), signer.clone(), index, roster_keys, cfg.chain_id, Duration::from_millis(block_time_ms), il_out, il_in);

        // A registered voting node keeps proving it is alive while it votes.
        let mut shard_me = None;
        if network_file.is_some() {
            match aether_node::candidate::CandidateKeys::load_or_create(std::path::Path::new(&data)) {
                Ok(keys) => {
                    shard_me = Some(keys.node_id());
                    tokio::spawn(aether_node::candidate::beacon_loop(chain.clone(), aether_node::candidate::Outbox::Local(gossip_tx.clone()), keys));
                }
                Err(e) => tracing::warn!(%e, "no node account: this validator sends no liveness beacons"),
            }
        }

        // Era shards (roadmap B5 phase 1): hold what the draw assigns and
        // check peers. History v2 networks only; a validator encodes from the
        // era files it seals itself.
        let shards = cfg.history_v2.then(|| {
            let s = std::sync::Arc::new(aether_node::shards::Shards::new(std::path::Path::new(&data), shard_me, max_shards));
            tokio::spawn(aether_node::shards::run(chain.clone(), None, s.clone()));
            s
        });
        if let Some(f) = &faucet_service {
            tracing::info!(address = %f.address, "faucet enabled (aether_faucet)");
        }
        let prover = start_prover(&chain, &data, None);
        let rpc_state = RpcState {
            chain,
            finality: aether_node::rpc::Finality::Marshal(marshal_mailbox),
            gossip: gossip_tx,
            faucet: faucet_service,
            registrar,
            network: network_file,
            upstream: None,
            handoff: handoff_service,
            snapshot: served_snapshot,
            prover,
            shards,
        };
        // Voting machinery is up: swap the endpoint's served state for the
        // full one (marshal-backed finality answers, handoff signing, prover
        // status, era shards). In-flight snapshot downloads keep working: the
        // cache is the same one the read-only state served from.
        *served_state.write().expect("served state") = rpc_state.clone();

        let rpc_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), rpc_port);
        tracing::info!(%rpc_addr, "rpc listening");
        if let Err(e) = rpc::serve(rpc_addr, rpc_state).await {
            tracing::error!(?e, "rpc server stopped");
        }
    });
}

/// Stop (rather than fork off with old rules) one block before a committee-signed
/// upgrade this binary does not implement activates. The chain is the authority
/// (every node learns an activation the same way); signed upgrades in
/// `<data>/upgrades/*.json` (verified; unsigned or foreign files are ignored) are
/// put on chain by this node's proposals.
fn watch_upgrades(
    chain: Chain,
    dir: std::path::PathBuf,
    identity: aether_light::Identity,
    chain_id: u64,
) {
    use aether_node::upgrade::{load, protocol_at, PROTOCOL};
    std::thread::spawn(move || {
        let mut reported = 0;
        loop {
            let (ups, skipped) = load(&dir, &identity, chain_id);
            for s in &skipped {
                tracing::warn!(%s, "ignoring upgrade file");
            }
            if ups.len() != reported {
                reported = ups.len();
                for u in &ups {
                    tracing::info!(
                        protocol = u.upgrade.protocol,
                        activate_at = u.upgrade.activate_at,
                        "committee-signed upgrade"
                    );
                }
            }
            let (next, on_chain) = {
                let mut g = chain.lock();
                g.upgrades_known = ups.clone();
                (g.finalized.height + 2, g.finalized.schedule.clone())
            };
            // The chain is the authority: an upgrade counts once it is on chain.
            let need = protocol_at(&on_chain, next);
            if need > PROTOCOL {
                tracing::error!(need, have = PROTOCOL, height = next, "UPGRADE REQUIRED: this binary runs protocol {PROTOCOL} but the committee activated {need}; stopping before the new rules apply. Install the signed release.");
                std::process::exit(3);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

/// Proof verification (protocol 2), installed before consensus or replay
/// starts. Validators verify with the pinned sidecar (restarted if it dies);
/// followers execute only certified blocks, whose proofs the committee checked.
fn install_verifier(chain: &Chain, data: &str, follower: bool) {
    use aether_node::prover::{find_binary, Verifier};
    struct Certified;
    impl aether_node::chain::ProofVerifier for Certified {
        fn verify(&self, _: &[u8], _: [u8; 32]) -> bool {
            true
        }
    }
    if follower {
        chain.lock().verifier = Some(std::sync::Arc::new(Certified));
        return;
    }
    match find_binary().map(|bin| {
        Verifier::start(
            &bin,
            &std::path::Path::new(data).join("prover").join("verify"),
        )
    }) {
        Some(Ok(v)) => {
            tracing::info!(program = %v.program(), "proof verifier ready");
            chain.lock().verifier = Some(std::sync::Arc::new(v));
        }
        Some(Err(e)) => {
            tracing::error!(%e, "no proof verifier: this validator cannot vote for blocks carrying proofs")
        }
        None => tracing::error!(
            "aether-prover not found: this validator cannot vote for blocks carrying proofs"
        ),
    }
    // Under protocol 2 a validator that cannot verify proofs would vote against
    // every block carrying one: refuse to run as one until it is fixed.
    let g = chain.lock();
    if g.verifier.is_none() && g.finalized.schedule.iter().any(|a| a.protocol >= 2) {
        tracing::error!("protocol 2 is scheduled and this validator has no working proof verifier (aether-prover): stopping");
        // Exit codes: 3 upgrade required, 4 storage (store::EXIT_STORAGE),
        // 5 this one — the app restarts with backoff for none of them.
        std::process::exit(5);
    }
}

/// With `AETHER_PROVE=<payout address>` this node also proves blocks and hands
/// the proofs to its proposals (validator) or to a validator (follower).
fn start_prover(
    chain: &Chain,
    data: &str,
    upstream: Option<std::sync::Arc<aether_node::follow::Upstream>>,
) -> Option<aether_node::prover::SharedStatus> {
    use aether_node::prover::{find_binary, spawn_service, Sidecar};
    let payout: Address = std::env::var("AETHER_PROVE")
        .ok()?
        .parse()
        .map_err(|_| tracing::warn!("AETHER_PROVE is not an address"))
        .ok()?;
    // --prover-max-memory=0: the user turned the prover off.
    if aether_node::resources::monitor().is_some_and(|m| m.limits.prover_max_memory == 0) {
        tracing::info!("the prover is off (--prover-max-memory=0)");
        return None;
    }
    let dir = std::path::Path::new(data).join("prover");
    let bin_path = find_binary()?;
    let sidecar = match Some(Sidecar::spawn_prover(&bin_path, &dir.join("prove"))) {
        Some(Ok(sc)) => sc,
        Some(Err(e)) => {
            tracing::warn!(%e, "cannot start the prover");
            return None;
        }
        None => {
            tracing::warn!("AETHER_PROVE is set but aether-prover was not found");
            return None;
        }
    };
    let status = aether_node::prover::SharedStatus::default();
    let handle = tokio::runtime::Handle::current();
    let target = chain.clone();
    spawn_service(
        chain.clone(),
        bin_path,
        dir.join("prove"),
        sidecar,
        payout,
        status.clone(),
        move |claim| match &upstream {
            // The prover thread waits for the validator's answer (and retries on refusal).
            Some(up) => handle
                .block_on(up.first("aether_submitProof", json!([claim])))
                .map(|_| ()),
            None => target.add_own_proof(claim),
        },
    );
    tracing::info!(%payout, "proving blocks (rewards to this address)");
    Some(status)
}

/// Leave no orphan: stop when the parent process is gone (reparented to launchd).
fn exit_with_parent_process() {
    let parent = std::os::unix::process::parent_id();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        if std::os::unix::process::parent_id() != parent {
            std::process::exit(0);
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn run_follow(
    network: Option<String>,
    from_rpc: Vec<String>,
    data: String,
    rpc_port: u16,
    validators: u64,
    candidate_keys: Option<String>,
    dev_epoch_blocks: Option<u64>,
    checkpoint: bool,
    dev_storage_fault: Option<u64>,
    history: HistoryArgs,
    resources: ResourceArgs,
) -> Result<(), String> {
    use aether_node::follow::{self, FinalityArchive, Upstream};
    use std::sync::Arc;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,commonware=warn".into()),
        )
        .init();
    // Resource limits before the chain opens (the caches trim under the budget).
    aether_node::resources::install(resources.limits()?, std::path::Path::new(&data).to_path_buf());
    // A network.json with no faucet funds nobody (mainnet: 사전 발행 0).
    let dev_alloc = network.is_none();
    let (chain_id, genesis, set, nodes) = match network {
        Some(path) => {
            let file = aether_node::roster::NetworkFile::load(std::path::Path::new(&path))?;
            let identity = file.identity.clone().ok_or(
                "network.json has no committee identity: use the one written by dkg/reshare",
            )?;
            let set = aether_light::ValidatorSet::from_hex(&identity)
                .map_err(|e| format!("identity: {e:?}"))?;
            let nodes = aether_node::roster::Roster::from_file(&file)?.nodes;
            (file.chain_id, file.genesis()?, set, nodes)
        }
        None => {
            let mut genesis = aether_node::roster::Genesis::default();
            if let Some(e) = dev_epoch_blocks {
                genesis.epoch_blocks = e;
            }
            (
                DEFAULT_CHAIN_ID,
                genesis,
                aether_light::ValidatorSet::devnet(validators),
                (1..=validators).map(aether_net::devnet_node_id).collect(),
            )
        }
    };
    let cfg = chain_config(chain_id, &genesis, dev_alloc);
    // Followers run at the network's 1 s block time.
    let max_shards = history.max_shards;
    let history_v2 = cfg.history_v2;
    let history = history.mode(cfg.history_v2, 1000)?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async move {
        // Start-up integrity (docs/design/24-self-healing.md layer 1): a state
        // database that does not verify is moved aside (never deleted; the
        // keys stay) and re-syncs — a certified snapshot first, as below. The
        // hidden dev flag runs the same start on a disk that fills.
        let (store, _) = match dev_storage_fault {
            Some(ms) => {
                tracing::warn!(ms, "--dev-storage-fault: this disk fails from now on (self-healing test)");
                follow::open_store_with(
                    std::path::Path::new(&data),
                    std::sync::Arc::new(move |p| {
                        aether_node::store::open_with_a_disk_that_fills(p, Duration::from_millis(ms))
                    }),
                )?
            }
            None => follow::open_store(std::path::Path::new(&data))?,
        };
        // Following over iroh, this Mac also serves wallets directly (capacity
        // review 2026-09-29): a public endpoint under its own persisted node
        // id, so phones spread their reads over follower Macs instead of
        // asking the validators. `--from-rpc` followers have no iroh endpoint.
        let mut wallet_ep = None;
        let upstream = Arc::new(if from_rpc.is_empty() {
            let ep = aether_net::bind(Some(wallet_node_key(std::path::Path::new(&data))?), vec![aether_net::ALPN_RPC.to_vec()])
                .await
                .map_err(|e| e.to_string())?;
            let client = aether_net::RpcClient::with_endpoint(ep.clone(), nodes.clone());
            wallet_ep = Some(ep);
            Upstream::Iroh(client, Default::default())
        } else {
            Upstream::Http(from_rpc)
        });
        // A new Mac starts from a certified snapshot instead of replaying history.
        if checkpoint && store.head().map_err(|e| e.to_string())?.is_none() {
            // Without a usable snapshot, replay from genesis instead of failing to start.
            let attempt = tokio::time::timeout(Duration::from_secs(900), follow::checkpoint(&upstream, &set, &cfg, &store)).await;
            if let Err(e) = attempt.map_err(|_| "timed out".to_string()).and_then(|r| r) {
                tracing::warn!(%e, "checkpoint sync failed; replaying history from genesis");
            }
        }
        let (chain, _) = match Chain::open(cfg.clone(), store) {
            Ok(opened) => opened,
            // Bad data only the full check catches (the rebuilt state does not
            // match the checkpoint): the same recovery as at open — move the
            // file aside, never delete it, and start from a certified snapshot.
            Err(e) if follow::is_corruption(&e) => {
                let store = follow::reset_store(std::path::Path::new(&data), &e)?;
                if checkpoint {
                    let attempt = tokio::time::timeout(Duration::from_secs(900), follow::checkpoint(&upstream, &set, &cfg, &store)).await;
                    if let Err(e) = attempt.map_err(|_| "timed out".to_string()).and_then(|r| r) {
                        tracing::warn!(%e, "checkpoint sync failed; replaying history from genesis");
                    }
                }
                Chain::open(cfg, store).map_err(|e| format!("restore state (delete the data dir to resync): {e}"))?
            }
            Err(e) => return Err(format!("restore state (delete the data dir to resync): {e}")),
        };
        install_verifier(&chain, &data, true);
        let archive = Arc::new(FinalityArchive::new(chain.store()));
        let (gossip, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(follow::forward(upstream.clone(), rx));
        // Handoffs in blocks are checked against the committee identity, as validators check them.
        chain.lock().identity = Some(*set.identity());
        watch_upgrades(chain.clone(), std::path::Path::new(&data).join("upgrades"), *set.identity(), chain_id);
        let mut shard_me = None;
        if let Some(dir) = &candidate_keys {
            let keys = aether_node::candidate::CandidateKeys::load_or_create(std::path::Path::new(dir))?;
            tracing::info!(voting_key = %hex::encode(keys.validator_key()), beaconer = %keys.beaconer(), "voting-node candidate: beacons every epoch once registered");
            shard_me = Some(keys.node_id());
            tokio::spawn(aether_node::candidate::beacon_loop(chain.clone(), aether_node::candidate::Outbox::Upstream(upstream.clone()), keys));
        }
        let joining = candidate_keys
            .as_ref()
            .and_then(|dir| aether_node::candidate::CandidateKeys::load_or_create(std::path::Path::new(dir)).ok())
            .map(|k| hex::encode(k.validator_key()));
        tokio::spawn(follow::run(chain.clone(), upstream.clone(), set, archive.clone(), joining));
        if let aether_node::prune::HistoryMode::Prune(r) = &history {
            tokio::spawn(aether_node::prune::run(chain.clone(), r.clone(), None));
        }
        // Era shards (roadmap B5 phase 1): a follower fetches the eras it is
        // owed shards of over the B4 path when it no longer keeps their files.
        let shards = history_v2.then(|| {
            let s = std::sync::Arc::new(aether_node::shards::Shards::new(std::path::Path::new(&data), shard_me, max_shards));
            tokio::spawn(aether_node::shards::run(chain.clone(), Some(upstream.clone()), s.clone()));
            s
        });
        tracing::info!(height = chain.finalized_height(), rpc_port, "following (not a validator): every block is verified and re-executed here");
        let prover = start_prover(&chain, &data, Some(upstream.clone()));
        let st = RpcState {
            chain,
            finality: aether_node::rpc::Finality::Archive(archive),
            gossip,
            faucet: None,
            registrar: None,
            network: None,
            upstream: Some(upstream.clone()),
            handoff: None,
            snapshot: Default::default(),
            prover,
            shards,
        };
        // Serve wallets over the public endpoint (the same answers the loopback
        // HTTP server gives; every one is verified by the reader), and announce
        // this Mac as a wallet server to the validators, every minute, signed
        // by this Mac's voting key (a registered candidate's key — validators
        // list the announcement only then; red-team 2026-09-29 §3). The
        // router owns the endpoint, so it is bound to outlive this setup —
        // like the validators' `_router`, it must never drop while running.
        let announce_keys = candidate_keys
            .as_ref()
            .map(|dir| aether_node::candidate::CandidateKeys::load_or_create(std::path::Path::new(dir)))
            .transpose()?
            .map(std::sync::Arc::new);
        let _wallet_router = wallet_ep.map(|ep| {
            tracing::info!(node_id = %ep.id(), "serving wallets over iroh; announced to the validators every minute");
            let endpoint_id = ep.id();
            let st = st.clone();
            let router = aether_net::serve_rpc(ep, move |req| {
                let st = st.clone();
                async move { rpc::handle_value(&st, req).await }
            });
            let (announcer, keys) = (upstream.clone(), announce_keys.clone());
            tokio::spawn(async move {
                if keys.is_none() {
                    tracing::debug!("no candidate keys: serving wallets, but not announced (aether run --candidate)");
                }
                loop {
                    if let (Upstream::Iroh(c, _), Some(keys)) = (announcer.as_ref(), keys.as_ref()) {
                        let params = aether_node::announce::signed(keys, &endpoint_id);
                        if let Err(e) = c.call("aether_announceWalletServer", serde_json::json!(params)).await {
                            tracing::debug!(%e, "wallet-server announce failed; retrying in a minute");
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
            });
            router
        });
        rpc::serve(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), rpc_port), st).await.map_err(|e| e.to_string())
    })
}

fn run_dkg(
    p2p: P2pArgs,
    chain_id: u64,
    data: String,
    round: u64,
    genesis: aether_node::roster::Genesis,
) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,commonware=warn".into()),
        )
        .init();
    let dir = std::path::PathBuf::from(&data);
    std::fs::create_dir_all(&dir).expect("data dir");
    let out_path = dir.join("threshold.json");
    let executor = cw_tokio::Runner::new(
        cw_tokio::Config::new().with_storage_directory(dir.join("dkg-runtime")),
    );
    let (mut public, dir_out) = (p2p.roster.to_file(chain_id), dir.clone());
    // Genesis facts survive the ceremony: the network.json it writes still names the faucet.
    public.faucet = genesis.faucet;
    public.registrar = genesis
        .registrar
        .map(|(x, y)| format!("{}{}", hex::encode(x), hex::encode(y)));
    public.epoch_blocks = (genesis.epoch_blocks != 0).then_some(genesis.epoch_blocks);
    public.min_streak = genesis.min_streak;
    public.draw_epochs = genesis.draw_epochs;
    public.history = (genesis.history != 0).then_some(genesis.history);
    public.node_rewards = genesis.node_rewards.then_some(true);
    public.reserve = genesis.reserve.as_ref().map(|r| aether_node::roster::ReserveFile {
        operator: r.operator,
        validators: r.members.iter().map(|(key, node)| aether_node::roster::Member { key: key.clone(), node: node.clone() }).collect(),
    });
    let result = executor.start(async move |context| {
        // Accept incoming validator links (the node's RPC is not needed here).
        let _router = aether_node::p2p::open_public(&p2p)
            .await
            .map(|ep| aether_net::serve_p2p(ep, loopback(p2p.port)));
        let (mut network, mut oracle) = lookup::Network::new(
            context.child("network"),
            aether_node::p2p::config(&p2p, b"_DKG"),
        );
        oracle.track(0, aether_node::p2p::peer_addresses(&p2p));
        let (sender, receiver) = network.register(0, Quota::per_second(NZU32!(256)));
        network.start();
        tracing::info!(index = p2p.index, n = p2p.n, round, "dkg: started");
        aether_node::dkg::run(
            p2p.keys.signer.clone(),
            aether_node::dkg::Round::dkg(p2p.validators(), round),
            None,
            sender,
            receiver,
            Default::default(),
        )
        .await
    });
    match result {
        Ok(None) => unreachable!("every DKG participant is a player"),
        Ok(Some((output, share))) => {
            let file = aether_node::dkg::KeyFile::new(round, &output, &share);
            write_secret(
                &out_path,
                &serde_json::to_vec_pretty(&file).expect("key file serializes"),
            );
            // network.json now also carries the committee identity wallets pin.
            public.identity = Some(file.identity.clone());
            public.round = round;
            public.output = Some(file.output.clone());
            std::fs::write(
                dir_out.join("network.json"),
                serde_json::to_vec_pretty(&public).expect("json"),
            )
            .expect("write network.json");
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
    commonware_cryptography::bls12381::primitives::sharing::Sharing<
        commonware_cryptography::bls12381::primitives::variant::MinSig,
    >,
    commonware_cryptography::bls12381::primitives::group::Share,
) {
    let path = std::path::Path::new(data).join("threshold.json");
    if let Ok(bytes) = std::fs::read(&path) {
        let file: aether_node::dkg::KeyFile =
            serde_json::from_slice(&bytes).expect("threshold.json");
        if let Some(r) = expected_round {
            assert_eq!(
                file.round, r,
                "threshold.json is from key round {} but network.json expects {r}: use the network.json written by the last dkg/reshare",
                file.round
            );
        }
        let (output, share) = file
            .decode(validators.len() as u32)
            .expect("threshold.json decodes");
        assert_eq!(
            output.players(),
            validators,
            "threshold.json is for a different validator set"
        );
        tracing::info!(identity = %file.identity, "committee key from DKG");
        return (output.players().clone(), output.public().clone(), share);
    }
    let n = validators.len() as u64;
    assert_eq!(
        validators,
        &aether_node::p2p::validators(n),
        "no threshold.json for this network: run `aether dkg --network …` first"
    );
    tracing::warn!(
        "no threshold.json: using the devnet dealer's shares (every share is public knowledge)"
    );
    let (participants, polynomial, shares) = aether_light::devnet_threshold(n);
    let share = shares
        .into_iter()
        .find(|(pk, _)| pk == me)
        .map(|(_, s)| s)
        .expect("key is a validator");
    (participants, polynomial, share)
}

/// This Mac's wallet-server node key: dedicated and persisted (`<data>/wallet-node.key`),
/// so the DHT record it publishes does not flap with the endpoint other roles
/// reuse. Regenerating it only changes which node id wallets are pointed at.
fn wallet_node_key(data: &std::path::Path) -> Result<aether_net::SecretKey, String> {
    let path = data.join("wallet-node.key");
    match std::fs::read(&path) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut b = [0u8; 32];
            b.copy_from_slice(&bytes);
            Ok(aether_net::SecretKey::from_bytes(&b))
        }
        _ => {
            let key = aether_net::SecretKey::generate();
            write_secret(&path, &key.to_bytes());
            Ok(key)
        }
    }
}

fn write_secret(path: &std::path::Path, bytes: &[u8]) {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .expect("open key file");
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
            let _ = out.send(
                Recipients::All,
                serde_json::to_vec(&il).expect("list serializes"),
                false,
            );
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
            pub_chain
                .lock()
                .inclusion
                .accept(&il, std::time::Instant::now());
            tracing::info!(height, txs = il.txs.len(), "published inclusion list");
            let _ = pub_send.send(il);
        }
    });
    tokio::spawn(async move {
        while let Ok((_peer, msg)) = inbox.recv().await {
            let Ok(il) = serde_json::from_slice::<InclusionList>(msg.as_ref()) else {
                continue;
            };
            let fin = chain.lock().finalized.height;
            if il.height + 16 < fin || il.height > fin + 16 {
                continue;
            }
            if let Err(e) = il.verify(&validators, chain_id) {
                tracing::debug!(member = il.member, ?e, "rejected inclusion list");
                continue;
            }
            let fresh = chain
                .lock()
                .inclusion
                .accept(&il, std::time::Instant::now());
            if fresh {
                let _ = send_tx.send(il);
            }
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

fn dev_address(dev: u8) -> Result<Address, String> {
    let s = P256Signer::from_seed(&dev_seed(dev)).map_err(|e| e.to_string())?;
    aether_crypto::address_of(&s.public_key()).map_err(|e| e.to_string())
}

/// Fee caps from the node's next base fees: 2x headroom (~70 full blocks of
/// growth) plus a 1 gwei tip; only the actual base + tip is charged.
const TIP: u128 = 1_000_000_000;

fn fee_caps(status: &Value, tip: u128) -> Result<aether_types::FeeVector, String> {
    let get = |k: &str| {
        status["base_fee"][k]
            .as_str()
            .and_then(|v| v.parse::<u128>().ok())
            .ok_or(format!("status has no base_fee.{k}"))
    };
    Ok(aether_types::FeeVector {
        exec: get("exec")? * 2 + tip,
        state: 0,
        prove: get("prove")? * 2,
    })
}

fn submit(rpc: &str, dev: u8, nonce: Option<u64>, c: EvmCall, wait: bool) -> Result<Value, String> {
    submit_with_tip(rpc, dev, nonce, c, wait, TIP)
}

/// `submit` with an explicit priority fee: 0 lets a zero-balance account send
/// (its tx only needs the base fee, which an empty block still covers).
fn submit_with_tip(rpc: &str, dev: u8, nonce: Option<u64>, c: EvmCall, wait: bool, tip: u128) -> Result<Value, String> {
    let signer = P256Signer::from_seed(&dev_seed(dev)).map_err(|e| e.to_string())?;
    let from = aether_crypto::address_of(&signer.public_key()).map_err(|e| e.to_string())?;
    let status = call(rpc, "aether_status", json!([]))?;
    let chain_id = status["chain_id"].as_u64().ok_or("no chain id")?;
    let nonce = match nonce {
        Some(n) => n,
        None => {
            let hex = call(rpc, "eth_getTransactionCount", json!([from]))?;
            u64::from_str_radix(hex.as_str().unwrap_or("0x0").trim_start_matches("0x"), 16)
                .map_err(|e| e.to_string())?
        }
    };
    let tx = sign_call_with(&signer, chain_id, nonce, fee_caps(&status, tip)?, tip, &c)
        .map_err(|e| e.to_string())?;
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

/// Fetch the finalized child block H+1 that commits to the state root after H,
/// and verify its certificate against the validator set.
fn trusted(
    validators: u64,
    identity: Option<String>,
) -> Result<aether_light::ValidatorSet, String> {
    match identity {
        Some(hex) => {
            aether_light::ValidatorSet::from_hex(&hex).map_err(|e| format!("identity: {e}"))
        }
        None => Ok(aether_light::ValidatorSet::devnet(validators)),
    }
}

fn certified_anchor(
    rpc: &str,
    height: u64,
    set: &aether_light::ValidatorSet,
) -> Result<aether_light::VerifiedBlock, String> {
    for _ in 0..40 {
        let v = call(rpc, "aether_getFinalized", json!([height + 1]))?;
        if !v.is_null() {
            let block = aether_light::from_hex(v["block"].as_str().unwrap_or_default())
                .map_err(|e| e.to_string())?;
            let fin = aether_light::from_hex(v["finalization"].as_str().unwrap_or_default())
                .map_err(|e| e.to_string())?;
            let links = v["links"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|l| aether_light::from_hex(l.as_str().unwrap_or_default()))
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            return aether_light::verify_finalized_chain(set, &block, &fin, &links)
                .map_err(|e| format!("CERTIFICATE REJECTED: {e} — do not trust this server"));
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
    let data = aether_light::verify_account(&anchor, &a, &proof)
        .map_err(|e| format!("PROOF REJECTED: {e}"))?
        .unwrap_or_default();
    let claimed: U256 = serde_json::from_value(v["balance"].clone()).map_err(|e| e.to_string())?;
    if U256::from(data.balance) != claimed {
        return Err(format!(
            "server claimed {claimed} but the proof says {}",
            data.balance
        ));
    }
    println!("address   {a}");
    println!("balance   {} wei", data.balance);
    println!("nonce     {}", data.nonce);
    println!("verified  ✓ finality certificate of block {}: one BLS threshold signature under committee key {}…", anchor.height, &set.identity_hex()[..16]);
    println!(
        "          ✓ it commits state root {} (after block {height})",
        anchor.parent_state_root
    );
    println!("          ✓ EIP-7864 proof for this address verifies under that root");
    Ok(())
}

fn verified_storage(
    rpc: &str,
    a: Address,
    slot: U256,
    set: &aether_light::ValidatorSet,
) -> Result<(), String> {
    let v = call(rpc, "aether_getStorage", json!([a, slot]))?;
    let proof: Proof = serde_json::from_value(v["proof"].clone()).map_err(|e| e.to_string())?;
    let height = v["height"].as_u64().unwrap_or_default();
    let anchor = certified_anchor(rpc, height, set)?;
    let value = aether_light::verify_storage(&anchor, &a, slot, &proof)
        .map_err(|e| format!("PROOF REJECTED: {e}"))?;
    println!("{a}[{slot}] = {value}");
    println!(
        "verified  ✓ finality certificate of block {} + proof under committed root {}",
        anchor.height, anchor.parent_state_root
    );
    Ok(())
}

fn print_blocks(v: &Value) {
    println!(
        "{:>7}  {:>4}  {:>9}  {:<66}  proposer",
        "height", "txs", "gas", "state root after block"
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The resource flags parse on `node`, `follow` and `run`, resolve to the
    /// sizes they name, and `run` forwards them verbatim to its children.
    #[test]
    fn resource_flags_parse_and_resolve() {
        let c = Cli::try_parse_from([
            "aether", "node", "--port", "1", "--rpc-port", "2", "--data", "d",
            "--prover-max-memory=8", "--prover-threads=6", "--prover-on-battery",
            "--max-memory=512M", "--min-free-disk=10G",
        ])
        .expect("node parses");
        let Cmd::Node { resources, .. } = c.cmd else { panic!("node") };
        assert_eq!(
            resources.forward(),
            vec![
                "--prover-max-memory=8",
                "--prover-threads=6",
                "--prover-on-battery",
                "--max-memory=512M",
                "--min-free-disk=10G",
            ]
        );
        let l = resources.limits().unwrap();
        assert_eq!(l.prover_max_memory, 8 * aether_node::resources::GB);
        assert_eq!(l.prover_threads, 6);
        assert!(l.prover_on_battery);
        assert_eq!(l.max_memory, 512 * 1024 * 1024);
        assert_eq!(l.min_free_disk, 10 * aether_node::resources::GB);

        // 0 turns the prover (and the disk guard) off, and forwards as-is.
        let c = Cli::try_parse_from(["aether", "run", "--data", "d", "--prover-max-memory=0"]).expect("run parses");
        let Cmd::Run { resources, .. } = c.cmd else { panic!("run") };
        assert_eq!(resources.forward(), vec!["--prover-max-memory=0"]);
        assert_eq!(resources.limits().unwrap().prover_max_memory, 0);

        let c = Cli::try_parse_from(["aether", "follow", "--data", "d", "--min-free-disk=0"]).expect("follow parses");
        let Cmd::Follow { resources, .. } = c.cmd else { panic!("follow") };
        assert_eq!(resources.limits().unwrap().min_free_disk, 0);

        // A size that is not a size says so.
        let c = Cli::try_parse_from(["aether", "node", "--port", "1", "--rpc-port", "2", "--data", "d", "--max-memory=lots"]).unwrap();
        let Cmd::Node { resources, .. } = c.cmd else { panic!("node") };
        assert!(resources.limits().is_err());
    }

    /// `raise_nofile_limit` reports the soft limit that is really in effect,
    /// never lowers it, raises a low inherited one (launchd's 256) as far as
    /// the hard limit and the kernel's per-process cap allow, and is idempotent.
    #[test]
    #[cfg(unix)]
    fn the_open_file_limit_is_raised_never_lowered() {
        let before = nofile().expect("read the open-file limit");
        let raised = raise_nofile_limit();
        let after = nofile().expect("read the open-file limit");
        assert_eq!(raised, after.0, "it reports the limit now in effect");
        assert!(after.0 >= before.0, "never lowers the soft limit");
        assert!(after.0 <= NOFILE_WANT.max(before.0), "asks for at most {NOFILE_WANT}");
        let ceiling = NOFILE_WANT.min(before.1).min(nofile_per_proc());
        if before.0 < ceiling {
            assert!(after.0 > before.0, "a low inherited limit was not raised");
        }
        assert_eq!(raise_nofile_limit(), after.0, "raising again changes nothing");
    }
}
