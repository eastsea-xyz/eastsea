//! Device registration with Apple DeviceCheck (docs/design/12-launch-plan.md
//! step 7, C1/C4).
//!
//! The Aether app on a Mac sends a DeviceCheck token (Apple-signed, bound to the
//! physical device and our team's apps). The registrar asks Apple whether this
//! device was registered before (per-device bits that survive reinstalls), and
//! if not, marks it. One Mac = one node identity, the anchor of the contribution
//! rank. The key (.p8, "DeviceCheck" key of the team) never leaves the registrar.

use aether_net::registrar::{EncryptionKey, RecipientSecret, TokenEnvelope};
use base64::Engine as _;
use p256::ecdsa::{signature::Signer as _, Signature, SigningKey};
use p256::pkcs8::DecodePrivateKey as _;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

const PRODUCTION: &str = "https://api.devicecheck.apple.com/v1";

#[derive(Debug, PartialEq, Eq)]
pub enum DeviceCheckError {
    /// Apple does not know this token (not from a genuine device / our team's app).
    InvalidToken(String),
    /// This device already registered a node.
    AlreadyRegistered,
    /// Another registration with this device token is still in flight.
    InProgress,
    /// The request is not signed by the voting key it registers.
    Ownership,
    /// Re-attestation: this device (or voting key) never registered.
    NotRegistered,
    /// Re-attestation: this voting key, or this device token, already
    /// re-attested this period.
    RateLimited,
    Apple(String),
}

impl std::fmt::Display for DeviceCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceCheckError::InvalidToken(m) => write!(f, "device token rejected by Apple: {m}"),
            DeviceCheckError::AlreadyRegistered => write!(f, "this Mac already has a registered node"),
            DeviceCheckError::InProgress => write!(f, "another registration with this device token is in flight"),
            DeviceCheckError::RateLimited => write!(f, "this key or device token already re-attested this period"),
            DeviceCheckError::Ownership => write!(f, "not signed by the voting key being registered"),
            DeviceCheckError::NotRegistered => write!(f, "this Mac or voting key never registered"),
            DeviceCheckError::Apple(m) => write!(f, "DeviceCheck: {m}"),
        }
    }
}

pub struct DeviceCheck {
    key: SigningKey,
    key_id: String,
    team: String,
    base: String,
    http: reqwest::Client,
}

fn b64url(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

impl DeviceCheck {
    /// `key_path`: the team's DeviceCheck key (.p8, PKCS#8 PEM).
    pub fn load(key_path: &std::path::Path, key_id: &str, team: &str) -> Result<Self, String> {
        let pem = std::fs::read_to_string(key_path).map_err(|e| format!("{}: {e}", key_path.display()))?;
        let key = SigningKey::from_pkcs8_pem(&pem).map_err(|e| format!("{}: {e}", key_path.display()))?;
        Ok(DeviceCheck { key, key_id: key_id.into(), team: team.into(), base: PRODUCTION.into(), http: reqwest::Client::new() })
    }

    /// ES256 JWT Apple expects (valid up to an hour; one per request is fine).
    fn jwt(&self) -> String {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let header = b64url(json!({ "alg": "ES256", "kid": self.key_id }).to_string().as_bytes());
        let claims = b64url(json!({ "iss": self.team, "iat": now }).to_string().as_bytes());
        let input = format!("{header}.{claims}");
        let sig: Signature = self.key.sign(input.as_bytes());
        format!("{input}.{}", b64url(&sig.to_bytes()))
    }

    async fn call(&self, path: &str, mut body: Value) -> Result<String, DeviceCheckError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default();
        body["transaction_id"] = json!(format!("{now:x}-{:x}", rand::random::<u64>()));
        body["timestamp"] = json!(now);
        let r = self
            .http
            .post(format!("{}/{path}", self.base))
            .bearer_auth(self.jwt())
            .json(&body)
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| DeviceCheckError::Apple(e.to_string()))?;
        let status = r.status();
        match status.as_u16() {
            200 => r.text().await.map_err(|_| DeviceCheckError::Apple("unreadable Apple response".into())),
            // Upstream bodies can echo identifying fields. They must never be
            // reflected in registrar RPC errors or candidate activity logs.
            400 => Err(DeviceCheckError::InvalidToken("Apple rejected the request".into())),
            _ => Err(DeviceCheckError::Apple(format!("Apple returned HTTP {}", status.as_u16()))),
        }
    }

    /// Whether this device already registered (bit 0).
    pub async fn is_registered(&self, device_token: &str) -> Result<bool, DeviceCheckError> {
        let text = self.call("query_two_bits", json!({ "device_token": device_token })).await?;
        // Apple answers plain text when the bits were never set.
        Ok(serde_json::from_str::<Value>(&text).ok().and_then(|v| v["bit0"].as_bool()).unwrap_or(false))
    }

    /// Register this device once: refuses a device that registered before.
    pub async fn register(&self, device_token: &str) -> Result<(), DeviceCheckError> {
        if self.is_registered(device_token).await? {
            return Err(DeviceCheckError::AlreadyRegistered);
        }
        self.call("update_two_bits", json!({ "device_token": device_token, "bit0": true, "bit1": false })).await.map(|_| ())
    }
}

/// Namespace of the voting key's proof that it asks to be registered.
pub const OWNERSHIP_NAMESPACE: &[u8] = b"aether-candidate-ownership";

/// What a registration attests besides the voting key: an existing key is
/// attested again only for the same operator, node and beacon account.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Binding {
    pub at: u64,
    pub operator: aether_types::Address,
    pub node: String,
    pub beaconer: aether_types::Address,
}

/// Registered voting keys (hex), kept in `<data>/registrations.json`. Older
/// files hold only a registration time per key (no binding yet).
pub struct Registry {
    path: std::path::PathBuf,
    entries: std::sync::Mutex<std::collections::BTreeMap<String, Value>>,
}

impl Registry {
    pub fn open(path: std::path::PathBuf) -> Self {
        let entries = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Registry { path, entries: std::sync::Mutex::new(entries) }
    }

    /// Registration time and, when recorded, what it was bound to.
    pub fn get(&self, node_key: &str) -> Option<(u64, Option<Binding>)> {
        let g = self.entries.lock().expect("registry lock");
        let v = g.get(node_key)?;
        match v.as_u64() {
            Some(at) => Some((at, None)),
            None => serde_json::from_value::<Binding>(v.clone()).ok().map(|b| (b.at, Some(b))),
        }
    }

    pub fn insert(&self, node_key: &str) -> u64 {
        self.bind(node_key, None)
    }

    fn bind(&self, node_key: &str, binding: Option<(aether_types::Address, String, aether_types::Address)>) -> u64 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let at = self.get(node_key).map(|(t, _)| t).unwrap_or(now);
        let value = match binding {
            Some((operator, node, beaconer)) => serde_json::to_value(Binding { at, operator, node, beaconer }).expect("binding serializes"),
            None => json!(at),
        };
        let mut g = self.entries.lock().expect("registry lock");
        g.insert(node_key.to_string(), value);
        if let Ok(b) = serde_json::to_vec_pretty(&*g) {
            let _ = std::fs::write(&self.path, b);
        }
        at
    }
}

/// Re-attestations already signed: one per voting key per period, and one per
/// device token hash per period across all keys (a borrowed token keeps at
/// most one key alive a day). Pruned to the last two periods — the RPC only
/// asks for the current one. In memory: a restart re-allows at most one more
/// re-attestation per key and token for the running period.
#[derive(Default)]
struct ReattestLedger {
    keys: std::collections::HashSet<(String, u64)>,
    tokens: std::collections::HashSet<([u8; 32], u64)>,
}

impl ReattestLedger {
    fn spent(&self, key: &str, token: &[u8; 32], period: u64) -> bool {
        self.keys.contains(&(key.to_string(), period)) || self.tokens.contains(&(*token, period))
    }

    fn record(&mut self, key: &str, token: &[u8; 32], period: u64) {
        self.keys.insert((key.to_string(), period));
        self.tokens.insert((*token, period));
        self.keys.retain(|(_, p)| *p + 2 >= period);
        self.tokens.retain(|(_, p)| *p + 2 >= period);
    }
}

/// Registrar: DeviceCheck plus the registry.
pub struct Registrar {
    /// Apple DeviceCheck; `None` only on a local devnet (every device is new).
    pub apple: Option<DeviceCheck>,
    pub registry: Registry,
    /// Signs attestations the CommitteeRegistry contract checks (its key is in genesis).
    /// The file key on devnets, the Secure Enclave helper on mainnet
    /// (crate::registrar_signer, docs/ops/registrar.md).
    pub signer: std::sync::Arc<dyn crate::registrar_signer::RegistrarSigner>,
    pub chain_id: u64,
    /// In-memory encryption key, attested by the chain signing key. The latter
    /// stays signing-only, including when it is a Secure Enclave helper.
    encryption: RecipientSecret,
    encryption_descriptor: std::sync::OnceLock<EncryptionKey>,
    /// Held across every Apple-touching path (query → update → record, the
    /// re-attestation limits): two registrations racing with different keys
    /// must not both pass Apple's query before either updates the bits.
    gate: tokio::sync::Mutex<()>,
    /// Device token hashes with a registration in flight: a second one is
    /// rejected at once instead of queueing behind the gate.
    in_flight: std::sync::Mutex<std::collections::HashSet<[u8; 32]>>,
    /// Already-signed re-attestations (the two rate limits above).
    reattests: std::sync::Mutex<ReattestLedger>,
}

/// What the registrar returns: the attestation the candidate submits on chain.
#[derive(Debug, PartialEq, Eq)]
pub struct Attestation {
    pub r: [u8; 32],
    pub s: [u8; 32],
    pub registered_at: u64,
}

impl Registrar {
    pub fn new(apple: Option<DeviceCheck>, registry: Registry, signer: std::sync::Arc<dyn crate::registrar_signer::RegistrarSigner>, chain_id: u64) -> Self {
        let encryption = loop {
            let seed = p256::elliptic_curve::zeroize::Zeroizing::new(rand::random::<[u8; 32]>());
            if let Ok(key) = RecipientSecret::from_seed(&seed) { break key; }
        };
        Registrar {
            apple,
            registry,
            signer,
            chain_id,
            encryption,
            encryption_descriptor: std::sync::OnceLock::new(),
            gate: tokio::sync::Mutex::new(()),
            in_flight: Default::default(),
            reattests: Default::default(),
        }
    }

    pub fn encryption_key(&self) -> Result<EncryptionKey, String> {
        if let Some(key) = self.encryption_descriptor.get() { return Ok(key.clone()); }
        let key = EncryptionKey::signed(self.chain_id, &self.encryption.public_key(), |msg| {
            self.signer.sign_bytes(msg).map(|(r, s)| [r, s].concat())
        })?;
        // Do not cache transient signing-helper failures. Concurrent initial
        // discoveries may sign the same recipient point, with equivalent
        // valid signatures; retain whichever completed first.
        let _ = self.encryption_descriptor.set(key);
        Ok(self.encryption_descriptor.get().expect("key initialized").clone())
    }

    /// Only the registrar decrypts. Every public param, the method and the
    /// chain are authenticated with the token; relays just validate its bound.
    pub fn open_token(&self, method: &str, params: &Value) -> Result<p256::elliptic_curve::zeroize::Zeroizing<String>, String> {
        let envelope = encrypted_token(params)?;
        self.encryption.open(&envelope, &token_context(self.chain_id, method, params)?)
    }

    /// One voting key per Mac: the device registers once (DeviceCheck), then the
    /// registrar attests (operator, voting key, node id, beaconer) for the registry.
    /// `ownership` is the voting key's own signature over the same data: nobody
    /// can register (and so squat) a voting key they do not hold.
    ///
    /// The whole Apple query → update → record path runs under `gate`, and a
    /// token with a registration already in flight is rejected at once: two
    /// requests from one Mac (the same token, or a refreshed value that Apple
    /// maps to the same device bits) cannot both register.
    #[allow(clippy::too_many_arguments)]
    pub async fn register(
        &self,
        device_token: &str,
        operator: aether_types::Address,
        validator_key: [u8; 32],
        node_id: [u8; 32],
        beaconer: aether_types::Address,
        ownership: &[u8],
    ) -> Result<Attestation, DeviceCheckError> {
        use commonware_codec::DecodeExt as _;
        use commonware_cryptography::Verifier as _;
        let msg = aether_execution::registry::attestation_message(self.chain_id, operator, validator_key, node_id, beaconer);
        let pk = commonware_cryptography::ed25519::PublicKey::decode(validator_key.as_slice()).map_err(|_| DeviceCheckError::Ownership)?;
        let sig = commonware_cryptography::ed25519::Signature::decode(ownership).map_err(|_| DeviceCheckError::Ownership)?;
        if !pk.verify(OWNERSHIP_NAMESPACE, &msg, &sig) {
            return Err(DeviceCheckError::Ownership);
        }
        if aether_net::EndpointId::from_bytes(&node_id).is_err() {
            return Err(DeviceCheckError::InvalidToken("node id is not a valid iroh id".into()));
        }
        let key_hex = hex::encode(validator_key);
        let node = hex::encode(node_id);
        let token: [u8; 32] = *blake3::hash(device_token.as_bytes()).as_bytes();
        if !self.in_flight.lock().expect("in-flight set").insert(token) {
            return Err(DeviceCheckError::InProgress);
        }
        let out = async {
            let _gate = self.gate.lock().await;
            let registered_at = match self.registry.get(&key_hex) {
                Some((_, Some(b))) if (b.operator, b.node.as_str(), b.beaconer) != (operator, node.as_str(), beaconer) => {
                    return Err(DeviceCheckError::AlreadyRegistered);
                }
                Some((at, Some(_))) => at,
                Some((_, None)) => self.registry.bind(&key_hex, Some((operator, node, beaconer))),
                None => {
                    if let Some(apple) = &self.apple {
                        apple.register(device_token).await?;
                    }
                    self.registry.bind(&key_hex, Some((operator, node, beaconer)))
                }
            };
            let (r, s) = self.signer.sign_bytes(&msg).map_err(DeviceCheckError::Apple)?;
            Ok(Attestation { r, s, registered_at })
        }
        .await;
        self.in_flight.lock().expect("in-flight set").remove(&token);
        out
    }

    /// Daily re-attestation (docs/design/15-node-rewards.md, A): the Mac holding
    /// `validator_key` sends a fresh DeviceCheck token; Apple must accept it as
    /// a genuine device of our app that registered before (bit 0), and the
    /// voting key must sign the request. The registrar then signs
    /// `reattest_message(chain, key, period)`, which validators check against
    /// the registrar key in the registry, as the contract checks registrations.
    /// The Apple call happens here, never in block validation.
    ///
    /// Limit: DeviceCheck gives no device identifier, so this proves "a genuine
    /// registered Mac running our app holds this voting key today", not which
    /// Mac; it stops keys copied to machines without DeviceCheck (servers, VMs).
    /// A key thief holding another real Mac's token can still get this
    /// signature (docs/design/15-node-rewards.md, re-attestation limits) —
    /// bounded by two rate limits, both under `gate` so concurrent requests
    /// cannot slip past them: one re-attestation per voting key per period, and
    /// one per device token hash per period across all keys (a borrowed token
    /// keeps at most one key alive a day). Both count only successes: a request
    /// Apple refused can be retried with a fresh token.
    pub async fn reattest(&self, device_token: &str, validator_key: [u8; 32], period: u64, ownership: &[u8]) -> Result<([u8; 32], [u8; 32]), DeviceCheckError> {
        use commonware_codec::DecodeExt as _;
        use commonware_cryptography::Verifier as _;
        let msg = aether_rewards::beacons::reattest_message(self.chain_id, &validator_key, period);
        let pk = commonware_cryptography::ed25519::PublicKey::decode(validator_key.as_slice()).map_err(|_| DeviceCheckError::Ownership)?;
        let sig = commonware_cryptography::ed25519::Signature::decode(ownership).map_err(|_| DeviceCheckError::Ownership)?;
        if !pk.verify(OWNERSHIP_NAMESPACE, &msg, &sig) {
            return Err(DeviceCheckError::Ownership);
        }
        let key_hex = hex::encode(validator_key);
        if self.registry.get(&key_hex).is_none() {
            return Err(DeviceCheckError::NotRegistered);
        }
        let token: [u8; 32] = *blake3::hash(device_token.as_bytes()).as_bytes();
        let _gate = self.gate.lock().await;
        if self.reattests.lock().expect("reattest ledger").spent(&key_hex, &token, period) {
            return Err(DeviceCheckError::RateLimited);
        }
        if let Some(apple) = &self.apple {
            if !apple.is_registered(device_token).await? {
                return Err(DeviceCheckError::NotRegistered);
            }
        }
        let signed = self.signer.sign_bytes(&msg).map_err(DeviceCheckError::Apple)?;
        self.reattests.lock().expect("reattest ledger").record(&key_hex, &token, period);
        Ok(signed)
    }
}

/// Parse and bound param 0 BEFORE any registrar hop; plaintext is never a
/// compatible fallback. The envelope remains intact when a validator relays.
pub fn encrypted_token(params: &Value) -> Result<TokenEnvelope, String> {
    let value = params.get(0).ok_or("missing encrypted DeviceCheck token")?;
    // Reject large fields before serde clones their contents. Relay validation
    // has the same token bound as the registrar's decryption path.
    if value["recipient"].as_str().is_none_or(|s| s.len() != 66)
        || value["ephemeral"].as_str().is_none_or(|s| s.len() != 66)
        || value["nonce"].as_str().is_none_or(|s| s.len() != 24)
        || value["ciphertext"].as_str().is_none_or(|s| s.len() > 2 * (aether_net::registrar::MAX_TOKEN_BYTES + 16)) {
        return Err("param 0 must be a bounded encrypted DeviceCheck token".into());
    }
    let envelope: TokenEnvelope = serde_json::from_value(value.clone()).map_err(|_| "param 0 must be an encrypted DeviceCheck token")?;
    envelope.validate()?;
    Ok(envelope)
}

fn token_context(chain_id: u64, method: &str, params: &Value) -> Result<Vec<u8>, String> {
    let params = params.as_array().ok_or("DeviceCheck params must be an array")?;
    let public = serde_json::to_vec(&params[1..]).map_err(|_| "invalid DeviceCheck params")?;
    Ok(aether_net::registrar::request_context(chain_id, method, &public))
}

/// The node (candidate loop and development CLI) encrypts before any RPC.
/// `registrar` is the key from authenticated finalized state/configuration.
pub fn encrypt_token_request(token: &str, descriptor: &EncryptionKey, chain_id: u64, registrar: &aether_crypto::PublicKey, method: &str, public_params: Vec<Value>) -> Result<Value, String> {
    let key = descriptor.authenticate(chain_id, registrar)?;
    let context = aether_net::registrar::request_context(chain_id, method, &serde_json::to_vec(&public_params).map_err(|_| "invalid DeviceCheck params")?);
    let seed = p256::elliptic_curve::zeroize::Zeroizing::new(rand::random::<[u8; 32]>());
    let envelope = aether_net::registrar::seal(token, &key, &context, &seed, rand::random())?;
    let mut params = vec![serde_json::to_value(envelope).map_err(|_| "invalid encrypted DeviceCheck token")?];
    params.extend(public_params);
    Ok(Value::Array(params))
}

/// Whether this node's registrar key can still sign attestations the chain would
/// accept. A committee-signed upgrade can replace the registrar key or stop it
/// by writing zeros (docs/design/14-registration.md 4); the registry's slots 0
/// and 1 are what the contract verifies and what validators check, so a node
/// whose own key is no longer there must not hand out attestations that are
/// dead on arrival. `mine` is the key this node signs with (x‖y hex).
pub fn registrar_key_check(state: &aether_execution::WorldState, mine: &str) -> Result<(), String> {
    let (x, y) = aether_execution::registry::registrar(state);
    if x == [0u8; 32] && y == [0u8; 32] {
        return Err("the registrar is stopped on chain (no key at genesis, or the committee zeroed it): no new registrations".into());
    }
    let onchain = format!("{}{}", hex::encode(x), hex::encode(y));
    if !onchain.eq_ignore_ascii_case(mine.trim()) {
        return Err(format!(
            "the registrar key in the registry is {onchain}, not this node's: it was rotated by a committee upgrade, and this node must switch to the new key (docs/ops/registrar.md)"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registrar_signer::RegistrarSigner as _;
    use aether_test_support::Port;

    /// A devnet file signer (seed 4): the key `<data>/registrar.key` holds on a
    /// local devnet, and the P-256 key these tests verify against.
    fn dev_signer() -> std::sync::Arc<dyn crate::registrar_signer::RegistrarSigner> {
        std::sync::Arc::new(crate::registrar_signer::FileSigner::from_seed(&[4u8; 32]).unwrap())
    }

    /// A loopback Apple: per-device bits (a token's device is the part before
    /// ':', as Apple maps every token of one Mac to the same bits), call
    /// counters, and a delay before each query so concurrent registrations
    /// really overlap.
    struct MockApple {
        bits: std::sync::Mutex<std::collections::HashMap<String, bool>>,
        queries: std::sync::atomic::AtomicUsize,
        updates: std::sync::atomic::AtomicUsize,
        delay_ms: u64,
    }

    fn device_of(token: &str) -> String {
        token.split(':').next().unwrap_or_default().to_string()
    }

    async fn mock_query(axum::extract::State(a): axum::extract::State<std::sync::Arc<MockApple>>, axum::Json(b): axum::Json<Value>) -> axum::Json<Value> {
        a.queries.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tokio::time::sleep(std::time::Duration::from_millis(a.delay_ms)).await;
        let token = b["device_token"].as_str().unwrap_or_default().to_string();
        let bit0 = a.bits.lock().unwrap().get(&device_of(&token)).copied().unwrap_or(false);
        axum::Json(json!({ "bit0": bit0, "bit1": false }))
    }

    async fn mock_update(axum::extract::State(a): axum::extract::State<std::sync::Arc<MockApple>>, axum::Json(b): axum::Json<Value>) -> &'static str {
        a.updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let token = b["device_token"].as_str().unwrap_or_default().to_string();
        a.bits.lock().unwrap().insert(device_of(&token), b["bit0"].as_bool().unwrap_or(false));
        ""
    }

    /// Serves the two DeviceCheck calls; returns the base URL and the state.
    async fn mock_apple(delay_ms: u64) -> (String, std::sync::Arc<MockApple>) {
        let a = std::sync::Arc::new(MockApple {
            bits: std::sync::Mutex::new(std::collections::HashMap::new()),
            queries: 0.into(),
            updates: 0.into(),
            delay_ms,
        });
        let app = axum::Router::new()
            .route("/v1/query_two_bits", axum::routing::post(mock_query))
            .route("/v1/update_two_bits", axum::routing::post(mock_update))
            .with_state(a.clone());
        let port = Port::reserve().expect("reserve mock DeviceCheck port");
        let listener = port.bind_tcp().unwrap();
        listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _port = port;
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}/v1"), a)
    }

    fn mock_devicecheck(base: String) -> DeviceCheck {
        DeviceCheck {
            key: SigningKey::from_slice(&[3u8; 32]).unwrap(),
            key_id: "K".into(),
            team: "T".into(),
            base,
            http: reqwest::Client::new(),
        }
    }

    #[tokio::test]
    async fn apple_rejection_bodies_never_return_to_a_relay_or_activity_log() {
        async fn echo(axum::extract::State(status): axum::extract::State<axum::http::StatusCode>, axum::Json(request): axum::Json<Value>) -> (axum::http::StatusCode, String) {
            (status, request.to_string())
        }
        for status in [axum::http::StatusCode::BAD_REQUEST, axum::http::StatusCode::INTERNAL_SERVER_ERROR] {
            let app = axum::Router::new().route("/query_two_bits", axum::routing::post(echo)).with_state(status);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let dc = mock_devicecheck(base);
            let token = "PRIVATE_DEVICECHECK_TOKEN_that_Apple_might_echo";
            let err = dc.is_registered(token).await.unwrap_err();
            assert!(!err.to_string().contains(token));
            assert!(!format!("{err:?}").contains(token));
            assert!(!err.to_string().contains("transaction_id"));
            task.abort();
            let _ = task.await;
        }
    }

    #[test]
    fn jwt_is_es256_with_team_and_key_id() {
        let key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let dc = DeviceCheck { key, key_id: "KID123".into(), team: "TEAM45".into(), base: PRODUCTION.into(), http: reqwest::Client::new() };
        let jwt = dc.jwt();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let dec = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).unwrap();
        let header: Value = serde_json::from_slice(&dec(parts[0])).unwrap();
        let claims: Value = serde_json::from_slice(&dec(parts[1])).unwrap();
        assert_eq!((header["alg"].as_str(), header["kid"].as_str()), (Some("ES256"), Some("KID123")));
        assert_eq!(claims["iss"], "TEAM45");
        // The signature verifies with the key's public half.
        use p256::ecdsa::signature::Verifier as _;
        let sig = Signature::from_slice(&dec(parts[2])).unwrap();
        dc.key.verifying_key().verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig).unwrap();
    }

    #[tokio::test]
    async fn registry_binds_keys_and_requires_the_voting_keys_own_signature() {
        use commonware_codec::Encode as _;
        use commonware_cryptography::Signer as _;
        let dir = std::env::temp_dir().join(format!("aether-registry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("registrations.json");
        let key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let upstream = Port::reserve().expect("reserve unreachable DeviceCheck port");
        let apple = DeviceCheck { key, key_id: "K".into(), team: "T".into(), base: format!("http://{}", upstream.addr()), http: reqwest::Client::new() };
        let signer = dev_signer();
        let r = Registrar::new(Some(apple), Registry::open(path.clone()), signer, 7);
        let node: [u8; 32] = *aether_net::SecretKey::from_bytes(&[2; 32]).public().as_bytes();
        let voting = commonware_cryptography::ed25519::PrivateKey::from_seed(5);
        let vk: [u8; 32] = voting.public_key().encode().as_ref().try_into().unwrap();
        let (op, other, beacon) = (aether_types::Address::repeat_byte(1), aether_types::Address::repeat_byte(9), aether_types::Address::repeat_byte(3));
        let own = |signer: &commonware_cryptography::ed25519::PrivateKey, op| {
            signer.sign(OWNERSHIP_NAMESPACE, &aether_execution::registry::attestation_message(7, op, vk, node, beacon)).encode().to_vec()
        };
        // A key known before bindings were kept (Apple not asked again: unreachable here).
        let t = r.registry.insert(&hex::encode(vk));
        let a = r.register("tok", op, vk, node, beacon, &own(&voting, op)).await.unwrap();
        assert_eq!(a.registered_at, t);
        assert!(Registry::open(path).get(&hex::encode(vk)).is_some_and(|(at, b)| at == t && b.is_some()), "bound and kept across restarts");
        // Same key, another operator: refused, even with a valid ownership signature.
        assert_eq!(r.register("tok", other, vk, node, beacon, &own(&voting, other)).await, Err(DeviceCheckError::AlreadyRegistered));
        // Not signed by the voting key: refused before anything else.
        let impostor = commonware_cryptography::ed25519::PrivateKey::from_seed(6);
        assert_eq!(r.register("tok", op, vk, node, beacon, &own(&impostor, op)).await, Err(DeviceCheckError::Ownership));
        // A new key needs Apple: the unreachable endpoint fails closed.
        let fresh = commonware_cryptography::ed25519::PrivateKey::from_seed(8);
        let fk: [u8; 32] = fresh.public_key().encode().as_ref().try_into().unwrap();
        let sig = fresh.sign(OWNERSHIP_NAMESPACE, &aether_execution::registry::attestation_message(7, op, fk, node, beacon)).encode().to_vec();
        assert!(r.register("tok", op, fk, node, beacon, &sig).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn reattestation_needs_the_voting_key_a_known_registration_and_apple() {
        use commonware_codec::Encode as _;
        use commonware_cryptography::Signer as _;
        let dir = std::env::temp_dir().join(format!("aether-reattest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let signer = dev_signer();
        let registrar_pk = aether_crypto::Signer::public_key(&aether_crypto::P256Signer::from_seed(&[4u8; 32]).unwrap());
        let dev = Registrar::new(None, Registry::open(dir.join("r.json")), signer, 7);
        let voting = commonware_cryptography::ed25519::PrivateKey::from_seed(5);
        let vk: [u8; 32] = voting.public_key().encode().as_ref().try_into().unwrap();
        let own = |k: &commonware_cryptography::ed25519::PrivateKey, period| {
            k.sign(OWNERSHIP_NAMESPACE, &aether_rewards::beacons::reattest_message(7, &vk, period)).encode().to_vec()
        };
        assert_eq!(dev.reattest("tok", vk, 3, &own(&voting, 3)).await, Err(DeviceCheckError::NotRegistered), "unknown key");
        dev.registry.insert(&hex::encode(vk));
        assert_eq!(dev.reattest("tok", vk, 3, &own(&voting, 4)).await, Err(DeviceCheckError::Ownership), "signed for another period");
        let (r, s) = dev.reattest("tok", vk, 3, &own(&voting, 3)).await.unwrap();
        // What validators check: the registrar's P-256 signature over the message.
        let msg = aether_rewards::beacons::reattest_message(7, &vk, 3);
        aether_crypto::verify(&registrar_pk, &msg, &[r, s].concat()).unwrap();
        // With Apple configured but unreachable, it fails closed.
        let key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let upstream = Port::reserve().expect("reserve unreachable DeviceCheck port");
        let apple = DeviceCheck { key, key_id: "K".into(), team: "T".into(), base: format!("http://{}", upstream.addr()), http: reqwest::Client::new() };
        let live = Registrar { apple: Some(apple), ..dev };
        assert!(live.reattest("tok", vk, 3, &own(&voting, 3)).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One Mac, two voting keys (and two operators), concurrent registrations:
    /// the query→update path is serialized, so the second request asks Apple
    /// only after the first one's update and sees a registered device.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_registrations_of_one_mac_register_one_key() {
        use commonware_codec::Encode as _;
        use commonware_cryptography::Signer as _;
        use std::sync::atomic::Ordering;
        let dir = std::env::temp_dir().join(format!("aether-concurrent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A slow query: without the gate both requests pass it before either update.
        let (base, apple) = mock_apple(200).await;
        let r = std::sync::Arc::new(Registrar::new(
            Some(mock_devicecheck(base)),
            Registry::open(dir.join("r.json")),
            dev_signer(),
            7,
        ));
        let node: [u8; 32] = *aether_net::SecretKey::from_bytes(&[2; 32]).public().as_bytes();
        let (op1, op2, beacon) = (aether_types::Address::repeat_byte(1), aether_types::Address::repeat_byte(2), aether_types::Address::repeat_byte(3));
        let key = |seed: u64| {
            let k = commonware_cryptography::ed25519::PrivateKey::from_seed(seed);
            let pk: [u8; 32] = k.public_key().encode().as_ref().try_into().unwrap();
            (pk, k)
        };
        let (vk1, k1) = key(11);
        let (vk2, k2) = key(12);
        let own = |k: &commonware_cryptography::ed25519::PrivateKey, vk: [u8; 32], op| {
            k.sign(OWNERSHIP_NAMESPACE, &aether_execution::registry::attestation_message(7, op, vk, node, beacon)).encode().to_vec()
        };
        // The token rotated between the two requests: same device, different bits.
        let (r1, own1, own2) = (r.clone(), own(&k1, vk1, op1), own(&k2, vk2, op2));
        let (a, b) = tokio::join!(
            r1.register("mac1:old", op1, vk1, node, beacon, &own1),
            r.register("mac1:fresh", op2, vk2, node, beacon, &own2)
        );
        let (winner, loser) = if a.is_ok() { (vk1, b) } else { (vk2, a) };
        assert_eq!(loser, Err(DeviceCheckError::AlreadyRegistered), "the second device check of the same Mac fails");
        assert_eq!(apple.updates.load(Ordering::Relaxed), 1, "Apple's bit is set once");
        assert_eq!(apple.queries.load(Ordering::Relaxed), 2, "the loser asked after the winner's update, and saw the bit");
        let keys = hex::encode(winner);
        assert!(r.registry.get(&keys).is_some(), "the winner is recorded");
        let other = if winner == vk1 { vk2 } else { vk1 };
        assert!(r.registry.get(&hex::encode(other)).is_none(), "the loser is not: {other:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same token value twice at once: the in-flight set rejects the twin
    /// outright, so Apple is asked (and the bit set) exactly once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_duplicate_registration_in_flight_is_rejected_without_asking_apple() {
        use commonware_codec::Encode as _;
        use commonware_cryptography::Signer as _;
        use std::sync::atomic::Ordering;
        let dir = std::env::temp_dir().join(format!("aether-inflight-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (base, apple) = mock_apple(400).await;
        let r = std::sync::Arc::new(Registrar::new(
            Some(mock_devicecheck(base)),
            Registry::open(dir.join("r.json")),
            dev_signer(),
            7,
        ));
        let node: [u8; 32] = *aether_net::SecretKey::from_bytes(&[2; 32]).public().as_bytes();
        let (op, beacon) = (aether_types::Address::repeat_byte(1), aether_types::Address::repeat_byte(3));
        let k = commonware_cryptography::ed25519::PrivateKey::from_seed(11);
        let vk: [u8; 32] = k.public_key().encode().as_ref().try_into().unwrap();
        let own = k.sign(OWNERSHIP_NAMESPACE, &aether_execution::registry::attestation_message(7, op, vk, node, beacon)).encode().to_vec();
        // The first registration parks in Apple's slow query; the twin arrives while it runs.
        let (r2, own2) = (r.clone(), own.clone());
        let first = tokio::spawn(async move { r2.register("mac1:t", op, vk, node, beacon, &own2).await });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let twin = r.register("mac1:t", op, vk, node, beacon, &own).await;
        assert!(first.await.unwrap().is_ok());
        assert_eq!(twin, Err(DeviceCheckError::InProgress), "rejected while the first still runs");
        assert_eq!(apple.queries.load(Ordering::Relaxed), 1, "the twin never reached Apple");
        assert_eq!(apple.updates.load(Ordering::Relaxed), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The two re-attestation limits: one per voting key per period, one per
    /// device token hash per period across all keys (so a token borrowed from
    /// another real Mac keeps at most one key alive a day). The next period
    /// starts clean, and a refused request (Apple said no) is retryable.
    #[tokio::test]
    async fn reattest_once_per_key_and_once_per_device_token_a_period() {
        use commonware_codec::Encode as _;
        use commonware_cryptography::Signer as _;
        use std::sync::atomic::Ordering;
        let dir = std::env::temp_dir().join(format!("aether-ratelimit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (base, apple) = mock_apple(0).await;
        apple.bits.lock().unwrap().insert("mac1".into(), true); // this Mac registered
        apple.bits.lock().unwrap().insert("mac2".into(), true); // the borrowed one
        let r = Registrar::new(Some(mock_devicecheck(base)), Registry::open(dir.join("r.json")), dev_signer(), 7);
        let key = |seed: u64| {
            let k = commonware_cryptography::ed25519::PrivateKey::from_seed(seed);
            let pk: [u8; 32] = k.public_key().encode().as_ref().try_into().unwrap();
            r.registry.insert(&hex::encode(pk));
            (pk, k)
        };
        let (vk, k) = key(5);
        let (vk2, k2) = key(6);
        let (vk3, k3) = key(7);
        let own = |k: &commonware_cryptography::ed25519::PrivateKey, vk: [u8; 32], period: u64| {
            k.sign(OWNERSHIP_NAMESPACE, &aether_rewards::beacons::reattest_message(7, &vk, period)).encode().to_vec()
        };
        r.reattest("mac1:t1", vk, 3, &own(&k, vk, 3)).await.unwrap();
        let queries = apple.queries.load(Ordering::Relaxed);
        // The same key again, even with a fresh token of the same Mac: refused.
        assert_eq!(r.reattest("mac1:t2", vk, 3, &own(&k, vk, 3)).await, Err(DeviceCheckError::RateLimited));
        // Another key with the same token value: the token is spent for the period.
        assert_eq!(r.reattest("mac1:t1", vk2, 3, &own(&k2, vk2, 3)).await, Err(DeviceCheckError::RateLimited));
        assert_eq!(apple.queries.load(Ordering::Relaxed), queries, "a rate-limited retry never reaches Apple");
        // A borrowed token keeps one key alive: the first use is signed…
        r.reattest("mac2:t9", vk3, 3, &own(&k3, vk3, 3)).await.unwrap();
        // …and no second key the same day.
        assert_eq!(r.reattest("mac2:t9", vk2, 3, &own(&k2, vk2, 3)).await, Err(DeviceCheckError::RateLimited));
        // vk2 with its Mac's next token is fine, and the next period starts clean.
        r.reattest("mac1:t3", vk2, 3, &own(&k2, vk2, 3)).await.unwrap();
        r.reattest("mac1:t1", vk, 4, &own(&k, vk, 4)).await.unwrap();
        // A token Apple refuses costs nothing: retry with a fresh one works.
        assert_eq!(r.reattest("mac3:t1", vk, 5, &own(&k, vk, 5)).await, Err(DeviceCheckError::NotRegistered));
        r.reattest("mac1:t4", vk, 5, &own(&k, vk, 5)).await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A committee-signed upgrade can rotate the registrar or stop it (G11):
    /// the service must only sign with the key the registry holds, and zeros
    /// mean "stopped" (docs/design/14-registration.md 4).
    #[test]
    fn the_service_only_signs_with_the_key_the_registry_holds() {
        use aether_execution::{registry, WorldState};
        let signer = crate::registrar_signer::FileSigner::from_seed(&[4u8; 32]).unwrap();
        let seeded = aether_crypto::P256Signer::from_seed(&[4u8; 32]).unwrap();
        let mut state = WorldState::default();
        assert!(registrar_key_check(&state, &signer.public_hex()).is_err(), "a chain without a registry");
        let xy = aether_crypto::p256_xy(&aether_crypto::Signer::public_key(&seeded).bytes).unwrap();
        registry::predeploy(&mut state, xy, registry::Params::default()).unwrap();
        assert_eq!(registrar_key_check(&state, &signer.public_hex()), Ok(()));
        assert!(!registry::registrar_revoked(&state));
        // Rotated to another key: the old key must stop signing.
        registry::set_registrar(&mut state, ([5u8; 32], [6u8; 32]));
        let err = registrar_key_check(&state, &signer.public_hex()).unwrap_err();
        assert!(err.contains("rotated"), "{err}");
        // Stopped: zeros stop every attestation.
        registry::set_registrar(&mut state, ([0u8; 32], [0u8; 32]));
        assert!(registry::registrar_revoked(&state));
        let err = registrar_key_check(&state, &signer.public_hex()).unwrap_err();
        assert!(err.contains("stopped"), "{err}");
    }

    /// Live check against Apple: AETHER_DEVICECHECK_KEY=path AETHER_DEVICECHECK_KEY_ID=… AETHER_DEVICECHECK_TOKEN=base64
    #[tokio::test]
    #[ignore]
    async fn apple_accepts_a_real_token() {
        let (Ok(k), Ok(id), Ok(tok)) =
            (std::env::var("AETHER_DEVICECHECK_KEY"), std::env::var("AETHER_DEVICECHECK_KEY_ID"), std::env::var("AETHER_DEVICECHECK_TOKEN"))
        else {
            return;
        };
        let dc = DeviceCheck::load(std::path::Path::new(&k), &id, "45WU468FZE").unwrap();
        dc.is_registered(tok.trim()).await.unwrap();
        assert!(matches!(dc.is_registered("bm90IGEgdG9rZW4=").await, Err(DeviceCheckError::InvalidToken(_))));
    }
}
