//! Resource limits (docs/ops/resource-limits.md): the history caches' memory
//! budget. Blocks are built and finalized the way a proposer and marshal do,
//! as in history.rs, so a couple of eras run in seconds.

use aether_crypto::{P256Signer, Signer as AetherSigner};
use aether_execution::{recommended_state_budget, sign_call_with, EvmCall};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{build_payload, dev_accounts, dev_seed, Chain, ChainConfig, Executed, Extras};
use aether_node::era;
use aether_node::store::Store;
use aether_state::mmr::ERA_LEN;
use aether_types::{Address, Bytes, FeeVector, GasVector, TxEnvelope, U256};
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible, Signer};
use std::path::PathBuf;
use std::sync::Arc;

const CHAIN: u64 = 7_791;

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
        protocol: 1,
        committee: vec![],
        reserve: None,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
    }
}

/// A validator's chain with a store, as in history.rs.
struct Node {
    chain: Chain,
    blocks: Vec<Block>,
    parent: Arc<Executed>,
    nonce: u64,
}

impl Node {
    fn start(dir: &PathBuf) -> Node {
        let (chain, genesis) = Chain::open(config(), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        chain.lock().identity = Some(*sharing.public());
        chain.finalize(&genesis).unwrap();
        let parent = chain.lock().finalized.clone();
        Node { chain, blocks: vec![genesis], parent, nonce: 0 }
    }

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
        let ts = 1_790_000_000_000 + h * 1000;
        let skeleton = Block::new(context.clone(), prev.digest(), height, ts, bytes::Bytes::new());
        let ctx = Chain::block_context(&self.chain.cfg(), &skeleton, &self.parent);
        let (pre, _) = self.chain.pre_state(&self.parent, self.parent.next_protocol(), &[], None, false).unwrap();
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
        let call = EvmCall {
            to: Some(Address::repeat_byte(0xb0)),
            value: U256::from(1_000u64),
            input: Bytes::new(),
            gas_limit: 21_000,
            delegate: None,
        };
        let fees = FeeVector { exec: 100_000_000_000, state: aether_execution::fees::STATE_UNIT_PRICE, prove: 100_000_000_000 };
        self.nonce += 1;
        let mut tx = sign_call_with(&signer, CHAIN, self.nonce - 1, fees, 1_000_000_000, &call).unwrap();
        // The sender is funded: reserve enough to pay for the new account the
        // transfer creates (paid state growth on new-genesis chains).
        tx.header.gas.state = recommended_state_budget(&call, Some(U256::from(10u128.pow(24))), fees.state);
        let mut signature = AetherSigner::sign(&signer, &tx.signing_bytes()).unwrap();
        signature.extend_from_slice(&AetherSigner::public_key(&signer).bytes);
        tx.signature = Bytes::from(signature);
        tx
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aether-resources-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Over the `--max-memory` budget, the oldest sealed era's summaries and
/// receipts leave memory — while its era file keeps serving the blocks, and
/// history proofs over the evicted era still verify. A memory-only chain
/// keeps its history rows because no durable archive backs eviction.
#[test]
fn the_history_caches_trim_to_their_budget_and_old_blocks_still_serve() {
    let dir = tmp("caches");
    let mut n = Node::start(&dir);
    // Two sealed eras and a bit of the third; a receipt to follow through.
    let mut receipt_at_100 = None;
    while n.parent.height < 2 * ERA_LEN + 10 {
        let h = n.parent.height + 1;
        let txs = if h % 100 == 0 { vec![n.transfer()] } else { vec![] };
        let exec = n.step(txs);
        if h == 100 {
            receipt_at_100 = exec.tx_hashes.first().copied();
        }
    }
    let receipt_at_100 = receipt_at_100.expect("a tx at height 100");
    // The era files are sealed off the consensus path: wait for era 0's.
    let store = n.chain.store().expect("the store");
    let era0 = store.era_dir().join(era::file_name(0));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !era0.exists() {
        assert!(std::time::Instant::now() < deadline, "era 0 was never sealed");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // Era 1 sealed a moment later (both seals are off the consensus path).
    while !store.era_dir().join(era::file_name(1)).exists() {
        assert!(std::time::Instant::now() < deadline, "era 1 was never sealed");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // A budget that fits does nothing.
    let all = n.chain.caches_bytes();
    assert!(all > 0);
    assert!(n.chain.lock().blocks.contains_key(&1), "era 0 is cached");
    n.chain.trim_history_caches_with(u64::MAX);
    assert_eq!(n.chain.lock().cache_below, 0, "nothing to drop");

    // The row-only total cannot fit with 25% headroom, even after optional
    // execution states leave. One sealed era makes room for the mandatory
    // head and parent while the newer sealed era and open era stay cached.
    let budget = n.chain.history_rows_bytes();
    assert!(all > budget, "full execution states are charged too");
    let ceiling = budget - budget / 4;
    n.chain.trim_history_caches_with(budget);
    {
        let g = n.chain.lock();
        assert_eq!(g.cache_below, ERA_LEN, "era 0's cache copy went");
        assert!(!g.blocks.contains_key(&1), "its summaries went");
        assert!(g.blocks.contains_key(&(ERA_LEN + 1)), "era 1's summaries stay");
        assert!(!g.receipts.contains_key(&receipt_at_100), "era 0's receipts went");
        assert!(g.blocks.contains_key(&(2 * ERA_LEN + 5)), "the open era stays");
    }
    assert!(n.chain.caches_bytes() <= ceiling, "total retention fits with 25% headroom");

    // What left memory still serves: the block from its era file...
    let old = n.chain.old_block(1).expect("era 0's file serves its blocks");
    assert_eq!(old.digest(), n.blocks[1].digest());
    // ...and a history proof over the evicted era still verifies.
    let (_, hash) = n.chain.history_proof(1, 2 * ERA_LEN).expect("a proof over era 0");
    assert_eq!(hash, format!("{}", n.blocks[1].digest()));

    // Budget zero (everything that can go, goes): only the open era stays.
    n.chain.trim_history_caches_with(0);
    {
        let g = n.chain.lock();
        assert_eq!(g.cache_below, 2 * ERA_LEN);
        assert!(!g.blocks.contains_key(&ERA_LEN), "era 1's cache copy went");
        assert!(g.blocks.contains_key(&(2 * ERA_LEN + 5)), "the open era stays");
    }
    let old = n.chain.old_block(ERA_LEN).expect("era 1's file serves its blocks");
    assert_eq!(old.digest(), n.blocks[ERA_LEN as usize].digest());

    let _ = std::fs::remove_dir_all(&dir);
}

/// A memory-only chain never evicts history rows: neither redb nor an era
/// file could serve the dropped heights.
#[test]
fn a_memory_only_chain_never_evicts_history_rows() {
    let mut cfg = config();
    cfg.history_v2 = false;
    let (chain, genesis) = Chain::new(cfg);
    assert!(chain.store().is_none(), "the fixture has no durable archive");
    let (_, sharing, _) = aether_light::devnet_threshold(4);
    chain.lock().identity = Some(*sharing.public());
    chain.finalize(&genesis).unwrap();
    let parent = chain.lock().finalized.clone();
    let mut n = Node { parent, chain, blocks: vec![genesis], nonce: 0 };
    while n.parent.height < 30 {
        n.step(vec![]);
    }
    let rows = n.chain.history_rows_bytes();
    let before_proof = n.chain.history_proof(1, 30).unwrap();
    n.chain.trim_history_caches_with(0);
    {
        let g = n.chain.lock();
        assert_eq!(g.cache_below, 0, "no durable archive, no history eviction");
        assert_eq!(g.blocks.len(), 31, "every finalized summary remains");
        assert!(g.blocks.contains_key(&1));
    }
    assert_eq!(n.chain.history_rows_bytes(), rows);
    assert_eq!(n.chain.history_proof(1, 30).unwrap(), before_proof);
    assert_eq!(n.chain.block_summary(1).unwrap().unwrap().hash, format!("{}", n.blocks[1].digest()));
}
