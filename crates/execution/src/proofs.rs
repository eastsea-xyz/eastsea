//! Proof market state (protocol 2, docs/design/13-protocol-2.md).
//!
//! Before its transactions, block h+1 records block h's statement commitment
//! and escrow share in the state tree (storage of `PROVER_ESCROW`, like
//! EIP-2935 keeps block hashes), so every node, also one started from a
//! checkpoint, checks proofs of past blocks the same way. The first valid proof
//! of a block pays its prover that block's escrow plus the block's issuance
//! (R7′: halving yearly, nothing for blocks nobody proved).

use crate::fees::PROVER_ESCROW;
use crate::world::{StateError, WorldState};
use aether_types::{Address, U256};

/// Issuance of block 0 (1 AETH), halved every `HALVING` blocks.
pub const ISSUE_0: u128 = 1_000_000_000_000_000_000;
/// ~1 year of 1 s blocks.
pub const HALVING: u64 = 31_536_000;
/// A block can be proven (and paid) for ~30 days of 1 s blocks.
pub const EXPIRY: u64 = 2_592_000;

const COMMITMENT: u64 = 0;
const ESCROW: u64 = 1;
const PROVER: u64 = 2;

fn slot(height: u64, field: u64) -> U256 {
    // Four slots per height; heights stay far below 2^254.
    (U256::from(height) << 2) + U256::from(field)
}

/// New tokens for proving block `height`.
pub fn issuance(height: u64) -> U256 {
    let halvings = height / HALVING;
    if halvings >= 128 {
        return U256::ZERO;
    }
    U256::from(ISSUE_0 >> halvings)
}

/// Record block `height`'s statement commitment and escrow share (by block height + 1).
pub fn record(state: &mut WorldState, height: u64, commitment: [u8; 32], escrow: U256) {
    state.set_storage(PROVER_ESCROW, slot(height, COMMITMENT), U256::from_be_bytes(commitment));
    state.set_storage(PROVER_ESCROW, slot(height, ESCROW), escrow);
}

/// Block `height`'s recorded statement commitment, if any.
pub fn commitment(state: &WorldState, height: u64) -> Option<[u8; 32]> {
    let c = state.storage(&PROVER_ESCROW, slot(height, COMMITMENT));
    (!c.is_zero()).then(|| c.to_be_bytes::<32>())
}

/// Who proved block `height` (None: nobody yet).
pub fn prover(state: &WorldState, height: u64) -> Option<Address> {
    let p = state.storage(&PROVER_ESCROW, slot(height, PROVER));
    (!p.is_zero()).then(|| Address::from_word(p.into()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimError {
    NotRecorded,
    AlreadyProven,
    Expired,
    State(StateError),
}

/// Whether a proof of block `height`, included at height `now`, may be paid.
pub fn claimable(state: &WorldState, height: u64, now: u64) -> Result<[u8; 32], ClaimError> {
    let c = commitment(state, height).ok_or(ClaimError::NotRecorded)?;
    if prover(state, height).is_some() {
        return Err(ClaimError::AlreadyProven);
    }
    if now.saturating_sub(height) > EXPIRY {
        return Err(ClaimError::Expired);
    }
    Ok(c)
}

/// Pay the first valid proof of block `height` (the caller verified it against
/// `claimable`'s commitment): the block's escrow share moves to `prover` and
/// the block's issuance is minted to it. Returns the amount paid.
pub fn pay(state: &mut WorldState, height: u64, now: u64, prover_addr: Address) -> Result<U256, ClaimError> {
    claimable(state, height, now)?;
    let escrow = state.storage(&PROVER_ESCROW, slot(height, ESCROW)).min(state.balance(&PROVER_ESCROW));
    let paid = escrow + issuance(height);
    state.set_balance(PROVER_ESCROW, state.balance(&PROVER_ESCROW) - escrow).map_err(ClaimError::State)?;
    state.set_balance(prover_addr, state.balance(&prover_addr) + paid).map_err(ClaimError::State)?;
    // A zero address would read as "not proven"; the marker is the address with its top bit set.
    let marker = U256::from_be_slice(prover_addr.as_slice()) | (U256::from(1u8) << 255);
    state.set_storage(PROVER_ESCROW, slot(height, PROVER), marker);
    Ok(paid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_proof_is_paid_once_with_escrow_and_halving_issuance() {
        let mut s = WorldState::default();
        s.set_balance(PROVER_ESCROW, U256::from(500u64)).unwrap();
        record(&mut s, 7, [9; 32], U256::from(300u64));
        let p = Address::repeat_byte(0x11);
        assert_eq!(claimable(&s, 7, 8), Ok([9; 32]));
        assert_eq!(pay(&mut s, 7, 8, p), Ok(U256::from(300u64) + U256::from(ISSUE_0)));
        assert_eq!(s.balance(&p), U256::from(300u64) + U256::from(ISSUE_0));
        assert_eq!(s.balance(&PROVER_ESCROW), U256::from(200u64));
        assert_eq!(pay(&mut s, 7, 9, p), Err(ClaimError::AlreadyProven));
        assert!(prover(&s, 7).is_some());
        assert_eq!(claimable(&s, 8, 9), Err(ClaimError::NotRecorded));
        record(&mut s, 8, [1; 32], U256::ZERO);
        assert_eq!(claimable(&s, 8, 8 + EXPIRY + 1), Err(ClaimError::Expired));
        assert_eq!(issuance(HALVING), U256::from(ISSUE_0 / 2));
        assert_eq!(issuance(HALVING * 200), U256::ZERO);
        // Even the zero address is recorded as a prover.
        record(&mut s, 10, [2; 32], U256::ZERO);
        pay(&mut s, 10, 11, Address::ZERO).unwrap();
        assert_eq!(pay(&mut s, 10, 11, p), Err(ClaimError::AlreadyProven));
    }
}
