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
use commonware_codec::Encode as _;

/// One item of the list: the rule, and whether the genesis has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub name: &'static str,
    pub ok: bool,
    /// What was checked (on a failing item: what is off).
    pub detail: String,
}

/// The running testnet's chain id: a new genesis needs a new id, and the
/// strict gate refuses it (a rehearsal may run either reserved id).
pub const TESTNET_CHAIN_ID: u64 = 7_780;
/// `scripts/mainnet-rehearsal.sh`'s default chain id: rehearsal-only, refused
/// by the strict gate so a rehearsal file cannot be launched by mistake.
pub const REHEARSAL_CHAIN_ID: u64 = 7_799;

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
        {
            // Audit 5, A5-4: the testnet's id and the rehearsal's id are not a
            // new mainnet chain id, however correct the rest of the file is.
            let reserved = matches!(cfg.chain_id, TESTNET_CHAIN_ID | REHEARSAL_CHAIN_ID);
            rule(
                "chain id",
                !reserved || rehearsal,
                if !reserved {
                    format!("chain id {} is a new id (not the testnet {TESTNET_CHAIN_ID}, not the rehearsal {REHEARSAL_CHAIN_ID})", cfg.chain_id)
                } else if rehearsal {
                    format!("REHEARSAL chain id {} is a reserved id (testnet {TESTNET_CHAIN_ID}, rehearsal {REHEARSAL_CHAIN_ID}); allowed only for a rehearsal", cfg.chain_id)
                } else {
                    format!("chain id {} is a reserved id (testnet {TESTNET_CHAIN_ID}, rehearsal {REHEARSAL_CHAIN_ID}): the mainnet launch needs its own id", cfg.chain_id)
                },
            )
        },
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

/// The final-file gate (audit 5, A5-4): the four rules that only a network
/// the genesis DKG actually wrote can satisfy. `check` rebuilds a genesis and
/// checks its flags; these decode the exact `output` string with the same
/// decoder a node command uses, seat the roster, pin `identity` to the group
/// public key and refuse revealed seated shares. A pre-DKG file (identity and
/// output both absent, what `assemble` writes) passes with a pre-DKG detail —
/// the assemble-time gate is `check`; a file carrying only one of the two
/// fields fails. `mainnet-rules` prints these after the 18 genesis rules.
pub fn check_final(file: &crate::roster::NetworkFile, rehearsal: bool) -> Vec<Rule> {
    let rule = |name: &'static str, ok: bool, detail: String| Rule { name, ok, detail };
    let n = file.validators.len() as u32;
    let identity = file.identity.as_deref().filter(|s| !s.is_empty());
    let output = file.output.as_deref().filter(|s| !s.is_empty());
    let both = match (identity, output) {
        (None, None) => {
            let pre = "pre-DKG file: assemble wrote this; the DKG writes the committee fields".to_string();
            return [
                ("committee output decodes", format!("{pre} — this gate runs on the network.json the DKG wrote")),
                ("output seats the genesis roster", pre.clone()),
                ("identity is the group public key", pre.clone()),
                ("no revealed seated share", pre),
            ]
            .into_iter().map(|(name, detail)| rule(name, true, detail)).collect();
        }
        (Some(_), None) | (None, Some(_)) => {
            let half = "identity and output appear together after the DKG; one alone is a broken file".to_string();
            return [
                ("committee output decodes", format!("half a final file ({half})")),
                ("output seats the genesis roster", half.clone()),
                ("identity is the group public key", half.clone()),
                ("no revealed seated share", half),
            ]
            .into_iter().map(|(name, detail)| rule(name, false, detail)).collect();
        }
        (Some(i), Some(o)) => (i, o),
    };
    // Decode exactly as the node does (Cmd::UpgradeCombine,
    // committee_keys): hex first, then the codec with this roster's n.
    let n_err = (n > 0).then_some(n).ok_or_else(|| "no validators".to_string());
    let decoded = match n_err.and_then(|n| {
        crate::dkg::KeyFile { round: file.round, output: both.1.to_string(), identity: String::new(), share: String::new() }
            .decode_output(n)
            .map_err(|e| e.to_string())
    }) {
        Ok(d) => Some(d),
        Err(_) => None,
    };
    let mut rules = vec![rule(
        "committee output decodes",
        decoded.is_some(),
        match (&decoded, hex::decode(both.1)) {
            (Some(d), _) => format!("the output decodes for {n} players with the same decoder the node's startup uses"),
            (None, Ok(_)) => format!("the output is valid hex but does not decode as a DKG output for {n} players"),
            (None, Err(_)) => "the output is not even hex (try the network.json the DKG wrote, not a hand-edited one)".to_string(),
        },
    )];
    let roster = crate::roster::Roster::from_file(file).ok().map(|r| r.validators());
    let seated = |rules: &mut Vec<Rule>, ok: bool, detail: String| rules.push(rule("output seats the genesis roster", ok, detail));
    let identity_rule = |rules: &mut Vec<Rule>, ok: bool, detail: String| rules.push(rule("identity is the group public key", ok, detail));
    let revealed_rule = |rules: &mut Vec<Rule>, ok: bool, detail: String| rules.push(rule("no revealed seated share", ok, detail));
    match (&decoded, &roster) {
        (Some(d), Some(vs)) => {
            let seats = d.players() == vs;
            seated(&mut rules, seats, if seats {
                format!("the output seats exactly the {} genesis validators (same keys, same count)", vs.len())
            } else {
                format!("the output seats {} players but network.json names {} validators: a DKG over a different set", d.players().len(), vs.len())
            });
            let group = hex::encode(d.public().public().encode());
            let pins = group.eq_ignore_ascii_case(both.0);
            identity_rule(&mut rules, pins, if pins {
                format!("identity {}… is the group public key the output carries", &both.0[..both.0.len().min(16)])
            } else {
                format!("identity {}… is not the group public key of this output ({}…): wallets would pin a key no committee can sign with", &both.0[..both.0.len().min(16)], &group[..group.len().min(16)])
            });
            // The A4-1 check, shared with startup. The legacy testnet keeps its
            // own checks (its files predate strict agreement).
            let legacy = file.chain_id == TESTNET_CHAIN_ID;
            let reveals = !legacy && crate::dkg::KeyFile::reveals_seated_share(d, vs);
            revealed_rule(&mut rules, !reveals || legacy, if legacy {
                format!("chain {TESTNET_CHAIN_ID} keeps its legacy DKG checks")
            } else if reveals {
                "a seated player's threshold share is revealed in the output: this committee key is public".to_string()
            } else {
                "no seated player's share is revealed in the output".to_string()
            });
        }
        _ => {
            let why = if decoded.is_none() { "the output does not decode" } else { "the validator roster does not parse" };
            seated(&mut rules, false, format!("cannot check: {why}"));
            identity_rule(&mut rules, false, format!("cannot check: {why}"));
            revealed_rule(&mut rules, false, format!("cannot check: {why}"));
        }
    }
    let _ = rehearsal; // the final gate has no rehearsal allowance: a DKG output either is usable or is not
    rules
}

/// Audit 5, A5-4's validator-side half: this Mac's `threshold.json` must
/// carry the round, output and identity the final network.json has, or the
/// node must refuse to start voting. `Err` explains the mismatch.
pub fn local_share_matches_network(key: &crate::dkg::KeyFile, file: &crate::roster::NetworkFile) -> Result<(), String> {
    let output = file.output.as_deref().filter(|s| !s.is_empty()).ok_or("network.json has no committee output: this is not a final file")?;
    let identity = file.identity.as_deref().filter(|s| !s.is_empty()).ok_or("network.json has no committee identity: this is not a final file")?;
    if key.round != file.round {
        return Err(format!("threshold.json is from key round {} but network.json is round {}: use the network.json written by the matching dkg/reshare", key.round, file.round));
    }
    if !key.output.eq_ignore_ascii_case(output) {
        return Err("threshold.json holds a different committee output than network.json (the public polynomial differs): vote only under the committee the final file names".to_string());
    }
    if !key.identity.eq_ignore_ascii_case(identity) {
        return Err("threshold.json holds a different committee identity than network.json: vote only under the committee the final file names".to_string());
    }
    Ok(())
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
    const NAMES: [&str; 18] = [
        "chain id",
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
        // Audit 5, A5-4: the testnet's id and the rehearsal's id are not a new
        // mainnet chain id, and only a rehearsal may reuse them.
        let mut rehearsal_id = mainnet();
        rehearsal_id.chain_id = 7_799;
        assert_eq!(off(rehearsal_id.clone()), ["chain id"]);
        assert!(check_with(&rehearsal_id, true).iter().all(|r| r.ok));
        let mut testnet_id = mainnet();
        testnet_id.chain_id = 7_780;
        assert_eq!(off(testnet_id), ["chain id"]);
    }

    /// A final network.json in the mainnet shape (four validators, a real
    /// registrar, the published policy), with the committee fields `run_dkg`
    /// writes: `output`, `identity`, `round`.
    fn final_file(round: u64, output: Option<String>, identity: Option<String>) -> crate::roster::NetworkFile {
        use commonware_cryptography::Signer as _;
        let (rx, ry) = real_registrar();
        let mut registrar = [0u8; 64];
        registrar[..32].copy_from_slice(&rx);
        registrar[32..].copy_from_slice(&ry);
        crate::roster::NetworkFile {
            chain_id: 7_801,
            validators: (1..=4u64)
                .map(|i| {
                    let k = aether_light::devnet_validator_key(i);
                    crate::roster::Member {
                        key: hex::encode(k.public_key().as_ref()),
                        node: aether_net::devnet_node_id(i).to_string(),
                    }
                })
                .collect(),
            identity,
            round,
            output,
            epochs: vec![],
            faucet: None,
            registrar: Some(hex::encode(registrar)),
            epoch_blocks: None,
            min_streak: None,
            draw_epochs: None,
            history: Some(2),
            protocol: Some(3),
            node_rewards: Some(true),
            reserve: Some(crate::roster::ReserveFile {
                operator: Address::repeat_byte(0x99),
                validators: (1..=3)
                    .map(|i| crate::roster::Member {
                        key: hex::encode([i; 32]),
                        node: aether_net::SecretKey::from_bytes(&[i; 32]).public().to_string(),
                    })
                    .collect(),
            }),
            group: None,
            max_committee: Some(crate::rotation::GROW_UNTIL as u64),
            genesis_validators: None,
        }
    }

    /// A real DKG output from an in-memory four-player ceremony (the harness
    /// pattern of tests/dkg.rs, run over a perfect network): what `run_dkg`
    /// hex-encodes into network.json is exactly this pair.
    fn ceremony_output() -> (String, String) {
        use crate::block::PublicKey;
        use crate::dkg::{Ceremony, KeyFile, Round, To};
        use commonware_cryptography::Signer as _;
        use commonware_utils::TryCollect;
        use rand::SeedableRng as _;
        use rand_chacha::ChaCha20Rng;
        use std::collections::VecDeque;
        let players: Vec<_> = (1..=4u64).map(aether_light::devnet_validator_key).collect();
        let pks: Vec<PublicKey> = players.iter().map(|k| k.public_key()).collect();
        let participants: commonware_utils::ordered::Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
        let mut queue: VecDeque<(PublicKey, PublicKey, crate::dkg::Msg)> = VecDeque::new();
        let mut cs = Vec::new();
        for (i, k) in players.iter().enumerate() {
            let (c, out) = Ceremony::start(ChaCha20Rng::seed_from_u64(11), k.clone(), Round::dkg(participants.clone(), 0), None).unwrap();
            for (to, msg) in out {
                let targets: Vec<PublicKey> = match to {
                    To::One(p) => vec![p],
                    To::All => pks.iter().filter(|p| *p != &pks[i]).cloned().collect(),
                };
                for t in targets {
                    queue.push_back((pks[i].clone(), t, msg.clone()));
                }
            }
            cs.push(c);
        }
        let idx = |p: &PublicKey| pks.iter().position(|x| x == p).unwrap();
        let mut output = None;
        for tick in 0..400 {
            for _ in 0..queue.len() {
                let Some((from, to, msg)) = queue.pop_front() else { break };
                let i = idx(&to);
                for (to, msg) in cs[i].on_message(&from, msg) {
                    let targets: Vec<PublicKey> = match to {
                        To::One(p) => vec![p],
                        To::All => pks.iter().filter(|p| *p != &pks[i]).cloned().collect(),
                    };
                    for t in targets {
                        queue.push_back((pks[i].clone(), t, msg.clone()));
                    }
                }
            }
            for i in 0..pks.len() {
                let mut out = cs[i].pending_deals();
                if cs[i].all_acked() || tick > 20 {
                    out.extend(cs[i].close_dealing());
                }
                if tick > 60 && cs[i].have_quorum_logs() {
                    if let Some((_, msg)) = cs[i].propose_transcript() {
                        out.push((To::All, msg.clone()));
                    }
                    out.extend(cs[i].tick_agreement());
                }
                if output.is_none() {
                    if let Some(digest) = cs[i].certified_transcript() {
                        let (o, s) = cs[i].finish_decided(&mut ChaCha20Rng::seed_from_u64(7), &digest).unwrap();
                        output = Some(KeyFile::new(0, &o, &s));
                    }
                }
                out.extend(cs[i].rebroadcast());
                for (to, msg) in out {
                    let targets: Vec<PublicKey> = match to {
                        To::One(p) => vec![p],
                        To::All => pks.iter().filter(|p| *p != &pks[i]).cloned().collect(),
                    };
                    for t in targets {
                        queue.push_back((pks[i].clone(), t, msg.clone()));
                    }
                }
            }
            if output.is_some() && cs.iter().all(|c| c.certified_transcript().is_some()) {
                break;
            }
        }
        let f = output.expect("ceremony completes");
        (f.output, f.identity)
    }

    /// Audit 5, A5-4: the final file gate decodes the exact output, seats the
    /// roster, pins the identity to the group public key and refuses revealed
    /// seated shares — a file with a valid structure but junk committee fields
    /// must FAIL, a real ceremony output must PASS.
    #[test]
    fn the_final_gate_refuses_junk_committee_fields() {
        let file = final_file(0, Some("bb".into()), Some("aa".into()));
        let rules = check_final(&file, false);
        assert_eq!(rules.iter().map(|r| r.name).collect::<Vec<_>>(), FINAL_NAMES);
        assert!(!rules.iter().any(|r| r.ok), "no rule may pass on an undecodable output");
        let undecodable = rules.iter().find(|r| r.name == "committee output decodes").unwrap();
        assert!(undecodable.detail.contains("decode"), "{}", undecodable.detail);
    }

    #[test]
    fn the_final_gate_accepts_a_real_ceremony_output() {
        let (output, identity) = ceremony_output();
        let file = final_file(0, Some(output), Some(identity));
        let rules = check_final(&file, false);
        assert!(rules.iter().all(|r| r.ok), "{}", missing(&rules));
        // The identity rule really compares (not vacuously ok): junk fails.
        let junk_id = final_file(0, file.output.clone(), Some("aa".into()));
        assert!(!check_final(&junk_id, false).iter().all(|r| r.ok));
    }

    #[test]
    fn the_final_gate_seats_exactly_the_genesis_roster() {
        let (output, identity) = ceremony_output();
        let mut file = final_file(0, Some(output), Some(identity));
        file.validators.pop(); // a roster the output does not seat
        let rules = check_final(&file, false);
        assert!(rules.iter().any(|r| !r.ok && r.name == "output seats the genesis roster"));
        // And a file whose round the output does not speak for fails the same gate
        // when the round is baked into the decode (round 0 vs round 9).
        let (output, identity) = ceremony_output();
        let file = final_file(9, Some(output), Some(identity));
        assert!(check_final(&file, false).iter().all(|r| r.ok), "round is bookkeeping, not part of the output bytes");
    }

    #[test]
    fn the_final_gate_tolerates_a_pre_dkg_file_and_refuses_half_of_one() {
        let pre = final_file(0, None, None);
        let rules = check_final(&pre, false);
        assert!(rules.iter().all(|r| r.ok), "{}", missing(&rules));
        assert!(rules.iter().all(|r| r.detail.contains("pre-DKG")), "the ok says why: no output yet");
        let half = final_file(0, Some("bb".into()), None);
        assert!(!check_final(&half, false).iter().all(|r| r.ok), "an output without an identity is half a final file");
    }

    /// The names `check_final` returns, in order.
    const FINAL_NAMES: [&str; 4] = [
        "committee output decodes",
        "output seats the genesis roster",
        "identity is the group public key",
        "no revealed seated share",
    ];

    /// Audit 5, A5-4: a validator votes only with the share the final file
    /// carries — round, output and identity must be the ones network.json has.
    #[test]
    fn a_local_share_must_match_the_final_network_file() {
        use crate::dkg::KeyFile;
        let (output, identity) = ceremony_output();
        let file = final_file(3, Some(output.clone()), Some(identity.clone()));
        let kf = |round, out: String, id: String| KeyFile { round, output: out, identity: id, share: String::new() };
        assert_eq!(local_share_matches_network(&kf(3, output.clone(), identity.clone()), &file), Ok(()));
        assert!(local_share_matches_network(&kf(2, output.clone(), identity.clone()), &file).is_err());
        assert!(local_share_matches_network(&kf(3, "bb".into(), identity.clone()), &file).is_err());
        assert!(local_share_matches_network(&kf(3, output.clone(), "aa".into()), &file).is_err());
        // A network file with no committee output cannot be voted under either.
        let pre = final_file(0, None, None);
        assert!(local_share_matches_network(&kf(0, output, identity), &pre).is_err());
    }
}
