//! Transaction body encoding, signing and sender verification.
//!
//! `TxPayload::Plain` carries an `EvmCall` in canonical bytes. The envelope
//! signature field is `sig ‖ public_key` for P-256 / Ed25519 (the key cannot be
//! recovered) and a 65-byte recoverable signature for secp256k1.

use aether_crypto::{address_of, recover_secp256k1, verify, CryptoError, PublicKey, Signer};
use aether_hash::{Blake3, Hasher};
use aether_types::{Address, Bytes, Canonical, FeeVector, GasVector, SignerScheme, TxEnvelope, TxHash, TxHeader, TxPayload, B256, U256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvmCall {
    /// None = contract creation.
    pub to: Option<Address>,
    pub value: U256,
    pub input: Bytes,
    pub gas_limit: u64,
    /// EIP-7702-style delegation set by this (self-signed) tx before it runs:
    /// the sender's code becomes `0xef0100 ‖ delegate`. `Address::ZERO` clears it.
    /// The tx signature is the authorization, so P-256 accounts can delegate
    /// without a secp256k1 authorization tuple.
    pub delegate: Option<Address>,
}

/// Exec gas a wallet signs for a plain value transfer (no calldata) to an
/// address without code: the intrinsic cost, nothing to run.
pub const PLAIN_TRANSFER_GAS: u64 = 21_000;
/// Exec gas for a plain value transfer to an address WITH code: a contract's
/// `receive()`, or an account delegated by EIP-7702 (`0xef0100 ‖ target`,
/// e.g. after the wallet's own batch/guardian features), where loading the
/// delegate and running its receive cost more than the intrinsic 21,000.
/// Found live (docs/research/contracts-live-2026-10-06.md): 21,000 to such a
/// recipient is included, fails out of gas and still pays its fee. Unused gas
/// is not charged; the quote stays a maximum. Six fresh receive() storage
/// writes need more than 100,000 gas, so reserve 300,000 for code recipients.
pub const CODE_RECIPIENT_TRANSFER_GAS: u64 = 300_000;

/// The exec gas limit for a plain transfer, from the recipient's code as the
/// node reports it (`eth_getCode`; empty for an ordinary account).
pub fn plain_transfer_gas_limit(recipient_code: &[u8]) -> u64 {
    if recipient_code.is_empty() {
        PLAIN_TRANSFER_GAS
    } else {
        CODE_RECIPIENT_TRANSFER_GAS
    }
}

/// Wallet default for the signed state budget on a paid-state genesis.
/// EVM fresh slots cost at least 20k execution gas, account creation 32k,
/// and deployed code 200 per byte. One unit per 200 gas covers normal log
/// volume, while the input and envelope allowance covers archived tx bytes.
/// Callers with a known storage layout may sign a tighter explicit budget.
pub fn recommended_state_budget(call: &EvmCall, balance: Option<U256>, state_price: u128) -> u64 {
    if state_price == 0 || balance.is_none_or(|b| b.is_zero()) { return 0; }
    // Covers the signed envelope (including a P-256 signature) and a receipt.
    // Calldata is added separately because zero bytes use only 4 execution gas.
    let persisted = (call.input.len() as u64).saturating_add(512).div_ceil(crate::fees::RECEIPT_BYTES_PER_STATE_UNIT);
    if call.to.is_some() && call.input.is_empty() && call.delegate.is_none() && call.gas_limit == PLAIN_TRANSFER_GAS {
        // Only the 21,000-gas no-code path is a plain transfer. A contract's
        // receive() or an existing 7702 delegation can run with a larger
        // limit even when calldata is empty, so use the execution estimate.
        // Reserve for the sender's first account and, for a positive transfer,
        // a recipient that has not existed before this transaction.
        return persisted.saturating_add(crate::fees::STATE_ACCOUNT_UNITS)
            .saturating_add(if call.value.is_zero() { 0 } else { crate::fees::STATE_ACCOUNT_UNITS });
    }
    (call.gas_limit / 200).saturating_add(crate::fees::STATE_ACCOUNT_UNITS).saturating_add(persisted).min(crate::fees::MAX_STATE_UNITS_PER_BLOCK)
}

/// The most a signed envelope can ever be charged in fees, whatever the base
/// fees and state price do before it lands — wallet policy for what a send
/// sheet shows (pre-audit 7, M1; B5 review round 2, finding 1), not a
/// consensus rule. Each dimension is bounded by what the envelope signs:
/// exec pays `min(base + tip, max_fee.exec)` per gas for at most `gas.exec`
/// gas; state pays the state price, which admission and execution refuse
/// above `max_fee.state`, for at most `gas.state` units; prove pays the prove
/// base fee, refused above `max_fee.prove`, for at most `gas.prove` steps.
/// The tip is inside the exec cap. Saturates instead of overflowing.
pub fn signed_fee_maximum(gas: &GasVector, max_fee: &FeeVector) -> u128 {
    u128::from(gas.exec)
        .saturating_mul(max_fee.exec)
        .saturating_add(u128::from(gas.state).saturating_mul(max_fee.state))
        .saturating_add(u128::from(gas.prove).saturating_mul(max_fee.prove))
}

/// The legacy pre-state-fees testnet's chain id (crates/node/src/mainnet.rs
/// `TESTNET_CHAIN_ID`): its fee policy has no state price, by design.
pub const LEGACY_STATELESS_CHAIN_ID: u64 = 7_780;

/// The state price a wallet signs against, from a node's reported
/// `base_fee.state` (wallet policy, shared by the app and the CLI — B5
/// review round 2, finding 6). Only the known stateless legacy chain prices
/// state at 0, whatever a node there reports. On any other chain the
/// mandatory state burn exists, so a missing or malformed report cannot be
/// quoted (an error), and a report under the fixed unit price — 0 included —
/// is stale or lying and takes the floor.
pub fn wallet_state_price(chain_id: u64, reported: Option<&str>) -> Result<u128, String> {
    if chain_id == LEGACY_STATELESS_CHAIN_ID {
        return Ok(0);
    }
    let price = reported.and_then(|v| v.parse::<u128>().ok()).ok_or_else(|| {
        "the network did not report a state price; this chain charges one for persistent data, so no fee can be quoted".to_string()
    })?;
    Ok(price.max(crate::fees::STATE_UNIT_PRICE))
}

/// Payload trailer tag for `delegate` (absent = no change, keeps old encodings valid).
const DELEGATE_TAG: u8 = 0xd7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxError {
    WrongChain,
    NonCanonicalGroup,
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
        if let Some(d) = self.delegate {
            out.push(DELEGATE_TAG);
            out.extend_from_slice(d.as_slice());
        }
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
        let delegate = match rest {
            [] => None,
            [DELEGATE_TAG, a @ ..] if a.len() == 20 => Some(Address::from_slice(a)),
            _ => return Err(TxError::MalformedPayload),
        };
        Ok(EvmCall { to, value, input, gas_limit, delegate })
    }
}

pub fn tx_hash(tx: &TxEnvelope) -> TxHash {
    B256::from(Blake3.hash_bytes(&tx.to_canonical_bytes()))
}

pub fn payload_commitment(payload: &[u8]) -> B256 {
    B256::from(Blake3.hash_bytes(payload))
}

/// Build and sign an envelope for `call`, paying up to `gas_price` per exec and prove unit.
pub fn sign_call(signer: &dyn Signer, chain_id: u64, nonce: u64, gas_price: u128, call: &EvmCall) -> Result<TxEnvelope, CryptoError> {
    sign_call_with(signer, chain_id, nonce, FeeVector { exec: gas_price, state: 0, prove: gas_price }, gas_price, call)
}

/// `sign_call` with per-dimension fee caps and a priority fee per exec gas:
/// exec pays `min(base + tip, max_fee.exec)` per gas, prove pays its base fee.
pub fn sign_call_with(signer: &dyn Signer, chain_id: u64, nonce: u64, max_fee: FeeVector, tip: u128, call: &EvmCall) -> Result<TxEnvelope, CryptoError> {
    let pk = signer.public_key();
    let payload = call.encode();
    let header = TxHeader {
        chain_id,
        sender: address_of(&pk)?,
        nonce,
        // Every interpreted instruction costs at least 1 gas, so prove steps <= gas_limit.
        gas: GasVector { exec: call.gas_limit, state: 0, prove: call.gas_limit },
        max_fee,
        tip,
        payload_commitment: payload_commitment(&payload),
        scheme: signer.scheme(),
        group: None,
    };
    let mut env = TxEnvelope { header, payload: TxPayload::Plain(Bytes::from(payload)), signature: Bytes::new() };
    let mut sig = signer.sign(&env.signing_bytes())?;
    if signer.scheme() != SignerScheme::Secp256k1 {
        sig.extend_from_slice(&pk.bytes);
    }
    env.signature = Bytes::from(sig);
    Ok(env)
}

/// `sign_call` for a chain of group `group` (0 = the only group today, same
/// bytes as `sign_call`): the group is part of what the sender signs, so a
/// signed tx cannot be replayed into another group's blocks.
pub fn sign_call_group(
    signer: &dyn Signer,
    chain_id: u64,
    nonce: u64,
    gas_price: u128,
    group: u16,
    call: &EvmCall,
) -> Result<TxEnvelope, CryptoError> {
    let mut tx = sign_call(signer, chain_id, nonce, gas_price, call)?;
    if group != 0 {
        tx.header.group = Some(group);
        let mut sig = signer.sign(&tx.signing_bytes())?;
        if tx.header.scheme != SignerScheme::Secp256k1 {
            sig.extend_from_slice(&signer.public_key().bytes);
        }
        tx.signature = Bytes::from(sig);
    }
    Ok(tx)
}

/// Stateless validity: chain, payload commitment, signature, sender binding.
/// Returns the decoded call. Nonce and balance are checked during execution.
pub fn validate_stateless(tx: &TxEnvelope, chain_id: u64) -> Result<EvmCall, TxError> {
    let h = &tx.header;
    if h.chain_id != chain_id {
        return Err(TxError::WrongChain);
    }
    if h.group == Some(0) {
        return Err(TxError::NonCanonicalGroup);
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
        EvmCall { to: Some(Address::repeat_byte(9)), value: U256::from(5), input: Bytes::from_static(b"\x01\x02"), gas_limit: 50_000, delegate: None }
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

    #[test]
    fn group_signatures_are_distinct_and_zero_is_canonical_only_when_absent() {
        let signer = P256Signer::from_seed(&seed(6)).unwrap();
        let legacy = sign_call(&signer, 7, 0, 1, &call()).unwrap();
        assert_eq!(sign_call_group(&signer, 7, 0, 1, 0, &call()).unwrap(), legacy);
        let grouped = sign_call_group(&signer, 7, 0, 1, 3, &call()).unwrap();
        assert_eq!(validate_stateless(&grouped, 7).unwrap(), call());
        let mut replay = grouped.clone();
        replay.header.group = Some(4);
        assert!(matches!(validate_stateless(&replay, 7), Err(TxError::BadSignature(_))));
        let mut noncanonical = legacy;
        noncanonical.header.group = Some(0);
        assert_eq!(validate_stateless(&noncanonical, 7), Err(TxError::NonCanonicalGroup));
    }
}
