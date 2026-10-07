//! Fixed, locally signed artifacts for wallet upgrade finality regressions.

use aether_hash::{ChainHasher, Hasher};
use aether_light::block::{Block, Context, Payload, Release, SignedUpgrade, Upgrade, EPOCH};
use aether_light::{consensus_namespace, devnet_threshold, devnet_validator_key, to_hex,
    verify_finalized, verify_finalized_chain, verify_upgrade, Scheme, ValidatorSet, UPGRADE_NAMESPACE};
use aether_types::{B256, GasVector};
use commonware_codec::Encode;
use commonware_consensus::simplex::types::{Finalization, Finalize, Proposal};
use commonware_consensus::types::{Height, Round, View};
use commonware_cryptography::bls12381::primitives::{ops, variant::MinSig};
use commonware_cryptography::{Digestible, Signer};
use commonware_parallel::Sequential;
use commonware_utils::non_empty;
use serde_json::{json, Value};

const CHAIN: u64 = 7_777;
const HEIGHT: u64 = 101;
const TIMESTAMP_MS: u64 = 1_780_000_000_000;

fn fixture() -> Value {
    let (participants, polynomial, shares) = devnet_threshold(4);
    let signed = |protocol, activate_at, registrar| {
        let upgrade = Upgrade {
            chain_id: CHAIN, protocol, activate_at, emergency: false,
            releases: vec![Release {
                platform: "macos-arm64-dmg".into(), version: "0.0.0-test".into(),
                blake3: "ab".repeat(32), url: "https://example.invalid/upgrade.dmg".into(),
            }],
            notes: format!("Finalized protocol {protocol}"), registrar,
        };
        let message = serde_json::to_vec(&upgrade).unwrap();
        let partials: Vec<_> = shares.iter().take(3).map(|(_, share)|
            ops::threshold::sign_message::<MinSig>(share, UPGRADE_NAMESPACE, &message)
        ).collect();
        let signature = ops::threshold::recover::<MinSig, _>(
            &polynomial, &partials, &Sequential,
        ).unwrap();
        SignedUpgrade { upgrade, signature: to_hex(&signature.encode()), emergency_approvals: vec![] }
    };
    let earlier = signed(4, HEIGHT + 604_800,
        Some((B256::repeat_byte(1), B256::repeat_byte(2))));
    let current = signed(5, HEIGHT + 1_209_600, None);
    let staged = signed(6, HEIGHT + 1_814_400, None);
    let conflicting = signed(4, earlier.upgrade.activate_at,
        Some((B256::repeat_byte(3), B256::repeat_byte(4))));
    let prior_metadata = serde_json::to_vec(&(
        GasVector::default(), Value::Null, Value::Null, vec![json!([3, 0])],
    )).unwrap();
    let prior_meta = B256::from(Hasher::hash_bytes(&ChainHasher::new(), &prior_metadata));
    let schedule = vec![json!([3, 0]), json!([
        earlier.upgrade.protocol, earlier.upgrade.activate_at, earlier.upgrade.registrar.unwrap(),
    ])];
    let metadata = serde_json::to_vec(&(
        GasVector::default(), Value::Null, Value::Null, schedule,
    )).unwrap();
    let legacy = B256::from(Hasher::hash_bytes(&ChainHasher::new(), &metadata));
    let archive_excess = 7u64;
    let parent_meta = B256::from(Hasher::hash_bytes(&ChainHasher::new(),
        &serde_json::to_vec(&(legacy, archive_excess)).unwrap()));

    let genesis = Block::genesis(CHAIN, B256::repeat_byte(3));
    let leader = devnet_validator_key(1).public_key();
    let parent = Block::new(Context {
        round: Round::new(EPOCH, View::new(HEIGHT - 1)), leader: leader.clone(),
        parent: (View::new(0), genesis.digest()),
    }, genesis.digest(), Height::new(HEIGHT - 1), TIMESTAMP_MS - 1_000,
        Payload { version: 3, parent_meta: prior_meta, upgrade: Some(earlier.clone()), ..Default::default() }.to_bytes());
    let block = Block::new(Context {
        round: Round::new(EPOCH, View::new(HEIGHT)), leader,
        parent: (View::new(HEIGHT - 1), parent.digest()),
    }, parent.digest(), Height::new(HEIGHT), TIMESTAMP_MS,
        Payload { version: 3, parent_meta, upgrade: Some(current.clone()), ..Default::default() }.to_bytes());
    let signers: Vec<_> = shares.iter().take(3).map(|(_, share)|
        Scheme::signer(&consensus_namespace(), participants.clone(), polynomial.clone(), share.clone()).unwrap()
    ).collect();
    let votes: Vec<_> = signers.iter().map(|signer| Finalize::sign(signer,
        Proposal::new(block.context.round, block.context.parent.0, block.digest())).unwrap()).collect();
    let certificate = Finalization::from_owned_finalizes(
        &signers[0], non_empty![@votes.into_iter()], &Sequential,
    ).unwrap();
    let set = ValidatorSet::devnet(4);
    verify_upgrade(set.identity(), &earlier).unwrap();
    verify_upgrade(set.identity(), &current).unwrap();
    verify_upgrade(set.identity(), &staged).unwrap();
    verify_upgrade(set.identity(), &conflicting).unwrap();
    let verified = verify_finalized(&set, &block.encode(), &certificate.encode()).unwrap();
    assert_eq!(verified.height, HEIGHT);
    assert_eq!(verified.timestamp_ms, TIMESTAMP_MS);
    let linked_parent = verify_finalized_chain(&set, &parent.encode(), &certificate.encode(),
        &[block.encode().to_vec()]).unwrap();
    assert_eq!(linked_parent.height, HEIGHT - 1);
    assert_eq!(linked_parent.timestamp_ms, TIMESTAMP_MS - 1_000);
    json!({
        "identity": set.identity_hex(), "chain_id": CHAIN,
        "height": HEIGHT, "timestamp_ms": TIMESTAMP_MS,
        "block": to_hex(&block.encode()), "finalization": to_hex(&certificate.encode()), "links": [],
        "parent_block": to_hex(&parent.encode()),
        "parent_upgrade_metadata": { "height": HEIGHT - 2, "encoded": to_hex(&prior_metadata), "archive_excess": 0 },
        "upgrade_metadata": { "height": HEIGHT - 1, "encoded": to_hex(&metadata), "archive_excess": archive_excess },
        "schedule": [[3, 0], [earlier.upgrade.protocol, earlier.upgrade.activate_at],
            [current.upgrade.protocol, current.upgrade.activate_at]],
        "newest_scheduled": current.upgrade.protocol, "upcoming_upgrades": [earlier, current],
        "staged_upgrade": staged, "conflicting_upgrade": conflicting,
    })
}

#[test]
fn finalized_upgrade_fixture_is_reproducible() {
    let actual = fixture();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if std::env::var("AETHER_WRITE_UPGRADE_FIXTURE").as_deref() == Ok("1") {
        let tmp = root.join("tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("upgrade-finalized.json"),
            serde_json::to_string_pretty(&actual).unwrap()).unwrap();
        return;
    }
    let expected: Value = serde_json::from_slice(&std::fs::read(
        root.join("crates/ffi/tests/fixtures/upgrade-finalized.json"),
    ).unwrap()).unwrap();
    assert_eq!(actual, expected, "upgrade certificate fixture must remain byte-identical");
}
