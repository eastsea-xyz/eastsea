//! DeviceCheck token envelopes. The registrar attests an in-memory P-256
//! encryption key with its existing chain signing key; callers authenticate
//! that signing key against certified registry state before encrypting.
//!
//! P-256 ECDH, SHA-256 ANSI X9.63 KDF (one 256-bit output block), and
//! ChaCha20-Poly1305. Caller-supplied ephemeral seeds/nonces must be fresh and
//! unpredictable. No token or recipient secret is part of the signed key.

use crate::{verify, CryptoError, PublicKey};
use chacha20poly1305::{aead::{Aead, Payload}, ChaCha20Poly1305, KeyInit as _};
use p256::elliptic_curve::{sec1::ToSec1Point as _, zeroize::Zeroizing};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const VERSION: u8 = 1;
const KEY_DOMAIN: &[u8] = b"aether-registrar-encryption-key-v1";
const KDF_DOMAIN: &[u8] = b"aether-devicecheck-ecies-v1";
const REQUEST_DOMAIN: &[u8] = b"aether-devicecheck-request-v1";
pub const MAX_TOKEN_BYTES: usize = 8_192;
const TAG_BYTES: usize = 16;

/// RPC descriptor; a relay may serve this, but cannot replace its signature.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptionKey {
    pub version: u8,
    pub chain_id: u64,
    pub public_key: String,
    pub signature: String,
}

impl EncryptionKey {
    pub fn signed(chain_id: u64, public_key: &[u8], sign: impl FnOnce(&[u8]) -> Result<Vec<u8>, String>) -> Result<Self, String> {
        let key = compressed(public_key)?;
        let signature = sign(&key_message(chain_id, &key))?;
        if signature.len() != 64 { return Err("invalid registrar key signature".into()); }
        Ok(Self { version: VERSION, chain_id, public_key: hex::encode(key), signature: hex::encode(signature) })
    }

    /// `registrar` MUST come from authenticated chain state, never this RPC.
    pub fn authenticate(&self, chain_id: u64, registrar: &PublicKey) -> Result<Vec<u8>, String> {
        if self.version != VERSION || self.chain_id != chain_id || self.public_key.len() != 66 || self.signature.len() != 128 {
            return Err("invalid registrar encryption key".into());
        }
        let key = compressed(&hex::decode(&self.public_key).map_err(|_| "invalid registrar encryption key")?)?;
        let signature = hex::decode(&self.signature).map_err(|_| "invalid registrar key signature")?;
        verify(registrar, &key_message(chain_id, &key), &signature).map_err(|_| "registrar encryption key is not authenticated")?;
        Ok(key)
    }
}

fn key_message(chain_id: u64, public_key: &[u8]) -> Vec<u8> {
    [KEY_DOMAIN, &[VERSION], &chain_id.to_be_bytes(), public_key].concat()
}

/// Opaque token at every intermediary. Legacy plaintext param 0 is rejected.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenEnvelope {
    pub version: u8,
    pub recipient: String,
    pub ephemeral: String,
    pub nonce: String,
    pub ciphertext: String,
}

struct Parts {
    recipient: Vec<u8>,
    ephemeral: Vec<u8>,
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
}

impl TokenEnvelope {
    /// Bound the envelope before decoding or forwarding it, even on relays.
    pub fn validate(&self) -> Result<(), String> { self.parts().map(|_| ()) }

    fn parts(&self) -> Result<Parts, String> {
        if self.version != VERSION || self.recipient.len() != 66 || self.ephemeral.len() != 66 || self.nonce.len() != 24
            || !(2 * (TAG_BYTES + 1)..=2 * (MAX_TOKEN_BYTES + TAG_BYTES)).contains(&self.ciphertext.len()) {
            return Err("invalid encrypted DeviceCheck token".into());
        }
        let decode = |s: &str| hex::decode(s).map_err(|_| "invalid encrypted DeviceCheck token".to_string());
        Ok(Parts {
            recipient: compressed(&decode(&self.recipient)?)?,
            ephemeral: compressed(&decode(&self.ephemeral)?)?,
            nonce: decode(&self.nonce)?.try_into().map_err(|_| "invalid encrypted DeviceCheck nonce")?,
            ciphertext: decode(&self.ciphertext)?,
        })
    }
}

/// This secret exists only at the registrar, for the lifetime of its process.
/// Its chain signing key can remain in a signing-only Secure Enclave helper.
pub struct RecipientSecret(p256::SecretKey);

impl RecipientSecret {
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self, CryptoError> {
        p256::SecretKey::from_slice(seed).map(Self).map_err(|_| CryptoError::InvalidSecret)
    }

    pub fn public_key(&self) -> Vec<u8> {
        self.0.public_key().to_sec1_point(true).as_bytes().to_vec()
    }

    pub fn open(&self, envelope: &TokenEnvelope, context: &[u8]) -> Result<Zeroizing<String>, String> {
        let p = envelope.parts()?;
        if p.recipient != self.public_key() {
            return Err("registrar encryption key expired; fetch a fresh authenticated key".into());
        }
        let key = derive(&self.0, &p.ephemeral, &p.recipient, &p.ephemeral, context)?;
        let cipher = ChaCha20Poly1305::new((&*key).into());
        let aad = aad(&p.recipient, &p.ephemeral, &p.nonce, context);
        let plaintext = Zeroizing::new(cipher.decrypt((&p.nonce).into(), Payload { msg: &p.ciphertext, aad: &aad })
            .map_err(|_| "encrypted DeviceCheck token could not be authenticated")?);
        let token = std::str::from_utf8(&plaintext).map_err(|_| "invalid encrypted DeviceCheck token")?.to_owned();
        Ok(Zeroizing::new(token))
    }
}

pub fn seal(token: &str, recipient: &[u8], context: &[u8], ephemeral_seed: &[u8; 32], nonce: [u8; 12]) -> Result<TokenEnvelope, String> {
    if token.is_empty() || token.len() > MAX_TOKEN_BYTES { return Err("invalid DeviceCheck token size".into()); }
    let recipient = compressed(recipient)?;
    let ephemeral = p256::SecretKey::from_slice(ephemeral_seed).map_err(|_| "invalid ephemeral secret")?;
    let public = ephemeral.public_key().to_sec1_point(true).as_bytes().to_vec();
    let key = derive(&ephemeral, &recipient, &recipient, &public, context)?;
    let cipher = ChaCha20Poly1305::new((&*key).into());
    let aad = aad(&recipient, &public, &nonce, context);
    let ciphertext = cipher.encrypt((&nonce).into(), Payload { msg: token.as_bytes(), aad: &aad }).map_err(|_| "DeviceCheck encryption failed")?;
    Ok(TokenEnvelope { version: VERSION, recipient: hex::encode(recipient), ephemeral: hex::encode(public), nonce: hex::encode(nonce), ciphertext: hex::encode(ciphertext) })
}

/// Compact serialization of ALL params after the envelope, before forwarding.
/// Domain separation binds the chain and canonical RPC method as well.
pub fn request_context(chain_id: u64, method: &str, public_params_json: &[u8]) -> Vec<u8> {
    [REQUEST_DOMAIN, &chain_id.to_be_bytes(), &(method.len() as u64).to_be_bytes(), method.as_bytes(), public_params_json].concat()
}

fn compressed(bytes: &[u8]) -> Result<Vec<u8>, String> {
    p256::PublicKey::from_sec1_bytes(bytes).map(|p| p.to_sec1_point(true).as_bytes().to_vec()).map_err(|_| "invalid P-256 encryption key".into())
}

fn aad(recipient: &[u8], ephemeral: &[u8], nonce: &[u8; 12], context: &[u8]) -> Vec<u8> {
    [&[VERSION], recipient, ephemeral, nonce, context].concat()
}

fn derive(secret: &p256::SecretKey, peer: &[u8], recipient: &[u8], ephemeral: &[u8], context: &[u8]) -> Result<Zeroizing<[u8; 32]>, String> {
    let peer = p256::PublicKey::from_sec1_bytes(peer).map_err(|_| "invalid P-256 encryption key")?;
    // Existing curve arithmetic avoids activating ECDH's otherwise unused HKDF
    // dependency. ANSI X9.63 KDF: SHA256(Z || counter=1 || SharedInfo), with
    // both public keys and request domain/context in SharedInfo.
    let shared = (p256::ProjectivePoint::from(*peer.as_affine()) * secret.to_nonzero_scalar().as_ref()).to_affine().to_sec1_point(true);
    let mut hash = Sha256::new();
    hash.update(&shared.as_bytes()[1..]);
    hash.update(1u32.to_be_bytes());
    hash.update(KDF_DOMAIN);
    hash.update(recipient);
    hash.update(ephemeral);
    hash.update(context);
    Ok(Zeroizing::new(hash.finalize().into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{P256Signer, Signer as _};

    #[test]
    fn registrar_key_requires_chain_signer_and_token_requires_full_context() {
        let signer = P256Signer::from_seed(&[4; 32]).unwrap();
        let recipient = RecipientSecret::from_seed(&[8; 32]).unwrap();
        let descriptor = EncryptionKey::signed(7, &recipient.public_key(), |m| signer.sign(m).map_err(|e| e.to_string())).unwrap();
        let public = descriptor.authenticate(7, &signer.public_key()).unwrap();
        assert!(descriptor.authenticate(8, &signer.public_key()).is_err());
        assert!(descriptor.authenticate(7, &P256Signer::from_seed(&[5; 32]).unwrap().public_key()).is_err());
        let mut changed = descriptor.clone();
        changed.public_key = hex::encode(RecipientSecret::from_seed(&[9; 32]).unwrap().public_key());
        assert!(changed.authenticate(7, &signer.public_key()).is_err());
        let context = request_context(7, "aether_registerDevice", b"[1,2,3,4,5]");
        let envelope = seal("SECRET DEVICECHECK TOKEN", &public, &context, &[6; 32], [9; 12]).unwrap();
        assert_eq!(&**recipient.open(&envelope, &context).unwrap(), "SECRET DEVICECHECK TOKEN");
        assert!(RecipientSecret::from_seed(&[7; 32]).unwrap().open(&envelope, &context).is_err());
        for altered in [request_context(8, "aether_registerDevice", b"[1,2,3,4,5]"), request_context(7, "aether_reattest", b"[1,2,3,4,5]"), request_context(7, "aether_registerDevice", b"[1,2,3,4,6]")] {
            assert!(recipient.open(&envelope, &altered).is_err());
        }
        let mut tampered = envelope.clone();
        let mut bytes = hex::decode(&tampered.ciphertext).unwrap();
        bytes[0] ^= 1;
        tampered.ciphertext = hex::encode(bytes);
        assert!(recipient.open(&tampered, &context).is_err());
        let mut oversized = envelope;
        oversized.ciphertext = "aa".repeat(MAX_TOKEN_BYTES + TAG_BYTES + 1);
        assert!(oversized.validate().is_err());
        assert!(seal("", &public, &context, &[6; 32], [9; 12]).is_err());
    }
}
