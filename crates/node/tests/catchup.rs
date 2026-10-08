//! Fast catch-up after sleep (docs/design/12-launch-plan.md P1): a Mac that
//! slept for hours comes back thousands of blocks behind.
//!
//! - A follower more than `JUMP_BEHIND` behind jumps to the network's
//!   certified snapshot (checked against the certified block after it, as a
//!   checkpoint start is), keeps only certified facts of the gap, restarts on
//!   what it jumped to, and ends at the same state root as the network.
//! - A snapshot that does not check against the certified chain is refused and
//!   the follower falls back to replaying — a snapshot that omits a deployed
//!   code (every root still matching) is refused the same way.
//! - A catch-up that never heard the network's height is not success: it
//!   returns Err, for an unknown height must not read as "0 behind".
//! - A short gap replays normally, without a snapshot, and `aether_status`
//!   reports `catching_up`/`behind` while it does.
//! - A committee member that slept does not act as one until caught up: no
//!   beacon goes out while it is behind, while the other three keep
//!   finalizing. (Votes and proposals come from the consensus engine, which
//!   `aether node` starts only after `follow::catch_up` returns — the same
//!   rule, enforced structurally; beacons are the part this test can drive.)
//! - Every validator of a network restarting at once recovers on its own
//!   (2026-09-29, docs/design/24-self-healing.md: 모든 검증자 동시 재시작): each
//!   serves its stored finalized state on its public endpoint while its
//!   startup gate runs, the gates learn the network's height from each
//!   other's answers, and the chain resumes finalizing with no operator and
//!   no env var. A member far behind refuses to start while a reachable peer
//!   is ahead of it, and catches up through the gate once that peer serves
//!   blocks; a network that never answers fails open only after a real wait.
//! - Replay is pipelined: over a 150 ms link it runs at hundreds of blocks a
//!   second where one-fetch-per-block ran at ~6.

use aether_crypto::{P256Signer, Signer};
use aether_execution::registry::{encode_register, REGISTRY};
use aether_execution::{recommended_state_budget, sign_call_with, EvmCall};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{
    build_payload, dev_accounts, dev_seed, Chain, ChainConfig, Executed, Extras,
};
use aether_node::follow::{self, FinalityArchive, Upstream};
use aether_node::rpc::{self, RpcState};
use aether_node::store::Store;
use aether_types::{Address, Bytes, FeeVector, GasVector, TxEnvelope, B256, U256};
use axum::{extract::State, routing::post, Json, Router};
use commonware_codec::Encode;
use commonware_consensus::simplex::types::{Finalization, Finalize, Proposal};
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible, Signer as _};
use commonware_parallel::Sequential;
use commonware_utils::non_empty;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CHAIN: u64 = 7_792;

fn config() -> ChainConfig {
    // The public dev registrar key, as `aether network --dev-registrar` writes
    // it: the gate test registers a member through the real dev flow.
    let k = aether_node::faucet::Faucet::from_seed(&dev_seed(11)).expect("dev registrar");
    let h = hex::decode(k.public_hex()).expect("registrar key hex");
    let registrar = (
        h[..32].try_into().expect("32"),
        h[32..].try_into().expect("32"),
    );
    ChainConfig {
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
        registrar: Some(registrar),
        epoch_blocks: 10,
        min_streak: None,
        draw_epochs: None,
        history_v2: true,
        protocol: 1,
        node_rewards: false,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
        committee: vec![],
        reserve: None,
    }
}

fn set() -> aether_light::ValidatorSet {
    aether_light::ValidatorSet::devnet(4)
}

/// The finalize signers: validators 1..=3 of 4 (validator 4 is the Mac that
/// sleeps in the tests; 3 of 4 still meets the threshold).
fn signers() -> Vec<aether_light::Scheme> {
    let (participants, polynomial, shares) = aether_light::devnet_threshold(4);
    (1..=3u64)
        .map(|i| {
            let me = aether_light::devnet_validator_key(i).public_key();
            let share = shares
                .iter()
                .find(|(pk, _)| *pk == me)
                .map(|(_, s)| s.clone())
                .expect("share");
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

struct Node {
    chain: Chain,
    blocks: Vec<Block>,
    parent: Arc<Executed>,
    nonce: u64,
    /// A certificate for every block this node finalized, as `aether_getFinalized` serves them.
    archive: Arc<FinalityArchive>,
    signers: Vec<aether_light::Scheme>,
}

impl Node {
    fn start(dir: &Path) -> Node {
        let (chain, genesis) =
            Chain::open(config(), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        chain.lock().identity = Some(*sharing.public());
        chain.finalize(&genesis).unwrap();
        let parent = chain.lock().finalized.clone();
        Node {
            chain,
            blocks: vec![genesis],
            parent,
            nonce: 0,
            archive: Arc::new(FinalityArchive::new(None)),
            signers: signers(),
        }
    }

    /// The same node after a restart (no blocks rebuilt; only what the store has).
    fn resume(dir: &Path) -> Node {
        let (chain, _) =
            Chain::open(config(), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        chain.lock().identity = Some(*sharing.public());
        let parent = chain.lock().finalized.clone();
        Node {
            chain,
            blocks: vec![],
            parent,
            nonce: 0,
            archive: Arc::new(FinalityArchive::new(None)),
            signers: signers(),
        }
    }

    /// Certify `block` the way the committee does: 3 of 4 shares sign finalize
    /// votes, assembled into one threshold certificate.
    fn certify(&self, block: &Block) {
        let h = block.height.get();
        let proposal = || {
            Proposal::new(
                Round::new(EPOCH, View::new(h)),
                View::new(h - 1),
                block.digest(),
            )
        };
        let votes: Vec<_> = self
            .signers
            .iter()
            .map(|s| Finalize::sign(s, proposal()).expect("finalize vote signs"))
            .collect();
        let fin = Finalization::from_owned_finalizes(
            &self.signers[0],
            non_empty![@votes.into_iter()],
            &Sequential,
        )
        .expect("3 of 4 meets the threshold");
        self.archive.insert(
            h,
            json!({
                "height": h,
                "block": aether_light::to_hex(&block.encode()),
                "finalization": aether_light::to_hex(&fin.encode()),
                "links": [],
            }),
        );
    }

    fn step(&mut self, txs: Vec<TxEnvelope>) -> Arc<Executed> {
        let prev = self.blocks.last().expect("parent block");
        let height = prev.height.next();
        let h = height.get();
        let leader = ed25519::PrivateKey::from_seed(h % 4).public_key();
        let context = Context {
            round: Round::new(EPOCH, View::new(h)),
            leader,
            parent: (View::new(h - 1), prev.digest()),
        };
        let ts = 1_790_000_000_000 + h * 1000;
        let skeleton = Block::new(
            context.clone(),
            prev.digest(),
            height,
            ts,
            bytes::Bytes::new(),
        );
        let ctx = Chain::block_context(&self.chain.cfg(), &skeleton, &self.parent);
        let (pre, _) = self
            .chain
            .pre_state(&self.parent, self.parent.next_protocol(), &[], None, false)
            .unwrap();
        let (payload, _) = build_payload(&self.parent, &pre, &ctx, txs, Extras::default());
        drop(pre);
        let block = Block::new(context, prev.digest(), height, ts, payload.to_bytes());
        let exec = self.chain.execute(&block, &self.parent).unwrap();
        self.chain.finalize(&block).unwrap();
        self.certify(&block);
        self.blocks.push(block);
        self.parent = exec.clone();
        exec
    }

    fn submit(&mut self, call: EvmCall) -> TxEnvelope {
        let signer = P256Signer::from_seed(&dev_seed(1)).unwrap();
        let fees = FeeVector {
            exec: 100_000_000_000,
            state: aether_execution::fees::STATE_UNIT_PRICE,
            prove: 100_000_000_000,
        };
        self.nonce += 1;
        let mut tx = sign_call_with(&signer, CHAIN, self.nonce - 1, fees, 1_000_000_000, &call).unwrap();
        // config() funds this sender; reserve enough to pay for its deployed
        // code, new accounts, and storage before the snapshot is built.
        tx.header.gas.state = recommended_state_budget(
            &call,
            Some(U256::from(10u128.pow(24))),
            fees.state,
        );
        let mut signature = signer.sign(&tx.signing_bytes()).unwrap();
        signature.extend_from_slice(&signer.public_key().bytes);
        tx.signature = Bytes::from(signature);
        tx
    }

    fn transfer(&mut self) -> TxEnvelope {
        let call = EvmCall {
            to: Some(Address::repeat_byte(0xb0)),
            value: U256::from(1_000u64),
            input: Bytes::new(),
            gas_limit: 21_000,
            delegate: None,
        };
        self.submit(call)
    }

    /// Run to `height`, with a transfer in every block `with_tx` names.
    fn run_to(&mut self, height: u64, with_tx: impl Fn(u64) -> bool) -> Vec<B256> {
        let mut txs = Vec::new();
        while self.parent.height < height {
            let t = if with_tx(self.parent.height + 1) {
                vec![self.transfer()]
            } else {
                vec![]
            };
            let exec = self.step(t);
            txs.extend(exec.tx_hashes.iter().copied());
        }
        txs
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aether-catchup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn funded_source_pays_for_transfer_and_deployed_code() {
    let dir = tmp("paid-source");
    let mut source = Node::start(&dir);
    let transfer = source.transfer();
    assert_eq!(transfer.header.max_fee.state, aether_execution::fees::STATE_UNIT_PRICE);
    assert!(transfer.header.gas.state >= aether_execution::fees::STATE_ACCOUNT_UNITS);
    let transfer_budget = transfer.header.gas.state;
    let paid = source.step(vec![transfer]);
    // The new recipient account plus, since audit 6, its persisted bytes.
    assert!(paid.receipts[0].state_gas > aether_execution::fees::STATE_ACCOUNT_UNITS);
    assert!(paid.receipts[0].state_gas <= transfer_budget);
    assert!(paid.receipts[0].state_fee > U256::ZERO);

    let mut init = hex::decode("600a600c600039600a6000f3").unwrap();
    init.extend_from_slice(&hex::decode("60ff60005260206000f3").unwrap());
    let create = source.submit(EvmCall {
        to: None,
        value: U256::ZERO,
        input: init.into(),
        gas_limit: 300_000,
        delegate: None,
    });
    let deployed = source.step(vec![create]);
    assert!(deployed.receipts[0].success);
    assert!(deployed.receipts[0].state_gas > 0);
    assert!(deployed.receipts[0].state_fee > U256::ZERO);
    drop(source);
    std::fs::remove_dir_all(dir).unwrap();
}

fn rpc_state(node: &Node, registrar: Option<Arc<aether_node::devicecheck::Registrar>>) -> RpcState {
    RpcState {
        chain: node.chain.clone(),
        finality: rpc::Finality::Archive(node.archive.clone()),
        gossip: tokio::sync::mpsc::unbounded_channel().0,
        faucet: None,
        registrar,
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

async fn call(st: &RpcState, method: &str, params: Value) -> Value {
    rpc::handle_value(
        st,
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
    )
    .await
}

/// What a source node serves: its real RPC answers, every method recorded, and
/// an optional latency per request (the slow link).
#[derive(Clone)]
struct Served {
    st: RpcState,
    calls: Arc<Mutex<Vec<String>>>,
    delay: Duration,
}

async fn served(State(s): State<Served>, Json(req): Json<Value>) -> Json<Value> {
    if let Some(m) = req.get("method").and_then(Value::as_str) {
        s.calls.lock().expect("calls").push(m.to_string());
    }
    if !s.delay.is_zero() {
        tokio::time::sleep(s.delay).await;
    }
    Json(rpc::handle_value(&s.st, req).await)
}

/// A node's RPC on a free port (with every method call recorded and an
/// optional per-request delay). Returns its URL and the call log.
fn serve(
    st: &RpcState,
    rt: &tokio::runtime::Runtime,
    delay: Duration,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new().route("/", post(served)).with_state(Served {
        st: st.clone(),
        calls: calls.clone(),
        delay,
    });
    let listener = rt
        .block_on(tokio::net::TcpListener::bind(("127.0.0.1", 0)))
        .expect("bind");
    let port = listener.local_addr().unwrap().port();
    rt.spawn(async move { axum::serve(listener, app).await.expect("serve") });
    (format!("http://127.0.0.1:{port}"), calls)
}

/// What a source serves with its first snapshot request held back: the member
/// cannot pass the barrier, so the moment it is thousands of blocks behind is
/// observed directly. A jump can finish between two polls of any sampler — the
/// test does not control that interval — but it cannot pass a barrier.
#[derive(Clone)]
struct Held {
    st: RpcState,
    /// Set when the first snapshot request arrives, before it is held: by then
    /// the member has heard the network's height (the jump is the step right
    /// after `aether_status`) and has not adopted a block.
    reached: Arc<AtomicBool>,
    /// Only the first snapshot is held, so a later one cannot hang the test.
    held_once: Arc<Mutex<bool>>,
    /// Released by the test to let the held request (and the rest) through.
    go: Arc<tokio::sync::Notify>,
}

async fn served_held(State(s): State<Held>, Json(req): Json<Value>) -> Json<Value> {
    let is_snapshot = req.get("method").and_then(Value::as_str) == Some("aether_snapshot");
    if is_snapshot {
        let first = {
            let mut held = s.held_once.lock().expect("held");
            let first = !*held;
            *held = true;
            first
        };
        if first {
            s.reached.store(true, Ordering::SeqCst);
            s.go.notified().await;
        }
    }
    Json(rpc::handle_value(&s.st, req).await)
}

/// A node's RPC on a free port whose first snapshot request is held until the
/// test releases it, so a member that jumps is provably observed behind before
/// it starts jumping. Returns its URL, the flag for "the jump asked", and the
/// release.
fn serve_held(
    st: &RpcState,
    rt: &tokio::runtime::Runtime,
) -> (String, Arc<AtomicBool>, Arc<tokio::sync::Notify>) {
    let reached = Arc::new(AtomicBool::new(false));
    let go = Arc::new(tokio::sync::Notify::new());
    let app = Router::new().route("/", post(served_held)).with_state(Held {
        st: st.clone(),
        reached: reached.clone(),
        held_once: Arc::new(Mutex::new(false)),
        go: go.clone(),
    });
    let listener = rt
        .block_on(tokio::net::TcpListener::bind(("127.0.0.1", 0)))
        .expect("bind");
    let port = listener.local_addr().unwrap().port();
    rt.spawn(async move { axum::serve(listener, app).await.expect("serve") });
    (format!("http://127.0.0.1:{port}"), reached, go)
}

/// A node's RPC on a free port whose served state can be swapped in place, as
/// `run_node` swaps the read-only state it serves while catching up for the
/// full one once voting starts (same URL throughout). Records every method.
fn serve_swappable(
    st: RpcState,
    rt: &tokio::runtime::Runtime,
) -> (String, Arc<Mutex<RpcState>>, Arc<Mutex<Vec<String>>>) {
    let served = Arc::new(Mutex::new(st));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/", post(served_swappable))
        .with_state((served.clone(), calls.clone()));
    let listener = rt
        .block_on(tokio::net::TcpListener::bind(("127.0.0.1", 0)))
        .expect("bind");
    let port = listener.local_addr().unwrap().port();
    rt.spawn(async move { axum::serve(listener, app).await.expect("serve") });
    (format!("http://127.0.0.1:{port}"), served, calls)
}

async fn served_swappable(
    State((served, calls)): State<(Arc<Mutex<RpcState>>, Arc<Mutex<Vec<String>>>)>,
    Json(req): Json<Value>,
) -> Json<Value> {
    if let Some(m) = req.get("method").and_then(Value::as_str) {
        calls.lock().expect("calls").push(m.to_string());
    }
    let st = served.lock().expect("served state").clone();
    Json(rpc::handle_value(&st, req).await)
}

/// The read-only state a restarting validator serves before it votes, as
/// `run_node` builds it: the stored finalized chain answers everything that
/// needs no consensus machinery. What it can serve beyond `aether_status`
/// depends on the node's archive, exactly as on a real restart.
fn early_state(node: &Node) -> RpcState {
    rpc_state(node, None)
}

/// Every roster peer that answered `aether_status`, with its finalized height
/// — the census the startup gate runs each round (a peer that does not answer
/// is absent, not "height 0").
async fn census(urls: &[String]) -> Vec<(String, u64)> {
    let mut answered = Vec::new();
    for u in urls {
        let height = Upstream::Http(vec![u.clone()])
            .first("aether_status", json!([]))
            .await
            .ok()
            .and_then(|v| v["height"].as_u64());
        if let Some(h) = height {
            answered.push((u.clone(), h));
        }
    }
    answered
}

/// Adopt a finalized block the way the engine's backfill does: execute and
/// finalize it, keep its certificate, and move the parent. (A resumed node's
/// `blocks` starts empty; `step` needs the parent block pushed back too.)
fn adopt(n: &mut Node, block: &Block, proof: Value) {
    let h = block.height.get();
    n.chain.finalize(block).unwrap();
    n.archive.insert(h, proof);
    n.blocks.push(block.clone());
    n.parent = n.chain.lock().finalized.clone();
}

/// Every request a member sent upstream, with how far behind the member was as
/// the request arrived — read off its chain, not sampled: a request is a fact, a
/// poll can miss.
type Sent = Arc<Mutex<Vec<(String, u64)>>>;

/// A server that only records what a member would have sent upstream, on a
/// free port. Returns its URL and the log.
fn recorder(rt: &tokio::runtime::Runtime, member: Chain) -> (String, Sent) {
    let sent: Sent = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/", post(recorded))
        .with_state((sent.clone(), member));
    let listener = rt
        .block_on(tokio::net::TcpListener::bind(("127.0.0.1", 0)))
        .expect("bind");
    let port = listener.local_addr().unwrap().port();
    rt.spawn(async move { axum::serve(listener, app).await.expect("serve") });
    (format!("http://127.0.0.1:{port}"), sent)
}

/// Records what a member would have sent upstream, and answers plausibly.
async fn recorded(
    State((sent, member)): State<(Sent, Chain)>,
    Json(req): Json<Value>,
) -> Json<Value> {
    if let Some(m) = req.get("method").and_then(Value::as_str) {
        sent.lock()
            .expect("sent")
            .push((m.to_string(), member.behind()));
    }
    Json(
        json!({ "jsonrpc": "2.0", "id": req.get("id").cloned().unwrap_or(Value::Null), "result": { "hash": format!("{:0>64}", "0"), "accepted": true } }),
    )
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The snapshot gate must not read this test host's live memory pressure (a
/// busy Mac sits at WARN most of the day, which used to refuse every build):
/// pin normal pressure and roomy memory for the nodes this process plays.
/// Test-only seam — a shipped binary compiles it out (resources.rs).
fn pin_snapshot_gate() {
    aether_node::resources::set_test_readings(
        Some(aether_node::resources::PRESSURE_NORMAL),
        Some(64 * aether_node::resources::GB),
    );
}

/// A Mac that slept ~85 minutes (5,100 blocks at 1 s each) jumps to the
/// network's certified snapshot instead of replaying: the gap's blocks are not
/// kept (only certified facts of the snapshot block), the state root matches
/// the network's, and a restart resumes from what it jumped to.
#[test]
fn a_follower_that_slept_jumps_to_a_certified_snapshot() {
    pin_snapshot_gate();
    let (dir_src, dir_fol) = (tmp("jump-src"), tmp("jump-follower"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut src = Node::start(&dir_src);
    let skipped = src.run_to(5_000, |h| h % 500 == 0);
    // The snapshot the network serves for the next 120 blocks is fixed at 5_000.
    let st = rpc_state(&src, None);
    let snap = rt.block_on(call(&st, "aether_snapshot", json!([])));
    assert_eq!(snap["result"]["height"], json!(5_000), "{snap}");
    let live = src.run_to(5_100, |h| h % 40 == 0);
    let (url, calls) = serve(&st, &rt, Duration::ZERO);
    let up = || Upstream::Http(vec![url.clone()]);

    let follower = Node::start(&dir_fol);
    let adopted = rt
        .block_on(follow::catch_up(&follower.chain, &up(), &set(), 0))
        .unwrap();
    assert_eq!(adopted, 5_100, "most of it by the jump, the rest replayed");
    assert_eq!(follower.chain.finalized_height(), 5_100);
    let root = src.chain.lock().finalized.state.root();
    assert_eq!(
        follower.chain.lock().finalized.state.root(),
        root,
        "the same state root as the network"
    );
    assert!(
        calls
            .lock()
            .unwrap()
            .contains(&"aether_snapshot".to_owned()),
        "it jumped rather than replayed"
    );
    {
        let g = follower.chain.lock();
        assert!(
            !g.blocks.contains_key(&1) && !g.blocks.contains_key(&4_999),
            "the gap is not kept"
        );
        assert_eq!(
            g.blocks[&5_000].hash,
            format!("{}", src.blocks[5_000].digest()),
            "the snapshot block's certified hash is"
        );
        assert_eq!(
            g.blocks[&5_000].txs.len(),
            0,
            "only certified facts of it (its txs are in the skipped era)"
        );
        assert!(g.history_index.is_none(), "no history index over a gap");
        assert!(
            g.receipts.values().all(|(h, _)| *h > 5_000),
            "pre-jump txs are not here; post-jump ones are"
        );
        assert!(
            g.receipts.contains_key(live.last().unwrap()),
            "blocks after the jump replay for real"
        );
        assert!(
            !g.receipts.contains_key(&skipped[0]),
            "the skipped era's txs are not"
        );
    }
    // Outside the guard above: `history_proof` takes the chain's lock itself.
    assert!(
        follower.chain.history_proof(10, 5_100).is_err(),
        "gap history is served by era files, not proofs"
    );

    // A restart resumes from the jumped-to state and keeps following (60 blocks
    // behind is a replay, not another jump).
    let snapshots = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|m| *m == "aether_snapshot")
        .count();
    drop(follower);
    let follower = Node::resume(&dir_fol);
    assert_eq!(follower.chain.finalized_height(), 5_100);
    assert_eq!(follower.chain.lock().finalized.state.root(), root);
    src.run_to(5_160, |_| false);
    rt.block_on(follow::catch_up(&follower.chain, &up(), &set(), 0))
        .unwrap();
    assert_eq!(follower.chain.finalized_height(), 5_160);
    assert_eq!(
        follower.chain.lock().finalized.state.root(),
        src.chain.lock().finalized.state.root()
    );
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "aether_snapshot")
            .count(),
        snapshots,
        "no second jump for a short gap"
    );

    drop((src, follower, st));
    let _ = std::fs::remove_dir_all(&dir_src);
    let _ = std::fs::remove_dir_all(&dir_fol);
}

/// Audit 7 A7-1: an archive node must never trade its history for speed. The
/// ordinary follower above jumps to the certified snapshot and loses the gap
/// (fine — history comes back from era files); an archive that jumps has no
/// history index afterward, and era export — its whole purpose — fails
/// forever. `follow::run` with `no_jump` replays the gap instead: slower, but
/// the store ends up with the network's full history index and every block,
/// which is what it exists to hold. `run_archive` also starts it with
/// `--checkpoint` off, so a fresh archive replays from genesis too.
#[test]
fn an_archive_replays_the_gap_instead_of_snapshot_jumping() {
    pin_snapshot_gate();
    let (dir_src, dir_fol) = (tmp("archive-src"), tmp("archive-follower"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut src = Node::start(&dir_src);
    src.run_to(5_100, |h| h % 500 == 0);
    let st = rpc_state(&src, None);
    let (url, calls) = serve(&st, &rt, Duration::ZERO);

    let follower = Node::start(&dir_fol);
    let archive = std::sync::Arc::new(FinalityArchive::new(follower.chain.store()));
    let task = rt.spawn(follow::run(
        follower.chain.clone(),
        std::sync::Arc::new(Upstream::Http(vec![url])),
        set(),
        archive,
        None,
        true, // no_jump: the archive policy
    ));
    wait_until("the archive replays the gap to the tip", || {
        follower.chain.finalized_height() == 5_100
    });
    task.abort();
    assert!(
        calls.lock().unwrap().iter().all(|m| m != "aether_snapshot"),
        "an archive never asks for a snapshot"
    );
    {
        let theirs = src.chain.lock().history_index.as_ref().expect("the source built one").eras.clone();
        let g = follower.chain.lock();
        assert!(g.blocks.contains_key(&1), "replay keeps the early blocks");
        let index = g.history_index.as_ref().expect("a full replay carries the history index");
        assert_eq!(index.eras, theirs, "the same era roots as the network");
    }
    assert_eq!(
        follower.chain.lock().finalized.state.root(),
        src.chain.lock().finalized.state.root(),
        "the same state root as the network"
    );
    drop((src, follower, st));
    let _ = std::fs::remove_dir_all(&dir_src);
    let _ = std::fs::remove_dir_all(&dir_fol);
}

/// A snapshot that does not check against the certified chain — a state the
/// network never finalized — is refused, and the follower replays instead.
#[test]
fn a_bad_snapshot_is_refused_and_replayed_instead() {
    pin_snapshot_gate();
    let (dir_src, dir_fol) = (tmp("bad-src"), tmp("bad-follower"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut src = Node::start(&dir_src);
    src.run_to(2_050, |h| h % 300 == 0);
    // The served snapshot carries a state no certified block commits to: same
    // shape, one entry flipped (its BLAKE3 is self-consistent, so only the
    // check against the certified block can catch it).
    let st = rpc_state(&src, None);
    rt.block_on(call(&st, "aether_snapshot", json!([])));
    let bytes = {
        st.snapshot
            .lock()
            .unwrap()
            .clone()
            .expect("cached snapshot")
            .1
    };
    let mut snap = aether_node::snapshot::Snapshot::from_bytes(&bytes).unwrap();
    snap.entries[0].1[0] ^= 1;
    *st.snapshot.lock().unwrap() = Some((2_050, Arc::new(snap.to_bytes())));
    src.run_to(2_100, |_| false);

    let (url, calls) = serve(&st, &rt, Duration::ZERO);
    let follower = Node::start(&dir_fol);
    rt.block_on(follow::catch_up(
        &follower.chain,
        &Upstream::Http(vec![url]),
        &set(),
        0,
    ))
    .unwrap();

    assert!(
        calls
            .lock()
            .unwrap()
            .contains(&"aether_snapshot".to_owned()),
        "the jump was attempted"
    );
    assert_eq!(
        follower.chain.finalized_height(),
        2_100,
        "fell back to replaying the whole way"
    );
    assert_eq!(
        follower.chain.lock().finalized.state.root(),
        src.chain.lock().finalized.state.root()
    );
    assert!(
        follower.chain.lock().blocks.contains_key(&1),
        "a replay keeps every block"
    );

    drop((src, follower, st));
    let _ = std::fs::remove_dir_all(&dir_src);
    let _ = std::fs::remove_dir_all(&dir_fol);
}

/// A snapshot that carries the tree's code-hash leaf but not the code's bytes
/// (2026-09-29, red-team 2): every root still matches, only the execution
/// index comes up short — the first call into that contract would fail. The
/// check demands every hash the tree names, and the follower replays instead.
#[test]
fn a_snapshot_that_omits_a_deployed_code_is_refused_and_replayed_instead() {
    pin_snapshot_gate();
    let (dir_src, dir_fol) = (tmp("code-src"), tmp("code-follower"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut src = Node::start(&dir_src);
    // Deploy a contract in block 1: CODECOPY/RETURN ten runtime bytes that
    // themselves return 32 bytes of memory — code the state will name.
    let mut init = hex::decode("600a600c600039600a6000f3").unwrap();
    let runtime = hex::decode("60ff60005260206000f3").unwrap();
    init.extend_from_slice(&runtime);
    let create = EvmCall {
        to: None,
        value: U256::ZERO,
        input: init.into(),
        gas_limit: 300_000,
        delegate: None,
    };
    let create = src.submit(create);
    let exec = src.step(vec![create]);
    assert!(exec.receipts[0].success, "the contract is deployed");
    let code_hash = B256::from(alloy_primitives::keccak256(&runtime));

    src.run_to(2_050, |h| h % 300 == 0);
    let st = rpc_state(&src, None);
    rt.block_on(call(&st, "aether_snapshot", json!([])));
    let bytes = {
        st.snapshot
            .lock()
            .unwrap()
            .clone()
            .expect("cached snapshot")
            .1
    };
    let mut snap = aether_node::snapshot::Snapshot::from_bytes(&bytes).unwrap();
    assert!(
        snap.codes.iter().any(|(h, _)| *h == code_hash),
        "the deployed code rides with the snapshot"
    );
    // The pristine snapshot checks (against the certified block after it).
    src.run_to(2_051, |_| false);
    let identity = src.chain.lock().identity.expect("identity");
    assert!(
        matches!(snap.check(&src.blocks[2_051], &config(), &identity), Ok(_)),
        "the carried code passes the check"
    );
    // With the code dropped — roots untouched — it must not.
    snap.codes.retain(|(h, _)| *h != code_hash);
    let err = match snap.check(&src.blocks[2_051], &config(), &identity) {
        Err(e) => e,
        Ok(_) => panic!("the omitted code must not pass"),
    };
    assert!(err.contains("does not carry"), "{err}");
    *st.snapshot.lock().unwrap() = Some((2_050, Arc::new(snap.to_bytes())));
    src.run_to(2_100, |_| false);

    let (url, calls) = serve(&st, &rt, Duration::ZERO);
    let follower = Node::start(&dir_fol);
    rt.block_on(follow::catch_up(
        &follower.chain,
        &Upstream::Http(vec![url]),
        &set(),
        0,
    ))
    .unwrap();

    assert!(
        calls
            .lock()
            .unwrap()
            .contains(&"aether_snapshot".to_owned()),
        "the jump was attempted"
    );
    assert_eq!(
        follower.chain.finalized_height(),
        2_100,
        "fell back to replaying the whole way"
    );
    assert_eq!(
        follower.chain.lock().finalized.state.root(),
        src.chain.lock().finalized.state.root()
    );
    // The replayed state really carries the code: the contract runs again.
    assert!(follower.chain.lock().finalized.state.codes().contains_key(&code_hash));

    drop((src, follower, st));
    let _ = std::fs::remove_dir_all(&dir_src);
    let _ = std::fs::remove_dir_all(&dir_fol);
}

/// 100 blocks behind is a replay, not a snapshot jump, and while it runs
/// `aether_status` says the node is catching up and by how much.
#[test]
fn a_short_gap_replays_without_a_snapshot() {
    let (dir_src, dir_fol) = (tmp("short-src"), tmp("short-follower"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut src = Node::start(&dir_src);
    src.run_to(1_000, |h| h % 200 == 0);
    let st = rpc_state(&src, None);
    // 20 ms per request: the replay stays observably in progress.
    let (url, calls) = serve(&st, &rt, Duration::from_millis(20));

    let follower = Node::start(&dir_fol);
    let st_fol = rpc_state(&follower, None);
    let status = || rt.block_on(call(&st_fol, "aether_status", json!([])))["result"].clone();
    let catch = || {
        let (chain, url) = (follower.chain.clone(), url.clone());
        rt.spawn(
            async move { follow::catch_up(&chain, &Upstream::Http(vec![url]), &set(), 0).await },
        )
    };

    // First sync (1,000 behind): a replay from genesis, reported as catching up.
    let caught = catch();
    wait_until("the first sync in progress", || {
        status()["catching_up"] == json!(true)
    });
    rt.block_on(async { caught.await.unwrap() }).unwrap();
    let s = status();
    assert_eq!(
        (s["catching_up"].clone(), s["behind"].clone()),
        (json!(false), json!(0)),
        "{s}"
    );
    assert_eq!(follower.chain.finalized_height(), 1_000);

    // Slept 100 blocks: replays them, still without a snapshot.
    src.run_to(1_100, |_| false);
    let caught = catch();
    wait_until("the catch-up in progress", || {
        status()["catching_up"] == json!(true)
    });
    let behind = status()["behind"].as_u64();
    assert!(behind.is_some_and(|b| b > 0), "{behind:?} blocks behind");
    rt.block_on(async { caught.await.unwrap() }).unwrap();
    let s = status();
    assert_eq!(
        (s["catching_up"].clone(), s["behind"].clone()),
        (json!(false), json!(0)),
        "{s}"
    );
    assert_eq!(follower.chain.finalized_height(), 1_100);
    assert_eq!(
        follower.chain.lock().finalized.state.root(),
        src.chain.lock().finalized.state.root()
    );
    assert!(
        !calls
            .lock()
            .unwrap()
            .contains(&"aether_snapshot".to_owned()),
        "no snapshot was ever fetched"
    );

    drop((src, follower, st, st_fol));
    let _ = std::fs::remove_dir_all(&dir_src);
    let _ = std::fs::remove_dir_all(&dir_fol);
}

/// A committee member that slept 5,000+ blocks does not act as one until
/// caught up: its beacon loop sends nothing while `behind` exceeds the margin
/// (`aether node` also starts the consensus engine — its votes and proposals —
/// only after the same `catch_up` returns), and it may jump to a certified
/// snapshot to rejoin. The other three kept finalizing while it slept.
#[test]
fn a_member_that_slept_does_not_beacon_until_caught_up() {
    let (dir_src, dir_mem) = (tmp("gate-src"), tmp("gate-member"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut src = Node::start(&dir_src);
    src.run_to(20, |_| false);

    // Register the sleeping member (as `aether candidate-register` does, through
    // the dev registrar) so its beacon loop has something to do once caught up.
    let keys = aether_node::candidate::CandidateKeys::load_or_create(&dir_mem).unwrap();
    let operator = dev_accounts(4).into_iter().next().unwrap().1;
    let registrar = Arc::new(aether_node::devicecheck::Registrar::new(
        None,
        aether_node::devicecheck::Registry::open(dir_src.join("registrations.json")),
        std::sync::Arc::new(
            aether_node::registrar_signer::FileSigner::from_seed(&dev_seed(11)).unwrap(),
        ),
        CHAIN,
    ));
    let descriptor = registrar.encryption_key().unwrap();
    let (x, y) = aether_execution::registry::registrar(&src.lock().finalized.state);
    let registrar_key = aether_crypto::PublicKey { scheme: aether_types::SignerScheme::P256, bytes: [&[4u8][..], &x, &y].concat() };
    let params = aether_node::devicecheck::encrypt_token_request("dev", &descriptor, CHAIN, &registrar_key, "aether_registerDevice", vec![
        json!(operator), json!(hex::encode(keys.validator_key())), json!(hex::encode(keys.node_id())),
        json!(keys.beaconer()), json!(hex::encode(keys.ownership(CHAIN, operator))),
    ]).unwrap();
    let st = rpc_state(&src, Some(registrar));
    let a = rt.block_on(call(
        &st,
        "aether_registerDevice",
        params,
    ));
    let part = |k: &str| -> [u8; 32] {
        hex::decode(a["result"][k].as_str().expect(k))
            .ok()
            .and_then(|b| b.try_into().ok())
            .expect(k)
    };
    let reg = EvmCall {
        to: Some(REGISTRY),
        value: U256::ZERO,
        input: encode_register(
            keys.validator_key(),
            keys.node_id(),
            keys.beaconer(),
            part("r"),
            part("s"),
        ),
        gas_limit: 400_000,
        delegate: None,
    };
    let reg = src.submit(reg);
    let exec = src.step(vec![reg]);
    assert!(exec.receipts[0].success, "the member is registered");

    src.run_to(5_300, |h| h % 500 == 0);
    // Fix the snapshot the member will jump to, taken here at 5,300: still
    // inside the 120-block window the source's cache serves when the member
    // jumps at 5,400, so the jump lands below the tip — with a certified block
    // after it to check against — and the last blocks replay for real.
    rt.block_on(call(&st, "aether_snapshot", json!([])));
    // The other three finished finalizing while the member slept. Its height is
    // settled before the member starts catching up, so `behind` can only shrink
    // from here: the readings below are not raced by the source growing.
    src.run_to(5_400, |_| false);

    // The member: 5,400 behind, with the beacon loop `aether follow --candidate`
    // runs and the catch-up `aether node` runs before voting. Beacons go to a
    // server that only records, with how far behind the member was when the
    // request arrived.
    let member = Node::start(&dir_mem);
    let (record_url, sent) = recorder(&rt, member.chain.clone());
    rt.spawn(aether_node::candidate::beacon_loop(
        member.chain.clone(),
        aether_node::candidate::Outbox::Upstream(Arc::new(Upstream::Http(vec![record_url]))),
        aether_node::candidate::CandidateKeys::load_or_create(&dir_mem).unwrap(),
    ));
    let (url, reached, go) = serve_held(&st, &rt);
    let catching = {
        let (chain, url) = (member.chain.clone(), url.clone());
        rt.spawn(async move {
            follow::catch_up(&chain, &Upstream::Http(vec![url]), &set(), follow::BEHIND_MARGIN)
                .await
        })
    };
    // Held at the jump's snapshot request: the member has heard 5,400 from the
    // source and has not adopted a block, so it is provably thousands behind —
    // the fact the old sampler missed whenever the jump finished inside one
    // 25 ms interval (it could not be seen on an idle machine, only under load).
    wait_until("the member reached the jump", || reached.load(Ordering::SeqCst));
    assert_eq!(
        member.chain.finalized_height(),
        0,
        "still at genesis when the jump asked"
    );
    let behind = member.chain.behind();
    assert!(
        behind > 5_000,
        "the member was thousands of blocks behind: {behind}"
    );
    assert!(
        sent.lock().unwrap().is_empty(),
        "a beacon went out while {behind} blocks behind"
    );
    go.notify_one();

    let adopted = rt.block_on(catching).unwrap().unwrap();
    assert!(adopted > 5_000, "a jump did most of it");
    assert_eq!(member.chain.finalized_height(), 5_400);

    // Not one beacon went out while behind: every recorded send carries the
    // member's `behind` as the request arrived, and the source's height was
    // already settled — the member only adopts blocks from here, so that
    // reading cannot be above what the guard saw.
    let log = sent.lock().unwrap().clone();
    for (m, behind) in log
        .iter()
        .filter(|(m, _)| m == "aether_sendTransaction" || m == "aether_sendBeacon")
    {
        assert!(
            *behind <= follow::BEHIND_MARGIN,
            "a beacon ({m}) went out while {behind} blocks behind"
        );
    }
    assert_eq!(
        member.chain.behind_known(),
        Some(0),
        "caught up, with the height it heard kept (not cleared to unknown)"
    );
    assert_eq!(
        member.chain.lock().finalized.state.root(),
        src.chain.lock().finalized.state.root()
    );
    // And now that it is current, the beacon goes out (registered, epochs behind).
    wait_until("the caught-up member's beacon", || {
        sent.lock()
            .unwrap()
            .iter()
            .any(|(m, _)| m == "aether_sendTransaction")
    });

    drop((member, st));
    let _ = std::fs::remove_dir_all(&dir_src);
    let _ = std::fs::remove_dir_all(&dir_mem);
}

/// A catch-up that never heard the network's height is not success
/// (2026-09-29): an unreachable upstream used to read as "0 behind" — the
/// unknown height as zero — and returned Ok, starting the validator on a
/// guess. It returns Err now, and no height was learned.
#[test]
fn a_catch_up_that_never_heard_a_height_is_not_success() {
    let dir_fol = tmp("deaf-follower");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let follower = Node::start(&dir_fol);
    let err = rt
        .block_on(follow::catch_up(
            &follower.chain,
            // Nothing listens there: every request is refused.
            &Upstream::Http(vec!["http://127.0.0.1:9".into()]),
            &set(),
            follow::BEHIND_MARGIN,
        ))
        .unwrap_err();
    assert!(err.contains("never learned the network height"), "{err}");
    assert_eq!(follower.chain.behind_known(), None, "still nothing heard");

    drop(follower);
    let _ = std::fs::remove_dir_all(&dir_fol);
}

/// The startup gate over HTTP peers: the census asks every URL, blocks come
/// from the tallest peer that answered. (`run_node` runs the same gate over
/// iroh; the HTTP shape is what a test can stand up hermetically — the iroh
/// path needs the Mainline DHT.)
fn gate(
    rt: &tokio::runtime::Runtime,
    chain: &Chain,
    urls: Vec<String>,
    patience: Duration,
) -> tokio::task::JoinHandle<u64> {
    let chain = chain.clone();
    rt.spawn(async move {
        follow::catch_up_before_voting(
            &chain,
            &set(),
            follow::BEHIND_MARGIN,
            patience,
            move || {
                let urls = urls.clone();
                async move { census(&urls).await }
            },
            |u: &String| Upstream::Http(vec![u.clone()]),
        )
        .await
    })
}

/// Every validator of a network stops and restarts at once — a reboot, a
/// power cut, launchd restarting the fleet — so nobody is voting and nobody
/// would ever have answered a height: the old catch-up loop spun forever and
/// the chain came back only if an operator set AETHER_SKIP_CATCH_UP on some
/// node. Now each validator serves its stored finalized state (read-only
/// answers) before it catches up, the startup gates learn the network's
/// height from each other's censuses, and the chain resumes finalizing on
/// its own. No env var is set anywhere.
#[test]
fn every_validator_restarting_at_once_resumes_on_its_own() {
    let dirs: Vec<_> = (0..4).map(|i| tmp(&format!("heal-restart-{i}"))).collect();
    let rt = tokio::runtime::Runtime::new().unwrap();
    // The network before it stopped: validator 1 produced to 30, the others
    // followed (validator 4 missed the last block). Txs are kept out: a
    // resumed node's tx nonce starts over.
    let mut a = Node::start(&dirs[0]);
    a.run_to(30, |_| false);
    let mut others = Vec::new();
    for (i, dir) in dirs.iter().enumerate().skip(1) {
        let mut n = Node::start(dir);
        for h in 1..=if i == 3 { 29 } else { 30 } {
            let block = a.blocks[h as usize].clone();
            let proof = a.archive.get(h).expect("certified");
            adopt(&mut n, &block, proof);
        }
        others.push(n);
    }
    assert_eq!(
        [&a, &others[0], &others[1], &others[2]].map(|n| n.chain.finalized_height()),
        [30, 30, 30, 29]
    );
    // What survives the restart: the certificates, and the last block (a
    // resumed node produces from it).
    let last_block = a.blocks.last().unwrap().clone();
    let proofs: Vec<Value> = (1..=30u64).map(|h| a.archive.get(h).expect("certified")).collect();

    drop((a, others));
    let mut nodes: Vec<Node> = dirs.iter().map(|d| Node::resume(d)).collect();
    for n in &mut nodes {
        for h in 1..=n.chain.finalized_height() {
            n.archive.insert(h, proofs[(h - 1) as usize].clone());
        }
    }
    // Every validator comes up at once, each serving read-only answers from
    // its stored finalized state while its own gate runs.
    let mut urls = Vec::new();
    let mut logs = Vec::new();
    for n in &nodes {
        let (url, _served, calls) = serve_swappable(early_state(n), &rt);
        urls.push(url);
        logs.push(calls);
    }
    let gates: Vec<_> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let others: Vec<String> = urls
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, u)| u.clone())
                .collect();
            gate(&rt, &n.chain, others, follow::STARTUP_PATIENCE)
        })
        .collect();
    // All four gates decide "at the tip" from each other's answers, in
    // seconds — not the patience window — and no blocks were fetched (nobody
    // is beyond the margin ahead; the engine covers the last one).
    let adopted = rt.block_on(async {
        let mut adopted = Vec::new();
        for g in gates {
            adopted.push(
                tokio::time::timeout(Duration::from_secs(30), g)
                    .await
                    .expect("the gate decided on its own")
                    .expect("the gate task ran"),
            );
        }
        adopted
    });
    assert_eq!(adopted, vec![0, 0, 0, 0]);
    assert_eq!(
        nodes.iter().map(|n| n.chain.finalized_height()).collect::<Vec<_>>(),
        vec![30, 30, 30, 29]
    );
    for calls in &logs {
        let calls = calls.lock().unwrap().clone();
        assert!(!calls.is_empty(), "the other validators asked this one");
        assert!(
            // New-genesis gate: a claimed height is backed by a certified block
            // fetch before it counts (audit 1, A1), so the census may be followed
            // by `aether_getFinalized` probes and nothing else.
            calls.iter().all(|m| m == "aether_status" || m == "aether_getFinalized"),
            "only the census and its certificate probes were asked: {calls:?}"
        );
    }
    assert_eq!(nodes[0].chain.behind_known(), Some(0));
    assert_eq!(nodes[3].chain.behind_known(), Some(1), "one behind, within the margin");

    // And the chain resumes finalizing: validator 1 produces again (from the
    // block it restarted on), the others adopt what it finalized — validator
    // 4 takes the block it missed through the same path.
    nodes[0].blocks.push(last_block);
    nodes[0].run_to(33, |_| false);
    let root = nodes[0].chain.lock().finalized.state.root();
    let produced: Vec<(Block, Value)> = (30..=33u64)
        .map(|h| {
            (
                nodes[0].blocks[(h - 30) as usize].clone(),
                nodes[0].archive.get(h).expect("certified"),
            )
        })
        .collect();
    for n in nodes.iter_mut().skip(1) {
        for h in n.chain.finalized_height() + 1..=33 {
            let (block, proof) = produced[(h - 30) as usize].clone();
            adopt(n, &block, proof);
        }
    }
    for n in &nodes {
        assert_eq!(n.chain.finalized_height(), 33);
        assert_eq!(n.chain.lock().finalized.state.root(), root);
    }

    drop(nodes);
    for d in &dirs {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// A member far behind refuses to start while a reachable peer is ahead of
/// it, and catches up through the gate once that peer serves blocks: the
/// peer's endpoint answers `aether_status` from its stored state right away,
/// but its finalized blocks only once its own startup finished. The tallest
/// node's gate, finding nobody ahead of it, proceeds at once — that is what
/// breaks the simultaneous-restart knot.
#[test]
fn a_member_far_behind_waits_then_catches_up_through_the_gate() {
    let (dir_net, dir_mem) = (tmp("heal-ahead"), tmp("heal-behind"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut net = Node::start(&dir_net);
    net.run_to(200, |_| false);
    let mut member = Node::start(&dir_mem);
    for h in 1..=40u64 {
        let block = net.blocks[h as usize].clone();
        let proof = net.archive.get(h).expect("certified");
        adopt(&mut member, &block, proof);
    }
    let proofs: Vec<Value> = (1..=200u64).map(|h| net.archive.get(h).expect("certified")).collect();
    drop((net, member));

    // Both restart at once. The member's read-only state is complete; the
    // tallest node answers `aether_status` immediately but serves no blocks
    // yet (its history is still coming up), then swaps in the state that
    // serves them — the same URL throughout.
    let net = Node::resume(&dir_net);
    let member = Node::resume(&dir_mem);
    for h in 1..=40u64 {
        member.archive.insert(h, proofs[(h - 1) as usize].clone());
    }
    let (net_url, net_served, _net_calls) = serve_swappable(early_state(&net), &rt);
    let (mem_url, _mem_served, _mem_calls) = serve_swappable(early_state(&member), &rt);

    let net_gate = gate(&rt, &net.chain, vec![mem_url], follow::STARTUP_PATIENCE);
    let member_gate = gate(&rt, &member.chain, vec![net_url], follow::STARTUP_PATIENCE);
    let net_adopted = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(30), net_gate)
            .await
            .expect("the tallest node's gate decided")
            .expect("the gate task ran")
    });
    assert_eq!(net_adopted, 0, "nobody is ahead of the tallest node");

    // While the peer ahead serves no blocks, the member refuses to start:
    // its gate is still deciding and it has adopted nothing.
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        !member_gate.is_finished(),
        "the member's gate is still waiting: the peer ahead has not come up"
    );
    assert_eq!(member.chain.finalized_height(), 40, "nothing adopted meanwhile");
    // An unproven claim does not set the network height (audit 1, A1): until the
    // peer ahead can show a certified block the member only knows it must wait.
    assert_eq!(member.chain.behind_known(), None, "an unproven claim is not a known height");

    // The tallest node finishes starting: the same endpoint now serves the
    // full state, history included.
    for h in 1..=200u64 {
        net.archive.insert(h, proofs[(h - 1) as usize].clone());
    }
    *net_served.lock().unwrap() = rpc_state(&net, None);
    let adopted = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(60), member_gate)
            .await
            .expect("the member's gate decided once the peer came up")
            .expect("the gate task ran")
    });
    assert_eq!(adopted, 160, "caught up through the gate");
    assert_eq!(member.chain.finalized_height(), 200);
    assert_eq!(member.chain.behind_known(), Some(0));
    assert_eq!(
        member.chain.lock().finalized.state.root(),
        net.chain.lock().finalized.state.root(),
        "the same state the tallest node has"
    );

    drop((net, member));
    let _ = std::fs::remove_dir_all(&dir_net);
    let _ = std::fs::remove_dir_all(&dir_mem);
}

/// A network that never answers — the peers are gone for good, or this Mac
/// is partitioned — fails open after a real wait: the gate warns and starts
/// voting anyway (the chain's own rules keep a stale member harmless), not
/// instantly as the old code did.
#[test]
fn a_network_that_never_answers_fails_open_after_a_real_wait() {
    let dir = tmp("heal-silent");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut node = Node::start(&dir);
    node.run_to(5, |_| false);
    let patience = Duration::from_millis(700);
    let t = Instant::now();
    let adopted = rt.block_on(follow::catch_up_before_voting(
        &node.chain,
        &set(),
        follow::BEHIND_MARGIN,
        patience,
        || async { census(&["http://127.0.0.1:9".into()]).await },
        |u: &String| Upstream::Http(vec![u.clone()]),
    ));
    assert_eq!(adopted, 0);
    assert_eq!(node.chain.finalized_height(), 5, "nothing was adopted from nowhere");
    assert!(t.elapsed() >= patience, "it really waited out the patience window");
    assert!(t.elapsed() < Duration::from_secs(30), "and did not hang forever");
    assert_eq!(node.chain.behind_known(), None, "nothing was ever heard");

    drop(node);
    let _ = std::fs::remove_dir_all(&dir);
}

/// One fetch per block (the old loop) against a 150 ms link runs at ~6 blocks/s;
/// pipelined batches run at hundreds. Both followers reach the same state.
#[test]
fn pipelined_following_beats_serial_fetches_over_a_slow_link() {
    let (dir_src, dir_b, dir_a) = (tmp("slow-src"), tmp("slow-before"), tmp("slow-after"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut src = Node::start(&dir_src);
    src.run_to(448, |h| h % 64 == 0);
    let st = rpc_state(&src, None);
    let (url, _calls) = serve(&st, &rt, Duration::from_millis(150));
    let up = || Upstream::Http(vec![url.clone()]);

    // Before: one network round trip per block, executed in between.
    let before = {
        let follower = Node::start(&dir_b);
        let t = Instant::now();
        rt.block_on(async {
            for h in 1..=64u64 {
                let (block, _) = follow::fetch(&up(), &set(), h).await.unwrap().unwrap();
                follower.chain.finalize(&block).unwrap();
            }
        });
        t.elapsed()
    };

    // After: the same machinery, batched.
    let after = {
        let follower = Node::start(&dir_a);
        let t = Instant::now();
        rt.block_on(follow::catch_up(&follower.chain, &up(), &set(), 0))
            .unwrap();
        t.elapsed()
    };
    assert_eq!(follower_height(&dir_a), 448);

    let (rate_before, rate_after) = (64f64 / before.as_secs_f64(), 448f64 / after.as_secs_f64());
    println!("catch-up over a 150 ms link: {rate_before:.1} blocks/s one-fetch-per-block ({before:.1?} for 64), {rate_after:.1} blocks/s pipelined ({after:.1?} for 448)");
    // The product target (hundreds of blocks/s over a 150 ms link) is a release
    // number: a debug build spends ~20 ms per block just executing and committing
    // it, which no amount of pipelining can hide behind a 150 ms round trip.
    let (floor, factor) = if cfg!(debug_assertions) {
        (30.0, 4.0)
    } else {
        (200.0, 10.0)
    };
    assert!(
        rate_after >= floor,
        "the pipelined follower must run far faster than one-per-block, not {rate_after:.1}"
    );
    assert!(
        rate_after > factor * rate_before,
        "and clearly faster than one-per-block at {rate_before:.1}"
    );

    drop((src, st));
    let _ = std::fs::remove_dir_all(&dir_src);
    let _ = std::fs::remove_dir_all(&dir_b);
    let _ = std::fs::remove_dir_all(&dir_a);
}

fn follower_height(dir: &Path) -> u64 {
    let store = Store::open(&dir.join("state.redb")).unwrap();
    store.head().unwrap().unwrap().0
}
