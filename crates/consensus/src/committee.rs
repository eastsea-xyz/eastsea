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
