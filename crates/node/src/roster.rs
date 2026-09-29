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
    /// The validators this network opened with. `validators` names the running
    /// committee (each handoff rewrites it), but a node re-syncing from
    /// genesis — and every node deriving the genesis rewards words — needs the
    /// very first roster however many committees came and went, so `aether
    /// network` freezes it here and ceremonies carry it on (`keep_genesis`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genesis_validators: Option<Vec<Member>>,
}

/// The founder's reserve keys in a network file: up to three validator
/// entries on one Mac, and the founder's operator address.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReserveFile {
    pub operator: aether_types::Address,
    pub validators: Vec<Member>,
}

/// What a network file fixes about genesis beyond the chain id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
                Some((
                    x.try_into().map_err(|_| "registrar x")?,
                    y.try_into()
                        .map_err(|_| "registrar must be 64 bytes (x‖y)")?,
                ))
            }
        };
        // A genesis may name the protocol it starts under, 1 (the testnet's
        // genesis) up to the newest this binary runs; anything else is a typo,
        // not a default to round.
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
            committee: self.committee()?,
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
        self.genesis_validators = from.genesis_validators.clone().or(self.genesis_validators.take());
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
            genesis_validators: None,
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
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(KEY_FILE);
        let j = KeyFileJson {
            consensus: hex::encode(self.signer.encode()),
            node: hex::encode(self.node_secret.to_bytes()),
        };
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("{}: {e} (keys are never overwritten)", path.display()))?;
        f.write_all(&serde_json::to_vec_pretty(&j).expect("json"))
            .map_err(|e| e.to_string())?;
        std::fs::write(
            dir.join(PUBLIC_FILE),
            serde_json::to_vec_pretty(&self.public()).expect("json"),
        )
        .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(json: &str) -> NetworkFile {
        serde_json::from_str(json).expect("network file")
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
