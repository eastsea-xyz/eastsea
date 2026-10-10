//! Ephemeral cohort observations. Public RPC and gossip share the same privacy
//! boundary: no individual records, observer identifiers or signed metadata.
//! Transport peers are authenticated, but their aggregate claims are unverified.

use aether_net::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const PING_INTERVAL: Duration = Duration::from_secs(60);
/// Local transport observations expire sooner than the public release window.
pub const TTL_SECONDS: u64 = 180;
pub const TIME_BUCKET_SECONDS: u64 = 600;
pub const MIN_BUCKET_SIZE: usize = 3;
pub const MAX_ENTRIES: usize = 4096;
const MIN_UPDATE_SECONDS: u64 = 10;
const FANOUT: usize = 8;
const SCOPE: &str = "unverified cohort observation";
const SUBREGIONS: [&str; 17] = ["015", "021", "030", "034", "035", "039", "053", "054", "057", "061", "143", "145", "151", "154", "155", "202", "419"];
const REGIONS: [&str; 25] = [
    "asia",
    "europe",
    "north_america",
    "south_america",
    "africa",
    "oceania",
    "unknown",
    "world",
    "015",
    "021",
    "030",
    "034",
    "035",
    "039",
    "053",
    "054",
    "057",
    "061",
    "143",
    "145",
    "151",
    "154",
    "155",
    "202",
    "419",
];

// Marketing release version, for local diagnostics only. It is not gossiped.
pub const NODE_VERSION: &str = match option_env!("AETHER_VERSION") {
    Some(v) => v,
    None => "0.7.3",
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Validator,
    Candidate,
    Follower,
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Self::Validator => "validator",
            Self::Candidate => "candidate",
            Self::Follower => "follower",
        }
    }
}

/// The only public presence record. Every breakdown is a disjoint partition
/// of `total`; a suppressed small total has no breakdowns at all.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema: u8,
    available: bool,
    scope: String,
    observed_at: u64,
    ttl_seconds: u64,
    minimum_bucket_size: usize,
    total: Option<usize>,
    by_role: BTreeMap<String, usize>,
    by_region: BTreeMap<String, usize>,
    by_version: BTreeMap<String, usize>,
}

impl Snapshot {
    fn empty(available: bool, wall: u64) -> Self {
        Self {
            schema: 2,
            available,
            scope: SCOPE.into(),
            observed_at: time_bucket(wall),
            ttl_seconds: TIME_BUCKET_SECONDS,
            minimum_bucket_size: MIN_BUCKET_SIZE,
            total: None,
            by_role: BTreeMap::new(),
            by_region: BTreeMap::new(),
            by_version: BTreeMap::new(),
        }
    }

    fn from_counts(
        wall: u64,
        total: usize,
        roles: BTreeMap<String, usize>,
        regions: BTreeMap<String, usize>,
    ) -> Self {
        let mut value = Self::empty(true, wall);
        if total >= MIN_BUCKET_SIZE {
            value.total = Some(total);
            value.by_role = fold_small(roles, "other");
            value.by_region = fold_small(regions, "world");
            // Individual peer versions/quality are no longer collected. Keep
            // the versioned consumer field as one coarse, safe unknown group.
            value.by_version.insert("unknown".into(), total);
        }
        value
    }

    fn valid(&self, wall: u64) -> bool {
        if self.schema != 2
            || !self.available
            || self.scope != SCOPE
            || self.observed_at != time_bucket(wall)
            || self.ttl_seconds != TIME_BUCKET_SECONDS
            || self.minimum_bucket_size != MIN_BUCKET_SIZE
        {
            return false;
        }
        let valid_partition = |counts: &BTreeMap<String, usize>, keys: &[&str]| {
            counts.len() <= keys.len()
                && counts.iter().all(|(key, count)| {
                    keys.contains(&key.as_str())
                        && *count >= MIN_BUCKET_SIZE
                        && *count <= MAX_ENTRIES
                })
                && counts.values().copied().sum::<usize>() == self.total.unwrap_or_default()
        };
        match self.total {
            None => {
                self.by_role.is_empty() && self.by_region.is_empty() && self.by_version.is_empty()
            }
            Some(total) => {
                total >= MIN_BUCKET_SIZE
                    && total <= MAX_ENTRIES
                    && valid_partition(
                        &self.by_role,
                        &["validator", "candidate", "follower", "unknown", "other"],
                    )
                    && valid_partition(&self.by_region, &REGIONS)
                    && valid_partition(&self.by_version, &["unknown"])
            }
        }
    }
}

/// Merge rare categories upward. When their combined remainder is still below
/// k, merge a whole safe sibling too: exposing that sibling and an exact total
/// would otherwise disclose the suppressed remainder by subtraction.
fn fold_small(mut counts: BTreeMap<String, usize>, parent: &str) -> BTreeMap<String, usize> {
    let mut folded = counts.remove(parent).unwrap_or_default();
    counts.retain(|_, count| {
        if *count < MIN_BUCKET_SIZE {
            folded += *count;
            false
        } else {
            true
        }
    });
    if folded > 0 && folded < MIN_BUCKET_SIZE {
        if let Some(key) = counts
            .iter()
            .min_by_key(|(_, count)| **count)
            .map(|(key, _)| key.clone())
        {
            folded += counts.remove(&key).expect("folded sibling");
        }
    }
    if folded >= MIN_BUCKET_SIZE {
        counts.insert(parent.into(), folded);
    }
    counts
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Packet {
    schema: u8,
    network: String,
    aggregate: Snapshot,
}

struct Observation {
    snapshot: Snapshot,
    wall: u64,
    expires: Instant,
}

struct Table {
    network: String,
    observations: BTreeMap<EndpointId, Observation>,
    released: Option<(Snapshot, Instant)>,
}

impl Table {
    fn new(network: String) -> Self {
        Self {
            network,
            observations: BTreeMap::new(),
            released: None,
        }
    }

    fn prune(&mut self, mono: Instant, wall: u64) {
        self.observations.retain(|_, observation| {
            observation.expires > mono
                && wall.saturating_sub(observation.wall) < TTL_SECONDS
                && observation.snapshot.observed_at == time_bucket(wall)
        });
    }

    fn accept(&mut self, remote: EndpointId, packet: Packet, wall: u64, mono: Instant) -> bool {
        if packet.schema != 2 || packet.network != self.network || !packet.aggregate.valid(wall) {
            return false;
        }
        self.prune(mono, wall);
        // One authenticated immediate peer gets one observation. The peer's
        // transport id stays local; it is never put in the exported packet.
        if self.observations.len() >= MAX_ENTRIES && !self.observations.contains_key(&remote) {
            return false;
        }
        self.observations.insert(
            remote,
            Observation {
                snapshot: packet.aggregate,
                wall,
                expires: mono + Duration::from_secs(TTL_SECONDS),
            },
        );
        true
    }

    fn release(&mut self, local: Snapshot, wall: u64, mono: Instant) -> Snapshot {
        // A floor timestamp alone is insufficient: changing counts inside a
        // window would still reveal the exact moment someone joined or left.
        if let Some((snapshot, until)) = &self.released {
            if snapshot.observed_at == time_bucket(wall) && *until > mono {
                return snapshot.clone();
            }
        }
        self.prune(mono, wall);
        let mut best = local;
        for observation in self.observations.values() {
            if observation.snapshot.total > best.total {
                best = observation.snapshot.clone();
            }
        }
        // Never add anonymous populations: their overlap cannot be established
        // without reintroducing individual identifiers. This is one complete
        // unverified cohort claim, not a global distinct-node census.
        self.released = Some((
            best.clone(),
            mono + Duration::from_secs(TIME_BUCKET_SECONDS),
        ));
        best
    }
}

pub struct Presence {
    endpoint: Endpoint,
    identity: EndpointId,
    pub peers: aether_net::peers::PeerTracker,
    role: Role,
    country: Mutex<Option<String>>,
    region: Mutex<Option<String>>,
    table: Mutex<Table>,
    validators: BTreeSet<EndpointId>,
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
        Arc::new(Self {
            endpoint,
            identity: identity.public(),
            peers,
            role,
            country: Mutex::new(country.filter(|c| validate_country(Some(c)).is_ok())),
            region: Mutex::new(None),
            table: Mutex::new(Table::new(network)),
            validators: validators.into_iter().collect(),
            local_settings: std::sync::atomic::AtomicBool::new(true),
        })
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
        // Do not invalidate the public cache: choices cannot reveal an exact
        // change time or a rare country by changing the aggregate mid-window.
        Ok(())
    }

    /// The default Mac sub-region is independent of optional country consent.
    pub fn set_region(&self, region: Option<String>) -> Result<(), String> {
        if !self.local_settings.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("region settings need a loopback-only HTTP server".into());
        }
        validate_region(region.as_deref())?;
        *self.region.lock().expect("presence region") = region;
        // Preserve the frozen release window when a local preference changes.
        Ok(())
    }

    pub fn disable_country_settings(&self) {
        self.local_settings
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    fn own_region(&self) -> &'static str {
        if let Some(country) = self.country.lock().expect("presence country").as_deref() {
            return country_region(country);
        }
        if let Some(region) = self.region.lock().expect("presence region").as_deref() {
            return SUBREGIONS.iter().copied().find(|code| *code == region).unwrap_or("unknown");
        }
        self.endpoint
            .addr()
            .relay_urls()
            .next()
            .map(|relay| relay_region(&relay.to_string()))
            .unwrap_or("unknown")
    }

    fn aggregate(&self) -> Snapshot {
        let (wall, mono) = (unix_now(), Instant::now());
        let mut table = self.table.lock().expect("presence table");
        table.prune(mono, wall);
        let mut observed: BTreeSet<_> = self.peers.connected_ids().into_iter().collect();
        observed.extend(table.observations.keys().copied());
        observed.remove(&self.endpoint.id());
        observed.remove(&self.identity);
        let mut roles = BTreeMap::from([(self.role.name().into(), 1)]);
        let mut total = 1;
        for id in observed.into_iter().take(MAX_ENTRIES - 1) {
            let role = if self.validators.contains(&id) {
                "validator"
            } else {
                "unknown"
            };
            *roles.entry(role.into()).or_default() += 1;
            total += 1;
        }
        // Remote individuals never advertise their region or build quality.
        // Only this node's local choice/home relay contributes a known region.
        let mut regions = BTreeMap::from([(self.own_region().into(), 1)]);
        *regions.entry("unknown".into()).or_default() += total - 1;
        let local = Snapshot::from_counts(wall, total, roles, regions);
        table.release(local, wall, mono)
    }

    pub fn snapshot(&self) -> Value {
        serde_json::to_value(self.aggregate()).expect("presence snapshot")
    }

    /// Identifiable connection diagnostics are exposed only by the native
    /// loopback aether_peers RPC, never by the public presence snapshot/gossip.
    pub fn peer_snapshot(&self) -> Value {
        if !self
            .local_settings
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return json!([]);
        }
        let peers: Vec<_> = self
            .peers
            .snapshot()
            .into_iter()
            .map(|mut row| {
                let id: Option<EndpointId> = row["node_id"].as_str().and_then(|id| id.parse().ok());
                if id.is_some_and(|id| self.validators.contains(&id)) {
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
        if bytes.len() > aether_net::MAX_PRESENCE_MESSAGE
            || remote == self.endpoint.id()
            || remote == self.identity
        {
            return;
        }
        let Ok(packet) = serde_json::from_slice::<Packet>(bytes) else {
            return;
        };
        self.table.lock().expect("presence table").accept(
            remote,
            packet,
            unix_now(),
            Instant::now(),
        );
    }

    fn packet(&self) -> Vec<u8> {
        let aggregate = self.aggregate();
        let network = self.table.lock().expect("presence table").network.clone();
        serde_json::to_vec(&Packet {
            schema: 2,
            network,
            aggregate,
        })
        .expect("presence packet")
    }

    /// Exchange the same frozen privacy-filtered cohort each minute with at
    /// most eight rotating neighbors. Raw identity/metadata gossip is gone.
    pub fn start(self: &Arc<Self>, seeds: Vec<EndpointAddr>) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut round = 0usize;
            let mut startup_retries = 2;
            let mut ticks = tokio::time::interval(PING_INTERVAL);
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! { _ = this.endpoint.closed() => break, _ = ticks.tick() => {} }
                let mut destinations: BTreeMap<_, _> =
                    seeds.iter().map(|a| (a.id, a.clone())).collect();
                for id in this.peers.connected_ids() {
                    destinations
                        .entry(id)
                        .or_insert_with(|| EndpointAddr::from(id));
                }
                destinations.remove(&this.endpoint.id());
                destinations.remove(&this.identity);
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
    serde_json::to_value(Snapshot::empty(false, unix_now())).expect("unavailable presence")
}

fn time_bucket(wall: u64) -> u64 {
    wall / TIME_BUCKET_SECONDS * TIME_BUCKET_SECONDS
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ISO 3166-1 alpha-2 input is local only; no locale or GeoIP lookup here.
pub fn validate_country(country: Option<&str>) -> Result<(), String> {
    const ISO: &str = "AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW";
    if country.is_some_and(|c| c.len() != 2 || !ISO.split_ascii_whitespace().any(|iso| c == iso)) {
        return Err(
            "country must be an uppercase ISO 3166-1 alpha-2 code, or null to stop sharing".into(),
        );
    }
    Ok(())
}

/// Only canonical UN M49 sub-region codes are accepted; broader relay labels
/// remain read-only compatibility data. No country or location lookup is made.
pub fn validate_region(region: Option<&str>) -> Result<(), String> {
    if region.is_some_and(|code| !SUBREGIONS.contains(&code)) {
        return Err("region must be a canonical UN M49 sub-region code, or null".into());
    }
    Ok(())
}

/// Local country choice maps to its statistical sub-region, never a public
/// country field. Source: https://unstats.un.org/unsd/methodology/m49/overview/
fn country_region(country: &str) -> &'static str {
    for (region, countries) in [
        ("015", "DZ EG EH LY MA SD TN"),
        ("021", "BM CA GL PM US"),
        ("030", "CN HK JP KP KR MN MO"),
        ("034", "AF BD BT IN IR LK MV NP PK"),
        ("035", "BN ID KH LA MM MY PH SG TH TL VN"),
        ("039", "AD AL BA ES GI GR HR IT ME MK MT PT RS SI SM VA"),
        ("053", "AU CC CX HM NF NZ"),
        ("054", "FJ NC PG SB VU"),
        ("057", "FM GU KI MH MP NR PW UM"),
        ("061", "AS CK NU PF PN TK TO TV WF WS"),
        ("143", "KG KZ TJ TM UZ"),
        ("145", "AE AM AZ BH CY GE IL IQ JO KW LB OM PS QA SA SY TR YE"),
        ("151", "BG BY CZ HU MD PL RO RU SK UA"),
        ("154", "AX DK EE FI FO GB GG IE IM IS JE LT LV NO SE SJ"),
        ("155", "AT BE CH DE FR LI LU MC NL"),
        ("202", "AO BF BI BJ BW CD CF CG CI CM CV DJ ER ET GA GH GM GN GQ GW IO KE KM LR LS MG ML MR MU MW MZ NA NE NG RE RW SC SH SL SN SO SS ST SZ TD TF TG TZ UG YT ZA ZM ZW"),
        ("419", "AG AI AR AW BB BL BO BQ BR BS BV BZ CL CO CR CU CW DM DO EC FK GD GF GP GS GT GY HN HT JM KN KY LC MF MQ MS MX NI PA PE PR PY SR SV SX TC TT UY VC VE VG VI"),
    ] {
        if countries.split_ascii_whitespace().any(|code| code == country) {
            return region;
        }
    }
    "unknown"
}

/// Home relay continent only; custom or unrecognized domains stay unknown.
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

    fn counts(pairs: &[(&str, usize)]) -> BTreeMap<String, usize> {
        pairs
            .iter()
            .map(|(key, count)| ((*key).into(), *count))
            .collect()
    }

    fn assert_safe(snapshot: &Snapshot) {
        assert_eq!(snapshot.observed_at % TIME_BUCKET_SECONDS, 0);
        for partition in [&snapshot.by_role, &snapshot.by_region, &snapshot.by_version] {
            assert!(partition.values().all(|count| *count >= MIN_BUCKET_SIZE));
            assert_eq!(
                partition.values().sum::<usize>(),
                snapshot.total.unwrap_or_default()
            );
        }
        let json = serde_json::to_value(snapshot).unwrap();
        for field in [
            "nodes",
            "node_id",
            "observer",
            "country",
            "by_country",
            "timestamp",
            "last_seen",
            "sender",
            "signature",
            "binding_signature",
            "pings",
        ] {
            assert!(
                !json.as_object().unwrap().contains_key(field),
                "{field} never exported"
            );
        }
    }

    #[test]
    fn small_cohorts_and_residuals_are_suppressed_at_the_producer() {
        for total in 1..MIN_BUCKET_SIZE {
            let value = Snapshot::from_counts(
                1791440017,
                total,
                counts(&[("validator", total)]),
                counts(&[("asia", total)]),
            );
            assert_eq!(value.total, None);
            assert_safe(&value);
        }
        let cases = [
            (
                4,
                counts(&[("validator", 3), ("follower", 1)]),
                counts(&[("asia", 3), ("europe", 1)]),
            ),
            (
                8,
                counts(&[("validator", 6), ("follower", 2)]),
                counts(&[("asia", 6), ("europe", 2)]),
            ),
            (
                6,
                counts(&[("validator", 3), ("candidate", 1), ("follower", 2)]),
                counts(&[("asia", 3), ("europe", 1), ("africa", 2)]),
            ),
            (
                9,
                counts(&[("validator", 6), ("follower", 3)]),
                counts(&[("asia", 6), ("europe", 3)]),
            ),
        ];
        for (total, roles, regions) in cases {
            let value = Snapshot::from_counts(1791440017, total, roles, regions);
            assert!(value.valid(1791440017));
            assert_safe(&value);
        }
        assert_eq!(
            fold_small(counts(&[("validator", 3), ("follower", 1)]), "other"),
            counts(&[("other", 4)])
        );
        assert_eq!(
            fold_small(counts(&[("asia", 6), ("europe", 2)]), "world"),
            counts(&[("world", 8)])
        );
        // Rare quality data uses the same complementary suppression rule.
        assert_eq!(
            fold_small(counts(&[("common", 6), ("rare", 2)]), "other"),
            counts(&[("other", 8)])
        );
    }

    #[test]
    fn releases_are_fixed_within_a_ten_minute_window_and_gossip_is_never_summed() {
        let mono = Instant::now();
        let mut table = Table::new("7777:0".into());
        let local = Snapshot::from_counts(
            1201,
            4,
            counts(&[("validator", 4)]),
            counts(&[("unknown", 4)]),
        );
        let release = table.release(local.clone(), 1201, mono);
        let larger = Snapshot::from_counts(
            1202,
            9,
            counts(&[("validator", 9)]),
            counts(&[("unknown", 9)]),
        );
        let packet = Packet {
            schema: 2,
            network: "7777:0".into(),
            aggregate: larger.clone(),
        };
        assert!(table.accept(aether_net::devnet_node_id(2), packet, 1202, mono));
        assert_eq!(
            table.release(larger, 1799, mono + Duration::from_secs(598)),
            release
        );
        let next = Snapshot::from_counts(
            1800,
            3,
            counts(&[("validator", 3)]),
            counts(&[("unknown", 3)]),
        );
        assert_eq!(
            table.release(next.clone(), 1800, mono + Duration::from_secs(599)),
            next
        );

        let mut table = Table::new("7777:0".into());
        for id in 2..=3 {
            let packet = Packet {
                schema: 2,
                network: "7777:0".into(),
                aggregate: local.clone(),
            };
            assert!(table.accept(aether_net::devnet_node_id(id), packet, 1201, mono));
        }
        let tiny = Snapshot::from_counts(
            1201,
            1,
            counts(&[("follower", 1)]),
            counts(&[("unknown", 1)]),
        );
        assert_eq!(
            table.release(tiny, 1201, mono).total,
            Some(4),
            "overlapping claims never add to eight"
        );
    }

    #[test]
    fn invalid_or_identifying_aggregate_payloads_are_not_accepted() {
        let value = Snapshot::from_counts(
            1200,
            4,
            counts(&[("validator", 4)]),
            counts(&[("unknown", 4)]),
        );
        for field in [
            "nodes",
            "observer",
            "country",
            "sender",
            "signature",
            "pings",
        ] {
            let mut json = serde_json::to_value(&value).unwrap();
            json[field] = json!("private");
            assert!(serde_json::from_value::<Snapshot>(json).is_err(), "{field}");
        }
        let mut rare = value.clone();
        rare.by_role = counts(&[("validator", 3), ("follower", 1)]);
        assert!(!rare.valid(1200));
        let mut residual = value.clone();
        residual.by_region = counts(&[("asia", 3)]);
        assert!(
            !residual.valid(1200),
            "exact total cannot expose a hidden residual"
        );
        let mut exact_time = value.clone();
        exact_time.observed_at += 1;
        assert!(!exact_time.valid(1201));
        assert!(!value.valid(1800), "previous bucket is stale");
        let mut version = value.clone();
        version.by_version = counts(&[("0.7.4", 4)]);
        assert!(!version.valid(1200), "individual build quality was removed");
        let mut table = Table::new("7777:0".into());
        let wrong = Packet {
            schema: 2,
            network: "7780:0".into(),
            aggregate: value,
        };
        assert!(!table.accept(aether_net::devnet_node_id(2), wrong, 1200, Instant::now()));
        assert!(table.observations.is_empty());
    }

    #[test]
    fn country_stays_local_and_only_yields_a_broad_region() {
        assert_eq!(country_region("KR"), "030");
        assert_eq!(country_region("US"), "021");
        assert_eq!(country_region("AU"), "053");
        assert_eq!(country_region("RU"), "151");
        assert_eq!(country_region("TR"), "145");
        assert_eq!(country_region("CY"), "145");
        assert_eq!(country_region("KZ"), "143");
        assert_eq!(country_region("AQ"), "unknown");
        for code in SUBREGIONS { assert!(validate_region(Some(code)).is_ok()); }
        for code in ["asia", "KR", "30", "000", "015 "] { assert!(validate_region(Some(code)).is_err()); }
        for c in ["kr", "ZZ", "Korea", "127.0.0.1"] {
            assert!(validate_country(Some(c)).is_err());
        }
        assert!(validate_country(None).is_ok());
        assert!(validate_country(Some("KR")).is_ok());
        assert_eq!(relay_region("https://aps1-1.relay.n0.iroh.link./"), "asia");
        assert_eq!(relay_region("https://my-relay.example/"), "unknown");
        assert_eq!(
            relay_region("https://aps1-1.relay.n0.iroh.link.attacker.example/"),
            "unknown"
        );
    }

    /// Inspect actual request/reply bytes over local QUIC and the actual public
    /// RPC handler JSON, including a populated cohort with a rare residual.
    #[tokio::test]
    async fn actual_rpc_and_gossip_export_only_the_same_safe_aggregates() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let tracker = aether_net::peers::PeerTracker::new();
            let server = aether_net::bind_local(
                aether_net::devnet_node_secret(1),
                "127.0.0.1:0".parse().unwrap(),
                tracker.clone(),
            )
            .await
            .unwrap();
            let presence = Presence::new(
                server.clone(),
                tracker,
                Role::Validator,
                (1..=3).map(aether_net::devnet_node_id).collect(),
                "7777:0".into(),
                Some("KR".into()),
            );
            let captured = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
            let callback = presence.callback();
            let packets = captured.clone();
            let router = aether_net::serve_with_presence(
                server.clone(),
                |_| async { json!({}) },
                None,
                None,
                Some(Arc::new(move |remote, bytes| {
                    packets.lock().unwrap().push(bytes.to_vec());
                    callback(remote, bytes)
                })),
            );
            let address = aether_net::EndpointAddr::from_parts(
                server.id(),
                server
                    .bound_sockets()
                    .into_iter()
                    .map(aether_net::TransportAddr::Ip),
            );
            let mut clients = Vec::new();
            let mut connections = Vec::new();
            for id in 2..=4 {
                let tracker = aether_net::peers::PeerTracker::new();
                let client = aether_net::bind_local(
                    aether_net::devnet_node_secret(id),
                    "127.0.0.1:0".parse().unwrap(),
                    tracker.clone(),
                )
                .await
                .unwrap();
                connections.push(
                    client
                        .connect(address.clone(), aether_net::ALPN_RPC)
                        .await
                        .unwrap(),
                );
                let publisher = Presence::new(
                    client.clone(),
                    tracker,
                    Role::Follower,
                    vec![],
                    "7777:0".into(),
                    Some("JP".into()),
                );
                clients.push((client, publisher));
            }
            // Two pinned validator peers + self and one unclassified peer:
            // publishing validator=3,total=4 would leak that one peer.
            while presence.peers.connected_ids().len() < 3 {
                tokio::task::yield_now().await;
            }
            let sent = clients[0].1.packet();
            let reply = aether_net::presence_exchange(&clients[0].0, address, &sent)
                .await
                .unwrap();
            let received: Packet = serde_json::from_slice(&reply).unwrap();
            assert_safe(&received.aggregate);
            assert_eq!(received.aggregate.total, Some(4));
            assert_eq!(received.aggregate.by_role, counts(&[("other", 4)]));
            assert_eq!(received.aggregate.by_region, counts(&[("world", 4)]));
            assert_eq!(captured.lock().unwrap().as_slice(), &[sent.clone()]);
            for wire in [&sent, &reply] {
                let packet: Packet = serde_json::from_slice(wire).unwrap();
                assert_safe(&packet.aggregate);
                let encoded = String::from_utf8(wire.to_vec()).unwrap();
                for id in 1..=4 {
                    assert!(!encoded.contains(&aether_net::devnet_node_id(id).to_string()));
                }
                for forbidden in [
                    "node_id",
                    "observer",
                    "signature",
                    "sender",
                    "pings",
                    "country",
                    "KR",
                    "JP",
                    NODE_VERSION,
                ] {
                    assert!(
                        !encoded.contains(forbidden),
                        "{forbidden} absent from actual gossip"
                    );
                }
                assert_eq!(packet.aggregate.observed_at % TIME_BUCKET_SECONDS, 0);
            }
            let mut state = crate::rpc::bare_state();
            state.public_read_only = true;
            state.presence = Some(presence.clone());
            let answer = crate::rpc::handle_remote_value(
                &state,
                json!({"jsonrpc":"2.0","id":7,"method":"aether_presence","params":[]}),
            )
            .await;
            let serialized: Value =
                serde_json::from_slice(&serde_json::to_vec(&answer).unwrap()).unwrap();
            assert_eq!(
                serialized["result"],
                serde_json::to_value(&received.aggregate).unwrap()
            );
            assert!(!serialized.to_string().contains(&server.id().to_string()));
            presence.set_region(Some("030".into())).unwrap();
            presence.set_country(None).unwrap();
            assert_eq!(presence.own_region(), "030", "declining country retains the default region");
            assert_eq!(
                presence.snapshot(),
                serialized["result"],
                "country changes do not alter a frozen export"
            );
            assert!(presence.set_country(Some("ZZ".into())).is_err());
            presence.disable_country_settings();
            assert!(presence.set_country(Some("US".into())).is_err());
            assert!(presence.set_region(Some("021".into())).is_err());
            assert_eq!(
                presence.peer_snapshot(),
                json!([]),
                "non-loopback binds cannot expose individual diagnostics"
            );
            // The removed wire shape is rejected rather than quietly relayed.
            presence.ingest(
                clients[1].0.id(),
                br#"{"schema":1,"sender":"private","binding_signature":"private","pings":[]}"#,
            );
            assert_eq!(presence.snapshot(), serialized["result"]);
            drop(connections);
            for (client, _) in clients {
                client.close().await;
            }
            router.shutdown().await.unwrap();
        })
        .await
        .expect("local presence privacy test completed");
    }
}
