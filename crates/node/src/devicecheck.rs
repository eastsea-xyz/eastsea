//! Device registration with Apple DeviceCheck (docs/design/12-launch-plan.md
//! step 7, C1/C4).
//!
//! The Aether app on a Mac sends a DeviceCheck token (Apple-signed, bound to the
//! physical device and our team's apps). The registrar asks Apple whether this
//! device was registered before (per-device bits that survive reinstalls), and
//! if not, marks it. One Mac = one node identity, the anchor of the contribution
//! rank. The key (.p8, "DeviceCheck" key of the team) never leaves the registrar.

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
    /// The request is not signed by the voting key it registers.
    Ownership,
    Apple(String),
}

impl std::fmt::Display for DeviceCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceCheckError::InvalidToken(m) => write!(f, "device token rejected by Apple: {m}"),
            DeviceCheckError::AlreadyRegistered => write!(f, "this Mac already has a registered node"),
            DeviceCheckError::Ownership => write!(f, "not signed by the voting key being registered"),
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
        let text = r.text().await.unwrap_or_default();
        match status.as_u16() {
            200 => Ok(text),
            400 => Err(DeviceCheckError::InvalidToken(text)),
            _ => Err(DeviceCheckError::Apple(format!("{status}: {text}"))),
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

/// Registrar: DeviceCheck plus the registry.
pub struct Registrar {
    /// Apple DeviceCheck; `None` only on a local devnet (every device is new).
    pub apple: Option<DeviceCheck>,
    pub registry: Registry,
    /// Signs attestations the CommitteeRegistry contract checks (its key is in genesis).
    pub signer: crate::faucet::Faucet,
    pub chain_id: u64,
}

/// What the registrar returns: the attestation the candidate submits on chain.
#[derive(Debug, PartialEq, Eq)]
pub struct Attestation {
    pub r: [u8; 32],
    pub s: [u8; 32],
    pub registered_at: u64,
}

impl Registrar {
    /// One voting key per Mac: the device registers once (DeviceCheck), then the
    /// registrar attests (operator, voting key, node id, beaconer) for the registry.
    /// `ownership` is the voting key's own signature over the same data: nobody
    /// can register (and so squat) a voting key they do not hold.
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
        let key_hex = hex::encode(validator_key);
        let node = hex::encode(node_id);
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let apple = DeviceCheck { key, key_id: "K".into(), team: "T".into(), base: "http://127.0.0.1:9".into(), http: reqwest::Client::new() };
        let signer = crate::faucet::Faucet::from_seed(&[4u8; 32]).unwrap();
        let r = Registrar { apple: Some(apple), registry: Registry::open(path.clone()), signer, chain_id: 7 };
        let voting = commonware_cryptography::ed25519::PrivateKey::from_seed(5);
        let vk: [u8; 32] = voting.public_key().encode().as_ref().try_into().unwrap();
        let (op, other, beacon) = (aether_types::Address::repeat_byte(1), aether_types::Address::repeat_byte(9), aether_types::Address::repeat_byte(3));
        let own = |signer: &commonware_cryptography::ed25519::PrivateKey, op| {
            signer.sign(OWNERSHIP_NAMESPACE, &aether_execution::registry::attestation_message(7, op, vk, [2; 32], beacon)).encode().to_vec()
        };
        // A key known before bindings were kept (Apple not asked again: unreachable here).
        let t = r.registry.insert(&hex::encode(vk));
        let a = r.register("tok", op, vk, [2; 32], beacon, &own(&voting, op)).await.unwrap();
        assert_eq!(a.registered_at, t);
        assert!(Registry::open(path).get(&hex::encode(vk)).is_some_and(|(at, b)| at == t && b.is_some()), "bound and kept across restarts");
        // Same key, another operator: refused, even with a valid ownership signature.
        assert_eq!(r.register("tok", other, vk, [2; 32], beacon, &own(&voting, other)).await, Err(DeviceCheckError::AlreadyRegistered));
        // Not signed by the voting key: refused before anything else.
        let impostor = commonware_cryptography::ed25519::PrivateKey::from_seed(6);
        assert_eq!(r.register("tok", op, vk, [2; 32], beacon, &own(&impostor, op)).await, Err(DeviceCheckError::Ownership));
        // A new key needs Apple: the unreachable endpoint fails closed.
        let fresh = commonware_cryptography::ed25519::PrivateKey::from_seed(8);
        let fk: [u8; 32] = fresh.public_key().encode().as_ref().try_into().unwrap();
        let sig = fresh.sign(OWNERSHIP_NAMESPACE, &aether_execution::registry::attestation_message(7, op, fk, [2; 32], beacon)).encode().to_vec();
        assert!(r.register("tok", op, fk, [2; 32], beacon, &sig).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
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
