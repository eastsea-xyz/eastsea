//! Fee policy v0 for a zero-value testnet (docs/research/tokenomics-2026.md §4).
//!
//! - Exec and prove base fees have EIP-4844-style exponential updates from
//!   the parent's excess. `base_exec` is burned (revm does not credit it);
//!   `base_prove` pays provers through the escrow. New-genesis state growth
//!   has a burned floor price and a congestion surcharge, even at zero
//!   exec/prove base. A rolling state-unit budget bounds sustained disk growth.
//! - Priority fees (tips) collect at `FEE_COLLECTOR` during the block and are
//!   split at the end: 60% proposer, 20% prover escrow, 20% burned (the burn
//!   floor makes self-paid fake tips cost the proposer).
//! - No fee-proportional rewards (R7). Issuance exists only for proven blocks
//!   (R7′, protocol 2) and lives in `proofs.rs`, not here.

use crate::world::WorldState;
use aether_types::{Address, FeeVector, GasVector, U256};
use alloy_primitives::address;

/// Where revm credits tips during a block (settled to zero at block end).
pub const FEE_COLLECTOR: Address = address!("00000000000000000000000000000000000fee00");
/// Prover escrow: base prove fees + the prover share of tips, paid to the
/// first valid proof of each block (R3, protocol 2: `proofs.rs`).
pub const PROVER_ESCROW: Address = address!("00000000000000000000000000000000000e5c00");

/// Scale of the base fee (R1′, docs/research/tokenomics-2026.md §6):
/// `base = SCALE · (e^(excess / (target · K)) − 1)`. Zero while blocks stay at or
/// under target (people without tokens can transact); sustained demand above
/// target raises it exponentially (spam pays for itself). One full block past
/// target costs ~1 gwei per unit, twelve ~13 gwei.
pub const SCALE: FeeVector = FeeVector { exec: 100_000_000_000, state: 0, prove: 100_000_000_000 };
/// Adjustment quotient (EIP-4844's role for K): how fast excess moves the fee.
pub const UPDATE_QUOTIENT: u64 = 96;
/// Tip split in percent: proposer, prover escrow, burn.
pub const TIP_SPLIT: (u64, u64, u64) = (60, 20, 20);

/// New-genesis persistent-byte and state growth price. New code costs one unit
/// per byte; a newly occupied storage slot or account costs 100 units.
/// Archived transaction/receipt bytes cost one unit per 32 bytes.
pub const STATE_UNIT_PRICE: u128 = 1_000_000_000_000;
pub const STATE_SLOT_UNITS: u64 = 100;
pub const STATE_ACCOUNT_UNITS: u64 = 100;
/// A conservative lower bound for a persisted receipt, including its hash,
/// status, gas, fee and length fields. Variable bytes are counted separately.
pub const RECEIPT_BASE_BYTES: u64 = 128;
/// Address, vector lengths and record/index overhead for each persisted event.
pub const EVENT_BASE_BYTES: u64 = 64;
pub const RECEIPT_BYTES_PER_STATE_UNIT: u64 = 32;
/// Consensus limits for new-genesis transactions, excluding capped system writes.
pub const MAX_STATE_UNITS_PER_BLOCK: u64 = 100_000;
/// Refill of the rolling new-genesis growth budget per finalized block (1 s).
/// A unit buys at most 32 archived logical bytes, hence at most 512 paid
/// stored bytes. 32 units/s bounds sustained paid rows to 1.416 GB/day,
/// plus a one-time 51.2 MB burst. Code/accounts/slots share this budget.
/// Retune through a committee-approved protocol upgrade; see design/27.
pub const STATE_UNITS_PER_BLOCK: u64 = 32;
/// Ordinary bursts keep the existing price. Congestion pricing begins after
/// half the 100,000-unit bucket is consumed; another 12,500 units multiplies
/// the floor by approximately e. The hard budget holds regardless of price.
pub const STATE_PRICE_FREE_BURST: u64 = MAX_STATE_UNITS_PER_BLOCK / 2;
pub const STATE_PRICE_UPDATE_UNITS: u64 = MAX_STATE_UNITS_PER_BLOCK / 8;
pub const MAX_NEW_SLOTS_PER_BLOCK: u64 = 512;
/// Upper bound for transaction-dependent redb key/value bytes per logical
/// metered byte. Receipt JSON hex doubles binary output/data and expands each
/// topic; account-history JSON and its reverse index can add rows per decoded
/// batch recipient or Transfer event. The 16x allowance covers those rows as
/// well as the signed transaction's staged copy and summary hash. It excludes
/// separately bounded protocol records and redb page/fragmentation overhead.
pub const MAX_STORED_BYTES_PER_METERED_BYTE: u64 = 16;
/// Maximum logical archived bytes in a new-genesis burst block. Sustained
/// growth is additionally bounded by the rolling state-unit budget above.
pub const MAX_PERSISTENT_BYTES_PER_BLOCK: u64 = 2 * 1024 * 1024;
/// Bound on the paid redb key/value representation at the logical block cap.
/// Separately bounded protocol records are not charged against it.
pub const MAX_PAID_STORED_BYTES_PER_BLOCK: u64 =
    MAX_PERSISTENT_BYTES_PER_BLOCK * MAX_STORED_BYTES_PER_METERED_BYTE;

/// Independent new-genesis archive budget, including all canonical payload
/// bytes: transaction envelopes, BAL, proofs and subsidized control traffic.
/// Retune only at a version-gated committee-approved protocol upgrade.
pub const MAX_ENCODED_PAYLOAD_BYTES: u64 = 8 << 20;
pub const ENCODED_PAYLOAD_BYTES_PER_BLOCK: u64 = 4096;
/// Planning allowance for staged/era and consensus archive copies.
pub const MAX_ARCHIVE_COPIES: u64 = 4;
/// Protected burst space for bounded handoff/seed/upgrade control payloads.
pub const CONTROL_ARCHIVE_RESERVE: u64 = 256 << 10;

pub fn encoded_payload_limit(excess: u64) -> u64 {
    MAX_ENCODED_PAYLOAD_BYTES.saturating_sub(excess)
}

pub fn next_archive_excess(excess: u64, used: u64) -> u64 {
    excess.saturating_add(used).saturating_sub(ENCODED_PAYLOAD_BYTES_PER_BLOCK)
}

/// Unspent burst capacity for the child of a finalized block. Encoding this
/// in the existing context's state limit also binds the prover to the budget,
/// without adding fields to legacy blocks, snapshots or proof statements.
pub fn state_block_limit(excess: u64) -> u64 {
    MAX_STATE_UNITS_PER_BLOCK.saturating_sub(excess)
}

/// Debt after a block. Refill is per height, never wall time or transaction.
pub fn next_state_excess(excess: u64, used: u64) -> u64 {
    excess.saturating_add(used).saturating_sub(STATE_UNITS_PER_BLOCK)
}

/// Burned state base price: unchanged for ordinary bursts, exponential under
/// sustained use, with an unconditional nonzero floor.
pub fn state_base_fee(excess: u64) -> u128 {
    fake_exponential(
        STATE_UNIT_PRICE,
        excess.saturating_sub(STATE_PRICE_FREE_BURST) as u128,
        STATE_PRICE_UPDATE_UNITS as u128,
    )
}

/// Headroom wallets sign over the state price, as they do over the exec base
/// fee (contracts-live bug #5): a tx queued behind a burst stays includable
/// through one doubling of the price. Only the actual price is charged.
pub const STATE_CAP_HEADROOM: u128 = 2;

/// The `max_fee.state` a wallet signs for a reported state `price`:
/// `STATE_CAP_HEADROOM × max(price, STATE_UNIT_PRICE)`. A chain that prices no
/// state (reported 0: the legacy chain) signs 0. Wallet policy, not consensus.
pub fn signed_state_cap(price: u128) -> u128 {
    if price == 0 {
        return 0;
    }
    price.max(STATE_UNIT_PRICE).saturating_mul(STATE_CAP_HEADROOM)
}

/// Blocks until the state price falls to `cap` or below, if no further state
/// is used: the debt refills `STATE_UNITS_PER_BLOCK` per height and the price
/// is `state_base_fee(debt)`. `Some(0)` when it already is; `None` when the
/// cap is under the floor, which no refill reaches. A wallet-facing estimate
/// read from the same math the chain prices with — never a consensus input.
pub fn blocks_until_state_price_at_most(excess: u64, cap: u128) -> Option<u64> {
    if state_base_fee(excess) <= cap {
        return Some(0);
    }
    if cap < STATE_UNIT_PRICE {
        return None;
    }
    // The price only falls as the debt falls, so the first height that meets
    // the cap is a binary search over the refill steps left.
    let (mut lo, mut hi) = (0u64, excess.div_ceil(STATE_UNITS_PER_BLOCK));
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let debt = excess.saturating_sub(mid.saturating_mul(STATE_UNITS_PER_BLOCK));
        if state_base_fee(debt) <= cap { hi = mid } else { lo = mid + 1 }
    }
    Some(lo)
}

/// What a block's fees follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// Excess after a block: exec/prove target half their limits; state debt
/// consumes actual units and refills by the fixed per-height allowance.
pub fn next_excess(excess: GasVector, used: GasVector, limits: GasVector) -> GasVector {
    let step = |e: u64, u: u64, l: u64| e.saturating_add(u).saturating_sub(l / 2);
    GasVector {
        exec: step(excess.exec, used.exec, limits.exec),
        state: if limits.state <= MAX_STATE_UNITS_PER_BLOCK && limits.state != 0 {
            next_state_excess(excess.state, used.state)
        } else { 0 },
        prove: step(excess.prove, used.prove, limits.prove),
    }
}

/// Base fees for a block with accumulated `excess`. Exec/prove are zero when
/// there is none; a finite nonzero state dimension has the growth floor.
pub fn base_fee(excess: GasVector, limits: GasVector) -> FeeVector {
    let dim = |scale: u128, e: u64, l: u64| {
        let target = (l / 2).max(1) as u128;
        fake_exponential(scale, e as u128, target * UPDATE_QUOTIENT as u128).saturating_sub(scale)
    };
    FeeVector {
        exec: dim(SCALE.exec, excess.exec, limits.exec),
        state: if limits.state > 0 && limits.state <= MAX_STATE_UNITS_PER_BLOCK {
            state_base_fee(excess.state)
        } else { 0 },
        prove: dim(SCALE.prove, excess.prove, limits.prove),
    }
}

/// Where a block's fees went (for receipts and tests).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settlement {
    /// Memory-only snapshots for exact incremental append previews, including
    /// whether settlement created an otherwise absent fee account.
    pub fee_accounts_before: [Option<aether_state::layout::BasicData>; 3],
    /// Account-presence and balance-touch flags before settlement adds BAL
    /// entries; populated by the block accumulator, never serialized.
    pub fee_bal_before: [(bool, bool); 3],
    pub tips: U256,
    pub to_proposer: U256,
    pub to_escrow: U256,
    pub burned_tips: U256,
    pub prove_fees: U256,
    /// State growth fees, removed from circulation like exec base fees.
    pub burned_state: U256,
}

/// End of block: split collected tips and move prove fees to the escrow.
pub fn settle(state: &mut WorldState, policy: &FeePolicy, prove_fees: U256) -> Settlement {
    let fee_accounts_before = [FEE_COLLECTOR, policy.proposer, PROVER_ESCROW]
        .map(|address| state.account(&address));
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
    Settlement { fee_accounts_before, fee_bal_before: [(false, false); 3], tips, to_proposer, to_escrow: credit, burned_tips, prove_fees, burned_state: U256::ZERO }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_archive_budget_bounds_sustained_system_payloads() {
        let mut debt = 0;
        let mut total = 0u64;
        for heights in 1..=86_400 {
            let used = encoded_payload_limit(debt);
            total += used;
            debt = next_archive_excess(debt, used);
            assert!(total <= MAX_ENCODED_PAYLOAD_BYTES + heights * ENCODED_PAYLOAD_BYTES_PER_BLOCK);
            assert_eq!(encoded_payload_limit(debt), ENCODED_PAYLOAD_BYTES_PER_BLOCK);
        }
        let combined_day = (32 * 512 + ENCODED_PAYLOAD_BYTES_PER_BLOCK * MAX_ARCHIVE_COPIES) * 86_400
            + MAX_STATE_UNITS_PER_BLOCK * 512 + MAX_ENCODED_PAYLOAD_BYTES * MAX_ARCHIVE_COPIES;
        assert_eq!(combined_day, 2_915_909_632);
        assert_eq!(encoded_payload_limit(u64::MAX), 0);
    }

    #[test]
    fn rolling_budget_bounds_every_interval_and_price_recovers() {
        let mut debt = 0;
        let mut used = 0u64;
        // Spend all available capacity, including the first full burst, then
        // sustain full blocks. No number of empty/cheap txs resets the debt.
        for blocks in 1..=86_400u64 {
            let available = state_block_limit(debt);
            used += available;
            debt = next_state_excess(debt, available);
            assert!(used <= MAX_STATE_UNITS_PER_BLOCK + blocks * STATE_UNITS_PER_BLOCK);
            assert_eq!(state_block_limit(debt), STATE_UNITS_PER_BLOCK);
        }
        assert!(state_base_fee(debt) > 50 * STATE_UNIT_PRICE);
        for _ in 0..MAX_STATE_UNITS_PER_BLOCK.div_ceil(STATE_UNITS_PER_BLOCK) {
            debt = next_state_excess(debt, 0);
        }
        assert_eq!(debt, 0);
        assert_eq!(state_base_fee(debt), STATE_UNIT_PRICE);
        assert_eq!(state_block_limit(u64::MAX), 0);
    }

    /// Contracts-live bug #5: a pending transaction whose state cap is under
    /// the price must be told how long the refill takes to bring it back.
    #[test]
    fn state_price_wait_follows_the_refill() {
        assert_eq!(blocks_until_state_price_at_most(0, STATE_UNIT_PRICE), Some(0));
        assert_eq!(blocks_until_state_price_at_most(90_000, STATE_UNIT_PRICE - 1), None, "under the floor: never");
        // The stress run's burst block left ~97,000 units of debt (~43x the floor).
        let debt = 97_147;
        assert!(state_base_fee(debt) > 40 * STATE_UNIT_PRICE);
        let cap = 2 * STATE_UNIT_PRICE;
        let k = blocks_until_state_price_at_most(debt, cap).expect("a cap above the floor is reached");
        assert!(state_base_fee(debt - k * STATE_UNITS_PER_BLOCK) <= cap, "met after k blocks");
        assert!(state_base_fee(debt - (k - 1) * STATE_UNITS_PER_BLOCK) > cap, "not one block sooner");
        // e^((d - 50,000)/12,500) <= 2 at d <= ~58,664: ~1,200 one-second blocks.
        assert!((1_150..1_250).contains(&k), "{k}");
        // At the floor price the debt has to fall to the free burst.
        let floor = blocks_until_state_price_at_most(debt, STATE_UNIT_PRICE).unwrap();
        assert_eq!(floor, (debt - STATE_PRICE_FREE_BURST).div_ceil(STATE_UNITS_PER_BLOCK));
    }

    #[test]
    fn wallets_sign_twice_the_state_price_never_under_the_floor() {
        assert_eq!(signed_state_cap(0), 0, "a chain without state pricing signs no state cap");
        assert_eq!(signed_state_cap(1), 2 * STATE_UNIT_PRICE, "a stale report below the floor signs twice the floor");
        assert_eq!(signed_state_cap(STATE_UNIT_PRICE), 2 * STATE_UNIT_PRICE);
        assert_eq!(signed_state_cap(43 * STATE_UNIT_PRICE), 86 * STATE_UNIT_PRICE);
        assert_eq!(signed_state_cap(u128::MAX), u128::MAX);
    }

    #[test]
    fn legacy_fee_dimensions_never_acquire_state_debt_or_price() {
        let used = GasVector { state: MAX_STATE_UNITS_PER_BLOCK, ..Default::default() };
        for state in [0, u64::MAX] {
            let limits = GasVector { exec: 30_000_000, state, prove: 200_000_000 };
            assert_eq!(next_excess(GasVector::default(), used, limits).state, 0);
            assert_eq!(base_fee(used, limits).state, 0);
        }
    }

    #[test]
    fn base_fee_is_zero_until_congested_then_exponential() {
        let limits = GasVector { exec: 30_000_000, state: 0, prove: 200_000_000 };
        assert_eq!(base_fee(GasVector::default(), limits), FeeVector::default(), "free while at or under target");
        // Blocks at target leave no excess.
        let half = GasVector { exec: limits.exec / 2, state: 0, prove: limits.prove / 2 };
        assert_eq!(next_excess(GasVector::default(), half, limits), GasVector::default());
        // One full block: ~1 gwei; twelve: ~13 gwei; it keeps climbing with demand.
        let one = base_fee(next_excess(GasVector::default(), limits, limits), limits).exec as f64 / 1e9;
        assert!((1.0..1.1).contains(&one), "{one}");
        let mut e = GasVector::default();
        for _ in 0..12 {
            e = next_excess(e, limits, limits);
        }
        let twelve = base_fee(e, limits).exec as f64 / 1e9;
        assert!((12.0..14.0).contains(&twelve), "{twelve}");
        let mut hot = e;
        for _ in 0..400 {
            hot = next_excess(hot, limits, limits);
        }
        assert!(base_fee(hot, limits).exec > 100 * base_fee(e, limits).exec, "sustained spam gets expensive");
        // Empty blocks bring it back to zero.
        let mut cool = e;
        for _ in 0..12 {
            cool = next_excess(cool, GasVector::default(), limits);
        }
        assert_eq!(base_fee(cool, limits), FeeVector::default());
    }
}
