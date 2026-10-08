//! Candidate diagnostics are an additive, read-only view of finalized state.

mod common;

use aether_execution::registry;
use aether_node::chain::{BlockSummary, Chain, ChainConfig};
use aether_node::rpc::{self, Finality, RpcState};
use aether_types::{Address, GasVector};
use serde_json::{json, Value};
use std::sync::Arc;

fn state(rewards: bool) -> RpcState {
    let committee: Vec<_> = (1..=4u8)
        .map(|i| {
            let node = aether_net::SecretKey::from_bytes(&[i; 32]).public();
            (hex::encode([i; 32]), node.to_string())
        })
        .collect();
    let (chain, _) = Chain::new(ChainConfig {
        chain_id: 7781,
        limits: GasVector {
            exec: 30_000_000,
            state: u64::MAX,
            prove: 200_000_000,
        },
        alloc: vec![],
        fees: false,
        registrar: Some(([1; 32], [2; 32])),
        epoch_blocks: 3_600,
        min_streak: Some(24),
        draw_epochs: Some(24),
        history_v2: false,
        protocol: 3,
        node_rewards: rewards,
        committee: committee.clone(),
        reserve: None,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
    });
    {
        let mut g = chain.lock();
        let mut head = (*g.finalized).clone();
        let epoch = if rewards { 47 } else { 23 };
        head.height = epoch * 3_600 + 1_800;
        head.timestamp = 1_000_000;
        // A finalized 100-block sample at one second per block: one-hour epochs.
        let height = head.height - 100;
        g.blocks.insert(
            height,
            BlockSummary {
                height,
                hash: "previous".into(),
                parent: "parent".into(),
                timestamp_ms: 900_000,
                proposer: Address::ZERO,
                state_root: Default::default(),
                parent_state_root: Default::default(),
                txs: vec![],
                gas_used: 0,
                prove_gas: 0,
                base_fee: Default::default(),
                excess: Default::default(),
                archive_excess: 0,
            },
        );
        if rewards {
            aether_rewards::registry_v3::genesis(&mut head.state).unwrap();
            // Followers have no node-local voting committee: use the chain's record.
            g.committee.members.clear();
        } else {
            g.committee.members = committee;
        }
        let c = registry::Candidate {
            index: 0,
            operator: Address::repeat_byte(0x44),
            validator_key: [9; 32],
            node_id: *aether_net::SecretKey::from_bytes(&[9; 32])
                .public()
                .as_bytes(),
            beaconer: Address::repeat_byte(0x44),
            registered_epoch: 0,
            last_epoch: if rewards { epoch - 1 } else { epoch },
            streak: 23,
            missed: 0,
        };
        aether_rewards::put_candidate(&mut head.state, &c);
        if rewards {
            for completed in epoch - 6..epoch {
                aether_rewards::beacons::note(
                    &mut head.state,
                    c.index,
                    completed,
                    aether_rewards::SLOTS,
                    true,
                );
            }
        }
        g.finalized = Arc::new(head);
    }
    let (gossip, _) = tokio::sync::mpsc::unbounded_channel();
    RpcState {
        chain,
        finality: Finality::Archive(Arc::new(aether_node::follow::FinalityArchive::default())),
        gossip,
        faucet: None,
        registrar: None,
        network: None,
        upstream: None,
        handoff: None,
        snapshot: Default::default(),
        prover: None,
        shards: None,
        public_read_only: false,
    }
}

async fn candidates(st: &RpcState, method: &str) -> Value {
    rpc::handle_value(
        st,
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": [] }),
    )
    .await
}

#[test]
fn candidate_observability_rpc_reports_the_next_draw_and_preserves_state() {
    let st = state(false);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let root = st.chain.lock().finalized.state.root();
    let answer = rt.block_on(candidates(&st, "aether_candidates"));
    let result = &answer["result"];
    assert_eq!(
        result["next_draw_epoch"], 24,
        "next draw uses registry epochs, not draw indices"
    );
    assert_eq!(result["open_seats"], 1);
    assert_eq!(result["epoch"], 23);
    assert!(result["max_per_epoch"].is_number());
    let c = &result["candidates"][0];
    assert_eq!(c["missed"], 0);
    assert_eq!(c["streak"], 23);
    assert_eq!(c["eligible_next_draw"], false);
    assert_eq!(c["why_not"], "streak");
    assert_eq!(c["hours_to_eligible"], 1.0);
    assert_eq!(answer, rt.block_on(candidates(&st, "eastsea_candidates")));
    assert_eq!(
        st.chain.lock().finalized.state.root(),
        root,
        "diagnostics never write chain state"
    );
    // Without a timestamp sample, the rule is known but its wall-clock ETA is not.
    st.chain.lock().blocks.clear();
    let answer = rt.block_on(candidates(&st, "aether_candidates"));
    assert_eq!(answer["result"]["candidates"][0]["why_not"], "streak");
    assert!(answer["result"]["candidates"][0]["hours_to_eligible"].is_null());
}

#[test]
fn candidate_observability_rpc_uses_v3_rules_and_the_recorded_committee_on_followers() {
    let st = state(true);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let root = st.chain.lock().finalized.state.root();
    let answer = rt.block_on(candidates(&st, "aether_candidates"));
    let result = &answer["result"];
    assert_eq!(
        result["open_seats"], 1,
        "followers read the committee from finalized state"
    );
    assert_eq!(result["next_draw_epoch"], 48);
    assert_eq!(result["candidates"][0]["eligible_next_draw"], true);
    assert_eq!(result["candidates"][0]["why_not"], "none");
    assert_eq!(result["candidates"][0]["hours_to_eligible"], 0.0);
    assert_eq!(st.chain.lock().finalized.state.root(), root);
    // A draw already frozen at epoch 48 is not opened again by a read.
    {
        let mut g = st.chain.lock();
        let mut head = (*g.finalized).clone();
        head.height = 48 * 3_600;
        g.finalized = Arc::new(head);
    }
    let answer = rt.block_on(candidates(&st, "aether_candidates"));
    assert_eq!(answer["result"]["next_draw_epoch"], 72);
    assert_eq!(
        answer["result"]["candidates"][0]["eligible_next_draw"], false,
        "old liveness is not carried into a later epoch"
    );
    assert_eq!(answer["result"]["candidates"][0]["why_not"], "last_epoch");
}

#[test]
fn candidate_observability_logs_the_pool_when_the_draw_actually_freezes() {
    use common::{Net, Opts};
    use std::io::Write;
    use std::sync::Mutex;
    #[derive(Clone)]
    struct Log(Arc<Mutex<Vec<u8>>>);
    impl Write for Log {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    for pool_size in [0, 1] {
        let committee = (1..=10u8)
            .map(|i| {
                (
                    hex::encode([i; 32]),
                    aether_net::SecretKey::from_bytes(&[i; 32])
                        .public()
                        .to_string(),
                )
            })
            .collect();
        let mut net = Net::new(Opts {
            chain_id: 7_793,
            node_rewards: true,
            epoch_blocks: 36,
            macs: 1,
            min_streak: Some(0),
            history_v2: false,
            protocol: 1,
            fees: false,
            reserve: None,
            committee: Some(committee),
        });
        if pool_size == 1 {
            let registration = net.register(0);
            net.step(vec![registration], None, vec![]);
        }
        let bytes = Arc::new(Mutex::new(vec![]));
        let writer = Log(bytes.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::INFO)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || net.run_to(24 * 36));
        let pool = aether_rewards::draw_pool(&net.parent.state)
            .expect("a frozen draw")
            .1;
        assert_eq!(pool.len(), pool_size);
        let log = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        let message = if pool_size == 0 {
            "empty candidate pool"
        } else {
            "fewer candidates than open seats"
        };
        assert!(
            log.contains(message) && log.contains("INFO") && log.contains("open_seats=3"),
            "the finalized draw logs its shortfall: {log}"
        );
    }
}

#[test]
fn candidate_observability_rpc_counts_a_committed_handoff_before_the_next_draw() {
    let st = state(false);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let target = 24 * 3_600;
    {
        let mut g = st.chain.lock();
        let mut head = (*g.finalized).clone();
        head.handoff = Some(Arc::new(aether_node::handoff::Pending {
            at: head.height,
            switch: head.height + aether_node::handoff::DELAY,
            handoff: aether_light::block::Handoff {
                round: 1,
                output: String::new(),
                signature: String::new(),
                ready: vec![],
                members: (1..=10u8)
                    .map(|i| {
                        (
                            hex::encode([i; 32]),
                            aether_net::SecretKey::from_bytes(&[i; 32])
                                .public()
                                .to_string(),
                        )
                    })
                    .collect(),
            },
        }));
        g.committee.members.clear();
        g.finalized = Arc::new(head);
    }
    let root = st.chain.lock().finalized.state.root();
    let answer = rt.block_on(candidates(&st, "aether_candidates"));
    assert_eq!(
        answer["result"]["open_seats"], 3,
        "the signed roster switches before the next draw"
    );
    {
        let mut g = st.chain.lock();
        let mut head = (*g.finalized).clone();
        let mut pending = (**head.handoff.as_ref().unwrap()).clone();
        pending.switch = target + 1;
        head.handoff = Some(Arc::new(pending));
        g.finalized = Arc::new(head);
    }
    let answer = rt.block_on(candidates(&st, "aether_candidates"));
    assert_eq!(
        answer["result"]["open_seats"], 0,
        "an unknown legacy roster must not be replaced with a stale genesis size"
    );
    assert_eq!(st.chain.lock().finalized.state.root(), root);
}
