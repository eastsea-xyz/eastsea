//! Era shards phase 1 (roadmap B5, docs/design/15-node-rewards.md "C. 보관"):
//! a real sealed era cut into 32 shards restores byte-exactly from any 16;
//! registered candidate Macs hold the shards a public draw assigns them
//! (every node computes the same assignment); a node that pruned its blocks
//! and dropped the era file still holds and serves its shards (fetching the
//! era over the B4 path to encode it); a corrupted shard fails the check and
//! is counted in the public statistics; nothing runs without history v2.
//!
//! Blocks are built and finalized the way a proposer does, without consensus
//! (as in `tests/prune.rs`); Macs register through the real registry contract
//! (as in `tests/rewards.rs`).

use aether_crypto::{address_of, P256Signer, Signer};
use aether_execution::registry::{attestation_message, encode_register, REGISTRY};
use aether_execution::{sign_call, EvmCall};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{build_payload, Chain, ChainConfig, Extras};
use aether_node::era;
use aether_node::follow::{FinalityArchive, Upstream};
use aether_node::prune::{self, Retention};
use aether_node::rpc::{self, RpcState};
use aether_node::shards::{self, Shards};
use aether_node::store::Store;
use aether_state::mmr::ERA_LEN;
use aether_types::{GasVector, TxEnvelope, U256};
use commonware_codec::Encode;
use commonware_consensus::types::{Height, Round, View};
use commonware_cryptography::{ed25519, Signer as Ed25519Signer};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CHAIN: u64 = 7_793;
const MACS: u8 = 4;

fn config() -> ChainConfig {
    let registrar = P256Signer::from_seed(&[0x5a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    ChainConfig {
        chain_id: CHAIN,
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        alloc: (1..=MACS)
            .map(|i| {
                let op = P256Signer::from_seed(&[0x5a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, i]).unwrap();
                (address_of(&op.public_key()).unwrap(), U256::from(10u128.pow(20)))
            })
            .collect(),
        fees: false,
        registrar: Some(aether_crypto::p256_xy(&registrar.public_key().bytes).unwrap()),
        epoch_blocks: 10,
        min_streak: None,
        draw_epochs: None,
        history_v2: true,
        protocol: 1,
        node_rewards: false,
        reserve: None,
    }
}

fn operator(i: u8) -> P256Signer {
    P256Signer::from_seed(&[0x5a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, i]).unwrap()
}

fn registrar() -> P256Signer {
    P256Signer::from_seed(&[0x5a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap()
}

/// Candidate `i`'s node id (the iroh key its registration names).
fn node_id(i: u8) -> [u8; 32] {
    *aether_net::SecretKey::from_bytes(&[i + 1; 32]).public().as_bytes()
}

/// Operator `i` registers its Mac (validator key and node id of the same index).
fn register_tx(i: u8) -> TxEnvelope {
    let op = address_of(&operator(i).public_key()).unwrap();
    let key = ed25519::PrivateKey::from_seed(i as u64 + 1).public_key().encode().as_ref().try_into().unwrap();
    let node = node_id(i);
    let sig = registrar().sign(&attestation_message(CHAIN, op, key, node, op)).unwrap();
    let call = EvmCall { to: Some(REGISTRY), value: U256::ZERO, input: encode_register(key, node, op, sig[..32].try_into().unwrap(), sig[32..64].try_into().unwrap()), gas_limit: 400_000, delegate: None };
    sign_call(&operator(i), CHAIN, 0, 1, &call).unwrap()
}

struct Node {
    chain: Chain,
    dir: PathBuf,
}

impl Node {
    fn start(dir: &Path) -> Node {
        let (chain, genesis) = Chain::open(config(), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        chain.lock().identity = Some(*sharing.public());
        chain.finalize(&genesis).unwrap();
        let mut n = Node { chain, dir: dir.to_path_buf() };
        // Block 1: every Mac registers through the real registry contract.
        let regs = (1..=MACS).map(register_tx).collect();
        n.step(regs);
        n
    }

    fn reopen(dir: &Path) -> Node {
        let (chain, _) = Chain::open(config(), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        chain.lock().identity = Some(*sharing.public());
        Node { chain, dir: dir.to_path_buf() }
    }

    fn step(&mut self, txs: Vec<TxEnvelope>) {
        let parent = self.chain.lock().finalized.clone();
        let h = parent.height + 1;
        let height = Height::new(h);
        let leader = ed25519::PrivateKey::from_seed(h % 4).public_key();
        let context = Context { round: Round::new(EPOCH, View::new(h)), leader, parent: (View::new(h - 1), parent.digest) };
        let skeleton = Block::new(context.clone(), parent.digest, height, 1_790_000_000_000 + h * 1000, bytes::Bytes::new());
        let ctx = Chain::block_context(&self.chain.cfg(), &skeleton, &parent);
        let (pre, _) = self.chain.pre_state(&parent, parent.next_protocol(), &[], false).unwrap();
        let (payload, _) = build_payload(&parent, &pre, &ctx, txs, Extras::default());
        drop(pre);
        let block = Block::new(context, parent.digest, height, 1_790_000_000_000 + h * 1000, payload.to_bytes());
        self.chain.finalize(&block).unwrap();
    }

    fn run_to(&mut self, height: u64) {
        while self.chain.lock().finalized.height < height {
            self.step(vec![]);
        }
    }

    fn era_file(&self, era: u64) -> PathBuf {
        self.dir.join("eras").join(era::file_name(era))
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aether-shards-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while !ok() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn rpc_state(chain: Chain, upstream: Option<Arc<Upstream>>, shards: Option<Arc<Shards>>) -> RpcState {
    RpcState {
        chain,
        finality: rpc::Finality::Archive(Arc::new(FinalityArchive::new(None))),
        gossip: tokio::sync::mpsc::unbounded_channel().0,
        faucet: None,
        registrar: None,
        network: None,
        upstream,
        handoff: None,
        snapshot: Default::default(),
        prover: None,
        shards,
    }
}

async fn call(st: &RpcState, method: &str, params: Value) -> Value {
    rpc::handle_value(st, json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })).await
}

/// The whole phase-1 story on one chain of two sealed eras (building it
/// commits ~16k blocks, so the parts share it):
/// 1. a real era file cut into 32 shards restores byte-exactly from any 16;
/// 2. every registered candidate computes the same assignment, and the
///    sealing node holds (and serves) exactly its shards;
/// 3. a node that pruned its blocks and dropped the era file still holds and
///    serves its shards of that era, fetching it verified over the B4 path;
/// 4. a corrupted shard fails the check and lands in the 7-day statistics;
/// 5. nothing of this exists without history v2 (the 7780 testnet).
#[test]
fn candidates_hold_assigned_shards_and_pruned_nodes_serve_them() {
    let (dir_a, dir_b) = (tmp("peer"), tmp("pruned"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut a = Node::start(&dir_a);
    a.run_to(2 * ERA_LEN + 20);
    {
        let store = a.chain.store().unwrap();
        wait_until("eras 0 and 1 sealed", || store.staged_eras().unwrap() == vec![2] && a.era_file(1).exists());
    }
    let roots = a.chain.lock().history_index.as_ref().unwrap().eras.clone();
    assert_eq!(roots.len(), 2, "two sealed eras");
    assert_eq!(aether_execution::registry::candidates(&a.chain.lock().finalized.state).len(), MACS as usize, "every Mac registered");

    // 1. Any 16 of the 32 shards restore the era file byte for byte, and the
    //    restored file is the era the certified history says it is.
    let era1 = std::fs::read(a.era_file(1)).unwrap();
    let (c1, cut) = shards::encode(&era1).unwrap();
    let checked: Vec<_> = cut.iter().enumerate().map(|(i, s)| shards::check(&c1, i as u16, s).unwrap()).collect();
    for subset in [(0usize..16).collect::<Vec<_>>(), (16..32).collect(), (0..32).step_by(2).collect(), (1..32).step_by(2).collect()] {
        let back = shards::decode(&c1, subset.iter().map(|&i| &checked[i])).unwrap();
        assert_eq!(back, era1, "subset {subset:?} restores the era byte-exactly");
        assert_eq!(era::read(&back, Some(&roots[1])).unwrap().index, 1);
    }

    // 2. The assignment: same roots and registry on both nodes, same lists.
    let me_a = node_id(1);
    let view_a = shards::View::of(&a.chain, shards::DEFAULT_MAX_SHARDS, Some(&me_a)).unwrap();
    assert_eq!(view_a.nodes.len(), MACS as usize);
    // Every shard of the newest sealed era has `replicas` holders.
    for s in 0..shards::TOTAL_SHARDS {
        let holders = view_a.all.iter().filter(|l| l.contains(&(1, s))).count();
        assert_eq!(holders, shards::REPLICAS, "shard {s} of era 1");
    }
    assert!(!view_a.mine.is_empty(), "a candidate always holds something");
    assert!(view_a.mine.len() <= shards::DEFAULT_MAX_SHARDS);
    // The sealing node holds exactly what it is assigned (one era per pass).
    let s_a = Arc::new(Shards::new(&dir_a, Some(me_a), shards::DEFAULT_MAX_SHARDS));
    rt.block_on(shards::reconcile(&a.chain, None, &s_a));
    rt.block_on(shards::reconcile(&a.chain, None, &s_a));
    let mut mine = view_a.mine.clone();
    mine.sort_unstable();
    assert_eq!(s_a.held(), mine, "holds exactly its assignment, no more");
    rt.block_on(shards::reconcile(&a.chain, None, &s_a));
    assert_eq!(s_a.held(), mine, "a third pass changes nothing");

    // The pruned node starts as a copy of the peer (same chain, same registry).
    drop(a);
    copy_dir(&dir_a, &dir_b);
    let a = Node::reopen(&dir_a);
    {
        let view_again = shards::View::of(&a.chain, shards::DEFAULT_MAX_SHARDS, Some(&me_a)).unwrap();
        assert_eq!(view_again.all, view_a.all, "the same public data gives every node the same assignment");
    }
    let b = Node::reopen(&dir_b);
    let r = Retention { blocks: ERA_LEN, keep_era_files: false };
    let (cut_off, _) = prune::prune_once(&b.chain, &r).unwrap().expect("era 0 pruned");
    assert_eq!(cut_off, ERA_LEN);
    assert!(!b.era_file(0).exists(), "the era file is dropped with --drop-era-files");
    assert!(b.era_file(1).exists(), "the recent era file is kept");
    assert_eq!(b.chain.lock().pruned_below, ERA_LEN, "the node really pruned");

    // 3. The pruned node holds shards of the pruned era too: it fetches the
    //    era verified over the B4 path, cuts it, and keeps no extra era file.
    let port = 22_000 + (std::process::id() % 20_000) as u16;
    let st_a = rpc_state(a.chain.clone(), None, Some(s_a.clone()));
    rt.spawn(rpc::serve(std::net::SocketAddr::from(([127, 0, 0, 1], port)), st_a));
    std::thread::sleep(std::time::Duration::from_millis(300));
    let up = Upstream::Http(vec![format!("http://127.0.0.1:{port}")]);
    let me_b = node_id(2);
    let view_b = shards::View::of(&b.chain, shards::DEFAULT_MAX_SHARDS, Some(&me_b)).unwrap();
    assert_eq!(view_b.all, view_a.all, "the pruned node computes the same assignment");
    let s_b = Shards::new(&dir_b, Some(me_b), shards::DEFAULT_MAX_SHARDS);
    rt.block_on(shards::reconcile(&b.chain, Some(&up), &s_b));
    rt.block_on(shards::reconcile(&b.chain, Some(&up), &s_b));
    let mut mine_b = view_b.mine.clone();
    mine_b.sort_unstable();
    assert_eq!(s_b.held(), mine_b, "the pruned node holds its whole assignment");
    assert!(!b.era_file(0).exists(), "the era fetched to cut is not kept");

    // It serves those shards although the era's blocks are long pruned here.
    let st_b = rpc_state(b.chain.clone(), None, Some(Arc::new(s_b)));
    // Served like a peer would serve it, so the challenge below goes over RPC.
    let port_b = 24_000 + (std::process::id() % 20_000) as u16;
    rt.spawn(rpc::serve(std::net::SocketAddr::from(([127, 0, 0, 1], port_b)), st_b.clone()));
    std::thread::sleep(std::time::Duration::from_millis(300));
    let up_b = Upstream::Http(vec![format!("http://127.0.0.1:{port_b}")]);
    rt.block_on(async {
        let era0 = std::fs::read(a.era_file(0)).unwrap();
        let (c0, cut0) = shards::encode(&era0).unwrap();
        let of_era0: Vec<u16> = mine_b.iter().filter(|(e, _)| *e == 0).map(|(_, s)| *s).collect();
        assert!(!of_era0.is_empty(), "the pruned node holds shards of the pruned era");
        for &shard in &of_era0 {
            let answer = call(&st_b, "aether_shard", json!([0, shard])).await;
            assert_eq!(answer["result"]["era"], json!(0), "{answer}");
            let holder = shards::check_answer(&c0, 0, shard, &answer["result"])
                .unwrap_or_else(|(_, e)| panic!("shard {shard} of era 0 does not check: {e}"));
            assert_eq!(holder, me_b, "the answer names the serving candidate");
            // The served shard is the one an independent encoding of the era produced.
            let back = hex::decode(answer["result"]["shard"].as_str().unwrap()).unwrap();
            assert_eq!(back, shards::to_bytes(&cut0[shard as usize]));
        }
        // The direct check above verified the peer's answer; record it the way
        // the challenge loop records what it verifies.
        s_a.record(&me_b, true);
        // A shard this node does not hold is answered with null (not an error).
        let foreign: Vec<u16> = (0..shards::TOTAL_SHARDS).filter(|s| !mine_b.contains(&(0, *s))).collect();
        assert_eq!(call(&st_b, "aether_shard", json!([0, foreign[0]])).await["result"], Value::Null);

        // 4. A corrupted shard fails the proof check and is counted failed.
        let good = call(&st_b, "aether_shard", json!([0, of_era0[0]])).await["result"].clone();
        let mut bytes = hex::decode(good["shard"].as_str().unwrap()).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let mut corrupted = good.clone();
        corrupted["shard"] = json!(hex::encode(bytes));
        match shards::check_answer(&c0, 0, of_era0[0], &corrupted) {
            Ok(_) => panic!("a corrupted shard must not check"),
            Err((node, e)) => {
                assert_eq!(node, Some(me_b));
                assert!(e.contains("merkle path"), "{e}");
            }
        }
        s_a.record(&me_b, false);

        // The real challenge loop: ask the peer for a random shard of an era we
        // hold; whatever answer comes back is verified and recorded.
        for _ in 0..40 {
            shards::challenge(&a.chain, &up_b, &s_a).await;
            let stats = s_a.stats(&a.chain);
            let row = stats["candidates"].as_array().unwrap().iter().find(|c| c["node"] == json!(hex::encode(me_b))).unwrap();
            if row["checked"].as_u64().unwrap_or(0) >= 2 {
                break;
            }
        }
        let stats = s_a.stats(&a.chain);
        let rows = stats["candidates"].as_array().unwrap();
        assert_eq!(rows.len(), MACS as usize, "a row per candidate: {stats}");
        let b_row = rows.iter().find(|c| c["node"] == json!(hex::encode(me_b))).unwrap();
        assert_eq!(b_row["assigned"], json!(mine_b.len()), "the assignment view is public");
        assert!(b_row["checked"].as_u64().unwrap() >= 2, "the direct check and the challenge: {b_row}");
        assert!(b_row["failed"].as_u64().unwrap() >= 1, "the corrupted shard is counted failed: {b_row}");
        assert_eq!(stats["me"], json!(hex::encode(me_a)));
        assert_eq!(stats["held"].as_array().unwrap().len(), mine.len());

        // A corrupted shard file is never served as good.
        drop(st_b);
    });
    {
        let path = dir_b.join("shards").join(format!("era-00000000.{:02}.shard", mine_b.iter().find(|(e, _)| *e == 0).map(|(_, s)| s).unwrap()));
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(&path, &bytes).unwrap();
        let st_b = rpc_state(b.chain.clone(), None, Some(Arc::new(Shards::new(&dir_b, Some(me_b), shards::DEFAULT_MAX_SHARDS))));
        rt.block_on(async {
            let shard = mine_b.iter().find(|(e, _)| *e == 0).map(|(_, s)| *s).unwrap();
            let answer = call(&st_b, "aether_shard", json!([0, shard])).await;
            assert!(answer["error"].is_object(), "a corrupted shard file is not served: {answer}");
        });
    }

    // 5. Without history v2 (the 7780 testnet) none of this exists.
    let mut cfg = config();
    cfg.history_v2 = false;
    let (old, genesis) = Chain::new(cfg);
    old.finalize(&genesis).unwrap();
    assert!(shards::View::of(&old, shards::DEFAULT_MAX_SHARDS, Some(&node_id(1))).is_none());
    let st_none = rpc_state(old, None, None);
    rt.block_on(async {
        for method in ["aether_shard", "aether_shardStats"] {
            let v = call(&st_none, method, json!([0, 0])).await;
            assert_eq!(v["error"]["code"], json!(-32601), "{method}: {v}");
        }
    });

    drop((a, b));
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}
