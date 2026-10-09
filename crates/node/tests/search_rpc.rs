//! Node discovery RPC and durable projection seams; no network or live keys.
use aether_execution::{Event, Receipt, WorldState};
use aether_node::{
    chain::{Chain, ChainConfig},
    rpc::{self, Finality, RpcState},
    search::{SearchEvent, SearchIndex, SearchMetadata},
    search_events,
    search_sources::{SearchSource, SearchSources},
    store::{Commit, Store},
};
use aether_types::{Address, GasVector, B256, U256};
use alloy_primitives::keccak256;
use serde_json::{json, Value};
use std::sync::Arc;

fn state() -> RpcState {
    let (chain, _) = Chain::new(ChainConfig {
        chain_id: 9911,
        limits: GasVector {
            exec: 30_000_000,
            state: u64::MAX,
            prove: 200_000_000,
        },
        alloc: vec![],
        fees: false,
        registrar: None,
        epoch_blocks: 0,
        min_streak: None,
        draw_epochs: None,
        history_v2: false,
        protocol: 1,
        node_rewards: false,
        committee: vec![],
        reserve: None,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
    });
    let (gossip, _) = tokio::sync::mpsc::unbounded_channel();
    RpcState {
        chain,
        gossip,
        finality: Finality::Archive(Arc::new(aether_node::follow::FinalityArchive::default())),
        faucet: None,
        registrar: None,
        network: None,
        upstream: None,
        handoff: None,
        snapshot: Default::default(),
        prover: None,
        shards: None,
        public_read_only: true,
        presence: None,
        app_bundles: None,
    }
}

fn published(id: &str, name: &str) -> SearchEvent {
    SearchEvent::AppPublished {
        id: id.into(),
        publisher: Address::repeat_byte(1),
        name: name.into(),
        url: format!("sea://{name}/"),
        content_hash: Some(B256::repeat_byte(3)),
        metadata: SearchMetadata {
            title: "Reader".into(),
            description: "Read chain records".into(),
            category: "tools".into(),
            contracts: vec![Address::repeat_byte(4)],
        },
        created_at: 100,
    }
}

async fn call(state: &RpcState, method: &str, params: Value) -> Value {
    rpc::handle_value(
        state,
        json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
    )
    .await
}

#[tokio::test]
async fn search_is_public_read_only_alias_compatible_and_strictly_bounded() {
    let st = state();
    st.chain
        .lock()
        .search
        .lock()
        .unwrap()
        .apply(published("fixture", "reader"));
    let answer = call(&st, "aether_search", json!(["reader", 20])).await;
    assert_eq!(
        answer,
        call(&st, "eastsea_search", json!(["reader", 20])).await
    );
    assert_eq!(answer["result"][0]["name"], "reader");
    assert_eq!(answer["result"][0]["verified"], true);
    assert_eq!(
        answer["result"][0]["publisher"],
        format!("{:#x}", Address::repeat_byte(1))
    );
    assert_eq!(
        call(&st, "aether_search", json!(["", 50])).await["result"],
        json!([])
    );
    for params in [
        json!([]),
        json!([null]),
        json!(["reader", 0]),
        json!(["reader", 51]),
        json!(["reader", "20"]),
        json!(["reader", null]),
        json!(["a".repeat(257)]),
        json!(["line\nbreak"]),
        json!(["reader", 20, true]),
        json!({"query":"reader"}),
    ] {
        assert_eq!(
            call(&st, "aether_search", params.clone()).await["error"]["code"],
            -32602,
            "{params}"
        );
    }
    let info = call(&st, "aether_searchInfo", json!([])).await;
    assert_eq!(info["result"]["sources_configured"], false);
    assert_eq!(info["result"]["records"], 1);
    assert_eq!(info, call(&st, "eastsea_searchInfo", json!([])).await);
}

fn publication_event(source: Address, publisher: Address, name: &str) -> Event {
    let mut data = vec![0; 160];
    data[31] = 160;
    data[32..64].fill(3);
    data[64..96].fill(4);
    data[159] = 224;
    data.extend_from_slice(&aether_types::U256::from(name.len()).to_be_bytes::<32>());
    let mut slug = [0; 32];
    slug[..name.len()].copy_from_slice(name.as_bytes());
    data.extend(slug);
    data.extend([0; 32]); // empty hint
    let mut owner = [0; 32];
    owner[12..].copy_from_slice(publisher.as_slice());
    Event {
        address: source,
        topics: vec![
            keccak256("Published(bytes32,address,string,bytes32,bytes32,address,string)"),
            search_events::app_id(publisher, name),
            B256::from(owner),
        ],
        data: data.into(),
    }
}

fn finalized_receipt(success: bool, events: Vec<Event>) -> Receipt {
    Receipt {
        tx_hash: B256::repeat_byte(8),
        success,
        gas_used: 21_000,
        prove_gas: 0,
        state_gas: 0,
        state_fee: U256::ZERO,
        contract_address: None,
        logs: events.len() as u32,
        output: Default::default(),
        events,
    }
}

#[test]
fn source_address_and_runtime_hash_gate_live_and_replay_events() {
    let source = Address::repeat_byte(1);
    let mut world = WorldState::default();
    world.set_code(source, vec![0x00].into()).unwrap();
    let mut event = publication_event(source, Address::repeat_byte(2), "reader");
    assert!(search_events::decode_from_source(&event, &world, &Default::default(), 100).is_empty());
    let sources = SearchSources {
        app_registries: vec![SearchSource {
            address: source,
            code_hash: world.code_hash(&source),
        }],
        name_services: vec![],
    };
    assert_eq!(
        search_events::decode_from_source(&event, &world, &sources, 100).len(),
        1
    );
    event.address = Address::repeat_byte(5);
    assert!(search_events::decode_from_source(&event, &world, &sources, 100).is_empty());
    event.address = source;
    world
        .set_code(source, vec![0x60, 0x00, 0x00].into())
        .unwrap();
    assert!(search_events::decode_from_source(&event, &world, &sources, 100).is_empty());
}

#[test]
fn finalized_metadata_is_retained_without_inventing_call_activity() {
    let source = Address::repeat_byte(1);
    let mut world = WorldState::default();
    world.set_code(source, vec![0x00].into()).unwrap();
    let sources = SearchSources {
        app_registries: vec![SearchSource { address: source, code_hash: world.code_hash(&source) }],
        name_services: vec![],
    };
    let publication = publication_event(source, Address::repeat_byte(2), "reader");
    let receipt = finalized_receipt(true, vec![publication.clone()]);
    let events = search_events::block_events(&[receipt], &world, &sources, 100);
    assert!(events.iter().any(|event| matches!(event, SearchEvent::UsageIncomplete { at: 100 })));
    assert!(!events.iter().any(|event| matches!(event, SearchEvent::ContractCalled { .. })));
    let index = SearchIndex::rebuild(events);
    let rows = index.search("reader", 50, 100);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "reader");
    assert_eq!(rows[0].url, search_events::app_url(B256::repeat_byte(4)));
    assert!(rows[0].verified, "published content-addressed integrity is retained");
    assert!(!rows[0].usage_complete);
    assert_eq!(rows[0].usage_7d, 0);
    let failed = finalized_receipt(false, vec![publication]);
    assert_eq!(search_events::block_events(&[failed], &world, &sources, 101), vec![SearchEvent::Tick { at: 101 }]);
}

#[test]
fn missing_traces_suppress_cached_usage_and_extend_the_missing_window() {
    let world = WorldState::default();
    let sources = SearchSources::default();
    let mut index = SearchIndex::new();
    index.apply(published("cached-reader", "reader"));
    index.apply(SearchEvent::ContractCalled {
        contract: Address::repeat_byte(4),
        caller: Address::repeat_byte(9),
        at: 101,
    });
    assert_eq!(index.search("reader", 1, 101)[0].usage_7d, 1);
    let success = finalized_receipt(true, vec![]);
    index.apply_batch(&search_events::block_events(&[success.clone()], &world, &sources, 102));
    let row = &index.search("reader", 1, 102)[0];
    assert!(!row.usage_complete);
    assert_eq!(row.usage_7d, 0);
    let window = aether_node::search::USAGE_WINDOW_SECONDS;
    index.apply_batch(&search_events::block_events(&[success], &world, &sources, 101 + window));
    index.advance(102 + window);
    assert!(!index.info().usage_complete, "a later success still has no call trace");
    assert_eq!(index.info().usage_incomplete_until, Some(101 + 2 * window));
    assert_eq!(search_events::block_events(&[], &world, &sources, 103 + window), vec![SearchEvent::Tick { at: 103 + window }]);
}

#[test]
fn bounded_rows_restore_the_same_results_and_bind_sources_and_head() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp")
        .join(format!("search-cache-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("search.redb");
    let mut index = SearchIndex::new();
    index.apply(published("fixture", "reader"));
    index.apply(SearchEvent::ContractCalled {
        contract: Address::repeat_byte(4),
        caller: Address::repeat_byte(5),
        at: 101,
    });
    index.apply(SearchEvent::AppReleaseQueued {
        id: "fixture".into(),
        content_hash: Some(B256::repeat_byte(6)),
        metadata: SearchMetadata::default(),
        activates_at: 200,
    });
    let sources = SearchSources::default().fingerprint();
    {
        let store = Store::open(&path).unwrap();
        store.put_search_index(1, [2; 32], &index, sources).unwrap();
        assert!(store.search_index(2, [2; 32], sources).unwrap().is_none());
        assert!(store.search_index(1, [3; 32], sources).unwrap().is_none());
        assert!(store
            .search_index(1, [2; 32], B256::repeat_byte(7))
            .unwrap()
            .is_none());
    }
    let store = Store::open(&path).unwrap();
    let mut restored = store.search_index(1, [2; 32], sources).unwrap().unwrap();
    assert_eq!(
        index.search("reader", 20, 101),
        restored.search("reader", 20, 101)
    );
    assert_eq!(index.info(), restored.info());
    index.apply(SearchEvent::Tick { at: 200 });
    restored.apply(SearchEvent::Tick { at: 200 });
    assert_eq!(
        index.search("reader", 20, 200),
        restored.search("reader", 20, 200)
    );
    drop(store);
    std::fs::remove_dir_all(directory).unwrap();
}

fn commit_empty_head(
    store: &Store,
    chain: &Chain,
    height: u64,
    timestamp_ms: u64,
    index: Option<(&aether_node::search::SearchDelta, &SearchIndex)>,
) {
    let g = chain.lock();
    let head = &g.finalized;
    let mut summary = g.blocks[&0].clone();
    summary.height = height;
    summary.timestamp_ms = timestamp_ms;
    summary.parent = summary.hash.clone();
    summary.hash = hex::encode([2; 32]);
    let commit = Commit {
        height,
        digest: [2; 32],
        root: head.state.root(),
        diff: head.state.journal(),
        summary: &summary,
        receipts: vec![],
        handoff: None,
        seed: None,
        history: &head.history,
        schedule: &head.schedule,
        upgrade_notices: &[],
        statement: &head.statement,
        staged: None,
    };
    if let Some((delta, index)) = index {
        store
            .commit_with_search(
                commit,
                &[],
                delta,
                index,
                SearchSources::default().fingerprint(),
                false,
            )
            .unwrap();
    } else {
        store.commit(commit).unwrap();
    }
}

#[test]
fn a_snapshot_gap_cannot_claim_complete_event_history_or_a_fake_usage_deadline() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp")
        .join(format!("search-gap-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("state.redb");
    let cfg = state().chain.cfg();
    let (chain, _) = Chain::open(cfg.clone(), Store::open(&path).unwrap()).unwrap();
    let store = chain.store().unwrap();
    commit_empty_head(&store, &chain, 20, 0, None);
    drop(store);
    drop(chain);
    let (chain, _) = Chain::open(cfg, Store::open(&path).unwrap()).unwrap();
    let search = chain.lock().search.clone();
    let mut index = search.lock().unwrap();
    assert!(!index.info().history_complete);
    assert!(index.info().awaiting_clock);
    assert!(!index.info().usage_complete);
    index.apply(SearchEvent::Tick { at: 1_790_000_000 });
    assert!(!index.info().usage_complete);
    assert_eq!(
        index.info().usage_incomplete_until,
        Some(1_790_000_000 + 7 * 86400)
    );
    drop(index);
    drop(search);
    drop(chain);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_first_post_snapshot_commit_replaces_pre_gap_disk_rows() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp")
        .join(format!("search-reset-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("state.redb");
    let cfg = state().chain.cfg();
    let (chain, _) = Chain::open(cfg.clone(), Store::open(&path).unwrap()).unwrap();
    let store = chain.store().unwrap();
    let mut old = SearchIndex::new();
    old.apply(published("stale", "reader"));
    store
        .put_search_index(0, [1; 32], &old, SearchSources::default().fingerprint())
        .unwrap();
    let mut fresh = SearchIndex::new();
    fresh.mark_history_incomplete();
    let delta = fresh.apply_batch(&[SearchEvent::Tick { at: 1_790_000_000 }]);
    commit_empty_head(
        &store,
        &chain,
        21,
        1_790_000_000_000,
        Some((&delta, &fresh)),
    );
    drop(store);
    drop(chain);
    let (chain, _) = Chain::open(cfg, Store::open(&path).unwrap()).unwrap();
    let search = chain.lock().search.clone();
    let index = search.lock().unwrap();
    assert!(
        index.is_empty(),
        "pre-gap records must never resurrect under a new header"
    );
    assert!(!index.info().history_complete);
    assert!(!index.info().usage_complete);
    drop(index);
    drop(search);
    drop(chain);
    std::fs::remove_dir_all(directory).unwrap();
}
