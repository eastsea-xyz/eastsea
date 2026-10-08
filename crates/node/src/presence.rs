//! Ephemeral, signed observations of this network. No consensus or chain writes.
//! A signature proves a node key, not a physical Mac or a claimed location.

use aether_net::{Endpoint, EndpointAddr, EndpointId, SecretKey, Signature};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const PING_INTERVAL: Duration = Duration::from_secs(60);
pub const TTL_SECONDS: u64 = 180;
pub const MAX_ENTRIES: usize = 4096;
pub const MAX_BATCH: usize = 64;
const MAX_PING_BYTES: usize = 512;
const CLOCK_SKEW_SECONDS: u64 = 10;
const MIN_UPDATE_SECONDS: u64 = 10;
const FANOUT: usize = 8;
const REGIONS: [&str; 7] = [
    "asia",
    "europe",
    "north_america",
    "south_america",
    "africa",
    "oceania",
    "unknown",
];
// Marketing release version: Cargo's workspace version predates the Mac releases.
// Release builders may override this through AETHER_VERSION at compile time.
pub const NODE_VERSION: &str = match option_env!("AETHER_VERSION") {
    Some(v) => v,
    None => "0.7.4",
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Validator,
    Candidate,
    Follower,
}

/// Signed wire record. The canonical signing tuple binds every public field,
/// the protocol version and network, preventing replays across chains.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ping {
    pub schema: u8,
    pub node_id: String,
    pub role: Role,
    pub version: String,
    pub timestamp: u64,
    pub region: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub country: Option<String>,
    pub network: String,
    pub signature: String,
}

impl Ping {
    #[allow(clippy::too_many_arguments)]
    pub fn signed(
        key: &SecretKey,
        role: Role,
        version: &str,
        timestamp: u64,
        region: &str,
        country: Option<String>,
        network: &str,
    ) -> Self {
        let mut p = Self {
            schema: 1,
            node_id: key.public().to_string(),
            role,
            version: version.into(),
            timestamp,
            region: region.into(),
            country,
            network: network.into(),
            signature: String::new(),
        };
        p.signature = hex::encode(key.sign(&p.message()).to_bytes());
        p
    }

    fn message(&self) -> Vec<u8> {
        serde_json::to_vec(&(
            "aether-presence-v1",
            self.schema,
            &self.node_id,
            self.role,
            &self.version,
            self.timestamp,
            &self.region,
            &self.country,
            &self.network,
        ))
        .expect("presence signing tuple")
    }

    fn verify(&self, network: &str, now: u64) -> Result<EndpointId, String> {
        if self.schema != 1 || self.network != network {
            return Err("presence protocol or network differs".into());
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_PING_BYTES
            || self.version.is_empty()
            || self.version.len() > 32
            || !self
                .version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
            || !REGIONS.contains(&self.region.as_str())
        {
            return Err("presence fields exceed their bounds".into());
        }
        validate_country(self.country.as_deref())?;
        if self.timestamp > now.saturating_add(CLOCK_SKEW_SECONDS)
            || now.saturating_sub(self.timestamp) >= TTL_SECONDS
        {
            return Err("presence timestamp is expired or ahead of this clock".into());
        }
        let id: EndpointId = self
            .node_id
            .parse()
            .map_err(|_| "invalid presence node id")?;
        // Canonical hex only: a key has one spelling, even on the wire.
        if self.node_id != id.to_string() || self.signature.len() != 128 {
            return Err("presence identity or signature is not canonical".into());
        }
        let bytes = hex::decode(&self.signature).map_err(|_| "invalid presence signature")?;
        let sig =
            Signature::try_from(bytes.as_slice()).map_err(|_| "invalid presence signature")?;
        id.verify(&self.message(), &sig)
            .map_err(|_| "presence signature rejected")?;
        Ok(id)
    }
}

// ISO 3166-1 alpha-2. No locale or geolocation lookup is performed here.
pub fn validate_country(country: Option<&str>) -> Result<(), String> {
    const ISO: &str = "AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW";
    if country.is_some_and(|c| c.len() != 2 || !ISO.split_ascii_whitespace().any(|iso| c == iso)) {
        return Err(
            "country must be an uppercase ISO 3166-1 alpha-2 code, or null to stop sharing".into(),
        );
    }
    Ok(())
}

struct Entry {
    ping: Ping,
    seen: u64,
    expires: Instant,
}

/// Time is explicit in this table so expiry and replay tests need no sleep.
pub struct Table {
    network: String,
    entries: BTreeMap<EndpointId, Entry>,
}

impl Table {
    pub fn new(network: String) -> Self {
        Self {
            network,
            entries: BTreeMap::new(),
        }
    }

    pub fn accept(&mut self, ping: Ping, wall: u64, mono: Instant) -> Result<bool, String> {
        let id = ping.verify(&self.network, wall)?;
        self.prune(mono, wall);
        if let Some(old) = self.entries.get(&id) {
            // Relaying the same signed ping never renews its TTL. One key gets
            // at most one update per ten seconds, even through different peers.
            if ping.timestamp <= old.ping.timestamp {
                return Ok(false);
            }
            if ping.timestamp - old.ping.timestamp < MIN_UPDATE_SECONDS {
                return Err("presence node update rate exceeded".into());
            }
        } else if self.entries.len() >= MAX_ENTRIES {
            return Err("presence table is full".into());
        }
        let lifetime = TTL_SECONDS - wall.saturating_sub(ping.timestamp);
        self.entries.insert(
            id,
            Entry {
                ping,
                seen: wall,
                expires: mono + Duration::from_secs(lifetime),
            },
        );
        Ok(true)
    }

    fn prune(&mut self, now: Instant, wall: u64) {
        // Exported records must still pass the signed clock window. A wall
        // clock jump or suspend must not keep an old record looking live;
        // the monotonic deadline also prevents a rollback renewing its TTL.
        self.entries.retain(|_, e| {
            e.expires > now
                && wall.saturating_sub(e.ping.timestamp) < TTL_SECONDS
                && e.ping.timestamp <= wall.saturating_add(CLOCK_SKEW_SECONDS)
        });
    }

    fn rows(&mut self, now: Instant, wall: u64) -> Vec<Value> {
        self.prune(now, wall);
        self.entries.values().map(|e| {
            let p = &e.ping;
            let mut row = json!({"node_id":p.node_id,"role":p.role,"version":p.version,"timestamp":p.timestamp,"last_seen":e.seen,"region":p.region});
            if let Some(country) = &p.country { row["country"] = json!(country); }
            row
        }).collect()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Packet {
    schema: u8,
    sender: String,
    binding_signature: String,
    pings: Vec<Ping>,
}

fn binding_message(transport: EndpointId, sender: EndpointId, timestamp: u64) -> Vec<u8> {
    serde_json::to_vec(&(
        "aether-presence-peer-v1",
        transport.to_string(),
        sender.to_string(),
        timestamp,
    ))
    .expect("presence binding tuple")
}

pub struct Presence {
    endpoint: Endpoint,
    identity: SecretKey,
    // A signed binding associates a logical node with its authenticated RPC
    // transport. Candidates retain the established, separate wallet endpoint
    // so background resharing remains the sole owner of their public node id.
    bindings: Mutex<BTreeMap<EndpointId, EndpointId>>,
    pub peers: aether_net::peers::PeerTracker,
    role: Role,
    country: Mutex<Option<String>>,
    table: Mutex<Table>,
    validators: BTreeSet<EndpointId>,
    packet_cursor: std::sync::atomic::AtomicUsize,
    local_settings: std::sync::atomic::AtomicBool,
}

impl Presence {
    pub fn new(
        endpoint: Endpoint,
        peers: aether_net::peers::PeerTracker,
        role: Role,
        validators: Vec<EndpointId>,
        network: String,
        country: Option<String>,
    ) -> Arc<Self> {
        let identity = endpoint.secret_key().clone();
        Self::with_identity(
            endpoint, peers, identity, role, validators, network, country,
        )
    }

    pub fn with_identity(
        endpoint: Endpoint,
        peers: aether_net::peers::PeerTracker,
        identity: SecretKey,
        role: Role,
        validators: Vec<EndpointId>,
        network: String,
        country: Option<String>,
    ) -> Arc<Self> {
        let p = Arc::new(Self {
            endpoint,
            identity,
            bindings: Mutex::new(BTreeMap::new()),
            peers,
            role,
            country: Mutex::new(country),
            table: Mutex::new(Table::new(network)),
            validators: validators.into_iter().collect(),
            packet_cursor: std::sync::atomic::AtomicUsize::new(0),
            local_settings: std::sync::atomic::AtomicBool::new(true),
        });
        p.renew();
        p
    }

    fn renew(&self) {
        let region = self
            .endpoint
            .addr()
            .relay_urls()
            .next()
            .map(|r| relay_region(&r.to_string()))
            .unwrap_or("unknown");
        let country = self.country.lock().expect("presence country").clone();
        let mut table = self.table.lock().expect("presence table");
        let ping = Ping::signed(
            &self.identity,
            self.role,
            NODE_VERSION,
            unix_now(),
            region,
            country,
            &table.network,
        );
        // Own heartbeat cannot be crowded out by other keys. Replacement is
        // local-only; received copies still go through signature/rate checks.
        let id = self.identity.public();
        table.prune(Instant::now(), unix_now());
        if table.entries.len() >= MAX_ENTRIES && !table.entries.contains_key(&id) {
            if let Some(oldest) = table
                .entries
                .iter()
                .min_by_key(|(_, e)| e.expires)
                .map(|(id, _)| *id)
            {
                table.entries.remove(&oldest);
            }
        }
        table.entries.insert(
            id,
            Entry {
                ping,
                seen: unix_now(),
                expires: Instant::now() + Duration::from_secs(TTL_SECONDS),
            },
        );
    }

    pub fn set_country(&self, country: Option<String>) -> Result<(), String> {
        if !self
            .local_settings
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err("country settings need a loopback-only HTTP server".into());
        }
        validate_country(country.as_deref())?;
        *self.country.lock().expect("presence country") = country;
        self.renew();
        Ok(())
    }

    pub fn disable_country_settings(&self) {
        self.local_settings
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Value {
        let nodes = self
            .table
            .lock()
            .expect("presence table")
            .rows(Instant::now(), unix_now());
        summarize(Some(self.identity.public()), &nodes)
    }

    pub fn peer_snapshot(&self) -> Value {
        let nodes = self
            .table
            .lock()
            .expect("presence table")
            .rows(Instant::now(), unix_now());
        let metadata: BTreeMap<_, _> = nodes
            .iter()
            .map(|n| (n["node_id"].as_str().unwrap_or_default(), n))
            .collect();
        let rows = self.peers.snapshot();
        let active: BTreeSet<EndpointId> = rows
            .iter()
            .filter_map(|r| r["node_id"].as_str()?.parse().ok())
            .collect();
        let mut bindings = self.bindings.lock().expect("presence bindings");
        bindings.retain(|id, _| active.contains(id));
        let peers: Vec<_> = rows
            .into_iter()
            .map(|mut row| {
                let transport: Option<EndpointId> =
                    row["node_id"].as_str().and_then(|id| id.parse().ok());
                let logical = transport.map(|id| bindings.get(&id).copied().unwrap_or(id));
                let id = logical.map(|id| id.to_string()).unwrap_or_default();
                if let Some(p) = metadata.get(id.as_str()) {
                    row["role"] = p["role"].clone();
                    row["version"] = p["version"].clone();
                }
                if row["role"].is_null() && logical.is_some_and(|id| self.validators.contains(&id))
                {
                    row["role"] = json!("validator");
                }
                row
            })
            .collect();
        json!(peers)
    }

    pub fn callback(self: &Arc<Self>) -> aether_net::PresenceCallback {
        let this = self.clone();
        Arc::new(move |remote, bytes| {
            this.ingest(remote, bytes);
            this.packet()
        })
    }

    fn ingest(&self, remote: EndpointId, bytes: &[u8]) {
        if bytes.len() > aether_net::MAX_PRESENCE_MESSAGE {
            return;
        }
        let Ok(packet) = serde_json::from_slice::<Packet>(bytes) else {
            return;
        };
        if packet.schema != 1 || packet.pings.len() > MAX_BATCH {
            return;
        }
        let mut table = self.table.lock().expect("presence table");
        let (wall, mono) = (unix_now(), Instant::now());
        for ping in packet.pings {
            if ping.node_id != self.identity.public().to_string() {
                let _ = table.accept(ping, wall, mono);
            }
        }
        // Only the logical identity's signature over this transport id earns
        // peer metadata. A forwarded ping alone proves no transport binding.
        if let Ok(sender) = packet.sender.parse::<EndpointId>() {
            if let Some(entry) = table.entries.get(&sender) {
                let valid = hex::decode(&packet.binding_signature)
                    .ok()
                    .and_then(|s| Signature::try_from(s.as_slice()).ok())
                    .is_some_and(|sig| {
                        sender
                            .verify(&binding_message(remote, sender, entry.ping.timestamp), &sig)
                            .is_ok()
                    });
                if valid {
                    let mut bindings = self.bindings.lock().expect("presence bindings");
                    if bindings.len() < MAX_ENTRIES || bindings.contains_key(&remote) {
                        bindings.insert(remote, sender);
                    }
                }
            }
        }
    }

    fn packet(&self) -> Vec<u8> {
        use std::sync::atomic::Ordering;
        let mut table = self.table.lock().expect("presence table");
        table.prune(Instant::now(), unix_now());
        let mine = table
            .entries
            .get(&self.identity.public())
            .map(|e| e.ping.clone());
        let all: Vec<_> = table
            .entries
            .iter()
            .filter(|(id, _)| **id != self.identity.public())
            .map(|(_, e)| e.ping.clone())
            .collect();
        let mut pings: Vec<_> = mine.into_iter().collect();
        let start = self
            .packet_cursor
            .fetch_add(MAX_BATCH - 1, Ordering::Relaxed);
        for i in 0..all.len().min(MAX_BATCH - pings.len()) {
            pings.push(all[(start.wrapping_add(i)) % all.len()].clone());
        }
        let timestamp = pings.first().map(|p| p.timestamp).unwrap_or_default();
        let sender = self.identity.public();
        let binding_signature = hex::encode(
            self.identity
                .sign(&binding_message(self.endpoint.id(), sender, timestamp))
                .to_bytes(),
        );
        serde_json::to_vec(&Packet {
            schema: 1,
            sender: sender.to_string(),
            binding_signature,
            pings,
        })
        .expect("presence packet")
    }

    /// Send one bounded batch each minute to a rotating sample of bootstrap
    /// and connected neighbors. Replies gossip their signed observations back.
    pub fn start(self: &Arc<Self>, seeds: Vec<EndpointAddr>) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut round = 0usize;
            let mut startup_retries = 2;
            let mut ticks = tokio::time::interval(PING_INTERVAL);
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! { _ = this.endpoint.closed() => break, _ = ticks.tick() => {} }
                this.renew();
                let mut destinations: BTreeMap<_, _> =
                    seeds.iter().map(|a| (a.id, a.clone())).collect();
                for id in this.peers.connected_ids() {
                    destinations
                        .entry(id)
                        .or_insert_with(|| EndpointAddr::from(id));
                }
                destinations.remove(&this.endpoint.id());
                destinations.remove(&this.identity.public());
                let destinations: Vec<_> = destinations.into_values().collect();
                let bytes = this.packet();
                let mut exchanges = tokio::task::JoinSet::new();
                for i in 0..destinations.len().min(FANOUT) {
                    let addr = destinations[(round.wrapping_add(i)) % destinations.len()].clone();
                    let (ep, bytes) = (this.endpoint.clone(), bytes.clone());
                    exchanges.spawn(async move {
                        let remote = addr.id;
                        aether_net::presence_exchange(&ep, addr, &bytes)
                            .await
                            .map(|bytes| (remote, bytes))
                    });
                }
                let mut failed = false;
                while let Some(reply) = exchanges.join_next().await {
                    match reply {
                        Ok(Ok((remote, bytes))) => this.ingest(remote, &bytes),
                        _ => failed = true,
                    }
                }
                // A peer may still be starting when our first ping goes out.
                // Bound startup retries; unsupported old nodes never create
                // a permanent retry loop or change the steady 60-second rate.
                if failed && startup_retries > 0 {
                    startup_retries -= 1;
                    ticks.reset_at(
                        tokio::time::Instant::now() + Duration::from_secs(MIN_UPDATE_SECONDS),
                    );
                }
                round = round.wrapping_add(FANOUT);
            }
        });
    }
}

pub fn unavailable() -> Value {
    summarize(None, &[])
}

fn summarize(observer: Option<EndpointId>, nodes: &[Value]) -> Value {
    let mut roles: BTreeMap<String, usize> = ["validator", "candidate", "follower"]
        .map(|s| (s.into(), 0))
        .into();
    let mut regions: BTreeMap<String, usize> = REGIONS.map(|s| (s.into(), 0)).into();
    let (mut versions, mut countries) = (
        BTreeMap::<String, usize>::new(),
        BTreeMap::<String, usize>::new(),
    );
    for n in nodes {
        for (field, counts) in [
            ("role", &mut roles),
            ("region", &mut regions),
            ("version", &mut versions),
            ("country", &mut countries),
        ] {
            if let Some(s) = n[field].as_str() {
                *counts.entry(s.into()).or_default() += 1;
            }
        }
    }
    json!({"schema":1,"available":observer.is_some(),"observer":observer.map(|id| id.to_string()),"scope":"what this node can see",
        "observed_at":unix_now(),"ttl_seconds":TTL_SECONDS,"total":nodes.len(),"by_role":roles,"by_version":versions,"by_region":regions,"by_country":countries,"nodes":nodes})
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Map the *home relay*, never the Mac's address, to a continent. Custom or
/// unrecognized relay names stay unknown. No URL leaves this function.
pub fn relay_region(url: &str) -> &'static str {
    let Some(host) = url
        .strip_prefix("https://")
        .and_then(|s| s.split('/').next())
    else {
        return "unknown";
    };
    let host = host.trim_end_matches('.');
    if !host.ends_with(".relay.n0.iroh.link") && !host.ends_with(".relay.iroh.network") {
        return "unknown";
    }
    let region = host
        .split('.')
        .next()
        .unwrap_or_default()
        .split('-')
        .next()
        .unwrap_or_default();
    match region {
        "aps1" | "apne1" | "apne2" | "apne3" | "apso1" | "apso2" | "apso3" | "ape1" => "asia",
        "euc1" | "euc2" | "euw1" | "euw2" | "euw3" | "eun1" | "eus1" | "eus2" | "eu" => "europe",
        "use1" | "use2" | "usw1" | "usw2" | "cac1" | "caw1" | "na" => "north_america",
        "sae1" => "south_america",
        "afs1" => "africa",
        "aps2" | "aps4" => "oceania",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ping(i: u64, at: u64) -> Ping {
        Ping::signed(
            &aether_net::devnet_node_secret(i),
            Role::Follower,
            "0.7.4",
            at,
            "asia",
            None,
            "7777:0",
        )
    }

    #[test]
    fn expiry_and_replays_never_renew_the_ttl() {
        let now = Instant::now();
        let mut t = Table::new("7777:0".into());
        assert!(t.accept(ping(1, 1000), 1000, now).unwrap());
        assert!(!t
            .accept(ping(1, 1000), 1100, now + Duration::from_secs(100))
            .unwrap());
        assert_eq!(t.rows(now + Duration::from_secs(179), 1179).len(), 1);
        assert!(t.rows(now + Duration::from_secs(180), 1180).is_empty());
        assert!(t
            .accept(ping(1, 1000), 1180, now + Duration::from_secs(180))
            .is_err());
        assert!(t
            .accept(ping(1, 1180), 1180, now + Duration::from_secs(180))
            .unwrap());
    }

    #[test]
    fn cached_records_expire_on_either_clock_without_ttl_renewal() {
        let mono = Instant::now();
        let mut t = Table::new("7777:0".into());
        t.accept(ping(1, 1000), 1000, mono).unwrap();
        // Wall time advanced while this runtime's monotonic clock did not.
        assert!(t.rows(mono, 1180).is_empty());
        let mut t = Table::new("7777:0".into());
        t.accept(ping(1, 1000), 1000, mono).unwrap();
        // A wall clock rollback cannot extend an already elapsed deadline.
        assert!(t.rows(mono + Duration::from_secs(180), 999).is_empty());
    }

    #[test]
    fn every_field_is_signed_and_networks_are_isolated() {
        let p = ping(1, 1000);
        for field in [
            "role",
            "version",
            "region",
            "country",
            "timestamp",
            "node_id",
            "signature",
            "network",
            "schema",
        ] {
            let mut v = serde_json::to_value(&p).unwrap();
            v[field] = match field {
                "role" => json!("validator"),
                "country" => json!("KR"),
                "timestamp" => json!(1001),
                "schema" => json!(2),
                "node_id" => json!(aether_net::devnet_node_id(2).to_string()),
                "signature" => json!("00".repeat(64)),
                _ => json!("europe"),
            };
            let p: Ping = serde_json::from_value(v).unwrap();
            assert!(p.verify("7777:0", 1000).is_err(), "{field}");
        }
        assert!(p.verify("7780:0", 1000).is_err());
    }

    #[test]
    fn one_key_counts_once_and_stale_metadata_cannot_replace_it() {
        let now = Instant::now();
        let mut t = Table::new("7777:0".into());
        t.accept(ping(1, 1000), 1000, now).unwrap();
        let mut newer = ping(1, 1060);
        newer = Ping::signed(
            &aether_net::devnet_node_secret(1),
            Role::Candidate,
            "0.7.4",
            newer.timestamp,
            "europe",
            Some("KR".into()),
            "7777:0",
        );
        t.accept(newer, 1060, now + Duration::from_secs(60))
            .unwrap();
        assert!(!t
            .accept(ping(1, 1000), 1060, now + Duration::from_secs(60))
            .unwrap());
        t.accept(ping(2, 1060), 1060, now + Duration::from_secs(60))
            .unwrap();
        let rows = t.rows(now + Duration::from_secs(60), 1060);
        let v = summarize(Some(aether_net::devnet_node_id(1)), &rows);
        assert_eq!(v["total"], 2);
        assert_eq!(v["by_role"]["candidate"], 1);
        assert_eq!(v["by_region"]["europe"], 1);
        assert_eq!(v["by_country"]["KR"], 1);
        assert!(
            !rows[1].as_object().unwrap().contains_key("country")
                || !rows[0].as_object().unwrap().contains_key("country")
        );
    }

    #[test]
    fn clock_size_country_and_node_rate_are_bounded() {
        assert!(ping(1, 1011).verify("7777:0", 1000).is_err());
        let mut p = ping(1, 1000);
        p.version = "x".repeat(513);
        assert!(p.verify("7777:0", 1000).is_err());
        for c in ["kr", "ZZ", "Korea", "127.0.0.1"] {
            assert!(validate_country(Some(c)).is_err());
        }
        assert!(validate_country(None).is_ok());
        assert!(validate_country(Some("KR")).is_ok());
        let now = Instant::now();
        let mut t = Table::new("7777:0".into());
        t.accept(ping(1, 1000), 1000, now).unwrap();
        assert!(t
            .accept(ping(1, 1001), 1001, now + Duration::from_secs(1))
            .is_err());
    }

    #[test]
    fn relay_regions_are_coarse_and_unknown_is_honest() {
        assert_eq!(relay_region("https://aps1-1.relay.n0.iroh.link./"), "asia");
        assert_eq!(relay_region("https://euw1-1.relay.iroh.network/"), "europe");
        assert_eq!(
            relay_region("https://use1-1.relay.n0.iroh.link./"),
            "north_america"
        );
        assert_eq!(
            relay_region("https://aps2-1.relay.iroh.network/"),
            "oceania"
        );
        assert_eq!(relay_region("https://my-relay.example/"), "unknown");
        assert_eq!(relay_region("https://127.0.0.1/"), "unknown");
        assert_eq!(relay_region("https://aps1-1.custom.example/"), "unknown");
        assert_eq!(
            relay_region("https://aps1-1.relay.n0.iroh.link.attacker.example/"),
            "unknown"
        );
    }

    #[tokio::test]
    async fn stable_identity_is_separate_from_transport_and_metadata_needs_its_binding() {
        let logical = aether_net::devnet_node_secret(1);
        let transport = aether_net::devnet_node_secret(41);
        let peers = aether_net::peers::PeerTracker::new();
        let server =
            aether_net::bind_local(transport, "127.0.0.1:0".parse().unwrap(), peers.clone())
                .await
                .unwrap();
        let candidate = Presence::with_identity(
            server.clone(),
            peers,
            logical.clone(),
            Role::Candidate,
            vec![],
            "7777:0".into(),
            None,
        );
        let router = aether_net::serve_with_presence(
            server.clone(),
            |_| async { json!({}) },
            None,
            None,
            Some(candidate.callback()),
        );
        let peer_tracker = aether_net::peers::PeerTracker::new();
        let client = aether_net::bind_local(
            aether_net::devnet_node_secret(2),
            "127.0.0.1:0".parse().unwrap(),
            peer_tracker.clone(),
        )
        .await
        .unwrap();
        let observer = Presence::new(
            client.clone(),
            peer_tracker,
            Role::Follower,
            vec![],
            "7777:0".into(),
            None,
        );
        let connection = client
            .connect(server.addr(), aether_net::ALPN_PRESENCE)
            .await
            .unwrap();
        let packet = candidate.packet();
        // A candidate's ping is valid when forwarded, but cannot attribute
        // role/version to an unrelated authenticated transport.
        observer.ingest(aether_net::devnet_node_id(42), &packet);
        let rows = observer.peer_snapshot();
        assert!(rows[0]["role"].is_null());
        assert!(rows[0]["version"].is_null());
        observer.ingest(server.id(), &packet);
        let rows = observer.peer_snapshot();
        assert_eq!(rows[0]["node_id"], server.id().to_string());
        assert_eq!(rows[0]["role"], "candidate");
        assert_eq!(rows[0]["version"], NODE_VERSION);
        assert_eq!(observer.snapshot()["total"], 2);
        assert_eq!(
            candidate.snapshot()["observer"],
            logical.public().to_string()
        );
        assert_ne!(server.id(), logical.public());

        // A paused follower advertising the same logical key is still one
        // node, even though its active transport has a different key.
        let at = unix_now() + MIN_UPDATE_SECONDS;
        let paused = Ping::signed(
            &logical,
            Role::Follower,
            NODE_VERSION,
            at,
            "unknown",
            None,
            "7777:0",
        );
        let mut table = observer.table.lock().unwrap();
        table.accept(paused, at, Instant::now()).unwrap();
        assert_eq!(table.rows(Instant::now(), unix_now()).len(), 2);
        drop(table);
        assert_eq!(observer.snapshot()["by_role"]["candidate"], 0);
        assert_eq!(observer.snapshot()["by_role"]["follower"], 2);
        drop(connection);
        client.close().await;
        router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn country_opt_in_and_opt_out_renew_every_signed_field() {
        let peers = aether_net::peers::PeerTracker::new();
        let endpoint = aether_net::bind_local(
            aether_net::devnet_node_secret(1),
            "127.0.0.1:0".parse().unwrap(),
            peers.clone(),
        )
        .await
        .unwrap();
        let p = Presence::new(
            endpoint.clone(),
            peers,
            Role::Follower,
            vec![],
            "7777:0".into(),
            None,
        );
        assert_eq!(p.snapshot()["by_country"], json!({}));
        assert!(!p.snapshot()["nodes"][0]
            .as_object()
            .unwrap()
            .contains_key("country"));
        p.set_country(Some("KR".into())).unwrap();
        assert_eq!(p.snapshot()["by_country"]["KR"], 1);
        let packet: Packet = serde_json::from_slice(&p.packet()).unwrap();
        assert_eq!(packet.pings[0].country.as_deref(), Some("KR"));
        assert!(packet.pings[0].verify("7777:0", unix_now()).is_ok());
        assert!(p.set_country(Some("ZZ".into())).is_err());
        assert_eq!(p.snapshot()["by_country"]["KR"], 1);
        p.set_country(None).unwrap();
        let packet: Packet = serde_json::from_slice(&p.packet()).unwrap();
        assert!(packet.pings[0].country.is_none());
        assert!(packet.pings[0].verify("7777:0", unix_now()).is_ok());
        p.disable_country_settings();
        assert!(p.set_country(Some("KR".into())).is_err());
        assert_eq!(p.snapshot()["by_country"], json!({}));
        endpoint.close().await;
    }

    #[tokio::test]
    async fn oversized_batches_and_invalid_pings_do_not_enter_the_table() {
        let peers = aether_net::peers::PeerTracker::new();
        let endpoint = aether_net::bind_local(
            aether_net::devnet_node_secret(1),
            "127.0.0.1:0".parse().unwrap(),
            peers.clone(),
        )
        .await
        .unwrap();
        let p = Presence::new(
            endpoint.clone(),
            peers,
            Role::Follower,
            vec![],
            "7777:0".into(),
            None,
        );
        let foreign = Ping::signed(
            &aether_net::devnet_node_secret(2),
            Role::Follower,
            NODE_VERSION,
            unix_now(),
            "unknown",
            None,
            "7777:0",
        );
        let packet = Packet {
            schema: 1,
            sender: foreign.node_id.clone(),
            binding_signature: "00".repeat(64),
            pings: vec![foreign.clone(); MAX_BATCH + 1],
        };
        p.ingest(
            aether_net::devnet_node_id(2),
            &serde_json::to_vec(&packet).unwrap(),
        );
        assert_eq!(p.snapshot()["total"], 1);
        let mut invalid = foreign;
        invalid.country = Some("KR".into()); // Signature did not authorize it.
        let packet = Packet {
            schema: 1,
            sender: invalid.node_id.clone(),
            binding_signature: "00".repeat(64),
            pings: vec![invalid],
        };
        p.ingest(
            aether_net::devnet_node_id(2),
            &serde_json::to_vec(&packet).unwrap(),
        );
        assert_eq!(p.snapshot()["total"], 1);
        p.ingest(
            aether_net::devnet_node_id(2),
            &vec![0; aether_net::MAX_PRESENCE_MESSAGE + 1],
        );
        assert_eq!(p.snapshot()["total"], 1);
        assert!(p.packet().len() <= aether_net::MAX_PRESENCE_MESSAGE);
        endpoint.close().await;
    }
}
