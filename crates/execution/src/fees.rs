//! Fee policy v0 for a zero-value testnet (docs/research/tokenomics-2026.md §4).
//!
//! - Base fee per dimension (exec, prove) with EIP-4844-style exponential
//!   updates from the parent's excess. `base_exec` is burned (revm does not
//!   credit it); `base_prove` pays provers through the escrow.
//! - Priority fees (tips) collect at `FEE_COLLECTOR` during the block and are
//!   split at the end: 60% proposer, 20% prover escrow, 20% burned (the burn
//!   floor makes self-paid fake tips cost the proposer).
//! - No issuance, no fee-proportional rewards (R7).

use crate::world::WorldState;
use aether_types::{Address, FeeVector, GasVector, U256};
use alloy_primitives::address;

/// Where revm credits tips during a block (settled to zero at block end).
pub const FEE_COLLECTOR: Address = address!("00000000000000000000000000000000000fee00");
/// Prover escrow: base prove fees + the prover share of tips, paid per proven
/// chunk later (R3; claims come with the proof market).
pub const PROVER_ESCROW: Address = address!("00000000000000000000000000000000000e5c00");

/// Minimum base fee per unit (R1): 1 gwei.
pub const FLOOR: FeeVector = FeeVector { exec: 1_000_000_000, state: 0, prove: 1_000_000_000 };
/// Adjustment quotient: a full block raises the base fee ~1.05%, an empty one lowers it ~1.04%.
pub const UPDATE_QUOTIENT: u64 = 96;
/// Tip split in percent: proposer, prover escrow, burn.
pub const TIP_SPLIT: (u64, u64, u64) = (60, 20, 20);

/// What a block's fees follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeePolicy {
    pub base: FeeVector,
    /// Receives the proposer share of tips.
    pub proposer: Address,
}

/// `factor * e^(numerator / denominator)`, EIP-4844's integer approximation.
pub fn fake_exponential(factor: u128, numerator: u128, denominator: u128) -> u128 {
    let mut i = 1u128;
    let mut output = 0u128;
    let mut accum = factor.saturating_mul(denominator);
    while accum > 0 {
        output = output.saturating_add(accum);
        accum = accum.saturating_mul(numerator) / denominator.saturating_mul(i);
        i += 1;
        if i > 1_000 {
            break;
        }
    }
    output / denominator
}

/// Excess after a block that used `used` on top of `excess` (targets = half the limits).
pub fn next_excess(excess: GasVector, used: GasVector, limits: GasVector) -> GasVector {
    let step = |e: u64, u: u64, l: u64| e.saturating_add(u).saturating_sub(l / 2);
    GasVector { exec: step(excess.exec, used.exec, limits.exec), state: 0, prove: step(excess.prove, used.prove, limits.prove) }
}

/// Base fees for a block with accumulated `excess`.
pub fn base_fee(excess: GasVector, limits: GasVector) -> FeeVector {
    let dim = |floor: u128, e: u64, l: u64| {
        let target = (l / 2).max(1) as u128;
        fake_exponential(floor, e as u128, target * UPDATE_QUOTIENT as u128).max(floor)
    };
    FeeVector { exec: dim(FLOOR.exec, excess.exec, limits.exec), state: 0, prove: dim(FLOOR.prove, excess.prove, limits.prove) }
}

/// Where a block's fees went (for receipts and tests).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settlement {
    pub tips: U256,
    pub to_proposer: U256,
    pub to_escrow: U256,
    pub burned_tips: U256,
    pub prove_fees: U256,
}

/// End of block: split collected tips and move prove fees to the escrow.
pub fn settle(state: &mut WorldState, policy: &FeePolicy, prove_fees: U256) -> Settlement {
    let tips = state.balance(&FEE_COLLECTOR);
    let (p, e, _) = TIP_SPLIT;
    let to_proposer = tips * U256::from(p) / U256::from(100u64);
    let to_escrow = tips * U256::from(e) / U256::from(100u64);
    let burned_tips = tips - to_proposer - to_escrow;
    if !tips.is_zero() {
        state.set_balance(FEE_COLLECTOR, U256::ZERO).expect("zero fits");
        let pb = state.balance(&policy.proposer) + to_proposer;
        state.set_balance(policy.proposer, pb).expect("proposer balance fits");
    }
    let credit = to_escrow + prove_fees;
    if !credit.is_zero() {
        let eb = state.balance(&PROVER_ESCROW) + credit;
        state.set_balance(PROVER_ESCROW, eb).expect("escrow balance fits");
    }
    Settlement { tips, to_proposer, to_escrow: credit, burned_tips, prove_fees }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_fee_moves_like_eip_4844() {
        let limits = GasVector { exec: 30_000_000, state: 0, prove: 200_000_000 };
        let floor = base_fee(GasVector::default(), limits);
        assert_eq!(floor, FeeVector { exec: FLOOR.exec, state: 0, prove: FLOOR.prove });
        // A full block: +~1.05%.
        let e = next_excess(GasVector::default(), limits, limits);
        let up = base_fee(e, limits);
        let ratio = up.exec as f64 / FLOOR.exec as f64;
        assert!((1.0100..1.0110).contains(&ratio), "{ratio}");
        // 12 full blocks: ~+13%.
        let mut e = GasVector::default();
        for _ in 0..12 {
            e = next_excess(e, limits, limits);
        }
        let r12 = base_fee(e, limits).exec as f64 / FLOOR.exec as f64;
        assert!((1.12..1.15).contains(&r12), "{r12}");
        // Empty blocks pull the excess back down; never below the floor.
        let down = next_excess(e, GasVector::default(), limits);
        assert!(base_fee(down, limits).exec < base_fee(e, limits).exec);
        assert_eq!(base_fee(next_excess(GasVector::default(), GasVector::default(), limits), limits).exec, FLOOR.exec);
    }
}
