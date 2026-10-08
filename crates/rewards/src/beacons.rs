//! Beacon slots (docs/design/15-node-rewards.md, A): twelve unpredictable
//! moments per epoch at which a registered Mac proves it is running.
//!
//! - **Slots.** Each slot's height is unknown until shortly before it. The
//!   hash of the block before an epoch (the previous epoch's last block) fixes
//!   slot 0's height within the epoch's first twelfth; the hash of slot k−1's
//!   block — recorded by the block right after it — fixes slot k's height
//!   within the epoch's (k+1)-th twelfth. A slot's place is therefore never
//!   known more than about an answer window (~90 s) ahead, and a Mac cannot
//!   plan the day's answers in one sitting: it has to be running
//!   (2026-09-29 red-team 1).
//! - **Answer.** When slot `k`'s block `s` is out, its hash is recorded by block
//!   `s + 1`. A registered Mac then signs `message(chain, epoch, k, hash)` with
//!   the voting key the registry binds to it, and the answer is valid in blocks
//!   `s + 1 ..= s + window` (90 blocks, ~90 s, on hour-long epochs).
//! - **Record.** Proposers put answers in the block (`Payload::beacons`, no
//!   transaction, no fee: a Mac with a zero balance answers). Validators check
//!   each signature; the state keeps one word per Mac: the epoch, a 12-bit mask
//!   of the slots answered, and the last re-attested period. An answer also
//!   advances the Mac's registry liveness (streak) exactly as the contract's
//!   paid `beacon()` call does, so the Mac never has to send that transaction.
//! - **Re-attestation.** Once a day, from a random slot `R_d` (fixed by the
//!   day's first seed), a Mac's answers count only if it re-attested for the
//!   current *period*: period `d + 1` runs from `R_d` to just before `R_{d+1}`.
//!   The answer carries the registrar's P-256 signature over
//!   `reattest_message(chain, voting key, period)`, which the registrar gives
//!   only for a fresh DeviceCheck token of a registered Mac (the Apple call
//!   stays at the registrar, off the consensus path, as for registration).
//!   Registration covers the periods up to the day after it. A Mac without a
//!   valid re-attestation cannot have its answers included: weight 0 until it
//!   re-attests.

use crate::{enabled, registry_v3, tagged, DAY_EPOCHS, REWARDS, SLOTS};
use aether_execution::registry::{self, Candidate};
use aether_execution::WorldState;
use alloy_primitives::{keccak256, Address, U256};

/// Blocks an answer stays valid after its slot's block (~90 s of 1 s blocks).
pub const ANSWER_WINDOW: u64 = 90;
/// Answers one block may carry (each is one signature check).
pub const MAX_ANSWERS_PER_BLOCK: usize = 1024;
/// Availability signals use slot numbers above the twelve ordinary slots.
/// Their number binds a block height and direction without changing the old
/// beacon payload shape: stale signatures expire with that block.
pub fn availability_slot(height: u64, leaving: bool) -> Option<u64> {
    height.checked_mul(2)?.checked_add(SLOTS + u64::from(!leaving))
}

pub fn availability_of(slot: u64) -> Option<(u64, bool)> {
    (slot >= SLOTS).then(|| ((slot - SLOTS) / 2, (slot - SLOTS).is_multiple_of(2)))
}
/// A registry candidate's liveness streak restarts after this many missed epochs (as in the contract).
pub const GRACE_EPOCHS: u64 = 24;

pub(crate) const TAG_SLOTS: u64 = 3;
pub(crate) const TAG_SLOT_HASH: u64 = 4;
pub(crate) const TAG_DAY: u64 = 5;
pub(crate) const TAG_BEACON: u64 = 6;

fn word_u64(w: U256, shift: usize) -> u64 {
    ((w >> shift) & U256::from(u64::MAX)).to::<u64>()
}

/// (segment, answer window, random span) for an epoch length; None when the
/// epoch is too short for twelve slots with room to answer (under 36 blocks).
pub fn layout(epoch_blocks: u64) -> Option<(u64, u64, u64)> {
    let segment = epoch_blocks / SLOTS;
    let window = ANSWER_WINDOW.min((segment / 2).max(1));
    (segment >= window + 2).then(|| (segment, window, segment - window - 1))
}

fn draw(seed: &[u8; 32], label: &[u8], k: u64) -> u64 {
    let h = keccak256([seed.as_slice(), label, &k.to_be_bytes()].concat());
    u64::from_be_bytes(h[..8].try_into().expect("8 bytes"))
}

/// Slot `slot`'s height in `epoch`: somewhere in the epoch's `slot + 1`-th
/// segment, early enough that its window ends inside the epoch. The seed is
/// the hash of the block before the epoch (slot 0) or of the slot before it
/// (every later slot) — see `on_block`, which draws them one at a time.
pub fn slot_height(seed: &[u8; 32], epoch: u64, slot: u64, epoch_blocks: u64) -> Option<u64> {
    let (segment, _, span) = layout(epoch_blocks)?;
    Some(epoch * epoch_blocks + slot * segment + 1 + draw(seed, b"slot", slot) % span)
}

/// The day's re-attestation slot (epoch within the day, slot), from its first seed.
pub fn reattest_slot(seed: &[u8; 32]) -> (u64, u64) {
    let r = draw(seed, b"reattest", 0) % (DAY_EPOCHS * SLOTS);
    (r / SLOTS, r % SLOTS)
}

/// What a Mac signs (with its registered voting key) to answer slot `slot` of `epoch`.
pub fn message(chain_id: u64, epoch: u64, slot: u64, hash: &[u8; 32]) -> Vec<u8> {
    [b"aether-beacon".as_slice(), &chain_id.to_be_bytes(), REWARDS.as_slice(), &epoch.to_be_bytes(), &slot.to_be_bytes(), hash].concat()
}

/// Domain-separated from slot answers and bound to one block height, so an
/// old sleep cannot be replayed after the Mac announces its return.
pub fn availability_message(chain_id: u64, height: u64, leaving: bool) -> Vec<u8> {
    [b"aether-availability/v1".as_slice(), &chain_id.to_be_bytes(), REWARDS.as_slice(), &height.to_be_bytes(), &[u8::from(leaving)]].concat()
}

/// What the registrar signs (P-256, SHA-256 of these bytes) after a fresh
/// DeviceCheck token of the Mac holding `validator_key`, for `period`.
pub fn reattest_message(chain_id: u64, validator_key: &[u8; 32], period: u64) -> Vec<u8> {
    [b"aether-reattest".as_slice(), &chain_id.to_be_bytes(), REWARDS.as_slice(), validator_key, &period.to_be_bytes()].concat()
}

/// Slot `slot`'s height this epoch, once drawn: slot 0 by the epoch's first
/// block, slot k by the block after slot k−1's — which is also the earliest
/// anyone not building that block could know it.
pub fn slot(state: &WorldState, slot: u64) -> Option<u64> {
    let w = state.storage(&REWARDS, tagged(TAG_SLOTS, U256::from(slot)));
    (!w.is_zero()).then(|| w.to::<u64>())
}

/// Hash of slot `slot`'s block, once recorded (this epoch only).
pub fn slot_hash(state: &WorldState, slot: u64) -> Option<[u8; 32]> {
    let w = state.storage(&REWARDS, tagged(TAG_SLOT_HASH, U256::from(slot)));
    (!w.is_zero()).then(|| w.to_be_bytes::<32>())
}

/// (day, epoch within the day, slot) of the current day's re-attestation.
pub fn day(state: &WorldState) -> Option<(u64, u64, u64)> {
    let w = state.storage(&REWARDS, tagged(TAG_DAY, U256::ZERO));
    (!w.is_zero()).then(|| (word_u64(w, 0), word_u64(w, 64), word_u64(w, 128)))
}

/// The re-attestation period slot `slot` of `epoch` falls in: day `d` before
/// `R_d`, `d + 1` from it on.
pub fn period(state: &WorldState, epoch: u64, slot: u64) -> u64 {
    let d = epoch / DAY_EPOCHS;
    match day(state) {
        Some((day, e, s)) if day == d && (epoch % DAY_EPOCHS, slot) >= (e, s) => d + 1,
        _ => d,
    }
}

/// A Mac's beacon record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Beacon {
    /// The epoch `mask` belongs to.
    pub epoch: u64,
    /// Slots of `epoch` answered (bit k = slot k).
    pub mask: u64,
    /// The last period this Mac re-attested for.
    pub attested: Option<u64>,
}

impl Beacon {
    fn pack(self) -> U256 {
        U256::from(self.epoch) | (U256::from(self.mask) << 64) | (U256::from(self.attested.map_or(0, |p| p + 1)) << 128)
    }
    fn unpack(w: U256) -> Self {
        let a = word_u64(w, 128);
        Beacon { epoch: word_u64(w, 0), mask: word_u64(w, 64), attested: a.checked_sub(1) }
    }
    /// Slots answered in `epoch`.
    pub fn answered(self, epoch: u64) -> u64 {
        if self.epoch == epoch {
            u64::from(self.mask.count_ones())
        } else {
            0
        }
    }
}

/// Beacon record of registry candidate `index`.
pub fn beacon(state: &WorldState, index: u64) -> Beacon {
    Beacon::unpack(state.storage(&REWARDS, tagged(TAG_BEACON, U256::from(index))))
}

/// Write candidate `index`'s beacon record directly (tests and simulations;
/// on chain only verified answers in blocks write it).
#[doc(hidden)]
pub fn put_beacon(state: &mut WorldState, index: u64, b: Beacon) {
    state.set_storage(REWARDS, tagged(TAG_BEACON, U256::from(index)), b.pack());
}

/// Days of history an hour-of-day profile keeps (the EMA's denominator).
pub const PROFILE_DAYS: u64 = 14;
/// Fixed-point scale of a profile bucket: a fully available hour is this
/// (twelve slots × 6, so one slot's share divides it exactly).
pub const PROFILE_SCALE: u64 = 72;
/// The scale `Profile` ratios are read in: a probability of 1 is this many
/// units. Integer fixed point, never floating point — the liveness rules that
/// read profiles (docs/design/13-roadmap.md, F) decide the next committee, so
/// every validator must compute bit-identical words.
pub const PROB_SCALE: u64 = 1_000_000_000;
/// What `recent` writes when a count is unknown (a Mac's first epoch, or a gap).
pub const NO_COUNT: u64 = 15;

pub(crate) const TAG_PROFILE: u64 = 11;
pub(crate) const TAG_OFFERED: u64 = 12;
pub(crate) const TAG_RECENT: u64 = 13;
/// New-genesis recent stability: last epoch, four up bits, six unexpected-drop
/// bits, and this day's announced sleeping hours. The next free REWARDS tag.
pub(crate) const TAG_STABILITY: u64 = 18;
/// Bits a profile bucket takes in its word (values fit in 7; the margin keeps
/// the packing comfortable).
const BUCKET_BITS: usize = 10;

/// One day's step of a bucket's EMA, in `PROFILE_SCALE` fixed point:
/// `v ← (v·(PROFILE_DAYS−1) + x) / PROFILE_DAYS`. The scale keeps a recovery
/// from zero moving (a full day lifts 0 to 4) and a lapse decaying to zero.
fn ema(v: u64, x: u64) -> u64 {
    (v * (PROFILE_DAYS - 1) + x) / PROFILE_DAYS
}

/// A Mac's hour-of-day availability (docs/design/13-roadmap.md, F): per hour
/// bucket, EMAs of the beacon slots answered and the slots offered. Derived
/// from beacon answers only — where the Mac was awake, never where it claims
/// to be — so it cannot be gamed by self-report and needs no location data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    /// Answered slots per hour bucket, in `PROFILE_SCALE` units.
    pub answered: [u64; DAY_EPOCHS as usize],
    /// Offered slots per hour bucket, in `PROFILE_SCALE` units.
    pub offered: [u64; DAY_EPOCHS as usize],
}

impl Profile {
    fn bucket(w: U256, h: usize) -> u64 {
        ((w >> (BUCKET_BITS * h)) & U256::from((1u64 << BUCKET_BITS) - 1)).to::<u64>()
    }
    /// Availability in hour bucket `h` (hours wrap), in `PROB_SCALE` units:
    /// the answered/offered ratio, once that hour has ever been offered.
    /// (The EMA keeps answered ≤ offered; the min only guards the division.)
    pub fn at(&self, h: u64) -> Option<u64> {
        let h = (h % DAY_EPOCHS) as usize;
        let (a, o) = (self.answered[h], self.offered[h]);
        (o > 0).then(|| a.min(o) * PROB_SCALE / o)
    }
    /// Availability over the whole day, in `PROB_SCALE` units.
    pub fn overall(&self) -> Option<u64> {
        let (a, o): (u64, u64) = (self.answered.iter().sum(), self.offered.iter().sum());
        (o > 0).then(|| a.min(o) * PROB_SCALE / o)
    }
    /// The worst observed hour's availability, in `PROB_SCALE` units.
    pub fn worst(&self) -> Option<u64> {
        (0..DAY_EPOCHS).filter_map(|h| self.at(h)).fold(None, |worst, p| Some(worst.map_or(p, |w| w.min(p))))
    }
}

/// The hour-of-day profile of registry candidate `index` (empty until its
/// first full day has passed).
pub fn profile(state: &WorldState, index: u64) -> Profile {
    let (a, o) = (
        state.storage(&REWARDS, tagged(TAG_PROFILE, U256::from(index))),
        state.storage(&REWARDS, tagged(TAG_OFFERED, U256::from(index))),
    );
    let mut p = Profile::default();
    for h in 0..DAY_EPOCHS as usize {
        p.answered[h] = Profile::bucket(a, h);
        p.offered[h] = Profile::bucket(o, h);
    }
    p
}

/// Fold epoch `epoch`'s observation (`answered` of `SLOTS` slots) into
/// candidate `index`'s hour-of-day profile and recent-answer record.
/// `distribute` calls this at every epoch boundary; `full` says the Mac was
/// registered for the whole epoch (a Mac registered mid-epoch is judged from
/// its first full day, so its ratio is exact rather than understated).
pub fn note(state: &mut WorldState, index: u64, epoch: u64, answered: u64, full: bool) {
    // A full epoch's observation is PROFILE_SCALE; each answered slot a twelfth of it.
    const SLOT: u64 = PROFILE_SCALE / SLOTS;
    let hour = (epoch % DAY_EPOCHS) as usize;
    let shift = BUCKET_BITS * hour;
    let clear = U256::from(!0u64 >> (64 - BUCKET_BITS)) << shift;
    let bucket = |w: U256| ((w >> shift) & U256::from((1u64 << BUCKET_BITS) - 1)).to::<u64>();
    let put = |w: U256, v: u64| (w & !clear) | (U256::from(v) << shift);
    let mut write = |tag: u64, x: u64| {
        let at = tagged(tag, U256::from(index));
        let w = state.storage(&REWARDS, at);
        let next = put(w, ema(bucket(w), x));
        if w != next {
            state.set_storage(REWARDS, at, next);
        }
    };
    if full {
        write(TAG_PROFILE, answered.min(SLOTS) * SLOT);
        write(TAG_OFFERED, PROFILE_SCALE);
    }
    // The last two epochs' counts: the beacon record itself keeps only the newest.
    let at = tagged(TAG_RECENT, U256::from(index));
    let old = state.storage(&REWARDS, at);
    let prev = if old.is_zero() {
        NO_COUNT
    } else {
        let last_of = |w: U256| ((w >> 4usize) & U256::from(0xFu64)).to::<u64>();
        // The old word's count is epoch − 1's only if it was written for that epoch.
        if ((old >> 8usize).to::<u64>() - 1) + 1 == epoch { last_of(old) } else { NO_COUNT }
    };
    let next = (U256::from(epoch + 1) << 8usize) | (U256::from(answered.min(NO_COUNT)) << 4usize) | U256::from(prev);
    if old != next {
        state.set_storage(REWARDS, at, next);
    }
    if full && registry_v3::is_v3(state) {
        let at = tagged(TAG_STABILITY, U256::from(index));
        let old = state.storage(&REWARDS, at);
        let last = word_u64(old, 0).checked_sub(1);
        let gap = last.map_or(1, |last| epoch.saturating_sub(last)).clamp(1, 6);
        let previous_up = ((old >> 64usize) & U256::from(15u8)).to::<u8>();
        let previous_drops = ((old >> 68usize) & U256::from(63u8)).to::<u8>();
        let leaving = registry_v3::announced_for_epoch(state, index, epoch);
        let up = (previous_up << gap | u8::from(answered >= SLOTS / 2)) & 15;
        let gap_drops = ((1u8 << (gap - 1)) - 1) << 1;
        let drops = (previous_drops << gap | gap_drops | u8::from(!leaving && answered < SLOTS / 2)) & 63;
        let sleep = announced_sleep(state, index) + u64::from(leaving && answered < SLOTS / 2);
        let next = U256::from(epoch + 1)
            | (U256::from(up) << 64usize)
            | (U256::from(drops) << 68usize)
            | (U256::from(sleep.min(24)) << 74usize);
        state.set_storage(REWARDS, at, next);
    }
}

/// The last two distributed epochs' answered-slot counts for candidate
/// `index`: (epoch, its count, the epoch before's), `NO_COUNT` where a count
/// is unknown. None until a distribution has seen the Mac. Early replacement
/// (docs/design/13-roadmap.md, F) reads this.
pub fn recent(state: &WorldState, index: u64) -> Option<(u64, u64, u64)> {
    let w = state.storage(&REWARDS, tagged(TAG_RECENT, U256::from(index)));
    if w.is_zero() {
        return None;
    }
    let field = |shift: usize| ((w >> shift) & U256::from(0xFu64)).to::<u64>();
    Some((((w >> 8usize).to::<u64>() - 1), field(4), field(0)))
}

/// (up in the last four epochs, unannounced drops in the last six).
/// A gap in observations is treated as an unannounced drop unless a signed
/// leaving announcement was in force when the missed epoch ended.
pub fn stability(state: &WorldState, index: u64, epoch: u64) -> (u32, bool) {
    let w = state.storage(&REWARDS, tagged(TAG_STABILITY, U256::from(index)));
    if w.is_zero() || word_u64(w, 0) != epoch {
        return (0, false);
    }
    let up = ((w >> 64usize) & U256::from(15u8)).to::<u8>();
    let drops = ((w >> 68usize) & U256::from(63u8)).to::<u8>();
    (up.count_ones(), drops == 0)
}

pub fn announced_sleep(state: &WorldState, index: u64) -> u64 {
    ((state.storage(&REWARDS, tagged(TAG_STABILITY, U256::from(index))) >> 74usize) & U256::from(255u8)).to::<u64>()
}

pub fn reset_announced_sleep(state: &mut WorldState, index: u64) {
    let at = tagged(TAG_STABILITY, U256::from(index));
    let w = state.storage(&REWARDS, at);
    let next = w & !(U256::from(255u8) << 74usize);
    if w != next {
        state.set_storage(REWARDS, at, next);
    }
}

fn candidate_slot(index: u64, k: u64) -> U256 {
    let base = U256::from_be_bytes(keccak256(U256::from(2u64).to_be_bytes::<32>()).0);
    base + U256::from(5 * index + k)
}

/// Registry candidate `index` alone (without reading every candidate).
pub fn candidate(state: &WorldState, index: u64) -> Option<Candidate> {
    let reg = registry::REGISTRY;
    if index >= state.storage(&reg, U256::from(2u64)).to::<u64>() {
        return None;
    }
    let slot = |k| state.storage(&reg, candidate_slot(index, k));
    let (s3, s4) = (slot(3), slot(4));
    Some(Candidate {
        index,
        operator: Address::from_slice(&slot(0).to_be_bytes::<32>()[12..]),
        validator_key: slot(1).to_be_bytes(),
        node_id: slot(2).to_be_bytes(),
        beaconer: Address::from_slice(&s3.to_be_bytes::<32>()[12..]),
        registered_epoch: word_u64(s3, 160),
        last_epoch: word_u64(s4, 0),
        streak: word_u64(s4, 64),
        missed: word_u64(s4, 128),
    })
}

/// An answer that may go in the block at `height`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Due {
    pub epoch: u64,
    pub slot: u64,
    /// Hash of the slot's block: what the Mac signs.
    pub hash: [u8; 32],
    /// The re-attestation period of the slot.
    pub period: u64,
    /// The answer must carry a re-attestation for `period`.
    pub needs_attestation: bool,
}

/// Whether candidate `c` may answer slot `slot` in the block at `height`
/// (before signatures, which the node checks).
pub fn check(state: &WorldState, height: u64, c: &Candidate, k: u64) -> Result<Due, String> {
    if !enabled(state) {
        return Err("no node rewards on this network".into());
    }
    let epoch_blocks = registry::epoch_blocks(state);
    let epoch = height / epoch_blocks;
    let (_, window, _) = layout(epoch_blocks).ok_or("epochs too short for beacon slots")?;
    let s = slot(state, k).ok_or("no such slot drawn yet")?;
    if s / epoch_blocks != epoch || height <= s || height > s + window {
        return Err(format!("slot {k} is not open at height {height}"));
    }
    let hash = slot_hash(state, k).ok_or("slot hash not recorded")?;
    let b = beacon(state, c.index);
    if b.epoch == epoch && b.mask & (1 << k) != 0 {
        return Err(format!("slot {k} already answered"));
    }
    let period = period(state, epoch, k);
    let covered = period <= c.registered_epoch / DAY_EPOCHS + 1 || b.attested == Some(period);
    Ok(Due { epoch, slot: k, hash, period, needs_attestation: !covered })
}

/// Record a verified answer of `c`: its slot bit, its re-attestation, and its registry liveness.
pub fn record(state: &mut WorldState, c: &Candidate, due: &Due, attested: bool) {
    if let Some((height, leaving)) = availability_of(due.slot) {
        registry_v3::set_availability(state, c.index, height, leaving);
        return;
    }
    let mut b = beacon(state, c.index);
    if b.epoch != due.epoch {
        b = Beacon { epoch: due.epoch, mask: 0, attested: b.attested };
    }
    b.mask |= 1 << due.slot;
    if attested {
        b.attested = Some(due.period);
    }
    state.set_storage(REWARDS, tagged(TAG_BEACON, U256::from(c.index)), b.pack());
    mark_live(state, c.index, due.epoch);
}

/// The registry contract's `beacon()` liveness update, as a system write.
fn mark_live(state: &mut WorldState, index: u64, epoch: u64) {
    let at = candidate_slot(index, 4);
    let w = state.storage(&registry::REGISTRY, at);
    let (last, streak, missed) = (word_u64(w, 0), word_u64(w, 64), word_u64(w, 128));
    if epoch <= last {
        return;
    }
    let gap = epoch - last;
    let announced_gap = registry_v3::is_v3(state)
        && registry_v3::last_leaving(state, index).is_some_and(|h| h / registry::epoch_blocks(state) >= last)
        && registry_v3::availability(state, index).is_some_and(|(h, leaving)| leaving || h / registry::epoch_blocks(state) + 1 >= epoch);
    let (streak, missed) = if announced_gap {
        (streak + 1, missed)
    } else if gap <= GRACE_EPOCHS {
        (streak + 1, missed + gap - 1)
    } else {
        (1, 0)
    };
    state.set_storage(registry::REGISTRY, at, U256::from(epoch) | (U256::from(streak) << 64) | (U256::from(missed) << 128));
}

fn first_block(epoch: u64, epoch_blocks: u64) -> u64 {
    (epoch * epoch_blocks).max(1)
}

/// Whether block `height` has beacon system writes (an epoch's first block, or right after a slot).
pub fn touches(state: &WorldState, height: u64) -> bool {
    if !enabled(state) {
        return false;
    }
    let epoch_blocks = registry::epoch_blocks(state);
    if layout(epoch_blocks).is_none() || height == 0 {
        return false;
    }
    height == first_block(height / epoch_blocks, epoch_blocks)
        || (0..SLOTS).any(|k| slot(state, k).is_some_and(|h| h + 1 == height))
}

/// Block `height`'s beacon system writes (after the epoch's distribution),
/// given the hash of its parent: at an epoch's first block the epoch's slot 0
/// (and at a day's first block its re-attestation slot); right after a slot's
/// block, that block's hash — and, drawn from that hash, the next slot's
/// height, which until this block existed nobody could know.
pub fn on_block(state: &mut WorldState, height: u64, parent_hash: [u8; 32]) {
    if !touches(state, height) {
        return;
    }
    let epoch_blocks = registry::epoch_blocks(state);
    let epoch = height / epoch_blocks;
    if height == first_block(epoch, epoch_blocks) {
        let h0 = slot_height(&parent_hash, epoch, 0, epoch_blocks).expect("layout checked");
        state.set_storage(REWARDS, tagged(TAG_SLOTS, U256::ZERO), U256::from(h0));
        for k in 1..SLOTS {
            let at = tagged(TAG_SLOTS, U256::from(k));
            if !state.storage(&REWARDS, at).is_zero() {
                state.set_storage(REWARDS, at, U256::ZERO);
            }
        }
        for k in 0..SLOTS {
            let at = tagged(TAG_SLOT_HASH, U256::from(k));
            if !state.storage(&REWARDS, at).is_zero() {
                state.set_storage(REWARDS, at, U256::ZERO);
            }
        }
        if epoch.is_multiple_of(DAY_EPOCHS) {
            let (e, s) = reattest_slot(&parent_hash);
            let day = epoch / DAY_EPOCHS;
            // Bit 192 keeps the word non-zero on day 0.
            let w = U256::from(day) | (U256::from(e) << 64) | (U256::from(s) << 128) | (U256::from(1u8) << 192);
            state.set_storage(REWARDS, tagged(TAG_DAY, U256::ZERO), w);
        }
    }
    for k in 0..SLOTS {
        if slot(state, k) != Some(height - 1) {
            continue;
        }
        state.set_storage(REWARDS, tagged(TAG_SLOT_HASH, U256::from(k)), U256::from_be_bytes(parent_hash));
        if k + 1 < SLOTS {
            let next = slot_height(&parent_hash, epoch, k + 1, epoch_blocks).expect("layout checked");
            state.set_storage(REWARDS, tagged(TAG_SLOTS, U256::from(k + 1)), U256::from(next));
        }
    }
}

/// Slots candidate `c` can answer in the block at `height`, oldest first.
pub fn due(state: &WorldState, height: u64, c: &Candidate) -> Vec<Due> {
    (0..SLOTS).filter_map(|k| check(state, height, c, k).ok()).collect()
}
