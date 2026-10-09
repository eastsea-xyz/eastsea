//! REL-23: legacy cache eviction and offline reclamation retain archival data.

use aether_crypto::P256Signer;
use aether_execution::{sign_call, EvmCall, Receipt, WorldState};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{
    build_payload, dev_accounts, dev_seed, BlockSummary, Chain, ChainConfig, Extras,
};
use aether_node::follow::FinalityArchive;
use aether_node::rpc::{self, RpcState};
use aether_node::store::{Commit, Store};
use aether_types::{Address, Bytes, GasVector, B256, U256};
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible, Signer};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

const CHAIN: u64 = 7_798;

fn temporary(name: &str) -> PathBuf {
    let root = std::env::var_os("TMPDIR").expect("tests require worktree TMPDIR");
    let path = PathBuf::from(root).join(format!("rel23-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn config() -> ChainConfig {
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
        fees: false,
        registrar: None,
        epoch_blocks: 10,
        min_streak: None,
        draw_epochs: None,
        history_v2: false,
        protocol: 2,
        node_rewards: false,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
        committee: vec![],
        reserve: None,
    }
}

fn rpc_state(chain: Chain) -> RpcState {
    RpcState {
        finality: rpc::Finality::Archive(Arc::new(FinalityArchive::new(chain.store()))),
        chain,
        gossip: tokio::sync::mpsc::unbounded_channel().0,
        faucet: None,
        registrar: None,
        network: None,
        upstream: None,
        handoff: None,
        snapshot: Default::default(),
        prover: None,
        shards: None,
        public_read_only: false,
        presence: None,
        app_bundles: None,
    }
}

#[test]
fn rel23_legacy_cache_budget_preserves_archived_queries_and_proofs() {
    let dir = temporary("legacy-cache");
    let path = dir.join("state.redb");
    let (chain, genesis) = Chain::open(config(), Store::open(&path).unwrap()).unwrap();
    let (_, sharing, _) = aether_light::devnet_threshold(4);
    chain.lock().identity = Some(*sharing.public());
    chain.finalize(&genesis).unwrap();
    let mut executions = vec![genesis.digest()];
    let mut previous = genesis;
    let mut parent = chain.lock().finalized.clone();
    let signer = P256Signer::from_seed(&dev_seed(1)).unwrap();
    let call = EvmCall {
        to: Some(Address::repeat_byte(0xb0)),
        value: U256::from(7u64),
        input: Bytes::new(),
        gas_limit: 21_000,
        delegate: None,
    };
    let tx = sign_call(&signer, CHAIN, 0, 1, &call).unwrap();
    let hash = aether_execution::tx_hash(&tx);
    let mut first = None;
    for h in 1..=40 {
        let height = previous.height.next();
        let context = Context {
            round: Round::new(EPOCH, View::new(h)),
            leader: ed25519::PrivateKey::from_seed(h % 4).public_key(),
            parent: (View::new(h - 1), previous.digest()),
        };
        let timestamp = 1_790_000_000_000 + h * 1000;
        let skeleton = Block::new(
            context.clone(),
            previous.digest(),
            height,
            timestamp,
            bytes::Bytes::new(),
        );
        let ctx = Chain::block_context(&chain.cfg(), &skeleton, &parent);
        let (pre, _) = chain
            .pre_state(&parent, parent.next_protocol(), &[], None, false)
            .unwrap();
        let txs = if h == 1 { vec![tx.clone()] } else { vec![] };
        let (payload, _) = build_payload(&parent, &pre, &ctx, txs, Extras::default());
        let block = Block::new(
            context,
            previous.digest(),
            height,
            timestamp,
            payload.to_bytes(),
        );
        let exec = chain.execute(&block, &parent).unwrap();
        chain.finalize(&block).unwrap();
        if h == 1 {
            first = Some(block.clone());
        }
        executions.push(block.digest());
        previous = block;
        parent = exec;
    }
    let receipt = chain
        .store()
        .unwrap()
        .receipt(&hash)
        .unwrap()
        .expect("durable receipt");
    let before_proof = chain.history_proof(1, 40).unwrap();
    let root = parent.state.root();
    let budget = 4_096;
    assert!(
        chain.caches_bytes() > budget,
        "fixture must exceed the cache budget"
    );
    chain.trim_history_caches_with(budget);
    // This deliberately tiny budget is below the mandatory head-and-parent
    // working state. Every optional execution and archival cache row must go;
    // realistic total-allocation budgets are covered by the H04 rescue tests.
    assert!(
        chain.history_rows_bytes() <= budget - budget / 4,
        "optional history rows fit with 25% headroom"
    );
    {
        let guard = chain.lock();
        assert_eq!(guard.cache_below, 40, "only the head summary stays cached");
        assert_eq!(guard.blocks.len(), 1);
        assert!(guard.blocks.contains_key(&40));
        assert!(guard.receipts.is_empty(), "archived receipts leave memory");
    }
    for (height, digest) in executions.iter().enumerate() {
        assert_eq!(
            chain.get(digest).is_some(),
            height >= 39,
            "only the finalized head and its parent retain execution state: height {height}"
        );
    }
    assert_eq!(
        chain.lock().pruned_below,
        0,
        "eviction is not archival pruning"
    );
    assert_eq!(chain.lock().finalized.state.root(), root);
    assert_eq!(
        chain.history_proof(1, 40).unwrap(),
        before_proof,
        "history verification survives eviction"
    );
    // Registration confirmations are derived from bounded finalized ids,
    // rather than durable transaction receipts. Simulate an evicted map row.
    let registration_id = B256::repeat_byte(0x44);
    {
        let mut guard = chain.lock();
        Arc::make_mut(&mut guard.finalized)
            .registration_ids
            .push(registration_id);
    }
    chain.finalize(first.as_ref().unwrap()).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let st = rpc_state(chain.clone());
    rt.block_on(async {
        let block = rpc::handle_value(
            &st,
            json!({"jsonrpc":"2.0","id":1,"method":"aether_getBlock","params":[1]}),
        )
        .await;
        assert_eq!(
            block["result"]["hash"],
            json!(format!("{}", first.unwrap().digest())),
            "{block}"
        );
        let answer = rpc::handle_value(
            &st,
            json!({"jsonrpc":"2.0","id":2,"method":"aether_getReceipt","params":[hash]}),
        )
        .await;
        assert_eq!(
            answer["result"],
            json!({"height":receipt.0,"receipt":receipt.1}),
            "{answer}"
        );
        let confirmation = rpc::handle_value(
            &st,
            json!({"jsonrpc":"2.0","id":3,
            "method":"aether_getReceipt","params":[registration_id]}),
        )
        .await;
        assert_eq!(
            confirmation["result"]["height"],
            json!(40),
            "{confirmation}"
        );
        assert_eq!(confirmation["result"]["receipt"]["success"], json!(true));
        assert_eq!(confirmation["result"]["receipt"]["gas_used"], json!(0));
    });
    assert!(
        !chain.lock().receipts.contains_key(&registration_id),
        "derived reads do not grow the cache"
    );
    assert!(
        chain
            .store()
            .unwrap()
            .receipt(&registration_id)
            .unwrap()
            .is_none(),
        "no durable orphan receipt"
    );
    drop(st);
    drop(chain);
    let loaded = Store::open(&path).unwrap().load().unwrap().unwrap();
    assert_eq!(loaded.height, 40);
    assert_eq!(loaded.state.root(), root);
    assert_eq!(loaded.blocks.len(), 41, "every durable summary remains");
    assert_eq!(loaded.receipts.len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

fn summary(height: u64, state: &WorldState) -> BlockSummary {
    BlockSummary {
        height,
        hash: hex::encode([height as u8; 32]),
        parent: hex::encode([height.saturating_sub(1) as u8; 32]),
        timestamp_ms: height * 1000,
        proposer: Address::ZERO,
        state_root: state.root(),
        parent_state_root: state.root(),
        txs: if height == 1 {
            vec![B256::repeat_byte(0x43)]
        } else {
            vec![]
        },
        gas_used: 0,
        prove_gas: 0,
        base_fee: Default::default(),
        excess: Default::default(),
        archive_excess: 0,
    }
}

#[test]
fn rel23_offline_compaction_reports_allocations_and_preserves_finalized_rows() {
    let dir = temporary("offline-compact");
    let fixture = dir.join("original.redb");
    let path = dir.join("copy.redb");
    let store = Store::open(&fixture).unwrap();
    let mut state = WorldState::default();
    state
        .set_balance(Address::repeat_byte(0x31), U256::from(123u64))
        .unwrap();
    let receipt = Receipt {
        tx_hash: B256::repeat_byte(0x43),
        success: true,
        gas_used: 21_000,
        prove_gas: 0,
        state_gas: 0,
        state_fee: U256::ZERO,
        contract_address: None,
        logs: 0,
        output: Bytes::from(vec![0xab; 256]),
        events: vec![],
    };
    for height in 0..=3 {
        let summary = summary(height, &state);
        store
            .commit(Commit {
                height,
                digest: [height as u8; 32],
                root: state.root(),
                diff: state.journal(),
                summary: &summary,
                receipts: if height == 1 {
                    vec![(receipt.tx_hash, &receipt)]
                } else {
                    vec![]
                },
                handoff: None,
                seed: None,
                history: &Default::default(),
                schedule: &Default::default(),
                upgrade_notices: &[],
                statement: &Default::default(),
                staged: None,
            })
            .unwrap();
    }
    let certificate = br#"{"height":1,"verified_certificate":"fixture"}"#;
    let reward = br#"{"height":3,"amount":"17"}"#;
    store.put_proof(1, certificate).unwrap();
    store.put_reward(&[0x31; 20], 3, 1, reward).unwrap();
    store.put_meta("churn", &vec![7; 8 << 20]).unwrap();
    store.put_meta("churn", b"archival marker").unwrap();
    let head = store.head().unwrap();
    drop(store);
    std::fs::copy(&fixture, &path).unwrap();
    let original = std::fs::read(&fixture).unwrap();
    let file_before = std::fs::metadata(&path).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aether"))
        .args(["db-maintenance", "--db"])
        .arg(&path)
        .arg("--compact")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "REL-23 offline storage command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["before"]["file_bytes"], json!(file_before.len()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            report["before"]["filesystem_allocated_bytes"],
            json!(file_before.blocks() * 512)
        );
    }
    let before = &report["before"];
    assert_eq!(
        before["allocated_bytes"].as_u64().unwrap(),
        before["allocated_pages"].as_u64().unwrap() * before["page_size"].as_u64().unwrap()
    );
    assert_eq!(
        before["reclaimable_estimate_bytes"].as_u64().unwrap(),
        file_before
            .len()
            .saturating_sub(before["allocated_bytes"].as_u64().unwrap())
    );
    let after = report["after"]["file_bytes"].as_u64().unwrap();
    assert!(
        after < file_before.len(),
        "compaction must reclaim fixture free pages: {report}"
    );
    let file_after = std::fs::metadata(&path).unwrap();
    assert_eq!(after, file_after.len());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            report["after"]["filesystem_allocated_bytes"],
            json!(file_after.blocks() * 512)
        );
    }
    assert_eq!(
        report["after"]["reclaimable_estimate_bytes"].as_u64().unwrap(),
        file_after.len().saturating_sub(report["after"]["allocated_bytes"].as_u64().unwrap())
    );
    let reopened = Store::open(&path).unwrap();
    let checkpoint = reopened.load().unwrap().unwrap();
    assert_eq!(reopened.head().unwrap(), head);
    assert_eq!(checkpoint.height, 3);
    assert_eq!(checkpoint.digest, [3; 32]);
    assert_eq!(checkpoint.state.root(), state.root());
    assert_eq!(checkpoint.blocks.len(), 4);
    assert_eq!(
        serde_json::to_value(reopened.receipt(&receipt.tx_hash).unwrap()).unwrap(),
        json!(Some((1u64, &receipt)))
    );
    assert_eq!(
        reopened.proof(1).unwrap().as_deref(),
        Some(certificate.as_slice())
    );
    assert_eq!(
        reopened.rewards(&[0x31; 20], 8).unwrap(),
        vec![reward.to_vec()]
    );
    assert_eq!(
        reopened.meta("churn").unwrap(),
        Some(b"archival marker".to_vec())
    );
    let locked = Command::new(env!("CARGO_BIN_EXE_aether"))
        .args(["db-maintenance", "--db"])
        .arg(&path)
        .arg("--compact")
        .output()
        .unwrap();
    assert!(
        !locked.status.success(),
        "an open node store must refuse offline compaction"
    );
    assert_eq!(reopened.head().unwrap(), head);
    drop(reopened);
    assert_eq!(
        std::fs::read(&fixture).unwrap(),
        original,
        "only the copy may change"
    );
    let absent = dir.join("absent.redb");
    let missing = Command::new(env!("CARGO_BIN_EXE_aether"))
        .args(["db-maintenance", "--db"])
        .arg(&absent)
        .arg("--compact")
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(
        !absent.exists(),
        "maintenance must never create a missing database"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
