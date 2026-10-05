//! Who the validators are, and this machine's own keys.
//!
//! A network is described by `network.json`: every validator's consensus key
//! (ed25519) and iroh node id, plus, after the DKG, the committee identity that
//! wallets pin. Each validator generates its own keys locally (`aether keygen`)
//! and only ever shares the public halves. Without a network file the node falls
//! back to the public devnet keys (anyone can impersonate those validators).

use crate::block::PublicKey;
use aether_net::{EndpointId, SecretKey};
use commonware_codec::{DecodeExt, Encode};
use commonware_cryptography::{ed25519, Signer as _};
use commonware_utils::{ordered::Set, TryCollect};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const KEY_FILE: &str = "validator.key";
pub const PUBLIC_FILE: &str = "validator.pub.json";
pub const NETWORK_FILE: &str = "network.json";

/// One validator's public entry.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Member {
    /// ed25519 consensus public key (hex).
    pub key: String,
    /// iroh node id (as printed by iroh).
    pub node: String,
}

/// A committee change: the new epoch's first height, and the hash of the
/// finalized block just before it (the old committee's last block).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EpochStart {
    pub height: u64,
    pub parent: String,
}

/// `network.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkFile {
    pub chain_id: u64,
    pub validators: Vec<Member>,
    /// Committee identity (hex) once the DKG ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    #[serde(default)]
    pub round: u64,
    /// Public DKG output (hex): the committee polynomial, which a reshare starts from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Committee changes so far (one per reshare), oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub epochs: Vec<EpochStart>,
    /// The only account funded at genesis on a public network (no public dev keys).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faucet: Option<aether_types::Address>,
    /// DeviceCheck registrar's P-256 key (x‖y hex): it attests voting-node candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registrar: Option<String>,
    /// Blocks per voting-node epoch (default one hour of 1 s blocks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch_blocks: Option<u64>,
    /// Epochs of unbroken liveness before a Mac can be drawn (default 24).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_streak: Option<u64>,
    /// Epochs between voting-set draws (default 24).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_epochs: Option<u64>,
    /// History format (2 = history v2: quiet empty blocks and era files, see
    /// `ChainConfig::history_v2`). Absent on 7780 and older networks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<u32>,
    /// Protocol rules this genesis starts under (e.g. 3: proof market, registry
    /// v2 with its registration cap and the 16-seat growth, from height 0;
    /// docs/design/15-node-rewards.md "업그레이드 불필요"). Absent on 7780 and
    /// older networks: protocol 1, later protocols arrive by committee-signed
    /// upgrade — so their genesis stays byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<u32>,
    /// Node rewards from genesis (docs/design/15-node-rewards.md; default off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_rewards: Option<bool>,
    /// Founder reserve keys (with node rewards; docs/design/12-launch-plan.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserve: Option<ReserveFile>,
    /// The consensus group this chain is (13-roadmap.md, 그룹 분열 준비):
    /// 0 — the default — is the only group today; a group other than 0 is a
    /// new genesis of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u16>,
    /// Committee ceiling the voting set grows to before draws swap seats
    /// (`rotation::GROW_UNTIL`, 16, by default; 4..=128).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_committee: Option<u64>,
    /// The validators this network opened with. `validators` names the running
    /// committee (each handoff rewrites it), but a node re-syncing from
    /// genesis — and every node deriving the genesis rewards words — needs the
    /// very first roster however many committees came and went, so `aether
    /// network` freezes it here and ceremonies carry it on (`keep_genesis`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genesis_validators: Option<Vec<Member>>,
    /// The Mac app's release-approval pin (docs/design/19-release-approval.md,
    /// checklist B6): the ReleaseLog predeploy, its runtime code hash and the
    /// three builder keys the updater trusts. Required on a new genesis (the
    /// mainnet rule "release pin"); absent on 7780, whose file stays
    /// byte-identical and keeps the legacy Sparkle-only update path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleasePin>,
}

/// `network.json` `release`: what the wallet's updater trusts, and nothing
/// else — no compiled-in fallback exists for a new-genesis chain.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleasePin {
    /// ReleaseLog address (0x + 40 hex): the new-genesis predeploy.
    pub log: String,
    /// keccak256 of its runtime code (0x + 64 hex).
    pub code_hash: String,
    /// The three builders' P-256 keys, uncompressed SEC1 (04‖x‖y, 130 hex).
    pub builder_keys: Vec<String>,
    /// Builder signatures a normal release needs (2).
    pub threshold: u8,
    /// Builder signatures an emergency release needs (3).
    pub emergency_threshold: u8,
}

impl ReleasePin {
    /// The pin for `builder_keys` on a new genesis: the predeploy address and
    /// code hash, the fixed 2-of-3 / 3-of-3 rule, keys lowercased.
    pub fn for_builders(builder_keys: &[String]) -> Result<Self, String> {
        let pin = ReleasePin {
            log: format!("{:#x}", aether_execution::release_log::ADDRESS),
            code_hash: format!("{:#x}", aether_execution::release_log::code_hash()),
            builder_keys: builder_keys.iter().map(|k| k.trim_start_matches("0x").to_ascii_lowercase()).collect(),
            threshold: aether_execution::release_log::THRESHOLD,
            emergency_threshold: aether_execution::release_log::EMERGENCY_THRESHOLD,
        };
        pin.validate()?;
        Ok(pin)
    }

    /// The pin from a release config file (`aether network --release`):
    /// `{"builder_keys": [three 04‖x‖y hex keys]}`. It may also name `log`,
    /// `code_hash`, `threshold` and `emergency_threshold` (e.g. copied from
    /// `scripts/release-approve.py contract-hash`); each must then equal the
    /// new-genesis predeploy and the 2/3, 3/3 rule — a config that expects
    /// another ReleaseLog is refused, never silently replaced.
    pub fn from_config(bytes: &[u8]) -> Result<Self, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Config {
            builder_keys: Vec<String>,
            log: Option<String>,
            code_hash: Option<String>,
            threshold: Option<u8>,
            emergency_threshold: Option<u8>,
        }
        let c: Config = serde_json::from_slice(bytes).map_err(|e| format!("release config: {e}"))?;
        let pin = Self::for_builders(&c.builder_keys)?;
        if let Some(log) = c.log.filter(|l| !l.eq_ignore_ascii_case(&pin.log)) {
            return Err(format!("release config: log {log} is not the new-genesis ReleaseLog {}", pin.log));
        }
        if let Some(hash) = c.code_hash.filter(|h| !h.eq_ignore_ascii_case(&pin.code_hash)) {
            return Err(format!(
                "release config: code_hash {hash} is not the ReleaseLog runtime this binary predeploys ({}); rebuild from the release tag",
                pin.code_hash
            ));
        }
        if c.threshold.is_some_and(|t| t != pin.threshold) || c.emergency_threshold.is_some_and(|t| t != pin.emergency_threshold) {
            return Err("release config: thresholds are 2 (normal) and 3 (emergency), docs/design/19".into());
        }
        Ok(pin)
    }

    /// Shape checks a node and `aether network` apply to any pin: hex sizes,
    /// three distinct builder keys on the P-256 curve, the 2/3 and 3/3 rule.
    /// Whether the address and code hash are the genesis's ReleaseLog is the
    /// mainnet rule "release pin" (it builds the genesis state).
    pub fn validate(&self) -> Result<(), String> {
        let hex_of = |s: &str, bytes: usize, what: &str| -> Result<Vec<u8>, String> {
            let body = s.strip_prefix("0x").ok_or_else(|| format!("release.{what}: 0x-prefixed hex"))?;
            let b = hex::decode(body).map_err(|e| format!("release.{what}: {e}"))?;
            if b.len() != bytes {
                return Err(format!("release.{what}: {bytes} bytes, got {}", b.len()));
            }
            Ok(b)
        };
        hex_of(&self.log, 20, "log")?;
        if hex_of(&self.code_hash, 32, "code_hash")?.iter().all(|b| *b == 0) {
            return Err("release.code_hash: the zero hash pins no code".into());
        }
        if self.builder_keys.len() != aether_execution::release_log::BUILDERS {
            return Err(format!(
                "release.builder_keys: exactly {} builder keys, got {}",
                aether_execution::release_log::BUILDERS,
                self.builder_keys.len()
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for (i, key) in self.builder_keys.iter().enumerate() {
            // Bare hex, as `builder-sign init` prints it and the app reads it.
            let b = hex::decode(key).map_err(|e| format!("release.builder_keys[{i}]: {e} (bare 04‖x‖y hex, no 0x)"))?;
            let valid = b.len() == 65
                && b[0] == 4
                && aether_crypto::p256_point_is_valid(
                    b[1..33].try_into().expect("32 bytes"),
                    b[33..65].try_into().expect("32 bytes"),
                );
            if !valid {
                return Err(format!(
                    "release.builder_keys[{i}]: not an uncompressed P-256 public key (04‖x‖y, on the curve; `builder-sign init` prints one)"
                ));
            }
            if !seen.insert(b) {
                return Err(format!("release.builder_keys[{i}]: the same builder key twice (three different Macs sign)"));
            }
        }
        if self.threshold != aether_execution::release_log::THRESHOLD
            || self.emergency_threshold != aether_execution::release_log::EMERGENCY_THRESHOLD
        {
            return Err(format!(
                "release: thresholds are {}/{} normal and {}/{} emergency (docs/design/19), got {} and {}",
                aether_execution::release_log::THRESHOLD,
                aether_execution::release_log::BUILDERS,
                aether_execution::release_log::EMERGENCY_THRESHOLD,
                aether_execution::release_log::BUILDERS,
                self.threshold,
                self.emergency_threshold
            ));
        }
        Ok(())
    }
}

/// The founder's reserve keys in a network file: up to three validator
/// entries on one Mac, and the founder's operator address.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReserveFile {
    pub operator: aether_types::Address,
    pub validators: Vec<Member>,
}

/// What a network file fixes about genesis beyond the chain id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Genesis {
    pub faucet: Option<aether_types::Address>,
    pub registrar: Option<([u8; 32], [u8; 32])>,
    /// Blocks per voting-node epoch (0 = the default).
    pub epoch_blocks: u64,
    pub min_streak: Option<u64>,
    pub draw_epochs: Option<u64>,
    /// History format version (0 or 1 = the original format).
    pub history: u32,
    /// Protocol rules from height 0 (1 = the testnet's genesis, upgrades turn
    /// later protocols on; 0 = `Default`, read as 1).
    pub protocol: u32,
    pub node_rewards: bool,
    /// The validators the network opened with (the file's
    /// `genesis_validators`, or its validators before the first handoff):
    /// node rewards record them in the genesis state as the first voting
    /// committee, so every node derives the same one.
    pub committee: Vec<(String, String)>,
    pub reserve: Option<crate::chain::Reserve>,
    /// The consensus group (0 today; `ChainConfig::group`).
    pub group: u16,
    /// The committee ceiling (`ChainConfig::max_committee`).
    pub max_committee: usize,
    /// The app's release-approval pin (not consensus: the wallet reads it).
    /// Carried here so the DKG's file keeps it (`carry_genesis`).
    pub release: Option<ReleasePin>,
}

/// Upper bounds a genesis may name for the voting-node epoch and the draw window
/// (blocks per epoch, epochs per draw). Mainnet runs 3600 and 24; the bounds are
/// generous, but finite so that no product of the two can overflow a u64 (an
/// audit found a genesis that passed every launch check and then crashed every
/// validator on `epoch_blocks * draw_epochs`).
pub const MAX_EPOCH_BLOCKS: u64 = 1 << 20;
pub const MAX_DRAW_EPOCHS: u64 = 1 << 10;

impl Default for Genesis {
    fn default() -> Self {
        Self {
            faucet: None,
            registrar: None,
            epoch_blocks: 0,
            min_streak: None,
            draw_epochs: None,
            history: 0,
            protocol: 0,
            node_rewards: false,
            committee: Vec::new(),
            reserve: None,
            group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
            release: None,
        }
    }
}

impl NetworkFile {
    pub fn genesis(&self) -> Result<Genesis, String> {
        let registrar = match &self.registrar {
            None => None,
            Some(h) => {
                let b = hex::decode(h.trim_start_matches("0x"))
                    .map_err(|e| format!("registrar: {e}"))?;
                let (x, y) = b
                    .split_at_checked(32)
                    .ok_or("registrar must be 64 bytes (x‖y)")?;
                let xy: ([u8; 32], [u8; 32]) = (
                    x.try_into().map_err(|_| "registrar x")?,
                    y.try_into()
                        .map_err(|_| "registrar must be 64 bytes (x‖y)")?,
                );
                // Audit 2, R2-3: the zero key (or any pair off the curve) can never
                // verify an attestation: registration would be dead from genesis.
                if !aether_crypto::p256_point_is_valid(&xy.0, &xy.1) {
                    return Err("registrar is not a valid P-256 public key (nonzero, on the curve)".into());
                }
                Some(xy)
            }
        };
        if self.epoch_blocks.unwrap_or(0) > MAX_EPOCH_BLOCKS {
            return Err(format!("epoch_blocks must be at most {MAX_EPOCH_BLOCKS}"));
        }
        if self.draw_epochs.unwrap_or(0) > MAX_DRAW_EPOCHS {
            return Err(format!("draw_epochs must be at most {MAX_DRAW_EPOCHS}"));
        }
        let max_committee = self.max_committee.unwrap_or(crate::rotation::GROW_UNTIL as u64);
        if !(4..=crate::rotation::MAX_VOTING_NODES as u64).contains(&max_committee) {
            return Err(format!("max_committee must be 4..={}", crate::rotation::MAX_VOTING_NODES));
        }
        if (self.group.unwrap_or(0) != 0 || max_committee != crate::rotation::GROW_UNTIL as u64)
            && !(self.node_rewards.unwrap_or(false) && self.history.unwrap_or(0) >= 2)
        {
            return Err("group and custom max_committee require node_rewards and history v2 at genesis".into());
        }
        if self.node_rewards.unwrap_or(false) && self.history.unwrap_or(0) >= 2
            && self.validators.len() > max_committee as usize
        {
            return Err("genesis validators exceed max_committee".into());
        }
        if self.group.unwrap_or(0) != 0 && (registrar.is_some() || self.reserve.is_some()) {
            return Err("registry and rewards reserve belong to root group 0".into());
        }
        // A genesis may name the protocol it starts under, 1 (the testnet's
        // genesis) up to the newest this binary runs; anything else is a typo,
        // not a default to round.
        if let Some(pin) = &self.release {
            if !(self.node_rewards.unwrap_or(false) && self.history.unwrap_or(0) >= 2) {
                return Err("release: only a new genesis (node rewards, history v2) has the ReleaseLog predeploy to pin".into());
            }
            pin.validate()?;
        }
        let protocol = match self.protocol {
            None => 1,
            Some(p) if (1..=crate::upgrade::PROTOCOL).contains(&p) => p,
            Some(p) => {
                return Err(format!("protocol {p}: expected 1 to {}", crate::upgrade::PROTOCOL))
            }
        };
        Ok(Genesis {
            faucet: self.faucet,
            registrar,
            epoch_blocks: self.epoch_blocks.unwrap_or(0),
            min_streak: self.min_streak,
            draw_epochs: self.draw_epochs,
            history: self.history.unwrap_or(0),
            protocol,
            node_rewards: self.node_rewards.unwrap_or(false),
            group: self.group.unwrap_or(0),
            max_committee: max_committee as usize,
            committee: self.committee()?,
            release: self.release.clone(),
            reserve: match &self.reserve {
                None => None,
                Some(r) => {
                    if self.node_rewards != Some(true) {
                        return Err("reserve keys need node rewards".into());
                    }
                    if r.validators.is_empty() || r.validators.len() > aether_rewards::MAX_RESERVE_KEYS {
                        return Err(format!("1 to {} reserve keys", aether_rewards::MAX_RESERVE_KEYS));
                    }
                    let reserve = crate::chain::Reserve {
                        operator: r.operator,
                        members: r.validators.iter().map(|m| (m.key.to_lowercase(), m.node.clone())).collect(),
                    };
                    // Every key and node id must parse, or no genesis.
                    reserve.bytes()?;
                    // No validator of this network is also a reserve key (red
                    // team, finding 3): a key that is both would sit in the
                    // committee and in the reserve, so the founder's Mac would
                    // run it either way and the overlap would hide from the
                    // independent-operator count.
                    let plain = |k: &str| k.trim_start_matches("0x").to_lowercase();
                    let same_node = |a: &str, b: &str| {
                        match (a.parse::<EndpointId>(), b.parse::<EndpointId>()) {
                            (Ok(a), Ok(b)) => a == b,
                            _ => a == b,
                        }
                    };
                    for (key, node) in &reserve.members {
                        for (i, v) in self.validators.iter().enumerate() {
                            if plain(&v.key) == plain(key) {
                                return Err(format!("validator {}: its key is also a reserve key", i + 1));
                            }
                            if same_node(&v.node, node) {
                                return Err(format!("validator {}: its node id is also a reserve key's", i + 1));
                            }
                        }
                    }
                    Some(reserve)
                }
            },
        })
    }

    /// The validators the network opened with, keys lowercased as roster
    /// words hold them: the file's `genesis_validators`, or its validators
    /// (before the first handoff rewrote them).
    pub fn committee(&self) -> Result<Vec<(String, String)>, String> {
        let source = self.genesis_validators.as_ref().unwrap_or(&self.validators);
        let mut members = Vec::with_capacity(source.len());
        for (i, m) in source.iter().enumerate() {
            let key = hex::decode(m.key.trim_start_matches("0x"))
                .map_err(|e| format!("validator {}: key: {e}", i + 1))?;
            <[u8; 32]>::try_from(key)
                .map_err(|_| format!("validator {}: key: 32-byte hex", i + 1))?;
            m.node
                .parse::<EndpointId>()
                .map_err(|e| format!("validator {}: node: {e}", i + 1))?;
            members.push((m.key.trim_start_matches("0x").to_lowercase(), m.node.clone()));
        }
        Ok(members)
    }

    /// Write every genesis fact of `g` into a file a ceremony (dkg) builds from
    /// the roster alone: the inverse of [`NetworkFile::genesis`]. A fact left out
    /// here silently reverts to its default in the written network.json (the
    /// protocol did: a rehearsal chain declared protocol 3 and ran protocol 1).
    pub fn carry_genesis(&mut self, g: &Genesis) {
        self.faucet = g.faucet;
        self.registrar = g.registrar.map(|(x, y)| format!("{}{}", hex::encode(x), hex::encode(y)));
        self.epoch_blocks = (g.epoch_blocks != 0).then_some(g.epoch_blocks);
        self.min_streak = g.min_streak;
        self.draw_epochs = g.draw_epochs;
        self.history = (g.history != 0).then_some(g.history);
        self.protocol = (g.protocol > 1).then_some(g.protocol);
        self.node_rewards = g.node_rewards.then_some(true);
        self.reserve = g.reserve.as_ref().map(|r| ReserveFile {
            operator: r.operator,
            validators: r.members.iter().map(|(key, node)| Member { key: key.clone(), node: node.clone() }).collect(),
        });
        self.group = (g.group != 0).then_some(g.group);
        self.max_committee = (g.max_committee != crate::rotation::GROW_UNTIL).then_some(g.max_committee as u64);
        // The roster the network opened with, frozen: handoffs rewrite `validators`.
        self.genesis_validators = Some(g.committee.iter().map(|(key, node)| Member { key: key.clone(), node: node.clone() }).collect());
        self.release = g.release.clone();
    }

    /// Carry genesis facts into a file written by a ceremony (dkg, reshare).
    pub fn keep_genesis(&mut self, from: &NetworkFile) {
        self.faucet = from.faucet.or(self.faucet);
        self.registrar = from.registrar.clone().or(self.registrar.take());
        self.epoch_blocks = from.epoch_blocks.or(self.epoch_blocks);
        self.min_streak = from.min_streak.or(self.min_streak);
        self.draw_epochs = from.draw_epochs.or(self.draw_epochs);
        self.history = from.history.or(self.history);
        self.protocol = from.protocol.or(self.protocol);
        self.node_rewards = from.node_rewards.or(self.node_rewards);
        self.reserve = from.reserve.clone().or(self.reserve.take());
        self.group = from.group.or(self.group);
        self.max_committee = from.max_committee.or(self.max_committee);
        self.genesis_validators = from.genesis_validators.clone().or(self.genesis_validators.take());
        self.release = from.release.clone().or(self.release.take());
    }
}

impl NetworkFile {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The validators, in network-file order (validator `i` is `keys[i - 1]`).
#[derive(Clone, Debug)]
pub struct Roster {
    pub keys: Vec<PublicKey>,
    pub nodes: Vec<EndpointId>,
}

impl Roster {
    /// Public devnet validators `1..=n`.
    pub fn devnet(n: u64) -> Self {
        Roster {
            keys: (1..=n)
                .map(|i| aether_light::devnet_validator_key(i).public_key())
                .collect(),
            nodes: (1..=n).map(aether_net::devnet_node_id).collect(),
        }
    }

    pub fn from_file(f: &NetworkFile) -> Result<Self, String> {
        let mut keys = Vec::new();
        let mut nodes = Vec::new();
        for (i, m) in f.validators.iter().enumerate() {
            let k = hex::decode(&m.key).map_err(|e| format!("validator {}: key: {e}", i + 1))?;
            keys.push(
                PublicKey::decode(k.as_slice())
                    .map_err(|e| format!("validator {}: key: {e:?}", i + 1))?,
            );
            nodes.push(
                m.node
                    .parse()
                    .map_err(|e| format!("validator {}: node: {e}", i + 1))?,
            );
        }
        let r = Roster { keys, nodes };
        r.validators_checked()?;
        Ok(r)
    }

    pub fn len(&self) -> u64 {
        self.keys.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    fn validators_checked(&self) -> Result<Set<PublicKey>, String> {
        self.keys
            .iter()
            .cloned()
            .try_collect()
            .map_err(|_| "duplicate validator key".to_string())
    }

    /// The validator set (sorted, as consensus uses it).
    pub fn validators(&self) -> Set<PublicKey> {
        self.validators_checked()
            .expect("roster checked at construction")
    }

    /// 1-based index of `key`.
    pub fn index_of(&self, key: &PublicKey) -> Option<u64> {
        self.keys
            .iter()
            .position(|k| k == key)
            .map(|p| p as u64 + 1)
    }

    pub fn key(&self, i: u64) -> &PublicKey {
        &self.keys[(i - 1) as usize]
    }

    pub fn node(&self, i: u64) -> EndpointId {
        self.nodes[(i - 1) as usize]
    }

    /// Union with `other` (self's order first, then members only in `other`).
    pub fn union(&self, other: &Roster) -> Roster {
        let mut r = self.clone();
        for (k, n) in other.keys.iter().zip(&other.nodes) {
            if !r.keys.contains(k) {
                r.keys.push(k.clone());
                r.nodes.push(*n);
            }
        }
        r
    }

    pub fn to_file(&self, chain_id: u64) -> NetworkFile {
        NetworkFile {
            chain_id,
            validators: self
                .keys
                .iter()
                .zip(&self.nodes)
                .map(|(k, n)| Member {
                    key: hex::encode(k.encode()),
                    node: n.to_string(),
                })
                .collect(),
            identity: None,
            round: 0,
            output: None,
            epochs: Vec::new(),
            faucet: None,
            registrar: None,
            epoch_blocks: None,
            min_streak: None,
            draw_epochs: None,
            history: None,
            protocol: None,
            node_rewards: None,
            reserve: None,
            group: None,
            max_committee: None,
            genesis_validators: None,
            release: None,
        }
    }
}

/// This validator's secrets: consensus signing key and iroh node key.
#[derive(Clone)]
pub struct LocalKeys {
    pub signer: ed25519::PrivateKey,
    pub node_secret: SecretKey,
}

#[derive(Serialize, Deserialize)]
struct KeyFileJson {
    consensus: String,
    node: String,
}

impl LocalKeys {
    pub fn devnet(i: u64) -> Self {
        LocalKeys {
            signer: aether_light::devnet_validator_key(i),
            node_secret: aether_net::devnet_node_secret(i),
        }
    }

    pub fn generate() -> Self {
        let seed: [u8; 32] = rand::random();
        let signer = ed25519::PrivateKey::decode(seed.as_slice()).expect("32-byte ed25519 seed");
        LocalKeys {
            signer,
            node_secret: SecretKey::from_bytes(&rand::random()),
        }
    }

    pub fn public(&self) -> Member {
        Member {
            key: hex::encode(self.signer.public_key().encode()),
            node: self.node_secret.public().to_string(),
        }
    }

    /// Load `<dir>/validator.key`.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join(KEY_FILE);
        let bytes = std::fs::read(&path).map_err(|e| {
            format!(
                "{}: {e} (run `aether keygen --data {}`)",
                path.display(),
                dir.display()
            )
        })?;
        let j: KeyFileJson =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let c = hex::decode(j.consensus).map_err(|e| e.to_string())?;
        let signer = ed25519::PrivateKey::decode(c.as_slice())
            .map_err(|e| format!("consensus key: {e:?}"))?;
        let n: [u8; 32] = hex::decode(j.node)
            .map_err(|e| e.to_string())?
            .try_into()
            .map_err(|_| "node key length".to_string())?;
        Ok(LocalKeys {
            signer,
            node_secret: SecretKey::from_bytes(&n),
        })
    }

    /// Write `<dir>/validator.key` (mode 600) and `<dir>/validator.pub.json`. Refuses to overwrite.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(KEY_FILE);
        if path.exists() {
            return Err(format!("{} exists (keys are never overwritten)", path.display()));
        }
        let j = KeyFileJson {
            consensus: hex::encode(self.signer.encode()),
            node: hex::encode(self.node_secret.to_bytes()),
        };
        // Atomic replacement (red team #5): a crash or a full disk mid-write
        // leaves no truncated key file a later start would treat as lost.
        crate::atomic::create(&path, &serde_json::to_vec_pretty(&j).expect("json"), 0o600)?;
        crate::atomic::replace(
            &dir.join(PUBLIC_FILE),
            &serde_json::to_vec_pretty(&self.public()).expect("json"),
            0o644,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(json: &str) -> NetworkFile {
        serde_json::from_str(json).expect("network file")
    }

    /// A real P-256 key as the registrar's x‖y hex.
    fn registrar_hex() -> String {
        use aether_crypto::Signer;
        let key = aether_crypto::P256Signer::from_seed(&[5; 32]).unwrap().public_key();
        let (x, y) = aether_crypto::p256_xy(&key.bytes).unwrap();
        format!("{}{}", hex::encode(x), hex::encode(y))
    }

    #[test]
    fn the_registrar_must_be_a_real_p256_key() {
        // Audit 2, R2-3: 64 zero bytes parsed fine and the launch check said ok.
        for bad in ["00".repeat(64), "ab".repeat(64)] {
            let msg = format!(r#"{{"chain_id":1,"validators":[],"node_rewards":true,"history":2,"registrar":"{bad}"}}"#);
            assert!(file(&msg).genesis().is_err(), "{bad}");
        }
        let ok = format!(r#"{{"chain_id":1,"validators":[],"node_rewards":true,"history":2,"registrar":"{}"}}"#, registrar_hex());
        assert!(file(&ok).genesis().is_ok());
    }

    #[test]
    fn the_protocol_field_names_the_genesis_rules() {
        // 7780 and older files have no field: protocol 1, upgrades turn the
        // later protocols on, and the file serializes back without the field.
        let old = file(r#"{"chain_id":7780,"validators":[]}"#);
        assert_eq!(old.genesis().unwrap().protocol, 1);
        assert!(!serde_json::to_string(&old).unwrap().contains("protocol"));
        // A new network names the protocol it starts under.
        let mainnet = file(r#"{"chain_id":7799,"validators":[],"protocol":3}"#);
        assert_eq!(mainnet.genesis().unwrap().protocol, 3);
        // Anything else is refused, not rounded to a default.
        for bad in [0u32, crate::upgrade::PROTOCOL + 1, u32::MAX] {
            let msg = format!(r#"{{"chain_id":1,"validators":[],"protocol":{bad}}}"#);
            assert!(file(&msg).genesis().is_err(), "protocol {bad}");
        }
        assert!(file(r#"{"chain_id":1,"validators":[],"protocol":1}"#).genesis().is_ok());
    }

    /// What `aether dkg` does: build a file from the roster alone and carry the
    /// genesis into it. The written file must describe the SAME genesis.
    fn dkg_roundtrip(json: &str) {
        let from = file(json);
        let genesis = from.genesis().expect("genesis");
        let mut written = file(&format!(r#"{{"chain_id":{},"validators":[]}}"#, from.chain_id));
        written.carry_genesis(&genesis);
        assert_eq!(written.genesis().expect("written genesis"), genesis, "{json}");
    }

    #[test]
    fn a_dkg_written_network_file_keeps_the_whole_genesis() {
        // The mainnet rehearsal shape: protocol 3, history v2, node rewards, a registrar.
        dkg_roundtrip(&format!(
            r#"{{"chain_id":7799,"validators":[],"protocol":3,"history":2,"node_rewards":true,"epoch_blocks":40,"min_streak":0,"draw_epochs":5,"registrar":"{}"}}"#,
            registrar_hex()
        ));
        // A group and a custom committee ceiling (new genesis only).
        dkg_roundtrip(r#"{"chain_id":7800,"validators":[],"protocol":3,"history":2,"node_rewards":true,"group":2,"max_committee":12}"#);
        // The testnet shape stays field-free (protocol 1, no history, no rewards).
        let old = file(r#"{"chain_id":7780,"validators":[]}"#);
        let mut written = file(r#"{"chain_id":7780,"validators":[]}"#);
        written.carry_genesis(&old.genesis().unwrap());
        assert!(written.protocol.is_none() && written.history.is_none() && written.node_rewards.is_none());
    }

    #[test]
    fn the_epoch_parameters_are_bounded_so_their_product_cannot_overflow() {
        // Audit 1, A2: 2^63 blocks an epoch and a draw every 2 epochs wrapped to
        // zero in release builds and crashed every validator at startup.
        for bad in [
            r#"{"chain_id":1,"validators":[],"epoch_blocks":9223372036854775808,"draw_epochs":2}"#,
            r#"{"chain_id":1,"validators":[],"epoch_blocks":18446744073709551615}"#,
            r#"{"chain_id":1,"validators":[],"draw_epochs":18446744073709551615}"#,
        ] {
            assert!(file(bad).genesis().is_err(), "{bad}");
        }
        let edge = format!(r#"{{"chain_id":1,"validators":[],"epoch_blocks":{MAX_EPOCH_BLOCKS},"draw_epochs":{MAX_DRAW_EPOCHS}}}"#);
        let g = file(&edge).genesis().expect("the bounds themselves are fine");
        assert!(g.epoch_blocks.checked_mul(g.draw_epochs.unwrap()).is_some());
        // Mainnet defaults and the rehearsal shape stay valid.
        assert!(file(r#"{"chain_id":1,"validators":[],"epoch_blocks":3600,"draw_epochs":24}"#).genesis().is_ok());
        assert!(file(r#"{"chain_id":1,"validators":[],"epoch_blocks":40}"#).genesis().is_ok());
    }

    #[test]
    fn ceremonies_carry_the_genesis_protocol() {
        // dkg/reshare write a new network.json: the genesis facts, the
        // protocol included, must survive the ceremony.
        let from = file(r#"{"chain_id":7799,"validators":[],"protocol":3,"history":2}"#);
        let mut written = file(r#"{"chain_id":7799,"validators":[]}"#);
        written.keep_genesis(&from);
        assert_eq!(written.protocol, Some(3));
        assert_eq!(written.genesis().unwrap().protocol, 3);
        assert_eq!(written.genesis().unwrap().history, 2);
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;

    fn file(max_committee: Option<u64>, group: Option<u16>) -> NetworkFile {
        let k = LocalKeys::devnet(1);
        NetworkFile {
            chain_id: 7_799,
            validators: vec![Member { key: hex::encode(k.signer.public_key().encode()), node: k.node_secret.public().to_string() }],
            identity: None,
            round: 0,
            output: None,
            epochs: Vec::new(),
            faucet: None,
            registrar: None,
            epoch_blocks: None,
            min_streak: None,
            draw_epochs: None,
            history: Some(2),
            protocol: None,
            node_rewards: Some(true),
            reserve: None,
            group,
            max_committee,
            genesis_validators: None,
            release: None,
        }
    }

    /// The genesis group and committee ceiling: absent keeps group 0 and
    /// `GROW_UNTIL` (every network today), and the ceiling must fit 4..=128.
    #[test]
    fn the_genesis_group_and_committee_ceiling_default_and_bounds() {
        assert_eq!((Genesis::default().group, Genesis::default().max_committee), (0, crate::rotation::GROW_UNTIL));
        let g = file(None, None).genesis().unwrap();
        assert_eq!((g.group, g.max_committee), (0, crate::rotation::GROW_UNTIL));
        let mut legacy = file(None, None);
        legacy.history = None;
        legacy.node_rewards = None;
        assert_eq!((legacy.genesis().unwrap().group, legacy.genesis().unwrap().max_committee), (0, crate::rotation::GROW_UNTIL));
        legacy.group = Some(1);
        assert!(legacy.genesis().is_err());
        legacy.group = None;
        legacy.max_committee = Some(8);
        assert!(legacy.genesis().is_err());
        let g = file(Some(8), Some(1)).genesis().unwrap();
        assert_eq!((g.group, g.max_committee), (1, 8));
        let mut nonroot = file(None, Some(1));
        nonroot.registrar = Some({
            use aether_crypto::Signer;
            let key = aether_crypto::P256Signer::from_seed(&[5; 32]).unwrap().public_key();
            let (x, y) = aether_crypto::p256_xy(&key.bytes).unwrap();
            format!("{}{}", hex::encode(x), hex::encode(y))
        });
        assert!(nonroot.genesis().unwrap_err().contains("root group 0"));
        assert_eq!(file(Some(4), None).genesis().unwrap().max_committee, 4);
        let mut too_many = file(Some(4), None);
        too_many.validators = vec![too_many.validators[0].clone(); 5];
        assert!(too_many.genesis().unwrap_err().contains("exceed max_committee"));
        assert_eq!(file(Some(crate::rotation::MAX_VOTING_NODES as u64), None).genesis().unwrap().max_committee, crate::rotation::MAX_VOTING_NODES);
        for bad in [0, 3, 129, u64::MAX] {
            let err = file(Some(bad), None).genesis().unwrap_err();
            assert!(err.contains("max_committee") && err.contains('4'), "{bad}: {err}");
        }
    }
}
