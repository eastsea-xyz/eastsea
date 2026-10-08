//! `eastsea_*` RPC aliases (docs/design/25-rename.md phase 4): the node keeps
//! answering every `aether_*` method exactly as before, and the same methods
//! under the project's new `eastsea_` prefix with byte-identical results.
//! An unknown method stays unknown under the new prefix; `eth_*` methods are
//! untouched.

use aether_node::chain::{Chain, ChainConfig};
use aether_node::rpc::{self, Finality, RpcState};
use aether_types::GasVector;
use serde_json::{json, Value};
use std::sync::Arc;

const ADDR: &str = "0x0000000000000000000000000000000000000001";

fn state() -> RpcState {
    let (chain, _) = Chain::new(ChainConfig {
        chain_id: 7781,
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
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

async fn call(st: &RpcState, method: &str, params: Value) -> Value {
    rpc::handle_value(st, json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })).await
}

/// Every `aether_*` method the node dispatches on (kept in one place here so a
/// newly added method fails this test until it is listed — and thereby also
/// proven to answer under both spellings).
const METHODS: &[&str] = &[
    "aether_accountHistory",
    "aether_candidates",
    "aether_eraChunk",
    "aether_eraInfo",
    "aether_eraProof",
    "aether_faucet",
    "aether_getAccount",
    "aether_getBlock",
    "aether_getCodeHash",
    "aether_getFinalized",
    "aether_getReceipt",
    "aether_getStorage",
    "aether_handoff",
    "aether_history",
    "aether_historyProof",
    "aether_network",
    "aether_proverProgram",
    "aether_proverStatus",
    "aether_reattest",
    "aether_recentBlocks",
    "aether_registerDevice",
    "aether_registrationNonce",
    "aether_releaseEntries",
    "aether_rewards",
    "aether_rewardStatus",
    "aether_rotation",
    "aether_sendBeacon",
    "aether_sendRegistration",
    "aether_sendTransaction",
    "aether_search",
    "aether_searchInfo",
    "aether_shard",
    "aether_shardStats",
    "aether_signHandoff",
    "aether_snapshot",
    "aether_snapshotChunk",
    "aether_status",
    "aether_submitProof",
];

#[test]
fn every_method_answers_identically_under_both_prefixes() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let st = state();
    for m in METHODS {
        let old = rt.block_on(call(&st, m, json!([])));
        // The old spelling keeps working: any answer is fine except not-found
        // (some methods legitimately answer -32601 "this node does not ...").
        let not_found = |v: &Value| {
            v.get("error")
                .and_then(|e| e["message"].as_str())
                .is_some_and(|msg| msg.contains("method not found"))
        };
        assert!(!not_found(&old), "{m} must keep answering under its aether_ name");
        let new = rt.block_on(call(&st, &format!("eastsea_{}", &m["aether_".len()..]), json!([])));
        assert!(!not_found(&new), "{m} must answer under its eastsea_ name");
        assert_eq!(old, new, "{m}: results must be identical under both prefixes");
    }
}

#[test]
fn sampled_methods_return_real_results_under_eastsea_prefix() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let st = state();
    let samples: &[(&str, Value)] = &[
        ("aether_status", json!([])),
        ("aether_network", json!([])),
        ("aether_recentBlocks", json!([5])),
        ("aether_history", json!([])),
        ("aether_getAccount", json!([ADDR])),
        ("aether_getStorage", json!([ADDR, "0x0"])),
    ];
    for (m, params) in samples {
        let old = rt.block_on(call(&st, m, params.clone()));
        assert!(old.get("result").is_some(), "{m}: expected a result, got {old}");
        let new = rt.block_on(call(&st, &format!("eastsea_{}", &m["aether_".len()..]), params.clone()));
        assert!(new.get("result").is_some(), "{m} via eastsea_: expected a result, got {new}");
        assert_eq!(old, new);
    }
}

#[test]
fn unknown_eastsea_method_is_not_found_and_eth_is_untouched() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let st = state();
    let missing = rt.block_on(call(&st, "eastsea_doesNotExist", json!([])));
    assert_eq!(missing["error"]["code"], -32601);
    assert!(missing["error"]["message"].as_str().unwrap().contains("method not found"));
    // The renamed family is only `eastsea_*`; other families are not aliased.
    assert_eq!(rt.block_on(call(&st, "eth_chainId", json!([])))["result"], "0x1e65");
    let no_alias = rt.block_on(call(&st, "eastsea_chainId", json!([])));
    assert_eq!(no_alias["error"]["code"], -32601);
}
