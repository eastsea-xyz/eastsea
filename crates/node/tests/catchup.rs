//! Fast catch-up after sleep (docs/design/12-launch-plan.md P1): a Mac that
//! slept for hours comes back thousands of blocks behind.
//!
//! - A follower more than `JUMP_BEHIND` behind jumps to the network's
//!   certified snapshot (checked against the certified block after it, as a
//!   checkpoint start is), keeps only certified facts of the gap, restarts on
//!   what it jumped to, and ends at the same state root as the network.
//! - A snapshot that does not check against the certified chain is refused and
//!   the follower falls back to replaying.
//! - A short gap replays normally, without a snapshot, and `aether_status`
//!   reports `catching_up`/`behind` while it does.
//! - A committee member that slept does not act as one until caught up: no
//!   beacon goes out while it is behind, while the other three keep
//!   finalizing. (Votes and proposals come from the consensus engine, which
//!   `aether node` starts only after `follow::catch_up` returns — the same
//!   rule, enforced structurally; beacons are the part this test can drive.)
//! - Replay is pipelined: over a 150 ms link it runs at hundreds of blocks a
//!   second where one-fetch-per-block ran at ~6.

use aether_crypto::P256Signer;
use aether_execution::registry::{encode_register, REGISTRY};
use aether_execution::{sign_call_with, EvmCall};
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
        node_rewards: false,
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
            .pre_state(&self.parent, self.parent.next_protocol(), &[], false)
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
            state: 0,
            prove: 100_000_000_000,
        };
        self.nonce += 1;
        sign_call_with(&signer, CHAIN, self.nonce - 1, fees, 1_000_000_000, &call).unwrap()
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

/// A server that only records what a member would have sent upstream, on a
/// free port. Returns its URL and the log.
fn recorder(rt: &tokio::runtime::Runtime) -> (String, Arc<Mutex<Vec<(Instant, String)>>>) {
    let sent: Arc<Mutex<Vec<(Instant, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/", post(recorded))
        .with_state(sent.clone());
    let listener = rt
        .block_on(tokio::net::TcpListener::bind(("127.0.0.1", 0)))
        .expect("bind");
    let port = listener.local_addr().unwrap().port();
    rt.spawn(async move { axum::serve(listener, app).await.expect("serve") });
    (format!("http://127.0.0.1:{port}"), sent)
}

/// Records what a member would have sent upstream, and answers plausibly.
async fn recorded(
    State(sent): State<Arc<Mutex<Vec<(Instant, String)>>>>,
    Json(req): Json<Value>,
) -> Json<Value> {
    if let Some(m) = req.get("method").and_then(Value::as_str) {
        sent.lock()
            .expect("sent")
            .push((Instant::now(), m.to_string()));
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

/// A Mac that slept ~85 minutes (5,100 blocks at 1 s each) jumps to the
/// network's certified snapshot instead of replaying: the gap's blocks are not
/// kept (only certified facts of the snapshot block), the state root matches
/// the network's, and a restart resumes from what it jumped to.
#[test]
fn a_follower_that_slept_jumps_to_a_certified_snapshot() {
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

/// A snapshot that does not check against the certified chain — a state the
/// network never finalized — is refused, and the follower replays instead.
#[test]
fn a_bad_snapshot_is_refused_and_replayed_instead() {
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
/// snapshot to rejoin. The other three validators keep finalizing meanwhile.
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
        aether_node::faucet::Faucet::from_seed(&dev_seed(11)).unwrap(),
        CHAIN,
    ));
    let st = rpc_state(&src, Some(registrar));
    let a = rt.block_on(call(
        &st,
        "aether_registerDevice",
        json!([
            "dev",
            operator,
            hex::encode(keys.validator_key()),
            hex::encode(keys.node_id()),
            keys.beaconer(),
            hex::encode(keys.ownership(CHAIN, operator)),
        ]),
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

    src.run_to(5_100, |h| h % 500 == 0);
    let (url, _calls) = serve(&st, &rt, Duration::ZERO);
    // Where the member would send beacons: a server that only records.
    let (record_url, sent) = recorder(&rt);

    // The member: behind by 5,100, with the beacon loop `aether follow
    // --candidate` runs and the catch-up `aether node` runs before voting.
    let member = Node::start(&dir_mem);
    rt.spawn(aether_node::candidate::beacon_loop(
        member.chain.clone(),
        aether_node::candidate::Outbox::Upstream(Arc::new(Upstream::Http(vec![record_url]))),
        aether_node::candidate::CandidateKeys::load_or_create(&dir_mem).unwrap(),
    ));
    let samples = Arc::new(Mutex::new(Vec::<(Instant, u64)>::new()));
    {
        let (samples, chain) = (samples.clone(), member.chain.clone());
        std::thread::spawn(move || loop {
            samples
                .lock()
                .unwrap()
                .push((Instant::now(), chain.behind()));
            std::thread::sleep(Duration::from_millis(25));
        });
    }
    // The other three keep finalizing while the member catches up.
    let src = Arc::new(Mutex::new(src));
    {
        let src = src.clone();
        std::thread::spawn(move || src.lock().unwrap().run_to(5_400, |_| false));
    }
    let adopted = rt
        .block_on(follow::catch_up(
            &member.chain,
            &Upstream::Http(vec![url.clone()]),
            &set(),
            follow::BEHIND_MARGIN,
        ))
        .unwrap();
    assert!(adopted > 5_000, "a jump did most of it");
    // The other three finished on their own; the member closes the last blocks.
    wait_until("the other three finished", || {
        src.lock().unwrap().chain.finalized_height() == 5_400
    });
    rt.block_on(follow::catch_up(
        &member.chain,
        &Upstream::Http(vec![url]),
        &set(),
        follow::BEHIND_MARGIN,
    ))
    .unwrap();
    assert_eq!(member.chain.finalized_height(), 5_400);

    // Not one beacon went out while behind (every recorded send has a
    // preceding `behind` sample within the margin), and it really was behind.
    let (samples, sent) = (
        samples.lock().unwrap().clone(),
        sent.lock().unwrap().clone(),
    );
    assert!(
        samples.iter().any(|(_, b)| *b > 5_000),
        "the member was thousands of blocks behind"
    );
    for (t, m) in sent
        .iter()
        .filter(|(_, m)| m == "aether_sendTransaction" || m == "aether_sendBeacon")
    {
        let behind = samples
            .iter()
            .rev()
            .find(|(s, _)| s <= t)
            .map(|(_, b)| *b)
            .unwrap_or(u64::MAX);
        assert!(
            behind <= follow::BEHIND_MARGIN,
            "a beacon ({m}) went out while {behind} blocks behind"
        );
    }
    assert_eq!(member.chain.behind(), 0, "caught up clears it");
    assert_eq!(
        member.chain.lock().finalized.state.root(),
        src.lock().unwrap().chain.lock().finalized.state.root()
    );
    // And now that it is current, the beacon goes out (registered, epochs behind).
    wait_until("the caught-up member's beacon", || {
        sent.iter().any(|(_, m)| m == "aether_sendTransaction")
    });

    drop((member, st));
    let _ = std::fs::remove_dir_all(&dir_src);
    let _ = std::fs::remove_dir_all(&dir_mem);
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
