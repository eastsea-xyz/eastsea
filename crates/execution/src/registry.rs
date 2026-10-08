//! The CommitteeRegistry predeploy (contracts/src/CommitteeRegistry.sol): Macs
//! registered with DeviceCheck become voting-node candidates and send liveness
//! beacons; nodes read this state to pick each epoch's voting nodes.

use crate::world::WorldState;
use alloy_primitives::{address, keccak256, Address, Bytes, B256, U256};
use alloy_sol_types::{sol, SolCall, SolValue};

/// Where the registry lives (genesis predeploy).
pub const REGISTRY: Address = address!("0000000000000000000000000000000000007703");
/// Blocks per epoch on the testnet (one hour of 1 s blocks); set at genesis.
pub const EPOCH_BLOCKS: u64 = 3_600;
/// Epochs of unbroken liveness before a Mac can be drawn into the voting set.
pub const MIN_STREAK: u64 = 24;
/// Epochs between voting-set draws (a day of one-hour epochs).
pub const DRAW_EPOCHS: u64 = 24;

/// Voting-set parameters, fixed at genesis (registry slots 4, 5, 6); nodes
/// read them from state. Changed only by a committee-signed upgrade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub epoch_blocks: u64,
    pub min_streak: u64,
    pub draw_epochs: u64,
}

impl Default for Params {
    fn default() -> Self {
        Params { epoch_blocks: EPOCH_BLOCKS, min_streak: MIN_STREAK, draw_epochs: DRAW_EPOCHS }
    }
}

sol! {
    function register(bytes32 validatorKey, bytes32 nodeId, address beaconer, bytes32 r, bytes32 s);
    function beacon(bytes32 validatorKey);
}

/// Runtime bytecode of `CommitteeRegistry` (solc 0.8.19, optimizer 200 runs).
pub fn code() -> Bytes {
    Bytes::from(alloy_primitives::hex::decode(include_str!("committee_registry.bin.hex").trim()).expect("valid hex"))
}

/// Genesis: the registry with the registrar's P-256 key (x, y) in slots 0 and
/// 1 and the voting-set parameters in slots 4, 5 and 6.
pub fn predeploy(state: &mut WorldState, registrar: ([u8; 32], [u8; 32]), params: Params) -> Result<(), crate::world::StateError> {
    state.set_code(REGISTRY, code())?;
    state.set_storage(REGISTRY, U256::ZERO, U256::from_be_bytes(registrar.0));
    state.set_storage(REGISTRY, U256::from(1u64), U256::from_be_bytes(registrar.1));
    state.set_storage(REGISTRY, U256::from(4u64), U256::from(params.epoch_blocks.max(1)));
    state.set_storage(REGISTRY, U256::from(5u64), U256::from(params.min_streak));
    state.set_storage(REGISTRY, U256::from(6u64), U256::from(params.draw_epochs.max(1)));
    Ok(())
}

/// Runtime bytecode of `CommitteeRegistry` v2 (protocol 2: registrations per
/// epoch bounded; same storage layout, new slots 7-9).
pub fn code_v2() -> Bytes {
    Bytes::from(alloy_primitives::hex::decode(include_str!("committee_registry_v2.bin.hex").trim()).expect("valid hex"))
}

/// New candidates per epoch from protocol 2 (slot 7).
pub const MAX_PER_EPOCH: u64 = 16;
/// Storage slot of the per-epoch bound (0 = none, the v1 registry).
const SLOT_MAX_PER_EPOCH: u64 = 7;

/// The per-epoch registration bound in force (0 before protocol 2 installed
/// the v2 registry, whether by an activation block or at genesis).
pub fn max_per_epoch(state: &WorldState) -> u64 {
    state.storage(&REGISTRY, U256::from(SLOT_MAX_PER_EPOCH)).to::<u64>()
}

/// Protocol 2: the registry's code becomes v2 with the per-epoch bound (no-op without a registry).
pub fn upgrade_to_v2(state: &mut WorldState) -> Result<(), crate::world::StateError> {
    if state.code(&REGISTRY).is_empty() {
        return Ok(());
    }
    state.set_code(REGISTRY, code_v2())?;
    state.set_storage(REGISTRY, U256::from(SLOT_MAX_PER_EPOCH), U256::from(MAX_PER_EPOCH));
    Ok(())
}

/// Replace the registrar key (a committee-signed upgrade); zeros stop registrations.
pub fn set_registrar(state: &mut WorldState, registrar: ([u8; 32], [u8; 32])) {
    state.set_storage(REGISTRY, U256::ZERO, U256::from_be_bytes(registrar.0));
    state.set_storage(REGISTRY, U256::from(1u64), U256::from_be_bytes(registrar.1));
}

/// The registrar P-256 key (x, y) in force: registry slots 0 and 1, the same
/// pair `CommitteeRegistry.register` verifies attestations against.
pub fn registrar(state: &WorldState) -> ([u8; 32], [u8; 32]) {
    (
        state.storage(&REGISTRY, U256::ZERO).to_be_bytes::<32>(),
        state.storage(&REGISTRY, U256::from(1u64)).to_be_bytes::<32>(),
    )
}

/// Whether the committee stopped the registrar: both key halves were zeroed by
/// an upgrade (`set_registrar(([0; 32], [0; 32]))`), so no attestation — a new
/// registration or a beacon re-attestation — verifies any more
/// (docs/design/14-registration.md 4). Candidates registered before stay.
pub fn registrar_revoked(state: &WorldState) -> bool {
    registrar(state) == ([0u8; 32], [0u8; 32])
}

/// Blocks per epoch as set at genesis.
pub fn epoch_blocks(state: &WorldState) -> u64 {
    state.storage(&REGISTRY, U256::from(4u64)).to::<u64>().max(1)
}

// ---- The free registration lane (docs/design/22-gas-pool.md 2층) ----
//
// A block may carry registrations as payload items, validated exactly like the
// contract's `register` and applied as a system write (beacon answers' pattern),
// so a Mac with a zero balance registers even while the chain is congested.
// The primitives below are the contract's own storage rules, shared by both
// paths so the two can never disagree.

/// Free registrations one block may carry: each costs two P-256 verifications
/// (the registrar's attestation and the operator wallet's relay signature).
/// Items are fixed-size, so this bounds the lane's bytes too.
pub const MAX_FREE_PER_BLOCK: usize = 4;

/// Slot of `indexOf` (validatorKey => index + 1; 0 is unknown).
fn index_of_slot(validator_key: &[u8; 32]) -> U256 {
    U256::from_be_bytes(keccak256([validator_key.as_slice(), &U256::from(3u64).to_be_bytes::<32>()].concat()).0)
}

/// The candidate index of `validator_key`, + 1 as the contract stores it
/// (0: not registered). The contract's `Known` check, as a read.
pub fn index_of(state: &WorldState, validator_key: &[u8; 32]) -> u64 {
    state.storage(&REGISTRY, index_of_slot(validator_key)).to::<u64>()
}

/// Free-lane items the operator at `operator` has already spent, in a tagged
/// slot far above the contract's own (its literal slots stay small numbers and
/// its arrays live at keccak-derived ones) — the rewards crate's convention.
const TAG_LANE_NONCE: u128 = 9;

fn lane_nonce_slot(operator: &Address) -> U256 {
    (U256::from(TAG_LANE_NONCE) << 200) | U256::from_be_slice(operator.as_slice())
}

/// The next relay nonce `operator`'s free-lane item must carry: an item is
/// consumed exactly once (the signed nonce makes replay a state check).
pub fn lane_nonce(state: &WorldState, operator: &Address) -> u64 {
    state.storage(&REGISTRY, lane_nonce_slot(operator)).to::<u64>()
}

/// Count one more spent item for `operator` (after its registration applied).
pub fn bump_lane_nonce(state: &mut WorldState, operator: &Address) {
    let slot = lane_nonce_slot(operator);
    let next = state.storage(&REGISTRY, slot).saturating_add(U256::from(1u8));
    state.set_storage(REGISTRY, slot, next);
}

/// New registrations epoch `epoch` may still take: the v2 contract's own bound
/// (slot 7) when it set one, else the built-in cap — one number for both paths.
pub fn per_epoch_cap(state: &WorldState) -> u64 {
    match state.storage(&REGISTRY, U256::from(7u64)).to::<u64>() {
        0 => MAX_PER_EPOCH,
        set => set,
    }
}

/// The epoch `count_registration` last wrote.
pub fn reg_epoch(state: &WorldState) -> u64 {
    state.storage(&REGISTRY, U256::from(8u64)).to::<u64>()
}

/// Registrations counted for that epoch.
pub fn reg_count(state: &WorldState) -> u64 {
    state.storage(&REGISTRY, U256::from(9u64)).to::<u64>()
}

/// Count a new registration in `epoch` (slots 8 and 9, exactly the words the
/// v2 contract keeps): a new epoch resets, the cap spent refuses.
fn count_registration(state: &mut WorldState, epoch: u64) -> Result<(), String> {
    let (mut e, mut n) = (reg_epoch(state), reg_count(state));
    if e != epoch {
        e = epoch;
        n = 0;
    }
    let cap = per_epoch_cap(state);
    if n >= cap {
        return Err(format!("epoch {epoch} already took {n} of {cap} registrations"));
    }
    state.set_storage(REGISTRY, U256::from(8u64), U256::from(e));
    state.set_storage(REGISTRY, U256::from(9u64), U256::from(n + 1));
    Ok(())
}

/// What the operator wallet signs to put its registration in a block's free
/// lane: domain-separated (chain, registry), over the whole registered content
/// plus the one-shot nonce and expiry. A relay carrying someone else's
/// attestation fails this check.
#[allow(clippy::too_many_arguments)] // the signed message is exactly these fields
pub fn relay_message(
    chain_id: u64,
    operator: Address,
    validator_key: &[u8; 32],
    node_id: &[u8; 32],
    beaconer: Address,
    attestation: &[u8],
    nonce: u64,
    expiry: u64,
) -> Vec<u8> {
    [
        b"aether-registration".as_slice(),
        &chain_id.to_be_bytes(),
        REGISTRY.as_slice(),
        operator.as_slice(),
        validator_key,
        node_id,
        beaconer.as_slice(),
        attestation,
        &nonce.to_be_bytes(),
        &expiry.to_be_bytes(),
    ]
    .concat()
}

/// Register a candidate as a system write: the exact words the contract's
/// `register` writes for a fresh key, the v2 epoch cap counted the same way.
/// Signature checks happen before this (the contract path inside `register`,
/// the free lane in `aether_node::registrations`).
pub fn register_system(
    state: &mut WorldState,
    height: u64,
    operator: Address,
    validator_key: [u8; 32],
    node_id: [u8; 32],
    beaconer: Address,
) -> Result<(), String> {
    if index_of(state, &validator_key) != 0 {
        return Err("voting key already registered".into());
    }
    let epoch = height / epoch_blocks(state);
    count_registration(state, epoch)?;
    let index = state.storage(&REGISTRY, U256::from(2u64)).to::<u64>();
    if index >= 100_000 {
        return Err("registry full".into());
    }
    let at = |k: u64| U256::from_be_bytes(keccak256(U256::from(2u64).to_be_bytes::<32>()).0) + U256::from(5 * index + k);
    state.set_storage(REGISTRY, at(0), U256::from_be_slice(operator.as_slice()));
    state.set_storage(REGISTRY, at(1), U256::from_be_bytes(validator_key));
    state.set_storage(REGISTRY, at(2), U256::from_be_bytes(node_id));
    state.set_storage(REGISTRY, at(3), U256::from_be_slice(beaconer.as_slice()) | (U256::from(epoch) << 160usize));
    // The contract pushes (lastEpoch: e, streak: 1, missed: 0).
    state.set_storage(REGISTRY, at(4), U256::from(epoch) | (U256::from(1u8) << 64usize));
    state.set_storage(REGISTRY, U256::from(2u64), U256::from(index + 1));
    state.set_storage(REGISTRY, index_of_slot(&validator_key), U256::from(index + 1));
    Ok(())
}

/// The voting-set parameters as set at genesis.
pub fn params(state: &WorldState) -> Params {
    Params {
        epoch_blocks: epoch_blocks(state),
        min_streak: state.storage(&REGISTRY, U256::from(5u64)).to::<u64>(),
        draw_epochs: state.storage(&REGISTRY, U256::from(6u64)).to::<u64>().max(1),
    }
}

/// The bytes the registrar signs (the contract checks SHA-256 of them with P256VERIFY).
pub fn attestation_message(chain_id: u64, operator: Address, validator_key: [u8; 32], node_id: [u8; 32], beaconer: Address) -> Vec<u8> {
    (U256::from(chain_id), REGISTRY, operator, B256::from(validator_key), B256::from(node_id), beaconer).abi_encode_params()
}

pub fn encode_register(validator_key: [u8; 32], node_id: [u8; 32], beaconer: Address, r: [u8; 32], s: [u8; 32]) -> Bytes {
    registerCall { validatorKey: validator_key.into(), nodeId: node_id.into(), beaconer, r: r.into(), s: s.into() }.abi_encode().into()
}

pub fn encode_beacon(validator_key: [u8; 32]) -> Bytes {
    beaconCall { validatorKey: validator_key.into() }.abi_encode().into()
}

/// A candidate as stored (5 slots each: operator, validatorKey, nodeId,
/// [beaconer | registeredEpoch], [lastEpoch | streak]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub index: u64,
    pub operator: Address,
    pub validator_key: [u8; 32],
    pub node_id: [u8; 32],
    pub beaconer: Address,
    pub registered_epoch: u64,
    pub last_epoch: u64,
    pub streak: u64,
    /// Epochs missed within the grace period during the current streak.
    pub missed: u64,
}

fn word_u64(w: &[u8; 32], from_end: usize) -> u64 {
    let end = 32 - from_end;
    u64::from_be_bytes(w[end - 8..end].try_into().expect("8 bytes"))
}

/// All candidates, read from state.
pub fn candidates(state: &WorldState) -> Vec<Candidate> {
    let n = state.storage(&REGISTRY, U256::from(2u64)).to::<u64>();
    let base = U256::from_be_bytes(keccak256(U256::from(2u64).to_be_bytes::<32>()).0);
    (0..n.min(100_000))
        .map(|i| {
            let slot = |k: u64| state.storage(&REGISTRY, base + U256::from(5 * i + k)).to_be_bytes::<32>();
            let (s3, s4) = (slot(3), slot(4));
            Candidate {
                index: i,
                operator: Address::from_slice(&slot(0)[12..]),
                validator_key: slot(1),
                node_id: slot(2),
                beaconer: Address::from_slice(&s3[12..]),
                registered_epoch: word_u64(&s3, 20),
                last_epoch: word_u64(&s4, 0),
                streak: word_u64(&s4, 8),
                missed: word_u64(&s4, 16),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> WorldState {
        let mut s = WorldState::default();
        predeploy(&mut s, ([1; 32], [2; 32]), Params::default()).unwrap();
        s
    }

    fn key(i: u8) -> [u8; 32] {
        [i; 32]
    }

    #[test]
    fn register_system_writes_what_put_candidate_reads() {
        let mut s = state();
        let (op, beaconer) = (Address::repeat_byte(7), Address::repeat_byte(9));
        register_system(&mut s, 7_200, op, key(1), key(2), beaconer).unwrap();
        // One epoch in (3_600-block epochs): the words the contract writes.
        assert_eq!(reg_epoch(&s), 2);
        assert_eq!(reg_count(&s), 1);
        assert_eq!(index_of(&s, &key(1)), 1);
        let c = &candidates(&s)[0];
        assert_eq!((c.operator, c.validator_key, c.node_id, c.beaconer), (op, key(1), key(2), beaconer));
        assert_eq!((c.registered_epoch, c.last_epoch, c.streak, c.missed), (2, 2, 1, 0));
        // A second registration of the same key is refused, lane or contract.
        assert!(register_system(&mut s, 7_201, op, key(1), key(2), beaconer).unwrap_err().contains("already"));
    }

    #[test]
    fn the_epoch_cap_is_shared_and_resets_per_epoch() {
        let mut s = state();
        // Slot 7 unset (protocol 1): the lane still bounds itself.
        assert_eq!(per_epoch_cap(&s), MAX_PER_EPOCH);
        for i in 0..MAX_PER_EPOCH {
            register_system(&mut s, 10 + i as u64, Address::repeat_byte(i as u8 + 1), key(i as u8 + 1), key(0), Address::ZERO).unwrap();
        }
        assert_eq!(reg_count(&s), MAX_PER_EPOCH);
        let full = register_system(&mut s, 99, Address::repeat_byte(0xfe), key(0xfe), key(0), Address::ZERO).unwrap_err();
        assert!(full.contains("already took 16 of 16"), "{full}");
        // The next epoch starts clean.
        register_system(&mut s, 3_600, Address::repeat_byte(0xff), key(0xff), key(0), Address::ZERO).unwrap();
        assert_eq!((reg_epoch(&s), reg_count(&s)), (1, 1));
        // What v2 sets in slot 7 is the cap for both paths.
        let mut v2 = state();
        upgrade_to_v2(&mut v2).unwrap();
        assert_eq!(per_epoch_cap(&v2), MAX_PER_EPOCH);
    }

    #[test]
    fn lane_nonces_are_per_operator_and_never_touch_contract_slots() {
        let mut s = state();
        let a = Address::repeat_byte(1);
        assert_eq!(lane_nonce(&s, &a), 0);
        let before: Vec<U256> = (0u64..=9).map(|k| s.storage(&REGISTRY, U256::from(k))).collect();
        bump_lane_nonce(&mut s, &a);
        bump_lane_nonce(&mut s, &a);
        assert_eq!(lane_nonce(&s, &a), 2);
        assert_eq!(lane_nonce(&s, &Address::repeat_byte(2)), 0);
        // The tagged slot is far above every literal slot the contract uses:
        // none of them moves.
        let after: Vec<U256> = (0u64..=9).map(|k| s.storage(&REGISTRY, U256::from(k))).collect();
        assert_eq!(before, after, "the lane never touches the contract's own slots");
        assert_eq!(index_of(&s, &key(1)), 0);
    }
}
