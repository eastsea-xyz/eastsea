//! History pruning (roadmap B4): a pruning node drops only sealed eras older
//! than its retention window, restarts on what is left, serves recent blocks,
//! and gets old eras back from a peer, verified against a history root it
//! trusts; corrupted or foreign eras are refused. Marshal's prunable archive
//! drops whole eras and survives a restart.
//!
//! Blocks are built and finalized the way a proposer and marshal do, without
//! consensus (as in `tests/history.rs`).

use aether_crypto::P256Signer;
use aether_execution::{sign_call_with, EvmCall};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{build_payload, dev_accounts, dev_seed, Chain, ChainConfig, Executed, Extras};
use aether_node::era;
use aether_node::era_net::{self, HistoryAnchor};
use aether_node::follow::{FinalityArchive, Upstream};
use aether_node::prune::{self, Retention};
use aether_node::rpc::{self, RpcState};
use aether_node::store::Store;
use aether_state::mmr::ERA_LEN;
use aether_types::{Address, Bytes, FeeVector, GasVector, TxEnvelope, B256, U256};
use commonware_codec::Encode;
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible, Signer};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CHAIN: u64 = 7_792;

fn config() -> ChainConfig {
    ChainConfig {
        chain_id: CHAIN,
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        alloc: dev_accounts(4).into_iter().map(|(_, a)| (a, U256::from(10u128.pow(24)))).collect(),
        fees: true,
        registrar: Some(([1; 32], [2; 32])),
        epoch_blocks: 10,
        min_streak: None,
        draw_epochs: None,
        history_v2: true,
        node_rewards: false,
    }
}

struct Node {
    chain: Chain,
    blocks: Vec<Block>,
    parent: Arc<Executed>,
    nonce: u64,
}

impl Node {
    fn start(dir: &Path) -> Node {
        let (chain, genesis) = Chain::open(config(), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        chain.lock().identity = Some(*sharing.public());
        chain.finalize(&genesis).unwrap();
        let parent = chain.lock().finalized.clone();
        Node { chain, blocks: vec![genesis], parent, nonce: 0 }
    }

    /// The same node after a restart (blocks and nonce carried over by the test).
    fn reopen(dir: &Path, blocks: Vec<Block>, nonce: u64) -> Node {
        let (chain, _) = Chain::open(config(), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        chain.lock().identity = Some(*sharing.public());
        let parent = chain.lock().finalized.clone();
        Node { chain, blocks, parent, nonce }
    }

    fn step(&mut self, txs: Vec<TxEnvelope>) -> Arc<Executed> {
        let prev = self.blocks.last().unwrap();
        let height = prev.height.next();
        let h = height.get();
        let leader = ed25519::PrivateKey::from_seed(h % 4).public_key();
        let context = Context { round: Round::new(EPOCH, View::new(h)), leader, parent: (View::new(h - 1), prev.digest()) };
        let ts = 1_790_000_000_000 + h * 1000;
        let skeleton = Block::new(context.clone(), prev.digest(), height, ts, bytes::Bytes::new());
        let ctx = Chain::block_context(&self.chain.cfg(), &skeleton, &self.parent);
        let (pre, _) = self.chain.pre_state(&self.parent, self.parent.next_protocol(), &[], false).unwrap();
        let (payload, _) = build_payload(&self.parent, &pre, &ctx, txs, Extras::default());
        drop(pre);
        let block = Block::new(context, prev.digest(), height, ts, payload.to_bytes());
        let exec = self.chain.execute(&block, &self.parent).unwrap();
        self.chain.finalize(&block).unwrap();
        self.blocks.push(block);
        self.parent = exec.clone();
        exec
    }

    fn transfer(&mut self) -> TxEnvelope {
        let signer = P256Signer::from_seed(&dev_seed(1)).unwrap();
        let call = EvmCall { to: Some(Address::repeat_byte(0xb0)), value: U256::from(1_000u64), input: Bytes::new(), gas_limit: 21_000, delegate: None };
        let fees = FeeVector { exec: 100_000_000_000, state: 0, prove: 100_000_000_000 };
        self.nonce += 1;
        sign_call_with(&signer, CHAIN, self.nonce - 1, fees, 1_000_000_000, &call).unwrap()
    }

    /// Run to `height`, with a transfer in every block `with_tx` names.
    fn run_to(&mut self, height: u64, with_tx: impl Fn(u64) -> bool) -> Vec<B256> {
        let mut txs = Vec::new();
        while self.parent.height < height {
            let t = if with_tx(self.parent.height + 1) { vec![self.transfer()] } else { vec![] };
            let exec = self.step(t);
            txs.extend(exec.tx_hashes.iter().copied());
        }
        txs
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aether-prune-{name}-{}", std::process::id()));
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

fn rpc_state(chain: Chain, upstream: Option<Arc<Upstream>>) -> RpcState {
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
    }
}

async fn call(st: &RpcState, method: &str, params: Value) -> Value {
    rpc::handle_value(st, json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })).await
}

fn du(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { du(&e.path()) } else { e.metadata().map(|m| m.len()).unwrap_or(0) })
                .sum()
        })
        .unwrap_or(0)
}

/// Check block `height`'s bytes against `anchor` blocks of history, with the proof `chain` serves.
fn assert_history_proof(chain: &Chain, blocks: &[Block], height: u64, anchor: u64) {
    let (proof, hash) = chain.history_proof(height, anchor).unwrap_or_else(|e| panic!("proof of {height}: {e}"));
    assert_eq!(hash, hex::encode(blocks[height as usize].digest()));
    let v = aether_light::VerifiedBlock {
        height: anchor,
        digest: String::new(),
        timestamp_ms: 0,
        parent_state_root: B256::ZERO,
        history_root: blocks[anchor as usize].payload().unwrap().history_root,
    };
    let (old, _) = aether_light::verify_old_block(&v, &blocks[height as usize].encode(), &proof).unwrap();
    assert_eq!(old.height, height);
}

/// The whole B4 story on one chain of two sealed eras and a bit (building it
/// commits ~16k blocks, so the phases share it):
/// 1. a pruning node drops only the sealed era older than its window;
/// 2. it answers RPC for recent heights as before and clearly for pruned ones;
/// 3. it restarts on what is left, with the same history index;
/// 4. it fetches the pruned era back from a peer and verifies it;
/// 5. a corrupted or foreign era is refused.
#[test]
fn pruning_keeps_recent_history_and_old_eras_come_back_verified() {
    let (dir_a, dir_b) = (tmp("peer"), tmp("pruned"));
    let mut a = Node::start(&dir_a);
    let txs = a.run_to(2 * ERA_LEN + 20, |h| h % 1000 == 0);
    let (store_a, era_file) = (a.chain.store().unwrap(), |d: &Path, e: u64| d.join("eras").join(era::file_name(e)));
    wait_until("eras 0 and 1 sealed", || store_a.staged_eras().unwrap() == vec![2] && era_file(&dir_a, 1).exists());
    let (blocks, nonce, index) = (a.blocks.clone(), a.nonce, a.chain.lock().history_index.clone().unwrap());
    let head = a.parent.height;
    drop(store_a);
    drop(a);
    // The pruning node starts as a copy of the peer (the same chain, all eras).
    copy_dir(&dir_a, &dir_b);
    let before = du(&dir_b);

    // 1. Retention of one era: era 0 is old and sealed, era 1 is sealed but recent, era 2 is open.
    let b = Node::reopen(&dir_b, blocks.clone(), nonce);
    let r = Retention { blocks: ERA_LEN, keep_era_files: false };
    let (cut, report) = prune::prune_once(&b.chain, &r).unwrap().expect("era 0 pruned");
    assert_eq!(cut, ERA_LEN);
    assert_eq!(report.summaries, ERA_LEN);
    let old_txs = txs.iter().filter(|t| !b.chain.lock().receipts.contains_key(*t)).count();
    assert_eq!(report.receipts as usize, old_txs);
    assert_eq!(old_txs, (ERA_LEN / 1000) as usize, "the transfers of era 0 only");
    assert!(prune::prune_once(&b.chain, &r).unwrap().is_none(), "nothing more is old enough");
    {
        let g = b.chain.lock();
        assert_eq!(g.pruned_below, ERA_LEN);
        assert_eq!(*g.blocks.keys().next().unwrap(), ERA_LEN);
        assert!(g.receipts.values().all(|(h, _)| *h >= ERA_LEN));
        assert_eq!(g.history_index.as_deref(), Some(&*index), "the history index is unchanged");
    }
    assert!(!era_file(&dir_b, 0).exists(), "--drop-era-files: only the root of era 0 is kept");
    assert!(era_file(&dir_b, 1).exists() && !era_file(&dir_b, 2).exists(), "era 1 kept, era 2 still open");
    // History proofs of kept heights still work; of the dropped era, a clear error.
    assert_history_proof(&b.chain, &blocks, ERA_LEN + 3, head);
    assert_history_proof(&b.chain, &blocks, 2 * ERA_LEN + 1, head);
    let e = b.chain.history_proof(5, head).unwrap_err();
    assert!(e.contains("pruned"), "{e}");

    // 2. RPC on the pruned node.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let st_b = rpc_state(b.chain.clone(), None);
    rt.block_on(async {
        let recent = call(&st_b, "aether_getBlock", json!([ERA_LEN + 1])).await;
        assert_eq!(recent["result"]["hash"], json!(format!("{}", blocks[(ERA_LEN + 1) as usize].digest())));
        let pruned = call(&st_b, "aether_getBlock", json!([5])).await;
        assert!(pruned["error"]["message"].as_str().unwrap().contains("pruned"), "{pruned}");
        let fin = call(&st_b, "aether_getFinalized", json!([5])).await;
        assert_eq!(fin["error"]["code"], json!(-32001), "{fin}");
        let hist = call(&st_b, "aether_history", json!([])).await;
        assert_eq!(hist["result"]["pruned_below"], json!(ERA_LEN));
        let receipt = call(&st_b, "aether_getReceipt", json!([txs[0]])).await;
        assert_eq!(receipt["result"], Value::Null, "receipts of pruned eras are gone");
        let receipt = call(&st_b, "aether_getReceipt", json!([txs[txs.len() - 1]])).await;
        assert!(receipt["result"]["height"].as_u64().unwrap() >= ERA_LEN);
    });

    // 3. Restart after pruning: same head, same history index, and it keeps going.
    drop(st_b);
    drop(b);
    let mut b = Node::reopen(&dir_b, blocks.clone(), nonce);
    {
        let g = b.chain.lock();
        assert_eq!(g.finalized.height, head);
        assert_eq!(g.pruned_below, ERA_LEN);
        assert_eq!(g.history_index.as_deref(), Some(&*index), "rebuilt from the kept era roots");
        assert_eq!(*g.blocks.keys().next().unwrap(), ERA_LEN, "the first kept summary loads on its own");
    }
    let after = du(&dir_b);
    println!("pruning node: {before} B before, {after} B after (one of three eras dropped)");
    // Measurements for the roadmap (B4): what a kept block costs, what a pruned one costs.
    let stats = b.chain.store().unwrap().stats().unwrap();
    let kept = |name: &str| stats.tables.iter().find(|t| t.name == name).map(|t| t.stored).unwrap_or(0);
    let n_kept = (head + 1 - ERA_LEN) as f64;
    let empty: Vec<usize> = blocks[(ERA_LEN + 1) as usize..].iter().filter(|b| b.payload().unwrap().txs.is_empty()).map(|b| b.encode().len()).collect();
    let era_bytes = std::fs::metadata(era_file(&dir_b, 1)).unwrap().len();
    println!(
        "kept per block: summary {:.0} B, receipts {:.1} B, marshal block (codec, empty) {:.0} B; pruned per block: era file {:.2} B + root {:.3} B",
        kept("blocks") as f64 / n_kept,
        kept("receipts") as f64 / n_kept,
        empty.iter().sum::<usize>() as f64 / empty.len() as f64,
        era_bytes as f64 / ERA_LEN as f64,
        32.0 / ERA_LEN as f64,
    );

    // 4. The peer (archive) serves era 0; the pruned node fetches it and checks it
    //    against its own finalized history root before keeping it.
    let a = Node::reopen(&dir_a, blocks.clone(), nonce);
    let port = 21_000 + (std::process::id() % 20_000) as u16;
    let url = format!("http://127.0.0.1:{port}");
    let st_a = rpc_state(a.chain.clone(), None);
    rt.spawn(rpc::serve(std::net::SocketAddr::from(([127, 0, 0, 1], port)), st_a));
    std::thread::sleep(std::time::Duration::from_millis(300));
    let up = || Upstream::Http(vec![url.clone()]);
    let path = rt.block_on(era_net::fetch_into(&b.chain, &up(), 0)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(era_file(&dir_a, 0)).unwrap());
    assert_eq!(b.chain.old_block(5).unwrap(), blocks[5]);
    assert_history_proof(&b.chain, &blocks, 5, head);
    // A follower answers aether_getBlock for a pruned height by fetching the era itself.
    std::fs::remove_file(&path).unwrap();
    b.chain.old_block(ERA_LEN + 1).unwrap(); // another era in the read cache
    let st_b = rpc_state(b.chain.clone(), Some(Arc::new(up())));
    let old = rt.block_on(call(&st_b, "aether_getBlock", json!([7])));
    assert_eq!(old["result"]["hash"], json!(format!("{}", blocks[7].digest())), "{old}");
    assert_eq!(old["result"]["pruned"], json!(true));
    assert_eq!(old["result"]["state_root"], json!(blocks[8].payload().unwrap().parent_state_root));

    // 5. Corruption: the peer's file is damaged; the fetch fails and nothing is kept.
    std::fs::remove_file(era_file(&dir_b, 0)).unwrap();
    let mut bad = std::fs::read(era_file(&dir_a, 0)).unwrap();
    let mid = bad.len() / 2;
    bad[mid] ^= 0x40;
    std::fs::write(era_file(&dir_a, 0), &bad).unwrap();
    let err = rt.block_on(era_net::fetch_into(&b.chain, &up(), 0)).unwrap_err();
    assert!(err.contains("era 0"), "{err}");
    assert!(!era_file(&dir_b, 0).exists(), "a corrupted era is never kept");
    // Well-formed eras that are not the one asked for, or not in the trusted history, are refused.
    let era1 = std::fs::read(era_file(&dir_b, 1)).unwrap();
    let anchor = HistoryAnchor::of_chain(&b.chain);
    let proof0 = b.chain.era_proof(0, anchor.leaves).unwrap();
    let proof1 = b.chain.era_proof(1, anchor.leaves).unwrap();
    era_net::verify(&era1, 1, &anchor, &proof1, None).unwrap();
    assert!(era_net::verify(&era1, 0, &anchor, &proof0, None).is_err(), "era 1 offered as era 0");
    let other = HistoryAnchor { leaves: anchor.leaves, root: B256::repeat_byte(7) };
    assert!(era_net::verify(&era1, 1, &other, &proof1, None).is_err(), "not in the trusted history");
    assert!(era_net::verify(&era1, 1, &anchor, &proof1, Some(&index.eras[0])).is_err(), "not the root this node keeps");

    // B5 (library only): a real era file cut into 32 shards comes back from the 16 parity ones.
    let (c, shards) = aether_node::shards::encode(&era1).unwrap();
    let parity: Vec<_> = (16..32u16).map(|i| aether_node::shards::check(&c, i, &shards[i as usize]).unwrap()).collect();
    let back = aether_node::shards::decode(&c, parity.iter()).unwrap();
    assert_eq!(era::read(&back, Some(&index.eras[1])).unwrap().blocks.len() as u64, ERA_LEN);

    // The restarted pruning node still extends the chain.
    b.run_to(head + 5, |_| false);
    assert_eq!(b.chain.finalized_height(), head + 5);
    drop((a, b, st_b));
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

/// Marshal's prunable archive (the pruning layout) drops whole eras and
/// reopens with only the kept ones, in a deterministic runtime.
#[test]
fn prunable_block_archive_drops_whole_eras_and_survives_restart() {
    use aether_node::archive::{prunable_config, Buffers, FinalizedBlocks};
    use commonware_consensus::marshal::store::Blocks;
    use commonware_consensus::types::Height;
    use commonware_runtime::buffer::paged::{page_size, CacheRef};
    use commonware_runtime::{deterministic, Runner as _, Supervisor as _};
    use commonware_storage::archive::{prunable, Identifier};
    use commonware_utils::NZUsize;

    let buffers = Buffers { write: NZUsize!(1 << 16), replay: NZUsize!(1 << 16) };
    let block = move |h: u64| {
        use commonware_cryptography::{Hasher as _, Sha256};
        let leader = ed25519::PrivateKey::from_seed(h % 4).public_key();
        let parent = Sha256::hash(&[h.to_be_bytes().as_slice()]);
        Block::new(Context { round: Round::new(EPOCH, View::new(h)), leader, parent: (View::new(h), parent) }, parent, Height::new(h), h, bytes::Bytes::new())
    };
    let total = 3 * ERA_LEN;
    let open = move |context: deterministic::Context| async move {
        let cache = CacheRef::from_pooler(&context, page_size(4096), NZUsize!(64));
        FinalizedBlocks::Prunable(
            prunable::Archive::init(
                context.child("blocks"),
                prunable_config("t", "blocks", cache, Block::codec_config(aether_light::MAX_BLOCK_BYTES), buffers),
            )
            .await
            .unwrap(),
        )
    };
    let (_, checkpoint) = deterministic::Runner::default().start_and_recover(|context| async move {
        let mut a = open(context).await;
        for h in 0..total {
            a = Blocks::put(a, block(h)).await.unwrap();
        }
        a = Blocks::sync(a).await.unwrap();
        // Pruning inside era 2 keeps all of era 2: sections are whole eras.
        a = Blocks::prune(a, Height::new(2 * ERA_LEN + 100)).await.unwrap();
        assert!(Blocks::get(&a, Identifier::Index(2 * ERA_LEN - 1)).await.unwrap().is_none());
        assert_eq!(Blocks::get(&a, Identifier::Index(2 * ERA_LEN)).await.unwrap(), Some(block(2 * ERA_LEN)));
        let d = block(ERA_LEN).digest();
        assert!(Blocks::get(&a, Identifier::Key(&d)).await.unwrap().is_none(), "by digest too");
        // Puts below the floor are dropped (marshal re-delivering an old block).
        a = Blocks::put(a, block(5)).await.unwrap();
        assert!(Blocks::get(&a, Identifier::Index(5)).await.unwrap().is_none());
        Blocks::sync(a).await.unwrap();
    });
    deterministic::Runner::from(checkpoint).start(|context| async move {
        let a = open(context).await;
        assert!(Blocks::get(&a, Identifier::Index(0)).await.unwrap().is_none());
        assert!(Blocks::get(&a, Identifier::Index(ERA_LEN)).await.unwrap().is_none());
        assert_eq!(Blocks::get(&a, Identifier::Index(total - 1)).await.unwrap(), Some(block(total - 1)));
        assert_eq!(Blocks::last_index(&a), Some(Height::new(total - 1)));
        let d = block(2 * ERA_LEN + 7).digest();
        assert_eq!(Blocks::get(&a, Identifier::Key(&d)).await.unwrap(), Some(block(2 * ERA_LEN + 7)));
    });
}
