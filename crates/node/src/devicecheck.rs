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
    Apple(String),
}

impl std::fmt::Display for DeviceCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceCheckError::InvalidToken(m) => write!(f, "device token rejected by Apple: {m}"),
            DeviceCheckError::AlreadyRegistered => write!(f, "this Mac already has a registered node"),
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

/// Registered node keys (hex) by registration time, kept in `<data>/registrations.json`.
pub struct Registry {
    path: std::path::PathBuf,
    entries: std::sync::Mutex<std::collections::BTreeMap<String, u64>>,
}

impl Registry {
    pub fn open(path: std::path::PathBuf) -> Self {
        let entries = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Registry { path, entries: std::sync::Mutex::new(entries) }
    }

    pub fn get(&self, node_key: &str) -> Option<u64> {
        self.entries.lock().expect("registry lock").get(node_key).copied()
    }

    pub fn insert(&self, node_key: &str) -> u64 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let mut g = self.entries.lock().expect("registry lock");
        g.insert(node_key.to_string(), now);
        if let Ok(b) = serde_json::to_vec_pretty(&*g) {
            let _ = std::fs::write(&self.path, b);
        }
        now
    }
}

/// Registrar: DeviceCheck plus the registry.
pub struct Registrar {
    pub apple: DeviceCheck,
    pub registry: Registry,
}

impl Registrar {
    /// One node key per Mac: a device registers once; a key cannot be re-registered.
    pub async fn register(&self, device_token: &str, node_key: &str) -> Result<u64, DeviceCheckError> {
        let key_ok = node_key.len() == 66 && node_key.bytes().all(|c| c.is_ascii_hexdigit());
        if !key_ok {
            return Err(DeviceCheckError::InvalidToken("node key must be a 33-byte compressed P-256 key in hex".into()));
        }
        if let Some(t) = self.registry.get(node_key) {
            return Ok(t);
        }
        self.apple.register(device_token).await?;
        Ok(self.registry.insert(node_key))
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
    async fn registry_persists_and_rejects_bad_keys_before_calling_apple() {
        let dir = std::env::temp_dir().join(format!("aether-registry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("registrations.json");
        let key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let apple = DeviceCheck { key, key_id: "K".into(), team: "T".into(), base: "http://127.0.0.1:9".into(), http: reqwest::Client::new() };
        let r = Registrar { apple, registry: Registry::open(path.clone()) };
        assert!(matches!(r.register("tok", "not-a-key").await, Err(DeviceCheckError::InvalidToken(_))));
        let node = "02".to_string() + &"ab".repeat(32);
        // An already registered key answers from the registry without asking Apple.
        let t = r.registry.insert(&node);
        assert_eq!(r.register("tok", &node).await, Ok(t));
        assert_eq!(Registry::open(path).get(&node), Some(t), "survives a restart");
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
