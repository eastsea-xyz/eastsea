//! Light client (docs/design/09-wallet.md): trust only the validator set.
//!
//! Anchoring (order-first, design D3): the state root after block H is carried
//! as `parent_state_root` in block H+1. A client therefore
//!   1. verifies the finalization certificate of block H+1 against the known
//!      validator public keys (a 2f+1 quorum of signatures),
//!   2. checks the certificate commits to exactly that block's digest,
//!   3. takes `parent_state_root` from the verified block, and
//!   4. checks the state proof is for the requested key and verifies under it.
//!
//! No RPC answer is trusted on its own.

pub mod block;

use aether_hash::Poseidon2KoalaBear;
use aether_state::layout::{basic_data_key, storage_slot_key, BasicData};
use aether_state::Proof;
use aether_types::{Address, B256, U256};
use block::{Block, PublicKey};
use commonware_codec::Decode;
use commonware_consensus::simplex::{scheme::ed25519::Scheme, types::Finalization};
use commonware_consensus::Heightable;
use commonware_cryptography::{certificate::Verifier as _, ed25519, sha256::Digest, Digestible, Signer as _};
use commonware_parallel::Sequential;
use commonware_utils::{ordered::Set, union, TryCollect};

/// Network namespace; must match the validators' configuration.
pub const NAMESPACE: &[u8] = b"_AETHER_DEVNET_V1";
/// Upper bound on an encoded block (txs + BAL).
pub const MAX_BLOCK_BYTES: u32 = 8 * 1024 * 1024;

pub fn consensus_namespace() -> Vec<u8> {
    union(NAMESPACE, b"_CONSENSUS")
}

/// Deterministic devnet validator key `i` (1-based). Public knowledge; devnet only.
pub fn devnet_validator_key(i: u64) -> ed25519::PrivateKey {
    ed25519::PrivateKey::from_seed(i)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LightError {
    BadEncoding(&'static str),
    CertificateInvalid,
    CertificateForDifferentBlock,
    WrongKey,
    ProofInvalid(String),
    RootNotCommitted,
}

impl core::fmt::Display for LightError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self, f)
    }
}
impl std::error::Error for LightError {}

/// The validator set a client trusts (fixed at genesis for the devnet).
#[derive(Clone)]
pub struct ValidatorSet {
    scheme: Scheme,
    size: usize,
}

impl ValidatorSet {
    pub fn new(keys: Vec<PublicKey>) -> Result<Self, LightError> {
        let size = keys.len();
        let participants: Set<PublicKey> = keys.into_iter().try_collect().map_err(|_| LightError::BadEncoding("duplicate validator key"))?;
        Ok(ValidatorSet { scheme: Scheme::verifier(&consensus_namespace(), participants), size })
    }

    pub fn devnet(n: u64) -> Self {
        Self::new((1..=n).map(|i| devnet_validator_key(i).public_key()).collect()).expect("unique devnet keys")
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

/// A block whose finality was proven by a validator quorum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedBlock {
    pub height: u64,
    pub digest: String,
    pub timestamp_ms: u64,
    /// State root after executing the parent (height - 1).
    pub parent_state_root: B256,
}

/// Verify `finalization` (codec bytes) certifies `block` (codec bytes).
pub fn verify_finalized(set: &ValidatorSet, block_bytes: &[u8], finalization_bytes: &[u8]) -> Result<VerifiedBlock, LightError> {
    let block = Block::decode_cfg(block_bytes, &Block::codec_config(MAX_BLOCK_BYTES)).map_err(|_| LightError::BadEncoding("block"))?;
    let cfg = set.scheme.certificate_codec_config();
    let fin = Finalization::<Scheme, Digest>::decode_cfg(finalization_bytes, &cfg).map_err(|_| LightError::BadEncoding("finalization"))?;
    if !fin.verify(&mut commonware_utils::sys_rng(), &set.scheme, &Sequential) {
        return Err(LightError::CertificateInvalid);
    }
    if fin.proposal.payload != block.digest() {
        return Err(LightError::CertificateForDifferentBlock);
    }
    let payload = block.payload().ok_or(LightError::BadEncoding("payload"))?;
    Ok(VerifiedBlock {
        height: block.height().get(),
        digest: format!("{}", block.digest()),
        timestamp_ms: block.timestamp,
        parent_state_root: payload.parent_state_root,
    })
}

fn check_proof(proof: &Proof, expected_key: [u8; 32], root: &B256) -> Result<(), LightError> {
    if proof.key != expected_key {
        return Err(LightError::WrongKey);
    }
    proof.verify(&Poseidon2KoalaBear::new(), &root.0).map_err(|e| LightError::ProofInvalid(format!("{e:?}")))
}

/// Account state proven against a certified root. `None` = provably absent.
pub fn verify_account(anchor: &VerifiedBlock, address: &Address, proof: &Proof) -> Result<Option<BasicData>, LightError> {
    let h = Poseidon2KoalaBear::new();
    check_proof(proof, basic_data_key(&h, address), &anchor.parent_state_root)?;
    Ok(proof.value.map(|v| BasicData::decode(&v)))
}

/// Storage slot proven against a certified root.
pub fn verify_storage(anchor: &VerifiedBlock, address: &Address, slot: U256, proof: &Proof) -> Result<U256, LightError> {
    let h = Poseidon2KoalaBear::new();
    check_proof(proof, storage_slot_key(&h, address, slot), &anchor.parent_state_root)?;
    Ok(proof.value.map(U256::from_be_bytes).unwrap_or_default())
}

/// Hex helpers for transporting codec bytes over JSON.
pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn from_hex(s: &str) -> Result<Vec<u8>, LightError> {
    let s = s.trim_start_matches("0x");
    if !s.len().is_multiple_of(2) {
        return Err(LightError::BadEncoding("hex"));
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| LightError::BadEncoding("hex"))).collect()
}

