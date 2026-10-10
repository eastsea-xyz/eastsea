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
        presence: None,
        public_read_only: false,
        app_bundles: None,
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
    "aether_peers",
    "aether_presence",
    "aether_proverProgram",
    "aether_proverStatus",
    "aether_reattest",
    "aether_recentBlocks",
    "aether_registrarEncryptionKey",
    "aether_registerDevice",
    "aether_registrationNonce",
    "aether_releaseEntries",
    "aether_rewards",
    "aether_rewardStatus",
    "aether_rotation",
    "aether_sendBeacon",
    "aether_sendRegistration",
    "aether_sendTransaction",
    "aether_setPresenceCountry",
    "aether_setPresenceRegion",
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
        let mut old = rt.block_on(call(&st, m, json!([])));
        // The old spelling keeps working: any answer is fine except not-found
        // (some methods legitimately answer -32601 "this node does not ...").
        let not_found = |v: &Value| {
            v.get("error")
                .and_then(|e| e["message"].as_str())
                .is_some_and(|msg| msg.contains("method not found"))
        };
        assert!(!not_found(&old), "{m} must keep answering under its aether_ name");
        let mut new = rt.block_on(call(&st, &format!("eastsea_{}", &m["aether_".len()..]), json!([])));
        assert!(!not_found(&new), "{m} must answer under its eastsea_ name");
        if *m == "aether_presence" {
            // Separate observations may cross a ten-minute release boundary.
            for answer in [&mut old, &mut new] {
                let observed_at = answer["result"].as_object_mut().unwrap().remove("observed_at").unwrap();
                assert!(observed_at.as_u64().is_some_and(|t| t > 0 && t % 600 == 0));
            }
        }
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

#[test]
fn offline_presence_and_peers_keep_a_stable_privacy_safe_rpc_shape() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let st = state();
    let peers = rt.block_on(call(&st, "aether_peers", json!([])));
    assert_eq!(peers["result"], json!([]));

    let answer = rt.block_on(call(&st, "aether_presence", json!([])));
    let mut result = answer["result"].clone();
    let observed_at = result.as_object_mut().expect("presence is an object").remove("observed_at").expect("observation timestamp");
    assert!(observed_at.as_u64().is_some_and(|t| t > 0 && t % 600 == 0));
    assert_eq!(result, json!({
        "schema": 2,
        "available": false,
        "total": null,
        "by_role": {},
        "by_version": {},
        "by_region": {},
        "ttl_seconds": 600,
        "minimum_bucket_size": 3,
        "scope": "unverified cohort observation",
    }));
}

#[test]
fn public_gateway_allows_presence_but_refuses_peers_and_country_settings() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut st = state();
    st.public_read_only = true;
    for method in ["aether_presence", "eastsea_presence"] {
        let answer = rt.block_on(call(&st, method, json!([])));
        assert!(answer.get("result").is_some(), "{method} must pass the public gateway: {answer}");
        assert_eq!(answer["result"]["available"], false);
    }
    for method in ["aether_peers", "eastsea_peers", "aether_setPresenceCountry", "eastsea_setPresenceCountry"] {
        let answer = rt.block_on(call(&st, method, json!([null])));
        assert_eq!(answer["error"]["code"], -32601, "{method} must be refused: {answer}");
        assert!(answer["error"]["message"].as_str().unwrap().contains("public read-only gateway"));
    }
}

fn assert_country_is_local_only(answer: &Value) {
    assert_eq!(answer["error"]["code"], -32601, "country preferences must stay local: {answer}");
    assert!(answer["error"]["message"].as_str().unwrap().contains("local-only"));
}

#[test]
fn remote_country_settings_are_refused_before_presence_lookup() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let st = state();
    rt.block_on(async {
        for method in ["aether_setPresenceCountry", "eastsea_setPresenceCountry"] {
            let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": ["KR"] });
            let native = rpc::handle_value(&st, request.clone()).await;
            assert_eq!(native["error"]["code"], -32000, "native requests reach the missing-endpoint check: {native}");
            assert!(native["error"]["message"].as_str().unwrap().contains("no iroh presence endpoint"));
            let remote = rpc::handle_remote_value(&st, request).await;
            assert_country_is_local_only(&remote);
        }
        let batch = json!([
            { "jsonrpc": "2.0", "id": 11, "method": "aether_setPresenceCountry", "params": ["KR"] },
            { "jsonrpc": "2.0", "id": 12, "method": "eastsea_setPresenceCountry", "params": [null] },
            { "jsonrpc": "2.0", "id": 13, "method": "aether_presence", "params": [] },
        ]);
        let remote = rpc::handle_remote_value(&st, batch).await;
        let answers = remote.as_array().expect("remote batch answers");
        assert_eq!(answers.len(), 3);
        assert_country_is_local_only(&answers[0]);
        assert_country_is_local_only(&answers[1]);
        assert_eq!(answers[0]["id"], 11);
        assert_eq!(answers[1]["id"], 12);
        assert_eq!(answers[2]["id"], 13);
        assert_eq!(answers[2]["result"]["available"], false);
    });
}

#[test]
fn peer_diagnostics_require_a_native_local_request_for_both_aliases_and_batches() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let st = state();
    rt.block_on(async {
        for method in ["aether_peers", "eastsea_peers"] {
            let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": [] });
            assert_eq!(rpc::handle_value(&st, request.clone()).await["result"], json!([]));
            let remote = rpc::handle_remote_value(&st, request).await;
            assert_eq!(remote["error"]["code"], -32601);
            assert!(remote["error"]["message"].as_str().unwrap().contains("local-only"));
            assert!(remote.get("result").is_none());
        }
        let answers = rpc::handle_remote_value(&st, json!([
            { "jsonrpc": "2.0", "id": 1, "method": "aether_peers", "params": [] },
            { "jsonrpc": "2.0", "id": 2, "method": "eastsea_peers", "params": [] },
            { "jsonrpc": "2.0", "id": 3, "method": "eastsea_presence", "params": [] }
        ])).await;
        assert_eq!(answers[0]["error"]["code"], -32601);
        assert_eq!(answers[1]["error"]["code"], -32601);
        assert!(answers[2].get("result").is_some());

        let app = rpc::http_router(st);
        for headers in [vec![("origin", "https://example.test")], vec![("sec-fetch-site", "same-origin")]] {
            let answer = http_call(app.clone(), json!({ "jsonrpc": "2.0", "id": 1, "method": "eastsea_peers", "params": [] }), &headers).await;
            assert_eq!(answer["error"]["code"], -32601);
            assert!(answer.get("result").is_none());
        }
    });
}

async fn http_call(app: axum::Router, request: Value, headers: &[(&str, &str)]) -> Value {
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let mut builder = Request::builder().method("POST").uri("/").header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = app.oneshot(builder.body(Body::from(request.to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).expect("HTTP JSON-RPC response")
}

#[test]
fn browser_headers_cannot_change_country_via_single_calls_aliases_or_batches() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let app = rpc::http_router(state());
    rt.block_on(async {
        for method in ["aether_setPresenceCountry", "eastsea_setPresenceCountry"] {
            let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": ["KR"] });
            let native = http_call(app.clone(), request, &[]).await;
            assert_eq!(native["error"]["code"], -32000, "native HTTP reaches the endpoint check: {native}");
            assert!(native["error"]["message"].as_str().unwrap().contains("no iroh presence endpoint"));
        }
        let browser_headers: &[&[(&str, &str)]] = &[
            &[("origin", "https://example.test")],
            &[("origin", "null")],
            &[("sec-fetch-site", "cross-site")],
            &[("sec-fetch-site", "same-origin")],
            &[("sec-fetch-site", "none")],
        ];
        for headers in browser_headers {
            for method in ["aether_setPresenceCountry", "eastsea_setPresenceCountry"] {
                let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": ["KR"] });
                let answer = http_call(app.clone(), request, headers).await;
                assert_country_is_local_only(&answer);
            }
            let batch = json!([
                { "jsonrpc": "2.0", "id": 21, "method": "aether_setPresenceCountry", "params": ["KR"] },
                { "jsonrpc": "2.0", "id": 22, "method": "eastsea_setPresenceCountry", "params": [null] },
                { "jsonrpc": "2.0", "id": 23, "method": "eastsea_presence", "params": [] },
            ]);
            let answer = http_call(app.clone(), batch, headers).await;
            let answers = answer.as_array().expect("HTTP batch answers");
            assert_eq!(answers.len(), 3);
            assert_country_is_local_only(&answers[0]);
            assert_country_is_local_only(&answers[1]);
            assert_eq!(answers[0]["id"], 21);
            assert_eq!(answers[1]["id"], 22);
            assert_eq!(answers[2]["id"], 23);
            assert_eq!(answers[2]["result"]["available"], false);
        }
    });
}

#[test]
fn region_preference_is_native_local_for_both_aliases_and_batches() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let st = state();
    rt.block_on(async {
        for method in ["aether_setPresenceRegion", "eastsea_setPresenceRegion"] {
            let request = json!({"jsonrpc":"2.0","id":17,"method":method,"params":["030"]});
            assert_eq!(rpc::handle_value(&st, request.clone()).await["error"]["code"], -32000);
            let remote = rpc::handle_remote_value(&st, request).await;
            assert_eq!(remote["error"]["code"], -32601);
            assert!(remote["error"]["message"].as_str().unwrap().contains("local-only"));
        }
        for header in ["origin", "sec-fetch-site"] {
            use tower::ServiceExt as _;
            let request = axum::http::Request::builder().method("POST").uri("/")
                .header("content-type", "application/json").header(header, "https://example.invalid")
                .body(axum::body::Body::from(serde_json::to_vec(&json!({
                    "id":21,"method":"eastsea_setPresenceRegion","params":["030"]
                })).unwrap())).unwrap();
            let response = rpc::http_router(st.clone()).oneshot(request).await.unwrap();
            let body = axum::body::to_bytes(response.into_body(), rpc::PUBLIC_MAX_BODY).await.unwrap();
            let value: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(value["error"]["code"], -32601, "browser header {header} cannot mutate region");
            assert!(value["error"]["message"].as_str().unwrap().contains("local-only"));
        }
        let response = rpc::handle_remote_value(&st, json!([
            {"id":1,"method":"aether_setPresenceRegion","params":["030"]},
            {"id":2,"method":"eastsea_setPresenceRegion","params":[null]},
            {"id":3,"method":"aether_presence","params":[]}
        ])).await;
        assert_eq!(response[0]["id"],1);
        assert_eq!(response[1]["id"],2);
        assert_eq!(response[0]["error"]["code"],-32601);
        assert_eq!(response[1]["error"]["code"],-32601);
        assert_eq!(response[2]["result"]["available"],false);
    });
}
