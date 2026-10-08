//! History storage (roadmap B1-B3): what each block costs on disk, history v2
//! (quiet empty blocks), history proofs across eras, and era files sealed by
//! the node. Blocks are built and finalized the way a proposer and marshal do,
//! without consensus, so thousands of blocks run in seconds.
//!
//! `cargo test -p aether-node --release --test history -- --ignored --nocapture measure`
//! prints the bytes-per-block table of the roadmap (B1/B3).

use aether_crypto::{P256Signer, Signer as AetherSigner};
use aether_execution::{recommended_state_budget, sign_call_with, EvmCall};
use aether_execution::{Event, Receipt};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{
    build_payload, dev_accounts, dev_seed, BlockSummary, Chain, ChainConfig, Executed, Extras,
};
use aether_node::era;
use aether_node::rpc::{self, Finality, RpcState};
use aether_node::store::Store;
use aether_node::upgrade::{combine, sign_partial, Release, SignedUpgrade, Upgrade};
use aether_state::mmr::ERA_LEN;
use aether_types::{Address, Bytes, FeeVector, GasVector, TxEnvelope, B256, U256};
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible, Signer};
use std::path::PathBuf;
use std::sync::Arc;
use serde_json::json;

const CHAIN: u64 = 7_791;
/// Legacy history mode upgrades to protocol 2 here; history v2 starts with it.
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
        // History v2 alone puts a chain under the mainnet rules
        // (`Chain::admissible_upgrade`: an ordinary upgrade needs seven days of
        // notice), so its genesis starts at protocol 2 — the rules 7780
        // activates at block 20 — instead of scheduling that upgrade.
        protocol: if history_v2 { 2 } else { 1 },
        node_rewards: false,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
        committee: vec![],
        reserve: None,
    }
}

fn upgrade_to_2() -> SignedUpgrade {
    let (_, sharing, shares) = aether_light::devnet_threshold(4);
    let u = Upgrade {
        chain_id: CHAIN,
        protocol: 2,
        activate_at: P2_AT,
        emergency: false,
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
        // A 7780-shaped chain (protocol 1) schedules protocol 2 twenty blocks
        // ahead; a history-v2 genesis already runs it from height 0.
        let upgrade = (h == 1 && self.parent.next_protocol() == 1).then(upgrade_to_2);
        let (pre, _) = self
            .chain
            .pre_state(&self.parent, self.parent.next_protocol(), &[], None, false)
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
            state: aether_execution::fees::STATE_UNIT_PRICE,
            prove: 100_000_000_000,
        };
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

#[tokio::test]
async fn account_history_indexes_finalized_sends_receives_and_rpc_cursor() {
    let dir = tmp("account-history");
    std::fs::create_dir_all(&dir).unwrap();
    let mut n = Node::start(true, Some(&dir));
    let sender = aether_crypto::address_of(&aether_crypto::Signer::public_key(&P256Signer::from_seed(&dev_seed(1)).unwrap())).unwrap();
    let receiver = Address::repeat_byte(0xb0);
    let first = n.transfer();
    n.step(vec![first]);
    let second = n.transfer();
    n.step(vec![second]);
    let own = n.chain.account_history(&sender, None, 1).unwrap();
    assert_eq!(own.entries.len(), 1);
    assert_eq!(own.entries[0].height, 2);
    assert_eq!(own.entries[0].direction, "out");
    let older = n.chain.account_history(&sender, own.next_cursor.as_deref(), 1).unwrap();
    assert_eq!(older.entries[0].height, 1);
    let received = n.chain.account_history(&receiver, None, 10).unwrap();
    assert_eq!(received.entries.len(), 2);
    assert!(received.entries.iter().all(|e| e.direction == "in" && e.value_wei == "1000"));

    let st = RpcState {
        chain: n.chain.clone(), finality: Finality::Archive(Arc::new(aether_node::follow::FinalityArchive::new(None))),
        gossip: tokio::sync::mpsc::unbounded_channel().0, faucet: None, registrar: None,
        network: None, upstream: None, handoff: None, snapshot: Default::default(), prover: None, shards: None, presence: None, public_read_only: false,
    };
    let answer = rpc::handle_value(&st, json!({"jsonrpc":"2.0","id":1,"method":"aether_accountHistory","params":[receiver,null,200]})).await;
    assert_eq!(answer["result"]["entries"].as_array().unwrap().len(), 2);
    assert_eq!(answer["result"]["indexed_height"], 2);
    let rejected = rpc::handle_value(&st, json!({"jsonrpc":"2.0","id":2,"method":"aether_accountHistory","params":[receiver,null,201]})).await;
    assert_eq!(rejected["error"]["code"], -32602);
    drop(st);
    drop(n);
    let (restored, _) = Chain::open(config(true), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
    assert_eq!(restored.account_history(&receiver, None, 10).unwrap().entries.len(), 2);
    restored.store().unwrap().prune_below(2, &[]).unwrap();
    let kept = restored.account_history(&receiver, None, 10).unwrap();
    assert_eq!(kept.entries.len(), 1);
    assert_eq!(kept.history_start, 2);
    drop(restored);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn account_history_indexes_all_delegated_batch_receivers_once_per_address() {
    let dir = tmp("delegated-batch");
    let mut n = Node::start(true, Some(&dir));
    let signer = P256Signer::from_seed(&dev_seed(1)).unwrap();
    let sender = aether_crypto::address_of(&aether_crypto::Signer::public_key(&signer)).unwrap();
    let first = Address::repeat_byte(0xa1);
    let second = Address::repeat_byte(0xa2);
    let calls = [
        (first, U256::from(3), Bytes::new()),
        (second, U256::from(7), Bytes::new()),
        (first, U256::from(11), Bytes::new()),
    ];
    let call = EvmCall {
        to: Some(sender), value: U256::ZERO,
        input: aether_execution::encode_execute(&calls),
        gas_limit: 200_000, delegate: Some(aether_execution::AETHER_ACCOUNT),
    };
    let fees = FeeVector { exec: 100_000_000_000, state: aether_execution::fees::STATE_UNIT_PRICE, prove: 100_000_000_000 };
    let mut tx = sign_call_with(&signer, CHAIN, 0, fees, 1_000_000_000, &call).unwrap();
    // The sender is funded: reserve what the batch's new accounts cost.
    tx.header.gas.state = recommended_state_budget(&call, Some(U256::from(10u128.pow(24))), fees.state);
    let mut signature = AetherSigner::sign(&signer, &tx.signing_bytes()).unwrap();
    signature.extend_from_slice(&AetherSigner::public_key(&signer).bytes);
    tx.signature = Bytes::from(signature);
    let exec = n.step(vec![tx]);
    assert!(exec.receipts[0].success);

    let first_rows = n.chain.account_history(&first, None, 10).unwrap().entries;
    assert_eq!(first_rows.len(), 1);
    assert_eq!(first_rows[0].value_wei, "14");
    assert_eq!(first_rows[0].direction, "in");
    let second_rows = n.chain.account_history(&second, None, 10).unwrap().entries;
    assert_eq!(second_rows.len(), 1);
    assert_eq!(second_rows[0].value_wei, "7");
    assert_eq!(second_rows[0].direction, "in");
    let sender_rows = n.chain.account_history(&sender, None, 10).unwrap().entries;
    assert_eq!(sender_rows.len(), 1);
    assert_eq!(sender_rows[0].value_wei, "21");
    assert_eq!(sender_rows[0].tx_hash, first_rows[0].tx_hash);
    drop(n);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn account_history_keeps_batch_receipts_when_delegation_is_cleared_later_in_block() {
    let dir = tmp("delegation-cleared");
    let mut n = Node::start(true, Some(&dir));
    let signer = P256Signer::from_seed(&dev_seed(1)).unwrap();
    let sender = aether_crypto::address_of(&aether_crypto::Signer::public_key(&signer)).unwrap();
    let recipient = Address::repeat_byte(0xa3);
    let fees = FeeVector { exec: 100_000_000_000, state: aether_execution::fees::STATE_UNIT_PRICE, prove: 100_000_000_000 };
    let paid = |nonce: u64, call: &EvmCall| {
        // The sender is funded: reserve what the call's new state costs.
        let mut tx = sign_call_with(&signer, CHAIN, nonce, fees, 1_000_000_000, call).unwrap();
        tx.header.gas.state = recommended_state_budget(call, Some(U256::from(10u128.pow(24))), fees.state);
        let mut signature = AetherSigner::sign(&signer, &tx.signing_bytes()).unwrap();
        signature.extend_from_slice(&AetherSigner::public_key(&signer).bytes);
        tx.signature = Bytes::from(signature);
        tx
    };
    let batch = EvmCall {
        to: Some(sender), value: U256::ZERO,
        input: aether_execution::encode_execute(&[(recipient, U256::from(5), Bytes::new())]),
        gas_limit: 150_000, delegate: Some(aether_execution::AETHER_ACCOUNT),
    };
    let clear = EvmCall {
        to: Some(sender), value: U256::ZERO, input: Bytes::new(),
        gas_limit: 100_000, delegate: Some(Address::ZERO),
    };
    let first = paid(0, &batch);
    // EIP-7702 authorization consumes the nonce after the outer transaction.
    let second = paid(2, &clear);
    let exec = n.step(vec![first, second]);
    assert_eq!(exec.receipts.len(), 2);
    assert!(exec.receipts.iter().all(|receipt| receipt.success));
    let rows = n.chain.account_history(&recipient, None, 10).unwrap().entries;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].value_wei, "5");
    drop(n);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn account_history_collects_token_receipts_and_system_rewards() {
    let signer = P256Signer::from_seed(&dev_seed(1)).unwrap();
    let sender = aether_crypto::address_of(&aether_crypto::Signer::public_key(&signer)).unwrap();
    let recipient = Address::repeat_byte(0xab);
    let token = Address::repeat_byte(0xcd);
    let call = EvmCall { to: Some(token), value: U256::ZERO,
        input: Bytes::from(hex::decode("a9059cbb").unwrap()), gas_limit: 100_000, delegate: None };
    let tx = sign_call_with(&signer, CHAIN, 0, FeeVector::default(), 0, &call).unwrap();
    let mut from = [0u8; 32];
    from[12..].copy_from_slice(sender.as_slice());
    let mut to = [0u8; 32];
    to[12..].copy_from_slice(recipient.as_slice());
    let topic = hex::decode("ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef").unwrap();
    let swap_topic = hex::decode("d78ad95fa46c994b6551d0da85fc275fe613ce37657fb8d5e3d130840159d822").unwrap();
    let swap_data = [U256::from(10u64), U256::ZERO, U256::ZERO, U256::from(250u64)]
        .into_iter().flat_map(|v| v.to_be_bytes::<32>()).collect::<Vec<_>>();
    let receipt = Receipt { tx_hash: aether_execution::tx_hash(&tx), success: true, gas_used: 50_000,
        prove_gas: 0, state_gas: 0, state_fee: U256::ZERO, contract_address: None, logs: 2, output: Bytes::new(),
        events: vec![Event { address: token,
            topics: vec![B256::from_slice(&topic), B256::from(from), B256::from(to)],
            data: Bytes::from(U256::from(250u64).to_be_bytes::<32>().to_vec()) },
            Event { address: Address::repeat_byte(0xef),
                topics: vec![B256::from_slice(&swap_topic), B256::from(from), B256::from(to)],
                data: Bytes::from(swap_data) }] };
    let rows = aether_node::account_history::transaction(&tx, &receipt, 3, 0, 3000, aether_types::U256::ZERO, false, true);
    assert_eq!(rows.len(), 2);
    let incoming = rows.iter().find(|r| r.address == recipient).unwrap();
    assert_eq!(incoming.kind, "erc20_transfer");
    assert_eq!(incoming.tokens[0].amount, "250");
    assert_eq!(incoming.direction, "in");
    assert!(incoming.pair_swaps.is_empty(), "recipient rows must not duplicate every swap in the sender's call");
    let outgoing = rows.iter().find(|r| r.address == sender).unwrap();
    assert_eq!(outgoing.pair_swaps[0].amount1_out, "250");
    let reward = aether_node::account_history::reward(recipient, 4, 0, 4000, U256::from(9), true);
    assert_eq!(reward.kind, "node_reward");
    assert_eq!(reward.value_wei, "9");
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
            receipts_root: None,
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
