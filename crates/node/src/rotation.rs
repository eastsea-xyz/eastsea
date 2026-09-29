//! Which voting set comes next (docs/design/07-consensus.md, "open voting
//! nodes"). No person decides: at the first block of every registry epoch,
//! each node computes the proposed set from the registry state that block
//! builds on. The running set then reshares its key to it in the background and
//! hands over with a signed handoff (`handoff.rs`); the chain never stops for it.
//!
//! At most a third of the seats change per epoch, so a newcomer group (even one
//! owner with many Macs and wallets) cannot take the whole set at once: it has
//! to stay live for several epochs, while the running set keeps a quorum.

use aether_consensus::committee::MIN_OPEN_COMMITTEE;
use aether_execution::registry;
use aether_execution::WorldState;
use aether_rewards::{beacons, DAY_EPOCHS};

/// In a validator's data dir: files a background reshare stages for a handoff.
pub const STAGED_THRESHOLD: &str = "threshold-next.json";
pub const STAGED_NETWORK: &str = "network-next.json";

/// In a voting node's data dir: the block (and finalization) it verified last
/// as a follower, which it starts from when it has no validator history.
pub const ANCHOR_FILE: &str = "anchor.json";

/// Largest voting set (the DKG cost grows with its square).
pub const MAX_VOTING_NODES: usize = MAX_DRAWN;

/// The running voting set, in roster order: (ed25519 key hex, iroh node id).
/// Empty on followers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Committee {
    pub members: Vec<(String, String)>,
}

impl Committee {
    fn has(&self, key: &str) -> bool {
        self.members.iter().any(|(k, _)| k == key)
    }
}

/// Largest voting set drawn (the DKG cost grows with its square).
pub const MAX_DRAWN: usize = 128;

/// Macs that can be drawn at registry epoch `epoch` (from the state its first
/// block builds on): registered with a valid iroh node id, alive in the previous
/// epoch, with at least `min_streak` epochs of unbroken liveness. One ticket
/// each: a wallet or owner with many addresses gains nothing, only more Macs.
pub fn eligible(state: &WorldState, epoch: u64, min_streak: u64) -> Vec<(String, String)> {
    if epoch == 0 {
        return vec![];
    }
    registry::candidates(state)
        .into_iter()
        // Up at least 95% of the time during the streak (missed * 20 <= streak).
        .filter(|c| c.last_epoch == epoch - 1 && c.streak >= min_streak && c.missed.saturating_mul(20) <= c.streak)
        .filter_map(|c| aether_net::EndpointId::from_bytes(&c.node_id).ok().map(|n| (hex::encode(c.validator_key), n.to_string())))
        .collect()
}

/// Registered voting keys (hex) and their operators (the address that registered them).
pub fn operators(state: &WorldState) -> std::collections::HashMap<String, String> {
    registry::candidates(state).iter().map(|c| (hex::encode(c.validator_key), format!("{:#x}", c.operator))).collect()
}

/// A Mac's place in the draw: H(seed ‖ voting key).
pub fn ticket(seed: &[u8], key: &str) -> Vec<u8> {
    use commonware_cryptography::Hasher as _;
    let mut h = commonware_cryptography::Sha256::default();
    h.update(seed);
    h.update(key.as_bytes());
    h.finalize().1.as_ref().to_vec()
}

/// Voting-set size for a pool: a quarter of it (a sample, not the whole
/// pool), as 3f+1, between 4 and `MAX_DRAWN`; small pools seat everyone.
pub fn target_size(pool: usize) -> usize {
    let n = (pool / 4).clamp(MIN_OPEN_COMMITTEE, MAX_DRAWN).min(pool);
    3 * ((n.max(1) - 1) / 3) + 1
}

/// The next voting set: the pool ordered by H(seed ‖ key), the first
/// `target_size` drawn with at most f of 3f+1 seats per operator (one owner
/// never holds a third); fewer than a third of the running seats change per
/// draw (at least one), members that are not candidates leave first. None when
/// the pool is too small or nothing changes. `candidate(key)` is the operator
/// of a registered voting key (None: not registered).
///
/// The operator is the address that registered the key, so the cap binds an
/// account, not a person: one owner using many addresses is limited only by
/// what makes each voting key cost something (one per Mac via DeviceCheck, at
/// most 16 new per epoch, a day of unbroken liveness before being drawn).
pub fn draw(pool: &[(String, String)], seed: &[u8], candidate: impl Fn(&str) -> Option<String>, running: &Committee) -> Option<Vec<(String, String)>> {
    draw_sized(pool, seed, candidate, running, target_size(pool.len()), MIN_OPEN_COMMITTEE)
}

/// `draw` with its target size and the size it never shrinks below given.
fn draw_sized(
    pool: &[(String, String)],
    seed: &[u8],
    candidate: impl Fn(&str) -> Option<String>,
    running: &Committee,
    target: usize,
    floor: usize,
) -> Option<Vec<(String, String)>> {
    if running.members.is_empty() || pool.len() < MIN_OPEN_COMMITTEE {
        return None;
    }
    let mut order: Vec<&(String, String)> = pool.iter().collect();
    order.sort_by_cached_key(|(k, _)| ticket(seed, k));
    let cap = ((target - 1) / 3).max(1);
    let mut seats: std::collections::HashMap<String, usize> = Default::default();
    let selected: Vec<(String, String)> = order
        .into_iter()
        .filter(|(k, _)| {
            let taken = seats.entry(candidate(k).unwrap_or_else(|| k.clone())).or_default();
            *taken += 1;
            *taken <= cap
        })
        .take(target)
        .cloned()
        .collect();
    let chosen = |k: &str| selected.iter().any(|(s, _)| s == k);
    let incoming: Vec<&(String, String)> = selected.iter().filter(|(k, _)| !running.has(k)).collect();
    let mut outgoing: Vec<&(String, String)> = running.members.iter().filter(|(k, _)| !chosen(k)).collect();
    outgoing.sort_by_key(|(k, _)| candidate(k).is_some());
    let n = running.members.len();
    let budget = (n.saturating_sub(1) / 3).max(1);
    let swaps = budget.min(incoming.len()).min(outgoing.len());
    let grow = budget.saturating_sub(swaps).min(incoming.len() - swaps).min(selected.len().saturating_sub(n)).min(MAX_VOTING_NODES.saturating_sub(n));
    let shrink = budget.saturating_sub(swaps).min(outgoing.len() - swaps).min(n.saturating_sub(selected.len().max(floor)));
    let leaving: Vec<&String> = outgoing.iter().take(swaps + shrink).map(|(k, _)| k).collect();
    let mut next: Vec<(String, String)> = running.members.iter().filter(|(k, _)| !leaving.contains(&k)).cloned().collect();
    // The cap counts the seats an operator keeps too: an incoming key is added
    // only while its operator holds fewer than `cap` seats in the new set.
    let op = |k: &str| candidate(k).unwrap_or_else(|| k.to_string());
    let mut held: std::collections::HashMap<String, usize> = Default::default();
    for (k, _) in &next {
        *held.entry(op(k)).or_default() += 1;
    }
    for m in incoming.iter().take(swaps + grow) {
        let n = held.entry(op(&m.0)).or_default();
        if *n < cap {
            *n += 1;
            next.push((*m).clone());
        }
    }
    let changed = next.len() != n || next.iter().any(|(k, _)| !running.has(k));
    (next.len() >= MIN_OPEN_COMMITTEE && changed).then_some(next)
}

/// Protocol 3: up to this many seats, a Mac that qualifies joins the voting set
/// instead of replacing a member (the same count that later turns mainnet
/// issuance on). From here on, draws swap seats as in `draw`.
pub const GROW_UNTIL: usize = 16;

/// Protocol-3 draw. Below `GROW_UNTIL` seats, qualifying Macs are added (in
/// ticket order, fewer than a third of the running seats per draw, at least
/// one) and nobody leaves, so every Mac that keeps a day of unbroken uptime
/// becomes a voter; one operator still never holds a third of the new set.
/// At `GROW_UNTIL` seats and above it is `draw`.
pub fn draw_v3(pool: &[(String, String)], seed: &[u8], candidate: impl Fn(&str) -> Option<String>, running: &Committee) -> Option<Vec<(String, String)>> {
    let n = running.members.len();
    if n >= GROW_UNTIL {
        // Swap seats from here on, but never below sixteen.
        let target = target_size(pool.len()).max(GROW_UNTIL);
        return draw_sized(pool, seed, candidate, running, target, GROW_UNTIL);
    }
    if n == 0 {
        return None;
    }
    let mut incoming: Vec<&(String, String)> = pool.iter().filter(|(k, _)| !running.has(k)).collect();
    incoming.sort_by_cached_key(|(k, _)| ticket(seed, k));
    let budget = (n.saturating_sub(1) / 3).max(1).min(GROW_UNTIL - n).min(incoming.len());
    if budget == 0 {
        return None;
    }
    let cap = ((n + budget - 1) / 3).max(1);
    let op = |k: &str| candidate(k).unwrap_or_else(|| k.to_string());
    let mut held: std::collections::HashMap<String, usize> = Default::default();
    for (k, _) in &running.members {
        *held.entry(op(k)).or_default() += 1;
    }
    let mut next = running.members.clone();
    for m in incoming {
        if next.len() - n == budget {
            break;
        }
        let h = held.entry(op(&m.0)).or_default();
        if *h < cap {
            *h += 1;
            next.push(m.clone());
        }
    }
    (next.len() > n).then_some(next)
}

// --- docs/design/13-roadmap.md, F: a committee that sleeps ---

/// The fixed point every probability below lives on: a probability of 1 is
/// this many units (`beacons::PROB_SCALE`, a billion). These rules decide the
/// next committee, which every validator must compute bit-identically, so all
/// of their math is integer math on this scale — products in u128, rounded
/// down — and never floating point.
const SCALE: u64 = beacons::PROB_SCALE;

/// Availability a seat is given where no profile reaches it (a Mac with no
/// data yet, an hour nobody has observed): a healthy desktop, so a new Mac is
/// not punished for being new — and with no profiles at all every candidate
/// scores the same, the ticket decides, and the spread draw is `draw_v3`.
pub const PRIOR: u64 = 99 * SCALE / 100;
/// "Always on": a candidate up at least this much in every hour of the day.
pub const ALWAYS_ON: u64 = 95 * SCALE / 100;
/// A reserve key's uptime. The keys run on the founder's one Mac: each is
/// nearly always up, but all of them fall together.
pub const RESERVE_UP: u64 = 9999 * SCALE / 10000;
/// Reserve keys seat themselves while the committee's worst hour of the day
/// is likelier than this to lose its quorum (docs/design/13-roadmap.md, F).
pub const RESERVE_JOIN_BELOW: u64 = 99 * SCALE / 100;
/// ...and step down only once the odds are comfortably back above this. The
/// gap between the two thresholds is hysteresis: a committee on the edge
/// does not flap keys in and out every epoch.
pub const RESERVE_LEAVE_ABOVE: u64 = 199 * SCALE / 200;
/// Two picks whose worst-hour odds differ by less than this are a tie: the
/// seed decides those, so which Macs are drawn stays unpredictable. Scores
/// are integers on `SCALE`, so a tie is plain equality — one unit is 1e-9.
const TIE: u64 = 1;
/// A committee member is silent in an epoch it answered fewer than this many
/// of the four beacon slots in.
pub const SILENT_BELOW: u64 = 2;

/// A seat's availability per hour bucket of the day, from its beacon profile
/// (`SCALE` units to 1).
pub type Hours = [u64; DAY_EPOCHS as usize];

/// Votes a committee of `n` seats needs: n − ⌊(n−1)/3⌋.
fn quorum(n: usize) -> usize {
    if n == 0 {
        0
    } else {
        n - (n - 1) / 3
    }
}

/// One more seat, up with probability `p` (`SCALE` units), folded into an
/// hour's Poisson-binomial tail: `t[q]` is P(≥ q seats up), and the new seat
/// shifts every quorum ask one way or the other.
fn fold(t: &mut Vec<u64>, p: u64) {
    let mut next = vec![SCALE; t.len() + 1];
    for q in 1..next.len() {
        next[q] = ((t[q - 1] as u128 * p as u128 + t.get(q).copied().unwrap_or(0) as u128 * (SCALE - p) as u128) / SCALE as u128) as u64;
    }
    *t = next;
}

/// A seat set's per-hour Poisson-binomial tails: `tails[h][q]` = P(at least q
/// of the seats are up in hour bucket `h`), in `SCALE` units. Nothing here is
/// self-reported — the probabilities are the seats' beacon-answer profiles,
/// so a Mac that sleeps every night shows up as exactly that.
fn tails(members: &[(String, String)], hours: &impl Fn(&str) -> Option<Hours>) -> Vec<Vec<u64>> {
    let mut t = vec![vec![SCALE]; DAY_EPOCHS as usize];
    for (k, _) in members {
        let p = hours(k);
        for (h, th) in t.iter_mut().enumerate() {
            fold(th, p.map_or(PRIOR, |a| a[h]));
        }
    }
    t
}

/// P(≥ `q` seats up) at the tails' worst hour of the day, in `SCALE` units.
fn worst_at(t: &[Vec<u64>], q: usize) -> u64 {
    t.iter().map(|t| t.get(q).copied().unwrap_or(0)).fold(SCALE, u64::min)
}

/// P(a quorum of `members` is up) at their worst hour of the day (what the
/// tests ask of a drawn set), in `SCALE` units.
#[cfg(test)]
fn worst_hour(members: &[(String, String)], hours: &impl Fn(&str) -> Option<Hours>) -> u64 {
    worst_at(&tails(members, hours), quorum(members.len()))
}

/// P(a quorum of the bare committee plus `r` reserve keys is up) at the worst
/// hour. The keys share the founder's one Mac, so they are not independent
/// seats: with probability `RESERVE_UP` all `r` are up and the rest must
/// muster `q − r`, otherwise they must muster the whole `q` alone.
fn worst_with(bare: &[Vec<u64>], r: usize) -> u64 {
    let at = |t: &Vec<u64>, q: usize| if q == 0 { SCALE } else { t.get(q).copied().unwrap_or(0) };
    let q = quorum(bare[0].len() - 1 + r);
    bare.iter()
        .map(|t| ((at(t, q.saturating_sub(r)) as u128 * RESERVE_UP as u128 + at(t, q) as u128 * (SCALE - RESERVE_UP) as u128) / SCALE as u128) as u64)
        .fold(SCALE, u64::min)
}

/// The worst hour's quorum odds after adding a seat with availability `p`:
/// the tails convolved with one more Bernoulli, asked at the larger
/// committee's quorum — the same words `fold` will produce, so the pick's
/// score is exactly the set it leaves behind.
fn score(t: &[Vec<u64>], q: usize, p: Option<&Hours>) -> u64 {
    let at = |t: &Vec<u64>, q: usize| if q == 0 { SCALE } else { t.get(q).copied().unwrap_or(0) };
    t.iter()
        .zip(p.map_or([PRIOR; DAY_EPOCHS as usize], |p| *p))
        .map(|(t, p)| ((at(t, q.saturating_sub(1)) as u128 * p as u128 + at(t, q) as u128 * (SCALE - p) as u128) / SCALE as u128) as u64)
        .fold(SCALE, u64::min)
}

/// The greedy seat-picker the spread draw and early replacement share: always
/// the candidate that leaves the committee the best odds of a quorum at its
/// worst hour of the day. Near-ties go to the always-on candidate, then to
/// the lowest ticket — the seed still decides who is drawn.
struct Greedy<'a> {
    seed: &'a [u8],
    hours: &'a dyn Fn(&str) -> Option<Hours>,
    /// Seats each operator already holds (new ones included).
    held: std::collections::HashMap<String, usize>,
    /// An operator never holds a third of the committee.
    cap: usize,
    op: &'a dyn Fn(&str) -> String,
}

impl Greedy<'_> {
    /// The next seat from `order` given the committee's per-hour tails, or
    /// None when no candidate is left that the cap allows.
    fn pick<'b>(&self, order: &[&'b (String, String)], t: &[Vec<u64>]) -> Option<&'b (String, String)> {
        let q = quorum(t[0].len());
        order.iter()
            .filter(|m| *self.held.get(&(self.op)(&m.0)).unwrap_or(&0) < self.cap)
            .map(|m| {
                let p = (self.hours)(&m.0);
                let s = score(t, q, p.as_ref()) / TIE;
                // Descending score, always-on first, lowest ticket — an
                // integer grid, so a tie is plain equality and the pick does
                // not depend on the order the candidates are listed in.
                (
                    std::cmp::Reverse(s),
                    !p.is_some_and(|p| p.iter().all(|&x| x >= ALWAYS_ON)),
                    ticket(self.seed, &m.0),
                    *m,
                )
            })
            .min_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)))
            .map(|x| x.3)
    }

    /// Seat `m`: fold its availability into the tails and the operator's count.
    fn seat(&mut self, m: &(String, String), t: &mut [Vec<u64>]) {
        let p = (self.hours)(&m.0);
        for (h, th) in t.iter_mut().enumerate() {
            fold(th, p.map_or(PRIOR, |a| a[h]));
        }
        *self.held.entry((self.op)(&m.0)).or_default() += 1;
    }
}

/// Every registered Mac's availability per hour bucket, keyed by voting key
/// hex: the spread draw's input, read from the beacon profiles only. Hours
/// the profile has not seen fall back to `PRIOR`.
pub fn availability(state: &WorldState) -> std::collections::HashMap<String, Hours> {
    let mut out = std::collections::HashMap::new();
    for c in registry::candidates(state) {
        let p = beacons::profile(state, c.index);
        let mut hours = [PRIOR; DAY_EPOCHS as usize];
        for (h, x) in hours.iter_mut().enumerate() {
            if let Some(a) = p.at(h as u64) {
                *x = a;
            }
        }
        out.insert(hex::encode(c.validator_key), hours);
    }
    out
}

/// Every registered Mac's last two epochs' beacon counts, keyed by voting key
/// hex: what `replace_silent` reads to spot a silent member.
pub fn recents(state: &WorldState) -> std::collections::HashMap<String, (u64, u64, u64)> {
    registry::candidates(state)
        .into_iter()
        .filter_map(|c| beacons::recent(state, c.index).map(|r| (hex::encode(c.validator_key), r)))
        .collect()
}

/// The spread draw (docs/design/13-roadmap.md, F): `draw_v3` with the worst
/// hour in mind. Early Macs cluster in one time zone, and a one-time-zone
/// committee stalls every night even at sixteen seats — so while the committee
/// grows to `GROW_UNTIL`, each added seat is the candidate that leaves it the
/// best odds of a quorum at the worst hour of the day. The budget, the
/// per-operator cap and the swap path at `GROW_UNTIL` and above are
/// `draw_v3`'s; with no profiles every candidate scores the same and this is
/// exactly `draw_v3`. None when nothing changes.
pub fn draw_spread(
    pool: &[(String, String)],
    seed: &[u8],
    candidate: impl Fn(&str) -> Option<String>,
    running: &Committee,
    hours: impl Fn(&str) -> Option<Hours>,
) -> Option<Vec<(String, String)>> {
    let n = running.members.len();
    if n >= GROW_UNTIL {
        // Swap seats from here on, but never below sixteen.
        let target = target_size(pool.len()).max(GROW_UNTIL);
        return draw_sized(pool, seed, candidate, running, target, GROW_UNTIL);
    }
    if n == 0 {
        return None;
    }
    let mut order: Vec<&(String, String)> = pool.iter().filter(|(k, _)| !running.has(k)).collect();
    let budget = (n.saturating_sub(1) / 3).max(1).min(GROW_UNTIL - n).min(order.len());
    if budget == 0 {
        return None;
    }
    let op = |k: &str| candidate(k).unwrap_or_else(|| k.to_string());
    let mut held: std::collections::HashMap<String, usize> = Default::default();
    for (k, _) in &running.members {
        *held.entry(op(k)).or_default() += 1;
    }
    let mut greedy = Greedy { seed, hours: &hours, held, cap: ((n + budget - 1) / 3).max(1), op: &op };
    let mut next = running.members.clone();
    let mut t = tails(&next, &hours);
    for _ in 0..budget {
        let Some(m) = greedy.pick(&order, &t) else {
            break;
        };
        let m = m.clone();
        greedy.seat(&m, &mut t);
        order.retain(|x| x.0 != m.0);
        next.push(m);
    }
    (next.len() > n).then_some(next)
}

/// A key's worst hour of the day (its profile's worst bucket; `PRIOR` with no
/// profile). Early replacement sends the worst profiles out first: the
/// silence record condemns a member, its profile only orders them.
fn worst_of(key: &str, hours: &impl Fn(&str) -> Option<Hours>) -> u64 {
    hours(key).map_or(PRIOR, |p| p.iter().copied().fold(PRIOR, u64::min))
}

/// Early replacement (docs/design/13-roadmap.md, F): a substitution is a
/// reshare, and a reshare needs the old committee's quorum — so a member
/// going silent must be replaced while the quorum still stands. Members that
/// answered fewer than `SILENT_BELOW` of an epoch's four beacon slots in each
/// of the last two epochs (`recent`, written by the beacon distributions)
/// hand their seat to the candidate the spread rule picks. At most a third
/// minus one of the seats change per epoch, and a committee under six seats
/// replaces nobody: the old set must keep the quorum its handoff signs with.
/// Worst profiles go first, ties by ticket; seats nobody can fill keep their
/// member, so replacement never shrinks the committee. None when nothing
/// changes.
pub fn replace_silent(
    running: &Committee,
    pool: &[(String, String)],
    seed: &[u8],
    candidate: impl Fn(&str) -> Option<String>,
    hours: impl Fn(&str) -> Option<Hours>,
    recent: impl Fn(&str) -> Option<(u64, u64, u64)>,
    epoch: u64,
) -> Option<Vec<(String, String)>> {
    let n = running.members.len();
    let swaps = (n / 3).saturating_sub(1);
    if swaps == 0 || epoch == 0 {
        return None;
    }
    // Silent through the last two whole epochs: the record must cover exactly
    // those (a gap or a first epoch counts as unknown, never as silence).
    let silent = |k: &str| {
        matches!(recent(k), Some((e, last, prev))
            if e + 1 == epoch && last != beacons::NO_COUNT && prev != beacons::NO_COUNT && last < SILENT_BELOW && prev < SILENT_BELOW)
    };
    let mut going: Vec<&(String, String)> = running.members.iter().filter(|(k, _)| silent(k)).collect();
    if going.is_empty() {
        return None;
    }
    going.sort_by(|a, b| {
        worst_of(&a.0, &hours)
            .cmp(&worst_of(&b.0, &hours))
            .then_with(|| ticket(seed, &a.0).cmp(&ticket(seed, &b.0)))
    });
    going.truncate(swaps);
    let op = |k: &str| candidate(k).unwrap_or_else(|| k.to_string());
    let mut greedy = Greedy { seed, hours: &hours, held: Default::default(), cap: ((n - 1) / 3).max(1), op: &op };
    let mut next: Vec<(String, String)> = running
        .members
        .iter()
        .filter(|(k, _)| !going.iter().any(|(g, _)| g == k))
        .cloned()
        .collect();
    for (k, _) in &next {
        *greedy.held.entry(op(k)).or_default() += 1;
    }
    let mut order: Vec<&(String, String)> = pool.iter().filter(|(k, _)| !running.has(k)).collect();
    let mut t = tails(&next, &hours);
    let mut filled = 0;
    while filled < going.len() {
        let Some(m) = greedy.pick(&order, &t) else {
            break;
        };
        let m = m.clone();
        greedy.seat(&m, &mut t);
        order.retain(|x| x.0 != m.0);
        next.push(m);
        filled += 1;
    }
    // Seats nobody could fill keep their silent member for now.
    next.extend(going[filled..].iter().map(|(k, node)| (k.clone(), node.clone())));
    (next.len() == n && next.iter().any(|m| !running.members.contains(m))).then_some(next)
}

/// Founder reserve keys (docs/design/12-launch-plan.md, "창업자 Mac 안전망"): the
/// founder's one extra right is running up to three voting keys on one Mac.
/// They are not registry candidates (no beacons, no rewards) and are seated
/// only while fewer than `MIN_OPEN_COMMITTEE` independent operators qualify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reserve {
    /// The founder's operator address (lowercase 0x hex): its own Macs are not independent.
    pub operator: String,
    /// (ed25519 key hex, iroh node id).
    pub members: Vec<(String, String)>,
}

impl Reserve {
    fn has(&self, key: &str) -> bool {
        self.members.iter().any(|(k, _)| k == key)
    }
}

/// Distinct operators, other than the founder, with a Mac in the pool.
pub fn independent(pool: &[(String, String)], candidate: impl Fn(&str) -> Option<String>, reserve: &Reserve) -> usize {
    pool.iter()
        .filter(|(k, _)| !reserve.has(k))
        .filter_map(|(k, _)| candidate(k))
        .filter(|op| *op != reserve.operator)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

/// The next voting set with the founder's reserve keys applied to `drawn`
/// (the draw's result, or None: the running set). While fewer than
/// `MIN_OPEN_COMMITTEE` independent operators qualify, reserve keys fill only
/// the seats the committee is short of `MIN_OPEN_COMMITTEE` — never more: a
/// committee that already stands at four seats takes none, because growing it
/// to seven would put three seats on the founder's one Mac and raise the
/// quorum past what the other validators alone can meet, so that one Mac
/// going offline would stall the chain. A committee short of seats first
/// takes qualifying Macs (ticket order, one seat per operator) and reserve
/// keys make up the rest; keys seated beyond the shortfall step down.
///
/// From four independent operators on, the seats are the Macs' own to hold —
/// but the keys are also a liveness safety net (docs/design/13-roadmap.md, F):
/// they seat themselves, as many as the odds need, while the committee's
/// worst hour of the day is likelier than `RESERVE_JOIN_BELOW` to lose its
/// quorum, and step down once it is comfortably back above
/// `RESERVE_LEAVE_ABOVE`. The keys share one Mac, so they are counted
/// together, never as independent seats; they earn nothing, and once the
/// quorum is already lost no substitution can help — recovery is the runbook.
/// None when nothing changes.
pub fn with_reserve(
    drawn: Option<Vec<(String, String)>>,
    pool: &[(String, String)],
    seed: &[u8],
    candidate: impl Fn(&str) -> Option<String>,
    reserve: &Reserve,
    running: &Committee,
    hours: impl Fn(&str) -> Option<Hours>,
) -> Option<Vec<(String, String)>> {
    if running.members.is_empty() {
        return drawn;
    }
    let needed = independent(pool, &candidate, reserve) < MIN_OPEN_COMMITTEE;
    // The keys seated right now, read from the running committee before the
    // draw's result replaces them: a normal draw orders non-candidate members
    // out first, so its set can carry none of the keys — and the hysteresis
    // band below must keep the keys the committee actually seats, not read the
    // draw's absence as a step down.
    let seated = running.members.iter().filter(|(k, _)| reserve.has(k)).count();
    let mut next = drawn.clone().unwrap_or_else(|| running.members.clone());
    // Qualifying Macs fill a committee that is short of seats (ticket order,
    // one seat per operator) — both when the reserve keys leave and while
    // they are seated.
    next.retain(|(k, _)| needed || !reserve.has(k));
    let op = |k: &str| candidate(k).unwrap_or_else(|| k.to_string());
    let mut seated_ops: std::collections::BTreeSet<String> = next.iter().map(|(k, _)| op(k)).collect();
    let mut order: Vec<&(String, String)> = pool.iter().filter(|(k, _)| !next.iter().any(|(n, _)| n == k)).collect();
    order.sort_by_cached_key(|(k, _)| ticket(seed, k));
    for m in order {
        if next.len() >= MIN_OPEN_COMMITTEE {
            break;
        }
        if seated_ops.insert(op(&m.0)) {
            next.push(m.clone());
        }
    }
    if needed {
        // Only as many reserve keys as the committee is still short of
        // MIN_OPEN_COMMITTEE seats, never more; keys seated beyond the
        // shortfall step down. The cap stays three (all of them).
        let short = MIN_OPEN_COMMITTEE.saturating_sub(next.iter().filter(|(k, _)| !reserve.has(k)).count()).min(reserve.members.len());
        let mut seated = 0;
        next.retain(|(k, _)| {
            !reserve.has(k) || {
                seated += 1;
                seated <= short
            }
        });
        for m in &reserve.members {
            if seated >= short {
                break;
            }
            if !next.iter().any(|(k, _)| *k == m.0) {
                next.push(m.clone());
                seated += 1;
            }
        }
    } else if next.len() < MIN_OPEN_COMMITTEE {
        // Dropping the reserve keys would leave the committee short and no
        // qualifying Mac can fill it: they stay seated for now.
        return drawn;
    } else {
        // The committee stands on its own seats; the keys' staying is a
        // question of its predicted worst hour (the profiles are public
        // state, so every node asks the same question and gets the same
        // answer).
        let t = tails(&next, &hours);
        let bare = worst_at(&t, quorum(next.len()));
        if bare < RESERVE_LEAVE_ABOVE {
            let want = if bare < RESERVE_JOIN_BELOW {
                // As many keys as the odds need: the smallest count that
                // restores the threshold, else the count that helps most
                // (which may be none — a committee the keys cannot save is
                // left to itself).
                let mut chosen = 0;
                let mut best = bare;
                for r in 1..=reserve.members.len() {
                    let p = worst_with(&t, r);
                    if p >= RESERVE_JOIN_BELOW {
                        chosen = r;
                        break;
                    }
                    if p > best {
                        best = p;
                        chosen = r;
                    }
                }
                chosen
            } else {
                // The hysteresis band: the current seating stays as it is.
                seated
            };
            next.extend(reserve.members.iter().take(want).cloned());
        }
    }
    let same = next.len() == running.members.len() && next.iter().all(|m| running.members.contains(m));
    (!same).then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_crypto::{address_of, P256Signer, Signer};
    use aether_execution::registry::{attestation_message, encode_beacon, encode_register, REGISTRY};
    use aether_execution::{execute_block, sign_call, BlockContext, EvmCall};
    use aether_types::{Address, GasVector, U256};

    const E: u64 = 10;

    fn seed(b: u8) -> [u8; 32] {
        let mut s = [0u8; 32];
        s[0] = 0x51;
        s[31] = b;
        s
    }

    fn run(state: &WorldState, from: &P256Signer, nonce: u64, input: aether_types::Bytes, block: u64) -> WorldState {
        let ctx = BlockContext {
            chain_id: 7,
            number: block,
            timestamp: block,
            beneficiary: Address::ZERO,
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            fees: None,
        };
        let call = EvmCall { to: Some(REGISTRY), value: U256::ZERO, input, gas_limit: 400_000, delegate: None };
        let out = execute_block(state, &ctx, &[sign_call(from, 7, nonce, 1, &call).unwrap()]).unwrap();
        assert!(out.receipts[0].success);
        out.state
    }

    /// `n` candidates from `n` operators, each beaconing in epoch `e`.
    fn registry_with(n: u8, e: u64) -> WorldState {
        let registrar = P256Signer::from_seed(&seed(0)).unwrap();
        let mut s = WorldState::default();
        registry::predeploy(
            &mut s,
            aether_crypto::p256_xy(&registrar.public_key().bytes).unwrap(),
            registry::Params { epoch_blocks: E, min_streak: 0, draw_epochs: 1 },
        )
        .unwrap();
        for i in 1..=n {
            let op = P256Signer::from_seed(&seed(i)).unwrap();
            let a = address_of(&op.public_key()).unwrap();
            s.set_balance(a, U256::from(10u128.pow(20))).unwrap();
            let sig = registrar.sign(&attestation_message(7, a, [i; 32], node(i), a)).unwrap();
            s = run(&s, &op, 0, encode_register([i; 32], node(i), a, sig[..32].try_into().unwrap(), sig[32..64].try_into().unwrap()), (e - 1) * E + 1);
            // Registered in epoch e-1, beacon in e: no epoch missed.
            s = run(&s, &op, 1, encode_beacon([i; 32]), e * E + 1);
        }
        s
    }

    fn node(i: u8) -> [u8; 32] {
        *aether_net::SecretKey::from_bytes(&[i; 32]).public().as_bytes()
    }

    fn key(i: u8) -> String {
        hex::encode([i; 32])
    }

    fn running(keys: &[u8]) -> Committee {
        Committee { members: keys.iter().map(|k| (key(*k), format!("node{k}"))).collect() }
    }

    fn draw_at(s: &WorldState, epoch: u64, seed: &[u8], set: &Committee) -> Option<Vec<(String, String)>> {
        let ops = operators(s);
        draw(&eligible(s, epoch, 0), seed, |k| ops.get(k).cloned(), set)
    }

    #[test]
    fn the_cap_counts_the_seats_an_operator_already_holds() {
        // Running {1(whale), 100, 101, 102}; the pool is mostly the whale's: it may not gain a second seat.
        let pool: Vec<(String, String)> = (1..=16u8).map(|i| (key(i), format!("node{i}"))).collect();
        let op = |k: &str| Some(if k <= key(12).as_str() { "0xwhale".to_string() } else { k.to_string() });
        let set = running(&[1, 100, 101, 102]);
        for seed in [b"a".as_slice(), b"b", b"c", b"d", b"e", b"f", b"g"] {
            if let Some(next) = draw(&pool, seed, op, &set) {
                let whale = next.iter().filter(|(k, _)| op(k).as_deref() == Some("0xwhale")).count();
                assert!(whale <= 1, "{whale} whale seats of {}", next.len());
            }
        }
    }

    #[test]
    fn one_operator_never_holds_a_third_of_the_drawn_seats() {
        // 16 eligible Macs, 12 of one owner: a 4-seat draw takes at most one of theirs.
        let pool: Vec<(String, String)> = (1..=16u8).map(|i| (key(i), format!("node{i}"))).collect();
        let op = |k: &str| Some(if k < key(13).as_str() { "0xwhale".to_string() } else { k.to_string() });
        let set = running(&[100, 101, 102, 103]);
        for seed in [b"a".as_slice(), b"b", b"c", b"d", b"e"] {
            let drawn = draw(&pool, seed, op, &set).unwrap();
            let whale = drawn.iter().filter(|(k, _)| op(k).as_deref() == Some("0xwhale")).count();
            assert!(whale <= 1, "{whale} whale seats of {}", drawn.len());
        }
    }

    #[test]
    fn a_third_of_the_seats_change_per_draw_starting_with_non_candidates() {
        let s = registry_with(5, 3);
        let genesis_set = running(&[0xa1, 0xa2, 0xa3, 0xa4]);
        let next = draw_at(&s, 4, b"seed", &genesis_set).expect("live candidates replace the genesis set");
        assert_eq!(next.len(), 4);
        assert_eq!(next.iter().filter(|(k, _)| k.starts_with("a")).count(), 3, "one seat changes");
        assert!(draw_at(&s, 5, b"seed", &genesis_set).is_none(), "nobody beaconed in epoch 4");
        assert!(draw_at(&s, 4, b"seed", &Committee::default()).is_none(), "followers draw nothing");
        // Repeated draws converge on the drawn set (five eligible: target 4 of them).
        let mut set = genesis_set;
        for _ in 0..6 {
            if let Some(n) = draw_at(&s, 4, b"seed", &set) {
                set = Committee { members: n };
            }
        }
        assert_eq!(set.members.len(), 4);
        assert!(set.members.iter().all(|(k, _)| !k.starts_with("a")), "only candidates remain");
    }

    #[test]
    fn the_seed_decides_who_is_drawn_and_nobody_else_can() {
        // A large pool: the draw is a sample, and different seeds draw different sets.
        let pool: Vec<(String, String)> = (0..64u8).map(|i| (hex::encode([i; 32]), format!("n{i}"))).collect();
        let set = Committee { members: pool[..16].to_vec() };
        assert_eq!(target_size(64), 16);
        let sample = |seed: &[u8]| {
            let mut order: Vec<&(String, String)> = pool.iter().collect();
            order.sort_by_cached_key(|(k, _)| ticket(seed, k));
            order.into_iter().take(16).map(|(k, _)| k.clone()).collect::<Vec<_>>()
        };
        assert_ne!(sample(b"one"), sample(b"two"));
        // Wallet addresses play no part: the draw only sees keys (one per Mac).
        let a = draw(&pool, b"one", |k| Some(k.to_string()), &set).unwrap();
        let b = draw(&pool, b"one", |k| Some(k.to_string()), &set).unwrap();
        assert_eq!(a, b, "deterministic for everyone");
        assert!(a.len() == 16 && a.iter().filter(|m| !set.members.contains(m)).count() <= 5, "fewer than a third change");
        assert_eq!(target_size(4), 4);
        assert_eq!(target_size(1000), 127);
    }

    fn committee(n: u8) -> Committee {
        Committee { members: (0..n).map(|i| (format!("g{i}"), format!("gn{i}"))).collect() }
    }

    fn mac(i: u8) -> (String, String) {
        (format!("m{i}"), format!("mn{i}"))
    }

    #[test]
    fn v3_one_qualifying_mac_joins_four_genesis_seats() {
        let next = draw_v3(&[mac(1)], &seed(1), |k| k.starts_with('m').then(|| format!("op-{k}")), &committee(4)).expect("grows");
        assert_eq!(next.len(), 5);
        assert!(next.iter().any(|(k, _)| k == "m1"));
        assert!(committee(4).members.iter().all(|m| next.contains(m)), "nobody leaves while growing");
    }

    #[test]
    fn v3_grows_by_less_than_a_third_per_draw_and_stops_at_sixteen() {
        let pool: Vec<_> = (1..=20).map(mac).collect();
        let ops = |k: &str| k.starts_with('m').then(|| format!("op-{k}"));
        assert_eq!(draw_v3(&pool, &seed(2), ops, &committee(4)).unwrap().len(), 5, "4 seats: one more");
        assert_eq!(draw_v3(&pool, &seed(2), ops, &committee(10)).unwrap().len(), 13, "10 seats: three more");
        assert_eq!(draw_v3(&pool, &seed(2), ops, &committee(15)).unwrap().len(), 16, "never past sixteen by growing");
        let at_cap = draw_v3(&pool, &seed(2), ops, &committee(16)).unwrap();
        assert_eq!(at_cap.len(), 16, "at sixteen, draws swap seats instead");
    }

    #[test]
    fn v3_one_operator_never_holds_a_third_of_the_grown_set() {
        // Two Macs of one operator; four genesis seats are each their own operator.
        let pool = vec![mac(1), mac(2)];
        let next = draw_v3(&pool, &seed(3), |k| k.starts_with('m').then(|| "same-owner".to_string()), &committee(4)).unwrap();
        assert_eq!(next.len(), 5, "one seat per draw at this size");
        let ten = draw_v3(&pool, &seed(3), |k| k.starts_with('m').then(|| "same-owner".to_string()), &committee(10)).unwrap();
        let theirs = ten.iter().filter(|(k, _)| k.starts_with('m')).count();
        assert!(theirs * 3 < ten.len(), "{theirs} of {} seats", ten.len());
    }

    #[test]
    fn v3_nothing_to_add_keeps_the_set() {
        assert!(draw_v3(&[], &seed(4), |_| None, &committee(4)).is_none());
        let running = committee(4);
        let already: Vec<_> = running.members.clone();
        assert!(draw_v3(&already, &seed(4), |_| None, &running).is_none());
    }

    fn reserve() -> Reserve {
        Reserve { operator: "0xf0".into(), members: (1..=3).map(|i| (format!("r{i}"), format!("rn{i}"))).collect() }
    }

    fn ops_of(k: &str) -> Option<String> {
        // Macs "m<i>" belong to operator "op<i>"; "f<i>" are the founder's own registered Macs.
        if let Some(i) = k.strip_prefix('m') {
            Some(format!("op{i}"))
        } else {
            k.starts_with('f').then(|| "0xf0".to_string())
        }
    }

    #[test]
    fn reserve_keys_fill_only_the_seats_a_short_committee_is_missing() {
        // The committee is the qualifying independents themselves: the reserve
        // keys make up the difference to four seats, and only that difference.
        let r = reserve();
        // No independent operator at all: only the founder's own registered Mac qualifies.
        let founder = ("f1".into(), "fn1".into());
        let next = with_reserve(None, std::slice::from_ref(&founder), &seed(5), ops_of, &r, &Committee { members: vec![founder.clone()] }, |_| None).expect("reserve joins");
        assert_eq!(next.len(), MIN_OPEN_COMMITTEE);
        assert!(r.members.iter().all(|m| next.contains(m)), "three seats short: every reserve key");
        // One to three independent operators seated: three, two, one reserve keys.
        for i in 1..=3u8 {
            let pool: Vec<_> = (1..=i).map(mac).collect();
            assert_eq!(independent(&pool, ops_of, &r), i as usize, "independents count");
            let next = with_reserve(None, &pool, &seed(5), ops_of, &r, &Committee { members: pool.clone() }, |_| None).expect("reserve joins");
            let seated = next.iter().filter(|(k, _)| k.starts_with('r')).count();
            assert_eq!((seated, next.len()), (MIN_OPEN_COMMITTEE - i as usize, MIN_OPEN_COMMITTEE), "{i} independent operator(s)");
        }
        // A committee that already stands at four seats takes no reserve key at
        // all, however few independent operators qualify: growing four seats
        // to seven would put three of them on the founder's one Mac.
        let pool = vec![mac(1), ("f1".into(), "fn1".into())];
        assert_eq!(independent(&pool, ops_of, &r), 1);
        assert!(with_reserve(None, &pool, &seed(5), ops_of, &r, &committee(4), |_| None).is_none(), "four seats standing: no reserve key joins");
        // Qualifying Macs fill a short committee before any reserve key does.
        let pool = vec![mac(1), mac(2), mac(3), ("f1".into(), "fn1".into())];
        let next = with_reserve(None, &pool, &seed(5), ops_of, &r, &Committee { members: vec![mac(1)] }, |_| None).expect("qualifying Macs fill the seats");
        assert!(next.iter().all(|(k, _)| !k.starts_with('r')), "the Macs cover the shortfall");
        assert_eq!(next.len(), MIN_OPEN_COMMITTEE);
        assert!(next.contains(&mac(2)) && next.contains(&mac(3)) && next.contains(&founder));
        // Already seated: nothing changes.
        let mut full = vec![mac(1)];
        full.extend(r.members.iter().cloned());
        assert!(with_reserve(None, std::slice::from_ref(&mac(1)), &seed(5), ops_of, &r, &Committee { members: full }, |_| None).is_none());
        // Many Macs of one operator are still one operator.
        let whale: Vec<_> = (0..10).map(|i| (format!("w{i}"), format!("wn{i}"))).collect();
        assert_eq!(independent(&whale, |_| Some("0xwhale".into()), &r), 1);
    }

    #[test]
    fn seated_reserve_keys_step_down_as_the_committee_fills() {
        // A committee that once carried every reserve key keeps fewer as other
        // seats stand, and none once four seats stand without them.
        let r = reserve();
        // The seven-seat set of the old rule (four genesis keys and every
        // reserve key): trimmed back to four seats.
        let mut legacy = committee(4).members;
        legacy.extend(r.members.iter().cloned());
        let next = with_reserve(None, std::slice::from_ref(&mac(1)), &seed(7), ops_of, &r, &Committee { members: legacy }, |_| None).expect("steps down");
        assert_eq!(next, committee(4).members, "a full committee keeps no reserve seat");
        // Two independents seated with three reserve keys: two step down.
        let mut seated = vec![mac(1), mac(2)];
        seated.extend(r.members.iter().cloned());
        let next = with_reserve(None, &seated, &seed(7), ops_of, &r, &Committee { members: seated.clone() }, |_| None).expect("steps down");
        assert_eq!(next, vec![mac(1), mac(2), r.members[0].clone(), r.members[1].clone()]);
        // A third independent operator seated: exactly one reserve key stays.
        let mut three = vec![mac(1), mac(2), mac(3)];
        three.push(r.members[0].clone());
        assert!(with_reserve(None, &three.clone(), &seed(7), ops_of, &r, &Committee { members: three }, |_| None).is_none(), "already the rule's seat count");
    }

    #[test]
    fn reserve_keys_leave_once_four_independent_operators_qualify() {
        let mut seated = vec![mac(1), mac(2)];
        seated.extend(reserve().members);
        let running = Committee { members: seated };
        let pool: Vec<_> = (1..=4).map(mac).collect();
        let next = with_reserve(None, &pool, &seed(6), ops_of, &reserve(), &running, |_| None).expect("reserve leaves");
        assert!(next.iter().all(|(k, _)| !k.starts_with('r')), "every reserve key leaves at once");
        assert_eq!(next.len(), MIN_OPEN_COMMITTEE, "qualifying Macs fill the seats");
        assert!(next.contains(&mac(3)) && next.contains(&mac(4)));
        // A draw result gets the same treatment.
        let drawn = vec![mac(1), mac(2), mac(3), mac(4), reserve().members[0].clone()];
        let next = with_reserve(Some(drawn), &pool, &seed(6), ops_of, &reserve(), &running, |_| None).unwrap();
        assert_eq!(next, vec![mac(1), mac(2), mac(3), mac(4)]);
        // Without reserve keys seated and enough operators: nothing to do.
        let clean = Committee { members: (1..=4).map(mac).collect() };
        assert!(with_reserve(None, &pool, &seed(6), ops_of, &reserve(), &clean, |_| None).is_none());
    }

    #[test]
    fn too_few_eligible_keep_the_running_set() {
        let s = registry_with(3, 3);
        assert!(draw_at(&s, 4, b"seed", &running(&[0xa1, 0xa2, 0xa3, 0xa4])).is_none());
    }

    /// `x`/`y` in `SCALE` units (the test's way of writing a probability).
    fn pct(x: u64, y: u64) -> u64 {
        x * SCALE / y
    }

    /// n seats all up with probability p at every hour.
    fn flat(n: usize, p: u64) -> (Vec<(String, String)>, impl Fn(&str) -> Option<Hours>) {
        let set: Vec<_> = (0..n).map(|i| (format!("k{i}"), format!("n{i}"))).collect();
        let hours = move |_: &str| -> Option<Hours> { Some([p; DAY_EPOCHS as usize]) };
        (set, hours)
    }

    #[test]
    fn the_odds_match_the_binomial_tables() {
        // docs/research/small-committee-liveness-2026.md: 4 seats at 0.9 keep
        // their quorum 94.77% of the time; 16 seats at 0.65 — one time zone
        // asleep — only about half. Integer fixed point rounds each fold down
        // by under one unit, so the tables match to a few parts in a billion.
        let (four, p9) = flat(4, pct(9, 10));
        assert!(worst_hour(&four, &p9).abs_diff(pct(9477, 10000)) < 1_000, "{:?}", worst_hour(&four, &p9));
        let (sixteen, p65) = flat(16, pct(65, 100));
        // The doc's "~50%" is the stall side: the quorum survives 49.0% of nights.
        let wh = worst_hour(&sixteen, &p65);
        assert!((pct(48, 100)..pct(50, 100)).contains(&wh), "{wh}");
        // No data at all: the PRIOR keeps a four-seat committee comfortably up.
        assert!(worst_hour(&four, &|_| None) > pct(999, 1000));
        // Three time zones eight hours apart cover each other's nights; one
        // zone alone stalls through every night of its own.
        let zone = |z: u64| std::array::from_fn(|h: usize| if ((h as u64 + 8 * z) % DAY_EPOCHS) < 16 { pct(95, 100) } else { pct(2, 10) });
        let mut avail = std::collections::HashMap::new();
        let mut spread = vec![];
        for z in 0..3u64 {
            for i in 0..5u64 {
                let k = format!("z{z}m{i}");
                avail.insert(k.clone(), zone(z));
                spread.push((k, format!("n{z}{i}")));
            }
        }
        let spread_hours = |k: &str| avail.get(k).copied();
        let (alone, alone_hours) = flat(15, pct(2, 10));
        assert!(worst_hour(&spread, &spread_hours) > worst_hour(&alone, &alone_hours) * 100);
    }

    #[test]
    fn the_spread_draw_covers_the_hours_the_running_set_sleeps_through() {
        // Three time zones eight hours apart: each candidate is up through its
        // zone's 16-hour day and dark (p 0.2) through its 8-hour night — the
        // early-pool shape, everyone in one country.
        let zone = |z: u64| std::array::from_fn(|h: usize| if ((h as u64 + 8 * z) % DAY_EPOCHS) < 16 { SCALE } else { pct(2, 10) });
        let mut avail = std::collections::HashMap::new();
        let mut pool = vec![];
        for (z, n) in [(0u64, 20u64), (1, 10), (2, 10)] {
            for i in 0..n {
                let k = format!("z{z}m{i}");
                avail.insert(k.clone(), zone(z));
                pool.push((k, format!("n{z}{i}")));
            }
        }
        let hours = |k: &str| avail.get(k).copied();
        let ops = |k: &str| Some(k.to_string());
        // Ten seats from one zone running, ten more of its Macs waiting in
        // the pool beside the other zones'; the draw may add three.
        let running = Committee { members: pool[..10].to_vec() };
        let next = draw_spread(&pool, &seed(1), ops, &running, hours).expect("grows");
        assert_eq!(next.len(), 13, "(10−1)/3 = 3 more seats");
        let added: Vec<&str> = next.iter().filter(|m| !running.has(&m.0)).map(|(k, _)| k.as_str()).collect();
        assert!(added.iter().all(|k| !k.starts_with("z0")), "the draw looks outside the sleeping zone: {added:?}");
        assert!(worst_hour(&next, &hours) > worst_hour(&running.members, &hours), "the worst hour improves");
        // Deterministic: the same state draws the same set for everyone.
        assert_eq!(draw_spread(&pool, &seed(1), ops, &running, hours).unwrap(), next);
        // The ticket-only draw never beats it, and sometimes sleeps.
        let mut slept = false;
        for s in 1..=10u8 {
            let spread = draw_spread(&pool, &seed(s), ops, &running, hours).unwrap();
            let ticketed = draw_v3(&pool, &seed(s), ops, &running).unwrap();
            assert!(worst_hour(&spread, &hours) >= worst_hour(&ticketed, &hours), "seed {s}");
            slept |= worst_hour(&spread, &hours) > worst_hour(&ticketed, &hours);
        }
        assert!(slept, "the ticket draw sometimes adds to the sleeping zone");
    }

    /// Three time zones' candidates plus an always-on cohort: zone `z`'s Macs
    /// are up through their 16-hour day and dark (p 1/5) through their 8-hour
    /// night; the always-on cohort is up around the clock.
    fn world() -> (std::collections::HashMap<String, Hours>, Vec<(String, String)>) {
        let zone = |z: u64| std::array::from_fn(|h: usize| if ((h as u64 + 8 * z) % DAY_EPOCHS) < 16 { SCALE } else { pct(1, 5) });
        let mut avail = std::collections::HashMap::new();
        let mut pool = vec![];
        for (z, n) in [(0u64, 20u64), (1, 10), (2, 10)] {
            for i in 0..n {
                let k = format!("z{z}m{i}");
                avail.insert(k.clone(), zone(z));
                pool.push((k, format!("n{z}{i}")));
            }
        }
        for i in 0..4u64 {
            avail.insert(format!("a{i}"), [SCALE; DAY_EPOCHS as usize]);
            pool.push((format!("a{i}"), format!("an{i}")));
        }
        (avail, pool)
    }

    #[test]
    fn the_same_state_draws_the_same_committee_every_time() {
        // The draw decides the next committee, so it is consensus: computed
        // twice it must land on the same words — and the order the pool
        // happens to be listed in (a HashMap's whim on a real node) must not
        // matter either.
        let (avail, pool) = world();
        let hours = |k: &str| avail.get(k).copied();
        let ops = |k: &str| Some(k.to_string());
        let running = Committee { members: pool[..10].to_vec() };
        let first = draw_spread(&pool, &seed(1), ops, &running, hours).expect("grows");
        assert_eq!(draw_spread(&pool, &seed(1), ops, &running, hours).unwrap(), first, "computed twice");
        let mut shuffled = pool.clone();
        shuffled.sort_by(|a, b| b.1.cmp(&a.1)); // by node id, not the insertion order
        assert_eq!(draw_spread(&shuffled, &seed(1), ops, &running, hours).unwrap(), first, "pool order does not matter");
        // The reserve keys' liveness rule computes the same words too.
        let r = Reserve { operator: "0xf0".into(), members: (1..=3).map(|i| (format!("r{i}"), format!("rn{i}"))).collect() };
        let seat = with_reserve(None, &pool, &seed(1), ops, &r, &running, hours).expect("a risky night seats a key");
        assert_eq!(with_reserve(None, &shuffled, &seed(1), ops, &r, &running, hours).unwrap(), seat);
        assert_eq!(with_reserve(None, &pool, &seed(1), ops, &r, &running, hours).unwrap(), seat);
    }

    #[test]
    fn a_fixed_state_draws_a_fixed_committee() {
        // Golden vector: any change to the draw's arithmetic — another
        // rounding, scale or tie grid — draws a different committee here.
        let (avail, pool) = world();
        let hours = |k: &str| avail.get(k).copied();
        let ops = |k: &str| Some(k.to_string());
        let running = Committee { members: pool[..10].to_vec() };
        assert_eq!(
            draw_spread(&pool, &seed(1), ops, &running, hours),
            Some(vec![
                ("z0m0".into(), "n00".into()),
                ("z0m1".into(), "n01".into()),
                ("z0m2".into(), "n02".into()),
                ("z0m3".into(), "n03".into()),
                ("z0m4".into(), "n04".into()),
                ("z0m5".into(), "n05".into()),
                ("z0m6".into(), "n06".into()),
                ("z0m7".into(), "n07".into()),
                ("z0m8".into(), "n08".into()),
                ("z0m9".into(), "n09".into()),
                // The three seats the budget allows all go to the always-on
                // cohort, in ticket order — the sleeping zone keeps none.
                ("a3".into(), "an3".into()),
                ("a1".into(), "an1".into()),
                ("a2".into(), "an2".into()),
            ])
        );
    }

    #[test]
    fn the_spread_draw_still_caps_one_operator() {
        // Ten seats, three of them one operator's; the pool is five more of
        // its always-on Macs: the cap (four of thirteen) allows just one more.
        let mut members: Vec<(String, String)> = (0..7).map(|i| (format!("g{i}"), format!("gn{i}"))).collect();
        let whale: Vec<(String, String)> = (0..3).map(|i| (format!("w{i}"), format!("wn{i}"))).collect();
        members.extend(whale);
        let pool: Vec<_> = (3..8).map(|i| (format!("w{i}"), format!("wn{i}"))).collect();
        let hours = |k: &str| k.starts_with('w').then_some([SCALE; DAY_EPOCHS as usize]);
        let ops = |k: &str| k.starts_with('w').then(|| "0xwhale".to_string());
        let running = Committee { members };
        let next = draw_spread(&pool, &seed(9), ops, &running, hours).expect("grows");
        assert_eq!(next.iter().filter(|(k, _)| k.starts_with('w')).count(), 4, "one more whale seat: the cap holds");
    }

    #[test]
    fn without_profiles_the_spread_draw_is_the_v3_draw() {
        let pool: Vec<_> = (1..=20).map(mac).collect();
        let ops = |k: &str| k.starts_with('m').then(|| format!("op-{k}"));
        for (s, n) in [(seed(1), 4u8), (seed(2), 7), (seed(3), 12)] {
            let running = committee(n);
            assert_eq!(
                draw_spread(&pool, &s, ops, &running, |_| None),
                draw_v3(&pool, &s, ops, &running),
                "no data: the ticket decides, as before"
            );
        }
    }

    #[test]
    fn reserve_keys_seat_themselves_while_the_night_is_risky() {
        let r = reserve();
        let ops = |k: &str| Some(format!("op-{k}"));
        let members: Vec<(String, String)> = (0..6).map(|i| (format!("c{i}"), format!("n{i}"))).collect();
        let sleeper = |k: &str| k.ends_with('4') || k.ends_with('5');
        let mut avail = std::collections::HashMap::new();
        // Four always-on seats and two at 0.5: P(≥5 of 6) = 0.75 < 0.99 — the
        // reserve keys' business now, and one key is exactly enough (with it
        // the four always-on seats alone carry the quorum of seven).
        let at = |p: u64| [p; DAY_EPOCHS as usize];
        for (k, _) in &members {
            avail.insert(k.clone(), at(if sleeper(k) { pct(1, 2) } else { SCALE }));
        }
        let hours = |k: &str| avail.get(k).copied();
        let running = Committee { members: members.clone() };
        let next = with_reserve(None, &members, &seed(8), ops, &r, &running, hours).expect("a key seats itself");
        assert_eq!(next.len(), 7);
        assert_eq!(next.iter().filter(|(k, _)| k.starts_with('r')).count(), 1, "only as many keys as the odds need");
        let seated = Committee { members: next.clone() };
        drop(hours);
        // The hysteresis band (0.99 ≤ P = 2p−p² < 0.995 at p = 0.905): the
        // seating stays exactly as it is, keys seated or not.
        for (k, p) in avail.iter_mut() {
            if sleeper(k) {
                *p = at(pct(905, 1000));
            }
        }
        let band = |k: &str| avail.get(k).copied();
        assert!(with_reserve(None, &members, &seed(8), ops, &r, &seated, band).is_none(), "seated stays seated in the band");
        assert!(with_reserve(None, &members, &seed(8), ops, &r, &running, band).is_none(), "bare stays bare in the band");
        // Comfortable odds again: the key steps down.
        for (k, p) in avail.iter_mut() {
            if sleeper(k) {
                *p = at(pct(99, 100));
            }
        }
        let after = with_reserve(None, &members, &seed(8), ops, &r, &seated, |k: &str| avail.get(k).copied()).expect("steps down");
        assert!(after.iter().all(|(k, _)| !k.starts_with('r')));
        assert_eq!(after.len(), 6);
        // Five of six seats dark half the day: no number of keys on one Mac
        // saves that quorum, so none seats itself.
        let mut dark = std::collections::HashMap::new();
        for (i, (k, _)) in members.iter().enumerate() {
            dark.insert(k.clone(), at(if i == 0 { SCALE } else { 0 }));
        }
        assert!(with_reserve(None, &members, &seed(8), ops, &r, &running, |k: &str| dark.get(k).copied()).is_none());
    }

    #[test]
    fn the_band_keeps_the_keys_a_draw_dropped() {
        // The 2026-09-29 finding: a normal draw orders non-candidate members
        // out first, so its result can carry none of the reserve keys even
        // while they hold a seat. Reading the seating from the draw then made
        // the hysteresis band "keep zero keys" — the seat quietly vanished
        // while the risk had never left the band. The band must keep what the
        // running committee seats.
        let r = reserve();
        let ops = |k: &str| Some(format!("op-{k}"));
        let members: Vec<(String, String)> = (0..5).map(|i| (format!("c{i}"), format!("n{i}"))).collect();
        let m1 = mac(1);
        let at = |p: u64| [p; DAY_EPOCHS as usize];
        // c0..c2 and the incoming m1 always on, c3 and c4 at 0.905: the drawn
        // six need ≥ 5 up, so P = 2p − p² = 0.990975 — inside the band.
        let mut avail = std::collections::HashMap::new();
        for (i, (k, _)) in members.iter().enumerate() {
            avail.insert(k.clone(), at(if i >= 3 { pct(905, 1000) } else { SCALE }));
        }
        avail.insert(m1.0.clone(), at(SCALE));
        let hours = |k: &str| avail.get(k).copied();
        let mut running = members.clone();
        running.push(r.members[0].clone()); // the key holds a seat
        let running = Committee { members: running };
        let mut drawn = members.clone();
        drawn.push(m1.clone()); // the draw replaced the key with a candidate
        // Four independents in the pool, so the seats are the Macs' own to hold.
        let pool: Vec<(String, String)> = (1..=4).map(mac).collect();
        let next = with_reserve(Some(drawn.clone()), &pool, &seed(8), ops, &r, &running, hours).expect("the band keeps the seat");
        assert!(next.contains(&r.members[0]), "the key the committee seats stays through the draw");
        assert!(next.contains(&m1) && next.len() == drawn.len() + 1);
        // Comfortable odds again: the draw's set stands without the key.
        for p in avail.values_mut() {
            if p[0] == pct(905, 1000) {
                *p = at(pct(99, 100));
            }
        }
        let next = with_reserve(Some(drawn), &pool, &seed(8), ops, &r, &running, |k: &str| avail.get(k).copied()).unwrap();
        assert!(next.iter().all(|(k, _)| !k.starts_with('r')), "past LEAVE_ABOVE the key steps down");
    }

    #[test]
    fn silent_members_are_replaced_within_a_third_minus_one() {
        let hours = |_: &str| -> Option<Hours> { Some([SCALE; DAY_EPOCHS as usize]) };
        let ops = |k: &str| k.starts_with('m').then(|| format!("op-{k}"));
        let pool: Vec<_> = (1..=2u8).map(mac).collect();
        let epoch = 30u64;
        let mut record = std::collections::HashMap::new();
        record.insert("g0".to_string(), (epoch - 1, 0, 0)); // silent two epochs
        record.insert("g1".to_string(), (epoch - 1, 1, 0)); // one slot, then none: silent both
        record.insert("g2".to_string(), (epoch - 1, 0, 4)); // silent now, fine before: stays
        record.insert("g3".to_string(), (epoch - 1, 0, beacons::NO_COUNT)); // unknown before: stays
        let recent = |k: &str| record.get(k).copied();
        // Seven seats: 7/3 − 1 = 1 swap per epoch, and the committee keeps its size.
        let next = replace_silent(&committee(7), &pool, &seed(3), ops, hours, recent, epoch).expect("one swap");
        assert_eq!(next.len(), 7);
        assert_eq!(next.iter().filter(|(k, _)| k == "g0" || k == "g1").count(), 1, "only one of the two silent members goes");
        assert!(next.iter().any(|(k, _)| k == "m1" || k == "m2"), "a candidate takes the seat");
        assert!(next.iter().any(|(k, _)| k == "g2") && next.iter().any(|(k, _)| k == "g3"), "one silent epoch, or an unknown one, keeps the seat");
        // A stale record (an epoch skipped) judges nobody.
        let stale = |k: &str| (k == "g0").then_some((epoch - 2, 0, 0));
        assert!(replace_silent(&committee(7), &pool, &seed(3), ops, hours, stale, epoch).is_none());
        // Committees under six seats replace nobody: the old quorum signs the handoff.
        assert!(replace_silent(&committee(4), &pool, &seed(3), ops, hours, recent, epoch).is_none());
        assert!(replace_silent(&committee(5), &pool, &seed(3), ops, hours, recent, epoch).is_none());
        // No silent members, no record: nothing happens.
        assert!(replace_silent(&committee(7), &pool, &seed(3), ops, hours, |_: &str| None, epoch).is_none());
        // Twelve seats, three silent, two candidates: two are replaced and
        // the third silent member keeps its seat — replacement never shrinks.
        let mut many = std::collections::HashMap::new();
        for k in ["g0", "g1", "g2"] {
            many.insert(k.to_string(), (epoch - 1, 0, 0));
        }
        let next = replace_silent(&committee(12), &pool, &seed(3), ops, hours, |k: &str| many.get(k).copied(), epoch).expect("replaces");
        assert_eq!(next.len(), 12);
        assert_eq!(next.iter().filter(|(k, _)| ["g0", "g1", "g2"].contains(&k.as_str())).count(), 1);
        assert!(next.iter().any(|(k, _)| k == "m1") && next.iter().any(|(k, _)| k == "m2"));
    }
}
