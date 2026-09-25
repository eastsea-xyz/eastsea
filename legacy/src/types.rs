use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct Address(pub [u8; 20]);

impl Address {
    pub fn new(id: u64) -> Self {
        let mut bytes = [0u8; 20];
        bytes[12..20].copy_from_slice(&id.to_be_bytes());
        Address(bytes)
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        let clean = s.trim_start_matches("0x");
        if clean.len() != 40 {
            // Try parsing as u64 short id if shorter
            if let Ok(id) = u64::from_str_radix(clean, 16) {
                return Some(Address::new(id));
            }
            return None;
        }
        let mut bytes = [0u8; 20];
        for i in 0..20 {
            bytes[i] = u8::from_str_radix(&clean[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Address(bytes))
    }

    pub fn to_hex(&self) -> String {
        format!("0x{}", hex::encode(&self.0))
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(&self.0[16..20]))
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(&self.0[16..20]))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Hash256(pub [u8; 32]);

impl Hash256 {
    pub fn of(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&result);
        Hash256(h)
    }
}

impl fmt::Debug for Hash256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(&self.0[..4]))
    }
}

impl fmt::Display for Hash256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(&self.0[..4]))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TxPayload {
    Transfer {
        to: Address,
        amount: u64,
    },
    Swap {
        pool_id: u64,
        amount_in: u64,
        min_out: u64,
    },
    DeployContract {
        name: String,
        template: String, // "counter", "token", "vault", "custom"
        params: Vec<u64>,
    },
    CallContract {
        contract: Address,
        method: String,
        args: Vec<u64>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Transaction {
    pub id: u64,
    pub sender: Address,
    pub nonce: u64,
    pub payload: TxPayload,
    pub gas_limit: u64,
}

impl Transaction {
    pub fn hash(&self) -> Hash256 {
        let encoded = serde_json::to_vec(self).unwrap_or_default();
        Hash256::of(&encoded)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncryptedTx {
    pub id: u64,
    pub sender: Address,
    pub ciphertext: Vec<u8>,
    pub ephemeral_key: [u8; 32],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vertex {
    pub author: Address,
    pub round: u64,
    pub parents: Vec<Hash256>,
    pub transactions: Vec<EncryptedTx>,
    pub hash: Hash256,
}

impl Vertex {
    pub fn new(author: Address, round: u64, parents: Vec<Hash256>, transactions: Vec<EncryptedTx>) -> Self {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&author.0);
        bytes.extend_from_slice(&round.to_be_bytes());
        for p in &parents {
            bytes.extend_from_slice(&p.0);
        }
        for tx in &transactions {
            bytes.extend_from_slice(&tx.id.to_be_bytes());
            bytes.extend_from_slice(&tx.ciphertext);
        }
        let hash = Hash256::of(&bytes);
        Vertex {
            author,
            round,
            parents,
            transactions,
            hash,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountState {
    pub balance: u64,
    pub nonce: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractInfo {
    pub address: Address,
    pub name: String,
    pub template: String,
    pub creator: Address,
    pub created_at_round: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeIdentity {
    pub node_id: String,
    pub address: Address,
    pub name: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerInfo {
    pub node_id: String,
    pub address: Address,
    pub endpoint: String,
    pub last_seen_ms: u64,
    pub latency_ms: u64,
    pub round: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GossipMessage {
    NewTx(Transaction),
    NewVertex(Vertex),
    Ping { node_id: String, round: u64 },
    Pong { node_id: String, round: u64 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncRequest {
    pub from_round: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncResponse {
    pub latest_round: u64,
    pub vertices: Vec<Vertex>,
    pub contracts: Vec<ContractInfo>,
    pub contract_slots: Vec<(Address, u64, u64)>,
    #[serde(default)]
    pub accounts: Vec<(Address, AccountState)>,
}

pub mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

