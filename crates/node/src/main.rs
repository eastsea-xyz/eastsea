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
    /// receipts). Default: an eighth of the RAM, between 1 GB and 4 GB.
    #[arg(long = "max-memory", value_name = "SIZE")]
    max_memory: Option<String>,
    /// Below this free-space floor, the node stops before further consensus
    /// and store writes; aether run resumes it after space returns. Default:
    /// 5 GB. 0 = off.
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
        /// The ceremony record (ceremony-check.json) the checked genesis is
        /// bound to before voting (audit 6). Default: <data>/ceremony-check.json,
        /// where verify-local stores it.
        #[arg(long)]
        ceremony: Option<String>,
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
        /// Sign registrar attestations with the Secure Enclave helper
        /// (`apps/registrar-signer`, docs/ops/registrar.md) listening on this
        /// Unix socket. Without it, attestations are signed with
        /// <data>/registrar.key — local devnets and rehearsals.
        #[arg(long)]
        registrar_signer: Option<String>,
        /// Test chains (no faucet): register every device without Apple (public dev registrar key).
        #[arg(long, hide = true)]
        dev_registrar: bool,
        /// Local devnet: blocks per voting-node epoch.
        #[arg(long, hide = true, conflicts_with = "network")]
        dev_epoch_blocks: Option<u64>,
        /// Exit when the launching process does (`aether run`, the Mac app).
        #[arg(long)]
        exit_with_parent: bool,
        /// Serve only the explorer's read methods, with caps, and bind loopback
        /// (docs/ops/read-gateway.md). Exposure is a cloudflared tunnel's job.
        #[arg(long)]
        public_read_only: bool,
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
        /// New-genesis ceremony round. Increase this after a failed ceremony
        /// so a retry uses a new journal and fresh dealer randomness.
        #[arg(long)]
        round: Option<u64>,
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
    /// The protocol this binary implements. Hidden: the app asks a candidate
    /// rollback binary (`Helpers/aether.prev`) this before returning to it —
    /// a rollback to a binary that cannot run the chain's scheduled protocol
    /// would stop the node for good (red team #3).
    #[command(hide = true)]
    Protocol,
    /// The proof program the validators of <network> verify with, by their
    /// `aether_proverProgram` answer (validators that predate it: the program
    /// compiled in for their chain). Hidden: the release gate
    /// (scripts/prover-gate.sh) refuses to package an app whose prover differs.
    #[command(hide = true)]
    ValidatorProgram {
        #[arg(long)]
        network: String,
        /// Ask these HTTP JSON-RPC endpoints instead of the validators over iroh.
        #[arg(long)]
        rpc: Vec<String>,
        /// Seconds to wait for an answer.
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
    /// BLAKE3 of a file, hex (dev-drill feature): scripts/upgrade-drill.sh
    /// hashes its simulated releases with it for the committee upgrade's
    /// releases[] record. No shipped build has this subcommand.
    #[cfg(feature = "dev-drill")]
    #[command(hide = true)]
    DevB3 { file: String },
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
        /// Serve only the explorer's read methods, with caps, and bind loopback
        /// (docs/ops/read-gateway.md). Exposure is a cloudflared tunnel's job.
        #[arg(long)]
        public_read_only: bool,
        /// Archive mode under `aether run --archive`: keep everything like
        /// `aether archive` (archive history, no snapshot start, no snapshot
        /// jump) and write the era file set to this directory. The RPC stays
        /// on loopback and candidate beacons keep working.
        #[arg(long)]
        archive_export: Option<String>,
        /// This Mac's wallet-server endpoint key (default: <data>/wallet-node.key).
        /// `aether run` keeps it on the internal disk when the chain data
        /// lives elsewhere, so the node id never changes with the disk.
        #[arg(long)]
        node_key: Option<String>,
        #[command(flatten)]
        history: HistoryArgs,
        #[command(flatten)]
        resources: ResourceArgs,
    },
    /// Keep everything, forever (roadmap B6): a follower that never prunes,
    /// serves old blocks and eras to pruned peers (`aether_eraInfo`,
    /// `aether_eraChunk`, `aether_eraProof`, `GET /era/<file>`), and writes
    /// every completed era as a static, torrent-ready file set a mirror
    /// (the NAS, GitHub Releases) can serve as-is. No voting, no proving, no
    /// registration: this is the box history lives on.
    Archive {
        /// network.json of the chain to archive.
        #[arg(long)]
        network: String,
        /// Validators' RPC URLs to pull from (default: find them on the Mainline DHT).
        #[arg(long, value_delimiter = ',')]
        from_rpc: Vec<String>,
        #[arg(long)]
        data: String,
        #[arg(long, default_value_t = 8545)]
        rpc_port: u16,
        /// Directory the era file set is written to (served by a mirror).
        #[arg(long)]
        export_dir: String,
        /// Extra webseed URL for the torrents: a bare URL gets the file name
        /// appended, `~name~` takes it where it falls (a GitHub Releases
        /// placeholder, for example).
        #[arg(long = "webseed", value_delimiter = ',')]
        webseed: Vec<String>,
        /// This node's public base URL (http://host:port) for the manifest's
        /// Https mirror and first webseed; its `/era/<file>` serves the bytes.
        #[arg(long)]
        https_base: Option<String>,
        /// Address the RPC (and webseed) server listens on. An archive node is
        /// a server others fetch from, so it listens everywhere by default,
        /// unlike a follower's loopback-only RPC.
        #[arg(long, default_value = "0.0.0.0")]
        bind: IpAddr,
        /// Ed25519 key signing the manifests (hex seed; created when missing).
        #[arg(long)]
        export_key: Option<String>,
        #[command(flatten)]
        resources: ResourceArgs,
    },
    /// Keep this Mac in the network: validator while in the voting set, verifying
    /// follower and candidate otherwise; rotations are followed automatically.
    Run {
        #[arg(long)]
        data: String,
        /// Where the bulky chain data goes (a secondary disk): the follower's
        /// `follow` (or `archive`) directory lives in <chain-data>; keys,
        /// network.json, run.lock and the validator's journals stay in
        /// <data>. The directory must exist: a missing one (an unplugged
        /// disk) exits 13 instead of filling the internal disk.
        #[arg(long)]
        chain_data: Option<String>,
        /// Keep the full history like `aether archive`: replay from genesis,
        /// never a snapshot jump, era files exported, in <chain dir>/archive
        /// (apart from `follow`, so turning it off returns to a normal
        /// follower and the archive can be kept or deleted).
        #[arg(long)]
        archive: bool,
        /// network.json to start from (copied into <data> the first time).
        #[arg(long)]
        network: Option<String>,
        /// The ceremony record (ceremony-check.json) the adopted genesis is
        /// bound to before voting (audit 6). Default: <data>/ceremony-check.json,
        /// where verify-local stores it.
        #[arg(long)]
        ceremony: Option<String>,
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
        #[arg(long)]
        reshare_timeout: Option<u64>,
        #[arg(long, hide = true)]
        dev_peer_dir: Option<String>,
        /// Exit when the launching app does (the Mac app's node switch).
        #[arg(long)]
        exit_with_parent: bool,
        /// The gateway role for this Mac: whichever child runs (`aether node` or
        /// `aether follow`) serves only the explorer's read methods with caps,
        /// on loopback, behind a cloudflared tunnel (docs/ops/read-gateway.md).
        #[arg(long)]
        public_read_only: bool,
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
        /// Priority fee (wei) for the legacy contract path; ignored on the free lane.
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
        /// The consensus group this chain is (13-roadmap.md, 그룹 분열 준비): 0 — the
        /// default — is the only group today; any other group is a new genesis of its own.
        #[arg(long)]
        group: Option<u16>,
        /// Seats the voting committee grows to before draws swap instead of add
        /// (default 16, at least 4, at most 128).
        #[arg(long)]
        max_committee: Option<u64>,
        /// Release config (JSON, `{"builder_keys": [three 04‖x‖y hex]}`): writes the app's
        /// release-approval pin — the ReleaseLog predeploy, its code hash, the builder keys,
        /// 2/3 normal and 3/3 emergency (docs/design/19). A new genesis needs it (mainnet rule
        /// "release pin"); 7780 never has one.
        #[arg(long)]
        release: Option<String>,
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
        /// Allow shortened epoch and candidate timing (a rehearsal must finish in
        /// minutes). Reported in the output; never use it for the real launch.
        #[arg(long)]
        rehearsal: bool,
        /// The release gate: also demand the coordinator's ceremony record next
        /// to the network file, pinning its exact bytes (what an app build
        /// ships — `apps/wallet/Resources`). Off for the ceremony's own check,
        /// which writes the record only after PASS. A bundle that is not a new
        /// genesis (the legacy 7780 testnet app) runs the record rule alone —
        /// the mainnet genesis rules are not asked of a testnet file.
        #[arg(long)]
        bundle: bool,
    },
    /// Write the ceremony record (ceremony-check.json) for a final network.json
    /// that passed `check` (scripts/mainnet-genesis.sh, audit 6): pins the
    /// chain id, DKG round, committee identity, the sha256 of the exact bytes
    /// that passed and the immutable genesis. Public; copy it to every
    /// validator Mac alongside the final network.json — verify-local and the
    /// node refuse to vote without it.
    CeremonyRecord {
        /// The final network.json that passed check.
        #[arg(long)]
        network: String,
        /// Where to write the record (default: next to the network file).
        #[arg(long)]
        out: Option<String>,
    },
    /// Bind a data dir to the checked genesis (verify-local's engine, audit 6):
    /// the final file, this Mac's network.json/threshold.json and the ceremony
    /// record through one fail-closed comparison. Ok = this Mac votes only
    /// under the committee the checked file carries.
    MainnetBind {
        #[arg(long)]
        network: String,
        #[arg(long)]
        data: String,
        #[arg(long)]
        ceremony: Option<String>,
    },
    /// Replay finalized blocks with this binary into an isolated scratch store.
    Shadow {
        /// A stopped history-v2 data directory or an archive peer's HTTP RPC URL.
        #[arg(long)]
        from: String,
        /// Last finalized height to check (inclusive).
        #[arg(long)]
        to: u64,
        /// Genesis network.json when it is outside the source data directory.
        #[arg(long)]
        network: Option<std::path::PathBuf>,
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
    /// Pay several addresses in ONE signed tx (EIP-7702 delegation to EastSeaAccount).
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
        /// Exec gas limit to sign (default 3,000,000; the wallet path sets it per call).
        #[arg(long)]
        gas: Option<u64>,
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
        /// Native value to send with the call, wei (the wallet path carries value with data).
        #[arg(long)]
        value: Option<U256>,
        /// Exec gas limit to sign (default 1,000,000).
        #[arg(long)]
        gas: Option<u64>,
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
    // Adopt before CLI dispatch or any unrelated helper can spawn. The guard
    // lives until main exits, including parent death during writer startup.
    let _writer_lease = aether_node::supervisor::inherited_writer_lease().unwrap_or_else(|e| {
        eprintln!("refusing invalid supervisor writer lease: {e}");
        std::process::exit(aether_node::supervisor::EXIT_LOCKED);
    });
    std::env::remove_var(aether_node::supervisor::WRITER_LEASE_ENV);
    let cli = Cli::parse();
    let res = match cli.cmd {
        Cmd::Node {
            index,
            validators,
            network,
            ceremony,
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
            registrar_signer,
            dev_registrar,
            dev_epoch_blocks,
            exit_with_parent,
            public_read_only,
            history,
            resources,
        } => {
            if exit_with_parent {
                exit_with_parent_process(_writer_lease.as_ref().map(|lease| lease.expected_parent()));
            }
            // A seated validator whose key file is gone or unreadable stops
            // with its own exit code (red team #5): it must not be replaced by
            // a devnet stand-in or a fresh identity.
            {
                let dir = std::path::Path::new(&data);
                if network.is_some()
                    && dir.join("threshold.json").exists()
                    && aether_node::roster::LocalKeys::load(dir).is_err()
                {
                    eprintln!(
                        "this Mac's validator key cannot be read but it holds a committee \
                         share: no new identity is generated. Restore {}/{} from a backup, or \
                         unregister this Mac and register a new one on purpose",
                        dir.display(),
                        aether_node::roster::KEY_FILE
                    );
                    std::process::exit(aether_node::candidate::EXIT_IDENTITY);
                }
            }
            let with_file = network.is_some();
            // Audit 6, A6-3 + A6-4: a validator votes only under the genesis
            // the ceremony checked. Before anything starts, bind the checked
            // --network file, this Mac's network.json/threshold.json and the
            // ceremony record through one fail-closed comparison; a new
            // genesis without a record refuses here, with the operator step.
            bind_to_checked_genesis(network.as_deref(), &data, ceremony.as_deref());
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
                    // The Secure Enclave helper (apps/registrar-signer) replaces
                    // the file key of a real DeviceCheck registrar, nothing else:
                    // there is no registrar service to sign for otherwise.
                    match (&registrar_signer, &devicecheck_key, dev_registrar) {
                        (Some(_), _, true) => {
                            return Err("--registrar-signer conflicts with --dev-registrar: the dev registrar signs with the public dev key".into())
                        }
                        (Some(_), None, _) => {
                            return Err("--registrar-signer needs --devicecheck-key: the helper signs the attestations of a DeviceCheck registrar".into())
                        }
                        _ => {}
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
                    let new_genesis = args.4.node_rewards || args.4.history >= 2;
                    let effective_ms = if new_genesis { block_time_ms.max(aether_node::application::MIN_BLOCK_INTERVAL_MS) } else { block_time_ms };
                    let mode = history.mode(args.4.history >= 2, effective_ms)?;
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
                        registrar_signer,
                        dev_registrar,
                        network_file,
                        public_read_only,
                        resources,
                    });
                })
        }
        Cmd::Keygen { data } => keygen(&data),
        Cmd::UpgradeSign { data, network, upgrade } => (|| {
            use aether_node::upgrade::{sign_emergency_partial, sign_partial, Upgrade};
            let file = aether_node::roster::NetworkFile::load(std::path::Path::new(&network))?;
            let key: aether_node::dkg::KeyFile =
                serde_json::from_slice(&std::fs::read(std::path::Path::new(&data).join("threshold.json")).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let (_, share) = key.decode(file.validators.len() as u32)?;
            let u: Upgrade = serde_json::from_slice(&std::fs::read(&upgrade).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            if u.chain_id != file.chain_id {
                return Err(format!("upgrade is for chain {}, network.json for {}", u.chain_id, file.chain_id));
            }
            let partial = if u.emergency {
                let keys = aether_node::roster::LocalKeys::load(std::path::Path::new(&data))?;
                sign_emergency_partial(&u, &share, &keys.signer)
            } else {
                sign_partial(&u, &share)
            };
            println!("{}", serde_json::to_string_pretty(&partial).expect("json"));
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
            if signed.upgrade.emergency {
                let committee: Vec<_> = file.validators.iter().map(|m| (m.key.trim_start_matches("0x").to_lowercase(), m.node.clone())).collect();
                aether_node::upgrade::verify_emergency(&signed, &committee)?;
            }
            println!("{}", serde_json::to_string_pretty(&signed).expect("json"));
            Ok(())
        })(),
        Cmd::UpgradeVerify { network, signed } => (|| {
            let file = aether_node::roster::NetworkFile::load(std::path::Path::new(&network))?;
            let set = aether_light::ValidatorSet::from_hex(file.identity.as_deref().ok_or("network.json has no identity")?).map_err(|e| format!("{e:?}"))?;
            let s: aether_node::upgrade::SignedUpgrade =
                serde_json::from_slice(&std::fs::read(&signed).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            aether_node::upgrade::verify(set.identity(), &s)?;
            if s.upgrade.emergency {
                let committee: Vec<_> = file.validators.iter().map(|m| (m.key.trim_start_matches("0x").to_lowercase(), m.node.clone())).collect();
                aether_node::upgrade::verify_emergency(&s, &committee)?;
            }
            println!("signed by the committee: protocol {} at height {} on chain {}", s.upgrade.protocol, s.upgrade.activate_at, s.upgrade.chain_id);
            Ok(())
        })(),
        Cmd::Follow { network, from_rpc, data, rpc_port, validators, exit_with_parent, candidate, dev_epoch_blocks, keys, checkpoint, dev_storage_fault, public_read_only, archive_export, node_key, history, resources } => {
            if exit_with_parent {
                exit_with_parent_process(_writer_lease.as_ref().map(|lease| lease.expected_parent()));
            }
            let keys = candidate.then(|| keys.unwrap_or_else(|| data.clone()));
            let export = follow_export(archive_export, &data);
            if let Some(e) = &export {
                if let Err(err) = std::fs::create_dir_all(&e.dir) {
                    eprintln!("{}: {err}", e.dir.display());
                    std::process::exit(1);
                }
            }
            let node_key = node_key.map(std::path::PathBuf::from).unwrap_or_else(|| std::path::Path::new(&data).join("wallet-node.key"));
            run_follow(network, from_rpc, data, rpc_port, validators, keys, dev_epoch_blocks, checkpoint, dev_storage_fault, public_read_only, history, resources, export, None, node_key)
        }
        Cmd::Archive { network, from_rpc, data, rpc_port, export_dir, webseed, https_base, bind, export_key, resources } => run_archive(network, from_rpc, data, rpc_port, export_dir, webseed, https_base, bind, export_key, resources),
        Cmd::CandidateInfo { data, operator, chain_id } => (|| {
            let dir = std::path::Path::new(&data);
            let k = match aether_node::candidate::CandidateKeys::load_or_create(dir) {
                Ok(k) => k,
                // A lost identity is its own exit code (red team #5): the app
                // shows the one sentence instead of a generic failure.
                Err(e) if aether_node::candidate::registered_identity(dir) => {
                    eprintln!("{e}");
                    std::process::exit(aether_node::candidate::EXIT_IDENTITY);
                }
                Err(e) => return Err(e),
            };
            let ownership = operator.zip(chain_id).map(|(op, id)| hex::encode(k.ownership(id, op)));
            println!(
                "{}",
                json!({ "validator_key": hex::encode(k.validator_key()), "node_id": hex::encode(k.node_id()), "beaconer": k.beaconer(), "ownership": ownership })
            );
            Ok(())
        })(),
        Cmd::Run { data, chain_data, archive, network, ceremony, port, rpc_port, reshare_port, node_args, follow_args, reshare_timeout, dev_peer_dir, exit_with_parent, public_read_only, resources } => {
            // First: the app's wake signal must never end the supervisor.
            aether_node::supervisor::install_wake_forwarding();
            if exit_with_parent {
                exit_with_parent_process(_writer_lease.as_ref().map(|lease| lease.expected_parent()));
            }
            tracing_subscriber::fmt()
                .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,commonware=warn".into()))
                .init();
            (|| {
                let dir = std::path::PathBuf::from(&data);
                // A chain-data disk that is not connected: stop before any
                // write at all (never create its /Volumes path — that would
                // re-sync the chain onto the internal disk).
                if let Some(chain) = chain_data.as_deref().map(std::path::Path::new) {
                    if !chain.is_dir() {
                        eprintln!("the chain data directory {} does not exist (the disk is not connected); stopping without writing anything", chain.display());
                        std::process::exit(aether_node::supervisor::EXIT_CHAIN_DATA_MISSING);
                    }
                }
                // Keys must stay on this Mac (design 36 §6.2): never read them
                // from the chain-data disk, and never run with the key
                // directory itself on a removable or network volume.
                if let Some(chain) = chain_data.as_deref().map(std::path::Path::new) {
                    let found = aether_node::supervisor::keys_in_chain_data(chain);
                    if !found.is_empty() {
                        eprintln!("keys must stay on this Mac: key files found in the chain data directory ({}); refusing to start",
                            found.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
                        std::process::exit(aether_node::supervisor::EXIT_KEYS_ON_CHAIN_DATA);
                    }
                }
                if aether_node::supervisor::keys_on_external_data(&dir, aether_node::supervisor::volume_is_external(&dir)) {
                    eprintln!("keys must stay on this Mac: the key directory {} is on a removable or network volume; refusing to start", dir.display());
                    std::process::exit(aether_node::supervisor::EXIT_KEYS_ON_CHAIN_DATA);
                }
                // The first-run key and network setup below can write before
                // Supervisor::run starts. Wait on the volume the chain data
                // is written to (the chosen disk when there is one).
                let floor_dir = chain_data.as_deref().map(std::path::PathBuf::from).unwrap_or_else(|| dir.clone());
                aether_node::supervisor::wait_for_data_disk(&floor_dir, resources.limits()?.min_free_disk, false);
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                // One data directory, one `aether` (red team #12): a second
                // app's run stops before touching anything, with its own exit
                // code — "already running" is not a crash to restart.
                let _lock = match aether_node::supervisor::lock_data_dir(&dir) {
                    Ok(l) => l,
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(aether_node::supervisor::EXIT_LOCKED);
                    }
                };
                // First install: create the keys. A directory that ever held an
                // identity refuses instead (red team #5) — and `aether run`
                // goes on as a follower without them, never a new identity.
                if let Err(e) = aether_node::candidate::CandidateKeys::load_or_create(&dir) {
                    tracing::error!(%e, "aether run: this Mac's identity cannot be loaded; running as a follower");
                }
                let chain_dir = chain_data.as_deref().map(std::path::Path::new);
                // A durable adoption already passed preflight. Resume it
                // before reading a source the interrupted move may have
                // quarantined; bind_run_to_ceremony still checks the adopted
                // file before any child starts.
                if !aether_node::supervisor::resume_network_adoption(&dir, chain_dir)? {
                    bind_incoming_network(&dir, network.as_deref(), ceremony.as_deref())?;
                    aether_node::supervisor::adopt_network_with_chain_data(
                        &dir, chain_dir, network.as_deref().map(std::path::Path::new),
                    )?;
                }
                // A committee install a previous run did not finish (red team
                // #19): complete it before the ceremony bind or any role decision reads the files.
                if let Err(e) = aether_node::supervisor::finish_incomplete(&dir) {
                    eprintln!("a completed handoff cannot be installed: {e}");
                    std::process::exit(aether_node::store::EXIT_STORAGE);
                }
                // Audit 6: after adopt_network, bind the adopted genesis to
                // the ceremony record before any child can vote (the record
                // verify-local stored in the data dir, --ceremony, or the one
                // shipped next to the --network file). The resolved record is
                // handed to the child as --ceremony.
                let ceremony = match bind_run_to_ceremony(&dir, network.as_deref(), ceremony.as_deref()) {
                    Ok(resolved) => resolved,
                    Err(e) => {
                        eprintln!("refusing to run: {e}");
                        std::process::exit(1);
                    }
                };
                // The same resource limits for whichever child runs (the
                // supervisor adds them to both `aether node` and `aether follow`).
                let forwarded = resources.forward();
                let (mut node_args, mut follow_args) = (node_args, follow_args);
                node_args.extend(forwarded.iter().cloned());
                follow_args.extend(forwarded);
                // The gateway role carries to whichever child runs.
                if public_read_only {
                    node_args.push("--public-read-only".into());
                    follow_args.push("--public-read-only".into());
                }
                aether_node::supervisor::Supervisor {
                    exe: std::env::current_exe().map_err(|e| e.to_string())?,
                    data: dir,
                    port,
                    rpc_port,
                    reshare_port: reshare_port.unwrap_or(port + 1),
                    node_args,
                    follow_args,
                    dev_peer_dir: dev_peer_dir.map(Into::into),
                    reshare_timeout: reshare_timeout.map(Duration::from_secs),
                    ceremony: ceremony.map(Into::into),
                    chain_data: chain_data.map(Into::into),
                    archive,
                }
                .run(&_lock)
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
            let (r, s) = (part("r")?, part("s")?);
            println!("candidate {}  node {}  beacons from {beaconer}", hex::encode(vk), hex::encode(nid));
            let status = call(&rpc, "aether_status", json!([]))?;
            let receipt = if candidate_registration_uses_lane(&status) {
                if status["chain_id"].as_u64() != Some(chain_id) {
                    return Err("registrar and registration RPC are on different chains".into());
                }
                let signer = P256Signer::from_seed(&dev_seed(from_dev)).map_err(|e| e.to_string())?;
                let nonce: u64 = serde_json::from_value(call(&rpc, "aether_registrationNonce", json!([operator]))?)
                    .map_err(|e| format!("registration nonce: {e}"))?;
                let expiry = status["height"].as_u64().ok_or("status has no height")?.saturating_add(7_200);
                let attestation = [r.as_slice(), s.as_slice()].concat();
                let signature = signer.sign(&aether_execution::registry::relay_message(
                    chain_id, operator, &vk, &nid, beaconer, &attestation, nonce, expiry,
                )).map_err(|e| e.to_string())?;
                let item = aether_light::block::NodeRegistration {
                    operator,
                    validator_key: vk.into(),
                    node_id: nid.into(),
                    beaconer,
                    attestation: attestation.into(),
                    signature: signature.into(),
                    operator_key: signer.public_key().bytes.into(),
                    nonce,
                    expiry,
                };
                let sent = call(&rpc, "aether_sendRegistration", json!([item]))?;
                let hash: TxHash = serde_json::from_value(sent["hash"].clone()).map_err(|e| e.to_string())?;
                println!("tx {hash}  from {operator}  nonce {nonce}  (signed with P-256)");
                wait_for_receipt(&rpc, hash)?
            } else {
                let input = aether_execution::registry::encode_register(vk, nid, beaconer, r, s);
                let c = EvmCall { to: Some(aether_execution::registry::REGISTRY), value: U256::ZERO, input, gas_limit: 400_000, delegate: None };
                submit_with_tip(&rpc, from_dev, None, c, true, tip)?
            };
            if receipt["receipt"]["success"] != json!(true) {
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
        Cmd::Protocol => (|| {
            println!("{}", aether_node::upgrade::implements());
            Ok(())
        })(),
        Cmd::ValidatorProgram { network, rpc, timeout } => validator_program(&network, rpc, timeout),
        #[cfg(feature = "dev-drill")]
        Cmd::DevB3 { file } => (|| {
            let bytes = std::fs::read(&file).map_err(|e| format!("read {file}: {e}"))?;
            println!("{}", hex::encode(blake3::hash(&bytes).as_bytes()));
            Ok(())
        })(),
        Cmd::Reshare { from, to, epoch_end, epoch_end_hash, stage, via_node, port, data, round, peers, link_base, offline, exit_with_parent } => {
            if exit_with_parent {
                exit_with_parent_process(_writer_lease.as_ref().map(|lease| lease.expected_parent()));
            }
            let boundary = match (stage, epoch_end, epoch_end_hash) {
                (true, _, _) => None,
                (false, Some(h), Some(parent)) => Some(aether_node::roster::EpochStart { height: h + 1, parent }),
                _ => unreachable!("clap requires --epoch-end and --epoch-end-hash without --stage"),
            };
            reshare(&from, &to, boundary, port, data, round, peers, link_base, offline, via_node)
        }
        Cmd::Network { chain_id, faucet, registrar, dev_registrar, epoch_blocks, min_streak, draw_epochs, node_rewards, history, protocol, reserve, reserve_operator, group, max_committee, release, members } => (|| {
            let release = match release {
                None => None,
                Some(p) => Some(aether_node::roster::ReleasePin::from_config(
                    &std::fs::read(&p).map_err(|e| format!("{p}: {e}"))?,
                )?),
            };
            let registrar = match (registrar, dev_registrar) {
                (None, true) => Some(dev_registrar_hex()),
                (r, _) => r,
            };
            let reserve = reserve_operator.map(|op| (op, reserve));
            assemble_network(chain_id, faucet, registrar, (epoch_blocks, min_streak, draw_epochs), (history, protocol, group, max_committee), (node_rewards, reserve), release, &members)
        })(),
        Cmd::RegistrarKey { data } => (|| {
            // Idempotent: an existing key is kept (and its public half printed).
            let path = std::path::Path::new(&data).join("registrar.key");
            if !path.exists() {
                aether_node::faucet::Faucet::generate(&path)?;
            }
            let k = aether_node::faucet::Faucet::load(&path)?;
            println!(
                "registrar key {}\nin {data}/registrar.key (put the key in network.json with `aether network --registrar`). \
                 A signing Mac keeps this key in the Secure Enclave instead: apps/registrar-signer, docs/ops/registrar.md",
                k.public_hex()
            );
            Ok(())
        })(),
        Cmd::DevAccounts => {
            for (i, a) in dev_accounts(DEV_ACCOUNTS) {
                println!("dev {i:>2}  {a}");
            }
            Ok(())
        }
        Cmd::MainnetRules { network, rehearsal, bundle } => (|| {
            let path = std::path::Path::new(&network);
            let file = aether_node::roster::NetworkFile::load(path)?;
            let rules = mainnet_rules(&file, path, rehearsal, bundle)?;
            for r in &rules {
                println!("{}  {}: {}", if r.ok { "ok" } else { "FAIL" }, r.name, r.detail);
            }
            let missing = aether_node::mainnet::missing(&rules);
            (rules.iter().all(|r| r.ok)).then_some(()).ok_or(missing)
        })(),
        Cmd::CeremonyRecord { network, out } => (|| {
            let path = std::path::Path::new(&network);
            let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let file = aether_node::roster::NetworkFile::load(path)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default();
            let record = aether_node::mainnet::ceremony_record(&file, &bytes, now)?;
            let out = out.map(Into::into).unwrap_or_else(|| {
                path.parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(std::path::Path::new("."))
                    .join(aether_node::mainnet::CEREMONY_RECORD_FILE)
            });
            let body = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
            std::fs::write(&out, body).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
            println!(
                "ceremony record: {} (chain {}, round {}, identity {}…, sha256 {}… over the checked bytes)",
                out.display(),
                record.chain_id,
                record.round,
                &record.identity[..record.identity.len().min(16)],
                &record.digest[..record.digest.len().min(16)]
            );
            println!(
                "  copy it to every validator Mac with the final network.json: verify-local and the node demand it (--ceremony)"
            );
            Ok(())
        })(),
        Cmd::MainnetBind { network, data, ceremony } => (|| {
            aether_node::mainnet::bind_data_dir(
                std::path::Path::new(&data),
                std::path::Path::new(&network),
                ceremony.as_deref().map(std::path::Path::new),
            )?;
            println!(
                "BIND PASS: the checked genesis is chain {network}'s ceremony file, and this Mac's share is its committee"
            );
            Ok(())
        })(),
        Cmd::Shadow { from, to, network } => run_shadow(&from, to, network.as_deref()),
        Cmd::Status { rpc } => call(&rpc, "aether_status", json!([])).map(|v| println!("{}", pretty(&v))),
        Cmd::Blocks { rpc, n } => call(&rpc, "aether_recentBlocks", json!([n])).map(|v| print_blocks(&v)),
        Cmd::Send { rpc, from_dev, to, value, nonce, wait } => {
            // Like the wallet (ffi `prepare_transfer`): a recipient with code —
            // a contract's receive() or a 7702-delegated account — needs more
            // than the intrinsic 21,000, or the transfer fails and still pays.
            let code = call(&rpc, "eth_getCode", json!([to, "latest"]))
                .ok()
                .and_then(|v| v.as_str().and_then(|h| alloy_primitives::hex::decode(h.trim_start_matches("0x")).ok()))
                .unwrap_or_default();
            let gas_limit = aether_execution::plain_transfer_gas_limit(&code);
            submit(&rpc, from_dev, nonce, EvmCall { to: Some(to), value, input: Bytes::new(), gas_limit, delegate: None }, wait).map(|_| ())
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
        Cmd::Deploy { rpc, from_dev, code, gas } => (|| {
            let input = Bytes::from(hex::decode(code.trim_start_matches("0x")).map_err(|e| e.to_string())?);
            let r = submit(&rpc, from_dev, None, EvmCall { to: None, value: U256::ZERO, input, gas_limit: gas.unwrap_or(3_000_000), delegate: None }, true)?;
            if let Some(a) = r.pointer("/receipt/contract_address") {
                println!("contract: {}", a.as_str().unwrap_or_default());
            }
            Ok(())
        })(),
        Cmd::Call { rpc, from_dev, to, data, value, gas, wait } => (|| {
            let input = Bytes::from(hex::decode(data.trim_start_matches("0x")).map_err(|e| e.to_string())?);
            submit(&rpc, from_dev, None, EvmCall { to: Some(to), value: value.unwrap_or(U256::ZERO), input, gas_limit: gas.unwrap_or(1_000_000), delegate: None }, wait).map(|_| ())
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
        group: genesis.group,
        max_committee: genesis.max_committee,
    }
}

fn run_shadow(from: &str, to: u64, network: Option<&std::path::Path>) -> Result<(), String> {
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    let base = std::env::current_dir().map_err(|e| e.to_string())?.join("tmp");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let scratch = base.join(format!("shadow-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos()));
    std::fs::create_dir(&scratch).map_err(|e| e.to_string())?;
    let _cleanup = Scratch(scratch.clone());
    let network = if let Some(path) = network {
        Some(aether_node::roster::NetworkFile::load(path)?)
    } else if from.starts_with("http://") || from.starts_with("https://") {
        let value = call(from, "aether_network", json!([]))?;
        if value.is_null() { None } else { Some(serde_json::from_value::<aether_node::roster::NetworkFile>(value).map_err(|e| e.to_string())?) }
    } else {
        let data = std::path::Path::new(from);
        let file = data.join("network.json");
        let parent = data.parent().unwrap_or(data).join("network.json");
        let file = if file.is_file() { file } else { parent };
        if file.is_file() { Some(aether_node::roster::NetworkFile::load(&file)?) }
        else { return Err("source network.json is missing; pass --network <genesis network.json>".into()); }
    };
    let source = aether_node::shadow::Source::open(from, &scratch)?;
    let cfg = if let Some(file) = network {
        chain_config(file.chain_id, &file.genesis()?, false)
    } else {
        let chain_id = if let Some(url) = from.starts_with("http://").then_some(from).or_else(|| from.starts_with("https://").then_some(from)) {
            call(url, "aether_status", json!([]))?["chain_id"].as_u64().ok_or("peer has no chain id")?
        } else {
            DEFAULT_CHAIN_ID
        };
        chain_config(chain_id, &aether_node::roster::Genesis::default(), true)
    };
    aether_node::shadow::replay(cfg, &source, to, &scratch)?;
    println!("shadow PASS through finalized height {to}");
    Ok(())
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
    requested_round: Option<u64>,
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
    if old_file.chain_id != 7_780 && aether_node::dkg::KeyFile::reveals_seated_share(&previous, &previous.players()) {
        return Err("current committee output reveals a seated player's threshold share".into());
    }
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
    let mut round = aether_node::dkg::Round::reshare(previous, new.validators(), old_file.round + 1);
    let legacy_agreement = old_file.chain_id == 7_780;
    if legacy_agreement {
        round = round.legacy_agreement();
    } else {
        round = round.with_chain_id(old_file.chain_id);
    }
    let next_round = if legacy_agreement {
        round.round
    } else {
        let chosen = requested_round.unwrap_or(round.round);
        if chosen < round.round {
            return Err(format!("reshare round {chosen} must be at least {}", round.round));
        }
        round.round = chosen;
        chosen
    };
    // The vote/dealing journals survive retries of this same key round. The
    // Commonware runtime directory below is intentionally fresh per attempt.
    let agreement_journal = dir.join(format!("dkg-agreement-reshare-{next_round}.journal"));
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
    let relay_inputs = (staged && !legacy_agreement)
        .then(|| (p2p.clone(), round.clone(), share.clone(), agreement_journal.clone()));
    let readiness_members: Vec<(String, String)> = new_file.validators.iter()
        .map(|m| (m.key.to_lowercase(), m.node.clone())).collect();
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
        if legacy_agreement {
            aether_node::dkg::run(p2p.keys.signer.clone(), round, share, sender, receiver, Default::default()).await
        } else {
            aether_node::dkg::run_with_journal(
                p2p.keys.signer.clone(), round, share, sender, receiver, Default::default(), agreement_journal,
            ).await
        }
    });
    let mut staged_share = None;
    match result.map_err(|e| format!("reshare failed: {e}"))? {
        Some((output, share)) => {
            if !legacy_agreement && aether_node::dkg::KeyFile::reveals_seated_share(&output, &output.players()) {
                return Err("reshare output reveals a seated player's threshold share; retry with a higher --round".into());
            }
            let file = aether_node::dkg::KeyFile::new(next_round, &output, &share);
            staged_share = Some(share.clone());
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
    if let Some((p2p, round, share, journal)) = relay_inputs {
        // Stage any new share before reopening the durable ceremony. Departing
        // dealers also relay the bundle and certificate to late players.
        let relay_dir = dir.join("reshare-relay-runtime").join(format!("{next_round}-{secs}"));
        let readiness = aether_node::dkg::ReadinessPlan {
            members: readiness_members,
            staged_share,
            destination: dir.join(aether_node::handoff::READY_FILE),
        };
        let relay = cw_tokio::Runner::new(cw_tokio::Config::new().with_storage_directory(relay_dir));
        relay.start(async move |context| {
            let _public = aether_node::p2p::open_reshare(&p2p, via_node, loopback(p2p.port))
                .await
                .map(|(ep, r)| (ep, r.map(Some).unwrap_or(None)));
            let (mut network, mut oracle) = lookup::Network::new(
                context.child("network"),
                aether_node::p2p::config(&p2p, b"_DKG"),
            );
            oracle.track(0, aether_node::p2p::peer_addresses(&p2p));
            let (sender, receiver) = network.register(0, Quota::per_second(NZU32!(256)));
            network.start();
            aether_node::dkg::run_relay_with_journal(
                p2p.keys.signer.clone(), round, share, sender, receiver,
                journal, Some(readiness),
            ).await
        }).map_err(|e| format!("reshare relay failed: {e}"))?;
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
    // The identity is both secrets: the voting key and the node account that
    // pays for and sends beacons. A directory with one but not the other is a
    // *lost* identity and never runs (candidate.rs, red team #5), so keygen —
    // the documented first install — writes both, exactly as `aether run` does
    // on a fresh directory. An existing account key is never replaced.
    let account = dir.join(aether_node::candidate::ACCOUNT_FILE);
    if !account.exists() {
        aether_node::faucet::Faucet::generate(&account)?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&keys.public()).expect("json")
    );
    println!(
        "secret keys in {} and {} (mode 600); share only {}",
        dir.join(aether_node::roster::KEY_FILE).display(),
        account.display(),
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
    format: (Option<u32>, Option<u32>, Option<u16>, Option<u64>),
    rewards: (bool, Option<(Address, Vec<String>)>),
    release: Option<aether_node::roster::ReleasePin>,
    members: &[String],
) -> Result<(), String> {
    let (epoch_blocks, min_streak, draw_epochs) = voting;
    let (history, protocol, group, max_committee) = format;
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
        group,
        max_committee,
        release,
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
    /// Unix socket of the Secure Enclave signer helper (apps/registrar-signer);
    /// without it the registrar signs with `<data>/registrar.key`.
    registrar_signer: Option<String>,
    dev_registrar: bool,
    /// network.json (with the committee output) when run from one: enables rotation.
    network_file: Option<Value>,
    /// Archive or prune (roadmap B4).
    history: aether_node::prune::HistoryMode,
    /// Era shards this Mac holds at most (roadmap B5 phase 1).
    max_shards: usize,
    /// Serve only the explorer's reads with caps (docs/ops/read-gateway.md).
    public_read_only: bool,
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
    use aether_node::registrar_signer::RegistrarSigner as _;
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
        registrar_signer,
        dev_registrar,
        network_file,
        history,
        max_shards,
        public_read_only,
        resources,
    } = a;
    aether_node::supervisor::install_fatal_watch(std::path::PathBuf::from(&data));
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
    // Where the registrar's attestation key is (docs/ops/registrar.md): the
    // Secure Enclave of the signing Mac when --registrar-signer is given, the
    // `<data>/registrar.key` seed file otherwise. A node that does not run the
    // registrar service never reads either — a validator needs no registrar key.
    let signer: Option<std::sync::Arc<dyn aether_node::registrar_signer::RegistrarSigner>> =
        if dev_registrar {
            Some(std::sync::Arc::new(
                aether_node::registrar_signer::FileSigner::from_seed(&dev_seed(DEV_REGISTRAR))
                    .expect("dev registrar"),
            ))
        } else {
            match (&registrar_signer, devicecheck.is_some()) {
                (Some(socket), _) => {
                    let signer = aether_node::registrar_signer::EnclaveSigner::connect(
                        std::path::Path::new(socket),
                    )
                    .unwrap_or_else(|e| {
                        eprintln!("error: --registrar-signer {socket}: {e}");
                        std::process::exit(2);
                    });
                    tracing::info!(
                        key = %signer.describe(),
                        "the registrar signs with the Secure Enclave on the signing Mac"
                    );
                    Some(std::sync::Arc::new(signer))
                }
                (None, true) => Some(std::sync::Arc::new(
                    aether_node::registrar_signer::FileSigner::load(
                        &std::path::Path::new(&data).join("registrar.key"),
                    )
                    .expect("<data>/registrar.key (aether registrar-key)"),
                )),
                (None, false) => None,
            }
        };
    // What this node signs with, checked against the registry once the chain is
    // open: the committee can rotate or stop the registrar by upgrade.
    let registrar_key = signer.as_ref().map(|s| s.public_hex());
    let registrar = match signer {
        Some(signer) if dev_registrar => {
            Some(std::sync::Arc::new(aether_node::devicecheck::Registrar::new(
                None,
                registry(),
                signer,
                chain_id,
            )))
        }
        Some(signer) => devicecheck.map(|(k, id, team)| {
            let apple =
                aether_node::devicecheck::DeviceCheck::load(std::path::Path::new(&k), &id, &team)
                    .expect("load --devicecheck-key");
            std::sync::Arc::new(aether_node::devicecheck::Registrar::new(
                Some(apple),
                registry(),
                signer,
                chain_id,
            ))
        }),
        None => None,
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
        let (participants, polynomial, share) = committee_keys(&data, &validator_set, &signer.public_key(), key_round, chain_id);
        let (handoff_share, handoff_sharing) = (share.clone(), polynomial.clone());
        let polynomial_identity = &polynomial.public().clone();
        let scheme = aether_light::Scheme::signer(&aether_light::consensus_namespace_of(cfg.group), participants, polynomial, share).expect("share matches polynomial");
        // Start-up integrity as a follower has it (docs/design/24-self-healing.md
        // layer 1): a database that does not verify is moved aside (never
        // deleted; the keys stay) and the catch-up below re-syncs it. A schema
        // newer than this binary reads (red team #15) is not corruption: the
        // data is untouched and the node stops with the update-required code.
        let (store, _) = match aether_node::follow::open_store(std::path::Path::new(&data)) {
            Ok(opened) => opened,
            Err(e) if aether_node::follow::is_too_new_error(&e) => {
                tracing::error!(%e, "UPDATE REQUIRED: the state database was written by a newer aether; stopping without touching it — install the signed release");
                std::process::exit(aether_node::supervisor::EXIT_UPGRADE_REQUIRED);
            }
            Err(e) => panic!("open state store: {e}"),
        };
        let (chain, genesis) = match Chain::open(cfg.clone(), store) {
            Ok(opened) => opened,
            Err(e) if aether_node::follow::is_corruption(&e) => {
                let store = aether_node::follow::reset_store(std::path::Path::new(&data), &e).expect("move a corrupt database aside");
                Chain::open(cfg.clone(), store).expect("restore state after moving a corrupt database aside")
            }
            Err(e) => panic!("restore state (delete the data dir to resync): {e:?}"),
        };
        chain.watch_releases(network_file.as_ref());
        install_verifier(&chain, &data, false);
        // The registrar key in the registry decides: the committee can rotate
        // or stop the registrar by a threshold-signed upgrade, and then this
        // node's attestations are already dead (docs/design/14-registration.md
        // 4). Registrations refuse below; say it here, where the operator looks
        // first, so a rotated key is noticed at startup and not at a failed
        // registration.
        if let Some(key) = &registrar_key {
            if let Err(e) = aether_node::devicecheck::registrar_key_check(&chain.lock().finalized.state, key) {
                tracing::warn!("registrar: {e}");
            }
        }
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
            public_read_only,
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
                let set = aether_light::ValidatorSet::for_group(*polynomial_identity, cfg.group);
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
                journal_dir: Some(std::path::PathBuf::from(&data)),
                me: signer.public_key(),
                scheme,
                identity: *polynomial_identity,
                group: cfg.group,
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
        let network_task = network.start();
        tokio::spawn(async move {
            let result = network_task.await;
            tracing::error!(?result, "validator network task stopped; restarting the node");
            std::process::exit(aether_node::supervisor::EXIT_FATAL_TASK);
        });
        if let Some(ep) = endpoint.clone().filter(|_| links) {
            let me = p2p.keys.node_secret.public();
            let nodes: Vec<_> = p2p.roster.nodes.iter().copied().filter(|n| *n != me).collect();
            if !nodes.is_empty() {
                tokio::spawn(validator_progress_watch(chain.clone(), ep, nodes));
            }
        }
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

        let effective_ms = if cfg.node_rewards || cfg.history_v2 { block_time_ms.max(aether_node::application::MIN_BLOCK_INTERVAL_MS) } else { block_time_ms };
        spawn_inclusion_lists(chain.clone(), signer.clone(), index, roster_keys, cfg.chain_id, Duration::from_millis(effective_ms), il_out, il_in);

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
            public_read_only,
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
        std::process::exit(aether_node::supervisor::EXIT_FATAL_TASK);
    });
}

/// Restart a validator whose finalized head stays frozen while another
/// committee member advances. A healthy network-wide halt is not diagnosed
/// by peer height alone; engine task death is caught independently.
async fn validator_progress_watch(chain: Chain, endpoint: aether_net::Endpoint, peers: Vec<aether_net::EndpointId>) {
    let mut height = chain.finalized_height();
    let mut since = std::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let ours = chain.finalized_height();
        if ours != height || aether_node::resources::monitor().is_some_and(|m| m.disk_low()) {
            height = ours;
            since = std::time::Instant::now();
            continue;
        }
        if since.elapsed() < Duration::from_secs(5 * 60) { continue; }
        let reports = aether_node::follow::roster_heights(&endpoint, &peers).await;
        // A single peer's RPC height is an unauthenticated claim about its
        // chain. Require two distinct committee members before restarting;
        // one faulty member cannot exhaust our supervisor's crash budget.
        let heights: Vec<u64> = reports.iter().map(|(_, h)| *h).collect();
        if validator_is_stalled(ours, &heights, since.elapsed()) {
            let ahead = heights.iter().filter(|h| **h > ours.saturating_add(aether_node::follow::BEHIND_MARGIN)).count();
            tracing::error!(ours, ahead_peers = ahead, frozen_seconds = since.elapsed().as_secs(), "validator finalized height stalled while committee peers advanced; restarting");
            std::process::exit(aether_node::supervisor::EXIT_FATAL_TASK);
        }
    }
}

fn validator_is_stalled(ours: u64, peers: &[u64], unchanged: Duration) -> bool {
    unchanged >= Duration::from_secs(5 * 60)
        && peers.iter().filter(|peer| **peer > ours.saturating_add(aether_node::follow::BEHIND_MARGIN)).count() >= 2
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
    use aether_node::upgrade::{implements, load, protocol_at};
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
            // The chain is the authority: an upgrade counts once it is on
            // chain, compared against what this binary claims to run (drill
            // builds may name a later release; see upgrade::implements).
            let (need, have) = (protocol_at(&on_chain, next), implements());
            if need > have {
                tracing::error!(need, have, height = next, "UPGRADE REQUIRED: this binary runs protocol {have} but the committee activated {need}; stopping before the new rules apply. Install the signed release.");
                std::process::exit(aether_node::supervisor::EXIT_UPGRADE_REQUIRED);
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
    let mut sidecar_start_failed = false;
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
            sidecar_start_failed = true;
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
        // A missing binary needs installation (5). A configured sidecar that
        // fails to start may recover after a restart (9, bounded by aether run).
        std::process::exit(if sidecar_start_failed {
            aether_node::supervisor::EXIT_FATAL_TASK
        } else {
            aether_node::supervisor::EXIT_NO_VERIFIER
        });
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
    let network_handle = handle.clone();
    let network_upstream = upstream.clone();
    let network_chain = chain.clone();
    let target = chain.clone();
    spawn_service(
        chain.clone(),
        bin_path,
        dir.join("prove"),
        sidecar,
        payout,
        status.clone(),
        move || match &network_upstream {
            Some(up) => {
                // Validators from before the RPC answer "method not found"; on a
                // chain whose old program is known that maps to it (a definite
                // mismatch or match instead of "cannot confirm").
                let chain_id = network_chain.lock().cfg.chain_id;
                let answer = network_handle.block_on(up.first("aether_proverProgram", json!([]))).and_then(|answer| {
                    answer
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "validator does not report its proof program".into())
                });
                aether_node::prover::network_program(chain_id, answer)
            }
            None => network_chain.lock().verifier.as_ref()
                .and_then(|v| v.program_id())
                .ok_or_else(|| "local validator has no proof verifier program".into()),
        },
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

/// `aether validator-program`: what the network's validators verify proofs
/// with, read the way a follower reads it before proving (`start_prover`).
fn validator_program(network: &str, rpc: Vec<String>, timeout: u64) -> Result<(), String> {
    use aether_node::follow::Upstream;
    let file = aether_node::roster::NetworkFile::load(std::path::Path::new(network))?;
    let chain_id = file.chain_id;
    let nodes = aether_node::roster::Roster::from_file(&file)?.nodes;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    let answer = rt.block_on(async move {
        let up = if rpc.is_empty() {
            let ep = aether_net::bind(None, vec![aether_net::ALPN_RPC.to_vec()]).await.map_err(|e| e.to_string())?;
            Upstream::Iroh(aether_net::RpcClient::with_endpoint(ep, nodes), Default::default())
        } else {
            Upstream::Http(rpc)
        };
        tokio::time::timeout(Duration::from_secs(timeout), up.first("aether_proverProgram", json!([])))
            .await
            .map_err(|_| format!("no validator answered aether_proverProgram within {timeout}s"))
    })?;
    let answer = answer.and_then(|v| v.as_str().map(str::to_owned).ok_or_else(|| "validator does not report its proof program".to_string()));
    let predates = matches!(&answer, Err(e) if e.contains("method not found: aether_proverProgram"));
    let program = aether_node::prover::network_program(chain_id, answer)?;
    if predates {
        eprintln!("chain {chain_id}: the validators predate aether_proverProgram; this is the program known for them");
    }
    println!("{program}");
    Ok(())
}

/// Leave no orphan: stop when the parent process is gone (reparented to launchd).
fn initial_writer_parent(expected: Option<u32>, actual: u32) -> Option<u32> {
    match expected {
        Some(parent) if parent > 0 && parent == actual => Some(parent),
        None if actual > 1 => Some(actual),
        _ => None,
    }
}

fn exit_with_parent_process(expected_parent: Option<u32>) {
    // An explicitly leased PID 1 is valid for container-init supervisors;
    // never substitute launchd for a sender that disappeared before startup.
    let Some(parent) = initial_writer_parent(expected_parent, std::os::unix::process::parent_id())
        else { std::process::exit(0) };
    std::thread::spawn(move || loop {
        if !aether_node::supervisor::expected_parent_is_current(parent) { std::process::exit(0); }
        std::thread::sleep(Duration::from_millis(100));
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
    public_read_only: bool,
    history: HistoryArgs,
    resources: ResourceArgs,
    export: Option<aether_node::export::ExportArgs>,
    // Where the RPC server listens (None: loopback, a follower's default).
    bind: Option<IpAddr>,
    // The wallet-server endpoint key file (`wallet_node_key`).
    node_key: std::path::PathBuf,
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
    aether_node::supervisor::install_fatal_watch(std::path::PathBuf::from(&data));
    aether_node::resources::install(resources.limits()?, std::path::Path::new(&data).to_path_buf());
    // A network.json with no faucet funds nobody (mainnet: 사전 발행 0).
    let dev_alloc = network.is_none();
    let (chain_id, genesis, set, nodes, network_file) = match network {
        Some(path) => {
            let file = aether_node::roster::NetworkFile::load(std::path::Path::new(&path))?;
            let genesis = file.genesis()?;
            let identity = file.identity.clone().ok_or(
                "network.json has no committee identity: use the one written by dkg/reshare",
            )?;
            let set = aether_light::ValidatorSet::from_hex(&identity)
                .map_err(|e| format!("identity: {e:?}"))?
                .with_group(genesis.group);
            let nodes = aether_node::roster::Roster::from_file(&file)?.nodes;
            let network_file = serde_json::to_value(&file).map_err(|e| format!("network.json: {e}"))?;
            (file.chain_id, genesis, set, nodes, Some(network_file))
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
                None,
            )
        }
    };
    let cfg = chain_config(chain_id, &genesis, dev_alloc);
    // Followers run at the network's 1 s block time.
    let max_shards = history.max_shards;
    let history_v2 = cfg.history_v2;
    // An archive node keeps everything by definition (roadmap B6): whatever
    // the flags say, it never prunes what it exists to hold.
    let history = if export.is_some() {
        aether_node::prune::HistoryMode::Archive
    } else {
        history.mode(cfg.history_v2, 1000)?
    };
    // An archive must own every block from genesis: a checkpoint start
    // installs a snapshot whose store has no history index, and the exporter
    // needs that index forever after (audit 7 A7-1). `follow::run` below also
    // refuses to snapshot-jump for the same reason; this gate covers the
    // startup path, that one the falling-behind path.
    let (checkpoint, no_jump) = sync_plan(checkpoint, export.is_some());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async move {
        // Start-up integrity (docs/design/24-self-healing.md layer 1): a state
        // database that does not verify is moved aside (never deleted; the
        // keys stay) and re-syncs — a certified snapshot first, as below. The
        // hidden dev flag runs the same start on a disk that fills. A schema
        // newer than this binary reads (red team #15) is none of those: the
        // data is never moved aside or written — the node stops with the
        // update-required exit code, which the supervisor and the app already
        // turn into "install the newer release" (`is_too_new_error`).
        let opened = match dev_storage_fault {
            Some(ms) => {
                tracing::warn!(ms, "--dev-storage-fault: this disk fails from now on (self-healing test)");
                follow::open_store_with(
                    std::path::Path::new(&data),
                    std::sync::Arc::new(move |p| {
                        aether_node::store::open_with_a_disk_that_fills(p, Duration::from_millis(ms))
                    }),
                )
            }
            None => follow::open_store(std::path::Path::new(&data)),
        };
        let (store, _) = match opened {
            Ok(opened) => opened,
            Err(e) if aether_node::follow::is_too_new_error(&e) => {
                tracing::error!(%e, "UPDATE REQUIRED: the state database was written by a newer aether; stopping without touching it — install the signed release");
                std::process::exit(aether_node::supervisor::EXIT_UPGRADE_REQUIRED);
            }
            Err(e) => return Err(e),
        };
        // Following over iroh, this Mac also serves wallets directly (capacity
        // review 2026-09-29): a public endpoint under its own persisted node
        // id, so phones spread their reads over follower Macs instead of
        // asking the validators. `--from-rpc` followers have no iroh endpoint.
        let mut wallet_ep = None;
        let upstream = Arc::new(if from_rpc.is_empty() {
            let ep = aether_net::bind(Some(wallet_node_key(&node_key)?), vec![aether_net::ALPN_RPC.to_vec()])
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
        chain.watch_releases(network_file.as_ref());
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
        let follow_task = tokio::spawn(follow::run(chain.clone(), upstream.clone(), set, archive.clone(), joining, no_jump));
        tokio::spawn(async move {
            let result = follow_task.await;
            tracing::error!(?result, "follower task stopped; restarting the node");
            std::process::exit(aether_node::supervisor::EXIT_FATAL_TASK);
        });
        if let aether_node::prune::HistoryMode::Prune(r) = &history {
            tokio::spawn(aether_node::prune::run(chain.clone(), r.clone(), None));
        }
        // Era shards (roadmap B5 phase 1): a follower fetches the eras it is
        // owed shards of over the B4 path when it no longer keeps their files.
        // An archive node holds whole era files — it has no shard duty.
        let shards = (history_v2 && export.is_none()).then(|| {
            let s = std::sync::Arc::new(aether_node::shards::Shards::new(std::path::Path::new(&data), shard_me, max_shards));
            tokio::spawn(aether_node::shards::run(chain.clone(), Some(upstream.clone()), s.clone()));
            s
        });
        // Era export (roadmap B6): every sealed era, re-verified against this
        // node's certified history index, written as a static file set.
        if let Some(args) = &export {
            tokio::spawn(aether_node::export::run(chain.clone(), args.clone()));
        }
        tracing::info!(height = chain.finalized_height(), rpc_port, "following (not a validator): every block is verified and re-executed here");
        let prover = if export.is_none() { start_prover(&chain, &data, Some(upstream.clone())) } else { None };
        let st = RpcState {
            chain,
            finality: aether_node::rpc::Finality::Archive(archive),
            gossip,
            faucet: None,
            registrar: None,
            network: network_file,
            upstream: Some(upstream.clone()),
            handoff: None,
            snapshot: Default::default(),
            prover,
            shards,
            public_read_only,
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
        let listen = bind.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let result = rpc::serve(SocketAddr::new(listen, rpc_port), st).await;
        tracing::error!(?result, "follower RPC server stopped; restarting the node");
        std::process::exit(aether_node::supervisor::EXIT_FATAL_TASK)
    })
}

/// `aether archive` (roadmap B6): a follower that keeps everything, serves
/// old eras, and writes every sealed era out as a static, torrent-ready set.
#[allow(clippy::too_many_arguments)]
/// `follow --archive-export DIR` (the child `aether run --archive` spawns):
/// the era export `aether archive` runs, signed with a key kept in the
/// follower's own data directory, no webseeds or public base (the wallet's
/// archive serves this Mac, not the world).
fn follow_export(archive_export: Option<String>, data: &str) -> Option<aether_node::export::ExportArgs> {
    archive_export.map(|dir| aether_node::export::ExportArgs {
        dir: std::path::PathBuf::from(dir),
        webseeds: Vec::new(),
        https_base: None,
        sign_key: std::path::Path::new(data).join("archive-export.key"),
    })
}

/// How a follower syncs: (start from a certified snapshot, never snapshot-
/// jump later). An archive (an era export) must own every block from
/// genesis, so it takes neither shortcut (audit 7 A7-1).
fn sync_plan(checkpoint: bool, exporting: bool) -> (bool, bool) {
    (checkpoint && !exporting, exporting)
}

fn run_archive(
    network: String,
    from_rpc: Vec<String>,
    data: String,
    rpc_port: u16,
    export_dir: String,
    webseed: Vec<String>,
    https_base: Option<String>,
    bind: IpAddr,
    export_key: Option<String>,
    resources: ResourceArgs,
) -> Result<(), String> {
    let export = aether_node::export::ExportArgs {
        dir: std::path::PathBuf::from(&export_dir),
        webseeds: webseed,
        https_base,
        sign_key: std::path::PathBuf::from(export_key.unwrap_or_else(|| format!("{data}/archive-export.key"))),
    };
    std::fs::create_dir_all(&export.dir).map_err(|e| format!("{}: {e}", export.dir.display()))?;
    // run_follow installs the tracing subscriber; a second install here panicked
    // at startup ("a global default trace dispatcher has already been set").
    eprintln!("era export set (roadmap B6) in {}: era files, manifests, torrents, index", export.dir.display());
    // A fresh archive replays from genesis like any follower with a store to
    // build (checkpoint off: a snapshot start leaves no history index, and the
    // exporter needs one forever after — audit 7 A7-1); the flags say prune,
    // the mode ignores them (history = Archive).
    let history = HistoryArgs {
        history: None,
        retain_days: aether_node::prune::DEFAULT_RETAIN_DAYS,
        drop_era_files: false,
        max_shards: aether_node::shards::DEFAULT_MAX_SHARDS,
    };
    let node_key = std::path::Path::new(&data).join("wallet-node.key");
    run_follow(Some(network), from_rpc, data, rpc_port, 4, None, None, false, None, false, history, resources, Some(export), Some(bind), node_key)
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
    let agreement_journal = dir.join(format!("dkg-agreement-genesis-{round}.journal"));
    let executor = cw_tokio::Runner::new(
        cw_tokio::Config::new().with_storage_directory(dir.join("dkg-runtime")),
    );
    let (mut public, dir_out) = (p2p.roster.to_file(chain_id), dir.clone());
    // Every genesis fact survives the ceremony (`carry_genesis` is the inverse of
    // `NetworkFile::genesis`; a hand-copied list had lost the protocol, group,
    // committee ceiling and the frozen genesis roster).
    public.carry_genesis(&genesis);
    let relay_inputs = (chain_id != 7_780).then(|| (p2p.clone(), agreement_journal.clone()));
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
        let mut key_round = aether_node::dkg::Round::dkg(p2p.validators(), round);
        if chain_id == 7_780 {
            key_round = key_round.legacy_agreement();
        } else {
            key_round = key_round.with_chain_id(chain_id);
        }
        if chain_id == 7_780 {
            aether_node::dkg::run(p2p.keys.signer.clone(), key_round, None, sender, receiver, Default::default()).await
        } else {
            aether_node::dkg::run_with_journal(
                p2p.keys.signer.clone(), key_round, None, sender, receiver, Default::default(), agreement_journal,
            ).await
        }
    });
    match result {
        Ok(None) => unreachable!("every DKG participant is a player"),
        Ok(Some((output, share))) => {
            if chain_id != 7_780 && aether_node::dkg::KeyFile::reveals_seated_share(&output, &output.players()) {
                eprintln!("dkg failed: output reveals a seated player's threshold share; retry with a higher --round");
                std::process::exit(1);
            }
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
    if let Some((p2p, journal)) = relay_inputs {
        // The genesis share is now durable. Keep the certified bundle and
        // decision reachable for a player that joins after this one returned.
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let relay = cw_tokio::Runner::new(cw_tokio::Config::new()
            .with_storage_directory(dir.join("dkg-relay-runtime").join(format!("{round}-{secs}"))));
        let result = relay.start(async move |context| {
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
            aether_node::dkg::run_relay_with_journal(
                p2p.keys.signer.clone(),
                aether_node::dkg::Round::dkg(p2p.validators(), round).with_chain_id(chain_id),
                None, sender, receiver, journal, None,
            ).await
        });
        if let Err(error) = result {
            eprintln!("dkg relay failed: {error}");
            std::process::exit(1);
        }
    }
}

/// This validator's threshold share: from `<data>/threshold.json` (DKG) when
/// present, else the devnet dealer's (insecure: the dealer knows every share).
/// Audit 6: the one fail-closed bind every path to a consensus signature
/// reaches — `aether node --network … --data …` (this gate, before anything
/// starts), `aether run` (after adopt_network, over the adopted file) and
/// verify-local (`aether mainnet-bind`). A devnet (no --network) and the
/// legacy testnet chain pass through; a new genesis without the ceremony
/// record refuses here, naming the operator step.
fn bind_to_checked_genesis(network: Option<&str>, data: &str, ceremony: Option<&str>) {
    // Without --network this command builds the explicit local devnet; it
    // does not consume a network.json from the data directory.
    let Some(checked) = network.map(std::path::Path::new) else { return };
    if let Err(e) = aether_node::mainnet::bind_data_dir(
        std::path::Path::new(data),
        checked,
        ceremony.map(std::path::Path::new),
    ) {
        eprintln!("refusing to start: {e}");
        std::process::exit(1);
    }
}

/// The rules `aether mainnet-rules` prints for `file` at `path`: the 21
/// genesis rules, the release pin and the 4 final-file gates, plus the bundled-record rule
/// under `--bundle`. A bundle that is not a new genesis (the legacy 7780
/// testnet app) runs the record rule alone: its network.json is not a launch
/// candidate, and the only thing the release gate asks of it is that it ship
/// no record — the mainnet genesis rules would fail every testnet app build
/// until the chain id changes.
fn mainnet_rules(
    file: &aether_node::roster::NetworkFile,
    path: &std::path::Path,
    rehearsal: bool,
    bundle: bool,
) -> Result<Vec<aether_node::mainnet::Rule>, String> {
    if bundle {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if aether_node::mainnet::shipped_legacy_network(&bytes) {
            return Ok(vec![aether_node::mainnet::check_bundle(path, file, &bytes)]);
        }
    }
    let genesis = file.genesis()?;
    let chain_id = file.chain_id;
    let cfg = chain_config(chain_id, &genesis, false);
    let mut rules = aether_node::mainnet::check_with(&cfg, rehearsal);
    // Checklist B6: what the app's updater trusts (no rehearsal allowance —
    // a throwaway builder key set satisfies it).
    rules.push(aether_node::mainnet::check_release(&cfg, file.release.as_ref()));
    // Audit 5, A5-4: the strict gate also decodes the exact committee
    // fields the file carries (or names the file pre-DKG).
    rules.extend(aether_node::mainnet::check_final(file, rehearsal));
    // The release gate: the app-bundle pair must ship the record that
    // pins the exact bytes the check passed.
    if bundle {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        rules.push(aether_node::mainnet::check_bundle(path, file, &bytes));
    }
    Ok(rules)
}

/// `aether run`'s half of the audit 6 bind: adopt_network has put the network
/// in `<data>/network.json` (or refused); bind that file — the one the
/// supervisor will hand to `aether node` — to the ceremony record. The record
/// is resolved fail-closed (`mainnet::resolve_ceremony_record`): --ceremony,
/// the copy verify-local stored in the data dir, or the one next to the
/// --network file — the app-bundle pair a consumer Mac runs, and the file a
/// reshare seated Mac still restarts on. A Mac with a threshold share gets
/// the full signer's bind; a follower or candidate Mac (no share to vote
/// with) is bound to the record's half and follows with a warning when no
/// record is reachable at all. The resolved record is returned for the
/// supervisor to pass the child as --ceremony.
/// Validate an incoming file before adoption can move its stored pin or share.
fn bind_incoming_network(
    dir: &std::path::Path,
    network: Option<&str>,
    ceremony: Option<&str>,
) -> Result<(), String> {
    let Some(path) = network.map(std::path::Path::new) else { return Ok(()) };
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let file = serde_json::from_slice(&bytes).map_err(|e| format!("{} is not a network.json: {e}", path.display()))?;
    if let Some(record) = aether_node::mainnet::resolve_ceremony_record(
        ceremony.map(std::path::Path::new), dir, Some(path),
    ) {
        aether_node::mainnet::bind_network_to_record(&file, &bytes, &aether_node::mainnet::load_ceremony_record(&record)?)?;
    } else if dir.join("threshold.json").exists() && !aether_node::mainnet::shipped_legacy_network(&bytes) {
        return Err("no ceremony record: verify-local must bind the incoming network before a signer can adopt it".into());
    }
    Ok(())
}

fn bind_run_to_ceremony(
    dir: &std::path::Path,
    network: Option<&str>,
    ceremony: Option<&str>,
) -> Result<Option<std::path::PathBuf>, String> {
    let local = dir.join("network.json");
    let bytes = std::fs::read(&local).map_err(|e| format!("cannot read {}: {e}", local.display()))?;
    let file: aether_node::roster::NetworkFile = serde_json::from_slice(&bytes)
        .map_err(|e| format!("{} is not a network.json: {e}", local.display()))?;
    let record = aether_node::mainnet::resolve_ceremony_record(
        ceremony.map(std::path::Path::new),
        dir,
        network.map(std::path::Path::new),
    );
    if record.is_none() && aether_node::mainnet::shipped_legacy_network(&bytes) {
        return Ok(None);
    }
    if dir.join("threshold.json").exists() {
        aether_node::mainnet::bind_data_dir(dir, &local, record.as_deref())?;
    } else {
        aether_node::mainnet::bind_shareless_to_ceremony(&file, &bytes, dir, record.as_deref())?;
    }
    Ok(record)
}

fn committee_keys(
    data: &str,
    validators: &commonware_utils::ordered::Set<PublicKey>,
    me: &PublicKey,
    expected_round: Option<u64>,
    chain_id: u64,
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
        assert!(
            chain_id == 7_780
                || !aether_node::dkg::KeyFile::reveals_seated_share(&output, validators),
            "threshold.json reveals a seated player's threshold share; do not run consensus with this committee key"
        );
        // Audit 5 A5-4's threshold-vs-network.json comparison (and audit 6's
        // A6-4 finding that its nesting here silently skipped on any missing
        // file) moved to the one fail-closed bind at startup:
        // bind_to_checked_genesis → mainnet::bind_data_dir, which every path
        // to a signature reaches before this point.
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

/// This Mac's wallet-server node key: dedicated and persisted (`<data>/wallet-node.key`
/// by default, `--node-key` under `aether run --chain-data/--archive`), so the
/// DHT record it publishes does not flap with the endpoint other roles reuse.
/// Regenerating it only changes which node id wallets are pointed at.
fn wallet_node_key(path: &std::path::Path) -> Result<aether_net::SecretKey, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut b = [0u8; 32];
            b.copy_from_slice(&bytes);
            Ok(aether_net::SecretKey::from_bytes(&b))
        }
        _ => {
            let key = aether_net::SecretKey::generate();
            write_secret(path, &key.to_bytes());
            Ok(key)
        }
    }
}

fn write_secret(path: &std::path::Path, bytes: &[u8]) {
    aether_node::atomic::replace(path, bytes, 0o600).expect("write key file");
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

fn candidate_registration_uses_lane(status: &Value) -> bool {
    status["free_registration"].as_bool().unwrap_or(false)
}

/// Fee caps from the node's next base fees: 2x headroom (~70 full blocks of
/// growth) plus a 1 gwei tip; only the actual base + tip is charged. The
/// state cap has the same 2x headroom over the B5 price, never under the
/// floor (contracts-live bug #5: a cap at today's price leaves a queued tx
/// unincludable after the next burst). Only the legacy 7780 chain signs a
/// zero state cap; elsewhere a 0 or missing price takes the floor, and a
/// malformed one is refused (round 2, finding 6).
const TIP: u128 = 1_000_000_000;

fn fee_caps(status: &Value, tip: u128) -> Result<aether_types::FeeVector, String> {
    let get = |k: &str| {
        status["base_fee"][k]
            .as_str()
            .and_then(|v| v.parse::<u128>().ok())
            .ok_or(format!("status has no base_fee.{k}"))
    };
    // The app's rule (B5 review round 2, finding 6): a zero state cap only on
    // the known stateless legacy chain. Elsewhere a zero or missing report
    // clamps to the fixed unit price — the spec's "clamp" branch, so a test
    // devnet whose node omits the field still sends (a cap and budget are
    // harmless where state is unpriced) — and a malformed one is refused.
    let chain = status["chain_id"].as_u64().ok_or("status has no chain_id")?;
    let reported = status["base_fee"].get("state").map_or(Some("0"), Value::as_str);
    let state_price = aether_execution::tx::wallet_state_price(chain, reported)?;
    Ok(aether_types::FeeVector {
        exec: get("exec")? * 2 + tip,
        state: aether_execution::fees::signed_state_cap(state_price),
        prove: get("prove")? * 2,
    })
}

fn submit(rpc: &str, dev: u8, nonce: Option<u64>, c: EvmCall, wait: bool) -> Result<Value, String> {
    submit_with_tip(rpc, dev, nonce, c, wait, TIP)
}

/// `submit` with an explicit priority fee; a zero tip does not waive base or
/// state-growth fees on the contract transaction path.
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
    let max_fee = fee_caps(&status, tip)?;
    let mut tx = sign_call_with(&signer, chain_id, nonce, max_fee, tip, &c)
        .map_err(|e| e.to_string())?;
    if max_fee.state != 0 {
        let balance_hex = call(rpc, "eth_getBalance", json!([from]))?;
        let balance = balance_hex.as_str().and_then(|h| U256::from_str_radix(h.trim_start_matches("0x"), 16).ok());
        tx.header.gas.state = aether_execution::recommended_state_budget(&c, balance, max_fee.state);
        let mut sig = signer.sign(&tx.signing_bytes()).map_err(|e| e.to_string())?;
        sig.extend_from_slice(&signer.public_key().bytes);
        tx.signature = Bytes::from(sig);
    }
    let r = call(rpc, "aether_sendTransaction", json!([tx]))?;
    let hash: TxHash = serde_json::from_value(r["hash"].clone()).map_err(|e| e.to_string())?;
    println!("tx {hash}  from {from}  nonce {nonce}  (signed with P-256)");
    if !wait {
        return Ok(Value::Null);
    }
    wait_for_receipt(rpc, hash)
}

fn wait_for_receipt(rpc: &str, hash: TxHash) -> Result<Value, String> {
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
        // Bug #5: a tx that left the pool says why instead of timing out.
        if r["status"] == "dropped" {
            return Err(format!("not included: dropped from the mempool ({})", r["reason"]));
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
    #[test]
    fn r06_explicit_pid1_and_missing_sender_have_distinct_parent_contracts() {
        assert_eq!(super::initial_writer_parent(Some(1), 1), Some(1),
            "R06 a validated explicit init supervisor remains supported");
        assert_eq!(super::initial_writer_parent(None, 1), None,
            "R06 missing sender never adopts launchd");
        assert_eq!(super::initial_writer_parent(Some(42), 1), None,
            "R06 late startup rejects the lost sender");
        assert_eq!(super::initial_writer_parent(Some(42), 42), Some(42));
        assert_eq!(super::initial_writer_parent(None, 42), Some(42));
        assert_eq!(super::initial_writer_parent(Some(0), 0), None);
    }

    /// `aether run --archive` spawns a follower that runs exactly like
    /// `aether archive`: the era export is on, so it neither starts from a
    /// snapshot nor jumps to one later (audit 7 A7-1).
    #[test]
    fn the_archive_child_never_jumps() {
        use clap::Parser as _;
        let argv = aether_node::supervisor::follower_args(
            std::path::Path::new("/n/network.json"), std::path::Path::new("/n"), Some(std::path::Path::new("/Volumes/E")),
            true, 18545, true, &[]);
        let c = super::Cli::try_parse_from(std::iter::once("aether".to_string()).chain(argv)).expect("the child argv parses");
        let super::Cmd::Follow { data, checkpoint, archive_export, candidate, .. } = c.cmd else { panic!("a follow child") };
        assert_eq!(data, "/Volumes/E/archive");
        assert!(candidate, "archive keeps beaconing");
        let export = super::follow_export(archive_export, &data).expect("archive mode exports eras");
        assert_eq!(export.dir, std::path::PathBuf::from("/Volumes/E/archive/era"));
        assert_eq!(export.sign_key, std::path::PathBuf::from("/Volumes/E/archive/archive-export.key"));
        assert_eq!(super::sync_plan(checkpoint, true), (false, true), "no snapshot start, no snapshot jump");

        // The normal follower keeps both shortcuts.
        let argv = aether_node::supervisor::follower_args(
            std::path::Path::new("/n/network.json"), std::path::Path::new("/n"), None, false, 18545, true, &[]);
        let c = super::Cli::try_parse_from(std::iter::once("aether".to_string()).chain(argv)).unwrap();
        let super::Cmd::Follow { data, checkpoint, archive_export, .. } = c.cmd else { panic!("a follow child") };
        assert!(super::follow_export(archive_export, &data).is_none());
        assert_eq!(super::sync_plan(checkpoint, false), (true, false));

        // `aether run` takes both storage flags.
        let c = super::Cli::try_parse_from(["aether", "run", "--data", "/n", "--chain-data", "/Volumes/E", "--archive"]).unwrap();
        let super::Cmd::Run { chain_data, archive, .. } = c.cmd else { panic!("run") };
        assert_eq!(chain_data.as_deref(), Some("/Volumes/E"));
        assert!(archive);
    }

    #[test]
    fn validator_liveness_requires_a_peer_ahead_and_a_frozen_local_head() {
        use super::validator_is_stalled;
        use std::time::Duration;
        let five_minutes = Duration::from_secs(5 * 60);
        assert!(!validator_is_stalled(100, &[10_000, 10_000], five_minutes - Duration::from_secs(1)));
        assert!(!validator_is_stalled(100, &[100, 10_000], five_minutes), "one lying peer cannot force a restart");
        assert!(validator_is_stalled(100, &[100, 10_000, 10_001], five_minutes));
    }
    use super::*;

    /// Contracts-live bug #5: `aether send` signed the state cap at the
    /// current price, so the next burst left it unincludable. Round 2,
    /// finding 6: a zero cap only on the known stateless legacy chain (as the
    /// app's `state_price_for` decides). On a paid chain a stale or faulty
    /// "0" or a missing price clamps to the floor — before the fix it signed
    /// `max_fee.state = 0` and skipped the state budget — and a malformed
    /// price is refused rather than signed as free.
    #[test]
    fn cli_state_cap_has_headroom_and_zero_only_on_the_legacy_chain() {
        use aether_execution::fees::STATE_UNIT_PRICE;
        let status = |chain: u64, state: Option<&str>| match state {
            Some(p) => json!({ "chain_id": chain, "base_fee": { "exec": "0", "prove": "0", "state": p } }),
            None => json!({ "chain_id": chain, "base_fee": { "exec": "0", "prove": "0" } }),
        };
        let price = STATE_UNIT_PRICE.to_string();
        assert_eq!(fee_caps(&status(7796, Some(&price)), TIP).unwrap().state, 2 * STATE_UNIT_PRICE);
        assert_eq!(fee_caps(&status(7796, Some(&(43 * STATE_UNIT_PRICE).to_string())), TIP).unwrap().state, 86 * STATE_UNIT_PRICE);
        assert_eq!(fee_caps(&status(7796, Some("0")), TIP).unwrap().state, 2 * STATE_UNIT_PRICE, "a paid chain's zero report clamps to the floor");
        assert_eq!(fee_caps(&status(7796, None), TIP).unwrap().state, 2 * STATE_UNIT_PRICE, "a missing price takes the floor, never 0");
        assert!(fee_caps(&status(7796, Some("free")), TIP).is_err(), "a malformed price is not a zero");
        assert!(fee_caps(&json!({ "chain_id": 7796, "base_fee": { "exec": "0", "prove": "0", "state": 5 } }), TIP).is_err(), "a non-string price is malformed");
        // The legacy stateless chain keeps its zero cap, whatever is reported.
        for state in [Some("0"), None, Some("3000000000000")] {
            assert_eq!(fee_caps(&status(7780, state), TIP).unwrap().state, 0);
        }
        assert!(fee_caps(&json!({ "base_fee": { "exec": "0", "prove": "0", "state": "0" } }), TIP).is_err(), "no chain id, no exception");
    }

    #[test]
    fn candidate_registration_selects_free_lane_only_when_advertised() {
        assert!(candidate_registration_uses_lane(&json!({ "free_registration": true })));
        assert!(!candidate_registration_uses_lane(&json!({ "free_registration": false })));
        assert!(!candidate_registration_uses_lane(&json!({ "chain_id": 7780 })));
    }

    /// The wallet path (crates/ffi) signs value-carrying contract calls and a
    /// per-call exec gas cap. The CLI's `call`/`deploy` must accept both, or
    /// every payable example contract is uncallable and the account contract's
    /// own deploy (3.4 M exec gas) cannot be submitted from the reference
    /// client. Defaults stay 1 M / 3 M and value 0.
    #[test]
    fn call_and_deploy_accept_value_and_gas_flags() {
        let call = Cli::try_parse_from([
            "aether", "call", "--rpc", "http://127.0.0.1:1", "--from-dev", "2",
            "--to", "0x0000000000000000000000000000000000000001", "--data", "0xabcdef",
            "--value", "5", "--gas", "2000000", "--wait",
        ]).expect("call parses --value and --gas");
        match call.cmd {
            Cmd::Call { value: Some(v), gas: Some(g), wait: true, .. } => {
                assert_eq!(v, U256::from(5u64));
                assert_eq!(g, 2_000_000);
            }
            _ => panic!("call parsed into another command"),
        }
        let deploy = Cli::try_parse_from([
            "aether", "deploy", "--rpc", "http://127.0.0.1:1", "--from-dev", "2",
            "--code", "0x600a", "--gas", "4000000",
        ]).expect("deploy parses --gas");
        match deploy.cmd {
            Cmd::Deploy { gas: Some(4_000_000), .. } => {}
            _ => panic!("deploy parsed into another command"),
        }
    }

    /// Audit 6, the run path's liveness half (the follow-up to f12495a):
    /// `aether run` binds through whichever record it can reach — the explicit
    /// --ceremony, the copy in the data dir, or the one next to the --network
    /// file (the app-bundle pair) — and a Mac with no share and no record
    /// anywhere follows with a warning instead of refusing to run. The
    /// resolved record is what the supervisor hands the child as --ceremony,
    /// and a Mac seated by a later reshare round binds to the bundled round-0
    /// record cleanly, with no operator step.
    #[test]
    fn the_run_path_binds_through_the_reachable_record_and_follows_without_one() {
        use aether_node::roster::{Member, NetworkFile};
        let new_genesis = |round: u64| NetworkFile {
            chain_id: 7_801,
            validators: vec![Member { key: "01".into(), node: "node".into() }],
            identity: Some("aa".repeat(32)),
            round,
            output: Some("bb".repeat(48)),
            epochs: vec![],
            faucet: None,
            registrar: None,
            epoch_blocks: None,
            min_streak: None,
            draw_epochs: None,
            history: Some(2),
            protocol: Some(3),
            node_rewards: Some(true),
            reserve: None,
            group: None,
            max_committee: None,
            genesis_validators: Some(vec![Member { key: "01".into(), node: "node".into() }]),
            release: None,
        };
        let dir = std::env::temp_dir().join(format!("aether-runbind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (bundle, data) = (dir.join("bundle"), dir.join("data"));
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::create_dir_all(&data).unwrap();

        // The app-bundle pair: the final file and the coordinator's record
        // shipped beside it (what the release gate checks, what the app runs).
        let final_file = new_genesis(0);
        let final_bytes = serde_json::to_vec_pretty(&final_file).unwrap();
        std::fs::write(bundle.join("network.json"), &final_bytes).unwrap();
        let record = aether_node::mainnet::ceremony_record(&final_file, &final_bytes, 1_759_000_000).unwrap();
        std::fs::write(bundle.join("ceremony-check.json"), serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let net = bundle.join("network.json").to_string_lossy().into_owned();

        // A shareless Mac: binds through the bundled record, stores it, and
        // the child gets the resolved record as --ceremony. (adopt_network
        // has already copied the bundled file to <data>/network.json.)
        std::fs::write(data.join("network.json"), &final_bytes).unwrap();
        let resolved = bind_run_to_ceremony(&data, Some(&net), None)
            .expect("a consumer Mac binds through the bundled record");
        assert_eq!(resolved, Some(bundle.join("ceremony-check.json")));
        assert!(data.join("ceremony-check.json").exists(), "stored: starts without --network bind too");

        // Stored records stay authoritative when all activation flags or the id change.
        for (tag, mut altered) in [("rewards", final_file.clone()), ("history", final_file.clone()), ("chain", final_file.clone())] {
            match tag {
                "rewards" => { altered.node_rewards = Some(false); altered.history = None; }
                "history" => { altered.history = Some(1); altered.node_rewards = None; }
                _ => altered.chain_id = 7_780,
            }
            std::fs::write(data.join("network.json"), serde_json::to_vec(&altered).unwrap()).unwrap();
            assert!(bind_run_to_ceremony(&data, None, None).is_err(), "{tag} cannot bypass the stored ceremony");
        }
        std::fs::write(data.join("network.json"), &final_bytes).unwrap();
        let mut incoming = final_file.clone(); incoming.chain_id = 7_780;
        std::fs::write(bundle.join("altered.json"), serde_json::to_vec(&incoming).unwrap()).unwrap();
        let incoming_path = bundle.join("altered.json").to_string_lossy().into_owned();
        assert!(bind_incoming_network(&data, Some(&incoming_path), None).is_err(), "adoption cannot move the existing record aside before checking it");
        assert!(data.join("ceremony-check.json").exists());

        // A shareless Mac with no record anywhere follows (warns, not refuses).
        let bare = dir.join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::write(bare.join("network.json"), &final_bytes).unwrap();
        assert_eq!(
            bind_run_to_ceremony(&bare, None, None).expect("a follower Mac keeps following"),
            None
        );

        // A mismatching bundled record (pins other bytes): refuse, naming why.
        let swapped = dir.join("swap");
        std::fs::create_dir_all(&swapped).unwrap();
        let mut other = final_file.clone();
        other.output = Some("cc".repeat(48));
        std::fs::write(swapped.join("network.json"), serde_json::to_vec_pretty(&other).unwrap()).unwrap();
        std::fs::write(swapped.join("ceremony-check.json"), serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let swapped_net = swapped.join("network.json").to_string_lossy().into_owned();
        let err = bind_run_to_ceremony(&swapped, Some(&swapped_net), None).unwrap_err();
        assert!(err.contains("digest"), "{err}");

        // A Mac seated by a later reshare (round-5 local files, the round-0
        // bundled record): restarts cleanly off the bundle alone.
        let seated = dir.join("seated");
        std::fs::create_dir_all(&seated).unwrap();
        let evolved = new_genesis(5);
        std::fs::write(seated.join("network.json"), serde_json::to_vec_pretty(&evolved).unwrap()).unwrap();
        std::fs::write(
            seated.join("threshold.json"),
            serde_json::to_vec(&aether_node::dkg::KeyFile {
                round: 5,
                output: "bb".repeat(48),
                identity: "aa".repeat(32),
                share: "00".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let resolved = bind_run_to_ceremony(&seated, Some(&net), None)
            .expect("a reshare-seated Mac restarts on the bundled record alone");
        assert_eq!(resolved, Some(bundle.join("ceremony-check.json")));
        assert!(seated.join("ceremony-check.json").exists(), "stored: the next start needs no --network either");
        // Recovery can replace both active files. The final bind must inspect
        // the recovered generation, including its immutable rule flags.
        let generation = seated.join("gen/6");
        std::fs::create_dir_all(&generation).unwrap();
        let mut recovered = new_genesis(6);
        recovered.node_rewards = None;
        recovered.history = None;
        std::fs::write(generation.join("network.json"), serde_json::to_vec(&recovered).unwrap()).unwrap();
        let mut recovered_share: aether_node::dkg::KeyFile = serde_json::from_slice(&std::fs::read(seated.join("threshold.json")).unwrap()).unwrap();
        recovered_share.round = 6;
        std::fs::write(generation.join("threshold.json"), serde_json::to_vec(&recovered_share).unwrap()).unwrap();
        std::fs::write(generation.join(".installed"), b"").unwrap();
        aether_node::supervisor::finish_incomplete(&seated).unwrap();
        assert!(bind_run_to_ceremony(&seated, None, None).is_err(), "the recovered handoff cannot change the pinned genesis");
        recovered.node_rewards = Some(true);
        recovered.history = Some(2);
        std::fs::write(generation.join("network.json"), serde_json::to_vec(&recovered).unwrap()).unwrap();
        aether_node::supervisor::finish_incomplete(&seated).unwrap();
        bind_run_to_ceremony(&seated, None, None).expect("a recovered reshare of the pinned genesis restarts");


        // The genesis validator with no record anywhere still refuses.
        let signer = dir.join("signer");
        std::fs::create_dir_all(&signer).unwrap();
        std::fs::write(signer.join("network.json"), &final_bytes).unwrap();
        std::fs::write(
            signer.join("threshold.json"),
            serde_json::to_vec(&aether_node::dkg::KeyFile {
                round: 0,
                output: "bb".repeat(48),
                identity: "aa".repeat(32),
                share: "00".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let err = bind_run_to_ceremony(&signer, None, None).unwrap_err();
        assert!(err.contains("verify-local"), "the signer's refusal names the operator step: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The release gate flag: `--bundle` adds the bundled-record rule on top
    /// of the 21 genesis + release pin + 4 final-file rules (the ceremony's own first check
    /// runs without it — it writes the record only after PASS).
    #[test]
    fn the_release_gate_flag_parses() {
        let c = Cli::try_parse_from(["aether", "mainnet-rules", "--network", "n", "--bundle"])
            .expect("--bundle parses");
        let Cmd::MainnetRules { bundle, .. } = c.cmd else { panic!("mainnet-rules") };
        assert!(bundle);
        let c = Cli::try_parse_from(["aether", "mainnet-rules", "--network", "n"]).unwrap();
        let Cmd::MainnetRules { bundle, .. } = c.cmd else { panic!("mainnet-rules") };
        assert!(!bundle, "off by default: the ceremony's own check runs before the record exists");
    }

    /// The release gate must not break the app build of the legacy testnet: a
    /// bundle that is not a new genesis (the 7780 app, apps/wallet/Resources)
    /// runs the record rule alone — running the 21 mainnet genesis rules on a
    /// testnet file would fail every testnet app build until the chain id
    /// changes. A new-genesis bundle still gets the full set (26 + the record).
    #[test]
    fn the_bundle_gate_lets_the_legacy_testnet_app_build() {
        use aether_node::roster::{Member, NetworkFile};
        let file = |chain_id: u64, genesis: bool| NetworkFile {
            chain_id,
            validators: vec![Member { key: "11".repeat(32), node: aether_net::devnet_node_id(1).to_string() }],
            identity: None,
            round: 0,
            output: None,
            epochs: vec![],
            faucet: None,
            registrar: None,
            epoch_blocks: None,
            min_streak: None,
            draw_epochs: None,
            history: genesis.then_some(2),
            protocol: genesis.then_some(3),
            node_rewards: genesis.then_some(true),
            reserve: None,
            group: None,
            max_committee: None,
            genesis_validators: genesis.then_some(vec![Member { key: "11".repeat(32), node: aether_net::devnet_node_id(1).to_string() }]),
            release: None,
        };
        let dir = std::env::temp_dir().join(format!("aether-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Only the exact shipped legacy file bypasses the ceremony record.
        let legacy_bytes = include_bytes!("../../../apps/wallet/Resources/network.json");
        let legacy: NetworkFile = serde_json::from_slice(legacy_bytes).unwrap();
        let net = dir.join("legacy.json");
        std::fs::write(&net, legacy_bytes).unwrap();
        let rules = mainnet_rules(&legacy, &net, false, true)
            .expect("the legacy testnet bundle is not a launch candidate; the gate only asks that it ship no record");
        assert_eq!(rules.len(), 1, "no mainnet genesis rules for a testnet bundle: {:?}", rules.iter().map(|r| r.name).collect::<Vec<_>>());
        assert_eq!(rules[0].name, "bundled ceremony record");
        assert!(rules[0].ok, "{}", rules[0].detail);

        // A new-genesis bundle: the full set, the record rule last.
        let new_genesis = file(7_801, true);
        let net = dir.join("new.json");
        std::fs::write(&net, serde_json::to_vec(&new_genesis).unwrap()).unwrap();
        let rules = mainnet_rules(&new_genesis, &net, false, true).unwrap();
        assert_eq!(rules.len(), 27, "21 genesis + release pin + 4 final-file + the record");
        assert_eq!(rules.last().unwrap().name, "bundled ceremony record");
        // Without --bundle: the ceremony's own 26, the release pin right after
        // the genesis rules — and failing here, where the file has none.
        let rules = mainnet_rules(&new_genesis, &net, false, false).unwrap();
        assert_eq!(rules.len(), 26);
        assert_eq!(rules[21].name, "release pin");
        assert!(!rules[21].ok, "a new genesis without a release pin fails the launch check");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_reshare_timeout_exceeds_four_player_child_return() {
        let cli = Cli::try_parse_from(["aether", "run", "--data", "d"]).expect("run parses");
        let Cmd::Run { reshare_timeout, .. } = cli.cmd else { panic!("run") };
        assert_eq!(reshare_timeout, None, "the shipped default derives from the player count");
        let child = aether_node::dkg::Timeouts::default().strict_return_bound(4);
        let supervisor = aether_node::supervisor::default_reshare_timeout(4);
        assert!(supervisor > child + aether_node::dkg::POST_STAGE_RELAY);
        let override_cli = Cli::try_parse_from(["aether", "run", "--data", "d", "--reshare-timeout", "600"]).expect("override parses");
        let Cmd::Run { reshare_timeout, .. } = override_cli.cmd else { panic!("run") };
        assert_eq!(reshare_timeout, Some(600));
    }

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
