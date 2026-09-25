//! Light client (docs/design/09-wallet.md): trust only the validator set.
//!
//! Consensus certificates are BLS12-381 threshold signatures (design D8): a
//! quorum of validators jointly produces ONE signature under the committee's
//! group public key, so a client needs only that key (the "identity") and
//! verifies one pairing per block, whatever the number of validators.
//!
//! Anchoring (order-first, design D3): the state root after block H is carried
//! as `parent_state_root` in block H+1. A client therefore
//!   1. verifies the finalization certificate of block H+1 against the
//!      committee identity,
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
use commonware_consensus::simplex::{
    elector::{Random, RandomVersion},
    scheme::bls12381_threshold::vrf,
    types::Finalization,
};
use commonware_consensus::Heightable;
use commonware_cryptography::bls12381::dkg::feldman_desmedt::deal;
use commonware_cryptography::bls12381::primitives::group::Share;
use commonware_cryptography::bls12381::primitives::sharing::{Mode, Sharing};
use commonware_cryptography::bls12381::primitives::variant::{MinSig, Variant};
use commonware_cryptography::{ed25519, sha256::Digest, Digestible, Signer as _};
use commonware_parallel::Sequential;
use commonware_utils::{ordered::Set, union, N3f1, TryCollect};
use rand_core::SeedableRng;

/// Network namespace; must match the validators' configuration.
pub const NAMESPACE: &[u8] = b"_AETHER_DEVNET_V1";
/// Upper bound on an encoded block (txs + BAL).
pub const MAX_BLOCK_BYTES: u32 = 8 * 1024 * 1024;

pub fn consensus_namespace() -> Vec<u8> {
    union(NAMESPACE, b"_CONSENSUS")
}

/// Consensus signing scheme: BLS12-381 threshold (signatures in G1, 48 bytes)
/// with a per-round threshold VRF seed.
pub type Scheme = vrf::Scheme<PublicKey, MinSig>;
/// The committee's group public key; constant across reshares.
pub type Identity = <MinSig as Variant>::Public;
/// Leader election from the previous round's VRF seed (unpredictable leaders).
pub type Elector = Random<commonware_cryptography::Sha256>;
pub const ELECTOR: Elector = Random::new(RandomVersion::V1);

/// Deterministic devnet validator key `i` (1-based). Public knowledge; devnet only.
pub fn devnet_validator_key(i: u64) -> ed25519::PrivateKey {
    ed25519::PrivateKey::from_seed(i)
}

/// Devnet threshold keys for validators `1..=n`, dealt from a fixed seed: the
/// public polynomial and each validator's share, in participant order.
/// Devnet only (the dealer knows every share); a real network runs a DKG.
pub fn devnet_threshold(n: u64) -> (Set<PublicKey>, Sharing<MinSig>, Vec<(PublicKey, Share)>) {
    let participants: Set<PublicKey> = (1..=n).map(|i| devnet_validator_key(i).public_key()).try_collect().expect("unique devnet keys");
    let mut seed = [0u8; 32];
    seed[..24].copy_from_slice(b"aether-devnet-threshold-");
    seed[24..].copy_from_slice(&n.to_be_bytes());
    let rng = rand_chacha::ChaCha20Rng::from_seed(seed);
    let (output, shares) = deal::<MinSig, PublicKey, N3f1>(rng, Mode::NonZeroCounter, participants.clone()).expect("deal devnet shares");
    let shares = shares.iter_pairs().map(|(pk, share)| (pk.clone(), share.clone())).collect();
    (participants, output.public().clone(), shares)
}

/// Devnet committee identity (group public key) for `n` validators.
pub fn devnet_identity(n: u64) -> Identity {
    *devnet_threshold(n).1.public()
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

/// The committee a client trusts: just its group public key.
#[derive(Clone)]
pub struct ValidatorSet {
    scheme: Scheme,
    identity: Identity,
}

impl ValidatorSet {
    pub fn new(identity: Identity) -> Self {
        ValidatorSet { scheme: Scheme::certificate_verifier(&consensus_namespace(), identity), identity }
    }

    pub fn devnet(n: u64) -> Self {
        Self::new(devnet_identity(n))
    }

    /// From a committee identity in hex (as printed by `aether dkg`).
    pub fn from_hex(identity: &str) -> Result<Self, LightError> {
        use commonware_codec::DecodeExt;
        let bytes = from_hex(identity)?;
        let id = Identity::decode(bytes.as_slice()).map_err(|_| LightError::BadEncoding("identity"))?;
        Ok(Self::new(id))
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn identity_hex(&self) -> String {
        use commonware_codec::Encode;
        to_hex(&self.identity.encode())
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
    // Threshold certificates have a fixed size: the codec needs no config.
    let fin = Finalization::<Scheme, Digest>::decode_cfg(finalization_bytes, &()).map_err(|_| LightError::BadEncoding("finalization"))?;
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
