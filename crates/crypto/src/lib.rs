//! Signers and signature verification (docs/design/03-traits.md, 09-wallet.md).
//!
//! Digest rules match the platforms that produce the signatures:
//! - P-256: ECDSA over SHA-256(msg), low-s only. This is exactly what Secure
//!   Enclave `signature(for: data)` produces and what P256VERIFY (EIP-7951) checks.
//! - secp256k1: Ethereum style, keccak256(msg), 65-byte r‖s‖v, low-s only.
//! - Ed25519: signs the message itself, strict verification.
//!
//! Secret keys are created from caller-supplied 32-byte seeds so this crate has
//! no RNG policy of its own; the Secure Enclave signer lives on the Swift side.

use aether_types::{Address, SignerScheme};
use alloy_primitives::keccak256;
use p256::ecdsa::signature::hazmat::{PrehashSigner, PrehashVerifier};
use sha2::{Digest as _, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CryptoError {
    InvalidSecret,
    InvalidPublicKey,
    InvalidSignature,
    HighS,
    Mismatch,
}

impl core::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for CryptoError {}

/// Encoded public key: P-256 and secp256k1 as 33-byte compressed SEC1, Ed25519 as 32 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicKey {
    pub scheme: SignerScheme,
    pub bytes: Vec<u8>,
}

pub trait Signer {
    fn scheme(&self) -> SignerScheme;
    fn public_key(&self) -> PublicKey;
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, CryptoError>;
}

/// Verify `sig` over `msg` for `pk`. Rejects malleable (high-s) ECDSA signatures.
pub fn verify(pk: &PublicKey, msg: &[u8], sig: &[u8]) -> Result<(), CryptoError> {
    match pk.scheme {
        SignerScheme::P256 => {
            let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(&pk.bytes).map_err(|_| CryptoError::InvalidPublicKey)?;
            let s = p256::ecdsa::Signature::from_slice(sig).map_err(|_| CryptoError::InvalidSignature)?;
            if s.normalize_s() != s {
                return Err(CryptoError::HighS);
            }
            vk.verify_prehash(&Sha256::digest(msg), &s).map_err(|_| CryptoError::Mismatch)
        }
        SignerScheme::Secp256k1 => {
            let recovered = recover_secp256k1(msg, sig)?;
            if recovered.bytes == pk.bytes {
                Ok(())
            } else {
                Err(CryptoError::Mismatch)
            }
        }
        SignerScheme::Ed25519 => {
            let bytes: [u8; 32] = pk.bytes.as_slice().try_into().map_err(|_| CryptoError::InvalidPublicKey)?;
            let vk = ed25519_dalek::VerifyingKey::from_bytes(&bytes).map_err(|_| CryptoError::InvalidPublicKey)?;
            let sig: [u8; 64] = sig.try_into().map_err(|_| CryptoError::InvalidSignature)?;
            vk.verify_strict(msg, &ed25519_dalek::Signature::from_bytes(&sig)).map_err(|_| CryptoError::Mismatch)
        }
    }
}

/// Recover the secp256k1 signer of `msg` from a 65-byte r‖s‖v signature.
pub fn recover_secp256k1(msg: &[u8], sig: &[u8]) -> Result<PublicKey, CryptoError> {
    if sig.len() != 65 {
        return Err(CryptoError::InvalidSignature);
    }
    let s = k256::ecdsa::Signature::from_slice(&sig[..64]).map_err(|_| CryptoError::InvalidSignature)?;
    if s.normalize_s() != s {
        return Err(CryptoError::HighS);
    }
    let v = if sig[64] >= 27 { sig[64] - 27 } else { sig[64] };
    let rid = k256::ecdsa::RecoveryId::from_byte(v).ok_or(CryptoError::InvalidSignature)?;
    let vk = k256::ecdsa::VerifyingKey::recover_from_prehash(keccak256(msg).as_slice(), &s, rid).map_err(|_| CryptoError::Mismatch)?;
    Ok(PublicKey { scheme: SignerScheme::Secp256k1, bytes: vk.to_sec1_point(true).as_bytes().to_vec() })
}

/// Account address for a public key.
/// - secp256k1: the Ethereum address, keccak256(uncompressed xy)[12..].
/// - P-256 / Ed25519: keccak256(scheme_byte ‖ key)[12..] (native-signer
///   addresses, EIP-8030 style); the 7702-delegation model is decided in phase 1.
pub fn address_of(pk: &PublicKey) -> Result<Address, CryptoError> {
    match pk.scheme {
        SignerScheme::Secp256k1 => {
            let vk = k256::ecdsa::VerifyingKey::from_sec1_bytes(&pk.bytes).map_err(|_| CryptoError::InvalidPublicKey)?;
            let uncompressed = vk.to_sec1_point(false);
            Ok(Address::from_slice(&keccak256(&uncompressed.as_bytes()[1..])[12..]))
        }
        SignerScheme::P256 | SignerScheme::Ed25519 => {
            let mut buf = Vec::with_capacity(1 + pk.bytes.len());
            buf.push(pk.scheme as u8);
            buf.extend_from_slice(&pk.bytes);
            Ok(Address::from_slice(&keccak256(&buf)[12..]))
        }
    }
}

pub struct P256Signer(p256::ecdsa::SigningKey);

impl P256Signer {
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self, CryptoError> {
        p256::ecdsa::SigningKey::from_slice(seed).map(Self).map_err(|_| CryptoError::InvalidSecret)
    }
}

impl Signer for P256Signer {
    fn scheme(&self) -> SignerScheme {
        SignerScheme::P256
    }
    fn public_key(&self) -> PublicKey {
        PublicKey { scheme: SignerScheme::P256, bytes: self.0.verifying_key().to_sec1_point(true).as_bytes().to_vec() }
    }
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let s: p256::ecdsa::Signature = self.0.sign_prehash(&Sha256::digest(msg)).map_err(|_| CryptoError::InvalidSignature)?;
        Ok(s.normalize_s().to_bytes().to_vec())
    }
}

pub struct Secp256k1Signer(k256::ecdsa::SigningKey);

impl Secp256k1Signer {
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self, CryptoError> {
        k256::ecdsa::SigningKey::from_slice(seed).map(Self).map_err(|_| CryptoError::InvalidSecret)
    }
}

impl Signer for Secp256k1Signer {
    fn scheme(&self) -> SignerScheme {
        SignerScheme::Secp256k1
    }
    fn public_key(&self) -> PublicKey {
        PublicKey { scheme: SignerScheme::Secp256k1, bytes: self.0.verifying_key().to_sec1_point(true).as_bytes().to_vec() }
    }
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, CryptoError> {
        // k256's recoverable signer already returns a low-s signature with a matching id.
        let (s, rid) = self.0.sign_prehash_recoverable(keccak256(msg).as_slice());
        let mut out = s.to_bytes().to_vec();
        out.push(rid.to_byte());
        Ok(out)
    }
}

pub struct Ed25519Signer(ed25519_dalek::SigningKey);

impl Ed25519Signer {
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self(ed25519_dalek::SigningKey::from_bytes(seed))
    }
}

impl Signer for Ed25519Signer {
    fn scheme(&self) -> SignerScheme {
        SignerScheme::Ed25519
    }
    fn public_key(&self) -> PublicKey {
        PublicKey { scheme: SignerScheme::Ed25519, bytes: self.0.verifying_key().to_bytes().to_vec() }
    }
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, CryptoError> {
        use ed25519_dalek::Signer as _;
        Ok(self.0.sign(msg).to_bytes().to_vec())
    }
}

/// Affine (x, y) of a P-256 public key given in SEC1 form (compressed or not):
/// what P256VERIFY and `AetherAccount.setGuardian` take.
pub fn p256_xy(sec1: &[u8]) -> Result<([u8; 32], [u8; 32]), CryptoError> {
    use p256::elliptic_curve::sec1::ToSec1Point;
    let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(sec1).map_err(|_| CryptoError::InvalidPublicKey)?;
    let point = vk.as_affine().to_sec1_point(false);
    let bytes = point.as_bytes();
    if bytes.len() != 65 {
        return Err(CryptoError::InvalidPublicKey);
    }
    Ok((bytes[1..33].try_into().expect("32"), bytes[33..65].try_into().expect("32")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn signers(seed: u8) -> Vec<Box<dyn Signer>> {
        // Valid for every curve: small big-endian scalar in [1, 255].
        let mut s = [0u8; 32];
        s[31] = seed.max(1);
        vec![Box::new(P256Signer::from_seed(&s).unwrap()), Box::new(Secp256k1Signer::from_seed(&s).unwrap()), Box::new(Ed25519Signer::from_seed(&s))]
    }

    #[test]
    fn sign_verify_round_trip_all_schemes() {
        for s in signers(7) {
            let sig = s.sign(b"hello").unwrap();
            assert_eq!(verify(&s.public_key(), b"hello", &sig), Ok(()), "{:?}", s.scheme());
            assert_eq!(verify(&s.public_key(), b"hellO", &sig), Err(CryptoError::Mismatch), "{:?}", s.scheme());
        }
    }

    #[test]
    fn wrong_key_rejected() {
        for (a, b) in signers(3).into_iter().zip(signers(4)) {
            let sig = a.sign(b"m").unwrap();
            assert!(verify(&b.public_key(), b"m", &sig).is_err());
        }
    }

    #[test]
    fn high_s_rejected_for_ecdsa() {
        let s = P256Signer::from_seed(&[9; 32]).unwrap();
        let sig = p256::ecdsa::Signature::from_slice(&s.sign(b"m").unwrap()).unwrap();
        let (r, low_s) = sig.split_scalars();
        let high = p256::ecdsa::Signature::from_scalars(r, -*low_s).unwrap();
        assert_eq!(verify(&s.public_key(), b"m", &high.to_bytes()), Err(CryptoError::HighS));
    }

    #[test]
    fn p256_matches_sha256_prehash_convention() {
        // A signature over SHA-256(msg) produced independently must verify: this is
        // the Secure Enclave `signature(for: data)` / P256VERIFY convention.
        let sk = p256::ecdsa::SigningKey::from_slice(&[5; 32]).unwrap();
        let sig: p256::ecdsa::Signature = sk.sign_prehash(&Sha256::digest(b"tx")).unwrap();
        let pk = PublicKey { scheme: SignerScheme::P256, bytes: sk.verifying_key().to_sec1_bytes().to_vec() };
        assert_eq!(verify(&pk, b"tx", &sig.normalize_s().to_bytes()), Ok(()));
    }

    #[test]
    fn secp256k1_address_matches_known_vector() {
        // Private key 0x...01 -> address 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf
        let mut seed = [0u8; 32];
        seed[31] = 1;
        let s = Secp256k1Signer::from_seed(&seed).unwrap();
        let addr = address_of(&s.public_key()).unwrap();
        assert_eq!(addr, "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf".parse::<Address>().unwrap());
    }

    #[test]
    fn recovered_key_equals_signer() {
        let s = Secp256k1Signer::from_seed(&[11; 32]).unwrap();
        let sig = s.sign(b"payload").unwrap();
        assert_eq!(recover_secp256k1(b"payload", &sig).unwrap(), s.public_key());
    }

    #[test]
    fn ecdsa_public_keys_are_compressed_sec1() {
        let seed = {
            let mut s = [0u8; 32];
            s[31] = 3;
            s
        };
        assert_eq!(P256Signer::from_seed(&seed).unwrap().public_key().bytes.len(), 33);
        assert_eq!(Secp256k1Signer::from_seed(&seed).unwrap().public_key().bytes.len(), 33);
    }

    #[test]
    fn native_addresses_are_scheme_separated() {
        let p = P256Signer::from_seed(&[2; 32]).unwrap().public_key();
        let e = Ed25519Signer::from_seed(&[2; 32]).public_key();
        assert_ne!(address_of(&p).unwrap(), address_of(&e).unwrap());
    }

    #[test]
    fn out_of_range_secrets_are_invalid_for_ecdsa() {
        assert!(P256Signer::from_seed(&[0; 32]).is_err());
        assert!(Secp256k1Signer::from_seed(&[0; 32]).is_err());
        // >= group order
        assert!(P256Signer::from_seed(&[0xff; 32]).is_err());
        assert!(Secp256k1Signer::from_seed(&[0xff; 32]).is_err());
    }

    proptest! {
        #[test]
        fn any_message_round_trips(msg in proptest::collection::vec(any::<u8>(), 0..256), seed in 1u8..=255) {
            for s in signers(seed) {
                let sig = s.sign(&msg).unwrap();
                prop_assert!(verify(&s.public_key(), &msg, &sig).is_ok());
            }
        }
    }
}
