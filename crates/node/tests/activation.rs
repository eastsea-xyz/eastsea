//! Protocol upgrades activate at a height without touching the genesis: the
//! committee-signed upgrade goes on chain, every node learns its activation the
//! same way, the activation block applies the protocol's one-time state changes,
//! and a node that does not run the new protocol stops there.

use aether_execution::{StateError, WorldState};
use aether_light::block::ProofClaim;
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
        registrar: None,
    };
    let partials: Vec<_> = shares.iter().take(3).map(|(_, s)| sign_partial(&u, s)).collect();
    combine(&sharing, &partials).unwrap()
}

/// A block on `parent` built the way a proposer builds it.
fn propose(chain: &Chain, parent: &Executed, parent_block: &Block, upgrade: Option<SignedUpgrade>) -> Block {
    propose_with(chain, parent, parent_block, upgrade, vec![])
}

fn propose_with(chain: &Chain, parent: &Executed, parent_block: &Block, upgrade: Option<SignedUpgrade>, proofs: Vec<ProofClaim>) -> Block {
    let height = parent_block.height.next();
    let leader = ed25519::PrivateKey::from_seed(1).public_key();
    let context = Context { round: Round::new(EPOCH, View::new(height.get())), leader, parent: (View::new(height.get() - 1), parent_block.digest()) };
    let ts = height.get() * 1_000;
    let skeleton = Block::new(context.clone(), parent_block.digest(), height, ts, bytes::Bytes::new());
    let ctx = Chain::block_context(&chain.cfg(), &skeleton, parent);
    let (pre, _) = chain.pre_state(parent, parent.next_protocol(), &proofs, false).unwrap();
    let (payload, _) = build_payload(parent, &pre, &ctx, vec![], Extras { upgrade, proofs, ..Default::default() });
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
    assert_eq!(*parent.schedule, vec![aether_node::upgrade::Activation { protocol: 2, at: 20, registrar: None }]);
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

#[test]
fn an_activation_block_survives_a_restart() {
    use aether_node::store::Store;
    let dir = std::env::temp_dir().join(format!("aether-activation-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chain.redb");
    let (chain, genesis) = Chain::open(config(), Store::open(&path).unwrap()).unwrap();
    {
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        let mut g = chain.lock();
        g.identity = Some(*sharing.public());
        g.protocol = 2;
        g.migrate = migrate;
    }
    let mut parent = chain.lock().finalized.clone();
    let mut last = propose(&chain, &parent, &genesis, Some(signed(2, 20)));
    parent = advance(&chain, parent, &last);
    for _ in 2..=21 {
        let b = propose(&chain, &parent, &last, None);
        parent = advance(&chain, parent, &b);
        last = b;
    }
    assert_eq!(parent.state.balance(&MARKER), U256::from(7u64));
    let root = parent.state.root();
    drop((chain, parent));
    // The activation's write is on disk with the block: the store reopens to the same root.
    let (again, _) = Chain::open(config(), Store::open(&path).unwrap()).unwrap();
    let f = again.lock().finalized.clone();
    assert_eq!((f.height, f.state.root()), (21, root));
    assert_eq!(f.state.balance(&MARKER), U256::from(7u64));
    assert_eq!(*f.schedule, vec![aether_node::upgrade::Activation { protocol: 2, at: 20, registrar: None }]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Accepts a "proof" that is the commitment itself (tests only).
struct EchoVerifier;
impl aether_node::chain::ProofVerifier for EchoVerifier {
    fn verify(&self, proof: &[u8], commitment: [u8; 32]) -> bool {
        proof == commitment
    }
}

#[test]
fn protocol_2_records_statements_and_pays_the_first_valid_proof() {
    use aether_execution::proofs;
    let (chain, genesis) = node(2);
    chain.lock().verifier = Some(Arc::new(EchoVerifier));
    let mut parent = chain.lock().finalized.clone();
    let mut last = propose(&chain, &parent, &genesis, Some(signed(2, 20)));
    parent = advance(&chain, parent, &last);
    for _ in 2..=21 {
        let b = propose(&chain, &parent, &last, None);
        parent = advance(&chain, parent, &b);
        last = b;
    }
    // From activation on, each block records its parent's statement: block 21 recorded
    // block 20's (the first protocol-2 block); protocol-1 blocks have none.
    assert!(proofs::commitment(&parent.state, 20).is_some());
    assert_eq!(proofs::commitment(&parent.state, 19), None);

    let prover = Address::repeat_byte(0x77);
    // The echo "proof" is the claim output (statement commitment bound to the prover).
    let claim = |h: u64, c: [u8; 32]| ProofClaim { height: h, prover, proof: hex::encode(aether_proving::block::claim(c, prover)) };
    let c20 = proofs::commitment(&parent.state, 20).unwrap();
    // A wrong proof, a proof of an unrecorded block, and two claims of one block are refused.
    // The same proof rerouted to another address does not verify.
    let stolen = ProofClaim { prover: Address::repeat_byte(0x66), ..claim(20, c20) };
    for bad in [vec![claim(20, [1; 32])], vec![claim(5, [0; 32])], vec![claim(20, c20), claim(20, c20)], vec![stolen]] {
        let b = with_payload(&propose(&chain, &parent, &last, None), |p| p.proofs = bad.clone());
        assert!(matches!(chain.execute(&b, &parent), Err(ChainError::Protocol(_))), "{bad:?}");
    }
    // The first valid proof is paid the block's issuance; block 21 (recorded by this block) can be proven here too.
    let c21 = parent.statement.commitment;
    let b = propose_with(&chain, &parent, &last, None, vec![claim(20, c20), claim(21, c21)]);
    let p22 = advance(&chain, parent.clone(), &b);
    assert_eq!(p22.state.balance(&prover), proofs::issuance(20) + proofs::issuance(21));
    assert_eq!(p22.payouts.len(), 2);
    assert_eq!(proofs::prover(&p22.state, 20), Some(prover));
    // Proven once only.
    let again = propose_with(&chain, &p22, &b, None, vec![]);
    let again = with_payload(&again, |p| p.proofs = vec![claim(20, c20)]);
    assert!(matches!(chain.execute(&again, &p22), Err(ChainError::Protocol(_))));
    // A node without a verifier refuses blocks with proofs.
    chain.lock().verifier = None;
    let fresh = with_payload(&propose(&chain, &parent, &last, None), |p| p.proofs = vec![claim(20, c20)]);
    assert!(matches!(chain.execute(&fresh, &parent), Err(ChainError::Protocol(_))));
}

#[test]
fn the_recorded_statement_is_what_the_prover_proves() {
    let (chain, genesis) = node(2);
    let mut parent = chain.lock().finalized.clone();
    let mut last = propose(&chain, &parent, &genesis, Some(signed(2, 20)));
    parent = advance(&chain, parent, &last);
    for _ in 2..=22 {
        let b = propose(&chain, &parent, &last, None);
        // The prover's input for this block: its pre-state (after the block's system writes), context, txs.
        let (pre, _) = chain.pre_state(&parent, parent.next_protocol(), &[], false).unwrap();
        let ctx = Chain::block_context(&chain.cfg(), &b, &parent);
        let input = aether_proving::block::input(&pre, &ctx, &b.payload().unwrap().txs, &[], Address::repeat_byte(1)).unwrap();
        let proved = aether_proving::block::execute(&input).unwrap().commitment();
        drop(pre);
        parent = advance(&chain, parent, &b);
        if parent.height >= 20 {
            assert_eq!(proved, parent.statement.commitment, "block {}", parent.height);
        } else {
            assert_eq!(parent.statement, Default::default(), "protocol-1 blocks keep no statement");
        }
        last = b;
    }
}

#[test]
fn a_signed_upgrade_replaces_the_registrar_when_it_activates() {
    use aether_execution::registry::REGISTRY;
    use aether_types::B256;
    let (chain, genesis) = node(3);
    let (_, sharing, shares) = aether_light::devnet_threshold(4);
    let sign = |u: &Upgrade| combine(&sharing, &shares.iter().take(3).map(|(_, s)| sign_partial(u, s)).collect::<Vec<_>>()).unwrap();
    // A registrar change cannot be announced before protocol 2 (protocol-1 nodes cannot read it).
    let mut early = signed(2, 20).upgrade;
    early.registrar = Some((B256::repeat_byte(5), B256::repeat_byte(6)));
    let parent = chain.lock().finalized.clone();
    let b = with_payload(&propose(&chain, &parent, &genesis, None), |p| p.upgrade = Some(sign(&early)));
    assert!(matches!(chain.execute(&b, &parent), Err(ChainError::Protocol(_))));

    // Protocol 2 at 20; then protocol 3 at 40 carrying the new registrar key.
    let mut parent = parent;
    let mut last = propose(&chain, &parent, &genesis, Some(signed(2, 20)));
    parent = advance(&chain, parent, &last);
    let mut later = signed(3, 40).upgrade;
    later.registrar = Some((B256::repeat_byte(5), B256::repeat_byte(6)));
    let before = parent.state.storage(&REGISTRY, U256::ZERO);
    for _ in 2..=40 {
        let up = (parent.height == 25).then(|| sign(&later));
        let b = propose(&chain, &parent, &last, up);
        parent = advance(&chain, parent, &b);
        last = b;
        if parent.height == 39 {
            assert_eq!(parent.state.storage(&REGISTRY, U256::ZERO), before, "unchanged until activation");
        }
    }
    assert_eq!(parent.state.storage(&REGISTRY, U256::ZERO), U256::from_be_bytes([5; 32]));
    assert_eq!(parent.state.storage(&REGISTRY, U256::from(1u64)), U256::from_be_bytes([6; 32]));
}

#[test]
fn a_snapshot_with_a_schedule_round_trips() {
    let (chain, genesis) = node(2);
    chain.lock().verifier = Some(Arc::new(EchoVerifier));
    let mut parent = chain.lock().finalized.clone();
    let mut last = propose(&chain, &parent, &genesis, Some(signed(2, 20)));
    parent = advance(&chain, parent, &last);
    for _ in 2..=21 {
        let b = propose(&chain, &parent, &last, None);
        parent = advance(&chain, parent, &b);
        last = b;
    }
    let snap = Snapshot::of(&chain);
    let back = Snapshot::from_bytes(&snap.to_bytes()).unwrap();
    assert_eq!(back.schedule, snap.schedule);
    assert_eq!(back.statement, snap.statement);
    assert_ne!(back.statement, Default::default());
}

#[test]
fn a_finalized_block_with_proofs_is_applied_even_without_a_local_verifier() {
    use aether_execution::proofs;
    let (a, genesis) = node(2);
    let (b, _) = node(2);
    a.lock().verifier = Some(Arc::new(EchoVerifier));
    let mut pa = a.lock().finalized.clone();
    let mut pb = b.lock().finalized.clone();
    let mut head = genesis.clone();
    let mut next = propose(&a, &pa, &genesis, Some(signed(2, 20)));
    for _ in 1..=21 {
        pa = advance(&a, pa, &next);
        pb = advance(&b, pb, &next);
        head = next;
        next = propose(&a, &pa, &head, None);
    }
    let _ = next;
    let prover = Address::repeat_byte(0x77);
    let c = proofs::commitment(&pa.state, 20).unwrap();
    let claim = ProofClaim { height: 20, prover, proof: hex::encode(aether_proving::block::claim(c, prover)) };
    let with_proof = propose_with(&a, &pa, &head, None, vec![claim]);
    // B has no verifier: it would not vote for this block...
    assert!(b.execute(&with_proof, &pb).is_err());
    // ...but once the committee finalized it, B applies it (the quorum verified the proof).
    b.finalize(&with_proof).unwrap();
    let applied = b.lock().finalized.clone();
    assert_eq!(applied.height, pb.height + 1);
    assert_eq!(applied.state.balance(&prover), proofs::issuance(20));
}
