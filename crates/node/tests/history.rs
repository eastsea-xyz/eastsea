//! History storage (roadmap B1-B3): what each block costs on disk, history v2
//! (quiet empty blocks), history proofs across eras, and era files sealed by
//! the node. Blocks are built and finalized the way a proposer and marshal do,
//! without consensus, so thousands of blocks run in seconds.
//!
//! `cargo test -p aether-node --release --test history -- --ignored --nocapture measure`
//! prints the bytes-per-block table of the roadmap (B1/B3).

use aether_crypto::P256Signer;
use aether_execution::{sign_call_with, EvmCall};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{
    build_payload, dev_accounts, dev_seed, BlockSummary, Chain, ChainConfig, Executed, Extras,
};
use aether_node::era;
use aether_node::store::Store;
use aether_node::upgrade::{combine, sign_partial, Release, SignedUpgrade, Upgrade};
use aether_state::mmr::ERA_LEN;
use aether_types::{Address, Bytes, FeeVector, GasVector, TxEnvelope, B256, U256};
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible, Signer};
use std::path::PathBuf;
use std::sync::Arc;

const CHAIN: u64 = 7_791;
/// Protocol 2 (the proof market, as on 7780) from this height.
const P2_AT: u64 = 20;

fn config(history_v2: bool) -> ChainConfig {
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
        registrar: Some(([1; 32], [2; 32])),
        epoch_blocks: 10,
        min_streak: None,
        draw_epochs: None,
        history_v2,
        node_rewards: false,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
        reserve: None,
    }
}

fn upgrade_to_2() -> SignedUpgrade {
    let (_, sharing, shares) = aether_light::devnet_threshold(4);
    let u = Upgrade {
        chain_id: CHAIN,
        protocol: 2,
        activate_at: P2_AT,
        releases: vec![Release {
            platform: "macos-arm64-dmg".into(),
            version: "0.6.0".into(),
            blake3: "ab".repeat(32),
            url: "https://x".into(),
        }],
        notes: String::new(),
        registrar: None,
    };
    let partials: Vec<_> = shares
        .iter()
        .take(3)
        .map(|(_, s)| sign_partial(&u, s))
        .collect();
    combine(&sharing, &partials).unwrap()
}

/// A validator's chain, with a store in `dir` when given.
struct Node {
    chain: Chain,
    blocks: Vec<Block>,
    parent: Arc<Executed>,
    nonce: u64,
}

impl Node {
    fn start(history_v2: bool, dir: Option<&PathBuf>) -> Node {
        let cfg = config(history_v2);
        let (chain, genesis) = match dir {
            Some(d) => Chain::open(cfg, Store::open(&d.join("state.redb")).unwrap()).unwrap(),
            None => Chain::new(cfg),
        };
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        chain.lock().identity = Some(*sharing.public());
        chain.finalize(&genesis).unwrap();
        let parent = chain.lock().finalized.clone();
        Node {
            chain,
            blocks: vec![genesis],
            parent,
            nonce: 0,
        }
    }

    /// Propose on the head and finalize, as a leader of a 4-validator set does.
    fn step(&mut self, txs: Vec<TxEnvelope>) -> Arc<Executed> {
        let prev = self.blocks.last().unwrap();
        let height = prev.height.next();
        let h = height.get();
        let leader = ed25519::PrivateKey::from_seed(h % 4).public_key();
        let context = Context {
            round: Round::new(EPOCH, View::new(h)),
            leader,
            parent: (View::new(h - 1), prev.digest()),
        };
        // 1 s blocks with tens of ms of jitter, as measured on the testnet.
        let ts = 1_790_000_000_000
            + h * 1000
            + u64::from(blake3::hash(&h.to_le_bytes()).as_bytes()[0] % 64);
        let skeleton = Block::new(
            context.clone(),
            prev.digest(),
            height,
            ts,
            bytes::Bytes::new(),
        );
        let ctx = Chain::block_context(&self.chain.cfg(), &skeleton, &self.parent);
        let upgrade = (h == 1).then(upgrade_to_2);
        let (pre, _) = self
            .chain
            .pre_state(&self.parent, self.parent.next_protocol(), &[], false)
            .unwrap();
        let (payload, _) = build_payload(
            &self.parent,
            &pre,
            &ctx,
            txs,
            Extras {
                upgrade,
                ..Default::default()
            },
        );
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
        let call = EvmCall {
            to: Some(Address::repeat_byte(0xb0)),
            value: U256::from(1_000u64),
            input: Bytes::new(),
            gas_limit: 21_000,
            delegate: None,
        };
        let fees = FeeVector {
            exec: 100_000_000_000,
            state: 0,
            prove: 100_000_000_000,
        };
        self.nonce += 1;
        sign_call_with(&signer, CHAIN, self.nonce - 1, fees, 1_000_000_000, &call).unwrap()
    }

    fn run_to(&mut self, height: u64, with_tx: impl Fn(u64) -> bool) {
        while self.parent.height < height {
            let txs = if with_tx(self.parent.height + 1) {
                vec![self.transfer()]
            } else {
                vec![]
            };
            let exec = self.step(txs.clone());
            assert_eq!(exec.tx_hashes.len(), txs.len(), "the transfer is included");
        }
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aether-history-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// 7780 rules record every protocol-2 block's statement in the state tree, so
/// even empty blocks change the state root; history v2 leaves empty blocks'
/// state and metadata untouched and still records blocks with transactions.
#[test]
fn history_v2_empty_blocks_leave_state_and_metadata_unchanged() {
    let mut old = Node::start(false, None);
    let mut v2 = Node::start(true, None);
    assert_ne!(
        old.blocks[0].digest(),
        v2.blocks[0].digest(),
        "history v2 is bound to the genesis hash"
    );
    for n in [&mut old, &mut v2] {
        n.run_to(P2_AT + 30, |_| false);
    }
    let roots = |n: &Node, from: usize| {
        n.blocks[from..]
            .iter()
            .map(|b| b.payload().unwrap())
            .map(|p| (p.parent_state_root, p.parent_meta))
            .collect::<Vec<_>>()
    };
    let old_roots = roots(&old, (P2_AT + 5) as usize);
    assert!(
        old_roots.windows(2).all(|w| w[0].0 != w[1].0),
        "7780 rules: every empty block changes the state root"
    );
    let v2_roots = roots(&v2, (P2_AT + 5) as usize);
    assert!(
        v2_roots.windows(2).all(|w| w[0] == w[1]),
        "history v2: nothing changes across empty blocks"
    );

    // A block with a transaction is still recorded for the proof market, and expires as before.
    let tx = v2.transfer();
    let with_tx = v2.step(vec![tx]).height;
    v2.step(vec![]);
    assert!(aether_execution::proofs::commitment(&v2.parent.state, with_tx).is_some());
    assert!(
        aether_execution::proofs::commitment(&v2.parent.state, with_tx - 1).is_none(),
        "the empty block before it is not"
    );
    let tx = old.transfer();
    let h = old.step(vec![tx]).height;
    old.step(vec![]);
    assert!(
        aether_execution::proofs::commitment(&old.parent.state, h - 1).is_some(),
        "7780 rules unchanged"
    );
}

/// History proofs across era boundaries come from era roots plus at most two
/// eras' block hashes, and verify against the anchor block's history root.
#[test]
fn history_proofs_cross_era_boundaries() {
    let mut n = Node::start(true, None);
    n.run_to(ERA_LEN + 20, |_| false);
    let idx = n.chain.lock().history_index.clone().unwrap();
    assert_eq!(idx.eras.len(), 1);
    assert_eq!(idx.leaves(), ERA_LEN + 21);
    for (height, anchor) in [
        (0, 1),
        (5, ERA_LEN - 1),
        (ERA_LEN - 1, ERA_LEN),
        (7, ERA_LEN + 20),
        (ERA_LEN + 3, ERA_LEN + 20),
    ] {
        let (proof, hash) = n.chain.history_proof(height, anchor).unwrap();
        let anchor_block = &n.blocks[anchor as usize];
        let v = aether_light::VerifiedBlock {
            height: anchor,
            digest: String::new(),
            timestamp_ms: 0,
            parent_state_root: B256::ZERO,
            history_root: anchor_block.payload().unwrap().history_root,
        };
        let hash: B256 = format!("0x{hash}").parse().unwrap();
        aether_light::verify_history(&v, height, &hash, &proof)
            .unwrap_or_else(|e| panic!("{height} under {anchor}: {e}"));
        let (old, _) = aether_light::verify_old_block(
            &v,
            &commonware_codec::Encode::encode(&n.blocks[height as usize]),
            &proof,
        )
        .unwrap();
        assert_eq!(old.height, height);
    }
    assert!(n.chain.history_proof(ERA_LEN + 20, ERA_LEN + 20).is_err());
}

/// Under history v2 the node keeps each era's blocks and seals the era into a
/// file when its last block is final; the file verifies against a later
/// history root, and the kept blocks are dropped. 7780 rules keep nothing extra.
#[test]
fn the_node_seals_eras_under_history_v2_only() {
    let dir = tmp("seal");
    let mut n = Node::start(true, Some(&dir));
    n.run_to(ERA_LEN + 2, |h| h % 1000 == 0);
    let path = dir.join("eras").join(era::file_name(0));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !path.exists() {
        assert!(std::time::Instant::now() < deadline, "era 0 was not sealed");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let bytes = std::fs::read(&path).unwrap();
    let e = era::read(&bytes, None).unwrap();
    assert_eq!(e.blocks.as_slice(), &n.blocks[..ERA_LEN as usize]);
    let idx = n.chain.lock().history_index.clone().unwrap();
    assert_eq!(e.root, idx.eras[0]);
    let anchor = n.parent.height;
    let proof = era::prove_era(&idx, anchor, 0).unwrap();
    e.verify_in_history(
        &proof,
        &n.blocks[anchor as usize].payload().unwrap().history_root,
    )
    .unwrap();
    let store = n.chain.store().unwrap();
    // Only the open era's blocks are still kept.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while store.staged_eras().unwrap() != vec![1] {
        assert!(
            std::time::Instant::now() < deadline,
            "kept blocks of era 0 not dropped: {:?}",
            store.staged_eras()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    drop(n);
    let _ = std::fs::remove_dir_all(&dir);

    let dir = tmp("seal-off");
    let mut n = Node::start(false, Some(&dir));
    n.run_to(50, |_| false);
    assert!(n.chain.store().unwrap().staged_eras().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bytes per block, before and after (roadmap B1/B3). Release build advised.
#[test]
#[ignore]
fn measure() {
    let blocks = ERA_LEN;
    println!("\n{blocks} blocks per run; B/block");
    println!(
        "{:<34} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "run", "json sum.", "packed", "state", "kept/blk", "era file"
    );
    for (name, v2, tx_every) in [
        ("7780 rules, empty", false, 0u64),
        ("7780 rules, 1 transfer/block", false, 1),
        ("history v2, empty", true, 0),
        ("history v2, 1 transfer/block", true, 1),
    ] {
        let dir = tmp(&format!("measure-{v2}-{tx_every}"));
        let mut n = Node::start(v2, Some(&dir));
        n.run_to(blocks - 1, |h| {
            tx_every > 0 && h > P2_AT && h % tx_every == 0
        });
        let store = n.chain.store().unwrap();
        let stats = store.stats().unwrap();
        let table = |t: &str| {
            stats.tables.iter().find(|x| x.name == t).unwrap().stored as f64 / blocks as f64
        };
        let json: usize = {
            let g = n.chain.lock();
            g.blocks
                .values()
                .map(|s: &BlockSummary| serde_json::to_vec(s).unwrap().len())
                .sum()
        };
        // What state.redb keeps for good per block (the open era's staged blocks are transient).
        let kept: f64 = ["state", "code", "blocks", "receipts", "meta"]
            .iter()
            .map(|t| table(t))
            .sum();
        // The era file of these blocks (7780 rules could archive the same way).
        let start = aether_state::mmr::Mmr::default();
        let era_bytes = era::write(&start, &n.blocks[..ERA_LEN as usize])
            .unwrap()
            .len() as f64
            / blocks as f64;
        println!(
            "{:<34} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>10.2}",
            name,
            json as f64 / blocks as f64,
            table("blocks"),
            table("state"),
            kept,
            era_bytes
        );
        drop(store);
        drop(n);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
