//! Node rewards (docs/design/15-node-rewards.md), a genesis parameter of a new
//! network (off on testnet 7780: nothing here runs unless the genesis enabled it).
//!
//! Every block's issuance splits in two:
//! - **node share** (`issuance / 2`): pooled per epoch and paid, in the first
//!   block of the next epoch, to the operators whose Macs beaconed that epoch;
//! - **proof share** (the rest): paid to the first valid proof of the block,
//!   only to a registered operator and at most 1/16 of an epoch's proof share
//!   per operator per epoch. Fee escrow is paid uncapped.
//!
//! Whatever a cap or a missing operator leaves is never minted.
//!
//! Issuance (these networks only; 7780 keeps the yearly halving of
//! `proofs::issuance`): 1 AETH per block, lowered a little every day so that it
//! falls 15% a year (about half every 4.3 years, no halving cliff), down to a
//! floor of 0.1 AETH per block that stays (reached after about 14.2 years).
//!
//! ```text
//! issuance(h) = max(ISSUE_0 × DAILY^(h / DAY_BLOCKS), TAIL)     DAILY = 0.85^(1/365)
//! ```
//!
//! Node share of operator i for epoch e (integer arithmetic, rounded down):
//!
//! ```text
//! mac weight   m = answered_slots × (WARMUP_STEPS + level)       (0 ..= FULL)
//! FULL         = SLOTS × 2 × WARMUP_STEPS                          (= 112)
//! operator     w_i = max of its Macs' m                           (≤ FULL)
//! W            = Σ w_i over operators with w_i > 0                 (N of them)
//! share_i      = pool(e) × w_i / max(W, MAX_SHARE × FULL)
//! ```
//!
//! With fewer than 16 full-weight operators, `W < 16 × FULL`, so each gets
//! exactly `pool / 16`; with 16 or more, `W ≥ 16 × FULL` and the pool is shared
//! by weight. Since `w_i ≤ FULL`, nobody ever gets more than `pool / 16`.
//!
//! The warm-up level of a Mac (registry candidate, not wallet) runs 0..=14:
//! weight 0.5 at 0, 1.0 at 14, one step per day (+1 on a good day: ≥ 90% of
//! the day's slots answered; −1 otherwise). A day on which the network as a
//! whole answered < 70% of its slots moves nobody.

use aether_execution::proofs::{self, ClaimError};
use aether_execution::registry;
use aether_execution::{StateError, WorldState};
use alloy_primitives::{address, keccak256, Address, U256};
use std::collections::BTreeMap;

/// Where node-reward state lives (storage only, no code).
pub const REWARDS: Address = address!("0000000000000000000000000000000000007704");
/// No operator gets more than 1/MAX_SHARE of an epoch's node or proof share.
pub const MAX_SHARE: u64 = 16;
/// Beacon slots per epoch (`beacons`): four unpredictable moments an hour.
pub const SLOTS: u64 = 4;
/// Epochs per warm-up day.
pub const DAY_EPOCHS: u64 = 24;
/// Warm-up days from 0.5 to 1.0 (one level per day).
pub const WARMUP_STEPS: u64 = 14;
/// A full-weight Mac: all slots, warmed up.
pub const FULL: u64 = SLOTS * 2 * WARMUP_STEPS;
/// A good day: at least this percentage of the day's slots answered.
pub const GOOD_DAY_PERCENT: u64 = 90;
/// Below this network-wide response, a day is neutral.
pub const NEUTRAL_BELOW_PERCENT: u64 = 70;

/// Blocks per issuance step (one day of 1 s blocks).
pub const DAY_BLOCKS: u64 = 86_400;
/// 0.85^(1/365) in 1e18 fixed point (rounded down): 15% less issuance a year.
pub const DAILY: u128 = 999_554_841_771_249_391;
/// The floor: 0.1 AETH per block, forever.
pub const TAIL: u128 = proofs::ISSUE_0 / 10;
const WAD: u128 = 1_000_000_000_000_000_000;

const ENABLED: u64 = 0;
const TAG_MAC: u64 = 1;
const TAG_OPERATOR: u64 = 2;
// 3..=6: beacons (slots, slot hashes, day, per-Mac record).
const TAG_RESERVE: u64 = 7;

pub mod beacons;

fn tagged(tag: u64, low: U256) -> U256 {
    (U256::from(tag) << 200) | low
}

/// Genesis: turn node rewards on for this network.
pub fn enable(state: &mut WorldState) {
    state.set_storage(REWARDS, U256::from(ENABLED), U256::from(1u8));
}

/// Whether this network's genesis turned node rewards on.
pub fn enabled(state: &WorldState) -> bool {
    !state.storage(&REWARDS, U256::from(ENABLED)).is_zero()
}

/// `DAILY^n` in 1e18 fixed point, rounded down at every multiplication
/// (square-and-multiply, so every node gets the same bits).
fn daily_pow(mut n: u64) -> U256 {
    let wad = U256::from(WAD);
    let mut result = wad;
    let mut base = U256::from(DAILY);
    while n > 0 {
        if n & 1 == 1 {
            result = result * base / wad;
        }
        base = base * base / wad;
        n >>= 1;
    }
    result
}

/// New tokens of block `height` on a network with node rewards.
pub fn issuance(height: u64) -> U256 {
    // Past ~14.2 years the decay is below the floor for good; skip the power.
    const FLOOR_DAYS: u64 = 5_200;
    let day = height / DAY_BLOCKS;
    if day >= FLOOR_DAYS {
        return U256::from(TAIL);
    }
    (U256::from(proofs::ISSUE_0) * daily_pow(day) / U256::from(WAD)).max(U256::from(TAIL))
}

/// Node share of block `height`'s issuance.
pub fn node_share(height: u64) -> U256 {
    issuance(height) / U256::from(2u8)
}

/// Proof share of block `height`'s issuance (the rest).
pub fn proof_share(height: u64) -> U256 {
    issuance(height) - node_share(height)
}

/// Σ f(h) over blocks h in epoch `epoch` (height 0 is the genesis: no issuance).
/// Constant within a day, so summed per day.
fn epoch_sum(epoch: u64, epoch_blocks: u64, f: fn(u64) -> U256) -> U256 {
    let start = epoch.saturating_mul(epoch_blocks).max(1);
    let end = epoch.saturating_add(1).saturating_mul(epoch_blocks);
    let mut total = U256::ZERO;
    let mut h = start;
    while h < end {
        let next = ((h / DAY_BLOCKS).saturating_add(1)).saturating_mul(DAY_BLOCKS).min(end);
        total += f(h) * U256::from(next - h);
        h = next;
    }
    total
}

/// The node pool of epoch `epoch`.
pub fn node_pool(epoch: u64, epoch_blocks: u64) -> U256 {
    epoch_sum(epoch, epoch_blocks, node_share)
}

/// The proof share of epoch `epoch`.
pub fn proof_pool(epoch: u64, epoch_blocks: u64) -> U256 {
    epoch_sum(epoch, epoch_blocks, proof_share)
}

/// A Mac's warm-up state, one storage word per registry candidate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mac {
    /// Warm-up level 0..=WARMUP_STEPS (weight (WARMUP_STEPS + level) / (2 × WARMUP_STEPS)).
    pub level: u64,
    /// Slots answered so far in the current day.
    pub answered: u64,
    /// Slots answered in the previous day.
    pub previous: u64,
}

impl Mac {
    fn pack(self) -> U256 {
        U256::from(self.level) | (U256::from(self.answered) << 64) | (U256::from(self.previous) << 128)
    }
    fn unpack(w: U256) -> Self {
        let mask = U256::from(u64::MAX);
        let field = |shift: usize| ((w >> shift) & mask).to::<u64>();
        Mac { level: field(0), answered: field(64), previous: field(128) }
    }
    /// Weight per slot answered (WARMUP_STEPS..=2 × WARMUP_STEPS).
    pub fn warmup(self) -> u64 {
        WARMUP_STEPS + self.level.min(WARMUP_STEPS)
    }
}

/// Warm-up state of registry candidate `index`.
pub fn mac(state: &WorldState, index: u64) -> Mac {
    Mac::unpack(state.storage(&REWARDS, tagged(TAG_MAC, U256::from(index))))
}

fn set_mac(state: &mut WorldState, index: u64, m: Mac) {
    let slot = tagged(TAG_MAC, U256::from(index));
    if state.storage(&REWARDS, slot) != m.pack() {
        state.set_storage(REWARDS, slot, m.pack());
    }
}

/// What an epoch's distribution paid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Distribution {
    pub epoch: u64,
    pub pool: U256,
    /// Operators paid, by address.
    pub paid: Vec<(Address, U256)>,
    /// Pool left unminted.
    pub unminted: U256,
}

/// Whether block `height` pays an epoch's node rewards (the first block of the next epoch).
pub fn distributes(state: &WorldState, height: u64) -> bool {
    height > 0 && enabled(state) && height.is_multiple_of(registry::epoch_blocks(state))
}

/// Run by the first block of an epoch, before its transactions (a system
/// write, like proof payouts): count the last epoch's beacons, pay its node
/// pool, and at the end of a day move every Mac's warm-up level.
pub fn distribute(state: &mut WorldState, height: u64) -> Result<Distribution, StateError> {
    let epoch_blocks = registry::epoch_blocks(state);
    let epoch = height / epoch_blocks - 1;
    let candidates = registry::candidates(state);
    let mut macs: Vec<Mac> = candidates.iter().map(|c| mac(state, c.index)).collect();

    // Weights use the warm-up level the Macs had during the epoch.
    let mut weights: BTreeMap<Address, u64> = BTreeMap::new();
    for (c, m) in candidates.iter().zip(macs.iter_mut()) {
        // Slots this Mac answered (signed, and re-attested when due) in the epoch.
        let answered = beacons::beacon(state, c.index).answered(epoch);
        m.answered += answered;
        let w = answered * m.warmup();
        if w > 0 {
            let e = weights.entry(c.operator).or_default();
            *e = (*e).max(w);
        }
    }
    let pool = node_pool(epoch, epoch_blocks);
    let total: u64 = weights.values().sum();
    let denominator = U256::from(total.max(MAX_SHARE * FULL));
    let mut paid = Vec::with_capacity(weights.len());
    let mut minted = U256::ZERO;
    for (operator, w) in weights {
        let amount = pool * U256::from(w) / denominator;
        if !amount.is_zero() {
            state.set_balance(operator, state.balance(&operator) + amount)?;
            minted += amount;
        }
        paid.push((operator, amount));
    }

    if (epoch + 1).is_multiple_of(DAY_EPOCHS) {
        end_day(&candidates, &mut macs, epoch + 1 - DAY_EPOCHS);
    }
    for (c, m) in candidates.iter().zip(macs) {
        set_mac(state, c.index, m);
    }
    Ok(Distribution { epoch, pool, paid, unminted: pool - minted })
}

/// A day (epochs `first..first + DAY_EPOCHS`) ended: move warm-up levels.
fn end_day(candidates: &[registry::Candidate], macs: &mut [Mac], first: u64) {
    let day_slots = DAY_EPOCHS * SLOTS;
    // Macs registered for the whole day; long-silent ones (nothing today or
    // yesterday) do not drag the network rate down forever.
    let full_day: Vec<usize> = (0..candidates.len()).filter(|&i| candidates[i].registered_epoch <= first).collect();
    let active: Vec<usize> = full_day.iter().copied().filter(|&i| macs[i].answered > 0 || macs[i].previous > 0).collect();
    let answered: u64 = active.iter().map(|&i| macs[i].answered).sum();
    let possible = active.len() as u64 * day_slots;
    let neutral = possible == 0 || answered * 100 < possible * NEUTRAL_BELOW_PERCENT;
    if !neutral {
        for &i in &full_day {
            let m = &mut macs[i];
            m.level = if m.answered * 100 >= day_slots * GOOD_DAY_PERCENT {
                (m.level + 1).min(WARMUP_STEPS)
            } else {
                m.level.saturating_sub(1)
            };
        }
    }
    for m in macs.iter_mut() {
        m.previous = m.answered;
        m.answered = 0;
    }
}

/// Whether `who` registered a Mac (is some candidate's operator).
/// TODO(scale): an operator index instead of a scan once registrations grow.
pub fn is_operator(state: &WorldState, who: Address) -> bool {
    registry::candidates(state).iter().any(|c| c.operator == who)
}

/// Proof issuance an operator was paid in an epoch: (epoch, amount), one word per operator.
fn operator_slot(who: Address) -> U256 {
    tagged(TAG_OPERATOR, U256::from_be_slice(who.as_slice()))
}

/// Proof issuance `who` was paid so far in epoch `epoch`.
pub fn proof_paid(state: &WorldState, who: Address, epoch: u64) -> U256 {
    let w = state.storage(&REWARDS, operator_slot(who));
    if (w >> 192usize).to::<u64>() == epoch && !w.is_zero() {
        w & ((U256::from(1u8) << 192) - U256::from(1u8))
    } else {
        U256::ZERO
    }
}

/// Pay the first valid proof of block `height` under node rewards: its fee
/// escrow in full, plus the block's proof share only to a registered operator
/// and only up to 1/16 of the proof share of the paying block's epoch (`now`).
/// Returns (paid in total, issuance minted).
pub fn pay_proof(state: &mut WorldState, height: u64, now: u64, prover: Address) -> Result<(U256, U256), ClaimError> {
    proofs::claimable(state, height, now)?;
    let issued = if is_operator(state, prover) {
        let epoch_blocks = registry::epoch_blocks(state);
        let epoch = now / epoch_blocks;
        let cap = proof_pool(epoch, epoch_blocks) / U256::from(MAX_SHARE);
        let so_far = proof_paid(state, prover, epoch);
        let issued = proof_share(height).min(cap.saturating_sub(so_far));
        if !issued.is_zero() {
            state.set_storage(REWARDS, operator_slot(prover), (U256::from(epoch) << 192) | (so_far + issued));
        }
        issued
    } else {
        U256::ZERO
    };
    // `proofs::pay` credits the testnet's halving issuance; this network's own
    // issuance replaces it in the same system write: what the rules withhold is
    // never minted, and after the first year (when the decay pays more than the
    // halving would) the difference is added. (`proofs.rs` stays byte-identical:
    // the pinned proving program is built from this crate.)
    let paid = proofs::pay(state, height, now, prover)?;
    let credited = proofs::issuance(height);
    state.set_balance(prover, state.balance(&prover) - credited + issued).map_err(ClaimError::State)?;
    Ok((paid - credited + issued, issued))
}

/// Founder reserve keys (docs/design/12-launch-plan.md, "창업자 Mac 안전망"): at most this many.
pub const MAX_RESERVE_KEYS: usize = 3;

/// A reserve key: (ed25519 voting key, iroh node id).
pub type ReserveKey = ([u8; 32], [u8; 32]);

/// Genesis: the founder's reserve validator keys (ed25519 key, iroh node id)
/// and the operator address they belong to. They are not registry candidates:
/// they answer no beacons and earn no node rewards; nodes seat them only while
/// fewer than `MIN_OPEN_COMMITTEE` independent operators qualify.
pub fn set_reserve(state: &mut WorldState, operator: Address, keys: &[ReserveKey]) -> Result<(), String> {
    if keys.is_empty() || keys.len() > MAX_RESERVE_KEYS {
        return Err(format!("1 to {MAX_RESERVE_KEYS} reserve keys"));
    }
    if !enabled(state) {
        return Err("reserve keys need node rewards".into());
    }
    let head = U256::from_be_slice(operator.as_slice()) | (U256::from(keys.len()) << 160);
    state.set_storage(REWARDS, tagged(TAG_RESERVE, U256::ZERO), head);
    for (i, (key, node)) in keys.iter().enumerate() {
        state.set_storage(REWARDS, tagged(TAG_RESERVE, U256::from(1 + 2 * i)), U256::from_be_bytes(*key));
        state.set_storage(REWARDS, tagged(TAG_RESERVE, U256::from(2 + 2 * i)), U256::from_be_bytes(*node));
    }
    Ok(())
}

/// The founder's reserve keys set at genesis: (operator, [(key, node id)]).
pub fn reserve(state: &WorldState) -> Option<(Address, Vec<ReserveKey>)> {
    let head = state.storage(&REWARDS, tagged(TAG_RESERVE, U256::ZERO));
    let n = (head >> 160usize).to::<u64>() as usize;
    if n == 0 {
        return None;
    }
    let operator = Address::from_slice(&head.to_be_bytes::<32>()[12..]);
    let word = |k: usize| state.storage(&REWARDS, tagged(TAG_RESERVE, U256::from(k))).to_be_bytes::<32>();
    Some((operator, (0..n.min(MAX_RESERVE_KEYS)).map(|i| (word(1 + 2 * i), word(2 + 2 * i))).collect()))
}

/// Write `c` as registry candidate `c.index` in the contract's storage layout
/// (tests and simulations; on chain only the contract writes candidates).
#[doc(hidden)]
pub fn put_candidate(state: &mut WorldState, c: &registry::Candidate) {
    let reg = registry::REGISTRY;
    let base = U256::from_be_bytes(keccak256(U256::from(2u64).to_be_bytes::<32>()).0);
    let at = |k: u64| base + U256::from(5 * c.index + k);
    if c.index >= state.storage(&reg, U256::from(2u64)).to::<u64>() {
        state.set_storage(reg, U256::from(2u64), U256::from(c.index + 1));
    }
    state.set_storage(reg, at(0), U256::from_be_slice(c.operator.as_slice()));
    state.set_storage(reg, at(1), U256::from_be_bytes(c.validator_key));
    state.set_storage(reg, at(2), U256::from_be_bytes(c.node_id));
    state.set_storage(reg, at(3), U256::from_be_slice(c.beaconer.as_slice()) | (U256::from(c.registered_epoch) << 160usize));
    state.set_storage(reg, at(4), U256::from(c.last_epoch) | (U256::from(c.streak) << 64usize) | (U256::from(c.missed) << 128usize));
}

#[cfg(test)]
mod tests;
