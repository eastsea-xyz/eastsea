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
use aether_types::{Address, Bytes, FeeVector, U256};
use commonware_codec::Encode as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    let probe = crate::block::Block::genesis_with(cfg.chain_id, state.root(), cfg.history_v2, cfg.group);
    let state_limit = Chain::block_context(cfg, &probe, &genesis).limits.state;
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
            format!("twelve liveness beacon slots an epoch of {epoch_blocks} blocks"),
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
            "receipt commitments",
            cfg.node_rewards || cfg.history_v2,
            "from height 1, each block commits its own execution receipts; validators reexecute and compare the root".into(),
        ),
        rule(
            "paid state growth",
            (cfg.node_rewards || cfg.history_v2)
                && state_limit == aether_execution::fees::MAX_STATE_UNITS_PER_BLOCK
                && Chain::next_base_fee(cfg, &genesis).state == aether_execution::fees::STATE_UNIT_PRICE
                && aether_execution::fees::STATE_ACCOUNT_UNITS > 0
                && aether_execution::fees::RECEIPT_BYTES_PER_STATE_UNIT > 0
                && aether_execution::fees::EVENT_BASE_BYTES > 0
                && economic_disk_bound()
                && paid_growth_probes(cfg, &genesis),
            format!(
                "{} wei per state unit at the floor, rising with congestion; {} units per new slot or account; {} burst units with {} units/block refill; entire encoded payload has an 8 MiB burst and 4 KiB/block refill (combined archive allowance below 3 GB/day including bursts); at most {} new slots per block",
                aether_execution::fees::STATE_UNIT_PRICE,
                aether_execution::fees::STATE_SLOT_UNITS,
                aether_execution::fees::MAX_STATE_UNITS_PER_BLOCK,
                aether_execution::fees::STATE_UNITS_PER_BLOCK,
                aether_execution::fees::MAX_NEW_SLOTS_PER_BLOCK
            ),
        ),
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
            { let base = Chain::next_base_fee(cfg, &genesis); base.exec == 0 && base.prove == 0 },
            "the first block's exec/prove base fees are 0; an already funded account can transact with zero tip".into(),
        ),
    ]
}

/// Launch policy includes a numerical disk envelope, not just a nonzero fee.
/// At most burst + refill * heights units can be consumed in any interval.
fn economic_disk_bound() -> bool {
    use aether_execution::fees;
    let daily_units = fees::MAX_STATE_UNITS_PER_BLOCK as u128
        + 86_400 * fees::STATE_UNITS_PER_BLOCK as u128;
    let daily_stored = daily_units * fees::RECEIPT_BYTES_PER_STATE_UNIT as u128
        * fees::MAX_STORED_BYTES_PER_METERED_BYTE as u128;
    let daily_archive = (fees::MAX_ENCODED_PAYLOAD_BYTES as u128
        + 86_400 * fees::ENCODED_PAYLOAD_BYTES_PER_BLOCK as u128)
        * fees::MAX_ARCHIVE_COPIES as u128;
    fees::STATE_UNITS_PER_BLOCK > 0
        && fees::STATE_UNITS_PER_BLOCK < fees::MAX_STATE_UNITS_PER_BLOCK
        && daily_stored <= 1_500_000_000
        && fees::ENCODED_PAYLOAD_BYTES_PER_BLOCK >= 1024
        && fees::ENCODED_PAYLOAD_BYTES_PER_BLOCK < fees::MAX_ENCODED_PAYLOAD_BYTES
        && fees::MAX_ARCHIVE_COPIES >= 4
        && fees::CONTROL_ARCHIVE_RESERVE < fees::MAX_ENCODED_PAYLOAD_BYTES
        && daily_stored + daily_archive <= 3_000_000_000
        && fees::encoded_payload_limit(fees::MAX_ENCODED_PAYLOAD_BYTES
            - fees::ENCODED_PAYLOAD_BYTES_PER_BLOCK) == fees::ENCODED_PAYLOAD_BYTES_PER_BLOCK
        && fees::state_block_limit(fees::MAX_STATE_UNITS_PER_BLOCK - fees::STATE_UNITS_PER_BLOCK)
            == fees::STATE_UNITS_PER_BLOCK
        && fees::state_base_fee(fees::STATE_PRICE_FREE_BURST) == fees::STATE_UNIT_PRICE
        && fees::state_base_fee(fees::STATE_PRICE_FREE_BURST + 1) > fees::STATE_UNIT_PRICE
}

/// Exercise the two audit-6 escape routes against the genesis executor. The
/// checklist checks actual admission and receipts, so keeping the finite state
/// limit while accidentally exempting senders or events cannot pass this gate.
fn paid_growth_probes(cfg: &ChainConfig, genesis: &crate::chain::Executed) -> bool {
    use aether_crypto::{P256Signer, Signer as _};
    use aether_execution::{check_admission, execute_block, sign_call_with, EvmCall};

    let probe = crate::block::Block::genesis_with(cfg.chain_id, genesis.state.root(), cfg.history_v2, cfg.group);
    let ctx = Chain::block_context(cfg, &probe, genesis);
    let sender = match P256Signer::from_seed(&[0xa6; 32]) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let plain = EvmCall { to: Some(Address::repeat_byte(0x42)), value: U256::ZERO, input: Bytes::new(), gas_limit: 21_000, delegate: None };
    let fresh = match sign_call_with(&sender, cfg.chain_id, 0, FeeVector::default(), 0, &plain) {
        Ok(tx) => tx,
        Err(_) => return false,
    };
    if check_admission(&genesis.state, &ctx, &fresh).is_ok()
        || execute_block(&genesis.state, &ctx, &[fresh]).is_ok()
    {
        return false;
    }

    let sender_address = match aether_crypto::address_of(&sender.public_key()) {
        Ok(a) => a,
        Err(_) => return false,
    };
    let contract = Address::repeat_byte(0x43);
    let mut state = genesis.state.clone();
    // LOG0 of 32 zero bytes: PUSH1 32, PUSH1 0, LOG0, STOP.
    if state.set_balance(sender_address, U256::from(1_000_000_000_000_000_000u128)).is_err()
        || state.set_code(contract, Bytes::from_static(&[0x60, 0x20, 0x60, 0x00, 0xa0, 0x00])).is_err()
    {
        return false;
    }
    let logger = EvmCall { to: Some(contract), value: U256::ZERO, input: Bytes::new(), gas_limit: 50_000, delegate: None };
    let paid = |call: &EvmCall| {
        let mut tx = sign_call_with(
            &sender, cfg.chain_id, 0,
            FeeVector { state: aether_execution::fees::STATE_UNIT_PRICE, ..FeeVector::default() },
            0, call,
        ).ok()?;
        tx.header.gas.state = 100;
        let mut signature = sender.sign(&tx.signing_bytes()).ok()?;
        signature.extend_from_slice(&sender.public_key().bytes);
        tx.signature = Bytes::from(signature);
        Some(tx)
    };
    let (Some(plain), Some(logged)) = (paid(&plain), paid(&logger)) else { return false };
    let (Ok(plain_out), Ok(log_out)) = (
        execute_block(&state, &ctx, &[plain]),
        execute_block(&state, &ctx, &[logged]),
    ) else { return false };
    let (Some(plain_receipt), Some(log_receipt)) = (plain_out.receipts.first(), log_out.receipts.first()) else { return false };
    log_receipt.events.len() == 1
        && log_receipt.state_gas > plain_receipt.state_gas
        && aether_execution::fees::MAX_PERSISTENT_BYTES_PER_BLOCK
            * aether_execution::fees::MAX_STORED_BYTES_PER_METERED_BYTE
            <= aether_execution::fees::MAX_PAID_STORED_BYTES_PER_BLOCK
        && aether_execution::fees::MAX_PAID_STORED_BYTES_PER_BLOCK <= 32 * 1024 * 1024
}

/// The published issuance schedule: 1 DBLN a block at height 0, decaying
/// smoothly (−15% a year, well under a tenth of a percent a day, never a
/// halving step) and floored at 0.1 DBLN (docs/design/15-node-rewards.md).
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
        detail: format!("1 DBLN a block, −15%/year ({year} after a year), floored at 0.1 DBLN"),
    }
}

/// The final-file gate (audit 5, A5-4): the four rules that only a network
/// the genesis DKG actually wrote can satisfy. `check` rebuilds a genesis and
/// checks its flags; these decode the exact `output` string with the same
/// decoder a node command uses, seat the roster, pin `identity` to the group
/// public key and refuse revealed seated shares. A pre-DKG file (identity and
/// output both absent, what `assemble` writes) passes with a pre-DKG detail —
/// the assemble-time gate is `check`; a file carrying only one of the two
/// fields fails. `mainnet-rules` prints these after the 20 genesis rules.
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
            (Some(_), _) => format!("the output decodes for {n} players with the same decoder the node's startup uses"),
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

// ===== audit 6, A6-3 + A6-4: the ceremony record every signer binds to =====
//
// The class of defects: a validator ends up voting from a genesis that is not
// the one the ceremony checked — a chain id swapped in transit (A6-3), a stale
// local network.json kept because its id and committee identity happen to
// match (A6-4), any file whose bytes differ from the ones the coordinator
// checked. The fix is one fail-closed bind: the coordinator's `check` writes
// an independent record pinning the whole immutable genesis, the exact bytes
// it passed, and every path to a consensus signature (`aether node --network`,
// `aether run`, verify-local) refuses to start without passing that record
// and this Mac's files through it.

/// Where a data dir keeps its copy of the ceremony record (verify-local
/// stores it here; `aether run`, which starts with no --network and no
/// --ceremony, binds against this copy).
pub const CEREMONY_RECORD_FILE: &str = "ceremony-check.json";
/// The only record format this binary binds to.
pub const CEREMONY_RECORD_VERSION: u64 = 1;

/// The immutable genesis, as the ceremony record pins it: every field a
/// genesis fixes for the chain's life, normalized (hex lowercase). Not
/// pinned: `validators` (a committee the chain itself rotates), `round`,
/// `output`, `epochs` — those evolve by reshare/handoff, and the bind
/// compares them against this Mac's own files instead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordGenesis {
    /// The opening roster the DKG froze (`genesis_validators`): the set the
    /// committee output must seat. Absent on a file the DKG never wrote.
    pub validators: Vec<crate::roster::Member>,
    pub faucet: Option<aether_types::Address>,
    pub registrar: Option<String>,
    pub epoch_blocks: Option<u64>,
    pub min_streak: Option<u64>,
    pub draw_epochs: Option<u64>,
    pub history: Option<u32>,
    pub protocol: Option<u32>,
    pub node_rewards: Option<bool>,
    pub reserve: Option<crate::roster::ReserveFile>,
    pub group: Option<u16>,
    pub max_committee: Option<u64>,
}

/// What the coordinator's `check` writes after PASS (`aether
/// ceremony-record`): the chain this ceremony assembled, the DKG round it
/// ran, the committee identity it pinned, the sha256 of the exact final-file
/// bytes that passed, and the immutable genesis above. Public — it names
/// what was checked, never a secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CeremonyRecord {
    pub version: u64,
    pub chain_id: u64,
    /// Unix seconds the check passed.
    pub checked: u64,
    /// sha256 over the final network.json bytes that passed, lowercase hex.
    pub digest: String,
    /// The DKG round the ceremony ran.
    pub round: u64,
    /// The committee identity the ceremony pinned (immutable for the chain's life).
    pub identity: String,
    /// The immutable genesis a signer may vote under.
    pub genesis: RecordGenesis,
}

fn lower_hex(s: &Option<String>) -> Option<String> {
    s.as_ref().map(|h| h.to_ascii_lowercase())
}

/// The immutable genesis of `file`, normalized for comparison. `Err` on a
/// file with no frozen opening roster (`genesis_validators`) — the DKG
/// always writes one, so its absence means the file was not written by a
/// ceremony.
pub fn record_genesis_of(file: &crate::roster::NetworkFile) -> Result<RecordGenesis, String> {
    Ok(RecordGenesis {
        validators: file.genesis_validators.clone().ok_or(
            "no frozen opening roster (genesis_validators): not a network.json the DKG wrote",
        )?,
        faucet: file.faucet,
        registrar: lower_hex(&file.registrar),
        epoch_blocks: file.epoch_blocks,
        min_streak: file.min_streak,
        draw_epochs: file.draw_epochs,
        history: file.history,
        protocol: file.protocol,
        node_rewards: file.node_rewards,
        reserve: file.reserve.clone().map(|r| crate::roster::ReserveFile {
            operator: r.operator,
            validators: r
                .validators
                .into_iter()
                .map(|m| crate::roster::Member { key: m.key.to_ascii_lowercase(), node: m.node })
                .collect(),
        }),
        group: file.group,
        max_committee: file.max_committee,
    })
}

/// Build the record for a final file that passed `check`. Only a final file
/// (committee identity present, frozen roster present) gets one.
pub fn ceremony_record(
    file: &crate::roster::NetworkFile,
    bytes: &[u8],
    checked_unix: u64,
) -> Result<CeremonyRecord, String> {
    let identity = file
        .identity
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or("no committee identity: a record is only taken from a final file the DKG wrote (run check first)")?;
    Ok(CeremonyRecord {
        version: CEREMONY_RECORD_VERSION,
        chain_id: file.chain_id,
        checked: checked_unix,
        digest: hex::encode(Sha256::digest(bytes)),
        round: file.round,
        identity: identity.to_ascii_lowercase(),
        genesis: record_genesis_of(file)?,
    })
}

/// Read and version-check a ceremony record. A missing, unreadable or
/// malformed record is an error — never a skip.
pub fn load_ceremony_record(path: &std::path::Path) -> Result<CeremonyRecord, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("cannot read the ceremony record {}: {e}", path.display()))?;
    let record: CeremonyRecord = serde_json::from_slice(&bytes).map_err(|e| {
        format!(
            "{} is not a ceremony record this binary can read ({e}): use the {CEREMONY_RECORD_FILE} the coordinator's check wrote",
            path.display()
        )
    })?;
    if record.version != CEREMONY_RECORD_VERSION {
        return Err(format!(
            "ceremony record {} carries version {}, this binary binds version {CEREMONY_RECORD_VERSION}: use the record this ceremony's check wrote",
            path.display(),
            record.version
        ));
    }
    Ok(record)
}

/// The only recordless public-network exception is the exact shipped 7780
/// file. Neither a reserved chain id nor mutable rule flags prove its origin.
pub fn shipped_legacy_network(bytes: &[u8]) -> bool {
    hex::encode(Sha256::digest(bytes))
        == "26faa6bca43e2c1f458ccd4051edb92665efe8939a3ef60652ab92c528b7b9cc"
}

/// The record-vs-file half of the bind: version, chain id (A6-3 — the
/// expected id comes from the record, never from the file being verified),
/// committee identity, and what pins the bytes at this round (the exact
/// digest at the ceremony's round; the immutable genesis for a later one).
/// A follower or candidate Mac — no share to vote with — is held to this
/// half; a signer goes on through `bind_to_ceremony`.
pub fn bind_network_to_record(
    checked: &crate::roster::NetworkFile,
    checked_bytes: &[u8],
    record: &CeremonyRecord,
) -> Result<(), String> {
    if record.version != CEREMONY_RECORD_VERSION {
        return Err(format!(
            "ceremony record version {} is not {CEREMONY_RECORD_VERSION}: use the record this ceremony's check wrote",
            record.version
        ));
    }
    // A6-3: a file swapped in transit must be refused, not self-accepted.
    if checked.chain_id != record.chain_id {
        return Err(format!(
            "the ceremony record pins chain {} but this network.json says chain {}: a file swapped in transit (or another ceremony's file) is not the checked genesis. Get the final network.json the coordinator checked and run scripts/mainnet-genesis.sh verify-local with --ceremony",
            record.chain_id, checked.chain_id
        ));
    }
    let checked_identity = checked.identity.as_deref().filter(|s| !s.is_empty()).ok_or(
        "the network.json to start from has no committee identity: not the final file the DKG wrote",
    )?;
    if !checked_identity.eq_ignore_ascii_case(&record.identity) {
        return Err(format!(
            "the committee identity {}… is not the one the ceremony record pins ({}…): vote only under the committee the ceremony checked",
            &checked_identity[..checked_identity.len().min(16)],
            &record.identity[..record.identity.len().min(16)]
        ));
    }
    // The round decides what pins the checked file: the ceremony's own round
    // demands the exact bytes; a later round is an evolution, pinned by the
    // immutable genesis.
    match checked.round.cmp(&record.round) {
        std::cmp::Ordering::Equal => {
            let digest = hex::encode(Sha256::digest(checked_bytes));
            if !digest.eq_ignore_ascii_case(&record.digest) {
                return Err(format!(
                    "digest mismatch: these bytes are not the final network.json the coordinator checked (sha256 {}…, the record pins {}…). Copy the checked file unchanged — do not reformat or edit it",
                    &digest[..digest.len().min(16)],
                    &record.digest[..record.digest.len().min(16)]
                ));
            }
        }
        std::cmp::Ordering::Greater => {
            let evolved = record_genesis_of(checked).map_err(|why| {
                format!("the evolved network.json to start from has no frozen genesis ({why}): not a file a reshare wrote")
            })?;
            if evolved != record.genesis {
                return Err(
                    "the evolved network.json to start from carries another immutable genesis than the ceremony checked: not an evolution, another chain".to_string(),
                );
            }
        }
        std::cmp::Ordering::Less => {
            return Err(format!(
                "round {} predates the ceremony's round {}: an old committee's file, not this ceremony's",
                checked.round, record.round
            ));
        }
    }
    Ok(())
}

/// The one fail-closed bind a signer passes. `checked`/`checked_bytes` is the
/// final network file this start is bound to (the CLI --network file, or the
/// adopted `<data>/network.json`); `local_threshold`/`local_network` are this
/// Mac's own files (a reshare rewrites both together); `record` is the
/// coordinator's independent pin. Any missing, malformed or differing input
/// is `Err`. Ok means: the bytes the coordinator checked are this genesis,
/// and this Mac's share belongs to the committee of the file it starts from.
pub fn bind_to_ceremony(
    checked: &crate::roster::NetworkFile,
    checked_bytes: &[u8],
    record: &CeremonyRecord,
    local_threshold: &crate::dkg::KeyFile,
    local_network: &crate::roster::NetworkFile,
) -> Result<(), String> {
    bind_network_to_record(checked, checked_bytes, record)?;
    // A6-4: a local network.json is only "the same network" when its whole
    // immutable genesis is — the same id and committee identity are not
    // enough for a stale file to be kept.
    if local_network.chain_id != record.chain_id {
        return Err(format!(
            "the local network.json is chain {}, not the ceremony's chain {}: a file from another chain. Move it aside and run verify-local with the coordinator's record",
            local_network.chain_id, record.chain_id
        ));
    }
    if !local_network
        .identity
        .as_deref()
        .unwrap_or_default()
        .eq_ignore_ascii_case(&record.identity)
    {
        return Err(
            "the local network.json names another committee identity than the ceremony record: a stale file. Reconcile on purpose (verify-local with the coordinator's record, or move the old data aside)".to_string(),
        );
    }
    let local_genesis = record_genesis_of(local_network).map_err(|why| {
        format!(
            "stale local network.json: its genesis is not the one the ceremony checked ({why}). Reconcile on purpose (verify-local with the coordinator's record, or move the old data aside); do not vote under an unchecked genesis"
        )
    })?;
    if local_genesis != record.genesis {
        return Err(
            "stale local network.json: its immutable genesis (roster, registrar, rule flags, reserve) differs from the one the ceremony checked. Reconcile on purpose (verify-local with the coordinator's record, or move the old data aside); do not vote under an unchecked genesis".to_string(),
        );
    }
    // The file this start is bound to must name the committee this Mac's
    // share belongs to (a reshare rewrites the local file and the share
    // together, so the legit evolution always lines up here).
    if checked.round != local_network.round
        || !checked
            .output
            .as_deref()
            .unwrap_or_default()
            .eq_ignore_ascii_case(local_network.output.as_deref().unwrap_or_default())
    {
        return Err(format!(
            "the network.json to start from (round {}) is not this Mac's committee (local round {}): start from the file this Mac's dkg/reshare wrote",
            checked.round, local_network.round
        ));
    }
    // A5-4's check, kept inside the bind: the share votes only under the
    // committee the local file carries.
    local_share_matches_network(local_threshold, local_network)?;
    Ok(())
}

/// The bind a follower or candidate Mac passes (`aether run` on a Mac with no
/// threshold.json — no share to vote with): its adopted network file against
/// the record's half. It serves wallets the chain the ceremony checked, so it
/// is bound to the same genesis; only the share comparison is a signer's.
/// A record that was named but is missing fails closed on it; a Mac with no
/// record anywhere and no share cannot vote (it verifies blocks by
/// certificate; a wrong genesis shows up as a chain that never syncs), so it
/// keeps following with a warning instead of stranding every consumer Mac.
/// On a successful bind the record is stored in the data dir, like
/// verify-local does — the next start (and a later reshare seat) binds even
/// without the --network file it came from.
pub fn bind_shareless_to_ceremony(
    file: &crate::roster::NetworkFile,
    bytes: &[u8],
    data: &std::path::Path,
    ceremony: Option<&std::path::Path>,
) -> Result<(), String> {
    let record_path = ceremony.map_or_else(|| data.join(CEREMONY_RECORD_FILE), std::path::Path::to_path_buf);
    if !record_path.exists() {
        if ceremony.is_some() {
            return Err(format!(
                "chain {}: no ceremony record at {}: this start was told to bind to that record and it is missing.\n  Copy the {CEREMONY_RECORD_FILE} the coordinator's check wrote next to the network.json, then start again",
                file.chain_id,
                record_path.display()
            ));
        }
        tracing::warn!(
            chain = file.chain_id,
            "no ceremony record for chain {} on this Mac and none was passed: a Mac with no \
             share cannot vote (it verifies blocks by certificate), so it keeps following. The \
             release build ships {CEREMONY_RECORD_FILE} next to the bundled network.json — \
             install it to pin the checked genesis; a wrong genesis only ever fails to sync",
            file.chain_id
        );
        return Ok(());
    }
    let record = load_ceremony_record(&record_path)?;
    bind_network_to_record(file, bytes, &record)?;
    let stored = data.join(CEREMONY_RECORD_FILE);
    if stored != record_path {
        let bytes = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
        std::fs::write(&stored, bytes)
            .map_err(|e| format!("cannot store the ceremony record at {}: {e}", stored.display()))?;
    }
    Ok(())
}

/// The record a start binds to, in fail-closed order: the explicit
/// --ceremony (passed through even when missing — the load then errors on the
/// named file, never silently falls through to another record), the copy
/// verify-local stored in the data dir, and the record next to the --network
/// file. The coordinator's check writes `ceremony-check.json` next to the
/// final network.json it passed, and the wallet app ships the same pair next
/// to its bundled network.json — that pair is how a consumer Mac receives the
/// record without any manual step. `None` means none was found.
pub fn resolve_ceremony_record(
    ceremony: Option<&std::path::Path>,
    data: &std::path::Path,
    network: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    if let Some(path) = ceremony {
        return Some(path.to_path_buf());
    }
    let stored = data.join(CEREMONY_RECORD_FILE);
    if stored.exists() {
        return Some(stored);
    }
    network
        .and_then(|n| n.parent())
        .map(|dir| dir.join(CEREMONY_RECORD_FILE))
        .filter(|sibling| sibling.exists())
}

/// The release gate (`aether mainnet-rules --bundle`): a new-genesis app
/// build must ship the coordinator's record next to the network.json it
/// bundles, pinning that file's exact bytes — a consumer Mac receives the
/// genesis through this pair, so no build may hand it an unchecked one (or
/// another ceremony's, or a file edited after the check). The legacy testnet
/// app ships no record and passes.
pub fn check_bundle(path: &std::path::Path, file: &crate::roster::NetworkFile, bytes: &[u8]) -> Rule {
    let name = "bundled ceremony record";
    let sibling = path.parent().map(|dir| dir.join(CEREMONY_RECORD_FILE));
    if !sibling.as_ref().is_some_and(|record| record.exists()) && shipped_legacy_network(bytes) {
        return Rule {
            name,
            ok: true,
            detail: format!(
                "chain {} is not a new genesis: this app build ships no record (the {TESTNET_CHAIN_ID} testnet app never does)",
                file.chain_id
            ),
        };
    }
    let Some(sibling) = path.parent().map(|dir| dir.join(CEREMONY_RECORD_FILE)) else {
        return Rule {
            name,
            ok: false,
            detail: format!("no {} next to {}: a new-genesis app build must ship the record the coordinator's check wrote", CEREMONY_RECORD_FILE, path.display()),
        };
    };
    if !sibling.exists() {
        return Rule {
            name,
            ok: false,
            detail: format!(
                "no {} next to {}: a new-genesis app build must ship the record the coordinator's check wrote next to the network.json",
                CEREMONY_RECORD_FILE,
                path.display()
            ),
        };
    }
    let record = match load_ceremony_record(&sibling) {
        Ok(record) => record,
        Err(why) => return Rule { name, ok: false, detail: why },
    };
    if record.chain_id != file.chain_id {
        return Rule {
            name,
            ok: false,
            detail: format!(
                "the bundled {} pins chain {} but the bundled network.json is chain {}: ship the record and the network.json from the same check, not another ceremony's",
                CEREMONY_RECORD_FILE,
                record.chain_id,
                file.chain_id
            ),
        };
    }
    let digest = hex::encode(Sha256::digest(bytes));
    if !digest.eq_ignore_ascii_case(&record.digest) {
        return Rule {
            name,
            ok: false,
            detail: format!(
                "digest mismatch: the bundled {} pins other bytes (sha256 {}…; the bundled file is {}…): ship the record and the network.json from the same check — the exact bytes, unchanged",
                CEREMONY_RECORD_FILE,
                &record.digest[..record.digest.len().min(16)],
                &digest[..digest.len().min(16)]
            ),
        };
    }
    Rule {
        name,
        ok: true,
        detail: format!(
            "{} next to the bundled file pins its exact bytes (chain {}, round {}, identity {}…)",
            CEREMONY_RECORD_FILE,
            file.chain_id,
            record.round,
            &record.identity[..record.identity.len().min(16)]
        ),
    }
}

/// The bind every path to a signature calls: `aether node --network … --data
/// …` (before anything starts), `aether run` (after adopt_network, over the
/// adopted file) and verify-local (`aether mainnet-bind`). Reads the checked
/// file, this Mac's network.json/threshold.json and the ceremony record
/// (explicit, or the copy verify-local left in the data dir), and runs them
/// through `bind_to_ceremony`. On success the record is (re)stored in the
/// data dir, so the wallet's `aether run` — which starts with no --network
/// and no --ceremony — binds to the same ceremony next time. Legacy chains
/// (the 7780 testnet, devnets without --network) pass through untouched.
pub fn bind_data_dir(
    data: &std::path::Path,
    checked: &std::path::Path,
    ceremony: Option<&std::path::Path>,
) -> Result<(), String> {
    let checked_bytes = std::fs::read(checked)
        .map_err(|e| format!("cannot read {}: {e}", checked.display()))?;
    let checked_file: crate::roster::NetworkFile = serde_json::from_slice(&checked_bytes)
        .map_err(|e| format!("{} is not a network.json: {e}", checked.display()))?;
    let resolved = resolve_ceremony_record(ceremony, data, Some(checked));
    if resolved.is_none() && shipped_legacy_network(&checked_bytes) {
        let local = data.join("network.json");
        let local_bytes = std::fs::read(&local).map_err(|e| format!("cannot read {}: {e}", local.display()))?;
        if !shipped_legacy_network(&local_bytes) {
            return Err("the local network.json is not the exact shipped 7780 file: verify-local must bind it to a ceremony record".into());
        }
        return Ok(());
    }
    let record_path = resolved.unwrap_or_else(|| data.join(CEREMONY_RECORD_FILE));
    if !record_path.exists() {
        return Err(format!(
            "chain {}: no ceremony record at {} and none was passed: a validator may not start voting on a new genesis without the record the coordinator's check wrote.\n  Run scripts/mainnet-genesis.sh verify-local <final network.json> --data {} --ceremony <{CEREMONY_RECORD_FILE}> on this Mac first (it stores the record in the data dir), then start the node",
            checked_file.chain_id,
            record_path.display(),
            data.display()
        ));
    }
    let record = load_ceremony_record(&record_path)?;
    let local_bytes = std::fs::read(data.join("network.json")).map_err(|e| {
        format!(
            "cannot read {}/network.json ({e}): this Mac must hold the network.json its dkg/reshare wrote before it can vote",
            data.display()
        )
    })?;
    let local_network: crate::roster::NetworkFile = serde_json::from_slice(&local_bytes)
        .map_err(|e| {
            format!(
                "{}/network.json is malformed ({e}): restore the file this Mac's dkg/reshare wrote",
                data.display()
            )
        })?;
    let threshold_bytes = std::fs::read(data.join("threshold.json"))
        .map_err(|e| format!("no threshold.json in {}: {e} — this Mac holds no committee share for this chain", data.display()))?;
    let local_threshold: crate::dkg::KeyFile = serde_json::from_slice(&threshold_bytes)
        .map_err(|e| format!("{}/threshold.json is malformed ({e})", data.display()))?;
    bind_to_ceremony(&checked_file, &checked_bytes, &record, &local_threshold, &local_network)?;
    let stored = data.join(CEREMONY_RECORD_FILE);
    if stored != record_path {
        let bytes = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
        std::fs::write(&stored, bytes)
            .map_err(|e| format!("cannot store the ceremony record at {}: {e}", stored.display()))?;
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
    const NAMES: [&str; 20] = [
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
        "receipt commitments",
        "paid state growth",
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
        let mut legacy = mainnet();
        legacy.node_rewards = false;
        legacy.history_v2 = false;
        let legacy_off = off(legacy);
        assert!(legacy_off.contains(&"paid state growth"));
        assert!(legacy_off.contains(&"receipt commitments"));
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
    /// writes: `output`, `identity`, `round` — and the opening roster
    /// `carry_genesis` freezes into `genesis_validators`.
    fn final_file(round: u64, output: Option<String>, identity: Option<String>) -> crate::roster::NetworkFile {
        use commonware_cryptography::Signer as _;
        let (rx, ry) = real_registrar();
        let mut registrar = [0u8; 64];
        registrar[..32].copy_from_slice(&rx);
        registrar[32..].copy_from_slice(&ry);
        let validators = (1..=4u64)
            .map(|i| {
                let k = aether_light::devnet_validator_key(i);
                crate::roster::Member {
                    key: hex::encode(k.public_key().as_ref()),
                    node: aether_net::devnet_node_id(i).to_string(),
                }
            })
            .collect::<Vec<_>>();
        crate::roster::NetworkFile {
            chain_id: 7_801,
            validators: validators.clone(),
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
            genesis_validators: Some(validators),
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

    // ===== audit 6, A6-3 + A6-4: binding the signer to the checked genesis =====

    /// The bytes a ceremony record is taken over, and the record itself, as the
    /// coordinator's `check` writes it after PASS (`aether ceremony-record`).
    fn record_for(file: &crate::roster::NetworkFile) -> (Vec<u8>, CeremonyRecord) {
        let bytes = serde_json::to_vec_pretty(file).unwrap();
        let record = ceremony_record(file, &bytes, 1_759_000_000).unwrap();
        (bytes, record)
    }

    /// A validator Mac's data dir holding exactly these two files.
    fn bind_dir(tag: &str, threshold: &crate::dkg::KeyFile, network: &crate::roster::NetworkFile) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-bind-{tag}-{}-{}", std::process::id(), network.round));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("threshold.json"), serde_json::to_vec(threshold).unwrap()).unwrap();
        std::fs::write(dir.join("network.json"), serde_json::to_vec_pretty(network).unwrap()).unwrap();
        dir
    }

    /// The happy path, both files this Mac actually holds: the checked final
    /// file itself, and a later reshare/handoff evolution of it — the pinned
    /// immutable genesis and the local share's match against the local file are
    /// what carry, never byte equality.
    #[test]
    fn the_signer_binds_to_the_genesis_the_ceremony_checked() {
        let (output, identity) = ceremony_output();
        let checked = final_file(0, Some(output.clone()), Some(identity.clone()));
        let (bytes, record) = record_for(&checked);
        let share = crate::dkg::KeyFile { round: 0, output: output.clone(), identity: identity.clone(), share: "00".into() };
        bind_to_ceremony(&checked, &bytes, &record, &share, &checked)
            .expect("a Mac holding the checked file and its share binds");
        // A Mac whose local file the chain itself evolved (round 3, a new
        // output, an epoch start) still binds to the round-0 record: the
        // immutable genesis is what carries, and a later round is an
        // evolution this Mac took part in (its share moved with it).
        let mut evolved = final_file(0, Some(output.clone()), Some(identity.clone()));
        evolved.round = 3;
        evolved.output = Some("a".repeat(96));
        evolved.epochs.push(crate::roster::EpochStart { height: 100, parent: "aa".into() });
        let evolved_bytes = serde_json::to_vec_pretty(&evolved).unwrap();
        let evolved_share = crate::dkg::KeyFile { round: 3, output: evolved.output.clone().unwrap(), identity: identity.clone(), share: "00".into() };
        bind_to_ceremony(&evolved, &evolved_bytes, &record, &evolved_share, &evolved)
            .expect("an evolution of the checked genesis binds to its record");
        // Starting from the ORIGINAL round-0 file while holding a round-3
        // share is refused: the node would vote under a committee this Mac's
        // key does not match (the file it starts from must be its own file).
        assert!(bind_to_ceremony(&checked, &bytes, &record, &evolved_share, &evolved).is_err());
        // But not a record from a LATER ceremony this file predates: a round
        // older than the record's is not an evolution, it is another committee.
        let older = final_file(0, Some(output), Some(identity));
        let older_bytes = serde_json::to_vec_pretty(&older).unwrap();
        let record9 = CeremonyRecord { round: 9, ..record.clone() };
        assert!(bind_to_ceremony(&older, &older_bytes, &record9, &share, &older).is_err(), "a round older than the record's");
    }

    /// A6-3: the coordinator checked chain 7801; the file this Mac received
    /// says 7802. The expected id comes from the record, never from the file
    /// being verified — the swap must be refused, not self-accepted.
    #[test]
    fn binding_refuses_a_chain_id_swapped_in_transit() {
        let (output, identity) = ceremony_output();
        let checked = final_file(0, Some(output.clone()), Some(identity.clone()));
        let (bytes, record) = record_for(&checked);
        let mut swapped = checked.clone();
        swapped.chain_id = 7_802;
        let share = crate::dkg::KeyFile { round: 0, output, identity, share: "00".into() };
        let swapped_bytes = serde_json::to_vec_pretty(&swapped).unwrap();
        let err = bind_to_ceremony(&swapped, &swapped_bytes, &record, &share, &swapped).unwrap_err();
        assert!(err.contains("7801") && err.contains("7802"), "{err}");
        assert!(err.contains("ceremony"), "the refusal names the ceremony record: {err}");
    }

    /// Same roster, same round, a different committee output: not the bytes the
    /// coordinator checked (digest), and a local share for that other committee
    /// is refused even when every other field lines up.
    #[test]
    fn binding_refuses_an_output_the_coordinator_did_not_check() {
        let (output, identity) = ceremony_output();
        let checked = final_file(0, Some(output.clone()), Some(identity.clone()));
        let (bytes, record) = record_for(&checked);
        let share = crate::dkg::KeyFile { round: 0, output: output.clone(), identity: identity.clone(), share: "00".into() };
        // The file itself was swapped for another output: digest mismatch.
        let mut other = checked.clone();
        other.output = Some("b".repeat(96));
        let other_bytes = serde_json::to_vec_pretty(&other).unwrap();
        let other_share = crate::dkg::KeyFile { round: 0, output: other.output.clone().unwrap(), identity, share: "00".into() };
        let err = bind_to_ceremony(&other, &other_bytes, &record, &other_share, &other).unwrap_err();
        assert!(err.contains("digest"), "{err}");
        // The Mac's own share is for another committee of the same roster and
        // round (the A6-4 sequence): refused against the local file it votes with.
        let err = bind_to_ceremony(&checked, &bytes, &record, &other_share, &checked).unwrap_err();
        assert!(err.contains("output"), "a different public polynomial must not vote: {err}");
        // Sanity of the refusal's premise: the matching share, on the other
        // hand, does bind (covered above — here only the mismatch is refused).
        assert!(local_share_matches_network(&share, &checked).is_ok());
    }

    /// A6-4: a local network.json with the same chain id and committee identity
    /// but a different immutable genesis (one flag off) is stale, not kept: the
    /// Mac would vote under a genesis the ceremony did not check.
    #[test]
    fn binding_refuses_a_stale_local_genesis() {
        let (output, identity) = ceremony_output();
        let checked = final_file(0, Some(output.clone()), Some(identity.clone()));
        let (bytes, record) = record_for(&checked);
        let share = crate::dkg::KeyFile { round: 0, output, identity, share: "00".into() };
        for stale in [
            { let mut f = checked.clone(); f.history = Some(1); f },        // an older rule set
            { let mut f = checked.clone(); f.registrar = Some("cd".repeat(32)); f }, // another registrar
            { let mut f = checked.clone(); f.genesis_validators = None; f }, // no frozen opening roster
        ] {
            let err = bind_to_ceremony(&checked, &bytes, &record, &share, &stale).unwrap_err();
            assert!(err.contains("stale"), "a same-id file with another genesis must be called stale: {err}");
        }
        // And a local file for another chain id outright.
        let mut other_chain = checked.clone();
        other_chain.chain_id = 7_802;
        let err = bind_to_ceremony(&checked, &bytes, &record, &share, &other_chain).unwrap_err();
        assert!(err.contains("7802") && err.contains("chain"), "{err}");
    }

    /// The record file itself: a version this binary does not know, or bytes
    /// that are not the format, are errors — never a skip.
    #[test]
    fn a_bad_ceremony_record_is_refused() {
        let (output, identity) = ceremony_output();
        let checked = final_file(0, Some(output.clone()), Some(identity.clone()));
        let (bytes, mut record) = record_for(&checked);
        let share = crate::dkg::KeyFile { round: 0, output, identity, share: "00".into() };
        record.version = 2;
        assert!(bind_to_ceremony(&checked, &bytes, &record, &share, &checked)
            .unwrap_err()
            .contains("version"));
        let dir = std::env::temp_dir().join(format!("aether-record-{}-{}", std::process::id(), std::thread::current().name().unwrap_or("t")));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ceremony-check.json"), b"{not json").unwrap();
        assert!(load_ceremony_record(&dir.join("ceremony-check.json")).is_err(), "not JSON");
        std::fs::write(dir.join("ceremony-check.json"), br#"{"chain_id": 7801}"#).unwrap();
        assert!(load_ceremony_record(&dir.join("ceremony-check.json")).is_err(), "a chain id alone is not a record");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// bind_data_dir — what `aether node`, `aether run` and verify-local all
    /// call before voting: a missing, malformed or absent local network.json is
    /// an error (A6-4's silent skip), and a chain with no record at all names
    /// the operator step instead of starting unbound.
    #[test]
    fn bind_data_dir_fails_closed_on_every_broken_input() {
        let (output, identity) = ceremony_output();
        let checked = final_file(0, Some(output.clone()), Some(identity.clone()));
        let (bytes, record) = record_for(&checked);
        let share = crate::dkg::KeyFile { round: 0, output, identity, share: "00".into() };
        let dir = bind_dir("inputs", &share, &checked);
        std::fs::write(dir.join("final.json"), &bytes).unwrap();
        std::fs::write(dir.join("record.json"), serde_json::to_vec_pretty(&record).unwrap()).unwrap();

        // The good bind, explicit record: passes and KEEPS the record in the
        // data dir, so the wallet's `aether run` (no --network, no --ceremony)
        // binds to the same ceremony on its next start.
        bind_data_dir(&dir, &dir.join("final.json"), Some(&dir.join("record.json"))).unwrap();
        assert!(dir.join(CEREMONY_RECORD_FILE).exists(), "the record is persisted for the run path");
        let persisted = load_ceremony_record(&dir.join(CEREMONY_RECORD_FILE)).unwrap();
        assert_eq!(persisted.chain_id, record.chain_id);
        // And that persisted record alone (no --ceremony) binds again.
        bind_data_dir(&dir, &dir.join("final.json"), None).unwrap();

        // No record anywhere: the operator step, not a silent unbound start.
        std::fs::remove_file(dir.join(CEREMONY_RECORD_FILE)).unwrap();
        std::fs::remove_file(dir.join("record.json")).unwrap();
        let err = bind_data_dir(&dir, &dir.join("final.json"), None).unwrap_err();
        assert!(err.contains("verify-local") && err.contains("ceremony"), "{err}");

        // A local network.json that is absent or malformed is an error, never a skip.
        std::fs::write(dir.join("record.json"), serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        std::fs::remove_file(dir.join("network.json")).unwrap();
        assert!(bind_data_dir(&dir, &dir.join("final.json"), Some(&dir.join("record.json")))
            .unwrap_err()
            .contains("network.json"));
        std::fs::write(dir.join("network.json"), b"{torn write").unwrap();
        assert!(bind_data_dir(&dir, &dir.join("final.json"), Some(&dir.join("record.json"))).is_err());
        std::fs::write(dir.join("network.json"), &bytes).unwrap();
        // A malformed threshold is an error too.
        std::fs::write(dir.join("threshold.json"), b"{torn").unwrap();
        assert!(bind_data_dir(&dir, &dir.join("final.json"), Some(&dir.join("record.json"))).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn h1_record_gate_survives_mutable_flags_and_chain_id() {
        let (output, identity) = ceremony_output();
        let checked = final_file(0, Some(output.clone()), Some(identity.clone()));
        let (bytes, record) = record_for(&checked);
        let share = crate::dkg::KeyFile { round: 0, output, identity, share: "00".into() };
        let dir = bind_dir("h1-flags", &share, &checked);
        std::fs::write(dir.join(CEREMONY_RECORD_FILE), serde_json::to_vec(&record).unwrap()).unwrap();
        let mut cases = Vec::new();
        let mut rewards = checked.clone(); rewards.node_rewards = Some(false); rewards.history = Some(1); cases.push(rewards);
        let mut history = checked.clone(); history.history = Some(1); history.node_rewards = None; cases.push(history);
        let mut swapped = checked.clone(); swapped.chain_id = TESTNET_CHAIN_ID; cases.push(swapped);
        for altered in cases {
            std::fs::write(dir.join("final.json"), serde_json::to_vec(&altered).unwrap()).unwrap();
            assert!(bind_data_dir(&dir, &dir.join("final.json"), None).is_err(), "mutable fields must not bypass the stored record");
        }
        std::fs::remove_file(dir.join(CEREMONY_RECORD_FILE)).unwrap();
        std::fs::write(dir.join("final.json"), &bytes).unwrap();
        let mut disabled = checked.clone(); disabled.node_rewards = None; disabled.history = None;
        std::fs::write(dir.join("final.json"), serde_json::to_vec(&disabled).unwrap()).unwrap();
        assert!(bind_data_dir(&dir, &dir.join("final.json"), None).is_err(), "a signer cannot disable the requirement with file flags");
        let legacy: crate::roster::NetworkFile = serde_json::from_slice(include_bytes!("../tests/fixtures/legacy-7780-network.json")).unwrap();
        std::fs::write(dir.join("final.json"), include_bytes!("../tests/fixtures/legacy-7780-network.json")).unwrap();
        assert!(bind_data_dir(&dir, &dir.join("final.json"), None).is_err(), "a pristine incoming legacy file cannot excuse another local genesis");
        std::fs::write(dir.join("network.json"), include_bytes!("../tests/fixtures/legacy-7780-network.json")).unwrap();
        bind_data_dir(&dir, &dir.join("final.json"), None).expect("the shipped 7780 remains usable");
        let mut forged = legacy; forged.faucet = None;
        std::fs::write(dir.join("final.json"), serde_json::to_vec(&forged).unwrap()).unwrap();
        assert!(bind_data_dir(&dir, &dir.join("final.json"), None).is_err(), "7780 alone does not identify the shipped genesis");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A follower or candidate Mac (no threshold.json — no share to vote with)
    /// is still bound to the record's half: the same refusals, minus the share.
    #[test]
    fn a_shareless_mac_is_bound_to_the_record_half() {
        let (output, identity) = ceremony_output();
        let checked = final_file(0, Some(output), Some(identity));
        let (bytes, record) = record_for(&checked);
        let dir = std::env::temp_dir().join(format!("aether-shareless-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("record.json"), serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        bind_shareless_to_ceremony(&checked, &bytes, &dir, Some(&dir.join("record.json")))
            .expect("a follower Mac binds its file to the record");
        assert!(dir.join(CEREMONY_RECORD_FILE).exists(), "the bound record is stored for the next start (the wallet's run passes no --ceremony)");
        // A chain id swapped in transit is refused even with no share at stake.
        let mut swapped = checked.clone();
        swapped.chain_id = 7_802;
        let swapped_bytes = serde_json::to_vec_pretty(&swapped).unwrap();
        let err = bind_shareless_to_ceremony(&swapped, &swapped_bytes, &dir, Some(&dir.join("record.json")))
            .unwrap_err();
        assert!(err.contains("7802"), "{err}");
        // A record that was named but is not on disk fails closed on it.
        let err = bind_shareless_to_ceremony(&checked, &bytes, &dir, Some(&dir.join("missing.json")))
            .unwrap_err();
        assert!(err.contains("no ceremony record"), "{err}");
        // No record anywhere: a Mac with no share cannot vote (it verifies
        // blocks by certificate), so it keeps following with a warning — a
        // consumer Mac must never be stranded for lacking a public file.
        bind_shareless_to_ceremony(&checked, &bytes, &dir, None)
            .expect("a shareless Mac follows without a record instead of refusing to run");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Where a start finds its record, in fail-closed order: the explicit
    /// --ceremony (named, so a missing one fails at the read, never a silent
    /// fallthrough to some other record), the copy verify-local stored in the
    /// data dir, and the record shipped next to the --network file (the
    /// coordinator's check writes it there; the wallet app bundles the pair).
    #[test]
    fn the_record_a_start_binds_to_is_explicit_then_stored_then_bundled() {
        let dir = std::env::temp_dir().join(format!("aether-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundle = dir.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        let explicit = dir.join("explicit.json");
        std::fs::write(&explicit, b"{}").unwrap();

        // Nothing anywhere: None.
        assert_eq!(resolve_ceremony_record(None, &dir, Some(&bundle.join("network.json"))), None);
        // A record next to the network file: the bundled pair.
        std::fs::write(bundle.join(CEREMONY_RECORD_FILE), b"{}").unwrap();
        assert_eq!(
            resolve_ceremony_record(None, &dir, Some(&bundle.join("network.json"))),
            Some(bundle.join(CEREMONY_RECORD_FILE))
        );
        // The data-dir copy beats the bundled one.
        std::fs::write(dir.join(CEREMONY_RECORD_FILE), b"{}").unwrap();
        assert_eq!(
            resolve_ceremony_record(None, &dir, Some(&bundle.join("network.json"))),
            Some(dir.join(CEREMONY_RECORD_FILE))
        );
        assert_eq!(
            resolve_ceremony_record(None, &dir, None),
            Some(dir.join(CEREMONY_RECORD_FILE)),
            "no --network file to look beside: the stored copy still carries"
        );
        // The explicit record beats everything, even a missing one: naming it
        // must fail closed on that file, not fall through to another record.
        assert_eq!(
            resolve_ceremony_record(Some(&explicit), &dir, Some(&bundle.join("network.json"))),
            Some(explicit)
        );
        assert_eq!(
            resolve_ceremony_record(Some(&dir.join("missing.json")), &dir, None),
            Some(dir.join("missing.json"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The release gate (`aether mainnet-rules --bundle`): a new-genesis app
    /// build must ship the coordinator's record next to the network.json it
    /// bundles, pinning the exact bytes — no Mac can be handed an unchecked
    /// genesis through an app update. The legacy testnet app ships none.
    #[test]
    fn the_app_bundle_gate_pins_the_bundled_record_to_the_file() {
        let dir = std::env::temp_dir().join(format!("aether-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (output, identity) = ceremony_output();
        let checked = final_file(0, Some(output), Some(identity));
        let (bytes, record) = record_for(&checked);
        let net = dir.join("network.json");
        std::fs::write(&net, &bytes).unwrap();

        // No record beside the bundled file: FAIL, naming what to ship.
        let rule = check_bundle(&net, &checked, &bytes);
        assert_eq!(rule.name, "bundled ceremony record");
        assert!(!rule.ok, "{}", rule.detail);
        assert!(rule.detail.contains(CEREMONY_RECORD_FILE), "the refusal names the missing file: {}", rule.detail);

        // The coordinator's record beside it: the digest carries.
        std::fs::write(dir.join(CEREMONY_RECORD_FILE), serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let ok = check_bundle(&net, &checked, &bytes);
        assert!(ok.ok, "{}", ok.detail);

        // A record pinning other bytes (a network.json changed after the
        // check) is not the pair the app may ship.
        let mut edited = checked.clone();
        edited.round = 4; // same genesis, other bytes than the record was taken over
        let edited_bytes = serde_json::to_vec_pretty(&edited).unwrap();
        let rule = check_bundle(&net, &edited, &edited_bytes);
        assert!(!rule.ok, "{}", rule.detail);
        assert!(rule.detail.contains("digest"), "{}", rule.detail);

        // Another ceremony's record (its chain id) is refused.
        let mut other_chain = record.clone();
        other_chain.chain_id = 7_802;
        std::fs::write(dir.join(CEREMONY_RECORD_FILE), serde_json::to_vec_pretty(&other_chain).unwrap()).unwrap();
        let rule = check_bundle(&net, &checked, &bytes);
        assert!(!rule.ok && rule.detail.contains("7802"), "{}", rule.detail);

        // Mutable flags and the reserved id cannot bypass the bundled record.
        let mut downgraded = checked.clone();
        downgraded.node_rewards = None;
        downgraded.history = None;
        assert!(!check_bundle(&net, &downgraded, &serde_json::to_vec(&downgraded).unwrap()).ok);
        downgraded.chain_id = TESTNET_CHAIN_ID;
        assert!(!check_bundle(&net, &downgraded, &serde_json::to_vec(&downgraded).unwrap()).ok);
        // The real shipped legacy testnet app needs no record.
        std::fs::remove_file(dir.join(CEREMONY_RECORD_FILE)).unwrap();
        let bytes = include_bytes!("../tests/fixtures/legacy-7780-network.json");
        let legacy = serde_json::from_slice(bytes).unwrap();
        let rule = check_bundle(&net, &legacy, bytes);
        assert!(rule.ok, "{}", rule.detail);
        assert!(rule.detail.contains(TESTNET_CHAIN_ID.to_string().as_str()), "the ok names which app needs no record: {}", rule.detail);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
