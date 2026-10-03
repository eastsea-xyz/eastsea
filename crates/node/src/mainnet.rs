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
    check_with(cfg, false)
}

/// `check`, with the one allowance a rehearsal needs: shortened epochs and
/// candidate timing (a rehearsal must finish in minutes). The allowance is
/// reported in the rule's detail, never silent; the real launch is `check`.
pub fn check_with(cfg: &ChainConfig, rehearsal: bool) -> Vec<Rule> {
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
            "registry v3",
            state.code(&registry::REGISTRY) == aether_rewards::registry_v3::code(),
            "the voting-node registry starts as the v3 code".into(),
        ),
        rule(
            "registrar key",
            cfg.registrar.is_some_and(|(x, y)| aether_crypto::p256_point_is_valid(&x, &y)),
            "the registrar is a real, nonzero P-256 key on the curve (a zero or made-up key would silently disable registration)".into(),
        ),
        rule(
            "registration cap",
            registry::max_per_epoch(state) == registry::MAX_PER_EPOCH,
            format!("at most {} new candidates an epoch, on chain", registry::MAX_PER_EPOCH),
        ),
        rule(
            "16-seat growth",
            next >= 3 && cfg.max_committee == crate::rotation::GROW_UNTIL,
            format!(
                "voting-set draws grow to {} seats (protocol 3); this genesis caps the committee at {}",
                crate::rotation::GROW_UNTIL,
                cfg.max_committee
            ),
        ),
        rule(
            "epoch parameters",
            cfg.epoch_blocks <= crate::roster::MAX_EPOCH_BLOCKS && cfg.draw_epochs.unwrap_or(0) <= crate::roster::MAX_DRAW_EPOCHS,
            format!(
                "blocks per epoch at most {} and epochs per draw at most {} (no overflow, no zero divisor)",
                crate::roster::MAX_EPOCH_BLOCKS,
                crate::roster::MAX_DRAW_EPOCHS
            ),
        ),
        {
            let params = registry::params(state);
            let (epoch, streak, draw) = (params.epoch_blocks, params.min_streak, params.draw_epochs);
            let policy = (registry::EPOCH_BLOCKS, registry::MIN_STREAK, registry::DRAW_EPOCHS);
            let matches = (epoch, streak, draw) == policy;
            rule(
                "candidate timing",
                matches || rehearsal,
                if matches {
                    format!("{epoch} blocks an epoch, {streak} epochs of warm-up, a draw every {draw} epochs: the published policy")
                } else if rehearsal {
                    format!("REHEARSAL VALUES ({epoch}, {streak}, {draw}) differ from the published policy {policy:?}; allowed only for a rehearsal")
                } else {
                    format!("({epoch} blocks, {streak} warm-up epochs, draw every {draw}) differs from the published policy {policy:?}")
                },
            )
        },
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
    /// A real P-256 registrar key (the old fixture's made-up pair is off the curve).
    fn real_registrar() -> ([u8; 32], [u8; 32]) {
        use aether_crypto::Signer;
        let key = aether_crypto::P256Signer::from_seed(&[5; 32]).unwrap().public_key();
        aether_crypto::p256_xy(&key.bytes).unwrap()
    }

    fn mainnet() -> ChainConfig {
        ChainConfig {
            chain_id: 7_801,
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: vec![],
            fees: true,
            registrar: Some(real_registrar()),
            epoch_blocks: 0,
            min_streak: None,
            draw_epochs: None,
            history_v2: true,
            protocol: upgrade::PROTOCOL,
            node_rewards: true,
            // No committee word: the checklist reads rules, not rosters, and
            // the real genesis committee is a ceremony output, not a flag.
            committee: vec![],
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
            group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
        }
    }

    /// The names `check` returns, in order: docs/ops/mainnet-launch.md's table.
    const NAMES: [&str; 17] = [
        "protocol from genesis",
        "proof market",
        "registry v3",
        "registrar key",
        "registration cap",
        "16-seat growth",
        "epoch parameters",
        "candidate timing",
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
            ["protocol from genesis", "proof market", "registration cap", "16-seat growth"]
        );
        // Audit 1, A5: a four-seat committee cap is not "16-seat growth".
        let mut capped = mainnet();
        capped.max_committee = 4;
        assert_eq!(off(capped), ["16-seat growth"]);
        // Audit 1, A2: epoch parameters outside the bounds fail their own rule
        // (the same config used to pass every rule and then crash on resume).
        let mut huge = mainnet();
        huge.epoch_blocks = 1 << 63;
        huge.draw_epochs = Some(2);
        assert_eq!(off(huge), ["epoch parameters", "candidate timing"]);
        // Audit 2, R2-3: a zero or made-up registrar key disables registration.
        let mut dead = mainnet();
        dead.registrar = Some(([0; 32], [0; 32]));
        assert_eq!(off(dead), ["registrar key"]);
        // Audit 2, R2-3: timing other than the published policy fails the strict
        // check and is allowed (and reported) only for a rehearsal.
        let mut quick = mainnet();
        quick.min_streak = Some(0);
        quick.draw_epochs = Some(1);
        quick.epoch_blocks = 40;
        assert_eq!(off(quick.clone()), ["candidate timing"]);
        let rehearsal = check_with(&quick, true);
        assert!(rehearsal.iter().all(|r| r.ok), "{}", missing(&rehearsal));
        assert!(rehearsal.iter().any(|r| r.name == "candidate timing" && r.detail.starts_with("REHEARSAL VALUES")));
        // A premine (or a faucet) funds genesis accounts.
        let mut premine = mainnet();
        premine.alloc = vec![(Address::repeat_byte(1), U256::from(1u8))];
        assert_eq!(off(premine), ["no premine, no faucet"]);
        // The v3 registry is installed only by a genesis with node rewards, history v2
        // and a registrar (chain.rs), so it also drops out with either of them.
        // Rewards off takes the epoch machinery with it.
        let mut cold = mainnet();
        cold.node_rewards = false;
        assert_eq!(off(cold), ["registry v3", "node rewards", "beacons", "re-attestation", "reserve rules"]);
        // History v1 keeps every block by default.
        let mut v1 = mainnet();
        v1.history_v2 = false;
        assert_eq!(off(v1), ["registry v3", "history v2", "pruning default"]);
        // The reserve keys are a genesis parameter.
        let mut none = mainnet();
        none.reserve = None;
        assert_eq!(off(none), ["reserve rules"]);
    }
}
