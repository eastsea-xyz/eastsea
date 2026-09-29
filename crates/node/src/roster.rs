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
    pub node_rewards: bool,
    pub reserve: Option<crate::chain::Reserve>,
    /// The consensus group (0 today; `ChainConfig::group`).
    pub group: u16,
    /// The committee ceiling (`ChainConfig::max_committee`).
    pub max_committee: usize,
}

impl Default for Genesis {
    fn default() -> Self {
        Self {
            faucet: None,
            registrar: None,
            epoch_blocks: 0,
            min_streak: None,
            draw_epochs: None,
            history: 0,
            node_rewards: false,
            reserve: None,
            group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
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
                Some((
                    x.try_into().map_err(|_| "registrar x")?,
                    y.try_into()
                        .map_err(|_| "registrar must be 64 bytes (x‖y)")?,
                ))
            }
        };
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
        Ok(Genesis {
            faucet: self.faucet,
            registrar,
            epoch_blocks: self.epoch_blocks.unwrap_or(0),
            min_streak: self.min_streak,
            draw_epochs: self.draw_epochs,
            history: self.history.unwrap_or(0),
            node_rewards: self.node_rewards.unwrap_or(false),
            group: self.group.unwrap_or(0),
            max_committee: max_committee as usize,
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
                    Some(reserve)
                }
            },
        })
    }

    /// Carry genesis facts into a file written by a ceremony (dkg, reshare).
    pub fn keep_genesis(&mut self, from: &NetworkFile) {
        self.faucet = from.faucet.or(self.faucet);
        self.registrar = from.registrar.clone().or(self.registrar.take());
        self.epoch_blocks = from.epoch_blocks.or(self.epoch_blocks);
        self.min_streak = from.min_streak.or(self.min_streak);
        self.draw_epochs = from.draw_epochs.or(self.draw_epochs);
        self.history = from.history.or(self.history);
        self.node_rewards = from.node_rewards.or(self.node_rewards);
        self.reserve = from.reserve.clone().or(self.reserve.take());
        self.group = from.group.or(self.group);
        self.max_committee = from.max_committee.or(self.max_committee);
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
            node_rewards: None,
            reserve: None,
            group: None,
            max_committee: None,
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
            node_rewards: Some(true),
            reserve: None,
            group,
            max_committee,
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
        nonroot.registrar = Some("11".repeat(64));
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
