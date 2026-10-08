//! An owned, loopback-only app devnet: publish a deterministic bundle to one
//! iroh peer, deploy and register through signed EVM transactions, then let a
//! fresh wallet-node cache fetch the registered content by hash.
//!
//! The Names and AppRegistry runtimes below are deliberately test-only,
//! single-record implementations of the resolver's read ABI. This exercises
//! real execution, finalized state and RPC, without claiming production
//! contract coverage or running a consensus network. Set
//! `AETHER_APP_PAGE_HARNESS` to the already-built wallet test executable to
//! additionally run the real Swift resolver, private scheme and page bridge.

use aether_crypto::{P256Signer, Signer as AetherSigner};
use aether_execution::{recommended_state_budget, sign_call_with, EvmCall};
use aether_node::app_bundle::{self, Bundle, Cache, Service};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{
    build_payload, dev_accounts, dev_seed, Chain, ChainConfig, Executed, Extras,
};
use aether_node::follow::FinalityArchive;
use aether_node::rpc::{self, RpcState};
use aether_node::store::Store;
use aether_types::{Address, Bytes, FeeVector, GasVector, TxEnvelope, U256};
use axum::{extract::State, routing::post, Json, Router};
use base64::Engine as _;
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible, Signer as _};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CHAIN: u64 = 7_794;
const NAME: &str = "demo.sea";
const MARKER: &str = "app-content-devnet-loaded";
// Year 2100: the Swift resolver checks wall-clock seconds, not block millis.
const EXPIRES: u64 = 4_102_444_800;
const CACHE_BYTES: u64 = 32 * 1024 * 1024;

/// Every artifact lives under this worktree's ./tmp, regardless of TMPDIR.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = workspace
            .join("tmp")
            .join(format!("app-content-e2e-{}-{suffix}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Devnet {
    chain: Chain,
    last: Block,
    parent: Arc<Executed>,
    nonce: u64,
}

impl Devnet {
    fn start(dir: &Path) -> Self {
        std::fs::create_dir_all(dir).unwrap();
        let cfg = ChainConfig {
            chain_id: CHAIN,
            limits: GasVector {
                exec: 30_000_000,
                state: u64::MAX,
                prove: 200_000_000,
            },
            alloc: dev_accounts(4)
                .into_iter()
                .map(|(_, a)| (a, U256::from(10u128.pow(24))))
                .collect(),
            fees: true,
            registrar: None,
            epoch_blocks: 0,
            min_streak: None,
            draw_epochs: None,
            history_v2: true,
            protocol: 1,
            node_rewards: false,
            group: 0,
            max_committee: aether_node::rotation::GROW_UNTIL,
            committee: vec![],
            reserve: None,
        };
        let (chain, genesis) =
            Chain::open(cfg, Store::open(&dir.join("state.redb")).unwrap()).unwrap();
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        chain.lock().identity = Some(*sharing.public());
        chain.finalize(&genesis).unwrap();
        let parent = chain.lock().finalized.clone();
        Self {
            chain,
            last: genesis,
            parent,
            nonce: 0,
        }
    }

    fn transact(&mut self, to: Option<Address>, input: Vec<u8>) -> Arc<Executed> {
        let signer = P256Signer::from_seed(&dev_seed(1)).unwrap();
        let fees = FeeVector {
            exec: 100_000_000_000,
            state: aether_execution::fees::STATE_UNIT_PRICE,
            prove: 100_000_000_000,
        };
        let call = EvmCall {
            to,
            value: U256::ZERO,
            input: input.into(),
            gas_limit: 1_000_000,
            delegate: None,
        };
        let mut tx =
            sign_call_with(&signer, CHAIN, self.nonce, fees, 1_000_000_000, &call).unwrap();
        tx.header.gas.state =
            recommended_state_budget(&call, Some(U256::from(10u128.pow(24))), fees.state);
        let mut signature = AetherSigner::sign(&signer, &tx.signing_bytes()).unwrap();
        signature.extend_from_slice(&AetherSigner::public_key(&signer).bytes);
        tx.signature = Bytes::from(signature);
        self.nonce += 1;
        self.step(vec![tx])
    }

    fn step(&mut self, txs: Vec<TxEnvelope>) -> Arc<Executed> {
        let height = self.last.height.next();
        let h = height.get();
        let context = Context {
            round: Round::new(EPOCH, View::new(h)),
            leader: ed25519::PrivateKey::from_seed(h % 4).public_key(),
            parent: (View::new(h - 1), self.last.digest()),
        };
        let timestamp = 1_790_000_000_000 + h * 1000;
        let skeleton = Block::new(
            context.clone(),
            self.last.digest(),
            height,
            timestamp,
            bytes::Bytes::new(),
        );
        let ctx = Chain::block_context(&self.chain.cfg(), &skeleton, &self.parent);
        let (pre, _) = self
            .chain
            .pre_state(&self.parent, self.parent.next_protocol(), &[], None, false)
            .unwrap();
        let (payload, _) = build_payload(&self.parent, &pre, &ctx, txs, Extras::default());
        drop(pre);
        let block = Block::new(
            context,
            self.last.digest(),
            height,
            timestamp,
            payload.to_bytes(),
        );
        let executed = self.chain.execute(&block, &self.parent).unwrap();
        self.chain.finalize(&block).unwrap();
        assert_eq!(
            executed.receipts.len(),
            1,
            "the signed transaction must be included"
        );
        assert!(
            executed.receipts[0].success,
            "the transaction must execute: {:?}",
            executed.receipts[0]
        );
        self.last = block;
        self.parent = executed.clone();
        executed
    }

    fn deploy(&mut self, runtime: &[u8]) -> (Address, Arc<Executed>) {
        let len = u16::try_from(runtime.len()).unwrap().to_be_bytes();
        // CODECOPY/RETURN the runtime after this 15-byte constructor.
        let mut init = vec![
            0x61, len[0], len[1], 0x61, 0, 15, 0x60, 0, 0x39, 0x61, len[0], len[1], 0x60, 0, 0xf3,
        ];
        init.extend_from_slice(runtime);
        let executed = self.transact(None, init);
        (
            executed.receipts[0]
                .contract_address
                .expect("deployment address"),
            executed,
        )
    }
}

fn state(chain: Chain, bundles: Arc<Service>) -> RpcState {
    RpcState {
        chain,
        finality: rpc::Finality::Archive(Arc::new(FinalityArchive::new(None))),
        gossip: tokio::sync::mpsc::unbounded_channel().0,
        faucet: None,
        registrar: None,
        network: None,
        upstream: None,
        handoff: None,
        snapshot: Default::default(),
        prover: None,
        shards: None,
        app_bundles: Some(bundles),
        public_read_only: false,
    }
}

async fn call(st: &RpcState, method: &str, params: Value) -> Value {
    rpc::handle_value(
        st,
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
    )
    .await
}

async fn read(st: &RpcState, to: Address, input: Vec<u8>) -> Vec<u8> {
    let response = call(
        st,
        "eth_call",
        json!([{ "to": address_hex(to), "data": hex_data(&input) }, "latest"]),
    )
    .await;
    let output = response["result"]
        .as_str()
        .unwrap_or_else(|| panic!("eth_call failed: {response}"));
    hex::decode(output.strip_prefix("0x").expect("hex call result")).unwrap()
}

async fn assert_receipt(st: &RpcState, executed: &Executed) {
    let response = call(
        st,
        "aether_getReceipt",
        json!([executed.receipts[0].tx_hash]),
    )
    .await;
    assert_eq!(response["result"]["height"], executed.height);
    assert_eq!(response["result"]["receipt"]["success"], true);
}

fn hex_data(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}
fn address_hex(address: Address) -> String {
    hex_data(address.as_slice())
}
fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn word(value: u64) -> [u8; 32] {
    let mut out = [0; 32];
    out[24..].copy_from_slice(&value.to_be_bytes());
    out
}

fn address_word(address: Address) -> [u8; 32] {
    let mut out = [0; 32];
    out[12..].copy_from_slice(address.as_slice());
    out
}

fn abi_string(text: &str) -> Vec<u8> {
    let mut out = word(text.len() as u64).to_vec();
    out.extend_from_slice(text.as_bytes());
    out.resize(32 + text.len().div_ceil(32) * 32, 0);
    out
}

fn selector(signature: &str) -> [u8; 4] {
    alloy_primitives::keccak256(signature.as_bytes())[..4]
        .try_into()
        .unwrap()
}

fn keyed_call(selector: [u8; 4], key: &[u8; 32]) -> Vec<u8> {
    [selector.as_slice(), key.as_slice()].concat()
}

/// A tiny assembler keeps the test runtimes auditable and removes any solc
/// or forge build dependency. Only these test contracts use it.
#[derive(Default)]
struct Asm {
    bytes: Vec<u8>,
    labels: BTreeMap<&'static str, usize>,
    jumps: Vec<(usize, &'static str)>,
}

impl Asm {
    fn op(&mut self, op: u8) {
        self.bytes.push(op);
    }
    fn push(&mut self, value: u16) {
        if value <= 255 {
            self.bytes.extend_from_slice(&[0x60, value as u8]);
        } else {
            self.bytes.push(0x61);
            self.bytes.extend_from_slice(&value.to_be_bytes());
        }
    }
    fn push32(&mut self, value: &[u8; 32]) {
        self.op(0x7f);
        self.bytes.extend_from_slice(value);
    }
    fn label(&mut self, label: &'static str) {
        assert!(self.labels.insert(label, self.bytes.len()).is_none());
        self.op(0x5b);
    }
    fn jump_if(&mut self, label: &'static str) {
        self.op(0x61);
        self.jumps.push((self.bytes.len(), label));
        self.bytes.extend_from_slice(&[0, 0, 0x57]);
    }
    fn route(&mut self, selector: [u8; 4], label: &'static str) {
        self.op(0x80); // DUP1 selector
        self.op(0x63);
        self.bytes.extend_from_slice(&selector);
        self.op(0x14); // EQ
        self.jump_if(label);
    }
    fn fail(&mut self) {
        self.push(0);
        self.push(0);
        self.op(0xfd);
    }
    fn load_store(&mut self, calldata: u16, slot: u16) {
        self.push(calldata);
        self.op(0x35);
        self.push(slot);
        self.op(0x55);
    }
    fn constant_store(&mut self, value: u16, slot: u16) {
        self.push(value);
        self.push(slot);
        self.op(0x55);
    }
    fn storage_to_memory(&mut self, slot: u16, offset: u16) {
        self.push(slot);
        self.op(0x54);
        self.push(offset);
        self.op(0x52);
    }
    fn return_memory(&mut self, size: u16) {
        self.push(size);
        self.push(0);
        self.op(0xf3);
    }
    fn require_key(&mut self, slot: u16) {
        self.push(4);
        self.op(0x35);
        self.push(slot);
        self.op(0x54);
        self.op(0x14);
        self.op(0x15);
        self.jump_if("reject");
    }
    fn require_empty(&mut self, slot: u16) {
        self.push(slot);
        self.op(0x54);
        self.jump_if("reject");
    }
    fn finish(mut self) -> Vec<u8> {
        for (at, label) in self.jumps {
            let offset = u16::try_from(*self.labels.get(label).expect("jump target"))
                .unwrap()
                .to_be_bytes();
            self.bytes[at..at + 2].copy_from_slice(&offset);
        }
        self.bytes
    }
}

fn names_runtime(node: &[u8; 32]) -> Vec<u8> {
    let mut a = Asm::default();
    a.push(0);
    a.op(0x35);
    a.push(224);
    a.op(0x1c); // calldata selector
    a.route(
        selector("register(bytes32,address,address,uint64,string)"),
        "register",
    );
    a.route([0x86, 0x4c, 0xe5, 0xe1], "node");
    a.route([0x7d, 0xd5, 0x64, 0x11], "owner");
    a.route([0xb2, 0x5b, 0xe1, 0x81], "addr");
    a.route([0x9d, 0xfc, 0xc6, 0x16], "expires");
    a.route([0xff, 0xdb, 0xd0, 0xe3], "text");
    a.op(0x50);
    a.fail();

    a.label("register");
    a.op(0x50);
    a.require_empty(1);
    a.push(36);
    a.op(0x35);
    a.op(0x33);
    a.op(0x14);
    a.op(0x15); // supplied owner == CALLER
    a.jump_if("reject");
    for (offset, slot) in [
        (4, 0),
        (36, 1),
        (68, 2),
        (100, 3),
        (164, 4),
        (196, 5),
        (228, 6),
        (260, 7),
    ] {
        a.load_store(offset, slot);
    }
    a.storage_to_memory(0, 0);
    a.return_memory(32);

    a.label("node");
    a.op(0x50);
    a.push32(node);
    a.push(0);
    a.op(0x52);
    a.return_memory(32);
    for (label, slot) in [("owner", 1), ("addr", 2), ("expires", 3)] {
        a.label(label);
        a.op(0x50);
        a.require_key(0);
        a.storage_to_memory(slot, 0);
        a.return_memory(32);
    }
    a.label("text");
    a.op(0x50);
    a.require_key(0);
    a.push(32);
    a.push(0);
    a.op(0x52);
    a.storage_to_memory(4, 32);
    for (slot, offset) in [(5, 64), (6, 96), (7, 128)] {
        a.storage_to_memory(slot, offset);
    }
    a.return_memory(160);
    a.label("reject");
    a.fail();
    a.finish()
}

fn registry_runtime(app_id: &[u8; 32]) -> Vec<u8> {
    let mut a = Asm::default();
    a.push(0);
    a.op(0x35);
    a.push(224);
    a.op(0x1c);
    a.route(
        selector("publish(string,bytes32,bytes32,address,string)"),
        "publish",
    );
    a.route([0x47, 0x0d, 0x94, 0x98], "release");
    a.op(0x50);
    a.fail();

    a.label("publish");
    a.op(0x50);
    a.require_empty(0);
    a.constant_store(1, 0);
    a.load_store(36, 1);
    a.load_store(68, 2);
    a.constant_store(1, 3);
    a.push32(app_id);
    a.push(4);
    a.op(0x55);
    a.op(0x33);
    a.push(5);
    a.op(0x55);
    a.storage_to_memory(4, 0);
    a.return_memory(32);

    a.label("release");
    a.op(0x50);
    a.require_key(4);
    for slot in 0..4 {
        a.storage_to_memory(slot, slot * 32);
    }
    a.return_memory(128);
    a.label("reject");
    a.fail();
    a.finish()
}

fn registration(node: &[u8; 32], owner: Address, app_id: &[u8; 32]) -> Vec<u8> {
    let mut input = selector("register(bytes32,address,address,uint64,string)").to_vec();
    for value in [
        *node,
        address_word(owner),
        address_word(owner),
        word(EXPIRES),
        word(160),
    ] {
        input.extend_from_slice(&value);
    }
    input.extend_from_slice(&abi_string(&hex_data(app_id)));
    input
}

fn publish(manifest_hash: &[u8; 32], bundle_hash: &[u8; 32]) -> Vec<u8> {
    let mut input = selector("publish(string,bytes32,bytes32,address,string)").to_vec();
    for value in [word(160), *manifest_hash, *bundle_hash, [0; 32], word(224)] {
        input.extend_from_slice(&value);
    }
    input.extend_from_slice(&abi_string("demo"));
    input.extend_from_slice(&abi_string(""));
    input
}

async fn endpoint() -> aether_net::Endpoint {
    // Minimal has no discovery; disabling relays and binding only loopback
    // keeps the owned test completely separate from live network peers.
    aether_net::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .bind()
        .await
        .unwrap()
}

fn served_bytes(response: &Value) -> Vec<u8> {
    assert!(
        response.get("error").is_none(),
        "bundle RPC failed: {response}"
    );
    base64::engine::general_purpose::STANDARD
        .decode(response["result"]["data"].as_str().expect("base64 file"))
        .unwrap()
}

async fn run_page_harness(scratch: &Path, fixture: Value) {
    let Some(executable) = std::env::var_os("AETHER_APP_PAGE_HARNESS") else {
        eprintln!(
            "wallet page portion skipped: set AETHER_APP_PAGE_HARNESS to an already-built helper"
        );
        return;
    };
    assert!(
        Path::new(&executable).is_file(),
        "AETHER_APP_PAGE_HARNESS must name an already-built executable"
    );
    let fixture_path = scratch.join("page-fixture.json");
    std::fs::write(&fixture_path, serde_json::to_vec_pretty(&fixture).unwrap()).unwrap();
    let stdout = scratch.join("page.stdout");
    let stderr = scratch.join("page.stderr");
    let mut child = Command::new(executable)
        .args(["--fixture"])
        .arg(&fixture_path)
        .env("TMPDIR", scratch)
        .stdout(Stdio::from(std::fs::File::create(&stdout).unwrap()))
        .stderr(Stdio::from(std::fs::File::create(&stderr).unwrap()))
        .spawn()
        .expect("launch only the standalone wallet test helper");
    let deadline = Instant::now() + Duration::from_secs(45);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "wallet page helper timed out: {}\n{}",
                std::fs::read_to_string(&stdout).unwrap_or_default(),
                std::fs::read_to_string(&stderr).unwrap_or_default()
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let output = std::fs::read_to_string(&stdout).unwrap_or_default();
    let errors = std::fs::read_to_string(&stderr).unwrap_or_default();
    assert!(
        status.success(),
        "wallet page helper failed ({status}): {output}\n{errors}"
    );
    eprintln!("{output}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn published_app_resolves_from_finalized_owned_devnet_and_loads_verified_content() {
    let scratch = Scratch::new();
    let folder = scratch.0.join("app");
    std::fs::create_dir_all(&folder).unwrap();
    let html = b"<!doctype html><html><head><meta charset=\"utf-8\"><title>Owned devnet app</title><script>window.inlineViolation=true</script><script src=\"app.js\" defer></script><script type=\"module\" src=\"module.mjs\"></script></head><body><p id=\"status\">Loading verified app</p><button id=\"increment\">Increment</button><output id=\"count\">0</output></body></html>";
    let script = format!("document.getElementById('increment').addEventListener('click', () => {{const output=document.getElementById('count');output.textContent=String(Number(output.textContent)+1);}}); window.aether.request({{method:'eth_chainId'}}).then(chainId => {{document.body.dataset.chainId = chainId; document.body.dataset.marker = '{MARKER}'; document.getElementById('status').textContent = '{MARKER}';}});");
    let manifest = br#"{"schema":"eastsea-app/1","name":"Owned devnet app","entry":"index.html"}"#;
    for (name, data) in [
        ("index.html", html.as_slice()),
        ("app.js", script.as_bytes()),
        ("manifest.json", manifest.as_slice()),
        ("module.mjs", b"import {marker} from './module-value.mjs'; document.body.dataset.module = marker;".as_slice()),
        ("module-value.mjs", b"export const marker = 'module-loaded';".as_slice()),
        ("asset.svg", b"<svg xmlns='http://www.w3.org/2000/svg'><script>window.inlineViolation=true</script></svg>".as_slice()),
    ] {
        std::fs::write(folder.join(name), data).unwrap();
    }
    let bundle = Bundle::from_folder(&folder).unwrap();
    let again = Bundle::from_folder(&folder).unwrap();
    assert_eq!(
        bundle.archive(),
        again.archive(),
        "publishing the same static files is deterministic"
    );
    let bundle_hash = bundle.hash().to_string();
    let mut corrupt = bundle.archive().to_vec();
    let file_at = corrupt
        .windows(html.len())
        .position(|bytes| bytes == html)
        .expect("HTML in tar");
    corrupt[file_at] ^= 1;
    assert!(
        Bundle::from_archive(corrupt, Some(&bundle_hash)).is_err(),
        "indexed file hash mismatches are rejected"
    );
    assert!(
        Bundle::from_archive(bundle.archive().to_vec(), Some(&"f".repeat(64))).is_err(),
        "bundle index hash mismatches are rejected"
    );
    let bundle_word: [u8; 32] = hex::decode(&bundle_hash).unwrap().try_into().unwrap();
    let manifest_word: [u8; 32] = Sha256::digest(manifest).into();
    let node: [u8; 32] = alloy_primitives::keccak256(NAME.as_bytes()).into();
    let owner = dev_accounts(4)[0].1;
    let mut app_identity = [address_word(owner).as_slice(), word(64).as_slice()].concat();
    app_identity.extend_from_slice(&abi_string("demo"));
    let app_id: [u8; 32] = alloy_primitives::keccak256(&app_identity).into();

    // The publish node owns the bytes; the wallet node starts with no bundle.
    let seed_dir = scratch.0.join("publisher-node");
    app_bundle::configure(
        &seed_dir,
        app_bundle::Config {
            enabled: true,
            seed: true,
        },
    )
    .unwrap();
    let seed_cache = Arc::new(Cache::open_with_limits(&seed_dir, CACHE_BYTES, 0).unwrap());
    seed_cache.put(&bundle, true).unwrap();
    let seed_endpoint = endpoint().await;
    let seed_addr = aether_net::EndpointAddr::from_parts(
        seed_endpoint.id(),
        seed_endpoint
            .bound_sockets()
            .into_iter()
            .map(aether_net::TransportAddr::Ip),
    );
    let seed_service = Arc::new(Service::new(
        seed_cache,
        Some(seed_endpoint.clone()),
        vec![],
    ));
    let wallet_dir = scratch.0.join("wallet-node");
    app_bundle::configure(
        &wallet_dir,
        app_bundle::Config {
            enabled: true,
            seed: false,
        },
    )
    .unwrap();
    let wallet_cache = Arc::new(Cache::open_with_limits(&wallet_dir, CACHE_BYTES, 0).unwrap());
    let wallet_endpoint = endpoint().await;
    let wallet_service = Arc::new(Service::new(
        wallet_cache,
        Some(wallet_endpoint.clone()),
        vec![seed_addr],
    ));
    let mut devnet = Devnet::start(&scratch.0.join("devnet"));
    let st = state(devnet.chain.clone(), wallet_service);
    let seed_state = state(devnet.chain.clone(), seed_service.clone());
    let seed_router = aether_net::serve_with_apps(
        seed_endpoint,
        move |request| {
            let st = seed_state.clone();
            async move { rpc::handle_value(&st, request).await }
        },
        None,
        None,
        Some(seed_service.handler()),
    );

    let names_code = names_runtime(&node);
    let registry_code = registry_runtime(&app_id);
    let (names, names_deploy) = devnet.deploy(&names_code);
    let (registry, registry_deploy) = devnet.deploy(&registry_code);
    let registered = devnet.transact(Some(names), registration(&node, owner, &app_id));
    let published = devnet.transact(Some(registry), publish(&manifest_word, &bundle_word));
    for executed in [&names_deploy, &registry_deploy, &registered, &published] {
        assert_receipt(&st, executed).await;
    }
    assert_eq!(
        devnet.chain.finalized_height(),
        4,
        "deployments and registration are finalized blocks"
    );
    assert_eq!(published.receipts[0].output.as_ref(), app_id.as_slice());

    for (address, runtime) in [(names, &names_code), (registry, &registry_code)] {
        let code = call(&st, "eth_getCode", json!([address_hex(address), "latest"])).await;
        assert_eq!(
            code["result"],
            hex_data(runtime),
            "the resolver pins the actual deployed runtime"
        );
    }
    let mut node_query = [vec![0x86, 0x4c, 0xe5, 0xe1], word(32).to_vec()].concat();
    node_query.extend_from_slice(&abi_string(NAME));
    assert_eq!(
        read(&st, names, node_query).await,
        node,
        "the name resolves to its registered node"
    );
    assert_eq!(
        read(&st, names, keyed_call([0x7d, 0xd5, 0x64, 0x11], &node)).await,
        address_word(owner)
    );
    assert_eq!(
        read(&st, names, keyed_call([0xb2, 0x5b, 0xe1, 0x81], &node)).await,
        address_word(owner)
    );
    assert_eq!(
        read(&st, names, keyed_call([0x9d, 0xfc, 0xc6, 0x16], &node)).await,
        word(EXPIRES)
    );
    let mut text_query = keyed_call([0xff, 0xdb, 0xd0, 0xe3], &node);
    text_query.extend_from_slice(&word(64));
    text_query.extend_from_slice(&abi_string("app"));
    let binding = read(&st, names, text_query).await;
    assert_eq!(
        binding,
        [word(32).to_vec(), abi_string(&hex_data(&app_id))].concat(),
        "the finalized name record binds the published app ID"
    );
    let release = read(&st, registry, keyed_call([0x47, 0x0d, 0x94, 0x98], &app_id)).await;
    assert_eq!(
        release,
        [
            word(1).as_slice(),
            manifest_word.as_slice(),
            bundle_word.as_slice(),
            word(1).as_slice()
        ]
        .concat()
    );

    // The first request has to obtain the archive over iroh, then verify its
    // canonical index against the on-chain bundleHash before caching it.
    assert!(!wallet_dir
        .join("apps")
        .join(format!("{bundle_hash}.tar"))
        .exists());
    let index = call(&st, "aether_appBundle", json!([bundle_hash, "bundle.json"])).await;
    assert_eq!(served_bytes(&index), bundle.index_bytes());
    assert_eq!(sha256(&served_bytes(&index)), bundle_hash);
    assert!(
        wallet_dir
            .join("apps")
            .join(format!("{bundle_hash}.tar"))
            .is_file(),
        "the fresh cache must receive the P2P bundle"
    );
    let _ = seed_router.shutdown().await;
    for (path, expected) in [
        ("index.html", html.as_slice()),
        ("app.js", script.as_bytes()),
    ] {
        let response = call(&st, "aether_appBundle", json!([bundle_hash, path])).await;
        assert_eq!(
            served_bytes(&response),
            expected,
            "the verified cache remains usable when its publisher goes offline"
        );
        assert_eq!(response["result"]["sha256"], sha256(expected));
    }
    let traversal = call(
        &st,
        "aether_appBundle",
        json!([bundle_hash, "../state.redb"]),
    )
    .await;
    assert!(
        traversal.get("error").is_some(),
        "a bundle can never read outside its indexed static files"
    );

    // Serve the same actual node state over an ephemeral loopback RPC for the
    // separately compiled Swift helper. No EastSea application is launched.
    async fn handle(State(st): State<RpcState>, Json(request): Json<Value>) -> Json<Value> {
        Json(rpc::handle_value(&st, request).await)
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/", post(handle)).with_state(st),
        )
        .await
    });
    run_page_harness(
        &scratch.0,
        json!({
            "rpcURL": rpc_url,
            "chainID": CHAIN,
            "link": format!("sea://{NAME}"),
            "namesAddress": address_hex(names),
            "namesRuntimeSHA256": sha256(&names_code),
            "registryAddress": address_hex(registry),
            "registryRuntimeSHA256": sha256(&registry_code),
            "appID": hex_data(&app_id),
            "bundleHash": bundle_hash,
            "expectedMarker": MARKER,
        }),
    )
    .await;
    server.abort();
    let _ = server.await;
    wallet_endpoint.close().await;
}
