//! U1 devnet fixture: permissionless publication is discovery, never approval.
//! These tests use only local finalized execution and public RPC surfaces.

use aether_crypto::{P256Signer, Signer as _};
use aether_execution::{sign_call_with, EvmCall};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{build_payload, dev_seed, Chain, ChainConfig, Extras};
use aether_node::rpc::{handle_value, Finality, RpcState};
use aether_node::store::Store;
use aether_types::{Address, Bytes, FeeVector, GasVector, U256};
use base64::Engine as _;
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible, Signer};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use std::sync::Arc;

const CHAIN: u64 = 9_134;
const PUBLISHED_AT: u64 = 1_700_000_000;
const WAIT: u64 = 72 * 60 * 60;

fn sender() -> Address {
    aether_crypto::address_of(&P256Signer::from_seed(&dev_seed(1)).unwrap().public_key()).unwrap()
}

fn config() -> ChainConfig {
    ChainConfig {
        chain_id: CHAIN,
        limits: GasVector {
            exec: 30_000_000,
            state: u64::MAX,
            prove: 200_000_000,
        },
        alloc: vec![(sender(), U256::from(10u128.pow(24)))],
        fees: false,
        registrar: None,
        epoch_blocks: 0,
        min_streak: None,
        draw_epochs: None,
        history_v2: true,
        protocol: 1,
        node_rewards: true,
        committee: vec![],
        reserve: None,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
    }
}

fn builders() -> Vec<String> {
    (11..=13)
        .map(|n| {
            let signer = P256Signer::from_seed(&dev_seed(n)).unwrap();
            let (x, y) = aether_crypto::p256_xy(&signer.public_key().bytes).unwrap();
            hex::encode([vec![4], x.to_vec(), y.to_vec()].concat())
        })
        .collect()
}

fn network() -> Value {
    json!({ "chain_id": CHAIN, "release": aether_node::roster::ReleasePin::for_builders(&builders()).unwrap() })
}

fn manifest(build: &str, emergency: bool) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "chain_id": CHAIN, "log_address": format!("{:#x}", aether_execution::release_log::ADDRESS),
        "platform": "macos-arm64-dmg", "version": "0.7.4", "build": build,
        "emergency": emergency, "sparkle_ed_signature": base64::engine::general_purpose::STANDARD.encode([7; 64]),
        "artifacts": [{ "name": "EastSea.dmg", "sha256": "ab".repeat(32), "size": 64,
            "url": "https://releases.example/EastSea.dmg" }],
    })).unwrap()
}

fn signatures(manifest: &[u8], count: usize) -> Vec<u8> {
    let keys = builders();
    serde_json::to_vec(&(11..=13).take(count).enumerate().map(|(i, n)| {
        let signer = P256Signer::from_seed(&dev_seed(n)).unwrap();
        json!({ "public_key": keys[i], "signature": hex::encode(signer.sign(manifest).unwrap()) })
    }).collect::<Vec<_>>()).unwrap()
}

fn word(n: u64) -> Vec<u8> {
    U256::from(n).to_be_bytes::<32>().to_vec()
}

fn dynamic(bytes: &[u8]) -> Vec<u8> {
    let mut out = word(bytes.len() as u64);
    out.extend_from_slice(bytes);
    out.resize(out.len().next_multiple_of(32), 0);
    out
}

fn calldata(manifest: &[u8], signatures: &[u8], emergency: bool, archive: [u8; 32]) -> Bytes {
    let m = dynamic(manifest);
    let s = dynamic(signatures);
    let selector = alloy_primitives::keccak256(b"publish(bytes,bytes32,bytes,bool)");
    let mut out = selector[..4].to_vec();
    out.extend(word(128));
    out.extend(archive);
    out.extend(word(128 + m.len() as u64));
    out.extend(word(u64::from(emergency)));
    out.extend(m);
    out.extend(s);
    out.into()
}

struct Fixture {
    state: RpcState,
    head: Block,
    nonce: u64,
}

impl Fixture {
    fn new() -> Self {
        let (chain, head) = Chain::new(config());
        Self::with(chain, head)
    }

    fn with(chain: Chain, head: Block) -> Self {
        let (gossip, _) = tokio::sync::mpsc::unbounded_channel();
        Self {
            state: RpcState {
                chain,
                app_bundles: None,
                finality: Finality::Archive(Arc::new(
                    aether_node::follow::FinalityArchive::default(),
                )),
                gossip,
                faucet: None,
                registrar: None,
                network: Some(network()),
                upstream: None,
                handoff: None,
                snapshot: Default::default(),
                prover: None,
                shards: None,
                public_read_only: false,
                presence: None,
            },
            head,
            nonce: 0,
        }
    }

    async fn rpc(&self, method: &str, params: Value) -> Value {
        handle_value(
            &self.state,
            json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
        )
        .await
    }

    async fn status(&self) -> Value {
        self.rpc("aether_status", json!([])).await["result"].clone()
    }

    async fn release(&self) -> Value {
        let status = self.status().await;
        assert!(
            status.get("release").is_some(),
            "aether_status must state whether a verified chain release exists"
        );
        status["release"].clone()
    }

    fn block(&self, input: Option<Bytes>, seconds: u64) -> Block {
        let parent = self.state.chain.lock().finalized.clone();
        let height = self.head.height.next();
        let leader = ed25519::PrivateKey::from_seed(1).public_key();
        let context = Context {
            round: Round::new(EPOCH, View::new(height.get())),
            leader,
            parent: (View::new(height.get() - 1), self.head.digest()),
        };
        let skeleton = Block::new(
            context.clone(),
            self.head.digest(),
            height,
            seconds * 1_000,
            bytes::Bytes::new(),
        );
        let ctx = Chain::block_context(&self.state.chain.cfg(), &skeleton, &parent);
        let txs = input
            .map(|input| {
                let signer = P256Signer::from_seed(&dev_seed(1)).unwrap();
                let mut tx = sign_call_with(
                    &signer,
                    CHAIN,
                    self.nonce,
                    FeeVector {
                        exec: 0,
                        state: 1_000_000_000_000_000,
                        prove: 0,
                    },
                    0,
                    &EvmCall {
                        to: Some(aether_execution::release_log::ADDRESS),
                        value: U256::ZERO,
                        input,
                        gas_limit: 2_000_000,
                        delegate: None,
                    },
                )
                .unwrap();
                tx.header.gas.state = aether_execution::fees::MAX_STATE_UNITS_PER_BLOCK;
                let mut signature = signer.sign(&tx.signing_bytes()).unwrap();
                signature.extend_from_slice(&signer.public_key().bytes);
                tx.signature = signature.into();
                tx
            })
            .into_iter()
            .collect();
        let (pre, _) = self
            .state
            .chain
            .pre_state(&parent, parent.next_protocol(), &[], None, false)
            .unwrap();
        let (payload, _) = build_payload(&parent, &pre, &ctx, txs, Extras::default());
        Block::new(
            context,
            self.head.digest(),
            height,
            seconds * 1_000,
            payload.to_bytes().into(),
        )
    }

    fn finalize(&mut self, block: Block) {
        self.state.chain.finalize(&block).unwrap();
        self.head = block;
    }

    fn publish(&mut self, manifest: &[u8], sigs: &[u8], emergency: bool, archive: [u8; 32]) {
        let block = self.block(
            Some(calldata(manifest, sigs, emergency, archive)),
            PUBLISHED_AT + self.head.height.get(),
        );
        self.finalize(block);
        assert!(
            self.state.chain.lock().finalized.receipts[0].success,
            "fixture publication executes"
        );
        self.nonce += 1;
    }

    fn certify(&mut self, seconds: u64) {
        let block = self.block(None, seconds);
        self.finalize(block);
    }
}

#[tokio::test]
async fn published_payload_requires_finalization_and_next_certified_state() {
    let mut f = Fixture::new();
    assert!(f.release().await.is_null());
    let m = manifest("74", false);
    let s = signatures(&m, 2);
    let block = f.block(Some(calldata(&m, &s, false, [0xab; 32])), PUBLISHED_AT);
    let parent = f.state.chain.lock().finalized.clone();
    f.state.chain.execute(&block, &parent).unwrap();
    assert!(
        f.release().await.is_null(),
        "an executed proposal is not finalized discovery"
    );
    f.finalize(block);
    assert!(
        f.release().await.is_null(),
        "the entry's post-state is certified by its next block"
    );
    f.certify(PUBLISHED_AT + 1);
    let r = f.release().await;
    assert_eq!(r["version"], "0.7.4");
    assert_eq!(r["build"], "74");
    assert_eq!(r["index"], 0);
    assert_eq!(r["chain_id"], CHAIN);
    assert_eq!(r["approved_at_height"], 1);
    assert_eq!(r["state_height"], 1);
    assert_eq!(r["certified_timestamp_ms"], (PUBLISHED_AT + 1) * 1_000);
    assert_eq!(r["manifest_hash"], hex::encode(Sha256::digest(&m)));
    assert_eq!(r["archive_sha256"], "ab".repeat(32));
    assert_eq!(r["signatures_hash"], hex::encode(Sha256::digest(&s)));
    assert_eq!(r["manifest"], String::from_utf8(m).unwrap());
    assert_eq!(r["signatures"], String::from_utf8(s).unwrap());
    assert_eq!(r["approvals"], 2);
    assert_eq!(r["required_approvals"], 2);
    assert_eq!(r["install_after_height"], 1 + WAIT);
    assert_eq!(
        r["ready"], false,
        "a release is announced while its install wait remains in force"
    );
    assert_eq!(r["restart_slot"]["slot_blocks"], 600);
}

#[tokio::test]
async fn forged_payload_cannot_replace_an_approved_release() {
    let mut f = Fixture::new();
    let _ = f.status().await;
    let m = manifest("74", false);
    f.publish(&m, &signatures(&m, 2), false, [0xab; 32]);
    f.certify(PUBLISHED_AT + 1);
    let original = f.release().await;
    assert_eq!(original["index"], 0);
    let forged = manifest("75", false);
    f.publish(&forged, &signatures(&m, 2), false, [0xab; 32]);
    f.certify(PUBLISHED_AT + 3);
    assert_eq!(
        f.release().await["manifest_hash"],
        original["manifest_hash"]
    );
}

#[tokio::test]
async fn insufficient_or_duplicate_builder_signatures_are_not_approval() {
    let mut f = Fixture::new();
    let _ = f.status().await;
    let m = manifest("74", false);
    f.publish(&m, &signatures(&m, 1), false, [0xab; 32]);
    f.certify(PUBLISHED_AT + 1);
    assert!(f.release().await.is_null());
    let one: Value = serde_json::from_slice(&signatures(&m, 1)).unwrap();
    let duplicated = serde_json::to_vec(&json!([one[0], one[0]])).unwrap();
    f.publish(&m, &duplicated, false, [0xab; 32]);
    f.certify(PUBLISHED_AT + 3);
    assert!(f.release().await.is_null());
}

#[tokio::test]
async fn archive_hash_and_pinned_runtime_must_match_the_finalized_entry() {
    let mut f = Fixture::new();
    let _ = f.status().await;
    let m = manifest("74", false);
    f.publish(&m, &signatures(&m, 2), false, [0xcd; 32]);
    f.certify(PUBLISHED_AT + 1);
    assert!(f.release().await.is_null());
    f.state.network.as_mut().unwrap()["release"]["code_hash"] =
        json!(format!("0x{}", "ef".repeat(32)));
    f.publish(&m, &signatures(&m, 2), false, [0xab; 32]);
    f.certify(PUBLISHED_AT + 3);
    assert!(
        f.release().await.is_null(),
        "a usable key pin cannot override a wrong code pin"
    );
}

#[tokio::test]
async fn emergency_needs_three_builders_and_keeps_both_seventy_two_hour_barriers() {
    let mut f = Fixture::new();
    let _ = f.status().await;
    let m = manifest("74", true);
    f.publish(&m, &signatures(&m, 2), true, [0xab; 32]);
    f.certify(PUBLISHED_AT + 1);
    assert!(f.release().await.is_null());
    f.publish(&m, &signatures(&m, 3), true, [0xab; 32]);
    f.certify(PUBLISHED_AT + 3);
    let r = f.release().await;
    assert_eq!(r["emergency"], true);
    assert_eq!(r["required_approvals"], 3);
    assert_eq!(r["install_after_height"], 3 + WAIT);
    assert_eq!(
        r["install_after_timestamp_ms"],
        (PUBLISHED_AT + 2 + WAIT) * 1_000
    );
    assert_eq!(r["ready"], false);
}

#[tokio::test]
async fn malformed_or_absent_pin_fails_closed_and_legacy_stays_null() {
    let mut f = Fixture::new();
    f.state.network = Some(json!({ "chain_id": CHAIN, "release": { "builder_keys": [] } }));
    let m = manifest("74", false);
    f.publish(&m, &signatures(&m, 2), false, [0xab; 32]);
    f.certify(PUBLISHED_AT + 1);
    assert!(f.release().await.is_null());
    f.state.network = Some(json!({ "chain_id": 7_780 }));
    assert!(f.release().await.is_null());
    f.state.network = None;
    assert!(f.release().await.is_null());
    let mut cfg = config();
    cfg.chain_id = 7_780;
    cfg.node_rewards = false;
    cfg.history_v2 = false;
    let (chain, genesis) = Chain::new(cfg);
    assert!(
        chain
            .lock()
            .finalized
            .state
            .code(&aether_execution::release_log::ADDRESS)
            .is_empty(),
        "the historical 7780 genesis contains no ReleaseLog predeploy"
    );
    let mut legacy = Fixture::with(chain, genesis);
    legacy.state.network =
        Some(serde_json::from_str(include_str!("fixtures/legacy-7780-network.json")).unwrap());
    assert!(legacy.release().await.is_null());
}

#[tokio::test]
async fn restored_payload_is_reverified_and_survives_receipt_cache_eviction() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp")
        .join(format!("release-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let (chain, genesis) =
        Chain::open(config(), Store::open(&root.join("state.redb")).unwrap()).unwrap();
    let mut f = Fixture::with(chain, genesis);
    let _ = f.status().await;
    let m = manifest("74", false);
    let s = signatures(&m, 2);
    f.publish(&m, &s, false, [0xab; 32]);
    f.certify(PUBLISHED_AT + 1);
    let before = f.release().await;
    assert_eq!(before["index"], 0);
    let head = f.head.clone();
    drop(f);
    let (chain, _) = Chain::open(config(), Store::open(&root.join("state.redb")).unwrap()).unwrap();
    chain.lock().receipts.clear();
    let mut f = Fixture::with(chain, head);
    let _ = f.status().await;
    f.certify(PUBLISHED_AT + 2);
    let after = f.release().await;
    assert_eq!(after["manifest"], before["manifest"]);
    assert_eq!(after["signatures"], before["signatures"]);
    drop(f);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn restart_slot_query_refuses_unknown_seats() {
    let f = Fixture::new();
    let reply = f.rpc("aether_restartSlot", json!(["ab".repeat(32)])).await;
    assert!(
        reply.get("error").is_none(),
        "the chain exposes a read-only restart-slot decision"
    );
    assert!(reply["result"]["seat_index"].is_null());
    assert_eq!(reply["result"]["allowed"], false);
    assert_eq!(reply["result"]["slot_blocks"], 600);
}

fn restart_fixture(height: u64) -> (Fixture, Vec<String>) {
    let f = Fixture::new();
    let keys: Vec<_> = (1..=4)
        .map(|n| ed25519::PrivateKey::from_seed(n).public_key())
        .collect();
    let key_hex: Vec<_> = keys.iter().map(|k| hex::encode(k.as_ref())).collect();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let mut g = f.state.chain.lock();
    g.committee.members = key_hex.iter().map(|k| (k.clone(), String::new())).collect();
    let template = g.blocks[&0].clone();
    g.blocks.clear();
    for h in height.saturating_sub(15)..=height {
        let mut block = template.clone();
        block.height = h;
        block.timestamp_ms = now_ms.saturating_sub((height - h) * 1_000);
        block.proposer = aether_node::chain::leader_address(&keys[h as usize % 4]);
        g.blocks.insert(h, block);
    }
    Arc::make_mut(&mut g.finalized).height = height;
    Arc::make_mut(&mut g.finalized).timestamp = now_ms;
    g.net_height = Some(height);
    drop(g);
    (f, key_hex)
}

#[tokio::test]
async fn restart_slot_requires_the_named_active_seat_and_a_safe_boundary() {
    let (f, keys) = restart_fixture(300);
    let slot = f.rpc("aether_restartSlot", json!([keys[0]])).await;
    assert!(
        slot.get("error").is_none(),
        "restart timing comes from the chain"
    );
    assert_eq!(slot["result"]["seat_index"], 0);
    assert_eq!(slot["result"]["distinct_proposers"], 4);
    assert_eq!(slot["result"]["allowed"], true);
    let other = f.rpc("aether_restartSlot", json!([keys[1]])).await;
    assert_eq!(other["result"]["allowed"], false);
    assert_eq!(other["result"]["next_slot_height"], 600);
    let (edge, keys) = restart_fixture(573);
    let boundary = edge.rpc("aether_restartSlot", json!([keys[0]])).await;
    assert_eq!(boundary["result"]["in_slot"], true);
    assert_eq!(boundary["result"]["slot_end_height"], 600);
    assert_eq!(
        boundary["result"]["allowed"], false,
        "a short permission cannot cross into another seat's slot"
    );
}

#[tokio::test]
async fn restart_lease_reserves_healthy_block_age_and_timestamp_skew() {
    let (f, keys) = restart_fixture(580);
    {
        let mut g = f.state.chain.lock();
        Arc::make_mut(&mut g.finalized).timestamp -= 9_000;
    }
    let boundary = f.rpc("aether_restartSlot", json!([keys[0]])).await;
    assert!(boundary.get("error").is_none());
    assert_eq!(boundary["result"]["healthy"], true);
    assert_eq!(boundary["result"]["in_slot"], true);
    assert_eq!(boundary["result"]["distinct_proposers"], 4);
    assert_eq!(
        boundary["result"]["allowed"], false,
        "a healthy but old block cannot grant a lease across the next seat's window"
    );
}

#[tokio::test]
async fn legacy_restart_slots_without_the_timestamp_floor_fail_closed() {
    let (f, keys) = restart_fixture(300);
    {
        let mut g = f.state.chain.lock();
        g.cfg.node_rewards = false;
        g.cfg.history_v2 = false;
    }
    let slot = f.rpc("aether_restartSlot", json!([keys[0]])).await;
    assert!(slot.get("error").is_none());
    assert_eq!(slot["result"]["healthy"], true);
    assert_eq!(slot["result"]["in_slot"], true);
    assert_eq!(
        slot["result"]["allowed"], false,
        "legacy consensus has no one-second block floor to bound the wallet lease"
    );
}

#[tokio::test]
async fn retired_proposers_and_stale_blocks_do_not_authorize_a_restart() {
    let (f, keys) = restart_fixture(300);
    f.state.chain.lock().committee.members[3].0 =
        hex::encode(ed25519::PrivateKey::from_seed(9).public_key().as_ref());
    let retired = f.rpc("aether_restartSlot", json!([keys[0]])).await;
    assert!(
        retired.get("error").is_none(),
        "restart timing comes from the active committee"
    );
    assert_eq!(retired["result"]["distinct_proposers"], 3);
    assert_eq!(retired["result"]["allowed"], false);
    let (stale, keys) = restart_fixture(300);
    let mut g = stale.state.chain.lock();
    Arc::make_mut(&mut g.finalized).timestamp -= 11_000;
    drop(g);
    let slot = stale.rpc("aether_restartSlot", json!([keys[0]])).await;
    assert_eq!(slot["result"]["healthy"], false);
    assert_eq!(slot["result"]["allowed"], false);
}

#[tokio::test]
async fn signed_install_barriers_can_extend_but_never_shorten_the_wait() {
    let mut f = Fixture::new();
    let _ = f.status().await;
    let mut extended: Value = serde_json::from_slice(&manifest("74", false)).unwrap();
    extended["install_after_height"] = json!(1);
    extended["restart_slot_height"] = json!(1 + WAIT + 600);
    let m = serde_json::to_vec(&extended).unwrap();
    f.publish(&m, &signatures(&m, 2), false, [0xab; 32]);
    f.certify(PUBLISHED_AT + WAIT + 1);
    let r = f.release().await;
    assert_eq!(r["install_after_height"], 1 + WAIT + 600);
    assert_eq!(
        r["ready"], false,
        "elapsed time alone does not bypass the signed slot height"
    );
}

#[tokio::test]
async fn unsupported_manifest_or_signature_payload_is_ignored_with_bounded_cache() {
    let mut f = Fixture::new();
    let _ = f.status().await;
    let mut unsupported: Value = serde_json::from_slice(&manifest("74", false)).unwrap();
    unsupported["platform"] = json!("forged-platform");
    let m = serde_json::to_vec(&unsupported).unwrap();
    f.publish(&m, &signatures(&m, 2), false, [0xab; 32]);
    f.certify(PUBLISHED_AT + 1);
    assert!(f.release().await.is_null());
    let m = manifest("75", false);
    f.publish(&m, &[b'x'; 4_096], false, [0xab; 32]);
    f.certify(PUBLISHED_AT + 3);
    assert!(
        f.release().await.is_null(),
        "the largest allowed unapproved contract payload is still ignored"
    );
}
