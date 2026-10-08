//! Transaction building for browsers: the Aether extension wallet signs with a
//! WebCrypto P-256 key and uses this module for the parts that must match the
//! chain byte for byte (addresses, envelopes, the signing message, low-s).
//! Network calls stay in JavaScript; the rules mirror `aether-ffi::prepare`.

use aether_crypto::{address_of, verify, PublicKey};
use aether_execution::{tx::payload_commitment, EvmCall};
use aether_light::{from_hex, verify_account, verify_finalized_chain, ValidatorSet};
use aether_state::Proof;
use aether_types::{Address, Bytes, FeeVector, GasVector, SignerScheme, TxEnvelope, TxHeader, TxPayload, U256};
use commonware_codec::Decode;
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

#[path = "../../ffi/src/typed_data.rs"]
mod typed_data;

#[cfg(test)]
#[path = "../../ffi/src/typed_data_tests.rs"]
mod typed_data_tests;

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

/// Fee caps from `aether_status` and the sender's balance: twice the base fee
/// plus a 1 gwei tip — or, when the chain is at its zero floor (below target
/// load the base fee is 0) or the sender has no balance, tip 0 and exec
/// capped at base × 2, so a new account can transact at all (G2). Mirrors
/// `aether-ffi::fee_caps`.
fn fee_caps(status: &Value, balance: Option<U256>) -> (FeeVector, u128) {
    let get = |k: &str| status["base_fee"][k].as_str().and_then(|v| v.parse::<u128>().ok()).unwrap_or(GWEI);
    let free = get("exec") == 0 || balance == Some(U256::ZERO);
    (
        FeeVector {
            exec: if free { get("exec") * 2 } else { get("exec") * 2 + GWEI },
            state: status["base_fee"]["state"].as_str().and_then(|v| v.parse().ok()).unwrap_or(0),
            prove: get("prove") * 2,
        },
        if free { 0 } else { GWEI },
    )
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
    /// The sender's balance (wei): a zero balance sends with tip 0 (G2).
    pub balance_wei: &'a str,
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
    let balance = if req.balance_wei.is_empty() { None } else { req.balance_wei.parse::<U256>().ok() };
    let (max_fee, tip) = fee_caps(status, balance);
    let header = TxHeader {
        chain_id,
        sender: from,
        nonce,
        // Every interpreted instruction costs at least 1 gas, so prove steps <= gas_limit.
        gas: GasVector { exec: gas_limit, state: aether_execution::recommended_state_budget(&call, balance, max_fee.state), prove: gas_limit },
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

/// Check an account answer against a certificate and a proof using the pinned
/// committee, chain, height and freshness checks of the native wallet.
/// `minimum_height` is stored by the caller across service-worker restarts.
pub fn verified_account(
    network: &Value,
    status: &Value,
    account: &Value,
    finalized: &Value,
    address: &str,
    minimum_height: u64,
    now_ms: u64,
) -> Result<Value, String> {
    let chain = network["chain_id"].as_u64().ok_or("network chain_id")?;
    if status["chain_id"].as_u64() != Some(chain) {
        return Err("node chain differs from pinned network".into());
    }
    let identity = network["identity"].as_str().ok_or("network identity")?;
    let group = match network.get("group") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().and_then(|n| u16::try_from(n).ok()).ok_or("network group")?,
    };
    let set = ValidatorSet::from_hex(identity).map_err(|e| format!("identity: {e}"))?.with_group(group);
    let height = account["height"].as_u64().ok_or("account height")?;
    let certified_height = height.checked_add(1).ok_or("account height overflow")?;
    if certified_height < minimum_height {
        return Err("finalized blocks never go back".into());
    }
    if finalized["height"].as_u64() != Some(height) {
        return Err("finalized answer is for another height".into());
    }
    let block = from_hex(finalized["block"].as_str().ok_or("finalized block")?).map_err(|e| e.to_string())?;
    let certificate = from_hex(finalized["finalization"].as_str().ok_or("finalization")?).map_err(|e| e.to_string())?;
    let links = finalized["links"].as_array().ok_or("finalized links")?
        .iter().map(|v| from_hex(v.as_str().ok_or("link")?).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, String>>()?;
    let anchor = verify_finalized_chain(&set, &block, &certificate, &links).map_err(|e| format!("certificate: {e}"))?;
    if anchor.height != certified_height {
        return Err("certificate is for another height".into());
    }
    if now_ms.saturating_sub(anchor.timestamp_ms) > 10 * 60 * 1000 {
        return Err("stale certificate".into());
    }
    for bytes in std::iter::once(block.as_slice()).chain(links.iter().map(Vec::as_slice)) {
        let decoded = aether_light::block::Block::decode_cfg(bytes, &aether_light::block::Block::codec_config(aether_light::MAX_BLOCK_BYTES))
            .map_err(|e| format!("block: {e}"))?;
        let payload = decoded.payload().ok_or("block payload")?;
        if payload.txs.iter().any(|tx| tx.header.chain_id != chain)
            || payload.upgrade.iter().any(|u| u.upgrade.chain_id != chain) {
            return Err("certificate commits to another chain".into());
        }
    }
    let a: Address = address.parse().map_err(|_| "address")?;
    if account["address"].as_str().is_none_or(|s| s.parse::<Address>().ok() != Some(a)) {
        return Err("account answer is for another address".into());
    }
    let proof: Proof = serde_json::from_value(account["proof"].clone()).map_err(|e| format!("proof: {e}"))?;
    let data = verify_account(&anchor, &a, &proof).map_err(|e| format!("proof: {e}"))?.unwrap_or_default();
    if account["state_root"].as_str() != Some(&anchor.parent_state_root.to_string()) {
        return Err("account state root differs from certificate".into());
    }
    let claimed_balance: U256 = serde_json::from_value(account["balance"].clone()).map_err(|_| "account balance")?;
    if claimed_balance != data.balance || account["nonce"].as_u64() != Some(data.nonce) {
        return Err("account fields differ from proof".into());
    }
    Ok(json!({ "address": a.to_checksum(None), "balance_wei": data.balance.to_string(),
        "nonce": data.nonce, "state_height": height, "certified_block": anchor.height,
        "timestamp_ms": anchor.timestamp_ms }))
}

fn check_receipt_transaction(payload: &aether_light::block::Payload, index: usize, receipt: &aether_execution::Receipt) -> Result<(), String> {
    let tx = payload.txs.get(index).ok_or("receipt index is outside the certified transaction list")?;
    if receipt.tx_hash != aether_execution::tx_hash(tx) {
        return Err("receipt transaction differs from certified transaction".into());
    }
    Ok(())
}

/// Verify a receipt answer against its own certified block and the pinned network.
/// Certified inclusion remains valid regardless of the receipt block timestamp.
pub fn verified_receipt(
    network: &Value,
    status: &Value,
    answer: &Value,
    finalized: &Value,
    minimum_height: u64,
    _now_ms: u64,
) -> Result<Value, String> {
    let chain = network["chain_id"].as_u64().ok_or("network chain_id")?;
    if status["chain_id"].as_u64() != Some(chain) {
        return Err("node chain differs from pinned network".into());
    }
    let group = match network.get("group") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().and_then(|n| u16::try_from(n).ok()).ok_or("network group")?,
    };
    let set = ValidatorSet::from_hex(network["identity"].as_str().ok_or("network identity")?)
        .map_err(|e| format!("identity: {e}"))?.with_group(group);
    if answer.get("certified_block") != Some(finalized) {
        return Err("receipt certified block differs from finalized answer".into());
    }
    let height = answer["height"].as_u64().ok_or("receipt height")?;
    if height < minimum_height {
        return Err("finalized blocks never go back".into());
    }
    if finalized["height"].as_u64() != Some(height) {
        return Err("finalized answer is for another height".into());
    }
    let block = from_hex(finalized["block"].as_str().ok_or("finalized block")?).map_err(|e| e.to_string())?;
    let certificate = from_hex(finalized["finalization"].as_str().ok_or("finalization")?).map_err(|e| e.to_string())?;
    let links = finalized["links"].as_array().ok_or("finalized links")?
        .iter().map(|v| from_hex(v.as_str().ok_or("link")?).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, String>>()?;
    let anchor = verify_finalized_chain(&set, &block, &certificate, &links).map_err(|e| format!("certificate: {e}"))?;
    if anchor.height != height {
        return Err("certificate is for another height".into());
    }
    for bytes in std::iter::once(block.as_slice()).chain(links.iter().map(Vec::as_slice)) {
        let decoded = aether_light::block::Block::decode_cfg(bytes, &aether_light::block::Block::codec_config(aether_light::MAX_BLOCK_BYTES))
            .map_err(|e| format!("block: {e}"))?;
        let payload = decoded.payload().ok_or("block payload")?;
        if payload.txs.iter().any(|tx| tx.header.chain_id != chain)
            || payload.upgrade.iter().any(|u| u.upgrade.chain_id != chain) {
            return Err("certificate commits to another chain".into());
        }
    }
    let index = answer["index"].as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("receipt index")?;
    let receipt: aether_execution::Receipt = serde_json::from_value(answer["receipt"].clone())
        .map_err(|e| format!("receipt: {e}"))?;
    let proof: aether_execution::receipt::ReceiptProof = serde_json::from_value(answer["proof"].clone())
        .map_err(|e| format!("proof: {e}"))?;
    aether_light::verify_receipt(&anchor, index, &receipt, &proof).map_err(|e| format!("proof: {e}"))?;
    let decoded = aether_light::block::Block::decode_cfg(block.as_slice(), &aether_light::block::Block::codec_config(aether_light::MAX_BLOCK_BYTES))
        .map_err(|e| format!("block: {e}"))?;
    check_receipt_transaction(&decoded.payload().ok_or("block payload")?, index, &receipt)?;
    Ok(json!({ "receipt": receipt, "index": index, "height": height,
        "certified_block": anchor.height, "timestamp_ms": anchor.timestamp_ms }))
}

// ---- JavaScript API (JSON strings in and out) ----

/// Verify one account answer. All JSON is untrusted except bundled `network_json`.
#[wasm_bindgen(js_name = verifyAccount)]
pub fn verify_account_js(network_json: &str, status_json: &str, account_json: &str,
    finalized_json: &str, address: &str, minimum_height: u64, now_ms: u64) -> Result<String, JsError> {
    let network = serde_json::from_str(network_json).map_err(err)?;
    let status = serde_json::from_str(status_json).map_err(err)?;
    let account = serde_json::from_str(account_json).map_err(err)?;
    let finalized = serde_json::from_str(finalized_json).map_err(err)?;
    Ok(verified_account(&network, &status, &account, &finalized, address, minimum_height, now_ms).map_err(err)?.to_string())
}

/// Verify a receipt inclusion answer. Only bundled `network_json` is trusted.
#[wasm_bindgen(js_name = verifyReceipt)]
pub fn verify_receipt_js(network_json: &str, status_json: &str, receipt_json: &str,
    finalized_json: &str, minimum_height: u64, now_ms: u64) -> Result<String, JsError> {
    let network = serde_json::from_str(network_json).map_err(err)?;
    let status = serde_json::from_str(status_json).map_err(err)?;
    let receipt = serde_json::from_str(receipt_json).map_err(err)?;
    let finalized = serde_json::from_str(finalized_json).map_err(err)?;
    Ok(verified_receipt(&network, &status, &receipt, &finalized, minimum_height, now_ms).map_err(err)?.to_string())
}

/// Prepare the owner's account-bound ERC-1271 message for an EIP-712 v4 request.
/// The caller checks current account/implementation code with
/// `accountSigningSupport` before asking WebCrypto to sign.
pub fn prepare_typed_message(
    public_key: &[u8],
    typed_json: &str,
    expected_chain: u64,
) -> Result<Value, String> {
    let account = address_of(&p256_key(public_key)?).map_err(|e| e.to_string())?;
    let prepared = typed_data::prepare(typed_json, expected_chain, account)?;
    Ok(json!({
        "chain_id": prepared.chain_id,
        "account": prepared.account.to_checksum(None),
        "signing_message": alloy_primitives::hex::encode_prefixed(prepared.signing_message),
        "digest_hex": prepared.digest.to_string(),
        "typed_data": prepared.typed_data,
    }))
}

pub fn attach_typed_signature(
    typed_json: &str,
    expected_chain: u64,
    account: &str,
    signature: &[u8],
    public_key: &[u8],
) -> Result<String, String> {
    let account = account
        .parse()
        .map_err(|_| "invalid signing account address")?;
    typed_data::attach(typed_json, expected_chain, account, signature, public_key)
}

pub fn account_signing_support(account_code: &str, implementation_code: &str) -> bool {
    typed_data::supports(account_code, implementation_code)
}

#[wasm_bindgen(js_name = prepareTypedMessage)]
pub fn prepare_typed_message_js(
    public_key: &[u8],
    typed_json: &str,
    expected_chain: u64,
) -> Result<String, JsError> {
    Ok(
        prepare_typed_message(public_key, typed_json, expected_chain)
            .map_err(err)?
            .to_string(),
    )
}

#[wasm_bindgen(js_name = attachTypedSignature)]
pub fn attach_typed_signature_js(
    typed_json: &str,
    expected_chain: u64,
    account: &str,
    signature: &[u8],
    public_key: &[u8],
) -> Result<String, JsError> {
    attach_typed_signature(typed_json, expected_chain, account, signature, public_key).map_err(err)
}

#[wasm_bindgen(js_name = accountSigningSupport)]
pub fn account_signing_support_js(account_code: &str, implementation_code: &str) -> bool {
    account_signing_support(account_code, implementation_code)
}

/// Checksummed account address for a P-256 public key (raw SEC1 bytes).
#[wasm_bindgen(js_name = accountAddress)]
pub fn account_address_js(public_key: &[u8]) -> Result<String, JsError> {
    address_for(public_key).map_err(err)
}

#[wasm_bindgen(js_name = publicKeyFromSecret)]
pub fn public_key_from_secret_js(secret: &[u8]) -> Result<Vec<u8>, JsError> {
    public_key_from_secret(secret).map_err(err)
}

/// `request_json`: `{to, value_wei, data, gas, balance_wei?}`; `status_json`: the `aether_status` result.
#[wasm_bindgen(js_name = prepareTx)]
pub fn prepare_tx_js(public_key: &[u8], status_json: &str, expected_chain: u64, nonce: u64, request_json: &str) -> Result<String, JsError> {
    let status: Value = serde_json::from_str(status_json).map_err(err)?;
    let r: Value = serde_json::from_str(request_json).map_err(err)?;
    let req = Request {
        to: r["to"].as_str().unwrap_or(""),
        value_wei: r["value_wei"].as_str().unwrap_or("0"),
        data_hex: r["data"].as_str().unwrap_or("0x"),
        gas_limit: r["gas"].as_u64().unwrap_or(0),
        balance_wei: r["balance_wei"].as_str().unwrap_or(""),
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
    use aether_light::verify_finalized;
    use p256::ecdsa::{signature::Signer as _, SigningKey};

    fn key() -> SigningKey {
        SigningKey::from_slice(&[7u8; 32]).unwrap()
    }

    #[test]
    fn browser_typed_message_adapter_returns_contract_ready_values() {
        let k = key();
        let public = pubkey(&k);
        let data = crate::typed_data_tests::mail().to_string();
        let prepared = prepare_typed_message(&public, &data, 1).unwrap();
        assert_eq!(prepared["chain_id"], 1);
        assert_eq!(prepared["account"], address_for(&public).unwrap());
        assert_eq!(prepared["typed_data"], crate::typed_data_tests::mail());
        let bytes = alloy_primitives::hex::decode(prepared["signing_message"].as_str().unwrap()).unwrap();
        let signature: p256::ecdsa::Signature = k.sign(&bytes);
        let packed = attach_typed_signature(&data, 1, prepared["account"].as_str().unwrap(), &signature.to_bytes(), &public).unwrap();
        assert_eq!(alloy_primitives::hex::decode(packed).unwrap().len(), 128);
    }

    #[test]
    fn captured_account_requires_certificate_proof_and_height_floor() {
        let f: Value = serde_json::from_str(include_str!("../../light/tests/fixtures/devnet4.json")).unwrap();
        let set = ValidatorSet::devnet(4);
        let block = from_hex(f["anchor_block"].as_str().unwrap()).unwrap();
        let certificate = from_hex(f["anchor_finalization"].as_str().unwrap()).unwrap();
        let anchor = verify_finalized(&set, &block, &certificate).unwrap();
        let network = json!({ "chain_id": 7777, "identity": set.identity_hex() });
        let status = json!({ "chain_id": 7777 });
        let account = json!({ "address": f["address"], "height": f["height"],
            "state_root": f["state_root"], "balance": f["balance"], "nonce": 0, "proof": f["proof"] });
        let finalized = json!({ "height": f["height"], "block": f["anchor_block"],
            "finalization": f["anchor_finalization"], "links": [] });
        let address = f["address"].as_str().unwrap();
        let result = verified_account(&network, &status, &account, &finalized, address, 0, anchor.timestamp_ms).unwrap();
        assert_eq!(result["balance_wei"], "4242");
        assert_eq!(result["certified_block"], 6);
        assert!(verified_account(&network, &status, &account, &finalized, address, 7, anchor.timestamp_ms).is_err());
        let mut bad = account.clone();
        bad["balance"] = json!("0xffff");
        assert!(verified_account(&network, &status, &bad, &finalized, address, 0, anchor.timestamp_ms).is_err());
        let mut bad = status.clone();
        bad["chain_id"] = json!(7780);
        assert!(verified_account(&network, &bad, &account, &finalized, address, 0, anchor.timestamp_ms).is_err());
        let mut bad = account.clone();
        bad["proof"] = f["other_proof"].clone();
        assert!(verified_account(&network, &status, &bad, &finalized, address, 0, anchor.timestamp_ms).is_err());
        assert!(verified_account(&network, &status, &account, &finalized, address, 0,
            anchor.timestamp_ms + 10 * 60 * 1000 + 1).is_err());
        let mut bad = finalized.clone();
        bad["finalization"] = json!("00");
        assert!(verified_account(&network, &status, &account, &bad, address, 0, anchor.timestamp_ms).is_err());
    }

    #[test]
    fn receipt_answers_require_a_certificate_height_and_committed_root() {
        use aether_execution::{receipt::receipt_proof, Receipt};
        use aether_types::B256;
        let f: Value = serde_json::from_str(include_str!("../../light/tests/fixtures/devnet4.json")).unwrap();
        let set = ValidatorSet::devnet(4);
        let block = from_hex(f["anchor_block"].as_str().unwrap()).unwrap();
        let certificate = from_hex(f["anchor_finalization"].as_str().unwrap()).unwrap();
        let anchor = verify_finalized(&set, &block, &certificate).unwrap();
        let receipt = Receipt { tx_hash: B256::repeat_byte(1), success: true, gas_used: 21_000,
            prove_gas: 0, state_gas: 0, state_fee: U256::ZERO, contract_address: None,
            logs: 0, output: Bytes::new(), events: vec![] };
        let proof = receipt_proof(std::slice::from_ref(&receipt), 0).unwrap();
        let network = json!({"chain_id": 7777, "identity": set.identity_hex()});
        let status = json!({"chain_id": 7777});
        let mut answer = json!({"height": anchor.height, "index": 0, "receipt": receipt, "proof": proof});
        let finalized = json!({"height": anchor.height, "block": f["anchor_block"],
            "finalization": f["anchor_finalization"], "links": []});
        answer["certified_block"] = finalized.clone();
        let check = |answer: &Value, finalized: &Value, minimum| {
            verified_receipt(&network, &status, answer, finalized, minimum, anchor.timestamp_ms)
        };
        assert!(check(&answer, &finalized, 0).unwrap_err().contains("RootNotCommitted"));
        // This legacy certificate has no receipt commitment, but its age must
        // not reject historical inclusion before commitment verification.
        assert!(verified_receipt(&network, &status, &answer, &finalized, 0, u64::MAX)
            .unwrap_err().contains("RootNotCommitted"));
        assert!(check(&answer, &finalized, anchor.height + 1).unwrap_err().contains("never go back"));
        let mut wrong_height = answer.clone();
        wrong_height["height"] = json!(anchor.height - 1);
        assert!(check(&wrong_height, &finalized, 0).unwrap_err().contains("another height"));
        let mut bad_certificate = finalized.clone();
        bad_certificate["finalization"] = json!("00");
        let mut bad_answer = answer.clone();
        bad_answer["certified_block"] = bad_certificate.clone();
        assert!(check(&bad_answer, &bad_certificate, 0).unwrap_err().contains("certificate"));
        assert!(check(&answer, &bad_certificate, 0).unwrap_err().contains("differs"));
    }

    #[test]
    fn verify_receipt_js_round_trips_a_certified_receipt_and_rejects_forged_logs() {
        let f: Value = serde_json::from_str(include_str!("../../light/tests/fixtures/receipt-devnet4.json")).unwrap();
        let network = json!({"chain_id": f["chain_id"], "identity": f["identity"]});
        let status = json!({"chain_id": f["chain_id"]});
        let finalized = json!({"height": f["height"], "block": f["block"],
            "finalization": f["finalization"], "links": []});
        let mut answer = json!({"height": f["height"], "index": 0,
            "receipt": f["receipt"], "proof": f["proof"], "certified_block": finalized});
        verified_receipt(&network, &status, &answer, &finalized, 0, u64::MAX).unwrap();
        let verified = verify_receipt_js(&network.to_string(), &status.to_string(),
            &answer.to_string(), &finalized.to_string(), 0, u64::MAX).unwrap();
        let verified: Value = serde_json::from_str(&verified).unwrap();
        assert_eq!(verified["receipt"], f["receipt"]);
        assert_eq!(verified["certified_block"], f["height"]);

        answer["receipt"]["logs"] = json!(1);
        assert!(verified_receipt(&network, &status, &answer, &finalized, 0, u64::MAX).is_err());
        answer["receipt"] = f["receipt"].clone();
        answer["proof"]["count"] = json!(2);
        assert!(verified_receipt(&network, &status, &answer, &finalized, 0, u64::MAX).is_err());
    }

    #[test]
    fn receipt_index_and_hash_must_match_the_certified_transaction() {
        use aether_execution::Receipt;
        use aether_types::B256;
        let req = Request { to: "", value_wei: "0", data_hex: "0x6000", gas_limit: 0, balance_wei: "" };
        let prepared = prepare_tx(&pubkey(&key()), &status(), 7780, 0, &req).unwrap();
        let tx: TxEnvelope = serde_json::from_value(prepared["envelope"].clone()).unwrap();
        let receipt = Receipt { tx_hash: aether_execution::tx_hash(&tx), success: true, gas_used: 21_000,
            prove_gas: 0, state_gas: 0, state_fee: U256::ZERO, contract_address: None,
            logs: 0, output: Bytes::new(), events: vec![] };
        let payload = aether_light::block::Payload { txs: vec![tx], ..Default::default() };
        let mut historical_anchor = aether_light::VerifiedBlock { height: 6, digest: String::new(),
            timestamp_ms: 1, parent_state_root: B256::ZERO, receipts_root: None, history_root: B256::ZERO };
        historical_anchor.receipts_root = Some(aether_execution::receipt::receipt_root(std::slice::from_ref(&receipt)));
        let proof = aether_execution::receipt::receipt_proof(std::slice::from_ref(&receipt), 0).unwrap();
        aether_light::verify_receipt(&historical_anchor, 0, &receipt, &proof).unwrap();
        check_receipt_transaction(&payload, 0, &receipt).unwrap();
        assert!(check_receipt_transaction(&payload, 1, &receipt).is_err());
        let forged = Receipt { tx_hash: B256::ZERO, ..receipt };
        assert!(check_receipt_transaction(&payload, 0, &forged).unwrap_err().contains("differs"));
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
        let req = Request { to: "0x00000000000000000000000000000000000000aa", value_wei: "1000", data_hex: "0x", gas_limit: 0, balance_wei: "" };
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
        let req = Request { to: "0x00000000000000000000000000000000000000aa", value_wei: "0", data_hex: "0xa9059cbb", gas_limit: 50_000, balance_wei: "" };
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
        let req = Request { to: "", value_wei: "0", data_hex: "0x6000", gas_limit: 0, balance_wei: "" };
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

    #[test]
    fn a_zero_base_fee_or_zero_balance_sends_with_no_tip() {
        // At the zero floor (below target load): cap 0, tip 0 — a new account pays nothing.
        assert_eq!(fee_caps(&status(), None), (FeeVector { exec: 0, state: 0, prove: 0 }, 0));
        assert_eq!(fee_caps(&status(), Some(U256::ZERO)), (FeeVector { exec: 0, state: 0, prove: 0 }, 0));
        // A funded sender at the floor still tips nothing (nothing to tip over).
        assert_eq!(fee_caps(&status(), Some(U256::from(1u8))), (FeeVector { exec: 0, state: 0, prove: 0 }, 0));
        // Congested: base 2 gwei — funded keeps the 1 gwei tip, a zero balance
        // caps at base × 2 with tip 0.
        let busy = json!({"chain_id": 7780, "base_fee": {"exec": "2000000000", "prove": "0"}});
        let (funded, tip) = fee_caps(&busy, Some(U256::from(1_000_000_000_000_000_000u64)));
        assert_eq!((funded.exec, tip), (5_000_000_000, GWEI));
        let (broke, tip) = fee_caps(&busy, Some(U256::ZERO));
        assert_eq!((broke.exec, tip), (4_000_000_000, 0));
        // The envelope carries the rule (and the chain's own check accepts it).
        let k = key();
        let p = prepare_tx(&pubkey(&k), &status(), 7780, 0, &Request { to: "0x00000000000000000000000000000000000000aa", value_wei: "0", data_hex: "0x", gas_limit: 0, balance_wei: "0" }).unwrap();
        let env: TxEnvelope = serde_json::from_value(p["envelope"].clone()).unwrap();
        assert_eq!((env.header.max_fee.exec, env.header.tip), (0, 0));
        let msg = alloy_primitives::hex::decode(p["signing_message"].as_str().unwrap()).unwrap();
        let sig: p256::ecdsa::Signature = k.sign(&msg);
        let signed = attach(&p["envelope"], &sig.to_bytes(), &pubkey(&k)).unwrap();
        validate_stateless(&serde_json::from_value(signed).unwrap(), 7780).expect("zero-tip tx is valid as built");
    }

    #[test]
    fn new_genesis_wallet_signs_a_state_budget_only_when_funded() {
        let status = json!({"chain_id": 7801, "base_fee": {"exec": "0", "state": "1000000000000", "prove": "0"}});
        let k = key();
        let pk = pubkey(&k);
        let contract = Request { to: "0x7777777777777777777777777777777777777777", value_wei: "0", data_hex: "0x01", gas_limit: 100_000, balance_wei: "1000000000000000000" };
        let prepared = prepare_tx(&pk, &status, 7801, 0, &contract).unwrap();
        // 600 units for the call's possible new state plus, since audit 6
        // (A6-1/A6-2), one unit per 32 persisted transaction/receipt bytes.
        let state = prepared["envelope"]["header"]["gas"]["state"].as_u64().unwrap();
        assert!((601..700).contains(&state), "state budget {state}");
        assert_eq!(prepared["envelope"]["header"]["max_fee"]["state"].as_u64(), Some(1_000_000_000_000));
        let free = Request { to: "0x7777777777777777777777777777777777777777", value_wei: "0", data_hex: "", gas_limit: 21_000, balance_wei: "0" };
        let prepared = prepare_tx(&pk, &status, 7801, 0, &free).unwrap();
        assert_eq!(prepared["envelope"]["header"]["gas"]["state"], 0);
    }
}
