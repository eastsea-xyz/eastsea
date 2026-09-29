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

/// Blocks per epoch as set at genesis.
pub fn epoch_blocks(state: &WorldState) -> u64 {
    state.storage(&REGISTRY, U256::from(4u64)).to::<u64>().max(1)
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
