//! Epoch committee selection (docs/design/07-consensus.md).
//!
//! Only a small, high-uptime subset of registered validators votes in BFT; the
//! rest verify. Selection is a seeded, uptime-weighted lottery:
//!   ticket_i = H(seed ‖ id_i) as u64,  score_i = ticket_i / weight_i  (lower wins)
//! compared exactly as ticket_i * weight_j < ticket_j * weight_i in u128.
//! Integer-only on purpose: floating-point `ln` differs across libms and would
//! split consensus. The seed is the previous epoch's finalized certificate hash
//! (a BLS threshold signature, so it cannot be ground by a proposer).

use aether_hash::AnyHasher;
use aether_types::ValidatorId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateInfo {
    pub id: ValidatorId,
    /// Uptime reputation in basis points of recent epochs voted (0..=10_000).
    pub uptime_bps: u16,
}

/// Minimum weight so a returning validator can be re-selected eventually.
const MIN_WEIGHT: u64 = 100;

pub fn select_committee(hasher: &AnyHasher, seed: &[u8; 32], candidates: &[CandidateInfo], size: usize) -> Vec<ValidatorId> {
    let mut scored: Vec<(u64, u64, ValidatorId)> = candidates
        .iter()
        .map(|c| {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(seed);
            buf[32..].copy_from_slice(c.id.as_slice());
            let d = hasher.hash_bytes(&buf);
            let ticket = u64::from_be_bytes(d[..8].try_into().expect("8 bytes"));
            let weight = (c.uptime_bps as u64).clamp(MIN_WEIGHT, 10_000);
            (ticket, weight, c.id)
        })
        .collect();
    scored.sort_by(|a, b| {
        let lhs = a.0 as u128 * b.1 as u128;
        let rhs = b.0 as u128 * a.1 as u128;
        lhs.cmp(&rhs).then(a.2.cmp(&b.2))
    });
    scored.dedup_by(|a, b| a.2 == b.2);
    let mut out: Vec<ValidatorId> = scored.into_iter().take(size).map(|(_, _, id)| id).collect();
    out.sort(); // canonical committee order
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_hash::HashId;
    use aether_types::B256;

    fn cands(n: u8, uptime: u16) -> Vec<CandidateInfo> {
        (0..n).map(|i| CandidateInfo { id: B256::repeat_byte(i + 1), uptime_bps: uptime }).collect()
    }

    fn h() -> AnyHasher {
        AnyHasher::new(HashId::Poseidon2KoalaBear16)
    }

    #[test]
    fn deterministic_and_input_order_independent() {
        let c = cands(30, 9_000);
        let mut rev = c.clone();
        rev.reverse();
        let a = select_committee(&h(), &[7; 32], &c, 7);
        assert_eq!(a, select_committee(&h(), &[7; 32], &rev, 7));
        assert_eq!(a.len(), 7);
        assert!(a.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn seed_changes_committee() {
        let c = cands(40, 9_000);
        assert_ne!(select_committee(&h(), &[1; 32], &c, 7), select_committee(&h(), &[2; 32], &c, 7));
    }

    #[test]
    fn small_candidate_set_and_duplicates() {
        let mut c = cands(3, 5_000);
        c.push(c[0].clone());
        assert_eq!(select_committee(&h(), &[0; 32], &c, 7).len(), 3);
    }

    #[test]
    fn higher_uptime_is_selected_more_often() {
        let mut c = cands(20, 1_000);
        for x in c.iter_mut().take(10) {
            x.uptime_bps = 10_000;
        }
        let high: std::collections::BTreeSet<_> = c.iter().take(10).map(|x| x.id).collect();
        let (mut hi, mut total) = (0, 0);
        for s in 0..200u8 {
            let mut seed = [0u8; 32];
            seed[0] = s;
            seed[1] = s.wrapping_mul(31);
            for id in select_committee(&h(), &seed, &c, 5) {
                total += 1;
                hi += high.contains(&id) as u32;
            }
        }
        assert!(hi * 100 / total >= 75, "high-uptime share {hi}/{total}");
    }
}

// ---------------- open committee (07-consensus.md "open committee") ----------------

/// A registered candidate, as read from the CommitteeRegistry contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenCandidate {
    /// Registration order (contract index).
    pub index: u64,
    /// Account that registered it; seats are capped per operator.
    pub operator: [u8; 20],
    pub validator_key: [u8; 32],
    pub registered_epoch: u64,
    pub last_epoch: u64,
    pub streak: u64,
}

/// Smallest committee the open selection builds when enough operators exist.
pub const MIN_OPEN_COMMITTEE: usize = 4;

/// Committee for the epoch after `epoch`: candidates that sent a beacon in
/// `epoch`, by longest streak, then earliest registration. With at least four
/// distinct operators, no operator holds a third or more of the seats (the
/// committee shrinks rather than break the cap); with fewer, the cap cannot be
/// met and the committee is simply the top candidates (bootstrap).
/// Deterministic: every node computes the same list from the same state.
pub fn select_open_committee(candidates: &[OpenCandidate], epoch: u64, max_size: usize) -> Vec<OpenCandidate> {
    let mut live: Vec<&OpenCandidate> = candidates.iter().filter(|c| c.last_epoch == epoch).collect();
    live.sort_by(|a, b| b.streak.cmp(&a.streak).then(a.registered_epoch.cmp(&b.registered_epoch)).then(a.index.cmp(&b.index)));
    let operators: std::collections::BTreeSet<[u8; 20]> = live.iter().map(|c| c.operator).collect();
    let target = live.len().min(max_size);
    if operators.len() < MIN_OPEN_COMMITTEE {
        return live.into_iter().take(target).cloned().collect();
    }
    if target < MIN_OPEN_COMMITTEE {
        // Too few seats for the cap to bind: one seat per operator, by rank.
        let mut seen = std::collections::BTreeSet::new();
        return live.into_iter().filter(|c| seen.insert(c.operator)).take(target).cloned().collect();
    }
    for n in (MIN_OPEN_COMMITTEE..=target).rev() {
        let cap = (n - 1) / 3; // strictly below n / 3
        let mut seats: std::collections::BTreeMap<[u8; 20], usize> = Default::default();
        let mut picked = Vec::with_capacity(n);
        for c in &live {
            let s = seats.entry(c.operator).or_default();
            if *s < cap {
                *s += 1;
                picked.push((*c).clone());
                if picked.len() == n {
                    return picked;
                }
            }
        }
    }
    // Four operators exist, so n = 4 with one seat each always fills.
    unreachable!("four distinct operators always fill a committee of four")
}

#[cfg(test)]
mod open_tests {
    use super::*;

    fn cand(index: u64, op: u8, streak: u64, last: u64) -> OpenCandidate {
        OpenCandidate { index, operator: [op; 20], validator_key: [index as u8; 32], registered_epoch: index, last_epoch: last, streak }
    }

    fn share(c: &[OpenCandidate], op: u8) -> usize {
        c.iter().filter(|x| x.operator == [op; 20]).count()
    }

    #[test]
    fn no_operator_reaches_a_third_once_four_operators_exist() {
        // The founder (op 1) has 8 long-running Macs; three others have one each.
        let mut cs: Vec<_> = (0..8).map(|i| cand(i, 1, 1_000, 10)).collect();
        cs.extend([cand(8, 2, 5, 10), cand(9, 3, 3, 10), cand(10, 4, 1, 10)]);
        let c = select_open_committee(&cs, 10, 16);
        assert!(share(&c, 1) * 3 < c.len(), "founder holds {} of {}", share(&c, 1), c.len());
        assert_eq!(c.len(), 4, "only as many seats as the cap allows: 1 founder + 3 others");
        // More independent operators grow the committee, the founder still under a third.
        cs.extend((11..20).map(|i| cand(i, i as u8, 2, 10)));
        let c = select_open_committee(&cs, 10, 16);
        assert_eq!(c.len(), 16);
        assert!(share(&c, 1) * 3 < c.len());
    }

    #[test]
    fn only_live_candidates_by_streak_then_seniority() {
        let cs = vec![cand(0, 1, 50, 9), cand(1, 2, 10, 10), cand(2, 3, 30, 10), cand(3, 4, 30, 10), cand(4, 5, 1, 10)];
        let c = select_open_committee(&cs, 10, 3);
        assert_eq!(c.iter().map(|x| x.index).collect::<Vec<_>>(), vec![2, 3, 1], "0 missed the epoch; ties go to earlier registration");
    }

    #[test]
    fn bootstrap_without_enough_operators_takes_the_top() {
        let cs: Vec<_> = (0..4).map(|i| cand(i, 1, 10, 10)).collect();
        assert_eq!(select_open_committee(&cs, 10, 16).len(), 4, "a lone founder still runs the chain until others join");
        assert!(select_open_committee(&cs, 11, 16).is_empty(), "no beacons this epoch, no committee");
    }

    #[test]
    fn deterministic_regardless_of_input_order() {
        let mut cs: Vec<_> = (0..12).map(|i| cand(i, (i % 5) as u8, i * 7 % 11, 10)).collect();
        let a = select_open_committee(&cs, 10, 8);
        cs.reverse();
        assert_eq!(a, select_open_committee(&cs, 10, 8));
    }
}
