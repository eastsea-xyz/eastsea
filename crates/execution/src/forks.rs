//! State changes a protocol upgrade makes once, at its activation block,
//! before that block's transactions (docs/design/12-launch-plan.md step 5).
//!
//! The genesis never changes: a later protocol that needs new predeploys or
//! fixed contract code adds its arm here (e.g. `2 => state.set_code(..)`), the
//! committee signs the upgrade with an activation height, and every node applies
//! the same change at that height. Account contract fixes deploy at a new
//! address instead; accounts move to it by delegating again (EIP-7702).

use crate::world::{StateError, WorldState};

/// How a protocol's one-time changes are applied (tests substitute their own).
pub type Migration = fn(u32, &mut WorldState) -> Result<(), StateError>;

/// The changes protocol `protocol` makes when it activates. Protocol 1 is the genesis.
pub fn activate(protocol: u32, state: &mut WorldState) -> Result<(), StateError> {
    // No protocol after 1 yet.
    let _ = (protocol, state);
    Ok(())
}
