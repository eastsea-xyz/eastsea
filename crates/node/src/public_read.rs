//! Operator policy for `eastsea/read/1`, independent of consensus state.

use aether_net::{Endpoint, EndpointId, PublicRead, ReadBudget, Reservation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_DAILY_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub daily_bytes: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            daily_bytes: DEFAULT_DAILY_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Usage {
    day: u64,
    bytes: u64,
}

/// One instance for both ALPNs. Reservations are durable before input is
/// accepted or output is sent. Interrupted output remains fully charged.
pub struct DailyBudget {
    config: PathBuf,
    usage: PathBuf,
    state: Mutex<Result<Usage, String>>,
}

fn today() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400
}

fn read_config(path: &Path) -> Result<Config, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("public read config invalid: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(format!("public read config unreadable: {e}")),
    }
}

impl DailyBudget {
    pub fn open(data: &Path) -> Self {
        let usage = data.join("public-read-usage.json");
        let state = match std::fs::read(&usage) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("public read usage invalid: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Usage::default()),
            Err(e) => Err(format!("public read usage unreadable: {e}")),
        };
        Self {
            config: data.join("public-read.json"),
            usage,
            state: Mutex::new(state),
        }
    }

    fn persist(&self, usage: Usage) -> Result<(), String> {
        let bytes = serde_json::to_vec(&usage).map_err(|e| e.to_string())?;
        crate::atomic::replace(&self.usage, &bytes, 0o600)
    }

    pub fn config(&self) -> Result<Config, String> {
        read_config(&self.config)
    }

    pub fn used_bytes(&self) -> Result<u64, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "public read budget lock failed".to_string())?;
        let usage = state.as_ref().map_err(Clone::clone)?;
        Ok(if today() > usage.day { 0 } else { usage.bytes })
    }

    fn reserve_on(&self, maximum: usize, day: u64) -> Result<Reservation, String> {
        let config = self.config()?;
        if !config.enabled {
            return Err("public read service disabled by operator".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "public read budget lock failed".to_string())?;
        let usage = state.as_mut().map_err(|e| e.clone())?;
        // Moving the clock backwards cannot earn a second daily allowance.
        let mut next = if day > usage.day {
            Usage { day, bytes: 0 }
        } else {
            *usage
        };
        let available = config.daily_bytes.saturating_sub(next.bytes);
        let bytes = available.min(maximum as u64) as usize;
        if bytes == 0 {
            return Err("public read daily byte cap reached".into());
        }
        next.bytes += bytes as u64;
        if let Err(error) = self.persist(next) {
            *state = Err(error.clone());
            return Err(error);
        }
        *usage = next;
        Ok(Reservation {
            period: next.day,
            bytes,
        })
    }
}

impl ReadBudget for DailyBudget {
    fn reserve(&self, maximum: usize) -> Result<Reservation, String> {
        self.reserve_on(maximum, today())
    }

    fn refund(&self, reservation: Reservation, unused: usize) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "public read budget lock failed".to_string())?;
        let usage = state.as_mut().map_err(|e| e.clone())?;
        if usage.day != reservation.period {
            return Ok(());
        }
        let mut next = *usage;
        next.bytes = next
            .bytes
            .saturating_sub(unused.min(reservation.bytes) as u64);
        if let Err(error) = self.persist(next) {
            *state = Err(error.clone());
            return Err(error);
        }
        *usage = next;
        Ok(())
    }
}

struct Status {
    endpoint: Endpoint,
    budget: Arc<DailyBudget>,
}
static STATUS: OnceLock<Mutex<Option<Status>>> = OnceLock::new();

/// Diagnostic hints for finding this endpoint. These never authenticate a head.
pub fn status() -> Value {
    let status = STATUS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("public read status");
    let Some(status) = status.as_ref() else {
        return Value::Null;
    };
    let config = status.budget.config();
    let relay = status
        .endpoint
        .addr()
        .addrs
        .iter()
        .find_map(|addr| match addr {
            aether_net::TransportAddr::Relay(url) => Some(url.to_string()),
            _ => None,
        });
    json!({
        "node": status.endpoint.id().to_string(),
        "relay": relay,
        "enabled": config.as_ref().is_ok_and(|c| c.enabled),
        "daily_bytes": config.as_ref().map(|c| c.daily_bytes).unwrap_or(0),
        "used_bytes": status.budget.used_bytes().ok(),
        "alpn": "eastsea/read/1",
    })
}

/// Build the service from an authenticated network's roster. Open advertisements
/// never enter this list, and registered wallet-server admission stays separate.
pub fn service<F>(
    endpoint: &Endpoint,
    data: &Path,
    peers: Vec<EndpointId>,
    rpc_exempt: Vec<EndpointId>,
    state: F,
) -> PublicRead
where
    F: Fn() -> crate::rpc::RpcState + Send + Sync + 'static,
{
    service_when_ready(endpoint, data, peers, rpc_exempt, move || Some(state()))
}

/// Presence and app discovery may start before checkpoint restoration finishes.
/// Public reads wait for the same verified state as the wallet RPC handler.
pub fn service_when_ready<F>(
    endpoint: &Endpoint,
    data: &Path,
    peers: Vec<EndpointId>,
    rpc_exempt: Vec<EndpointId>,
    state: F,
) -> PublicRead
where
    F: Fn() -> Option<crate::rpc::RpcState> + Send + Sync + 'static,
{
    let budget = Arc::new(DailyBudget::open(data));
    *STATUS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("public read status") = Some(Status {
        endpoint: endpoint.clone(),
        budget: budget.clone(),
    });
    let mut peers = peers;
    peers.push(endpoint.id());
    peers.sort();
    peers.dedup();
    peers.truncate(32);
    PublicRead::new(
        move |req| {
            let state = state();
            let peers = peers.clone();
            async move { answer_when_ready(state, req, &peers).await }
        },
        budget,
        rpc_exempt,
    )
}

async fn answer_when_ready(state: Option<crate::rpc::RpcState>, req: Value, peers: &[EndpointId]) -> Value {
    match state {
        Some(mut state) => {
            state.public_read_only = true;
            crate::rpc::handle_public_value(&state, req, peers).await
        }
        None => json!({"jsonrpc":"2.0", "id":req.get("id").cloned().unwrap_or(Value::Null),
            "error":{"code":-32000,"message":"node is starting; try again after checkpoint sync"}}),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContactSequence {
    node: String,
    seq: i64,
}

/// Advertise the same node key under a chain/group-scoped raw BEP-44 salt.
/// Known keys can retrieve this capability record; torrent-topic enumeration
/// and a browser HTTP bridge for salted raw records remain separate work.
pub fn publish_contact(
    endpoint: Endpoint,
    data: &Path,
    chain: crate::chain::Chain,
    identity: &aether_light::Identity,
) {
    use aether_net::contact::{Contact, Publisher, chain_fingerprint, contact_salt};
    use commonware_codec::Encode;
    if std::env::var("AETHER_IROH_NO_DHT").as_deref() == Ok("1") {
        return;
    }
    let cfg = chain.cfg();
    let identity: [u8; 96] = match identity.encode().as_ref().try_into() {
        Ok(identity) => identity,
        Err(_) => {
            tracing::warn!("public read contact: identity is not canonical MinSig");
            return;
        }
    };
    let genesis = crate::block::Block::genesis_with(
        cfg.chain_id,
        cfg.genesis_state().root(),
        cfg.history_v2,
        cfg.group,
    );
    let fingerprint = chain_fingerprint(
        cfg.chain_id,
        &identity,
        &crate::chain::genesis_digest(&genesis),
    );
    let salt = contact_salt(&fingerprint, cfg.group);
    let seq_path = data.join(format!(
        "public-read-contact-{}-{}.json",
        hex::encode(fingerprint),
        cfg.group
    ));
    let config_path = data.join("public-read.json");
    tokio::spawn(async move {
        let publisher = match Publisher::new() {
            Ok(publisher) => publisher,
            Err(error) => {
                tracing::warn!(%error, "public read contact publisher unavailable");
                return;
            }
        };
        let mut seq = match std::fs::read(&seq_path) {
            Ok(bytes) => match serde_json::from_slice::<ContactSequence>(&bytes) {
                Ok(saved) if saved.node == endpoint.id().to_string() && saved.seq > 0 => saved.seq,
                _ => {
                    tracing::warn!("public read contact sequence invalid; refusing rollback");
                    return;
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => {
                tracing::warn!(%error, "public read contact sequence unreadable");
                return;
            }
        };
        // Recover the network high-water mark even when local state was lost.
        loop {
            match publisher.latest_seq(&endpoint.id(), &salt).await {
                Ok(remote) => {
                    seq = seq.max(remote);
                    break;
                }
                Err(error) => {
                    tracing::debug!(%error, "public read contact sequence lookup; retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }
            }
        }
        let mut next_publish = std::time::Instant::now();
        let mut last_addresses = None;
        let mut backoff = 5u64;
        loop {
            let addresses = endpoint.addr();
            if last_addresses.as_ref() != Some(&addresses) {
                last_addresses = Some(addresses);
                next_publish = std::time::Instant::now();
            }
            let enabled =
                read_config(&config_path).is_ok_and(|cfg| cfg.enabled && cfg.daily_bytes > 0);
            if enabled && std::time::Instant::now() >= next_publish {
                let Some(next) = seq.checked_add(1) else {
                    tracing::warn!("public read contact sequence exhausted");
                    return;
                };
                let saved = ContactSequence {
                    node: endpoint.id().to_string(),
                    seq: next,
                };
                let persisted = serde_json::to_vec(&saved)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| crate::atomic::replace(&seq_path, &bytes, 0o600));
                if let Err(error) = persisted {
                    tracing::warn!(%error, "public read contact sequence write failed");
                    return;
                }
                seq = next;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let contact = Contact::for_endpoint(
                    &endpoint,
                    fingerprint,
                    cfg.chain_id,
                    cfg.group,
                    chain.finalized_height(),
                    now,
                );
                let published = match contact.signed(endpoint.secret_key(), seq) {
                    Ok(item) => match publisher.publish(item).await {
                        Ok(()) => {
                            tracing::debug!(node=%endpoint.id(), chain=%hex::encode(fingerprint), seq, "public read contact published under network salt");
                            true
                        }
                        Err(error) => {
                            tracing::debug!(%error, "public read contact publish failed");
                            false
                        }
                    },
                    Err(error) => {
                        tracing::warn!(%error, "public read contact invalid");
                        false
                    }
                };
                let delay = if published {
                    backoff = 5;
                    rand::random_range(960..=1440)
                } else {
                    let delay = backoff;
                    backoff = (backoff * 2).min(300);
                    delay
                };
                next_publish = std::time::Instant::now() + std::time::Duration::from_secs(delay);
            } else if !enabled {
                // Re-enabling publication is picked up without a restart.
                next_publish = std::time::Instant::now();
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn startup_reads_wait_for_verified_state_and_then_keep_the_public_gate() {
        let request = json!({"jsonrpc":"2.0","id":"startup","method":"aether_status","params":[]});
        let pending = answer_when_ready(None, request.clone(), &[]).await;
        assert_eq!(pending["id"], "startup");
        assert_eq!(pending["error"]["code"], -32000);
        assert!(pending.get("result").is_none());
        let ready = answer_when_ready(Some(crate::rpc::bare_state()), request, &[]).await;
        assert_eq!(ready["result"]["height"], 0);
        let private = answer_when_ready(Some(crate::rpc::bare_state()),
            json!({"id":2,"method":"aether_appBundle","params":[]}), &[]).await;
        assert_eq!(private["error"]["code"], -32601);
    }

    fn directory(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/p2p-read/budget-tests");
        let dir = root.join(format!("{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Each test owns a distinct directory; no system /tmp access.
        for name in ["public-read.json", "public-read-usage.json"] {
            let path = dir.join(name);
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
        dir
    }

    fn configure(dir: &Path, enabled: bool, bytes: u64) {
        crate::atomic::replace(
            &dir.join("public-read.json"),
            &serde_json::to_vec(&Config {
                enabled,
                daily_bytes: bytes,
            })
            .unwrap(),
            0o600,
        )
        .unwrap();
    }

    #[test]
    fn defaults_toggle_and_malformed_config_fail_closed() {
        let dir = directory("toggle");
        let budget = DailyBudget::open(&dir);
        assert_eq!(budget.config().unwrap(), Config::default());
        configure(&dir, false, 1000);
        assert!(budget.reserve(1).is_err());
        configure(&dir, true, 1000);
        assert_eq!(budget.reserve(10).unwrap().bytes, 10);
        std::fs::write(dir.join("public-read.json"), b"{").unwrap();
        assert!(budget.reserve(1).is_err());
        configure(&dir, true, 0);
        assert!(budget.reserve(1).is_err());
    }

    #[test]
    fn both_directions_and_errors_share_a_durable_daily_cap() {
        let dir = directory("restart");
        configure(&dir, true, 100);
        let budget = DailyBudget::open(&dir);
        let input = budget.reserve(40).unwrap();
        budget.refund(input, 10).unwrap();
        assert_eq!(budget.reserve(60).unwrap().bytes, 60);
        drop(budget);
        let restarted = DailyBudget::open(&dir);
        assert_eq!(restarted.used_bytes().unwrap(), 90);
        assert_eq!(restarted.reserve(20).unwrap().bytes, 10);
        assert!(restarted.reserve(1).is_err());
    }

    #[test]
    fn day_rollover_does_not_refund_old_reservations_or_clock_rollback() {
        let dir = directory("rollover");
        configure(&dir, true, 100);
        let budget = DailyBudget::open(&dir);
        let old = budget.reserve_on(100, 10).unwrap();
        assert_eq!(budget.reserve_on(50, 11).unwrap().period, 11);
        budget.refund(old, 100).unwrap();
        assert_eq!(budget.reserve_on(100, 10).unwrap().bytes, 50);
        assert!(budget.reserve_on(1, 10).is_err());
    }

    #[test]
    fn corrupt_usage_cannot_earn_a_fresh_allowance() {
        let dir = directory("corrupt");
        std::fs::write(dir.join("public-read-usage.json"), b"partial").unwrap();
        assert!(DailyBudget::open(&dir).reserve(1).is_err());
    }
}
