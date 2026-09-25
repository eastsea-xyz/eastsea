//! Wallet core for Swift (docs/design/09-wallet.md).
//!
//! Keys never enter Rust: the app holds a Secure Enclave P-256 key, Rust builds
//! the exact bytes to sign, Swift signs them (`SecureEnclave.P256.Signing`
//! hashes with SHA-256, matching the chain's P-256 rule), and Rust normalizes
//! the signature to low-s and submits. Every balance shown is verified through
//! a finality certificate plus an EIP-7864 proof (see `aether-light`).

uniffi::setup_scaffolding!();

use aether_crypto::{address_of, P256Signer, PublicKey, Signer};
use aether_execution::{sign_call, EvmCall};
use aether_light::{from_hex, verify_account, verify_finalized, ValidatorSet, VerifiedBlock};
use aether_state::Proof;
use aether_types::{Address, Bytes, FeeVector, GasVector, SignerScheme, TxEnvelope, TxHash, TxHeader, TxPayload, U256};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum WalletError {
    #[error("network: {0}")]
    Network(String),
    #[error("verification failed: {0}")]
    Verification(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("rejected by node: {0}")]
    Rejected(String),
}

type R<T> = Result<T, WalletError>;

#[derive(uniffi::Record)]
pub struct ChainStatus {
    pub chain_id: u64,
    pub height: u64,
    pub state_root: String,
    pub mempool: u64,
}

#[derive(uniffi::Record)]
pub struct VerifiedAccount {
    pub address: String,
    pub balance_wei: String,
    pub nonce: u64,
    /// State height the balance is for.
    pub state_height: u64,
    /// Block whose finality certificate committed that state.
    pub certified_block: u64,
    pub state_root: String,
    pub validators: u32,
}

#[derive(uniffi::Record)]
pub struct PreparedTx {
    pub from: String,
    pub nonce: u64,
    /// Bytes to sign with the Secure Enclave key (SHA-256 is applied by CryptoKit).
    pub signing_message: Vec<u8>,
    /// Envelope without signature; pass back to `submit_signed`.
    pub envelope_json: String,
}

#[derive(uniffi::Record)]
pub struct TxReceipt {
    pub height: u64,
    pub success: bool,
    pub gas_used: u64,
}

#[derive(uniffi::Record)]
pub struct BlockInfo {
    pub height: u64,
    pub txs: u32,
    pub gas_used: u64,
    pub state_root: String,
    pub proposer: String,
    pub timestamp_ms: u64,
}

fn call(rpc: &str, method: &str, params: Value) -> R<Value> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let resp: Value = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| WalletError::Network(e.to_string()))?
        .post(rpc)
        .json(&body)
        .send()
        .map_err(|e| WalletError::Network(e.to_string()))?
        .json()
        .map_err(|e| WalletError::Network(e.to_string()))?;
    if let Some(err) = resp.get("error") {
        return Err(WalletError::Rejected(err["message"].as_str().unwrap_or("error").to_string()));
    }
    Ok(resp.get("result").cloned().unwrap_or(Value::Null))
}

fn parse<T: serde::de::DeserializeOwned>(v: &Value, what: &str) -> R<T> {
    serde_json::from_value(v.clone()).map_err(|e| WalletError::Invalid(format!("{what}: {e}")))
}

fn p256_key(compressed: &[u8]) -> R<PublicKey> {
    // Accept compressed (33) or X9.63 uncompressed (65) SEC1 and normalize to compressed.
    let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(compressed).map_err(|_| WalletError::Invalid("P-256 public key".into()))?;
    Ok(PublicKey { scheme: SignerScheme::P256, bytes: vk.to_sec1_point(true).as_bytes().to_vec() })
}

#[uniffi::export]
pub fn chain_status(rpc: String) -> R<ChainStatus> {
    let v = call(&rpc, "aether_status", json!([]))?;
    Ok(ChainStatus {
        chain_id: v["chain_id"].as_u64().unwrap_or_default(),
        height: v["height"].as_u64().unwrap_or_default(),
        state_root: v["state_root"].as_str().unwrap_or_default().to_string(),
        mempool: v["mempool"].as_u64().unwrap_or_default(),
    })
}

/// Account address for a Secure Enclave P-256 public key.
#[uniffi::export]
pub fn account_address(p256_public_key: Vec<u8>) -> R<String> {
    let pk = p256_key(&p256_public_key)?;
    Ok(address_of(&pk).map_err(|e| WalletError::Invalid(e.to_string()))?.to_checksum(None))
}

fn anchor(rpc: &str, height: u64, set: &ValidatorSet) -> R<VerifiedBlock> {
    for _ in 0..40 {
        let v = call(rpc, "aether_getFinalized", json!([height + 1]))?;
        if !v.is_null() {
            let block = from_hex(v["block"].as_str().unwrap_or_default()).map_err(|e| WalletError::Verification(e.to_string()))?;
            let fin = from_hex(v["finalization"].as_str().unwrap_or_default()).map_err(|e| WalletError::Verification(e.to_string()))?;
            return verify_finalized(set, &block, &fin).map_err(|e| WalletError::Verification(format!("certificate: {e}")));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(WalletError::Network(format!("block {} not finalized yet", height + 1)))
}

/// Balance and nonce, verified against a validator-signed state root.
#[uniffi::export]
pub fn verified_account(rpc: String, address: String, validators: u32) -> R<VerifiedAccount> {
    let a: Address = address.parse().map_err(|_| WalletError::Invalid("address".into()))?;
    let v = call(&rpc, "aether_getAccount", json!([a]))?;
    let proof: Proof = parse(&v["proof"], "proof")?;
    let height = v["height"].as_u64().unwrap_or_default();
    let set = ValidatorSet::devnet(validators as u64);
    let anchor = anchor(&rpc, height, &set)?;
    let data = verify_account(&anchor, &a, &proof).map_err(|e| WalletError::Verification(format!("proof: {e}")))?.unwrap_or_default();
    Ok(VerifiedAccount {
        address: a.to_checksum(None),
        balance_wei: data.balance.to_string(),
        nonce: data.nonce,
        state_height: height,
        certified_block: anchor.height,
        state_root: format!("{}", anchor.parent_state_root),
        validators,
    })
}

/// Build a transfer for the Secure Enclave key to sign.
#[uniffi::export]
pub fn prepare_transfer(rpc: String, p256_public_key: Vec<u8>, to: String, value_wei: String) -> R<PreparedTx> {
    let pk = p256_key(&p256_public_key)?;
    let from = address_of(&pk).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let to: Address = to.parse().map_err(|_| WalletError::Invalid("recipient address".into()))?;
    let value: U256 = value_wei.parse().map_err(|_| WalletError::Invalid("amount".into()))?;
    let chain_id = call(&rpc, "aether_status", json!([]))?["chain_id"].as_u64().unwrap_or_default();
    let nonce_hex = call(&rpc, "eth_getTransactionCount", json!([from]))?;
    let nonce = u64::from_str_radix(nonce_hex.as_str().unwrap_or("0x0").trim_start_matches("0x"), 16).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let call_body = EvmCall { to: Some(to), value, input: Bytes::new(), gas_limit: 21_000 };
    let payload = call_body.encode();
    let header = TxHeader {
        chain_id,
        sender: from,
        nonce,
        gas: GasVector { exec: call_body.gas_limit, state: 0, prove: 0 },
        max_fee: FeeVector { exec: 1, ..Default::default() },
        payload_commitment: aether_execution::tx::payload_commitment(&payload),
        scheme: SignerScheme::P256,
    };
    let env = TxEnvelope { header, payload: TxPayload::Plain(Bytes::from(payload)), signature: Bytes::new() };
    Ok(PreparedTx {
        from: from.to_checksum(None),
        nonce,
        signing_message: env.signing_bytes(),
        envelope_json: serde_json::to_string(&env).map_err(|e| WalletError::Invalid(e.to_string()))?,
    })
}

/// Attach a Secure Enclave signature (raw r‖s, 64 bytes) and submit.
#[uniffi::export]
pub fn submit_signed(rpc: String, envelope_json: String, signature: Vec<u8>, p256_public_key: Vec<u8>) -> R<String> {
    let mut env: TxEnvelope = serde_json::from_str(&envelope_json).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let pk = p256_key(&p256_public_key)?;
    let sig = p256::ecdsa::Signature::from_slice(&signature).map_err(|_| WalletError::Invalid("signature must be 64-byte r‖s".into()))?;
    // Secure Enclave may return high-s; the chain only accepts low-s.
    let mut bytes = sig.normalize_s().to_bytes().to_vec();
    aether_crypto::verify(&pk, &env.signing_bytes(), &bytes).map_err(|e| WalletError::Invalid(format!("signature does not match key: {e}")))?;
    bytes.extend_from_slice(&pk.bytes);
    env.signature = Bytes::from(bytes);
    let v = call(&rpc, "aether_sendTransaction", json!([env]))?;
    let h: TxHash = parse(&v["hash"], "hash")?;
    Ok(format!("{h}"))
}

#[uniffi::export]
pub fn receipt(rpc: String, tx_hash: String) -> R<Option<TxReceipt>> {
    let h: TxHash = tx_hash.parse().map_err(|_| WalletError::Invalid("tx hash".into()))?;
    let v = call(&rpc, "aether_getReceipt", json!([h]))?;
    if v.get("receipt").is_none() {
        return Ok(None);
    }
    Ok(Some(TxReceipt {
        height: v["height"].as_u64().unwrap_or_default(),
        success: v["receipt"]["success"].as_bool().unwrap_or(false),
        gas_used: v["receipt"]["gas_used"].as_u64().unwrap_or_default(),
    }))
}

#[uniffi::export]
pub fn recent_blocks(rpc: String, n: u32) -> R<Vec<BlockInfo>> {
    let v = call(&rpc, "aether_recentBlocks", json!([n]))?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|b| BlockInfo {
            height: b["height"].as_u64().unwrap_or_default(),
            txs: b["txs"].as_array().map(|t| t.len() as u32).unwrap_or(0),
            gas_used: b["gas_used"].as_u64().unwrap_or_default(),
            state_root: b["state_root"].as_str().unwrap_or_default().to_string(),
            proposer: b["proposer"].as_str().unwrap_or_default().to_string(),
            timestamp_ms: b["timestamp_ms"].as_u64().unwrap_or_default(),
        })
        .collect())
}

/// Devnet faucet: send test coins (zero value) from the public dev account 10.
#[uniffi::export]
pub fn devnet_faucet(rpc: String, to: String, value_wei: String) -> R<String> {
    let mut seed = [0u8; 32];
    seed[0] = 0xae;
    seed[31] = 10;
    let signer = P256Signer::from_seed(&seed).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let from = address_of(&signer.public_key()).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let to: Address = to.parse().map_err(|_| WalletError::Invalid("address".into()))?;
    let value: U256 = value_wei.parse().map_err(|_| WalletError::Invalid("amount".into()))?;
    let chain_id = call(&rpc, "aether_status", json!([]))?["chain_id"].as_u64().unwrap_or_default();
    let nonce_hex = call(&rpc, "eth_getTransactionCount", json!([from]))?;
    let nonce = u64::from_str_radix(nonce_hex.as_str().unwrap_or("0x0").trim_start_matches("0x"), 16).unwrap_or(0);
    let tx = sign_call(&signer, chain_id, nonce, 1, &EvmCall { to: Some(to), value, input: Bytes::new(), gas_limit: 21_000 })
        .map_err(|e| WalletError::Invalid(e.to_string()))?;
    let v = call(&rpc, "aether_sendTransaction", json!([tx]))?;
    Ok(v["hash"].as_str().unwrap_or_default().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A signature made over SHA-256(message) by an independent P-256 key
    /// (what Secure Enclave produces), including the high-s form, is accepted
    /// and normalized; a wrong key is rejected before submission.
    #[test]
    fn enclave_style_signatures_normalize_and_bind_to_key() {
        use p256::ecdsa::signature::hazmat::PrehashSigner;
        use sha2_shim::digest;
        let sk = p256::ecdsa::SigningKey::from_slice(&[7u8; 32]).unwrap();
        let pk = sk.verifying_key().to_sec1_point(false).as_bytes().to_vec(); // X9.63 form, as CryptoKit exports
        let msg = b"aether signing message";
        let s: p256::ecdsa::Signature = sk.sign_prehash(&digest(msg)).unwrap();
        let (r, lo) = s.normalize_s().split_scalars();
        let high = p256::ecdsa::Signature::from_scalars(r, -*lo).unwrap();
        for sig in [s, high] {
            let norm = p256::ecdsa::Signature::from_slice(&sig.to_bytes()).unwrap().normalize_s().to_bytes();
            let key = p256_key(&pk).unwrap();
            assert!(aether_crypto::verify(&key, msg, &norm).is_ok());
        }
        let other = p256::ecdsa::SigningKey::from_slice(&[8u8; 32]).unwrap();
        let other_pk = p256_key(other.verifying_key().to_sec1_point(true).as_bytes()).unwrap();
        assert!(aether_crypto::verify(&other_pk, msg, &s.normalize_s().to_bytes()).is_err());
    }

    #[test]
    fn address_is_same_for_compressed_and_uncompressed_keys() {
        let sk = p256::ecdsa::SigningKey::from_slice(&[9u8; 32]).unwrap();
        let c = sk.verifying_key().to_sec1_point(true).as_bytes().to_vec();
        let u = sk.verifying_key().to_sec1_point(false).as_bytes().to_vec();
        assert_eq!(account_address(c).unwrap(), account_address(u).unwrap());
    }

    mod sha2_shim {
        pub fn digest(m: &[u8]) -> [u8; 32] {
            use sha2::Digest;
            sha2::Sha256::digest(m).into()
        }
    }
}
