//! Transaction building for browsers: the Aether extension wallet signs with a
//! WebCrypto P-256 key and uses this module for the parts that must match the
//! chain byte for byte (addresses, envelopes, the signing message, low-s).
//! Network calls stay in JavaScript; the rules mirror `aether-ffi::prepare`.

use aether_crypto::{address_of, verify, PublicKey};
use aether_execution::{tx::payload_commitment, EvmCall};
use aether_types::{Address, Bytes, FeeVector, GasVector, SignerScheme, TxEnvelope, TxHeader, TxPayload, U256};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

const GWEI: u128 = 1_000_000_000;
/// Same cap as the app wallet (`aether-ffi::prepare_call`).
const MAX_GAS: u64 = 10_000_000;
const DEFAULT_CALL_GAS: u64 = 3_000_000;
const TRANSFER_GAS: u64 = 21_000;

fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

/// Compressed SEC1 P-256 key from compressed (33) or uncompressed (65) bytes.
fn p256_key(bytes: &[u8]) -> Result<PublicKey, String> {
    let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(bytes).map_err(|_| "not a P-256 public key".to_string())?;
    Ok(PublicKey { scheme: SignerScheme::P256, bytes: vk.to_sec1_point(true).as_bytes().to_vec() })
}

/// Fee caps from `aether_status`: twice the base fee plus a 1 gwei tip.
fn fee_caps(status: &Value) -> (FeeVector, u128) {
    let get = |k: &str| status["base_fee"][k].as_str().and_then(|v| v.parse::<u128>().ok()).unwrap_or(GWEI);
    (FeeVector { exec: get("exec") * 2 + GWEI, state: 0, prove: get("prove") * 2 }, GWEI)
}

pub fn address_for(public_key: &[u8]) -> Result<String, String> {
    let pk = p256_key(public_key)?;
    Ok(address_of(&pk).map_err(|e| e.to_string())?.to_checksum(None))
}

/// A request to build: what a page (or the popup) asked to send.
pub struct Request<'a> {
    pub to: &'a str,
    pub value_wei: &'a str,
    pub data_hex: &'a str,
    pub gas_limit: u64,
}

/// Build the envelope for `req` from `public_key`. Returns JSON:
/// `{from, nonce, signing_message (hex), envelope}`.
pub fn prepare_tx(public_key: &[u8], status: &Value, expected_chain: u64, nonce: u64, req: &Request) -> Result<Value, String> {
    let pk = p256_key(public_key)?;
    let from = address_of(&pk).map_err(|e| e.to_string())?;
    let chain_id = status["chain_id"].as_u64().ok_or("status has no chain_id")?;
    if chain_id != expected_chain {
        return Err(format!("the node reports chain {chain_id}, but this wallet is for chain {expected_chain}"));
    }
    let to: Option<Address> = if req.to.is_empty() { None } else { Some(req.to.parse().map_err(|_| "bad recipient address")?) };
    let value: U256 = if req.value_wei.is_empty() { U256::ZERO } else { req.value_wei.parse().map_err(|_| "bad value")? };
    let input = alloy_primitives::hex::decode(req.data_hex.trim_start_matches("0x")).map_err(|_| "call data is not hex")?;
    let gas_limit = match req.gas_limit {
        0 if input.is_empty() && to.is_some() => TRANSFER_GAS,
        0 => DEFAULT_CALL_GAS,
        g => g.min(MAX_GAS),
    };
    let call = EvmCall { to, value, input: Bytes::from(input), gas_limit, delegate: None };
    let payload = call.encode();
    let (max_fee, tip) = fee_caps(status);
    let header = TxHeader {
        chain_id,
        sender: from,
        nonce,
        // Every interpreted instruction costs at least 1 gas, so prove steps <= gas_limit.
        gas: GasVector { exec: gas_limit, state: 0, prove: gas_limit },
        max_fee,
        tip,
        payload_commitment: payload_commitment(&payload),
        scheme: SignerScheme::P256,
        group: None,
    };
    let env = TxEnvelope { header, payload: TxPayload::Plain(Bytes::from(payload)), signature: Bytes::new() };
    Ok(json!({
        "from": from.to_checksum(None),
        "nonce": nonce,
        "gas_limit": gas_limit,
        "signing_message": alloy_primitives::hex::encode_prefixed(env.signing_bytes()),
        "envelope": serde_json::to_value(&env).map_err(|e| e.to_string())?,
    }))
}

/// Attach a raw r‖s signature (WebCrypto may return high-s: normalized here),
/// check it against the key, and return the envelope ready for `aether_sendTransaction`.
pub fn attach(envelope: &Value, signature: &[u8], public_key: &[u8]) -> Result<Value, String> {
    let mut env: TxEnvelope = serde_json::from_value(envelope.clone()).map_err(|e| format!("envelope: {e}"))?;
    let pk = p256_key(public_key)?;
    if address_of(&pk).map_err(|e| e.to_string())? != env.header.sender {
        return Err("this key is not the envelope's sender".into());
    }
    let sig = p256::ecdsa::Signature::from_slice(signature).map_err(|_| "signature must be 64-byte r‖s")?;
    let mut bytes = sig.normalize_s().to_bytes().to_vec();
    verify(&pk, &env.signing_bytes(), &bytes).map_err(|e| format!("signature does not match key: {e:?}"))?;
    bytes.extend_from_slice(&pk.bytes);
    env.signature = Bytes::from(bytes);
    serde_json::to_value(&env).map_err(|e| e.to_string())
}

/// Uncompressed SEC1 public key (65 bytes) for a 32-byte P-256 secret, so an
/// imported key can be handed to WebCrypto as a JWK (which needs x and y).
pub fn public_key_from_secret(secret: &[u8]) -> Result<Vec<u8>, String> {
    let sk = p256::ecdsa::SigningKey::from_slice(secret).map_err(|_| "not a P-256 private key (32 bytes, below the curve order)".to_string())?;
    Ok(sk.verifying_key().to_sec1_point(false).as_bytes().to_vec())
}

// ---- JavaScript API (JSON strings in and out) ----

/// Checksummed account address for a P-256 public key (raw SEC1 bytes).
#[wasm_bindgen(js_name = accountAddress)]
pub fn account_address_js(public_key: &[u8]) -> Result<String, JsError> {
    address_for(public_key).map_err(err)
}

#[wasm_bindgen(js_name = publicKeyFromSecret)]
pub fn public_key_from_secret_js(secret: &[u8]) -> Result<Vec<u8>, JsError> {
    public_key_from_secret(secret).map_err(err)
}

/// `request_json`: `{to, value_wei, data, gas}`; `status_json`: the `aether_status` result.
#[wasm_bindgen(js_name = prepareTx)]
pub fn prepare_tx_js(public_key: &[u8], status_json: &str, expected_chain: u64, nonce: u64, request_json: &str) -> Result<String, JsError> {
    let status: Value = serde_json::from_str(status_json).map_err(err)?;
    let r: Value = serde_json::from_str(request_json).map_err(err)?;
    let req = Request {
        to: r["to"].as_str().unwrap_or(""),
        value_wei: r["value_wei"].as_str().unwrap_or("0"),
        data_hex: r["data"].as_str().unwrap_or("0x"),
        gas_limit: r["gas"].as_u64().unwrap_or(0),
    };
    Ok(prepare_tx(public_key, &status, expected_chain, nonce, &req).map_err(err)?.to_string())
}

/// Returns the signed envelope JSON for `aether_sendTransaction`.
#[wasm_bindgen(js_name = attachSignature)]
pub fn attach_js(envelope_json: &str, signature: &[u8], public_key: &[u8]) -> Result<String, JsError> {
    let env: Value = serde_json::from_str(envelope_json).map_err(err)?;
    Ok(attach(&env, signature, public_key).map_err(err)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_execution::validate_stateless;
    use p256::ecdsa::{signature::Signer as _, SigningKey};

    fn key() -> SigningKey {
        SigningKey::from_slice(&[7u8; 32]).unwrap()
    }

    fn status() -> Value {
        json!({"chain_id": 7780, "base_fee": {"exec": "0", "prove": "0"}})
    }

    fn pubkey(k: &SigningKey) -> Vec<u8> {
        use p256::elliptic_curve::sec1::ToSec1Point as _;
        k.verifying_key().as_affine().to_sec1_point(false).as_bytes().to_vec()
    }

    #[test]
    fn prepared_and_signed_envelope_verifies_like_the_chain() {
        let k = key();
        let pk = pubkey(&k);
        let req = Request { to: "0x00000000000000000000000000000000000000aa", value_wei: "1000", data_hex: "0x", gas_limit: 0 };
        let p = prepare_tx(&pk, &status(), 7780, 3, &req).unwrap();
        assert_eq!(p["gas_limit"], 21_000);
        let msg = alloy_primitives::hex::decode(p["signing_message"].as_str().unwrap()).unwrap();
        let sig: p256::ecdsa::Signature = k.sign(&msg);
        let env = attach(&p["envelope"], &sig.to_bytes(), &pk).unwrap();
        let env: TxEnvelope = serde_json::from_value(env).unwrap();
        assert_eq!(env.header.nonce, 3);
        let call = validate_stateless(&env, 7780).expect("the chain's own check accepts it");
        assert_eq!(call.value, U256::from(1000));
        assert_eq!(env.header.sender.to_checksum(None), address_for(&pk).unwrap());
    }

    #[test]
    fn high_s_signatures_are_normalized() {
        let k = key();
        let pk = pubkey(&k);
        let req = Request { to: "0x00000000000000000000000000000000000000aa", value_wei: "0", data_hex: "0xa9059cbb", gas_limit: 50_000 };
        let p = prepare_tx(&pk, &status(), 7780, 0, &req).unwrap();
        let msg = alloy_primitives::hex::decode(p["signing_message"].as_str().unwrap()).unwrap();
        let sig: p256::ecdsa::Signature = k.sign(&msg);
        let (r, s) = sig.split_scalars();
        let high = p256::ecdsa::Signature::from_scalars(r, -*s).unwrap();
        let low = if high.normalize_s() == high { sig } else { high };
        assert!(attach(&p["envelope"], &low.to_bytes(), &pk).is_ok());
        assert!(attach(&p["envelope"], &high.to_bytes(), &pk).is_ok());
    }

    #[test]
    fn imported_secret_gives_the_same_public_key() {
        let k = key();
        assert_eq!(public_key_from_secret(&[7u8; 32]).unwrap(), pubkey(&k));
        assert!(public_key_from_secret(&[0u8; 32]).is_err());
    }

    #[test]
    fn wrong_chain_and_wrong_key_are_rejected() {
        let k = key();
        let pk = pubkey(&k);
        let req = Request { to: "", value_wei: "0", data_hex: "0x6000", gas_limit: 0 };
        assert!(prepare_tx(&pk, &status(), 1, 0, &req).unwrap_err().contains("chain"));
        let p = prepare_tx(&pk, &status(), 7780, 0, &req).unwrap();
        assert_eq!(p["gas_limit"], 3_000_000);
        let other = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let msg = alloy_primitives::hex::decode(p["signing_message"].as_str().unwrap()).unwrap();
        let sig: p256::ecdsa::Signature = other.sign(&msg);
        assert!(attach(&p["envelope"], &sig.to_bytes(), &pk).is_err());
        // A matching signature from another key still cannot claim this sender.
        let other_pk = pubkey(&other);
        assert!(attach(&p["envelope"], &sig.to_bytes(), &other_pk).unwrap_err().contains("sender"));
    }
}
