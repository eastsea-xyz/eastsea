//! New-genesis registry v3 status words. Kept outside aether-execution so the
//! pinned proving guest and testnet 7780 program ID do not change.

use aether_execution::{registry, StateError, WorldState};
use alloy_primitives::{keccak256, Bytes, U256};
use std::sync::OnceLock;

/// Runtime compiled from contracts/src/CommitteeRegistryV3.sol with solc
/// 0.8.19, optimizer 200 runs. V1/v2 bytecode remains available to old chains.
pub fn code() -> Bytes {
    static CODE: OnceLock<Bytes> = OnceLock::new();
    CODE.get_or_init(|| Bytes::from(alloy_primitives::hex::decode(include_str!("committee_registry_v3.bin.hex").trim()).expect("valid registry v3 hex"))).clone()
}

pub fn is_v3(state: &WorldState) -> bool {
    state.code(&registry::REGISTRY) == code()
}

pub fn genesis(state: &mut WorldState) -> Result<(), StateError> {
    state.set_code(registry::REGISTRY, code())
}

fn mapping_slot(index: u64, slot: u64) -> U256 {
    U256::from_be_bytes(keccak256([U256::from(index).to_be_bytes::<32>().as_slice(), &U256::from(slot).to_be_bytes::<32>()].concat()).0)
}

/// Most recent status: the finalized block height and leaving flag.
pub fn availability(state: &WorldState, index: u64) -> Option<(u64, bool)> {
    let w = state.storage(&registry::REGISTRY, mapping_slot(index, 10));
    (!w.is_zero()).then(|| (((w >> 1usize).to::<u64>() - 1), (w & U256::from(1u8)) != U256::ZERO))
}

pub fn last_leaving(state: &WorldState, index: u64) -> Option<u64> {
    let w = state.storage(&registry::REGISTRY, mapping_slot(index, 11));
    (!w.is_zero()).then(|| w.to::<u64>() - 1)
}

/// A return in the observed epoch still counts as an announced sleep.
pub fn announced_for_epoch(state: &WorldState, index: u64, epoch: u64) -> bool {
    let Some((height, leaving)) = availability(state, index) else { return false };
    leaving || (height / registry::epoch_blocks(state) == epoch && last_leaving(state, index).is_some())
}

/// Match CommitteeRegistryV3._announce after a voting-key signature check.
pub fn set_availability(state: &mut WorldState, index: u64, height: u64, leaving: bool) {
    state.set_storage(registry::REGISTRY, mapping_slot(index, 10), (U256::from(height + 1) << 1usize) | U256::from(u8::from(leaving)));
    if leaving {
        state.set_storage(registry::REGISTRY, mapping_slot(index, 11), U256::from(height + 1));
    }
}
