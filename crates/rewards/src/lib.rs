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
//! `proofs::issuance`): 1 DBLN per block, lowered a little every day so that it
//! falls 15% a year (about half every 4.3 years, no halving cliff), down to a
//! floor of 0.1 DBLN per block that stays (reached after about 14.2 years).
//!
//! ```text
//! issuance(h) = max(ISSUE_0 × DAILY^(h / DAY_BLOCKS), TAIL)     DAILY = 0.85^(1/365)
//! ```
//!
//! Node share of operator i for epoch e (integer arithmetic, rounded down):
//!
//! ```text
//! mac weight   m = answered_slots × (WARMUP_STEPS + level)       (0 ..= FULL)
//! FULL         = SLOTS × 2 × WARMUP_STEPS                          (= 336)
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
//!
//! An epoch the founder's reserve keys served counts, for the founder's
//! operator alone, as its Mac answering every slot — at the warm-up it already
//! has (the credit is a weight: it moves no day count, no level, and an
//! operator still takes the max over its Macs, so it adds no second share).
//! Serving means seated in the voting committee for the whole epoch, a record
//! the chain writes at each committee switch (`switch_reserve`, read by
//! `reserve_served`).

use aether_execution::proofs::{self, ClaimError};
use aether_execution::registry;
use aether_execution::{StateError, WorldState};
use alloy_primitives::{address, keccak256, Address, Bytes, U256};
use std::collections::BTreeMap;

/// Where node-reward state lives (new-genesis networks also predeploy the read-only randomness view).
pub const REWARDS: Address = address!("0000000000000000000000000000000000007704");

/// Read-only `randomness(uint64)` runtime predeployed at REWARDS on new-genesis
/// networks. It reads only the tagged epoch slot; no external call can write
/// rewards storage. Built from contracts/src/Randomness.sol with solc 0.8.19.
pub fn randomness_code() -> Bytes {
    Bytes::from(alloy_primitives::hex::decode(include_str!("randomness.bin.hex").trim()).expect("valid randomness runtime"))
}
/// No operator gets more than 1/MAX_SHARE of an epoch's node or proof share.
pub const MAX_SHARE: u64 = 16;
/// Beacon slots per epoch (`beacons`): twelve unpredictable moments an hour,
/// each unknown until roughly an answer window before it.
pub const SLOTS: u64 = 12;
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
/// The floor: 0.1 DBLN per block, forever.
pub const TAIL: u128 = proofs::ISSUE_0 / 10;
const WAD: u128 = 1_000_000_000_000_000_000;

const ENABLED: u64 = 0;
const TAG_MAC: u64 = 1;
const TAG_OPERATOR: u64 = 2;
// 3..=6: beacons (slots, slot hashes, day, per-Mac record).
const TAG_RESERVE: u64 = 7;
/// Seating of the founder's reserve keys, rewritten at each committee switch.
const TAG_SEATED: u64 = 8;
// 9..=10 are the randomness and genesis ceiling words; 11..=13 are the
// beacons profile words. Reserve-hardening tags start past them.
/// The running voting committee: the genesis roster first, then the members of
/// each handoff from the block they take over at (a chain system write, so
/// every node — validator, follower, or syncing from a snapshot — holds the
/// same committee in state).
const TAG_COMMITTEE: u64 = 16;
/// The voting set the chain committed to for the current draw: the draw seed's
/// decision, or a reserve reseat's. Only a handoff naming exactly these
/// members may hand the committee over (chain.rs `next_handoff`).
const TAG_ROSTER: u64 = 17;
/// The draw's candidate pool, frozen at its first block from the registry
/// state that block builds on (the same `rotation::eligible` list the draw
/// reads).
const TAG_POOL: u64 = 14;
/// Consecutive registry epochs the founder's reserve keys have held seats
/// while five or more independent operators qualified (`TAG_OVERDUE, 0` =
/// `epoch | count << 64`): past `RESERVE_GRACE_EPOCHS` of them the service
/// credit stops.
const TAG_OVERDUE: u64 = 15;

/// The epoch randomness words, written by its first block.
const TAG_RANDOM: u64 = 9;
/// New-genesis voting-set ceiling, committed by the genesis state root.
const TAG_MAX_COMMITTEE: u64 = 10;

/// Every storage tag of REWARDS (here and in beacons.rs), in one list: a new
/// record takes the next free number (two records once shared tag 8).
#[cfg(test)]
pub(crate) const ALL_TAGS: [u64; 19] = [
    ENABLED,
    TAG_MAC,
    TAG_OPERATOR,
    TAG_RESERVE,
    TAG_SEATED,
    TAG_COMMITTEE,
    TAG_ROSTER,
    TAG_POOL,
    TAG_OVERDUE,
    TAG_RANDOM,
    TAG_MAX_COMMITTEE,
    beacons::TAG_SLOTS,
    beacons::TAG_SLOT_HASH,
    beacons::TAG_DAY,
    beacons::TAG_BEACON,
    beacons::TAG_PROFILE,
    beacons::TAG_OFFERED,
    beacons::TAG_RECENT,
    beacons::TAG_STABILITY,
];

pub mod beacons;
pub mod registry_v3;

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

/// Bind the committee ceiling to the new genesis state root.
pub fn set_max_committee(state: &mut WorldState, size: u64) {
    state.set_storage(REWARDS, tagged(TAG_MAX_COMMITTEE, U256::ZERO), U256::from(size));
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

/// Write candidate `index`'s warm-up state directly (tests and simulations;
/// on chain only `distribute` writes it).
#[doc(hidden)]
pub fn put_mac(state: &mut WorldState, index: u64, m: Mac) {
    set_mac(state, index, m)
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

/// Every operator's weight for an epoch: each candidate's `answered` slots ×
/// its Mac's warm-up weight, an operator's taking the max over its Macs (extra
/// Macs do not raise the 1/16 cap). `distribute` and the read-only view behind
/// `aether_rewardStatus` both count through this, so the two can never
/// disagree about N or about an operator's weight. `macs` and `answered` line
/// up with `candidates`.
pub fn operator_weights(
    candidates: &[registry::Candidate],
    macs: &[Mac],
    answered: &[u64],
) -> BTreeMap<Address, u64> {
    let mut weights: BTreeMap<Address, u64> = BTreeMap::new();
    for ((c, m), a) in candidates.iter().zip(macs).zip(answered) {
        let w = a * m.warmup();
        if w > 0 {
            let e = weights.entry(c.operator).or_default();
            *e = (*e).max(w);
        }
    }
    weights
}

/// An operator's payout of an epoch's node `pool` for weight `w` out of
/// `total`: the denominator never drops below `MAX_SHARE × FULL`, so nobody
/// passes 1/16 of the pool; the division rounds down.
pub fn share(pool: U256, w: u64, total: u64) -> U256 {
    pool * U256::from(w) / U256::from(total.max(MAX_SHARE * FULL))
}

/// Run by the first block of an epoch, before its transactions (a system
/// write, like proof payouts): count the last epoch's beacons, pay its node
/// pool, and at the end of a day move every Mac's warm-up level.
pub fn distribute(state: &mut WorldState, height: u64) -> Result<Distribution, StateError> {
    let epoch_blocks = registry::epoch_blocks(state);
    let epoch = height / epoch_blocks - 1;
    let candidates = registry::candidates(state);
    let mut macs: Vec<Mac> = candidates.iter().map(|c| mac(state, c.index)).collect();

    // Slots each Mac answered (signed, and re-attested when due) in the epoch.
    let mut answered: Vec<u64> =
        candidates.iter().map(|c| beacons::beacon(state, c.index).answered(epoch)).collect();
    for (m, a) in macs.iter_mut().zip(answered.iter()) {
        m.answered += a;
    }
    // The founder's reserve credit folds in only now, after the day counts:
    // an epoch the reserve keys served counts as the founder's full
    // participation for the weights and for nothing else.
    apply_reserve_credit(state, epoch, &candidates, &mut answered);
    // Weights use the warm-up level the Macs had during the epoch.
    let weights = operator_weights(&candidates, &macs, &answered);
    let pool = node_pool(epoch, epoch_blocks);
    let total: u64 = weights.values().sum();
    let mut paid = Vec::with_capacity(weights.len());
    let mut minted = U256::ZERO;
    for (operator, w) in weights {
        let amount = share(pool, w, total);
        if !amount.is_zero() {
            state.set_balance(operator, state.balance(&operator) + amount)?;
            minted += amount;
        }
        paid.push((operator, amount));
    }

    // The hour-of-day profile and the last two epochs' counts (13-roadmap.md,
    // F): what the spread draw and early replacement read. A Mac registered
    // mid-epoch is judged from its first full day.
    for (c, a) in candidates.iter().zip(answered.iter()) {
        beacons::note(state, c.index, epoch, *a, c.registered_epoch < epoch);
    }
    if (epoch + 1).is_multiple_of(DAY_EPOCHS) {
        end_day(state, &candidates, &mut macs, epoch + 1 - DAY_EPOCHS);
    }
    for (c, m) in candidates.iter().zip(macs) {
        set_mac(state, c.index, m);
    }
    Ok(Distribution { epoch, pool, paid, unminted: pool - minted })
}

/// The randomness word of `epoch`: keccak256 over the domain, epoch, draw
/// number and the committee's threshold signature (the same seed the voting-set draw
/// uses). A threshold signature needs a quorum, so no single proposer can
/// bias it — but the seed is on chain before the epoch begins, so the word is
/// known one epoch ahead: contracts must commit before they reveal
/// (contracts/src/Randomness.sol documents the slot).
pub fn set_randomness(state: &mut WorldState, epoch: u64, draw: u64, signature: &[u8]) {
    let word = keccak256([b"aether-randomness/v1".as_slice(), &epoch.to_be_bytes(), &draw.to_be_bytes(), signature].concat());
    state.set_storage(REWARDS, tagged(TAG_RANDOM, U256::from(epoch)), U256::from_be_bytes(word.0));
}

/// The randomness word written at the start of `epoch` (0 before the first
/// epoch that had a draw seed on chain — a contract sees 0, never a stale word).
pub fn randomness(state: &WorldState, epoch: u64) -> U256 {
    state.storage(&REWARDS, tagged(TAG_RANDOM, U256::from(epoch)))
}

/// A day (epochs `first..first + DAY_EPOCHS`) ended: move warm-up levels.
fn end_day(state: &mut WorldState, candidates: &[registry::Candidate], macs: &mut [Mac], first: u64) {
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
            let announced = if registry_v3::is_v3(state) { beacons::announced_sleep(state, candidates[i].index) } else { 0 };
            let expected = day_slots.saturating_sub(announced * SLOTS);
            m.level = if expected == 0 {
                m.level
            } else if m.answered * 100 >= expected * GOOD_DAY_PERCENT {
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
    if registry_v3::is_v3(state) {
        for c in candidates {
            beacons::reset_announced_sleep(state, c.index);
        }
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

/// Through this many independent operators, genesis reserve keys stay
/// eligible standby and seated service never expires. At five, the ordinary
/// exit and service-credit grace apply; neither retires the reserve set.
pub const RESERVE_STANDBY_MAX_OPERATORS: usize = 4;

/// A reserve key: (ed25519 voting key, iroh node id).
pub type ReserveKey = ([u8; 32], [u8; 32]);

/// Genesis: the founder's reserve validator keys (ed25519 key, iroh node id)
/// and the operator address they belong to. They are not registry candidates:
/// they answer no beacons and earn no node rewards; nodes seat them only into
/// seats a committee is short of four (`rotation::with_reserve`), and the
/// chain records that seating for their service credit (`switch_reserve`).
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
    // A reserve key must never also register as a candidate (finding 1): the
    // genesis write puts a sentinel into the deployed registry's `indexOf`
    // mapping, so the pinned contract's `register()` reverts `Known()` on a
    // direct registration too — the bytecode itself never changes.
    for (key, _) in keys {
        let at = U256::from_be_bytes(
            keccak256([key.as_slice(), &U256::from(3u64).to_be_bytes::<32>()].concat()).0,
        );
        state.set_storage(registry::REGISTRY, at, U256::MAX);
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

/// The founder's reserve keys in the committee now: how many are members, and
/// the height they have been seated since without a break (`since` is 0 while
/// none are). One word the chain rewrites at each committee switch: seating
/// itself lives in the node (`rotation`, the carried handoff), not in
/// execution state, so the block a committee takes over at records it here as
/// one of its system writes (chain.rs `pre_state_with`) — the same
/// deterministic state on every node.
pub fn seated(state: &WorldState) -> (u64, u64) {
    let w = state.storage(&REWARDS, tagged(TAG_SEATED, U256::ZERO));
    let field = |shift: usize| ((w >> shift) & U256::from(u64::MAX)).to::<u64>();
    (field(0), field(64))
}

/// Record the committee that takes over at `height`: `members` exactly as the
/// carried handoff names them (voting key hex, like `rotation::Reserve`).
/// Seats held before and now keep their original height — the service did not
/// break — and a committee with no reserve key clears the word, so the next
/// seating restarts it at its own height. Writes nothing without reserve keys
/// at genesis.
pub fn switch_reserve(state: &mut WorldState, height: u64, members: &[(String, String)]) {
    let Some((_, keys)) = reserve(state) else { return };
    let holds = |k: &str| {
        hex::decode(k.trim_start_matches("0x"))
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
            .is_some_and(|key| keys.iter().any(|(rk, _)| rk == &key))
    };
    let count = members.iter().filter(|(k, _)| holds(k)).count() as u64;
    let (_, since) = seated(state);
    let since = if count > 0 && since > 0 {
        since // seated before and still: the service did not break
    } else if count > 0 {
        height
    } else {
        0
    };
    let w = U256::from(count) | (U256::from(since) << 64);
    if state.storage(&REWARDS, tagged(TAG_SEATED, U256::ZERO)) != w {
        state.set_storage(REWARDS, tagged(TAG_SEATED, U256::ZERO), w);
    }
}

/// A roster member as the words below hold it: (voting key bytes, iroh node
/// id bytes). None when the strings are not exactly that.
fn member_bytes(m: &(String, String)) -> Option<([u8; 32], [u8; 32])> {
    let key = hex::decode(m.0.trim_start_matches("0x"))
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok())?;
    let node = m.1.parse::<aether_net::EndpointId>().ok()?;
    Some((key, *node.as_bytes()))
}

/// Whether two rosters name the same members (parsed key and node id pairs;
/// order and spelling do not matter). A roster with a member that is not
/// (key hex, node id) matches nothing — a handoff carrying one is refused.
pub fn same_roster(a: &[(String, String)], b: &[(String, String)]) -> bool {
    let parsed = |m: &[(String, String)]| -> Option<Vec<([u8; 32], [u8; 32])>> {
        let mut v: Vec<_> = m.iter().map(member_bytes).collect::<Option<_>>()?;
        v.sort();
        Some(v)
    };
    parsed(a).zip(parsed(b)).is_some_and(|(a, b)| a == b)
}

/// Write a member list as one word per member half: the head (`head`, holding
/// the count), then each member's key and node id. Zeroes whatever a longer
/// list left behind. Refuses a member that is not (key hex, node id).
fn write_members(
    state: &mut WorldState,
    tag: u64,
    head: U256,
    members: &[(String, String)],
) -> Result<(), String> {
    let old = ((state.storage(&REWARDS, tagged(tag, U256::ZERO)) >> 64usize) & U256::from(u64::MAX))
        .to::<u64>() as usize;
    let mut words = vec![head];
    for (key, node) in members {
        let (k, n) = member_bytes(&(key.clone(), node.clone()))
            .ok_or_else(|| format!("member {key}: 32-byte key hex and node id"))?;
        words.push(U256::from_be_bytes(k));
        words.push(U256::from_be_bytes(n));
    }
    for i in 0..words.len().max(1 + 2 * old) {
        let w = words.get(i).copied().unwrap_or(U256::ZERO);
        state.set_storage(REWARDS, tagged(tag, U256::from(i as u64)), w);
    }
    Ok(())
}

/// Read a member list back: (head's low word, member count, members).
fn read_members(state: &WorldState, tag: u64) -> (u64, u64, Vec<(String, String)>) {
    let head = state.storage(&REWARDS, tagged(tag, U256::ZERO));
    let low = (head & U256::from(u64::MAX)).to::<u64>();
    let count = ((head >> 64usize) & U256::from(u64::MAX)).to::<u64>();
    let word = |k: u64| state.storage(&REWARDS, tagged(tag, U256::from(k))).to_be_bytes::<32>();
    let members = (0..count as usize)
        .filter_map(|i| {
            let node = aether_net::EndpointId::from_bytes(&word(2 + 2 * i as u64)).ok()?;
            Some((hex::encode(word(1 + 2 * i as u64)), node.to_string()))
        })
        .collect();
    (low, count, members)
}

/// Genesis: the network's first voting committee (network.json keeps it as
/// `genesis_validators` across handoffs, so every node derives the same
/// genesis state however many committees came and went). Node rewards only;
/// an empty list writes nothing.
pub fn set_committee(state: &mut WorldState, members: &[(String, String)]) -> Result<(), String> {
    if members.is_empty() {
        return Ok(());
    }
    write_members(state, TAG_COMMITTEE, U256::from(members.len()) << 64usize, members)
}

/// The voting committee the chain last recorded: the genesis roster, or the
/// members of the handoff that switched in last. Empty without node rewards.
pub fn committee(state: &WorldState) -> Vec<(String, String)> {
    read_members(state, TAG_COMMITTEE).2
}

/// Commit the voting set decided for draw `draw` — the draw seed's decision,
/// or a reserve reseat's. Every node derives the same roster from state, and
/// only a handoff naming exactly these members may carry it over.
pub fn commit_roster(state: &mut WorldState, draw: u64, members: &[(String, String)]) -> Result<(), String> {
    write_members(
        state,
        TAG_ROSTER,
        U256::from(draw) | (U256::from(members.len()) << 64usize),
        members,
    )
}

/// Drop the committed roster (the committee it named has switched in).
pub fn clear_roster(state: &mut WorldState) {
    if next_roster(state).is_some() {
        let _ = write_members(state, TAG_ROSTER, U256::ZERO, &[]);
    }
}

/// The voting set committed for a draw: (draw, members). None while no
/// decision is committed — nothing may hand over then.
pub fn next_roster(state: &WorldState) -> Option<(u64, Vec<(String, String)>)> {
    let (draw, count, members) = read_members(state, TAG_ROSTER);
    (count > 0).then_some((draw, members))
}

/// Freeze the draw's candidate pool at its first block (`rotation::eligible`
/// of the state that block builds on): the draw and every reserve reseat in
/// the draw read this word, so the pool is not the proposer's choice.
pub fn freeze_pool(state: &mut WorldState, draw: u64, members: &[(String, String)]) -> Result<(), String> {
    write_members(
        state,
        TAG_POOL,
        U256::from(draw) | (U256::from(members.len()) << 64usize),
        members,
    )
}

/// The candidate pool frozen for the draw that tagged it: (draw, members).
/// None when no pool is frozen.
pub fn draw_pool(state: &WorldState) -> Option<(u64, Vec<(String, String)>)> {
    let (draw, count, members) = read_members(state, TAG_POOL);
    (draw != 0 || count > 0).then_some((draw, members))
}

/// Finding 6 (red team, 2026-09-29): how many consecutive registry epochs the
/// reserve keys may keep seats nobody needs before their credit stops. Seats
/// held while five or more independent operators qualify mean the handoff
/// home never completed; past this many epochs of that, the service credit is
/// gone until the keys stand down or the count returns to four or fewer.
/// Expiry stops service credit, never reserve registration or eligibility.
pub const RESERVE_GRACE_EPOCHS: u64 = 2;

/// Record the overdue count as of the epoch opening at `epoch` (a chain
/// system write at every epoch boundary while reserve keys exist).
pub fn set_overdue(state: &mut WorldState, epoch: u64, count: u64) {
    let w = U256::from(epoch) | (U256::from(count) << 64usize);
    if state.storage(&REWARDS, tagged(TAG_OVERDUE, U256::ZERO)) != w {
        state.set_storage(REWARDS, tagged(TAG_OVERDUE, U256::ZERO), w);
    }
}

/// (epoch, count) of the last overdue write; (0, 0) when there was none.
pub fn overdue(state: &WorldState) -> (u64, u64) {
    let w = state.storage(&REWARDS, tagged(TAG_OVERDUE, U256::ZERO));
    (
        (w & U256::from(u64::MAX)).to::<u64>(),
        ((w >> 64usize) & U256::from(u64::MAX)).to::<u64>(),
    )
}

/// Whether the founder's reserve keys served the epoch that opens at `epoch`:
/// seated in the voting committee from the epoch's first block on (the seating
/// word has them from `since` at or before `epoch × epoch_blocks`, unbroken).
/// Returns the founder's operator address; None without reserve keys at
/// genesis, or while they hold no seat (a committee at four seats takes none,
/// `rotation::with_reserve`). A seating or unseating inside the epoch does not
/// count: only a full epoch of service does.
///
/// Seating is the state's cheap witness of reserve service: a finalization
/// carries one threshold signature (no per-validator signers to count) and the
/// proposer record never reaches the execution state. The chain is a
/// contiguous finalized history, so the distribution block existing at all
/// means the epoch's blocks were finalized — the ≥ 90% participation the rule
/// asks for. The genesis committee is named by no handoff, so a seat it held
/// is recorded by no switch: that epoch pays no credit (the ops procedure
/// keeps the reserve keys out of the genesis roster) — conservative, never
/// more.
pub fn reserve_served(state: &WorldState, epoch: u64) -> Option<Address> {
    let (operator, _) = reserve(state)?;
    let (count, since) = seated(state);
    if count == 0 || since > epoch.saturating_mul(registry::epoch_blocks(state)) {
        return None;
    }
    // Finding 6: seats nobody needs earn nothing past the grace epochs. The
    // count must be this epoch's — the chain writes the word at every epoch
    // boundary while reserve keys exist — so a missing or stale count pays
    // nothing rather than guessing.
    let (written, count) = overdue(state);
    (written == epoch && count <= RESERVE_GRACE_EPOCHS).then_some(operator)
}

/// Fold the founder's reserve credit into the per-Mac slot counts an epoch is
/// paid by: an epoch the reserve keys served (see `reserve_served`) counts as
/// the founder operator's full participation, so every Mac it registered
/// answers all `SLOTS` slots — at the warm-up it already has. An operator's
/// weight still takes the max over its Macs, so the credit never adds a second
/// share, and it moves no day count and no warm-up level (only a Mac's own
/// answers do). `distribute` and the reward view count through this same
/// function, so the two cannot disagree about the founder either.
pub fn apply_reserve_credit(state: &WorldState, epoch: u64, candidates: &[registry::Candidate], answered: &mut [u64]) {
    let Some(founder) = reserve_served(state, epoch) else { return };
    for (c, a) in candidates.iter().zip(answered.iter_mut()) {
        if c.operator == founder {
            *a = (*a).max(SLOTS);
        }
    }
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
