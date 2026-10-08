//! Self-healing (docs/design/24-self-healing.md): a shipped app runs where
//! nobody is watching, so the node and the app around it recover by themselves.
//! These are the release-condition faults of the design's test list:
//!
//! - a disk that fills once mid-follow (the incident of 2026-09-29: 288
//!   retries of one block) heals inside the node — no restart, no exit;
//! - a disk that never heals ends the process with the storage exit code (4),
//!   so the app restarts it: checked on the real `aether` binary through the
//!   hidden `--dev-storage-fault`;
//! - a garbled or truncated database is moved aside (never deleted; keys
//!   stay) and the node re-syncs through the certified snapshot jump;
//! - a network cut heals into a catch-up, with no progress while cut.

use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{build_payload, dev_accounts, dev_seed, Chain, ChainConfig, Executed, Extras};
use aether_node::follow::{self, FinalityArchive, Upstream};
use aether_node::rpc::{self, RpcState};
use aether_node::store::{self, Store};
use aether_test_support::{Port, TestChild};
use aether_types::{GasVector, U256};
use axum::{extract::State, routing::post, Json, Router};
use commonware_codec::Encode;
use commonware_consensus::marshal::Update;
use commonware_consensus::simplex::types::{Finalization, Finalize, Proposal};
use commonware_consensus::{types::{Round, View}, Reporter as _};
use commonware_cryptography::{ed25519, Digestible, Signer as _};
use commonware_parallel::Sequential;
use commonware_utils::{acknowledgement::Exact, non_empty, Acknowledgement as _};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CHAIN: u64 = 7_793;

fn config(chain_id: u64) -> ChainConfig {
    let k = aether_node::faucet::Faucet::from_seed(&dev_seed(11)).expect("dev registrar");
    let h = hex::decode(k.public_hex()).expect("registrar key hex");
    let registrar = (h[..32].try_into().expect("32"), h[32..].try_into().expect("32"));
    ChainConfig {
        chain_id,
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        alloc: dev_accounts(10).into_iter().map(|(_, a)| (a, U256::from(10u128.pow(24)))).collect(),
        fees: true,
        registrar: Some(registrar),
        epoch_blocks: 10,
        min_streak: None,
        draw_epochs: None,
        history_v2: false,
        node_rewards: false,
        protocol: 1,
        committee: Vec::new(),
        reserve: None,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
    }
}

/// The devnet a bare `aether follow` (no `--network`) computes, so the real
/// binary replays this test's blocks: same chain id, same alloc, same defaults
/// a `Genesis::default()` normalizes to (registry epoch 3,600, history v1).
fn devnet() -> ChainConfig {
    let mut cfg = config(7_777);
    cfg.epoch_blocks = 3_600;
    cfg
}

fn set() -> aether_light::ValidatorSet {
    aether_light::ValidatorSet::devnet(4)
}

/// The finalize signers: validators 1..=3 of 4 (3 of 4 meets the threshold).
fn signers() -> Vec<aether_light::Scheme> {
    let (participants, polynomial, shares) = aether_light::devnet_threshold(4);
    (1..=3u64)
        .map(|i| {
            let me = aether_light::devnet_validator_key(i).public_key();
            let share = shares.iter().find(|(pk, _)| *pk == me).map(|(_, s)| s.clone()).expect("share");
            aether_light::Scheme::signer(
                &aether_light::consensus_namespace(),
                participants.clone(),
                polynomial.clone(),
                share,
            )
            .expect("share matches polynomial")
        })
        .collect()
}

/// A chain of empty blocks with a certificate for each, as the network a
/// follower pulls from: `step` builds, executes, finalizes and certifies one.
struct Node {
    chain: Chain,
    blocks: Vec<Block>,
    parent: Arc<Executed>,
    archive: Arc<FinalityArchive>,
    signers: Vec<aether_light::Scheme>,
}

/// A chain over a store the caller opened (the self-healing tests open theirs
/// through a fault-injecting opener): finalize genesis, mark the committee.
fn chain_on(cfg: ChainConfig, store: Store) -> Node {
    let (chain, genesis) = Chain::open(cfg, store).unwrap();
    let (_, sharing, _) = aether_light::devnet_threshold(4);
    chain.lock().identity = Some(*sharing.public());
    chain.finalize(&genesis).unwrap();
    let archive = Arc::new(FinalityArchive::new(chain.store()));
    let parent = chain.lock().finalized.clone();
    Node { chain, blocks: vec![genesis], parent, archive, signers: signers() }
}

fn certify(node: &Node, block: &Block) {
    let h = block.height.get();
    let proposal = || Proposal::new(Round::new(EPOCH, View::new(h)), View::new(h - 1), block.digest());
    let votes: Vec<_> = node.signers.iter().map(|s| Finalize::sign(s, proposal()).expect("finalize vote signs")).collect();
    let fin = Finalization::from_owned_finalizes(&node.signers[0], non_empty![@votes.into_iter()], &Sequential)
        .expect("3 of 4 meets the threshold");
    node.archive.insert(
        h,
        json!({
            "height": h,
            "block": aether_light::to_hex(&block.encode()),
            "finalization": aether_light::to_hex(&fin.encode()),
            "links": [],
        }),
    );
}

/// Build the next empty block on `node`'s tip (the leader a 1 s devnet gives it).
fn build(node: &Node) -> Block {
    let prev = node.blocks.last().expect("parent block").clone();
    let height = prev.height.next();
    let h = height.get();
    let leader = ed25519::PrivateKey::from_seed(h % 4).public_key();
    let context = Context { round: Round::new(EPOCH, View::new(h)), leader, parent: (View::new(h - 1), prev.digest()) };
    let ts = 1_790_000_000_000 + h * 1000;
    let skeleton = Block::new(context.clone(), prev.digest(), height, ts, bytes::Bytes::new());
    let ctx = Chain::block_context(&node.chain.cfg(), &skeleton, &node.parent);
    let (pre, _) = node.chain.pre_state(&node.parent, node.parent.next_protocol(), &[], None, false).unwrap();
    let (payload, _) = build_payload(&node.parent, &pre, &ctx, vec![], Extras::default());
    drop(pre);
    Block::new(context, prev.digest(), height, ts, payload.to_bytes())
}

fn step(node: &mut Node) {
    let block = build(node);
    let exec = node.chain.execute(&block, &node.parent).unwrap();
    node.chain.finalize(&block).unwrap();
    certify(node, &block);
    node.blocks.push(block);
    node.parent = exec;
}

fn run_to(node: &mut Node, height: u64) {
    while node.parent.height < height {
        step(node);
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aether-selfheal-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rpc_state(node: &Node) -> RpcState {
    RpcState {
        chain: node.chain.clone(),
        finality: rpc::Finality::Archive(node.archive.clone()),
        gossip: tokio::sync::mpsc::unbounded_channel().0,
        faucet: None,
        registrar: None,
        network: None,
        upstream: None,
        handoff: None,
        snapshot: Default::default(),
        prover: None,
        shards: None,
        presence: None,
        public_read_only: false,
    }
}

/// A source node's RPC on a free port, with an optional per-request delay.
fn serve(st: &RpcState, rt: &tokio::runtime::Runtime, delay: Duration) -> (String, Arc<Mutex<Vec<String>>>) {
    #[derive(Clone)]
    struct Served {
        st: RpcState,
        calls: Arc<Mutex<Vec<String>>>,
        delay: Duration,
    }
    async fn served(State(s): State<Served>, Json(req): Json<Value>) -> Json<Value> {
        if let Some(m) = req.get("method").and_then(Value::as_str) {
            s.calls.lock().expect("calls").push(m.to_owned());
        }
        if !s.delay.is_zero() {
            tokio::time::sleep(s.delay).await;
        }
        Json(rpc::handle_value(&s.st, req).await)
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new().route("/", post(served)).with_state(Served { st: st.clone(), calls: calls.clone(), delay });
    let port_guard = Port::reserve().expect("reserve source RPC port");
    let listener = port_guard.bind_tcp().expect("bind");
    listener.set_nonblocking(true).unwrap();
    let listener = rt.block_on(async { tokio::net::TcpListener::from_std(listener).unwrap() });
    let port = listener.local_addr().unwrap().port();
    rt.spawn(async move {
        let _port = port_guard;
        axum::serve(listener, app).await.expect("serve")
    });
    (format!("http://127.0.0.1:{port}"), calls)
}

/// The same RPC behind a wire that can be cut and healed: while `cut` says so
/// every method fails, as a network partition does.
fn serve_cut(st: &RpcState, rt: &tokio::runtime::Runtime) -> (String, Arc<AtomicBool>) {
    #[derive(Clone)]
    struct Cut {
        st: RpcState,
        cut: Arc<AtomicBool>,
    }
    async fn served(State(s): State<Cut>, Json(req): Json<Value>) -> Json<Value> {
        if s.cut.load(Ordering::SeqCst) {
            return Json(json!({
                "jsonrpc": "2.0",
                "id": req.get("id").cloned().unwrap_or(Value::Null),
                "error": { "code": -32603, "message": "partitioned" }
            }));
        }
        Json(rpc::handle_value(&s.st, req).await)
    }
    let cut = Arc::new(AtomicBool::new(false));
    let app = Router::new().route("/", post(served)).with_state(Cut { st: st.clone(), cut: cut.clone() });
    let port_guard = Port::reserve().expect("reserve partitionable-source RPC port");
    let listener = port_guard.bind_tcp().expect("bind");
    listener.set_nonblocking(true).unwrap();
    let listener = rt.block_on(async { tokio::net::TcpListener::from_std(listener).unwrap() });
    let port = listener.local_addr().unwrap().port();
    rt.spawn(async move {
        let _port = port_guard;
        axum::serve(listener, app).await.expect("serve")
    });
    (format!("http://127.0.0.1:{port}"), cut)
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The disk of the incident of 2026-09-29: fine until it fills, and full for
/// `WINDOW` once it does (a user freeing space, moments later). Exactly one
/// incident per arm.
struct Incident {
    armed: AtomicBool,
    until: Mutex<Option<Instant>>,
    denied: AtomicU64,
}

const WINDOW: Duration = Duration::from_millis(400);

impl Incident {
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
    fn full(&self) -> bool {
        if self.armed.swap(false, Ordering::SeqCst) {
            *self.until.lock().unwrap() = Some(Instant::now() + WINDOW);
        }
        let full = self.until.lock().unwrap().is_some_and(|t| Instant::now() < t);
        if full {
            self.denied.fetch_add(1, Ordering::SeqCst);
        }
        full
    }
    fn denied(&self) -> u64 {
        self.denied.load(Ordering::SeqCst)
    }
}

impl Default for Incident {
    fn default() -> Self {
        Incident { armed: AtomicBool::new(false), until: Mutex::new(None), denied: AtomicU64::new(0) }
    }
}

/// A store on the incident's disk: `Store::open_with` through the fault backend.
fn store_on(dir: &Path, incident: Arc<Incident>) -> Store {
    Store::open_with(&dir.join("state.redb"), {
        Arc::new(move |p| {
            let full = incident.clone();
            store::open_on_a_full_disk(p, Arc::new(move || full.full()))
        })
    })
    .unwrap()
}

/// The incident, end to end: the disk fills while blocks keep coming, and the
/// node heals inside itself — re-opens the database, rolls back to the last
/// durable checkpoint, re-fetches what came after it — with no restart, no
/// exit, and the same state root as the network at the end.
#[test]
fn a_full_disk_during_follow_heals_without_a_restart() {
    std::env::set_var("AETHER_STORE_RECOVERY", "10,50"); // fast attempts for the test
    let (dir_src, dir_fol) = (tmp("full-src"), tmp("full-follower"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut src = chain_on(config(CHAIN), Store::open(&dir_src.join("state.redb")).unwrap());
    run_to(&mut src, 30);
    let (url, _calls) = serve(&rpc_state(&src), &rt, Duration::ZERO);

    let incident = Arc::new(Incident::default());
    let fol = chain_on(config(CHAIN), store_on(&dir_fol, incident.clone()));
    let up = || Upstream::Http(vec![url.clone()]);
    rt.block_on(follow::catch_up(&fol.chain, &up(), &set(), 0)).unwrap();
    assert_eq!(fol.chain.finalized_height(), 30);

    // Blocks keep coming, the disk fills, and it stays full for a while.
    run_to(&mut src, 35);
    incident.arm();
    rt.block_on(follow::catch_up(&fol.chain, &up(), &set(), 0)).unwrap();

    assert!(incident.denied() > 0, "the disk did fail the node");
    assert_eq!(fol.chain.finalized_height(), 35, "and the node went on without a restart");
    assert_eq!(
        fol.chain.lock().finalized.state.root(),
        src.chain.lock().finalized.state.root(),
        "the same state root as the network"
    );
    let (height, _) = fol.chain.store().expect("store").head().unwrap().expect("durable head");
    assert_eq!(height, 35, "what it reached is on disk");
    let _ = std::fs::remove_dir_all(&dir_src);
    let _ = std::fs::remove_dir_all(&dir_fol);
}

/// The same incident on a validator: the commit of a finalized block fails on
/// a full disk, the report call that marshal waits on heals the store and
/// retries the commit, and the block is acknowledged only once it is on disk
/// (an acknowledgement marshal never takes back).
#[test]
fn a_validator_heals_a_full_disk_before_acknowledging() {
    std::env::set_var("AETHER_STORE_RECOVERY", "10,50"); // fast attempts for the test
    let dir = tmp("full-validator");
    let incident = Arc::new(Incident::default());
    let mut node = chain_on(config(CHAIN), store_on(&dir, incident.clone()));
    run_to(&mut node, 5);
    incident.arm();
    let block = build(&node);
    certify(&node, &block);

    // Delivered the way marshal does: the block, and the acknowledgement it
    // waits for before it considers the block delivered.
    let delivered = block.clone();
    let digest = block.digest();
    let (ack, waiter) = Exact::handle();
    let mut app = aether_node::application::Application::new(node.chain.clone(), 0);
    let _ = app.report(Update::Block(Arc::new(block), ack));

    assert!(incident.denied() > 0, "the disk did fail the commit");
    assert_eq!(node.chain.finalized_height(), 6, "the block went on after the heal");
    let (height, _) = node.chain.store().expect("store").head().unwrap().expect("durable head");
    assert_eq!(height, 6, "what it reached is on disk");
    let rt = tokio::runtime::Runtime::new().unwrap();
    assert!(rt.block_on(waiter).is_ok(), "acknowledged — the block is on disk");
    // And the node keeps finalizing on the healed store.
    node.parent = node.chain.get(&digest).expect("executed on the healed store");
    node.blocks.push(delivered);
    step(&mut node);
    assert_eq!(node.chain.finalized_height(), 7);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A disk that never heals is not looped on: the real `aether follow` exits
/// with the storage code (4), which is the app's signal to restart the node.
#[test]
fn a_disk_that_never_heals_exits_with_the_storage_code() {
    let (dir_src, dir_fol) = (tmp("exit-src"), tmp("exit-follower"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut src = chain_on(devnet(), Store::open(&dir_src.join("state.redb")).unwrap());
    run_to(&mut src, 10);
    let (url, _calls) = serve(&rpc_state(&src), &rt, Duration::ZERO);

    // A loopback port for the child's own RPC.
    let rpc_port = Port::reserve().expect("reserve follower RPC port");

    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_aether"));
    command.args([
            "follow",
            "--data", dir_fol.to_str().unwrap(),
            "--from-rpc", &url,
            "--rpc-port", &rpc_port.to_string(),
            "--dev-storage-fault", "1200",
        ])
        .env("AETHER_STORE_RECOVERY", "5,8"); // exhaust in ~1.3 s, not ~3 min
    let child = TestChild::spawn(command, dir_fol.join("node.log")).expect("spawn aether");
    let deadline = Instant::now() + Duration::from_secs(180); // a fresh 187 MB binary can sit in dyld for a while on a busy Mac
    let status = loop {
        if std::net::TcpStream::connect(rpc_port.addr()).is_ok() {
            child.mark_started();
        }
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(Instant::now() < deadline, "the node never exited; log: {}", std::fs::read_to_string(dir_fol.join("node.log")).unwrap_or_default());
        step(&mut src); // blocks keep coming, so a commit is attempted after the disk fills
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(store::EXIT_STORAGE), "exit 4 is storage (log: {})", std::fs::read_to_string(dir_fol.join("node.log")).unwrap_or_default());
    let _ = std::fs::remove_dir_all(&dir_src);
    let _ = std::fs::remove_dir_all(&dir_fol);
}

/// A database that no longer opens as one — garbled, or truncated by a crash
/// mid-write — is moved aside (never deleted; the keys stay) and the node
/// re-syncs through the certified snapshot jump, ending at the network's state.
#[test]
fn a_damaged_database_is_moved_aside_and_the_node_resyncs() {
    let (dir_src, dir_gone) = (tmp("gone-src"), tmp("gone-follower"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut src = chain_on(config(CHAIN), Store::open(&dir_src.join("state.redb")).unwrap());
    run_to(&mut src, 2_100); // far enough that a fresh node jumps instead of replaying
    let (url, calls) = serve(&rpc_state(&src), &rt, Duration::ZERO);

    let damages: [(&str, Box<dyn Fn(&Path)>); 2] = [
        ("garbled", Box::new(|p| {
            std::fs::write(p, [0x5au8; 4_096]).unwrap();
        })),
        ("truncated", Box::new(|p| {
            let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
            f.set_len(17).unwrap();
        })),
    ];
    for (name, damage) in damages {
        let dir = dir_gone.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        // What the node left behind: a real database, and this Mac's keys.
        drop(Store::open(&dir.join("state.redb")).unwrap());
        std::fs::write(dir.join("validator.key"), b"the keys stay").unwrap();
        // A validator's consensus vote journal and a block-archive partition
        // (default partition prefix "aether"): the archive re-syncs, the vote
        // journal must never move (self-healing red team #4).
        std::fs::create_dir_all(dir.join("aether-consensus-r7")).unwrap();
        std::fs::create_dir_all(dir.join("aether-finalized-blocks-ordinal")).unwrap();
        damage(&dir.join("state.redb"));

        let (fresh, reset) = follow::open_store(&dir).unwrap();
        assert!(reset, "{name}: the damaged file was moved aside");
        assert!(fresh.head().unwrap().is_none(), "{name}: the store starts empty");
        assert!(dir.join("validator.key").exists(), "{name}: the keys were not touched");
        let aside: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("corrupt-")))
            .collect();
        assert_eq!(aside.len(), 1, "{name}: one move-aside, not a deletion");
        assert!(aside[0].join("state.redb").exists(), "{name}: the damaged file is still there");
        assert!(dir.join("aether-consensus-r7").exists(), "{name}: the vote journal stays in place");
        assert!(!aside[0].join("aether-consensus-r7").exists(), "{name}: the vote journal was not moved");
        assert!(aside[0].join("aether-finalized-blocks-ordinal").exists(), "{name}: the block archive moved with the state");

        // And it comes back: more than JUMP_BEHIND behind, it jumps to the
        // network's certified snapshot and replays the rest.
        let calls = calls.clone();
        calls.lock().unwrap().clear();
        let fol = chain_on(config(CHAIN), fresh);
        rt.block_on(follow::catch_up(&fol.chain, &Upstream::Http(vec![url.clone()]), &set(), 0)).unwrap();
        assert_eq!(fol.chain.finalized_height(), 2_100, "{name}: back at the network's tip");
        assert_eq!(
            fol.chain.lock().finalized.state.root(),
            src.chain.lock().finalized.state.root(),
            "{name}: the same state root as the network"
        );
        assert!(
            calls.lock().unwrap().contains(&"aether_snapshot".to_owned()),
            "{name}: it jumped to a certified snapshot"
        );
    }
    let _ = std::fs::remove_dir_all(&dir_src);
    let _ = std::fs::remove_dir_all(&dir_gone);
}

/// A network cut stops progress and a healed one catches up: while cut, the
/// follower holds exactly where it was; once the wire is back, it follows the
/// blocks that were finalized meanwhile.
#[test]
fn a_network_cut_heals_into_a_catch_up() {
    let (dir_src, dir_fol) = (tmp("cut-src"), tmp("cut-follower"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut src = chain_on(config(CHAIN), Store::open(&dir_src.join("state.redb")).unwrap());
    run_to(&mut src, 20);
    let (url, cut) = serve_cut(&rpc_state(&src), &rt);

    let fol = chain_on(config(CHAIN), Store::open(&dir_fol.join("state.redb")).unwrap());
    rt.spawn(follow::run(fol.chain.clone(), Arc::new(Upstream::Http(vec![url.clone()])), set(), fol.archive.clone(), None, false));
    wait_until("caught up to 20", || fol.chain.finalized_height() >= 20);
    std::thread::sleep(Duration::from_millis(500)); // settled at the tip

    cut.store(true, Ordering::SeqCst);
    run_to(&mut src, 26);
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(fol.chain.finalized_height(), 20, "while cut, no progress — and no invented progress either");

    cut.store(false, Ordering::SeqCst);
    run_to(&mut src, 30);
    wait_until("caught up to 30 after the cut healed", || fol.chain.finalized_height() >= 30);
    assert_eq!(
        fol.chain.lock().finalized.state.root(),
        src.chain.lock().finalized.state.root(),
        "the same state root as the network"
    );
    let _ = std::fs::remove_dir_all(&dir_src);
    let _ = std::fs::remove_dir_all(&dir_fol);
}

/// Red team #2: inject snapshot work while the finalized height is frozen.
/// The status RPC is called directly: this exercises the watchdog's wire
/// fields without requiring a local listening socket in restricted builds.
#[test]
fn a_frozen_height_with_rising_activity_is_not_a_stuck_node() {
    let dir = tmp("activity");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let node = chain_on(config(CHAIN), Store::open(&dir.join("state.redb")).unwrap());
    let read_status = || rt.block_on(rpc::handle_value(
        &rpc_state(&node),
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "aether_status", "params": []}),
    ));
    let before = read_status();
    aether_node::chain::set_stage(Some("snapshot"));
    for _ in 0..5 {
        aether_node::chain::tick(); // one completed snapshot chunk each
    }
    let working = read_status();
    assert_eq!(working["result"]["height"], before["result"]["height"]);
    assert!(working["result"]["activity"].as_u64().unwrap() > before["result"]["activity"].as_u64().unwrap());
    assert_eq!(working["result"]["stage"], "snapshot");
    aether_node::chain::set_stage(None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Red team #3: the app rolls back to `Helpers/aether.prev` only after asking
/// that binary what protocol it implements. The hidden arm prints it, so the
/// comparison needs no chain and no running node.
#[test]
fn a_binary_reports_the_protocol_it_implements() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_aether"))
        .arg("protocol")
        .output()
        .expect("spawn aether");
    assert!(out.status.success(), "aether protocol: {}", String::from_utf8_lossy(&out.stderr));
    let said: u32 = String::from_utf8(out.stdout).unwrap().trim().parse().unwrap();
    assert_eq!(said, aether_node::upgrade::PROTOCOL);
}

/// A database written by a newer aether (the app rolled back to `aether.prev`
/// after crashes; red team #15) is intact data, not damage: the recovery path
/// that moves broken files aside never fires, the keys and the file stay
/// exactly where they were, and the refusal names the update, not corruption.
#[test]
fn a_too_new_database_is_not_corruption_and_is_never_moved_aside() {
    let dir = tmp("too-new");
    let path = dir.join("state.redb");
    {
        let store = Store::open(&path).unwrap();
        // What a newer binary left (the on-disk contract: meta rows
        // "schema_version" and "min_read_version", u32 big-endian).
        store.put_meta("schema_version", &2u32.to_be_bytes()).unwrap();
        store.put_meta("min_read_version", &2u32.to_be_bytes()).unwrap();
        store.put_meta("height", &30u64.to_be_bytes()).unwrap();
    }
    std::fs::write(dir.join("validator.key"), b"the keys stay").unwrap();

    // The exact path every node start goes through.
    match follow::open_store(&dir) {
        Err(e) => assert!(follow::is_too_new_error(&e), "the refusal names the schema, not damage: {e}"),
        Ok(_) => panic!("a database this binary cannot read must not open"),
    }
    match Store::open(&path) {
        Err(e) => assert!(!follow::is_corruption(&e), "newer data is never corruption — nothing moves it aside"),
        Ok(_) => panic!(),
    }
    assert!(path.exists(), "the database file was not deleted");
    assert!(dir.join("validator.key").exists(), "the keys were not touched");
    assert!(
        !std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("corrupt-")),
        "no move-aside: re-syncing from scratch would destroy intact newer data"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same meeting, end to end on the real binary: `aether follow` stops with
/// the update-required exit code (3) — the existing code the supervisor
/// refuses to restart through and the app turns into "install the newer
/// release" — without touching the data directory.
#[test]
fn a_node_that_meets_a_too_new_database_exits_with_the_update_code() {
    let dir = tmp("too-new-exit");
    {
        let store = Store::open(&dir.join("state.redb")).unwrap();
        store.put_meta("schema_version", &2u32.to_be_bytes()).unwrap();
        store.put_meta("min_read_version", &2u32.to_be_bytes()).unwrap();
    }
    std::fs::write(dir.join("validator.key"), b"the keys stay").unwrap();

    // A loopback port for the child's own RPC; the upstream URL is never
    // reached — the store opens before any network.
    let rpc_port = Port::reserve().expect("reserve follower RPC port");
    let upstream = Port::reserve().expect("reserve unreachable upstream RPC port");
    let upstream_url = format!("http://{}", upstream.addr());

    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_aether"));
    command.args([
            "follow",
            "--data", dir.to_str().unwrap(),
            "--from-rpc", &upstream_url,
            "--rpc-port", &rpc_port.to_string(),
        ]);
    let child = TestChild::spawn(command, dir.join("node.log")).expect("spawn aether");
    let deadline = Instant::now() + Duration::from_secs(180); // dyld can be slow on a busy Mac
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(Instant::now() < deadline, "the node never exited; log: {}", std::fs::read_to_string(dir.join("node.log")).unwrap_or_default());
        std::thread::sleep(Duration::from_millis(50));
    };
    let log = std::fs::read_to_string(dir.join("node.log")).unwrap_or_default();
    assert_eq!(
        status.code(),
        Some(aether_node::supervisor::EXIT_UPGRADE_REQUIRED),
        "exit 3 is update-required (log: {log})"
    );
    assert!(log.contains("UPDATE REQUIRED"), "the log says what to do: {log}");
    assert!(dir.join("state.redb").exists(), "the newer data is untouched");
    assert!(dir.join("validator.key").exists(), "the keys are untouched");
    assert!(
        !std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("corrupt-")),
        "nothing moved aside"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
