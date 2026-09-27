//! Protocol upgrades activate at a height without touching the genesis: the
//! committee-signed upgrade goes on chain, every node learns its activation the
//! same way, the activation block applies the protocol's one-time state changes,
//! and a node that does not run the new protocol stops there.

use aether_execution::{StateError, WorldState};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{build_payload, Chain, ChainConfig, ChainError, Executed, Extras};
use aether_node::snapshot::Snapshot;
use aether_node::upgrade::{combine, sign_partial, Release, SignedUpgrade, Upgrade};
use aether_types::{Address, GasVector, U256};
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible, Signer};
use std::sync::Arc;

const CHAIN: u64 = 7_790;
/// Voting-node epoch = upgrade notice, in blocks.
const EPOCH_BLOCKS: u64 = 10;
const MARKER: Address = Address::repeat_byte(0x42);

fn config() -> ChainConfig {
    ChainConfig {
        chain_id: CHAIN,
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        alloc: vec![],
        fees: false,
        registrar: Some(([1; 32], [2; 32])),
        epoch_blocks: EPOCH_BLOCKS,
        min_streak: None,
        draw_epochs: None,
    }
}

/// Protocol 2 of this test marks its activation in the state.
fn migrate(protocol: u32, state: &mut WorldState) -> Result<(), StateError> {
    if protocol == 2 {
        state.set_balance(MARKER, U256::from(7u64))?;
    }
    Ok(())
}

fn node(protocol: u32) -> (Chain, Block) {
    let (chain, genesis) = Chain::new(config());
    let (_, sharing, _) = aether_light::devnet_threshold(4);
    let mut g = chain.lock();
    g.identity = Some(*sharing.public());
    g.protocol = protocol;
    g.migrate = migrate;
    drop(g);
    (chain, genesis)
}

fn signed(protocol: u32, activate_at: u64) -> SignedUpgrade {
    let (_, sharing, shares) = aether_light::devnet_threshold(4);
    let u = Upgrade {
        chain_id: CHAIN,
        protocol,
        activate_at,
        releases: vec![Release { platform: "macos-arm64-dmg".into(), version: "0.6.0".into(), blake3: "ab".repeat(32), url: "https://x".into() }],
        notes: String::new(),
    };
    let partials: Vec<_> = shares.iter().take(3).map(|(_, s)| sign_partial(&u, s)).collect();
    combine(&sharing, &partials).unwrap()
}

/// A block on `parent` built the way a proposer builds it.
fn propose(chain: &Chain, parent: &Executed, parent_block: &Block, upgrade: Option<SignedUpgrade>) -> Block {
    let height = parent_block.height.next();
    let leader = ed25519::PrivateKey::from_seed(1).public_key();
    let context = Context { round: Round::new(EPOCH, View::new(height.get())), leader, parent: (View::new(height.get() - 1), parent_block.digest()) };
    let ts = height.get() * 1_000;
    let skeleton = Block::new(context.clone(), parent_block.digest(), height, ts, bytes::Bytes::new());
    let ctx = Chain::block_context(&chain.cfg(), &skeleton, parent);
    let pre = chain.pre_state(parent, parent.next_protocol()).unwrap();
    let (payload, _) = build_payload(parent, &pre, &ctx, vec![], Extras { upgrade, ..Default::default() });
    Block::new(context, parent_block.digest(), height, ts, payload.to_bytes())
}

fn with_payload(b: &Block, f: impl FnOnce(&mut aether_node::block::Payload)) -> Block {
    let mut p = b.payload().unwrap();
    f(&mut p);
    Block::new(b.context.clone(), b.parent, b.height, b.timestamp, p.to_bytes())
}

fn advance(chain: &Chain, parent: Arc<Executed>, block: &Block) -> Arc<Executed> {
    let exec = chain.execute(block, &parent).unwrap();
    chain.finalize(block).unwrap();
    exec
}

#[test]
fn a_signed_upgrade_activates_at_its_height_and_old_nodes_stop_there() {
    let (chain, genesis) = node(2);
    let (old, _) = node(1);
    let mut parent = chain.lock().finalized.clone();
    let mut blocks = vec![genesis.clone()];

    // Block 1 puts the upgrade on chain: protocol 2 from height 20.
    let b1 = propose(&chain, &parent, &genesis, Some(signed(2, 20)));
    parent = advance(&chain, parent, &b1);
    assert_eq!(*parent.schedule, vec![(2, 20)]);
    blocks.push(b1);
    for _ in 2..20 {
        let b = propose(&chain, &parent, blocks.last().unwrap(), None);
        assert_eq!(b.payload().unwrap().version, 1);
        parent = advance(&chain, parent, &b);
        blocks.push(b);
    }
    assert_eq!(parent.height, 19);
    assert_eq!(parent.state.balance(&MARKER), U256::ZERO, "nothing changes before activation");

    // Block 20 runs protocol 2 and applies its one-time change; block 21 does not repeat it.
    let b20 = propose(&chain, &parent, blocks.last().unwrap(), None);
    assert_eq!(b20.payload().unwrap().version, 2);
    let wrong = with_payload(&b20, |p| p.version = 1);
    assert!(matches!(chain.execute(&wrong, &parent), Err(ChainError::Protocol(_))), "old rules at the activation height");
    parent = advance(&chain, parent, &b20);
    assert_eq!(parent.state.balance(&MARKER), U256::from(7u64));
    blocks.push(b20.clone());
    let b21 = propose(&chain, &parent, &b20, None);
    let p21 = advance(&chain, parent.clone(), &b21);
    assert_eq!(p21.state.balance(&MARKER), U256::from(7u64));

    // A node running only protocol 1 follows up to the activation and stops at it.
    let mut op = old.lock().finalized.clone();
    for b in &blocks[1..20] {
        op = advance(&old, op, b);
    }
    match old.execute(&b20, &op) {
        Err(ChainError::Protocol(e)) => assert!(e.contains("UPGRADE REQUIRED"), "{e}"),
        other => panic!("old node must stop at the new protocol: {:?}", other.map(|_| ())),
    }

    // A checkpoint carries the schedule, certified by the next block.
    let snap = Snapshot::of(&old);
    let (_, sharing, _) = aether_light::devnet_threshold(4);
    snap.check(&b20, &config(), sharing.public()).unwrap();
    let mut forged = snap.clone();
    forged.schedule.clear();
    assert!(forged.check(&b20, &config(), sharing.public()).is_err(), "schedule is certified");
}

#[test]
fn upgrades_that_may_not_go_on_chain_are_refused() {
    let (chain, genesis) = node(2);
    let parent = chain.lock().finalized.clone();
    let refused = |u: SignedUpgrade| {
        let b = propose_unchecked(&chain, &parent, &genesis, u);
        matches!(chain.execute(&b, &parent), Err(ChainError::Protocol(_)))
    };
    assert!(refused(signed(2, 5)), "less notice than one epoch");
    assert!(refused(signed(1, 50)), "not a newer protocol");
    let mut other = signed(2, 50);
    other.upgrade.chain_id = CHAIN + 1;
    assert!(refused(other), "another chain");
    let mut forged = signed(2, 50);
    forged.upgrade.activate_at = 60;
    assert!(refused(forged), "not what the committee signed");
    let mut big = signed(2, 50);
    big.upgrade.notes = "x".repeat(10_000);
    assert!(refused(big), "too large");

    // The proposer only offers what may go on chain, once.
    chain.lock().upgrades_known = vec![signed(2, 5), signed(2, 50)];
    assert_eq!(chain.upgrade_for(&parent).map(|s| s.upgrade.activate_at), Some(50));
    let b1 = propose(&chain, &parent, &genesis, chain.upgrade_for(&parent));
    let p1 = advance(&chain, parent, &b1);
    assert!(chain.upgrade_for(&p1).is_none(), "already on chain");
    let again = propose_unchecked(&chain, &p1, &b1, signed(2, 50));
    assert!(matches!(chain.execute(&again, &p1), Err(ChainError::Protocol(_))), "replayed upgrade");
}

#[test]
fn only_the_canonical_payload_encoding_is_accepted() {
    let (chain, genesis) = node(1);
    let parent = chain.lock().finalized.clone();
    let b1 = propose(&chain, &parent, &genesis, None);
    let mut json: serde_json::Value = serde_json::from_slice(&b1.data).unwrap();
    json["future_field"] = serde_json::json!(1);
    let unknown = Block::new(b1.context.clone(), b1.parent, b1.height, b1.timestamp, serde_json::to_vec(&json).unwrap().into());
    assert!(matches!(chain.execute(&unknown, &parent), Err(ChainError::BadPayload)), "unknown field");
    let spaced = Block::new(b1.context.clone(), b1.parent, b1.height, b1.timestamp, serde_json::to_vec_pretty(&b1.payload().unwrap()).unwrap().into());
    assert!(matches!(chain.execute(&spaced, &parent), Err(ChainError::BadPayload)), "second encoding");
    chain.execute(&b1, &parent).unwrap();
}

/// A block carrying `u` without the proposer's own filtering.
fn propose_unchecked(chain: &Chain, parent: &Executed, parent_block: &Block, u: SignedUpgrade) -> Block {
    with_payload(&propose(chain, parent, parent_block, None), |p| p.upgrade = Some(u))
}

#[test]
fn a_store_never_mixes_two_geneses() {
    use aether_node::store::{Store, StoreError};
    let dir = std::env::temp_dir().join(format!("aether-genesis-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chain.redb");
    drop(Chain::open(config(), Store::open(&path).unwrap()).unwrap());
    // The same genesis reopens; another one (here: another chain id) is refused.
    drop(Chain::open(config(), Store::open(&path).unwrap()).unwrap());
    let other = ChainConfig { chain_id: CHAIN + 1, ..config() };
    assert!(matches!(Chain::open(other, Store::open(&path).unwrap()), Err(StoreError::OtherGenesis)));
    let _ = std::fs::remove_dir_all(&dir);
}
