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

sol! {
    function register(bytes32 validatorKey, bytes32 nodeId, address beaconer, bytes32 r, bytes32 s);
    function beacon(bytes32 validatorKey);
}

/// Runtime bytecode of `CommitteeRegistry` (solc 0.8.19, optimizer 200 runs).
pub fn code() -> Bytes {
    Bytes::from(alloy_primitives::hex::decode(include_str!("committee_registry.bin.hex").trim()).expect("valid hex"))
}

/// Genesis: the registry with the registrar's P-256 key (x, y) in slots 0 and 1
/// and the epoch length in slot 4.
pub fn predeploy(state: &mut WorldState, registrar: ([u8; 32], [u8; 32]), epoch_blocks: u64) -> Result<(), crate::world::StateError> {
    state.set_code(REGISTRY, code())?;
    state.set_storage(REGISTRY, U256::ZERO, U256::from_be_bytes(registrar.0));
    state.set_storage(REGISTRY, U256::from(1u64), U256::from_be_bytes(registrar.1));
    state.set_storage(REGISTRY, U256::from(4u64), U256::from(epoch_blocks.max(1)));
    Ok(())
}

/// Blocks per epoch as set at genesis.
pub fn epoch_blocks(state: &WorldState) -> u64 {
    state.storage(&REGISTRY, U256::from(4u64)).to::<u64>().max(1)
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
            }
        })
        .collect()
}
