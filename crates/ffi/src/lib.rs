//! Wallet core for Swift (docs/design/09-wallet.md).
//!
//! Keys never enter Rust: the app holds a Secure Enclave P-256 key, Rust builds
//! the exact bytes to sign, Swift signs them (`SecureEnclave.P256.Signing`
//! hashes with SHA-256, matching the chain's P-256 rule), and Rust normalizes
//! the signature to low-s and submits. Every balance shown is verified through
//! a finality certificate plus an EIP-7864 proof (see `aether-light`).

uniffi::setup_scaffolding!();

mod paper;
use aether_crypto::{address_of, PublicKey};
use aether_execution::EvmCall;
use aether_light::{from_hex, verify_account, verify_finalized_chain, ValidatorSet, VerifiedBlock};
use aether_state::Proof;
use aether_types::{Address, Bytes, FeeVector, GasVector, SignerScheme, TxEnvelope, TxHash, TxHeader, TxPayload, U256};
pub use paper::*;
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
    /// Estimated fee (wei) of a plain transfer at the next block's base fee plus the tip.
    pub transfer_fee_wei: String,
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

/// Devnet validators the wallet knows by id (their addresses come from the DHT).
const DEVNET_VALIDATORS: u64 = 4;

struct Net {
    rt: tokio::runtime::Runtime,
    client: aether_net::RpcClient,
}

fn net() -> R<&'static Net> {
    static NET: std::sync::OnceLock<Result<Net, String>> = std::sync::OnceLock::new();
    NET.get_or_init(|| {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
        let configured = NODES.lock().expect("nodes lock").clone();
        let ids = configured.unwrap_or_else(|| (1..=DEVNET_VALIDATORS).map(aether_net::devnet_node_id).collect());
        let client = rt.block_on(aether_net::RpcClient::new(ids)).map_err(|e| e.to_string())?;
        Ok(Net { rt, client })
    })
    .as_ref()
    .map_err(|e| WalletError::Network(e.clone()))
}

/// Fee caps from the node's next base fees: 2x headroom plus a 1 gwei tip
/// (only base + tip is charged). Older nodes without `base_fee` get the floor.
fn fee_caps(status: &Value) -> (FeeVector, u128) {
    const GWEI: u128 = 1_000_000_000;
    let get = |k: &str| status["base_fee"][k].as_str().and_then(|v| v.parse::<u128>().ok()).unwrap_or(GWEI);
    (FeeVector { exec: get("exec") * 2 + GWEI, state: 0, prove: get("prove") * 2 }, GWEI)
}

/// A 21k-gas transfer runs no bytecode (no prove gas): it pays base + tip per gas.
fn transfer_fee(status: &Value) -> u128 {
    const GWEI: u128 = 1_000_000_000;
    let base = status["base_fee"]["exec"].as_str().and_then(|v| v.parse::<u128>().ok()).unwrap_or(0);
    21_000 * (base + GWEI)
}

/// The node running on this Mac (the app's node switch), if on. Wallet reads then
/// go to it; it verifies every block itself, and the wallet still checks every
/// certificate and proof, so nothing about it has to be trusted either.
static LOCAL_NODE: std::sync::Mutex<Option<u16>> = std::sync::Mutex::new(None);

/// Use the node at 127.0.0.1:`port` (Some) or the validators over the network (None).
#[uniffi::export]
pub fn use_local_node(port: Option<u16>) {
    *LOCAL_NODE.lock().expect("local node lock") = port;
}

/// Finalized height of the node at 127.0.0.1:`port`, if it answers.
#[uniffi::export]
pub fn local_node_height(port: u16) -> Option<u64> {
    local_call(port, "aether_status", json!([])).ok().and_then(|v| v["height"].as_u64())
}

/// JSON-RPC over plain HTTP/1.1 to the local node (loopback only).
fn local_call(port: u16, method: &str, params: Value) -> R<Value> {
    use std::io::{Read, Write};
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| WalletError::Network(format!("local node: {e}")))?;
    s.set_read_timeout(Some(Duration::from_secs(30))).ok();
    write!(s, "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        .map_err(|e| WalletError::Network(format!("local node: {e}")))?;
    let mut resp = Vec::new();
    s.take(64 * 1024 * 1024).read_to_end(&mut resp).map_err(|e| WalletError::Network(format!("local node: {e}")))?;
    let split = resp.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| WalletError::Network("local node: bad response".into()))?;
    let v: Value = serde_json::from_slice(&resp[split + 4..]).map_err(|e| WalletError::Network(format!("local node: {e}")))?;
    match v.get("error") {
        Some(e) => Err(WalletError::Rejected(e["message"].as_str().unwrap_or("error").to_string())),
        None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
    }
}

fn call(method: &str, params: Value) -> R<Value> {
    // Copy the setting out first: never hold the lock across a network read.
    let local = *LOCAL_NODE.lock().expect("local node lock");
    if let Some(port) = local {
        return local_call(port, method, params);
    }
    let n = net()?;
    n.rt.block_on(n.client.call(method, params)).map_err(|e| {
        let m = e.to_string();
        if m.contains("connect") || m.contains("timed out") || m.contains("stream") {
            WalletError::Network(m)
        } else {
            WalletError::Rejected(m)
        }
    })
}

/// How the wallet currently reaches the network (for display).
#[uniffi::export]
pub fn connection() -> String {
    let local = *LOCAL_NODE.lock().expect("local node lock");
    if let Some(port) = local {
        return format!("This Mac's node (127.0.0.1:{port})");
    }
    match net() {
        Ok(n) => format!("Mainline DHT · {}", n.rt.block_on(n.client.describe())),
        Err(e) => format!("offline: {e}"),
    }
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
pub fn chain_status() -> R<ChainStatus> {
    let v = call("aether_status", json!([]))?;
    Ok(ChainStatus {
        chain_id: v["chain_id"].as_u64().unwrap_or_default(),
        height: v["height"].as_u64().unwrap_or_default(),
        state_root: v["state_root"].as_str().unwrap_or_default().to_string(),
        mempool: v["mempool"].as_u64().unwrap_or_default(),
        transfer_fee_wei: transfer_fee(&v).to_string(),
    })
}

/// Account address for a Secure Enclave P-256 public key.
#[uniffi::export]
pub fn account_address(p256_public_key: Vec<u8>) -> R<String> {
    let pk = p256_key(&p256_public_key)?;
    Ok(address_of(&pk).map_err(|e| WalletError::Invalid(e.to_string()))?.to_checksum(None))
}

fn anchor(height: u64, set: &ValidatorSet) -> R<VerifiedBlock> {
    for _ in 0..40 {
        let v = call("aether_getFinalized", json!([height + 1]))?;
        if !v.is_null() {
            let block = from_hex(v["block"].as_str().unwrap_or_default()).map_err(|e| WalletError::Verification(e.to_string()))?;
            let fin = from_hex(v["finalization"].as_str().unwrap_or_default()).map_err(|e| WalletError::Verification(e.to_string()))?;
            // A height without its own certificate comes with the blocks built on it.
            let links = v["links"]
                .as_array()
                .map(|a| a.iter().map(|l| from_hex(l.as_str().unwrap_or_default())).collect::<Result<Vec<_>, _>>())
                .transpose()
                .map_err(|e| WalletError::Verification(e.to_string()))?
                .unwrap_or_default();
            let vb = verify_finalized_chain(set, &block, &fin, &links).map_err(|e| WalletError::Verification(format!("certificate: {e}")))?;
            // A valid but old certificate would let a node replay past state (e.g. an
            // old, looser session that the owner then re-signs): require a recent one.
            let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(u64::MAX);
            if now_ms.saturating_sub(vb.timestamp_ms) > MAX_ANCHOR_AGE_MS {
                return Err(WalletError::Verification(format!("the node served state from {} s ago; refusing stale data", (now_ms - vb.timestamp_ms) / 1000)));
            }
            return Ok(vb);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(WalletError::Network(format!("block {} not finalized yet", height + 1)))
}

/// Verified state older than this is refused (clock skew and a slow network included).
const MAX_ANCHOR_AGE_MS: u64 = 10 * 60 * 1000;

static COMMITTEE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static NODES: std::sync::Mutex<Option<Vec<aether_net::EndpointId>>> = std::sync::Mutex::new(None);

/// The chain every signature is for: from network.json, else the public devnet.
/// Never taken from a node: a malicious node could otherwise collect signatures
/// valid on another network where the same account holds funds.
static CHAIN_ID: std::sync::Mutex<u64> = std::sync::Mutex::new(7_777);

fn expected_chain(status: &Value) -> R<u64> {
    let want = *CHAIN_ID.lock().expect("chain id lock");
    match status["chain_id"].as_u64() {
        Some(c) if c == want => Ok(want),
        other => Err(WalletError::Verification(format!("the node reports chain {other:?}, but this wallet is for chain {want}"))),
    }
}

/// Configure from network.json: the validators' node ids (looked up in the
/// Mainline DHT) and the committee identity to pin. Call before anything else.
#[uniffi::export]
pub fn configure_network(network_json: String) -> R<u32> {
    let v: Value = serde_json::from_str(&network_json).map_err(|e| WalletError::Invalid(format!("network.json: {e}")))?;
    let nodes = v["validators"]
        .as_array()
        .ok_or_else(|| WalletError::Invalid("network.json: validators".into()))?
        .iter()
        .map(|m| m["node"].as_str().unwrap_or_default().parse::<aether_net::EndpointId>().map_err(|e| WalletError::Invalid(format!("node id: {e}"))))
        .collect::<R<Vec<_>>>()?;
    if let Some(id) = v["identity"].as_str() {
        set_committee_identity(id.to_string())?;
    }
    if let Some(c) = v["chain_id"].as_u64() {
        *CHAIN_ID.lock().expect("chain id lock") = c;
    }
    let n = nodes.len() as u32;
    *NODES.lock().expect("nodes lock") = Some(nodes);
    Ok(n)
}

/// Pin the committee identity (hex, printed by `aether dkg`) that finality
/// certificates must verify under. Without it, the devnet dealer's identity.
#[uniffi::export]
pub fn set_committee_identity(identity_hex: String) -> R<()> {
    ValidatorSet::from_hex(&identity_hex).map_err(|e| WalletError::Invalid(format!("identity: {e}")))?;
    *COMMITTEE.lock().expect("committee lock") = Some(identity_hex);
    Ok(())
}

fn trusted_set(validators: u32) -> R<ValidatorSet> {
    match COMMITTEE.lock().expect("committee lock").as_deref() {
        Some(hex) => ValidatorSet::from_hex(hex).map_err(|e| WalletError::Invalid(format!("identity: {e}"))),
        None => Ok(ValidatorSet::devnet(validators as u64)),
    }
}

/// Balance and nonce, verified against a validator-signed state root.
#[uniffi::export]
pub fn verified_account(address: String, validators: u32) -> R<VerifiedAccount> {
    let a: Address = address.parse().map_err(|_| WalletError::Invalid("address".into()))?;
    let v = call("aether_getAccount", json!([a]))?;
    let proof: Proof = parse(&v["proof"], "proof")?;
    let height = v["height"].as_u64().unwrap_or_default();
    let set = trusted_set(validators)?;
    let anchor = anchor(height, &set)?;
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
pub fn prepare_transfer(p256_public_key: Vec<u8>, to: String, value_wei: String) -> R<PreparedTx> {
    let to: Address = to.parse().map_err(|_| WalletError::Invalid("recipient address".into()))?;
    let value: U256 = value_wei.parse().map_err(|_| WalletError::Invalid("amount".into()))?;
    prepare(&p256_public_key, |_| Ok(EvmCall { to: Some(to), value, input: Bytes::new(), gas_limit: 21_000, delegate: None }))
}

/// One recipient of a batch.
#[derive(uniffi::Record)]
pub struct Payment {
    pub to: String,
    pub value_wei: String,
}

/// Several payments, all or nothing, under ONE signature (one Touch ID).
/// The account delegates to AetherAccount (EIP-7702) in the same tx the first
/// time; afterwards it just calls its own `execute`.
#[uniffi::export]
pub fn prepare_batch(p256_public_key: Vec<u8>, payments: Vec<Payment>) -> R<PreparedTx> {
    if payments.is_empty() {
        return Err(WalletError::Invalid("no payments".into()));
    }
    let calls = payments
        .iter()
        .map(|p| {
            let to: Address = p.to.parse().map_err(|_| WalletError::Invalid(format!("recipient {}", p.to)))?;
            let v: U256 = p.value_wei.parse().map_err(|_| WalletError::Invalid(format!("amount {}", p.value_wei)))?;
            Ok((to, v, Bytes::new()))
        })
        .collect::<R<Vec<_>>>()?;
    prepare(&p256_public_key, |from| {
        Ok(EvmCall {
            to: Some(from),
            value: U256::ZERO,
            input: aether_execution::encode_execute(&calls),
            gas_limit: 60_000 + 40_000 * calls.len() as u64,
            delegate: Some(aether_execution::AETHER_ACCOUNT),
        })
    })
}

fn hex_lower(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn prepare(p256_public_key: &[u8], body: impl FnOnce(Address) -> R<EvmCall>) -> R<PreparedTx> {
    let pk = p256_key(p256_public_key)?;
    let from = address_of(&pk).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let status = call("aether_status", json!([]))?;
    let chain_id = expected_chain(&status)?;
    let (max_fee, tip) = fee_caps(&status);
    let nonce_hex = call("eth_getTransactionCount", json!([from]))?;
    let nonce = u64::from_str_radix(nonce_hex.as_str().unwrap_or("0x0").trim_start_matches("0x"), 16).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let call_body = body(from)?;
    let payload = call_body.encode();
    let header = TxHeader {
        chain_id,
        sender: from,
        nonce,
        // Every interpreted instruction costs at least 1 gas, so prove steps <= gas_limit.
        gas: GasVector { exec: call_body.gas_limit, state: 0, prove: call_body.gas_limit },
        max_fee,
        tip,
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

/// This Mac as a voting node: the registry's view of it (display only; the
/// numbers are not proven against a certificate).
#[derive(uniffi::Record)]
pub struct VotingNodeStatus {
    /// Registered in the voting-node registry.
    pub registered: bool,
    /// Consecutive epochs with a liveness beacon (the contribution rank).
    pub streak: u64,
    pub last_epoch: u64,
    pub epoch: u64,
    /// In the current voting set (building and signing blocks).
    pub voting: bool,
    /// Registered candidates on the network.
    pub candidates: u32,
}

#[uniffi::export]
pub fn voting_node_status(validator_key: String) -> R<VotingNodeStatus> {
    let key = validator_key.trim_start_matches("0x").to_lowercase();
    let v = call("aether_candidates", json!([]))?;
    let list = v["candidates"].as_array().cloned().unwrap_or_default();
    let mine = list.iter().find(|c| c["validator_key"].as_str() == Some(key.as_str()));
    let voting = call("aether_network", json!([]))
        .ok()
        .and_then(|n| n["validators"].as_array().map(|a| a.iter().any(|m| m["key"].as_str() == Some(key.as_str()))))
        .unwrap_or(false);
    Ok(VotingNodeStatus {
        registered: mine.is_some(),
        streak: mine.and_then(|c| c["streak"].as_u64()).unwrap_or(0),
        last_epoch: mine.and_then(|c| c["last_epoch"].as_u64()).unwrap_or(0),
        epoch: v["epoch"].as_u64().unwrap_or(0),
        voting,
        candidates: list.len() as u32,
    })
}

/// Register this Mac as a voting-node candidate, operated by the wallet's account.
/// `device_token` is Apple's DeviceCheck token (base64): one Mac, one candidate.
/// `ownership` is the voting key's own signature (`aether candidate-info
/// --operator`), so nobody can register a voting key they do not hold.
/// The registrar (a validator holding the network's DeviceCheck key) attests;
/// the returned transaction, signed with the wallet key, puts it on chain.
#[uniffi::export]
pub fn prepare_register_node(
    p256_public_key: Vec<u8>,
    device_token: String,
    validator_key: String,
    node_id: String,
    beaconer: String,
    ownership: String,
) -> R<PreparedTx> {
    let hex32 = |s: &str, what: &str| -> R<[u8; 32]> {
        aether_light::from_hex(s).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| WalletError::Invalid(format!("{what}: 32-byte hex")))
    };
    let (key, node) = (hex32(&validator_key, "voting key")?, hex32(&node_id, "node id")?);
    let beaconer: Address = beaconer.parse().map_err(|_| WalletError::Invalid("beaconer address".into()))?;
    let operator = address_of(&p256_key(&p256_public_key)?).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let a = registrar_call(json!([device_token, operator, hex_lower(&key), hex_lower(&node), beaconer, ownership]))?;
    let (r, s) = (hex32(a["r"].as_str().unwrap_or_default(), "attestation r")?, hex32(a["s"].as_str().unwrap_or_default(), "attestation s")?);
    let input = aether_execution::registry::encode_register(key, node, beaconer, r, s);
    prepare(&p256_public_key, |_| Ok(EvmCall { to: Some(aether_execution::registry::REGISTRY), value: U256::ZERO, input, gas_limit: 400_000, delegate: None }))
}

/// Ask validators in turn until the one running the registrar answers.
fn registrar_call(params: Value) -> R<Value> {
    let n = net()?;
    let mut last = WalletError::Network("no registrar reachable".into());
    for _ in 0..8 {
        match n.rt.block_on(n.client.call("aether_registerDevice", params.clone())) {
            Ok(v) => return Ok(v),
            Err(e) if e.to_string().contains("does not register devices") => {
                n.rt.block_on(n.client.rotate());
                last = WalletError::Network("no registrar reachable".into());
            }
            Err(e) => return Err(WalletError::Rejected(e.to_string())),
        }
    }
    Err(last)
}

/// Attach a Secure Enclave signature (raw r‖s, 64 bytes) and submit.
#[uniffi::export]
pub fn submit_signed(envelope_json: String, signature: Vec<u8>, p256_public_key: Vec<u8>) -> R<String> {
    let mut env: TxEnvelope = serde_json::from_str(&envelope_json).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let pk = p256_key(&p256_public_key)?;
    let sig = p256::ecdsa::Signature::from_slice(&signature).map_err(|_| WalletError::Invalid("signature must be 64-byte r‖s".into()))?;
    // Secure Enclave may return high-s; the chain only accepts low-s.
    let mut bytes = sig.normalize_s().to_bytes().to_vec();
    aether_crypto::verify(&pk, &env.signing_bytes(), &bytes).map_err(|e| WalletError::Invalid(format!("signature does not match key: {e}")))?;
    bytes.extend_from_slice(&pk.bytes);
    env.signature = Bytes::from(bytes);
    let v = call("aether_sendTransaction", json!([env]))?;
    let h: TxHash = parse(&v["hash"], "hash")?;
    Ok(format!("{h}"))
}

#[uniffi::export]
pub fn receipt(tx_hash: String) -> R<Option<TxReceipt>> {
    let h: TxHash = tx_hash.parse().map_err(|_| WalletError::Invalid("tx hash".into()))?;
    let v = call("aether_getReceipt", json!([h]))?;
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
pub fn recent_blocks(n: u32) -> R<Vec<BlockInfo>> {
    let v = call("aether_recentBlocks", json!([n]))?;
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

/// Test tokens (zero value) from the node's faucet, rate-limited by the node.
#[uniffi::export]
pub fn devnet_faucet(to: String, value_wei: String) -> R<String> {
    // The node's faucet decides the amount and rate limits; `value_wei` is kept for API compatibility.
    let _ = value_wei;
    let to: Address = to.parse().map_err(|_| WalletError::Invalid("address".into()))?;
    let v = call("aether_faucet", json!([to]))?;
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

// ---------------- recovery (guardians, delayed and cancellable) ----------------

use aether_execution::account::{self as acct, slots};

/// This device's recovery-key code: its P-256 public key as x‖y hex. Give it to
/// someone whose account this device should be able to recover.
#[uniffi::export]
pub fn recovery_key_code(p256_public_key: Vec<u8>) -> R<String> {
    let (x, y) = aether_crypto::p256_xy(&p256_public_key).map_err(|e| WalletError::Invalid(format!("{e:?}")))?;
    Ok(format!("{}{}", hex_lower(&x), hex_lower(&y)))
}

fn parse_code(code: &str) -> R<([u8; 32], [u8; 32])> {
    let b = from_hex(code.trim()).map_err(|e| WalletError::Invalid(format!("recovery key code: {e}")))?;
    if b.len() != 64 {
        return Err(WalletError::Invalid("recovery key code must be 64 bytes (x‖y)".into()));
    }
    // Reject points not on the curve before putting them on chain.
    let mut sec1 = vec![4u8];
    sec1.extend_from_slice(&b);
    aether_crypto::p256_xy(&sec1).map_err(|_| WalletError::Invalid("recovery key code is not a P-256 public key".into()))?;
    Ok((b[..32].try_into().expect("32"), b[32..].try_into().expect("32")))
}

/// A storage slot of `account`, proven against a certified state root.
fn verified_slot(account: Address, slot: U256, set: &ValidatorSet) -> R<U256> {
    let v = call("aether_getStorage", json!([account, slot]))?;
    let proof: Proof = parse(&v["proof"], "proof")?;
    let height = v["height"].as_u64().unwrap_or_default();
    let anchor = anchor(height, set)?;
    aether_light::verify_storage(&anchor, &account, slot, &proof).map_err(|e| WalletError::Verification(format!("account storage: {e}")))
}

/// Make the device with `recovery_code` able to recover this account after a
/// 48-hour delay that this account can cancel (delegates to AetherAccount first
/// if needed). Sign with the Secure Enclave and submit.
#[uniffi::export]
pub fn prepare_set_recovery_key(p256_public_key: Vec<u8>, recovery_code: String) -> R<PreparedTx> {
    let (x, y) = parse_code(&recovery_code)?;
    prepare(&p256_public_key, |from| {
        Ok(EvmCall {
            to: Some(from),
            value: U256::ZERO,
            input: aether_execution::encode_execute(&[(from, U256::ZERO, aether_execution::encode_set_guardian(x, y))]),
            gas_limit: 300_000,
            delegate: Some(aether_execution::AETHER_ACCOUNT),
        })
    })
}

/// Recovery settings and any pending recovery of an account (all verified).
#[derive(uniffi::Record)]
pub struct RecoveryStatus {
    pub guardians: u32,
    pub threshold: u8,
    pub delay_seconds: u64,
    /// A recovery was proposed and not yet run or cancelled.
    pub pending: bool,
    /// Unix time after which the pending recovery may run.
    pub ready_at: u64,
}

/// Add a recovery key (another device's code, or recovery words) next to the
/// account's other recovery keys. The contract appends on chain and keeps the
/// threshold and delay (the first key: 1-of-1, 48 h), so nothing is rewritten
/// from a possibly stale copy of the list.
#[uniffi::export]
pub fn prepare_add_recovery_key(p256_public_key: Vec<u8>, recovery_code: String) -> R<PreparedTx> {
    let (x, y) = parse_code(&recovery_code)?;
    prepare(&p256_public_key, |from| {
        Ok(EvmCall {
            to: Some(from),
            value: U256::ZERO,
            input: aether_execution::encode_execute(&[(from, U256::ZERO, aether_execution::account::encode_add_guardian(x, y))]),
            gas_limit: 300_000,
            delegate: Some(aether_execution::AETHER_ACCOUNT),
        })
    })
}

#[uniffi::export]
pub fn recovery_status(account: String, validators: u32) -> R<RecoveryStatus> {
    let a: Address = account.parse().map_err(|_| WalletError::Invalid("account address".into()))?;
    let set = trusted_set(validators)?;
    let count = verified_slot(a, slots::guardian_count(), &set)?;
    let (threshold, delay) = slots::unpack_threshold_and_delay(verified_slot(a, slots::threshold_and_delay(), &set)?);
    let pending = !verified_slot(a, slots::pending(), &set)?.is_zero();
    let ready_at = verified_slot(a, slots::ready_at(), &set)?;
    Ok(RecoveryStatus { guardians: count.to::<u32>(), threshold, delay_seconds: delay, pending, ready_at: ready_at.to::<u64>() })
}

/// A recovery this device (a guardian) proposes: move `lost`'s verified balance
/// to this device's account once the delay has passed.
#[derive(uniffi::Record)]
pub struct RecoveryRequest {
    pub lost: String,
    pub to: String,
    pub value_wei: String,
    /// Proposal nonce the signature covers.
    pub guardian_nonce: u64,
    /// This device's position in the account's guardian list.
    pub guardian_index: u8,
    pub delay_seconds: u64,
    /// Sign with this device's Secure Enclave key (SHA-256 applied by CryptoKit).
    pub message: Vec<u8>,
}

#[uniffi::export]
pub fn prepare_recovery(p256_public_key: Vec<u8>, lost_account: String, validators: u32) -> R<RecoveryRequest> {
    let me = address_of(&p256_key(&p256_public_key)?).map_err(|e| WalletError::Invalid(e.to_string()))?;
    prepare_recovery_to(p256_public_key, lost_account, me.to_checksum(None), validators)
}

/// Like `prepare_recovery`, with the funds going to `to` (a paper recovery key
/// recovers to the new Mac's account, not to an account of its own).
#[uniffi::export]
pub fn prepare_recovery_to(p256_public_key: Vec<u8>, lost_account: String, to: String, validators: u32) -> R<RecoveryRequest> {
    let me: Address = to.parse().map_err(|_| WalletError::Invalid("destination address".into()))?;
    let (mx, my) = aether_crypto::p256_xy(&p256_public_key).map_err(|e| WalletError::Invalid(format!("{e:?}")))?;
    let lost: Address = lost_account.parse().map_err(|_| WalletError::Invalid("lost account address".into()))?;
    let set = trusted_set(validators)?;
    // Balance, guardian list, threshold and proposal nonce, all proven against certified state roots.
    let account = verified_account(lost.to_checksum(None), validators)?;
    let count = verified_slot(lost, slots::guardian_count(), &set)?.to::<u64>().min(8); // the contract allows at most 8
    let (threshold, delay) = slots::unpack_threshold_and_delay(verified_slot(lost, slots::threshold_and_delay(), &set)?);
    if count == 0 {
        return Err(WalletError::Invalid("that account has no recovery devices".into()));
    }
    let index = (0..count)
        .find(|&i| {
            let x = verified_slot(lost, slots::guardian(i), &set).ok().map(|v| v.to_be_bytes::<32>());
            let y = verified_slot(lost, slots::guardian(i) + U256::from(1u64), &set).ok().map(|v| v.to_be_bytes::<32>());
            x == Some(mx) && y == Some(my)
        })
        .ok_or_else(|| WalletError::Invalid("this device is not a recovery device of that account".into()))?;
    if threshold > 1 {
        return Err(WalletError::Invalid(format!(
            "that account needs {threshold} recovery devices to sign; use `aether recover` with each device's signature"
        )));
    }
    if !verified_slot(lost, slots::pending(), &set)?.is_zero() {
        return Err(WalletError::Invalid("a recovery of that account is already pending".into()));
    }
    let nonce = verified_slot(lost, slots::recovery_nonce(), &set)?.to::<u64>();
    let value: U256 = account.balance_wei.parse().map_err(|_| WalletError::Invalid("balance".into()))?;
    let chain_id = expected_chain(&call("aether_status", json!([]))?)?;
    let calls = [(me, value, Bytes::new())];
    Ok(RecoveryRequest {
        lost: lost.to_checksum(None),
        to: me.to_checksum(None),
        value_wei: value.to_string(),
        guardian_nonce: nonce,
        guardian_index: index as u8,
        delay_seconds: delay,
        message: acct::recovery_message(chain_id, lost, nonce, &calls),
    })
}

fn request_calls(request: &RecoveryRequest) -> R<(Address, Vec<aether_execution::AccountCall>)> {
    let lost: Address = request.lost.parse().map_err(|_| WalletError::Invalid("lost".into()))?;
    let to: Address = request.to.parse().map_err(|_| WalletError::Invalid("to".into()))?;
    let value: U256 = request.value_wei.parse().map_err(|_| WalletError::Invalid("value".into()))?;
    Ok((lost, vec![(to, value, Bytes::new())]))
}

/// The tx that proposes a signed recovery; this device pays the gas (sign it too).
/// The funds move only when `prepare_finish_recovery` runs after the delay.
#[uniffi::export]
pub fn prepare_recovery_submit(p256_public_key: Vec<u8>, request: RecoveryRequest, guardian_signature: Vec<u8>) -> R<PreparedTx> {
    let (lost, calls) = request_calls(&request)?;
    let sig = normalize_p256(&guardian_signature)?;
    let (r, s): ([u8; 32], [u8; 32]) = (sig[..32].try_into().expect("32"), sig[32..64].try_into().expect("32"));
    let input = acct::encode_propose_recovery(&calls, &[(request.guardian_index, r, s)]);
    prepare(&p256_public_key, |_| Ok(EvmCall { to: Some(lost), value: U256::ZERO, input, gas_limit: 400_000, delegate: None }))
}

/// After the delay: run the proposed recovery (anyone may; this device pays the gas).
#[uniffi::export]
pub fn prepare_finish_recovery(p256_public_key: Vec<u8>, request: RecoveryRequest) -> R<PreparedTx> {
    let (lost, calls) = request_calls(&request)?;
    let input = acct::encode_execute_recovery(&calls);
    prepare(&p256_public_key, |_| Ok(EvmCall { to: Some(lost), value: U256::ZERO, input, gas_limit: 300_000, delegate: None }))
}

/// Stop a pending recovery of this account (e.g. one this owner did not ask for).
#[uniffi::export]
pub fn prepare_cancel_recovery(p256_public_key: Vec<u8>) -> R<PreparedTx> {
    prepare(&p256_public_key, |from| {
        Ok(EvmCall {
            to: Some(from),
            value: U256::ZERO,
            input: aether_execution::encode_execute(&[(from, U256::ZERO, acct::encode_cancel_recovery())]),
            gas_limit: 200_000,
            delegate: Some(aether_execution::AETHER_ACCOUNT),
        })
    })
}

/// Remove every recovery key and any pending recovery with them (e.g. a
/// recovery device was lost or stolen). Trusted keys are then added again.
#[uniffi::export]
pub fn prepare_remove_recovery_keys(p256_public_key: Vec<u8>) -> R<PreparedTx> {
    prepare(&p256_public_key, |from| {
        Ok(EvmCall {
            to: Some(from),
            value: U256::ZERO,
            input: aether_execution::encode_execute(&[(from, U256::ZERO, aether_execution::encode_set_guardian([0; 32], [0; 32]))]),
            gas_limit: 300_000,
            delegate: Some(aether_execution::AETHER_ACCOUNT),
        })
    })
}

fn normalize_p256(signature: &[u8]) -> R<Vec<u8>> {
    let sig = p256::ecdsa::Signature::from_slice(signature).map_err(|_| WalletError::Invalid("signature must be 64-byte r‖s".into()))?;
    Ok(sig.normalize_s().to_bytes().to_vec())
}

// ---------------- session keys (limited keys, e.g. for AI agents) ----------------

/// Session 0 of an account: its limits and use, proven against certified roots.
#[derive(uniffi::Record)]
pub struct SessionStatus {
    pub exists: bool,
    /// The session key's recovery-key-style code (x‖y hex), to match against a device key.
    pub key_code: String,
    pub per_payment_wei: String,
    pub per_day_wei: String,
    /// What it may still pay right now (per-day limit minus today's and yesterday's payments, UTC).
    pub left_wei: String,
    pub expires: u64,
    pub allow: Vec<String>,
    pub nonce: u64,
}

#[uniffi::export]
pub fn session_status(account: String, validators: u32) -> R<SessionStatus> {
    let a: Address = account.parse().map_err(|_| WalletError::Invalid("account address".into()))?;
    let set = trusted_set(validators)?;
    if verified_slot(a, slots::session_count(), &set)?.is_zero() {
        return Ok(SessionStatus {
            exists: false,
            key_code: String::new(),
            per_payment_wei: "0".into(),
            per_day_wei: "0".into(),
            left_wei: "0".into(),
            expires: 0,
            allow: vec![],
            nonce: 0,
        });
    }
    let base = slots::session(0);
    let word = |o: u64| verified_slot(a, base + U256::from(o), &set);
    let (x, y) = (word(0)?.to_be_bytes::<32>(), word(1)?.to_be_bytes::<32>());
    let (per_payment, per_day) = slots::unpack_limits(word(2)?);
    let (day, expires, spent) = slots::unpack_usage(word(3)?);
    let prev = word(4)?.to::<u128>();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
    let n_allow = word(5)?.to::<u64>().min(16);
    let allow = (0..n_allow)
        .map(|j| {
            verified_slot(a, slots::session_allow(0, j), &set).map(|v| Address::from_word(aether_types::B256::from(v.to_be_bytes::<32>())).to_checksum(None))
        })
        .collect::<R<Vec<_>>>()?;
    Ok(SessionStatus {
        exists: true,
        key_code: format!("{}{}", hex_lower(&x), hex_lower(&y)),
        per_payment_wei: per_payment.to_string(),
        per_day_wei: per_day.to_string(),
        left_wei: slots::left_now(per_day, day, spent, prev, now).to_string(),
        expires,
        allow,
        nonce: word(6)?.to::<u64>(),
    })
}

/// From the account owner's key: replace session 0 with `session_code`'s key
/// under these limits, and send `gas_wei` to the session key's own address so it
/// can pay for its transactions (which bounds what it can ever spend on gas).
/// Limits for a session key (amounts in wei).
#[derive(uniffi::Record)]
pub struct SessionSettings {
    /// The session key as x‖y hex (like a recovery-key code).
    pub session_code: String,
    pub per_payment_wei: String,
    pub per_day_wei: String,
    /// Unix seconds after which the key stops working (0 = never).
    pub expires: u64,
    /// Allowed recipients; empty = anyone.
    pub allow: Vec<String>,
    /// Sent to the session key's own address for its gas.
    pub gas_wei: String,
}

#[uniffi::export]
pub fn prepare_set_session(owner_public_key: Vec<u8>, settings: SessionSettings, validators: u32) -> R<PreparedTx> {
    let SessionSettings { session_code, per_payment_wei, per_day_wei, expires, allow, gas_wei } = settings;
    let (x, y) = parse_code(&session_code)?;
    let mut sec1 = vec![4u8];
    sec1.extend_from_slice(&x);
    sec1.extend_from_slice(&y);
    // Addresses are derived from the compressed key, as tx authentication does.
    let gas_payer = address_of(&p256_key(&sec1)?).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let amount = |v: &str, what: &str| v.parse::<u128>().map_err(|_| WalletError::Invalid(format!("{what}: {v}")));
    let limits = acct::SessionLimits {
        per_payment: amount(&per_payment_wei, "per payment")?,
        per_day: amount(&per_day_wei, "per day")?,
        expires,
        allow: allow.iter().map(|a| a.parse::<Address>().map_err(|_| WalletError::Invalid(format!("allowed recipient {a}")))).collect::<R<_>>()?,
    };
    let gas: U256 = gas_wei.parse().map_err(|_| WalletError::Invalid("gas".into()))?;
    let pk = p256_key(&owner_public_key)?;
    let owner = address_of(&pk).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let set = trusted_set(validators)?;
    let existing = verified_slot(owner, slots::session_count(), &set)?.to::<u64>();
    let mut calls: Vec<aether_execution::AccountCall> = (0..existing).map(|_| (owner, U256::ZERO, acct::encode_remove_session(0))).collect();
    calls.push((owner, U256::ZERO, acct::encode_add_session(x, y, &limits)));
    if !gas.is_zero() {
        calls.push((gas_payer, gas, Bytes::new()));
    }
    prepare(&owner_public_key, |from| {
        Ok(EvmCall {
            to: Some(from),
            value: U256::ZERO,
            input: aether_execution::encode_execute(&calls),
            gas_limit: 400_000 + 60_000 * calls.len() as u64 + 25_000 * limits.allow.len() as u64,
            delegate: Some(aether_execution::AETHER_ACCOUNT),
        })
    })
}

/// A payment a session key signs for `account` (then `prepare_session_submit`).
#[derive(uniffi::Record)]
pub struct SessionRequest {
    pub account: String,
    pub payments: Vec<Payment>,
    pub nonce: u64,
    /// Sign with the session key (SHA-256 applied by CryptoKit).
    pub message: Vec<u8>,
}

fn session_calls(payments: &[Payment]) -> R<Vec<aether_execution::AccountCall>> {
    payments
        .iter()
        .map(|p| {
            let to: Address = p.to.parse().map_err(|_| WalletError::Invalid(format!("recipient {}", p.to)))?;
            let v: U256 = p.value_wei.parse().map_err(|_| WalletError::Invalid(format!("amount {}", p.value_wei)))?;
            Ok((to, v, Bytes::new()))
        })
        .collect()
}

#[uniffi::export]
pub fn prepare_session_payment(account: String, payments: Vec<Payment>, validators: u32) -> R<SessionRequest> {
    if payments.is_empty() {
        return Err(WalletError::Invalid("no payments".into()));
    }
    let a: Address = account.parse().map_err(|_| WalletError::Invalid("account address".into()))?;
    let calls = session_calls(&payments)?;
    let set = trusted_set(validators)?;
    let nonce = verified_slot(a, slots::session(0) + U256::from(6u64), &set)?.to::<u64>();
    let id = verified_slot(a, slots::session(0) + U256::from(7u64), &set)?.to::<u64>();
    let chain_id = expected_chain(&call("aether_status", json!([]))?)?;
    Ok(SessionRequest { account: a.to_checksum(None), message: acct::session_message(chain_id, a, id, nonce, &calls), payments, nonce })
}

/// The tx the session key's own address sends (it pays the gas).
#[uniffi::export]
pub fn prepare_session_submit(session_public_key: Vec<u8>, request: SessionRequest, session_signature: Vec<u8>) -> R<PreparedTx> {
    let a: Address = request.account.parse().map_err(|_| WalletError::Invalid("account".into()))?;
    let calls = session_calls(&request.payments)?;
    let sig = normalize_p256(&session_signature)?;
    let input = acct::encode_session_execute(&calls, 0, sig[..32].try_into().expect("32"), sig[32..64].try_into().expect("32"));
    let gas_limit = 120_000 + 40_000 * calls.len() as u64;
    prepare(&session_public_key, |_| Ok(EvmCall { to: Some(a), value: U256::ZERO, input, gas_limit, delegate: None }))
}
