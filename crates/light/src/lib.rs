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
pub use aether_execution::receipt::ReceiptProof;

use aether_hash::ChainHasher;
use aether_state::layout::{basic_data_key, code_hash_key, storage_slot_key, BasicData};
use aether_state::Proof;
use aether_types::{Address, B256, U256};
use block::{Block, PublicKey};
use commonware_codec::{Decode, DecodeExt};
use commonware_consensus::simplex::{
    elector::{Random, RandomVersion},
    scheme::bls12381_threshold::vrf,
    types::Finalization,
};
use commonware_consensus::Heightable;
use commonware_cryptography::bls12381::dkg::feldman_desmedt::deal;
use commonware_cryptography::bls12381::primitives::group::Share;
use commonware_cryptography::bls12381::primitives::ops;
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
    consensus_namespace_of(0)
}

/// The namespace group `group`'s finalization certificates verify under: a
/// certificate is a threshold signature over the namespace, so one group's
/// proof of finality can never pass as another's. Group 0 — every network
/// today — keeps the namespace unchanged.
pub fn consensus_namespace_of(group: u16) -> Vec<u8> {
    let ns = union(NAMESPACE, b"_CONSENSUS");
    if group == 0 {
        ns
    } else {
        union(&ns, &group.to_be_bytes())
    }
}

/// Consensus signing scheme: BLS12-381 threshold (signatures in G1, 48 bytes)
/// with a per-round threshold VRF seed.
pub type Scheme = vrf::Scheme<PublicKey, MinSig>;
/// The committee's group public key; constant across reshares.
pub type Identity = <MinSig as Variant>::Public;
pub const UPGRADE_NAMESPACE: &[u8] = b"aether-upgrade-v1";

/// Check a committee-signed upgrade without trusting the node that served it.
pub fn verify_upgrade(identity: &Identity, signed: &block::SignedUpgrade) -> Result<(), String> {
    let bytes = from_hex(&signed.signature).map_err(|e| format!("signature: {e}"))?;
    let signature = <MinSig as Variant>::Signature::decode(bytes.as_slice()).map_err(|e| format!("signature: {e:?}"))?;
    let message = serde_json::to_vec(&signed.upgrade).map_err(|e| e.to_string())?;
    ops::verify_message::<MinSig>(identity, UPGRADE_NAMESPACE, &message, &signature)
        .map_err(|_| "the committee did not sign this upgrade".to_string())
}
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
    /// A block does not build on the one before it in a certified chain.
    BrokenLink,
    /// A block of another consensus group: a group's client verifies only its
    /// own group's chain.
    WrongGroup,
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
    /// The consensus group whose blocks this client verifies (0 today).
    group: u16,
}

impl ValidatorSet {
    pub fn new(identity: Identity) -> Self {
        Self::for_group(identity, 0)
    }

    /// A client of group `group`'s chain: certificates verify under the
    /// group's namespace and every block must carry the group.
    pub fn for_group(identity: Identity, group: u16) -> Self {
        ValidatorSet { scheme: Scheme::certificate_verifier(&consensus_namespace_of(group), identity), identity, group }
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

    /// The same client, verifying another group's chain.
    pub fn with_group(mut self, group: u16) -> Self {
        self.scheme = Scheme::certificate_verifier(&consensus_namespace_of(group), self.identity);
        self.group = group;
        self
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// The consensus group this client verifies (0 today).
    pub fn group(&self) -> u16 {
        self.group
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
    /// Receipt root from this block, when the protocol commits it.
    pub receipts_root: Option<B256>,
    /// MMR root of blocks 0..height (aether_state::mmr): inclusion proofs of any earlier block.
    pub history_root: B256,
}

/// Verify `finalization` (codec bytes) certifies `block` (codec bytes).
pub fn verify_finalized(set: &ValidatorSet, block_bytes: &[u8], finalization_bytes: &[u8]) -> Result<VerifiedBlock, LightError> {
    verify_finalized_chain(set, block_bytes, finalization_bytes, &[])
}

/// Most blocks a certificate may reach back through (`links`).
pub const MAX_LINKS: usize = 64;

/// Not every height carries its own certificate: finalizing a block finalizes
/// its ancestors. Verify `block` through `links` (its descendants, in height
/// order, each building on the previous) up to the last one, which
/// `finalization` certifies. With no links, the certificate is `block`'s own.
pub fn verify_finalized_chain(set: &ValidatorSet, block_bytes: &[u8], finalization_bytes: &[u8], links: &[Vec<u8>]) -> Result<VerifiedBlock, LightError> {
    if links.len() > MAX_LINKS {
        return Err(LightError::BadEncoding("too many links"));
    }
    let decode = |b: &[u8]| Block::decode_cfg(b, &Block::codec_config(MAX_BLOCK_BYTES)).map_err(|_| LightError::BadEncoding("block"));
    let block = decode(block_bytes)?;
    let check_group = |b: &Block| match b.payload() {
        Some(payload) if payload.group == set.group => Ok(()),
        Some(_) => Err(LightError::WrongGroup),
        None => Err(LightError::BadEncoding("payload")),
    };
    check_group(&block)?;
    let mut tip = block.clone();
    for l in links {
        let next = decode(l)?;
        if next.parent != tip.digest() || next.height.get() != tip.height.get() + 1 {
            return Err(LightError::BrokenLink);
        }
        check_group(&next)?;
        tip = next;
    }
    // Threshold certificates have a fixed size: the codec needs no config.
    let fin = Finalization::<Scheme, Digest>::decode_cfg(finalization_bytes, &()).map_err(|_| LightError::BadEncoding("finalization"))?;
    if !fin.verify(&mut commonware_utils::sys_rng(), &set.scheme, &Sequential) {
        return Err(LightError::CertificateInvalid);
    }
    if fin.proposal.payload != tip.digest() {
        return Err(LightError::CertificateForDifferentBlock);
    }
    let payload = block.payload().ok_or(LightError::BadEncoding("payload"))?;
    Ok(VerifiedBlock {
        height: block.height().get(),
        digest: format!("{}", block.digest()),
        timestamp_ms: block.timestamp,
        parent_state_root: payload.parent_state_root,
        receipts_root: payload.receipts_root,
        history_root: payload.history_root,
    })
}

fn check_proof(proof: &Proof, expected_key: [u8; 32], root: &B256) -> Result<(), LightError> {
    if proof.key != expected_key {
        return Err(LightError::WrongKey);
    }
    proof.verify(&ChainHasher::new(), &root.0).map_err(|e| LightError::ProofInvalid(format!("{e:?}")))
}

/// Account state proven against a certified root. `None` = provably absent.
pub fn verify_account(anchor: &VerifiedBlock, address: &Address, proof: &Proof) -> Result<Option<BasicData>, LightError> {
    let h = ChainHasher::new();
    check_proof(proof, basic_data_key(&h, address), &anchor.parent_state_root)?;
    Ok(proof.value.map(|v| BasicData::decode(&v)))
}

/// A transaction receipt proven against the root committed by its certified block.
/// Legacy certified blocks without a receipt commitment cannot prove receipts.
pub fn verify_receipt(
    certified_block: &VerifiedBlock,
    index: usize,
    receipt: &aether_execution::Receipt,
    proof: &ReceiptProof,
) -> Result<(), LightError> {
    let root = certified_block.receipts_root.ok_or(LightError::RootNotCommitted)?;
    if aether_execution::receipt::verify_receipt(root, index, receipt, proof) {
        Ok(())
    } else {
        Err(LightError::ProofInvalid("receipt inclusion".into()))
    }
}

/// Storage slot proven against a certified root.
pub fn verify_storage(anchor: &VerifiedBlock, address: &Address, slot: U256, proof: &Proof) -> Result<U256, LightError> {
    let h = ChainHasher::new();
    check_proof(proof, storage_slot_key(&h, address, slot), &anchor.parent_state_root)?;
    Ok(proof.value.map(U256::from_be_bytes).unwrap_or_default())
}

/// Contract runtime code hash proven against a certified root.
pub fn verify_code_hash(anchor: &VerifiedBlock, address: &Address, proof: &Proof) -> Result<Option<B256>, LightError> {
    let h = ChainHasher::new();
    check_proof(proof, code_hash_key(&h, address), &anchor.parent_state_root)?;
    Ok(proof.value.map(B256::from))
}

/// Block `height` with hash `block_hash` is in the history certified by
/// `anchor` (whose payload commits to the MMR of blocks 0..anchor.height).
pub fn verify_history(anchor: &VerifiedBlock, height: u64, block_hash: &B256, proof: &aether_state::mmr::MmrProof) -> Result<(), LightError> {
    let h = ChainHasher::new();
    if proof.leaves != anchor.height || proof.index != height {
        return Err(LightError::WrongKey);
    }
    let leaf = aether_state::mmr::leaf(&h, height, &block_hash.0);
    if proof.verify(&h, &leaf, &anchor.history_root.0) {
        Ok(())
    } else {
        Err(LightError::ProofInvalid("history inclusion".into()))
    }
}

/// An old block's full contents (codec bytes, from any untrusted archive or
/// era file) verified through `anchor`'s history root: its hash is recomputed
/// from the bytes, so the returned payload is exactly what was certified.
pub fn verify_old_block(anchor: &VerifiedBlock, block_bytes: &[u8], proof: &aether_state::mmr::MmrProof) -> Result<(VerifiedBlock, block::Payload), LightError> {
    let b = Block::decode_cfg(block_bytes, &Block::codec_config(MAX_BLOCK_BYTES)).map_err(|_| LightError::BadEncoding("block"))?;
    let digest: [u8; 32] = b.digest().as_ref().try_into().map_err(|_| LightError::BadEncoding("digest"))?;
    let height = b.height().get();
    verify_history(anchor, height, &B256::from(digest), proof)?;
    let payload = b.payload().ok_or(LightError::BadEncoding("payload"))?;
    let v = VerifiedBlock {
        height,
        digest: format!("{}", b.digest()),
        timestamp_ms: b.timestamp,
        parent_state_root: payload.parent_state_root,
        receipts_root: payload.receipts_root,
        history_root: payload.history_root,
    };
    Ok((v, payload))
}

/// The root of era `era` (blocks `era * ERA_LEN ..`, `aether_state::mmr`) is in
/// the history certified by `anchor`: every block of an era file hashing to
/// this root is then certified too.
pub fn verify_era_root(anchor: &VerifiedBlock, era: u64, root: &[u8; 32], proof: &aether_state::mmr::MmrProof) -> Result<(), LightError> {
    use aether_state::mmr::{ERA_BITS, ERA_LEN};
    let first = era.checked_mul(ERA_LEN).ok_or(LightError::WrongKey)?;
    if proof.leaves != anchor.height || proof.index != first || first + ERA_LEN > anchor.height {
        return Err(LightError::WrongKey);
    }
    if proof.verify_node(&ChainHasher::new(), root, ERA_BITS, &anchor.history_root.0) {
        Ok(())
    } else {
        Err(LightError::ProofInvalid("era root".into()))
    }
}

/// Hex helpers for transporting codec bytes over JSON.
pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn from_hex(s: &str) -> Result<Vec<u8>, LightError> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    // Untrusted input: work on bytes, so non-ASCII text is an error, never a panic.
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return Err(LightError::BadEncoding("hex"));
    }
    let nibble = |c: u8| (c as char).to_digit(16).map(|d| d as u8).ok_or(LightError::BadEncoding("hex"));
    b.as_chunks::<2>().0.iter().map(|[hi, lo]| Ok(nibble(*hi)? << 4 | nibble(*lo)?)).collect()
}

#[cfg(test)]
mod hex_tests {
    use super::*;

    #[test]
    fn hex_rejects_non_ascii_without_panicking() {
        assert!(from_hex("€a").is_err());
        assert!(from_hex("0xzz").is_err());
        assert_eq!(from_hex("0x0aFf").unwrap(), vec![0x0a, 0xff]);
    }
}
