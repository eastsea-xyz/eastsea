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
        Roster { keys: (1..=n).map(|i| aether_light::devnet_validator_key(i).public_key()).collect(), nodes: (1..=n).map(aether_net::devnet_node_id).collect() }
    }

    pub fn from_file(f: &NetworkFile) -> Result<Self, String> {
        let mut keys = Vec::new();
        let mut nodes = Vec::new();
        for (i, m) in f.validators.iter().enumerate() {
            let k = hex::decode(&m.key).map_err(|e| format!("validator {}: key: {e}", i + 1))?;
            keys.push(PublicKey::decode(k.as_slice()).map_err(|e| format!("validator {}: key: {e:?}", i + 1))?);
            nodes.push(m.node.parse().map_err(|e| format!("validator {}: node: {e}", i + 1))?);
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
        self.keys.iter().cloned().try_collect().map_err(|_| "duplicate validator key".to_string())
    }

    /// The validator set (sorted, as consensus uses it).
    pub fn validators(&self) -> Set<PublicKey> {
        self.validators_checked().expect("roster checked at construction")
    }

    /// 1-based index of `key`.
    pub fn index_of(&self, key: &PublicKey) -> Option<u64> {
        self.keys.iter().position(|k| k == key).map(|p| p as u64 + 1)
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
            validators: self.keys.iter().zip(&self.nodes).map(|(k, n)| Member { key: hex::encode(k.encode()), node: n.to_string() }).collect(),
            identity: None,
            round: 0,
            output: None,
            epochs: Vec::new(),
            faucet: None,
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
        LocalKeys { signer: aether_light::devnet_validator_key(i), node_secret: aether_net::devnet_node_secret(i) }
    }

    pub fn generate() -> Self {
        let seed: [u8; 32] = rand::random();
        let signer = ed25519::PrivateKey::decode(seed.as_slice()).expect("32-byte ed25519 seed");
        LocalKeys { signer, node_secret: SecretKey::from_bytes(&rand::random()) }
    }

    pub fn public(&self) -> Member {
        Member { key: hex::encode(self.signer.public_key().encode()), node: self.node_secret.public().to_string() }
    }

    /// Load `<dir>/validator.key`.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join(KEY_FILE);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e} (run `aether keygen --data {}`)", path.display(), dir.display()))?;
        let j: KeyFileJson = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let c = hex::decode(j.consensus).map_err(|e| e.to_string())?;
        let signer = ed25519::PrivateKey::decode(c.as_slice()).map_err(|e| format!("consensus key: {e:?}"))?;
        let n: [u8; 32] = hex::decode(j.node).map_err(|e| e.to_string())?.try_into().map_err(|_| "node key length".to_string())?;
        Ok(LocalKeys { signer, node_secret: SecretKey::from_bytes(&n) })
    }

    /// Write `<dir>/validator.key` (mode 600) and `<dir>/validator.pub.json`. Refuses to overwrite.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(KEY_FILE);
        let j = KeyFileJson { consensus: hex::encode(self.signer.encode()), node: hex::encode(self.node_secret.to_bytes()) };
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("{}: {e} (keys are never overwritten)", path.display()))?;
        f.write_all(&serde_json::to_vec_pretty(&j).expect("json")).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(PUBLIC_FILE), serde_json::to_vec_pretty(&self.public()).expect("json")).map_err(|e| e.to_string())
    }
}
