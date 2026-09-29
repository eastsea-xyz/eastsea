//! The mainnet rule set: every rule the mainnet must have active at height 1,
//! in one list (docs/ops/mainnet-launch.md §2).
//!
//! Gap G1 was this list being implicit: a new genesis started at protocol 1,
//! so the proof market, the per-epoch registration cap and the 16-seat growth
//! were all off until someone noticed. `check` builds the genesis a node would
//! build from a network.json and names each rule on or off, and
//! `aether mainnet-rules`, `scripts/mainnet-rehearsal.sh` and a unit test all
//! assert every item — a rule cannot silently fail to be on again. A new rule
//! goes here, into the doc's table and into the pinned name list in the tests
//! together.

use crate::chain::{Chain, ChainConfig};
use crate::upgrade;
use aether_execution::registry;
use aether_types::{FeeVector, U256};

/// One item of the list: the rule, and whether the genesis has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub name: &'static str,
    pub ok: bool,
    /// What was checked (on a failing item: what is off).
    pub detail: String,
}

/// Every rule the mainnet must have active at height 1, checked against a
/// freshly built genesis. Same order as the table in docs/ops/mainnet-launch.md.
pub fn check(cfg: &ChainConfig) -> Vec<Rule> {
    let (chain, _) = Chain::new(cfg.clone());
    let genesis = chain.lock().finalized.clone();
    let state = &genesis.state;
    let at1 = upgrade::protocol_at(&genesis.schedule, 1);
    let next = genesis.next_protocol();
    let epoch_blocks = registry::params(state).epoch_blocks;
    let rule = |name: &'static str, ok: bool, detail: String| Rule { name, ok, detail };
    vec![
        rule(
            "protocol from genesis",
            at1 == upgrade::PROTOCOL,
            format!("protocol at height 1 is {at1}, this binary runs {}", upgrade::PROTOCOL),
        ),
        rule(
            "proof market",
            next >= 2,
            "blocks record statements from height 1 and proofs pay (protocol 2)".into(),
        ),
        rule(
            "registry v2",
            state.code(&registry::REGISTRY) == registry::code_v2(),
            "the voting-node registry starts as the v2 code".into(),
        ),
        rule(
            "registration cap",
            registry::max_per_epoch(state) == registry::MAX_PER_EPOCH,
            format!("at most {} new candidates an epoch, on chain", registry::MAX_PER_EPOCH),
        ),
        rule(
            "16-seat growth",
            next >= 3,
            format!("voting-set draws grow to {} seats (protocol 3)", crate::rotation::GROW_UNTIL),
        ),
        rule(
            "node rewards",
            aether_rewards::enabled(state),
            "half the issuance to the operators whose Macs beaconed, half to provers, from block 1".into(),
        ),
        rule(
            "beacons",
            aether_rewards::enabled(state) && aether_rewards::beacons::layout(epoch_blocks).is_some(),
            format!("four liveness beacon slots an epoch of {epoch_blocks} blocks"),
        ),
        rule(
            "re-attestation",
            cfg.registrar.is_some() && aether_rewards::enabled(state),
            format!("each Mac re-confirms with DeviceCheck once a day ({} epochs), registrar-signed", aether_rewards::DAY_EPOCHS),
        ),
        rule(
            "reserve rules",
            match (&cfg.reserve, aether_rewards::reserve(state)) {
                (Some(want), Some((operator, keys))) => {
                    *want.operator == *operator && keys.len() == want.members.len()
                }
                _ => false,
            },
            format!(
                "the founder's reserve keys are on chain (up to {}), seated only below four independent operators",
                aether_rewards::MAX_RESERVE_KEYS
            ),
        ),
        smooth_issuance(),
        rule("history v2", cfg.history_v2, "quiet empty blocks, era files, history proofs over them".into()),
        rule(
            "pruning default",
            matches!(
                crate::prune::HistoryMode::resolve(
                    None,
                    cfg.history_v2,
                    crate::prune::DEFAULT_RETAIN_DAYS,
                    1_000,
                    false
                ),
                Ok(crate::prune::HistoryMode::Prune(h)) if h.blocks == 30 * 86_400 && h.keep_era_files
            ),
            "nodes prune by default: 30 days of blocks, era files kept".into(),
        ),
        rule(
            "no premine, no faucet",
            cfg.alloc.is_empty(),
            "every genesis balance is zero: tokens exist only through issuance".into(),
        ),
        rule(
            "zero-tip acceptance",
            Chain::next_base_fee(cfg, &genesis) == FeeVector::default(),
            "the first block's base fee is 0: a zero-balance account transacts with tip 0".into(),
        ),
    ]
}

/// The published issuance schedule: 1 AETH a block at height 0, decaying
/// smoothly (−15% a year, well under a tenth of a percent a day, never a
/// halving step) and floored at 0.1 AETH (docs/design/15-node-rewards.md).
fn smooth_issuance() -> Rule {
    use aether_rewards::{issuance, DAY_BLOCKS, TAIL};
    let full = issuance(0);
    let day = issuance(DAY_BLOCKS);
    let year = issuance(365 * DAY_BLOCKS);
    let ok = full == U256::from(aether_execution::proofs::ISSUE_0)
        && day < full
        && day > full * U256::from(999u32) / U256::from(1_000u32)
        && year <= full * U256::from(85u32) / U256::from(100u32)
        && year > full * U256::from(84u32) / U256::from(100u32)
        && issuance(6_000 * DAY_BLOCKS) == U256::from(TAIL);
    Rule {
        name: "smooth issuance",
        ok,
        detail: format!("1 AETH a block, −15%/year ({year} after a year), floored at 0.1 AETH"),
    }
}

/// The items of `check` that are off, as one message (empty list = all on).
pub fn missing(rules: &[Rule]) -> String {
    let off: Vec<&Rule> = rules.iter().filter(|r| !r.ok).collect();
    match off.len() {
        0 => "every mainnet rule is on".to_string(),
        _ => format!(
            "mainnet rules off: {}",
            off.iter().map(|r| format!("{} ({})", r.name, r.detail)).collect::<Vec<_>>().join("; ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::Reserve;
    use aether_types::{Address, GasVector};

    /// The mainnet flags, as docs/ops/mainnet-launch.md §2 assembles them
    /// (`aether network --protocol 3 --history 2 --node-rewards --registrar …`).
    fn mainnet() -> ChainConfig {
        ChainConfig {
            chain_id: 7_801,
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: vec![],
            fees: true,
            registrar: Some(([7; 32], [8; 32])),
            epoch_blocks: 0,
            min_streak: None,
            draw_epochs: None,
            history_v2: true,
            protocol: upgrade::PROTOCOL,
            node_rewards: true,
            reserve: Some(Reserve {
                operator: Address::repeat_byte(0x99),
                members: (1..=3)
                    .map(|i| {
                        (
                            hex::encode([i; 32]),
                            aether_net::SecretKey::from_bytes(&[i; 32]).public().to_string(),
                        )
                    })
                    .collect(),
            }),
        }
    }

    /// The names `check` returns, in order: docs/ops/mainnet-launch.md's table.
    const NAMES: [&str; 14] = [
        "protocol from genesis",
        "proof market",
        "registry v2",
        "registration cap",
        "16-seat growth",
        "node rewards",
        "beacons",
        "re-attestation",
        "reserve rules",
        "smooth issuance",
        "history v2",
        "pruning default",
        "no premine, no faucet",
        "zero-tip acceptance",
    ];

    #[test]
    fn the_mainnet_flags_turn_every_rule_on() {
        let rules = check(&mainnet());
        assert_eq!(rules.iter().map(|r| r.name).collect::<Vec<_>>(), NAMES);
        assert!(rules.iter().all(|r| r.ok), "{}", missing(&rules));
    }

    #[test]
    fn a_missing_flag_fails_exactly_its_rules() {
        let off = |cfg: ChainConfig| {
            check(&cfg).iter().filter(|r| !r.ok).map(|r| r.name).collect::<Vec<_>>()
        };
        // Gap G1's shape: a protocol-1 genesis opens without the new rules.
        let mut g1 = mainnet();
        g1.protocol = 1;
        assert_eq!(
            off(g1),
            ["protocol from genesis", "proof market", "registry v2", "registration cap", "16-seat growth"]
        );
        // A premine (or a faucet) funds genesis accounts.
        let mut premine = mainnet();
        premine.alloc = vec![(Address::repeat_byte(1), U256::from(1u8))];
        assert_eq!(off(premine), ["no premine, no faucet"]);
        // Rewards off takes the epoch machinery with it.
        let mut cold = mainnet();
        cold.node_rewards = false;
        assert_eq!(off(cold), ["node rewards", "beacons", "re-attestation", "reserve rules"]);
        // History v1 keeps every block by default.
        let mut v1 = mainnet();
        v1.history_v2 = false;
        assert_eq!(off(v1), ["history v2", "pruning default"]);
        // The reserve keys are a genesis parameter.
        let mut none = mainnet();
        none.reserve = None;
        assert_eq!(off(none), ["reserve rules"]);
    }
}
