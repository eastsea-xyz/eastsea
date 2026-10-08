//! Local work scheduling only: validators still accept the first valid proof.

use aether_types::Address;
use std::time::Duration;

const DOMAIN: &[u8] = b"aether/prover-assignment/v1";
pub const MAX_WINDOW: usize = 4096;

#[derive(Clone, Debug)]
pub struct Config {
    pub designated: usize,
    pub grace: Duration,
    pub window: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            designated: 3,
            grace: Duration::from_secs(60),
            window: 128,
        }
    }
}

impl Config {
    /// Node-local settings; no genesis fields or proof validation rules change.
    pub fn from_env() -> Result<Self, String> {
        fn number(name: &str, default: u64) -> Result<u64, String> {
            match std::env::var(name) {
                Ok(value) => value
                    .parse()
                    .map_err(|_| format!("{name} must be an unsigned integer")),
                Err(std::env::VarError::NotPresent) => Ok(default),
                Err(_) => Err(format!("{name} must be an unsigned integer")),
            }
        }
        let defaults = Self::default();
        let designated = number("AETHER_PROVER_ASSIGNMENT_K", defaults.designated as u64)?;
        let grace = number("AETHER_PROVER_GRACE_SECS", defaults.grace.as_secs())?;
        let window = number("AETHER_PROVER_WINDOW", defaults.window as u64)?;
        if !(1..=MAX_WINDOW as u64).contains(&designated) {
            return Err(format!(
                "AETHER_PROVER_ASSIGNMENT_K must be between 1 and {MAX_WINDOW}"
            ));
        }
        if !(1..=MAX_WINDOW as u64).contains(&window) {
            return Err(format!(
                "AETHER_PROVER_WINDOW must be between 1 and {MAX_WINDOW}"
            ));
        }
        Ok(Self {
            designated: designated as usize,
            grace: Duration::from_secs(grace),
            window: window as usize,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct OpenBlock {
    pub height: u64,
    pub timestamp_ms: u64,
}

fn score(height: u64, operator: Address) -> [u8; 32] {
    *blake3::Hasher::new()
        .update(DOMAIN)
        .update(&height.to_be_bytes())
        .update(operator.as_slice())
        .finalize()
        .as_bytes()
}

fn operators(registered: &[Address]) -> Vec<Address> {
    let mut unique = registered.to_vec();
    unique.sort_unstable();
    unique.dedup();
    unique
}

/// Highest hashes win; the address breaks ties. Multiple Macs of one operator
/// do not give that operator additional assignment tickets.
pub fn designated(height: u64, registered: &[Address], k: usize) -> Vec<Address> {
    let mut ranked: Vec<_> = operators(registered)
        .into_iter()
        .map(|op| (score(height, op), op))
        .collect();
    ranked.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    ranked.into_iter().take(k).map(|(_, op)| op).collect()
}

/// Pick inside the last W open blocks. The caller excludes proven, lost, and
/// already attempted jobs. Designated jobs precede grace rescues; both queues
/// are oldest first. Age comes from the finalized block timestamp, including
/// time spent queued or catching up, rather than local arrival time.
pub fn select(
    blocks: &[OpenBlock],
    registered: &[Address],
    prover: Address,
    now_ms: u64,
    config: &Config,
) -> Option<u64> {
    let registered = operators(registered);
    let my_registration = registered.binary_search(&prover).is_ok();
    let mut blocks: Vec<_> = blocks.iter().collect();
    blocks.sort_unstable_by_key(|b| b.height);
    let window = &blocks[blocks.len().saturating_sub(config.window)..];
    let assigned = |b: &&OpenBlock| {
        if !my_registration || config.designated == 0 {
            return false;
        }
        let mine = (score(b.height, prover), prover);
        let ahead = registered
            .iter()
            .filter(|&&op| {
                let other = (score(b.height, op), op);
                other.0 > mine.0 || (other.0 == mine.0 && other.1 < mine.1)
            })
            .count();
        ahead < config.designated
    };
    window
        .iter()
        .find(|b| assigned(b))
        .or_else(|| {
            window.iter().find(|b| {
                u128::from(now_ms.saturating_sub(b.timestamp_ms)) > config.grace.as_millis()
            })
        })
        .map(|b| b.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignments_are_deterministic_unique_and_have_exactly_k_operators() {
        let ops: Vec<_> = (1..=16).map(Address::repeat_byte).collect();
        let mut reordered = ops.clone();
        reordered.reverse();
        reordered.extend_from_slice(&ops);
        for height in 1..=100 {
            let chosen = designated(height, &ops, 3);
            assert_eq!(chosen.len(), 3);
            assert_eq!(chosen, designated(height, &reordered, 3));
            for op in &ops {
                let job = select(
                    &[OpenBlock {
                        height,
                        timestamp_ms: 1_000,
                    }],
                    &ops,
                    *op,
                    1_000,
                    &Config::default(),
                );
                assert_eq!(job, chosen.contains(op).then_some(height));
            }
        }
        assert_eq!(designated(1, &ops[..2], 3).len(), 2);
        assert!(designated(1, &[], 3).is_empty());
        assert!(designated(1, &ops, 0).is_empty());
    }

    #[test]
    fn grace_is_strict_and_unregistered_provers_only_rescue_after_it() {
        let config = Config::default();
        let op = Address::repeat_byte(1);
        let stranger = Address::repeat_byte(2);
        let blocks = [OpenBlock {
            height: 7,
            timestamp_ms: 1_000,
        }];
        assert_eq!(select(&blocks, &[op], op, 1_000, &config), Some(7));
        assert_eq!(select(&blocks, &[op], stranger, 61_000, &config), None);
        assert_eq!(select(&blocks, &[op], stranger, 61_001, &config), Some(7));
        assert_eq!(select(&blocks, &[], stranger, 61_001, &config), Some(7));
        assert_eq!(select(&blocks, &[], stranger, 0, &config), None);
    }

    #[test]
    fn designated_jobs_precede_oldest_grace_rescues_within_the_window() {
        let ops: Vec<_> = (1..=16).map(Address::repeat_byte).collect();
        let me = ops[0];
        let old = (1..100)
            .find(|&h| !designated(h, &ops, 3).contains(&me))
            .unwrap();
        let assigned = (old + 1..200)
            .find(|&h| designated(h, &ops, 3).contains(&me))
            .unwrap();
        let config = Config {
            window: 2,
            ..Config::default()
        };
        let blocks = [
            OpenBlock {
                height: assigned,
                timestamp_ms: 100_000,
            },
            OpenBlock {
                height: old,
                timestamp_ms: 1_000,
            },
        ];
        assert_eq!(select(&blocks, &ops, me, 100_000, &config), Some(assigned));
        let stranger = Address::repeat_byte(99);
        assert_eq!(select(&blocks, &ops, stranger, 200_000, &config), Some(old));
        let config = Config {
            window: 1,
            ..config
        };
        assert_eq!(
            select(&blocks, &ops, stranger, 200_000, &config),
            Some(assigned)
        );
    }
}
