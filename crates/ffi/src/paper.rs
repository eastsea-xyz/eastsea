//! Paper recovery key: 24 words that stand in for a recovery device.
//!
//! The wallet's own key lives in the Secure Enclave and can never be written
//! down. Instead, 24 BIP-39 words derive a P-256 key that the account registers
//! as a guardian (recovery key). With every device lost, typing the words on a
//! new Mac starts a recovery to that Mac's account; the funds move only after
//! the account's delay (48 h by default), and while any of the owner's devices
//! is alive it can cancel. Stolen words alone start a visible, cancellable
//! recovery, not an instant theft.
//!
//! Key: SHA-256("aether paper recovery key v1" ‖ BIP-39 seed ‖ counter), the
//! first value that is a valid P-256 scalar.

use crate::{WalletError, R};
use p256::ecdsa::{signature::Signer as _, Signature, SigningKey};
use sha2::{Digest, Sha256};

fn key(words: &str) -> R<SigningKey> {
    let m = bip39::Mnemonic::parse_normalized(words.trim()).map_err(|e| WalletError::Invalid(format!("recovery words: {e}")))?;
    if m.word_count() != 24 {
        return Err(WalletError::Invalid("recovery words: expected 24 words".into()));
    }
    let seed = m.to_seed("");
    for counter in 0u8..=255 {
        let d = Sha256::new().chain_update(b"aether paper recovery key v1").chain_update(seed).chain_update([counter]).finalize();
        if let Ok(k) = SigningKey::from_slice(&d) {
            return Ok(k);
        }
    }
    Err(WalletError::Invalid("recovery words: no valid key".into()))
}

/// New paper recovery key: 24 words (256 bits of entropy).
#[uniffi::export]
pub fn paper_key_new() -> String {
    bip39::Mnemonic::generate(24).expect("24 words").to_string()
}

/// The paper key's public key (SEC1 compressed), for registering it as a recovery key.
#[uniffi::export]
pub fn paper_key_public(words: String) -> R<Vec<u8>> {
    use p256::elliptic_curve::sec1::ToSec1Point as _;
    Ok(key(&words)?.verifying_key().as_affine().to_sec1_point(true).as_bytes().to_vec())
}

/// Sign `message` with the paper key (raw r‖s, low-s), as a recovery device signs.
#[uniffi::export]
pub fn paper_key_sign(words: String, message: Vec<u8>) -> R<Vec<u8>> {
    let sig: Signature = key(&words)?.sign(&message);
    Ok(sig.normalize_s().to_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_derive_one_key_that_signs_verifiably() {
        let words = paper_key_new();
        assert_eq!(words.split_whitespace().count(), 24);
        let pk = paper_key_public(words.clone()).unwrap();
        assert_eq!(pk, paper_key_public(format!("  {words}  ")).unwrap(), "surrounding whitespace is ignored");
        let sig = paper_key_sign(words.clone(), b"recover".to_vec()).unwrap();
        let apk = aether_crypto::PublicKey { scheme: aether_types::SignerScheme::P256, bytes: pk.clone() };
        aether_crypto::verify(&apk, b"recover", &sig).unwrap();
        // A different phrase is a different key; a mistyped word fails its checksum.
        assert_ne!(pk, paper_key_public(paper_key_new()).unwrap());
        let mut w: Vec<&str> = words.split_whitespace().collect();
        w[0] = if w[0] == "abandon" { "ability" } else { "abandon" };
        assert!(paper_key_public(w.join(" ")).is_err());
    }
}
