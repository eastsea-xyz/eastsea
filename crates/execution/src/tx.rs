//! Transaction body encoding, signing and sender verification.
//!
//! `TxPayload::Plain` carries an `EvmCall` in canonical bytes. The envelope
//! signature field is `sig ‖ public_key` for P-256 / Ed25519 (the key cannot be
//! recovered) and a 65-byte recoverable signature for secp256k1.

use aether_crypto::{address_of, recover_secp256k1, verify, CryptoError, PublicKey, Signer};
use aether_hash::{Blake3, Hasher};
use aether_types::{
    Address, Bytes, Canonical, FeeVector, GasVector, SignerScheme, TxEnvelope, TxHash, TxHeader, TxPayload, B256, U256,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvmCall {
    /// None = contract creation.
    pub to: Option<Address>,
    pub value: U256,
    pub input: Bytes,
    pub gas_limit: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxError {
    WrongChain,
    EncryptedPayloadUnsupported,
    PayloadCommitment,
    MalformedPayload,
    BadSignature(CryptoError),
    SenderMismatch,
    GasMismatch,
}

impl EvmCall {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + 20 + 32 + 8 + 8 + self.input.len());
        match self.to {
            Some(a) => {
                out.push(1);
                out.extend_from_slice(a.as_slice());
            }
            None => out.push(0),
        }
        out.extend_from_slice(&self.value.to_be_bytes::<32>());
        out.extend_from_slice(&self.gas_limit.to_be_bytes());
        out.extend_from_slice(&(self.input.len() as u64).to_be_bytes());
        out.extend_from_slice(&self.input);
        out
    }

    pub fn decode(b: &[u8]) -> Result<Self, TxError> {
        let mut rest = b;
        let mut take = |n: usize| -> Result<&[u8], TxError> {
            if rest.len() < n {
                return Err(TxError::MalformedPayload);
            }
            let (h, t) = rest.split_at(n);
            rest = t;
            Ok(h)
        };
        let to = match take(1)?[0] {
            0 => None,
            1 => Some(Address::from_slice(take(20)?)),
            _ => return Err(TxError::MalformedPayload),
        };
        let value = U256::from_be_slice(take(32)?);
        let gas_limit = u64::from_be_bytes(take(8)?.try_into().expect("8"));
        let len = u64::from_be_bytes(take(8)?.try_into().expect("8")) as usize;
        let input = Bytes::copy_from_slice(take(len)?);
        if !rest.is_empty() {
            return Err(TxError::MalformedPayload);
        }
        Ok(EvmCall { to, value, input, gas_limit })
    }
}

pub fn tx_hash(tx: &TxEnvelope) -> TxHash {
    B256::from(Blake3.hash_bytes(&tx.to_canonical_bytes()))
}

pub fn payload_commitment(payload: &[u8]) -> B256 {
    B256::from(Blake3.hash_bytes(payload))
}

/// Build and sign an envelope for `call`.
pub fn sign_call(signer: &dyn Signer, chain_id: u64, nonce: u64, gas_price: u128, call: &EvmCall) -> Result<TxEnvelope, CryptoError> {
    let pk = signer.public_key();
    let payload = call.encode();
    let header = TxHeader {
        chain_id,
        sender: address_of(&pk)?,
        nonce,
        gas: GasVector { exec: call.gas_limit, state: 0, prove: 0 },
        max_fee: FeeVector { exec: gas_price, ..Default::default() },
        payload_commitment: payload_commitment(&payload),
        scheme: signer.scheme(),
    };
    let mut env = TxEnvelope { header, payload: TxPayload::Plain(Bytes::from(payload)), signature: Bytes::new() };
    let mut sig = signer.sign(&env.signing_bytes())?;
    if signer.scheme() != SignerScheme::Secp256k1 {
        sig.extend_from_slice(&pk.bytes);
    }
    env.signature = Bytes::from(sig);
    Ok(env)
}

/// Stateless validity: chain, payload commitment, signature, sender binding.
/// Returns the decoded call. Nonce and balance are checked during execution.
pub fn validate_stateless(tx: &TxEnvelope, chain_id: u64) -> Result<EvmCall, TxError> {
    let h = &tx.header;
    if h.chain_id != chain_id {
        return Err(TxError::WrongChain);
    }
    let payload = match &tx.payload {
        TxPayload::Plain(b) => b,
        TxPayload::Encrypted { .. } => return Err(TxError::EncryptedPayloadUnsupported),
    };
    if payload_commitment(payload) != h.payload_commitment {
        return Err(TxError::PayloadCommitment);
    }
    let msg = tx.signing_bytes();
    let pk = match h.scheme {
        SignerScheme::Secp256k1 => recover_secp256k1(&msg, &tx.signature).map_err(TxError::BadSignature)?,
        SignerScheme::P256 | SignerScheme::Ed25519 => {
            let (sig_len, pk_len) = if h.scheme == SignerScheme::P256 { (64, 33) } else { (64, 32) };
            if tx.signature.len() != sig_len + pk_len {
                return Err(TxError::BadSignature(CryptoError::InvalidSignature));
            }
            let pk = PublicKey { scheme: h.scheme, bytes: tx.signature[sig_len..].to_vec() };
            verify(&pk, &msg, &tx.signature[..sig_len]).map_err(TxError::BadSignature)?;
            pk
        }
    };
    if address_of(&pk).map_err(TxError::BadSignature)? != h.sender {
        return Err(TxError::SenderMismatch);
    }
    let call = EvmCall::decode(payload)?;
    if call.gas_limit != h.gas.exec {
        return Err(TxError::GasMismatch);
    }
    Ok(call)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_crypto::{Ed25519Signer, P256Signer, Secp256k1Signer};

    fn seed(b: u8) -> [u8; 32] {
        let mut s = [0u8; 32];
        s[31] = b;
        s
    }

    fn call() -> EvmCall {
        EvmCall { to: Some(Address::repeat_byte(9)), value: U256::from(5), input: Bytes::from_static(b"\x01\x02"), gas_limit: 50_000 }
    }

    #[test]
    fn call_codec_round_trip_and_rejects_trailing() {
        for c in [call(), EvmCall { to: None, input: Bytes::new(), ..call() }] {
            let e = c.encode();
            assert_eq!(EvmCall::decode(&e).unwrap(), c);
            let mut bad = e.clone();
            bad.push(0);
            assert!(EvmCall::decode(&bad).is_err());
            assert!(EvmCall::decode(&e[..e.len() - 1]).is_err());
        }
    }

    #[test]
    fn every_scheme_signs_valid_envelopes() {
        let signers: Vec<Box<dyn Signer>> = vec![
            Box::new(P256Signer::from_seed(&seed(1)).unwrap()),
            Box::new(Secp256k1Signer::from_seed(&seed(2)).unwrap()),
            Box::new(Ed25519Signer::from_seed(&seed(3))),
        ];
        for s in signers {
            let tx = sign_call(s.as_ref(), 7, 0, 1, &call()).unwrap();
            assert_eq!(validate_stateless(&tx, 7).unwrap(), call(), "{:?}", s.scheme());
            assert_eq!(validate_stateless(&tx, 8), Err(TxError::WrongChain));
        }
    }

    #[test]
    fn tampering_is_detected() {
        let s = P256Signer::from_seed(&seed(4)).unwrap();
        let tx = sign_call(&s, 7, 0, 1, &call()).unwrap();

        let mut t = tx.clone();
        t.header.nonce = 1;
        assert!(matches!(validate_stateless(&t, 7), Err(TxError::BadSignature(_))));

        let mut t = tx.clone();
        t.payload = TxPayload::Plain(Bytes::from(EvmCall { value: U256::from(999), ..call() }.encode()));
        assert_eq!(validate_stateless(&t, 7), Err(TxError::PayloadCommitment));

        // Someone else's valid signature cannot claim this sender.
        let other = P256Signer::from_seed(&seed(5)).unwrap();
        let mut t = sign_call(&other, 7, 0, 1, &call()).unwrap();
        t.header.sender = tx.header.sender;
        assert!(validate_stateless(&t, 7).is_err());
    }
}
