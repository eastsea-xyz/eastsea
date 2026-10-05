//! Wallet core for Swift (docs/design/09-wallet.md).
//!
//! Keys never enter Rust: the app holds a Secure Enclave P-256 key, Rust builds
//! the exact bytes to sign, Swift signs them (`SecureEnclave.P256.Signing`
//! hashes with SHA-256, matching the chain's P-256 rule), and Rust normalizes
//! the signature to low-s and submits. Every balance shown is verified through
//! a finality certificate plus an EIP-7864 proof (see `aether-light`).

uniffi::setup_scaffolding!();

mod atomic_swap;
mod paper;
use aether_crypto::{address_of, PublicKey};
use aether_execution::EvmCall;
use aether_light::{from_hex, verify_account, verify_finalized_chain, ValidatorSet, VerifiedBlock};
use aether_state::Proof;
use aether_types::{Address, Bytes, FeeVector, GasVector, SignerScheme, TxEnvelope, TxHash, TxHeader, TxPayload, U256};
pub use atomic_swap::*;
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
    /// Maximum estimated fee (wei) of a wallet plain transfer, including two
    /// possible new accounts and the transaction/receipt bytes on a paid-state genesis.
    pub transfer_fee_wei: String,
    /// Scheduled notices reported by the selected node (JSON array).
    pub upgrades_json: String,
    /// Highest chain protocol this wallet build knows how to display and submit to.
    pub supported_protocol: u32,
    /// The node's faucet, when it has one — so the wallet can name test grants
    /// in the balance breakdown instead of counting them as ordinary received.
    pub faucet: Option<String>,
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

/// A ReleaseLog entry proven against a committee-certified state root.
#[derive(uniffi::Record)]
pub struct VerifiedRelease {
    pub manifest_sha256: String,
    pub archive_sha256: String,
    pub signatures_sha256: String,
    pub published_block: u64,
    pub published_at: u64,
    pub emergency: bool,
    pub state_height: u64,
    pub certified_block: u64,
    pub certified_timestamp_ms: u64,
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
    /// The state fee actually burned (0 when the recipient already had an
    /// account) — replaces the quote's "maximum" after the send (audit 6, A6-7).
    pub state_fee_wei: String,
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
    generation: u64,
    /// The validators: the fallback, and where writes go.
    client: std::sync::Arc<aether_net::RpcClient>,
    /// Follower Macs wallet reads spread over.
    spread: std::sync::Arc<Spread>,
}

static NET: std::sync::Mutex<Option<Result<std::sync::Arc<Net>, String>>> = std::sync::Mutex::new(None);
static NETWORK_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Held only while one thread builds the client, so a build (bind + first
/// discovery) can never hold `NET` itself — `anchor`'s generation check
/// locks `NET` briefly and must not queue behind a network build.
static NET_BUILD: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Building the client is bounded too: a reboot morning's cold discovery
/// (the incident of 2026-10-05) must not wedge every remote read behind it.
const DEFAULT_NET_BUILD_TIMEOUT: Duration = Duration::from_secs(20);
/// One remote read's whole budget — followers, rotation and the validator
/// fallback included. A healthy network answers in milliseconds; anything
/// still running at this deadline is stuck, and the read says so instead of
/// hanging the caller (the incident's `recent_blocks`/`connection`/
/// `verified_account` threads, blocked without end).
const DEFAULT_READ_DEADLINE: Duration = Duration::from_secs(35);
/// The on-demand check behind `authenticated_remote_height`: short, because
/// the wallet's route decision waits on it every poll.
const DEFAULT_REMOTE_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
/// After this many consecutive failed remote reads the client is rebuilt
/// from scratch — fresh bind, fresh discovery, no cached state — because a
/// client that keeps failing is stuck on stale addresses (the validators
/// restarted on new ones), and `rotate` only re-dials the same list.
const DEFAULT_REBUILD_AFTER: u32 = 6;
/// A rebuild redoes discovery, which is exactly what a transient outage does
/// not need: never more often than this.
const DEFAULT_REBUILD_EVERY: Duration = Duration::from_secs(30);

/// The budgets above, overridable so a test's black-holed validator costs
/// milliseconds instead of its production minutes (crate-private: the
/// uniffi surface does not change).
#[derive(Clone, Copy)]
struct Budgets {
    net_build: Duration,
    read: Duration,
    remote_check: Duration,
    rebuild_after: u32,
    rebuild_every: Duration,
}

static BUDGETS: std::sync::Mutex<Option<Budgets>> = std::sync::Mutex::new(None);

fn budgets() -> Budgets {
    BUDGETS.lock().expect("budgets lock").unwrap_or(Budgets {
        net_build: DEFAULT_NET_BUILD_TIMEOUT,
        read: DEFAULT_READ_DEADLINE,
        remote_check: DEFAULT_REMOTE_CHECK_TIMEOUT,
        rebuild_after: DEFAULT_REBUILD_AFTER,
        rebuild_every: DEFAULT_REBUILD_EVERY,
    })
}

/// Consecutive remote reads that ended in a network error, and when the
/// stuck client was last rebuilt because of them (plus how many rebuilds
/// happened — the tests' evidence).
static REMOTE_FAILURES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static LAST_REBUILD: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
static REBUILDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A remote read succeeded: the network path is alive again.
fn note_remote_success() {
    REMOTE_FAILURES.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// A remote read failed (timeout, no route). After enough in a row, drop the
/// whole client — endpoint, DHT session, cached relay paths — so the next
/// read redoes discovery from scratch instead of re-dialing dead addresses
/// forever.
fn note_remote_failure() {
    let n = REMOTE_FAILURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if n < budgets().rebuild_after {
        return;
    }
    let b = budgets();
    let mut last = LAST_REBUILD.lock().expect("rebuild lock");
    if last.is_some_and(|t| t.elapsed() < b.rebuild_every) {
        return;
    }
    *last = Some(std::time::Instant::now());
    drop(last);
    REMOTE_FAILURES.store(0, std::sync::atomic::Ordering::Relaxed);
    REBUILDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // In-flight readers keep their `Arc<Net>` and finish; every new read
    // builds a fresh client.
    *NET.lock().expect("network lock") = None;
}

/// The network client, built at most once at a time. While a build is in
/// flight, other callers fail fast ("still connecting") instead of stacking
/// behind it — the caller retries on its next poll, and a wallet with a
/// healthy local node switches to it meanwhile (the incident of 2026-10-05:
/// every remote read queued behind one unbounded build, and the UI showed
/// "Connecting" for as long as the network stayed cold).
fn net() -> R<std::sync::Arc<Net>> {
    if let Some(cached) = NET.lock().expect("network lock").clone() {
        return cached.map_err(WalletError::Network);
    }
    let Ok(_building) = NET_BUILD.try_lock() else {
        return Err(WalletError::Network("the network client is still connecting".into()));
    };
    if let Some(cached) = NET.lock().expect("network lock").clone() {
        return cached.map_err(WalletError::Network);
    }
    let (generation, built) = build_net();
    let mut cached = NET.lock().expect("network lock");
    if generation != NETWORK_GENERATION.load(std::sync::atomic::Ordering::SeqCst) {
        // Reconfigured while building: this client belongs to a network the
        // wallet no longer uses. Store nothing; the next call builds fresh.
        return Err(WalletError::Network("the network changed while connecting; retrying".into()));
    }
    *cached = Some(built.clone());
    built.map_err(WalletError::Network)
}

fn build_net() -> (u64, Result<std::sync::Arc<Net>, String>) {
    let generation = NETWORK_GENERATION.load(std::sync::atomic::Ordering::SeqCst);
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => return (generation, Err(e.to_string())),
    };
    let pinned = PINNED_SERVERS.lock().expect("pinned servers lock").clone();
    let built: std::result::Result<_, String> = rt.block_on(async {
        tokio::time::timeout(budgets().net_build, async {
            let client = match &pinned {
                Some((_, validators)) => {
                    aether_net::RpcClient::with_addrs(validators.clone()).await
                }
                None => {
                    let ids = NODES
                        .lock()
                        .expect("nodes lock")
                        .clone()
                        .unwrap_or_else(|| {
                            (1..=DEVNET_VALIDATORS)
                                .map(aether_net::devnet_node_id)
                                .collect()
                        });
                    aether_net::RpcClient::new(ids).await
                }
            };
            let client = client.map_err(|e| e.to_string())?;
            let pinned_followers = pinned.as_ref().map(|(f, _)| f.clone()).unwrap_or_default();
            let endpoint = client.endpoint().clone();
            Ok((client, Spread::on(endpoint, pinned_followers)))
        })
        .await
        .map_err(|_| "discovery did not answer in time; will retry".to_string())?
    });
    let (client, spread) = match built {
        Ok(v) => v,
        Err(e) => return (generation, Err(e)),
    };
    let (client, spread) = (std::sync::Arc::new(client), std::sync::Arc::new(spread));
    // Ask the validators every few minutes which follower Macs serve
    // wallets (nothing to ask when the servers were pinned: tests).
    if pinned.is_none() {
        let (spread, refresh) = (spread.clone(), client.clone());
        rt.spawn(async move {
            loop {
                spread.discover(&refresh).await;
                tokio::time::sleep(Duration::from_secs(5 * 60)).await;
            }
        });
        // And every 30 s, the newest finalized height off a validator's
        // own certificate: the bar a serving node's anchor must not lag
        // far behind (red-team 2026-09-29 §3).
        let refresh = client.clone();
        rt.spawn(async move {
            loop {
                refresh_finalized_height(&refresh, generation).await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });
    }
    (generation, Ok(std::sync::Arc::new(Net { rt, generation, client, spread })))
}

// ---------------- wallet reads spread over follower Macs ----------------
//
// Capacity review 2026-09-29: iPhone wallets (and any wallet without its own
// node) poll every 2 s, and asking the validators directly is ~500k req/s on
// 16 validators at a million phones. Follower Macs — any Mac running
// `aether run` — announce themselves as wallet servers (a registered
// candidate's signed announcement; validators list only those), and this
// wallet keeps a few of them active, rotates away from trouble, and falls
// back to the validators when no follower answers — or every one of them is
// busy or failing (red-team 2026-09-29 §3: an all-busy pool must not deny
// reads). Nothing about verification changes: every answer still passes the
// same certificate, proof and chain checks, so a follower cannot lie — it
// can only be slow or stale, and the monotonic-height rule already rejects
// stale.

/// Followers one wallet reads from at once: the connection footprint of a phone.
const ACTIVE_FOLLOWERS: usize = 3;
/// Followers remembered at all (discovery merges into this pool).
const FOLLOWER_POOL: usize = 12;
/// How long connecting to one follower may take before the next is tried.
const CONNECT_FOLLOWER: Duration = Duration::from_secs(12);

/// Why a follower is passed over for a while.
#[derive(Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Parked {
    /// It answered "server busy" (its DoS limits): back off, politely.
    Busy,
    /// It broke (connect, stream, timeout): rotate away.
    Error,
    /// Its answers verify but lag the chain: try a fresher follower.
    Stale,
    /// It served something that failed verification: demoted.
    Lying,
}

impl Parked {
    fn as_str(self) -> &'static str {
        match self {
            Parked::Busy => "busy",
            Parked::Error => "error",
            Parked::Stale => "stale",
            Parked::Lying => "lying",
        }
    }

    /// The first park of its kind, and the longest it can grow to by doubling.
    fn base(self) -> Duration {
        match self {
            Parked::Busy => Duration::from_secs(1),
            Parked::Error => Duration::from_secs(2),
            Parked::Stale => Duration::from_secs(5),
            Parked::Lying => Duration::from_secs(10 * 60),
        }
    }

    fn cap(self) -> Duration {
        match self {
            Parked::Busy => Duration::from_secs(30),
            Parked::Error => Duration::from_secs(5 * 60),
            Parked::Stale => Duration::from_secs(60),
            Parked::Lying => Duration::from_secs(6 * 60 * 60),
        }
    }
}

/// One follower this wallet may read from, and how it has been behaving.
struct Follower {
    addr: aether_net::EndpointAddr,
    conn: Option<aether_net::Connection>,
    /// Moving average of call latency (ms); none until the first answer.
    latency_ms: Option<f64>,
    /// Passed over until this instant.
    parked: Option<(Parked, std::time::Instant)>,
    /// Parks in a row of the same kind (each doubles the next one).
    strikes: u32,
}

impl Follower {
    fn healthy(&self, now: std::time::Instant) -> bool {
        self.parked.is_none_or(|(_, until)| now >= until)
    }
}

/// Reads spread over follower Macs: round-robin over a few active ones,
/// preferring low latency when (re)filling, rotating away from trouble.
struct Spread {
    endpoint: aether_net::Endpoint,
    inner: std::sync::Mutex<Inner>,
}

struct Inner {
    followers: Vec<Follower>,
    /// The `ACTIVE_FOLLOWERS` ids requests rotate over.
    active: Vec<aether_net::EndpointId>,
    /// Next position in the rotation.
    rr: usize,
}

/// A fresh active follower: the lowest-latency of a small random sample of
/// the healthy pool (unmeasured servers win, so new blood is always tried).
fn choose(
    followers: &[Follower],
    active: &[aether_net::EndpointId],
    now: std::time::Instant,
) -> Option<aether_net::EndpointId> {
    use rand::seq::IndexedRandom as _;
    let candidates: Vec<usize> = followers
        .iter()
        .enumerate()
        .filter(|(_, f)| !active.contains(&f.addr.id) && f.healthy(now))
        .map(|(i, _)| i)
        .collect();
    (0..3)
        .filter_map(|_| candidates.choose(&mut rand::rng()).copied())
        .min_by_key(|&i| followers[i].latency_ms.map(|l| l as u64).unwrap_or(0))
        .map(|i| followers[i].addr.id)
}

impl Spread {
    /// On `endpoint` (shared with the validator client), with `pinned`
    /// followers that are never re-discovered (tests and previews).
    fn on(endpoint: aether_net::Endpoint, pinned: Vec<aether_net::EndpointAddr>) -> Spread {
        Spread {
            endpoint,
            inner: std::sync::Mutex::new(Inner {
                followers: pinned
                    .into_iter()
                    .map(|addr| Follower {
                        addr,
                        conn: None,
                        latency_ms: None,
                        parked: None,
                        strikes: 0,
                    })
                    .collect(),
                active: Vec::new(),
                rr: 0,
            }),
        }
    }

    /// Ask the validators which follower Macs serve wallets, and merge them
    /// into the pool (a healthy server is never dropped for a new one; the
    /// longest-parked inactive one makes room).
    async fn discover(&self, validators: &aether_net::RpcClient) {
        let Ok(list) = validators.call("aether_walletServers", json!([])).await else {
            return;
        };
        let ids = list.as_array().map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().and_then(|s| s.parse().ok()))
                .collect::<Vec<aether_net::EndpointId>>()
        });
        let Some(ids) = ids else { return };
        let mut g = self.inner.lock().expect("spread lock");
        for id in ids {
            if g.followers.iter().any(|f| f.addr.id == id) {
                continue;
            }
            if g.followers.len() >= FOLLOWER_POOL {
                let room = (0..g.followers.len())
                    .filter(|&i| {
                        !g.active.contains(&g.followers[i].addr.id)
                            && g.followers[i].parked.is_some()
                    })
                    .max_by_key(|&i| g.followers[i].parked.expect("checked").1);
                match room {
                    Some(victim) => {
                        g.followers.remove(victim);
                    }
                    None => return, // full of healthy servers: nothing to add
                }
            }
            g.followers.push(Follower {
                addr: aether_net::EndpointAddr::from(id),
                conn: None,
                latency_ms: None,
                parked: None,
                strikes: 0,
            });
        }
    }

    /// The next follower to ask, refilling the rotation first: round-robin
    /// over up to `ACTIVE_FOLLOWERS` healthy ones.
    fn pick(&self) -> Option<(aether_net::EndpointId, aether_net::EndpointAddr)> {
        let mut g = self.inner.lock().expect("spread lock");
        let now = std::time::Instant::now();
        let mut active: Vec<aether_net::EndpointId> = Vec::new();
        for id in g.active.iter() {
            if g.followers
                .iter()
                .any(|f| &f.addr.id == id && f.healthy(now))
            {
                active.push(*id);
            }
        }
        while active.len() < ACTIVE_FOLLOWERS {
            let Some(id) = choose(&g.followers, &active, now) else {
                break;
            };
            active.push(id);
        }
        g.active = active;
        g.rr = g.rr.wrapping_add(1);
        let id = *g.active.get(g.rr % g.active.len().max(1))?;
        let addr = g.followers.iter().find(|f| f.addr.id == id)?.addr.clone();
        Some((id, addr))
    }

    /// A healthy cached connection to `id`, or a fresh one.
    async fn conn_to(
        &self,
        id: &aether_net::EndpointId,
        addr: &aether_net::EndpointAddr,
    ) -> Result<aether_net::Connection, aether_net::RpcError> {
        if let Some(c) = self
            .inner
            .lock()
            .expect("spread lock")
            .followers
            .iter()
            .find(|f| &f.addr.id == id)
            .and_then(|f| {
                f.conn
                    .as_ref()
                    .filter(|c| c.close_reason().is_none())
                    .cloned()
            })
        {
            return Ok(c);
        }
        let c = aether_net::connect_rpc(&self.endpoint, addr, CONNECT_FOLLOWER)
            .await
            .map_err(aether_net::RpcError::Transport)?;
        if let Some(f) = self
            .inner
            .lock()
            .expect("spread lock")
            .followers
            .iter_mut()
            .find(|f| &f.addr.id == id)
        {
            f.conn = Some(c.clone());
        }
        Ok(c)
    }

    /// Record that `id` answered in `took` (a healthy server again).
    fn answered(&self, id: &aether_net::EndpointId, took: Duration) {
        let mut g = self.inner.lock().expect("spread lock");
        let Some(f) = g.followers.iter_mut().find(|f| &f.addr.id == id) else {
            return;
        };
        let ms = took.as_secs_f64() * 1000.0;
        f.latency_ms = Some(match f.latency_ms {
            Some(ema) => ema * 0.7 + ms * 0.3,
            None => ms,
        });
        f.strikes = 0;
        f.parked = None;
    }

    /// Pass `id` over for a while, longer every time in a row.
    fn park(&self, id: &aether_net::EndpointId, why: Parked) {
        let mut g = self.inner.lock().expect("spread lock");
        let Some(f) = g.followers.iter_mut().find(|f| &f.addr.id == id) else {
            return;
        };
        f.strikes = if f.parked.is_some_and(|(w, _)| w == why) {
            f.strikes + 1
        } else {
            1
        };
        let mut for_how_long = why.base();
        for _ in 1..f.strikes {
            for_how_long = (for_how_long * 2).min(why.cap());
        }
        f.parked = Some((why, std::time::Instant::now() + for_how_long));
        if matches!(why, Parked::Error | Parked::Lying) {
            f.conn = None;
        }
        g.active.retain(|a| a != id);
    }

    /// Healthy followers right now (display and diagnostics).
    fn healthy(&self) -> usize {
        let g = self.inner.lock().expect("spread lock");
        let now = std::time::Instant::now();
        g.followers.iter().filter(|f| f.healthy(now)).count()
    }

    /// The pool as it stands (diagnostics; tests).
    fn describe(&self) -> Vec<WalletServerInfo> {
        let g = self.inner.lock().expect("spread lock");
        let now = std::time::Instant::now();
        g.followers
            .iter()
            .map(|f| WalletServerInfo {
                node: f.addr.id.to_string(),
                active: g.active.contains(&f.addr.id),
                latency_ms: f.latency_ms.map(|l| l as u32),
                parked: f.parked.map(|(w, _)| w.as_str().to_string()),
                parked_for_ms: f
                    .parked
                    .map(|(_, until)| until.saturating_duration_since(now).as_millis() as u64),
            })
            .collect()
    }
}

impl Net {
    /// One read: follower Macs first, rotating over the active ones on
    /// trouble, the validators as the fallback when no follower answers —
    /// or every one asked is busy or failing (red-team 2026-09-29 §3: an
    /// all-busy pool must not turn into a denial of service). A single busy
    /// follower is still not a reason: the rotation simply goes around it,
    /// and it backs off politely.
    async fn read(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        for _ in 0..=ACTIVE_FOLLOWERS {
            let Some((id, addr)) = self.spread.pick() else {
                break;
            };
            let conn = match self.spread.conn_to(&id, &addr).await {
                Ok(c) => c,
                Err(_) => {
                    self.spread.park(&id, Parked::Error);
                    continue;
                }
            };
            let started = std::time::Instant::now();
            match aether_net::rpc_call(&conn, method, params.clone()).await {
                Ok(v) => {
                    self.spread.answered(&id, started.elapsed());
                    note_served(id);
                    return Ok(v);
                }
                Err(aether_net::RpcError::Server { message, .. })
                    if message.contains("server busy") =>
                {
                    self.spread.park(&id, Parked::Busy);
                    continue;
                }
                Err(aether_net::RpcError::Server { message, .. }) => {
                    // The follower's own answer (e.g. a pruned height): pass it on.
                    self.spread.answered(&id, started.elapsed());
                    note_served(id);
                    return Err(anyhow::anyhow!(message));
                }
                Err(aether_net::RpcError::Transport(_)) => {
                    self.spread.park(&id, Parked::Error);
                    continue;
                }
            }
        }
        self.client.call(method, params).await
    }
}

thread_local! {
    /// The followers that served the current verified read (this thread): a
    /// verification failure parks exactly these.
    static SERVED_BY: std::cell::RefCell<Vec<aether_net::EndpointId>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn note_served(id: aether_net::EndpointId) {
    SERVED_BY.with_borrow_mut(|v| {
        if !v.contains(&id) {
            v.push(id);
        }
        while v.len() > 4 {
            v.remove(0);
        }
    });
}

/// Wrap a verified read so its failures demote whoever served them: stale
/// answers (a follower behind the chain, or a block not finalized yet) only
/// rotate away briefly, while anything that failed the certificate, proof or
/// chain checks is demoted for a long while — and the validator that served
/// it is rotated past. Nothing about the checks themselves changes.
fn demote_on_failure<T>(read: impl FnOnce() -> R<T>) -> R<T> {
    SERVED_BY.with_borrow_mut(Vec::clear);
    let r = read();
    if r.is_err() && LOCAL_NODE.lock().expect("local node lock").is_none() {
        if let Ok(n) = net() {
            let lying = !matches!(&r, Err(e) if e.to_string().contains("stale") || e.to_string().contains("never go back") || e.to_string().contains("not finalized yet"));
            SERVED_BY.with_borrow(|served| {
                for id in served {
                    n.spread
                        .park(id, if lying { Parked::Lying } else { Parked::Stale });
                }
            });
            if lying {
                n.rt.block_on(n.client.rotate());
            }
        }
    }
    r
}

/// Fee caps from the node's next base fees and the sender's balance: 2x
/// headroom plus a 1 gwei tip (only base + tip is charged). A sender with no
/// balance, or a chain at its zero floor (below target load the base fee is 0),
/// sends with tip 0 and exec capped at base × 2 — one base-fee doubling of
/// headroom. A paid-state genesis also needs a native balance for the sender
/// account and persisted transaction/receipt bytes. Older nodes without
/// `base_fee` get the floor.
fn fee_caps(status: &Value, balance: Option<U256>) -> (FeeVector, u128) {
    const GWEI: u128 = 1_000_000_000;
    let get = |k: &str| {
        status["base_fee"][k]
            .as_str()
            .and_then(|v| v.parse::<u128>().ok())
            .unwrap_or(GWEI)
    };
    let free = get("exec") == 0 || balance == Some(U256::ZERO);
    (
        FeeVector {
            exec: if free { get("exec") * 2 } else { get("exec") * 2 + GWEI },
            state: get("state").max(aether_execution::fees::STATE_UNIT_PRICE),
            prove: get("prove") * 2,
        },
        if free { 0 } else { GWEI },
    )
}

fn check_paid_state_balance(state_price: u128, balance: Option<U256>) -> R<()> {
    if state_price > 0 {
        match balance {
            None => return Err(WalletError::Invalid("Could not read the AETH balance needed to pay this transaction's state fee".into())),
            Some(b) if b.is_zero() => return Err(WalletError::Invalid("Add AETH before sending: this transaction must pay for its persistent bytes and first-use account".into())),
            Some(_) => {}
        }
    }
    Ok(())
}

/// The fee a plain native transfer is quoted (audit 6, A6-7): the exec fee,
/// plus — until a certified account proves the recipient exists — the
/// mandatory new-recipient state charge. `fee_is_maximum` tells the UI the
/// number still carries a charge an existing recipient will not pay, so the
/// sendable balance must reserve it.
#[derive(uniffi::Record)]
pub struct TransferQuote {
    pub fee_wei: String,
    /// The part of `fee_wei` only a recipient without an account pays.
    pub new_recipient_charge_wei: String,
    /// False once the recipient's certified account exists (the quote is exact).
    pub fee_is_maximum: bool,
}

/// A 21k-gas transfer runs no bytecode (no prove gas): it pays base + tip per
/// gas, and nothing while the base fee is 0 — but a recipient without an
/// account still burns the fixed 100-unit state charge, whatever the base fee
/// is (audit 6, A6-7). `recipient_exists` unknown (`None`) keeps the charge:
/// the quote is then a maximum.
fn quote_from_status(status: &Value, recipient_exists: Option<bool>) -> TransferQuote {
    const GWEI: u128 = 1_000_000_000;
    // Canonical wallet plain transfers plus the 128-byte receipt base fit
    // within 1024 bytes. State growth charges one unit per 32 persisted bytes.
    const PLAIN_TRANSFER_PERSISTENT_UNITS: u128 = 32;
    let base = status["base_fee"]["exec"]
        .as_str()
        .and_then(|v| v.parse::<u128>().ok())
        .unwrap_or(0);
    let exec = if base == 0 { 0 } else { 21_000u128.saturating_mul(base.saturating_add(GWEI)) };
    // The state price the node reports: the fixed unit price on a paid-state
    // genesis, 0 on the legacy 7780 chain (no state charge there).
    let state_price = status["base_fee"]["state"]
        .as_str()
        .and_then(|v| v.parse::<u128>().ok())
        .unwrap_or(0);
    let account = u128::from(aether_execution::fees::STATE_ACCOUNT_UNITS).saturating_mul(state_price);
    // A recipient without a certified account pays one new account (A6-7).
    let charge = if recipient_exists == Some(true) { 0 } else { account };
    // Always reserved: a possible first-use sender account (audit 6, A6-2) and
    // the persisted transaction/receipt bytes, both upper bounds — so the quote
    // is always a maximum; the receipt shows the actual charge.
    let fixed = account.saturating_add(PLAIN_TRANSFER_PERSISTENT_UNITS.saturating_mul(state_price));
    TransferQuote {
        fee_wei: exec.saturating_add(charge).saturating_add(fixed).to_string(),
        new_recipient_charge_wei: charge.to_string(),
        fee_is_maximum: true,
    }
}

/// Whether `address` holds a non-empty account in certified state (empty
/// accounts are cleared, so nonce-or-balance is existence). `None` when the
/// account cannot be verified right now — the quote then stays a maximum,
/// which is the safe side (audit 6, A6-7).
fn certified_recipient_exists(address: &str, validators: u32) -> Option<bool> {
    let a = verified_account_at(address.to_string(), validators).ok()?;
    Some(a.nonce > 0 || a.balance_wei != "0")
}

/// The send sheet's fee for a plain transfer to `recipient` (audit 6, A6-7):
/// the exec fee plus the possible new-recipient state charge, exact (not a
/// maximum) once the recipient's certified account exists.
#[uniffi::export]
pub fn transfer_quote(recipient: String, validators: u32) -> R<TransferQuote> {
    let a: Address = recipient.parse().map_err(|_| WalletError::Invalid("recipient address".into()))?;
    let status = call("aether_status", json!([]))?;
    let exists = certified_recipient_exists(&a.to_checksum(None), validators);
    Ok(quote_from_status(&status, exists))
}

/// A U256 the way this chain's JSON may spell it: a decimal string, a 0x-hex
/// string, or a bare number. `None` when absent or unparsable.
fn u256_of_json(v: &Value) -> Option<U256> {
    if let Some(s) = v.as_str() {
        return s
            .strip_prefix("0x")
            .and_then(|h| U256::from_str_radix(h, 16).ok())
            .or_else(|| s.parse().ok());
    }
    v.as_u64().map(U256::from)
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

/// Methods only a validator serves: writes, and the registrar's attestation.
/// Everything else a follower Mac can answer, and reads spread over those.
fn needs_a_validator(method: &str) -> bool {
    matches!(
        method,
        "aether_sendTransaction"
            | "aether_faucet"
            | "aether_registerDevice"
            | "aether_sendBeacon"
            | "aether_sendRegistration"
            | "aether_reattest"
            | "aether_submitProof"
            | "aether_signHandoff"
    )
}

fn call(method: &str, params: Value) -> R<Value> {
    // Copy the setting out first: never hold the lock across a network read.
    let local = *LOCAL_NODE.lock().expect("local node lock");
    if let Some(port) = local {
        return local_call(port, method, params);
    }
    let n = net()?;
    // The whole read — followers, rotation, validator fallback — runs under
    // one deadline: a stuck path returns an error the caller can show and
    // retry, never a thread parked inside the FFI call (2026-10-05).
    // The timeout future is built inside `block_on`: constructing a timer
    // outside the runtime ("no reactor running") is a panic, not an error.
    let v = n.rt.block_on(async {
        tokio::time::timeout(budgets().read, async {
            if needs_a_validator(method) {
                n.client.call(method, params).await
            } else {
                n.read(method, params).await
            }
        })
        .await
    });
    let v = match v {
        Ok(v) => v,
        Err(_) => Err(anyhow::anyhow!(
            "timed out after {} ms waiting for the network",
            budgets().read.as_millis()
        )),
    };
    let out = v.map_err(|e| {
        let m = e.to_string();
        if m.contains("connect")
            || m.contains("timed out")
            || m.contains("stream")
            || m.contains("server busy")
        {
            WalletError::Network(m)
        } else {
            WalletError::Rejected(m)
        }
    });
    match &out {
        Ok(_) => note_remote_success(),
        Err(WalletError::Network(_)) => note_remote_failure(),
        Err(_) => {}
    }
    out
}

/// How the wallet currently reaches the network (for display).
#[uniffi::export]
pub fn connection() -> String {
    let local = *LOCAL_NODE.lock().expect("local node lock");
    if let Some(port) = local {
        return format!("This Mac's node (127.0.0.1:{port})");
    }
    match net() {
        Ok(n) => {
            let validators = n.rt.block_on(n.client.describe());
            match n.spread.healthy() {
                0 => format!("Mainline DHT · {validators}"),
                followers => format!("{followers} follower Macs · {validators}"),
            }
        }
        Err(e) => format!("offline: {e}"),
    }
}

/// A server by node id and one socket address, for pinning (tests, previews).
#[derive(uniffi::Record)]
pub struct PinnedServer {
    pub node: String,
    pub socket: String,
}

/// Pin the follower Macs and validators to fixed addresses, bypassing
/// discovery (tests and previews). Call before anything else.
#[doc(hidden)]
#[uniffi::export]
pub fn pin_servers(followers: Vec<PinnedServer>, validators: Vec<PinnedServer>) -> R<()> {
    let parse = |s: &PinnedServer| -> R<aether_net::EndpointAddr> {
        let id: aether_net::EndpointId = s
            .node
            .parse()
            .map_err(|e| WalletError::Invalid(format!("node id: {e}")))?;
        let sa: std::net::SocketAddr = s
            .socket
            .parse()
            .map_err(|e| WalletError::Invalid(format!("socket: {e}")))?;
        Ok(aether_net::EndpointAddr::from_parts(
            id,
            [aether_net::TransportAddr::Ip(sa)],
        ))
    };
    let (f, v) = (
        followers.iter().map(&parse).collect::<R<Vec<_>>>()?,
        validators.iter().map(&parse).collect::<R<Vec<_>>>()?,
    );
    *PINNED_SERVERS.lock().expect("pinned servers lock") = Some((f, v));
    Ok(())
}

/// One follower Mac in the wallet's read pool (diagnostics; tests).
#[derive(Debug, uniffi::Record)]
pub struct WalletServerInfo {
    pub node: String,
    /// Requests rotate over it right now.
    pub active: bool,
    /// Moving average of its call latency.
    pub latency_ms: Option<u32>,
    /// Why it is passed over, when it is.
    pub parked: Option<String>,
    pub parked_for_ms: Option<u64>,
}

/// The follower Macs this wallet's reads spread over (diagnostics; tests).
#[uniffi::export]
pub fn wallet_servers() -> Vec<WalletServerInfo> {
    match net() {
        Ok(n) => n.spread.describe(),
        Err(_) => Vec::new(),
    }
}

fn parse<T: serde::de::DeserializeOwned>(v: &Value, what: &str) -> R<T> {
    serde_json::from_value(v.clone()).map_err(|e| WalletError::Invalid(format!("{what}: {e}")))
}

fn p256_key(compressed: &[u8]) -> R<PublicKey> {
    // Accept compressed (33) or X9.63 uncompressed (65) SEC1 and normalize to compressed.
    let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(compressed)
        .map_err(|_| WalletError::Invalid("P-256 public key".into()))?;
    Ok(PublicKey {
        scheme: SignerScheme::P256,
        bytes: vk.to_sec1_point(true).as_bytes().to_vec(),
    })
}

#[uniffi::export]
pub fn chain_status() -> R<ChainStatus> {
    let v = call("aether_status", json!([]))?;
    Ok(ChainStatus {
        chain_id: v["chain_id"].as_u64().unwrap_or_default(),
        height: v["height"].as_u64().unwrap_or_default(),
        state_root: v["state_root"].as_str().unwrap_or_default().to_string(),
        mempool: v["mempool"].as_u64().unwrap_or_default(),
        transfer_fee_wei: quote_from_status(&v, None).fee_wei,
        upgrades_json: scheduled_upgrade_json(&v),
        // Bump with the bundled node/light-client release, not with a remote node's version.
        supported_protocol: 3,
        faucet: v["faucet"].as_str().map(str::to_string),
    })
}

/// Only show notices signed by the pinned committee and present in the node's
/// finalized schedule. Status RPC is otherwise an untrusted read.
fn scheduled_upgrade_json(status: &Value) -> String {
    let Ok(chain) = expected_chain(status) else { return "[]".into() };
    let validators = NODES.lock().expect("nodes lock").as_ref().map_or(4, |n| n.len() as u32);
    let Ok(set) = trusted_set(validators) else { return "[]".into() };
    let Some(schedule) = status["schedule"].as_array() else { return "[]".into() };
    let notices = status["upcoming_upgrades"].as_array().into_iter().flatten().filter_map(|value| {
        let signed: aether_light::block::SignedUpgrade = serde_json::from_value(value.clone()).ok()?;
        let u = &signed.upgrade;
        if u.chain_id != chain || u.activate_at <= status["height"].as_u64()? {
            return None;
        }
        if !schedule.iter().any(|a| a[0].as_u64() == Some(u.protocol as u64) && a[1].as_u64() == Some(u.activate_at)) {
            return None;
        }
        aether_light::verify_upgrade(set.identity(), &signed).ok()?;
        Some(json!({ "protocol": u.protocol, "activate_at": u.activate_at, "emergency": u.emergency, "notes": u.notes }))
    }).collect::<Vec<_>>();
    Value::Array(notices).to_string()
}

/// The chain id this wallet is configured for (network.json; the devnet otherwise).
/// Never taken from a node (see `CHAIN_ID`).
#[uniffi::export]
pub fn configured_chain_id() -> u64 {
    *CHAIN_ID.lock().expect("chain id lock")
}

/// Params for a read-only `eth_call` at the latest block, after checking the inputs.
fn eth_call_params(to: &str, data_hex: &str) -> R<Value> {
    let addr: Address = to.trim().parse().map_err(|_| WalletError::Invalid(format!("{to} is not a 0x address")))?;
    let data = data_hex.trim();
    let body = data.strip_prefix("0x").unwrap_or(data);
    if body.len() % 2 != 0 || !body.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(WalletError::Invalid("call data must be hex".into()));
    }
    Ok(json!([{ "to": format!("{addr:#x}"), "data": format!("0x{body}") }, "latest"]))
}

/// Read-only contract call (`eth_call` at the latest block) through the node the
/// wallet uses. Returns the 0x-hex return data. Signs nothing; the answer is the
/// node's, not light-client verified.
#[uniffi::export]
pub fn eth_call(to: String, data_hex: String) -> R<String> {
    let v = call("eth_call", eth_call_params(&to, &data_hex)?)?;
    v.as_str().map(str::to_string).ok_or_else(|| WalletError::Rejected(format!("eth_call returned {v}")))
}

/// Account address for a Secure Enclave P-256 public key.
#[uniffi::export]
pub fn account_address(p256_public_key: Vec<u8>) -> R<String> {
    let pk = p256_key(&p256_public_key)?;
    Ok(address_of(&pk).map_err(|e| WalletError::Invalid(e.to_string()))?.to_checksum(None))
}

fn anchor(height: u64, set: &ValidatorSet) -> R<VerifiedBlock> {
    let generation = NETWORK_GENERATION.load(std::sync::atomic::Ordering::SeqCst);
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
            // The chain this wallet is for, before any of this node's state is used.
            let chain = expected_chain(&call("aether_status", json!([]))?)?;
            let vb = verify_finalized_chain(set, &block, &fin, &links).map_err(|e| WalletError::Verification(format!("certificate: {e}")))?;
            if vb.height != height + 1 {
                return Err(WalletError::Verification(format!("asked for block {}, got one for {}", height + 1, vb.height)));
            }
            check_anchor_chain(&block, &links, chain)?;
            let _network = NET.lock().expect("network lock");
            if generation != NETWORK_GENERATION.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(WalletError::Verification("network changed during verification".into()));
            }
            remember_height(chain, vb.height)?;
            check_freshness(vb.timestamp_ms)?;
            check_lag(chain, vb.height)?;
            return Ok(vb);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(WalletError::Network(format!("block {} not finalized yet", height + 1)))
}

/// Verified state older than this is refused (clock skew and a slow network included).
const MAX_ANCHOR_AGE_MS: u64 = 10 * 60 * 1000;

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(u64::MAX)
}

/// A valid but old certificate would let a node replay past state (e.g. an old,
/// looser session that the owner then re-signs): require a recent one.
fn check_freshness(timestamp_ms: u64) -> R<()> {
    let now = now_ms();
    if now.saturating_sub(timestamp_ms) > MAX_ANCHOR_AGE_MS {
        return Err(WalletError::Verification(format!("the node served state from {} s ago; refusing stale data", (now - timestamp_ms) / 1000)));
    }
    Ok(())
}

/// How far an anchor may lag the newest finalized height a validator's own
/// certificate showed (red-team 2026-09-29 §3): a follower keeps a small
/// natural lag, but one far behind serves old state as if current.
const MAX_FOLLOWER_LAG: u64 = 30;
/// How long a checked height stays usable; the refresh loop keeps it fresh,
/// and a wallet offline longer than this checks nothing rather than something
/// stale itself.
const CHECKED_HEIGHT_MAX_AGE: Duration = Duration::from_secs(10 * 60);

/// Newest finalized height learned from a validator with its certificate
/// verified, one entry per chain, with when. Never taken from the node serving
/// the anchor (see [`refresh_finalized_height`]).
static CHECKED_HEIGHT: std::sync::Mutex<Vec<(u64, u64, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());

/// An anchor more than [`MAX_FOLLOWER_LAG`] behind the newest finalized height
/// a validator's certificate showed is old state served as current, whatever
/// fresh-looking timestamp it carries (the monotonic rule still applies on top).
fn check_lag(chain_id: u64, height: u64) -> R<()> {
    let g = CHECKED_HEIGHT.lock().expect("checked height lock");
    for (chain, newest, at) in g.iter() {
        if *chain == chain_id
            && at.elapsed() < CHECKED_HEIGHT_MAX_AGE
            && newest.saturating_sub(height) > MAX_FOLLOWER_LAG
        {
            return Err(WalletError::Verification(format!(
                "the node serves block {height}, but validators finalized block {newest}: stale data"
            )));
        }
    }
    Ok(())
}

/// Remember the newest finalized height a validator's own certificate showed
/// (kept per chain, never going back — a validator replaying an old
/// certificate cannot lower the bar — and current: every successful refresh
/// says the validators were just asked).
fn remember_checked_height(chain_id: u64, height: u64) {
    let mut g = CHECKED_HEIGHT.lock().expect("checked height lock");
    match g.iter_mut().find(|(c, _, _)| *c == chain_id) {
        Some((_, best, at)) => {
            *best = (*best).max(height);
            *at = std::time::Instant::now();
        }
        None => g.push((chain_id, height, std::time::Instant::now())),
    }
}

/// How many validators the wallet is configured with (the committee's size,
/// which `ValidatorSet::devnet` needs in dev mode).
fn validator_count() -> u32 {
    NODES.lock().expect("nodes lock").as_ref().map_or(DEVNET_VALIDATORS as u32, |n| n.len() as u32)
}

/// Ask a validator for the newest finalized height and verify the certificate
/// it answers with (never a follower's, and never the status's word alone):
/// the same checks an anchor passes, minus serving it to anyone. A failure
/// simply leaves the last checked height in place.
async fn refresh_finalized_height(client: &aether_net::RpcClient, generation: u64) {
    let Ok(set) = trusted_set(validator_count()) else { return };
    let Ok(status) = client.call("aether_status", json!([])).await else { return };
    let (Ok(chain), Some(hint)) = (expected_chain(&status), status["height"].as_u64()) else { return };
    let Ok(v) = client.call("aether_getFinalized", json!([hint])).await else { return };
    if v.is_null() {
        return;
    }
    let (block, fin) = match (v["block"].as_str(), v["finalization"].as_str()) {
        (Some(b), Some(f)) => (from_hex(b).ok(), from_hex(f).ok()),
        _ => return,
    };
    let (Some(block), Some(fin)) = (block, fin) else { return };
    let links = v["links"]
        .as_array()
        .map(|a| a.iter().filter_map(|l| from_hex(l.as_str().unwrap_or_default()).ok()).collect::<Vec<_>>())
        .unwrap_or_default();
    let Ok(vb) = verify_finalized_chain(&set, &block, &fin, &links) else { return };
    let _network = NET.lock().expect("network lock");
    if generation == NETWORK_GENERATION.load(std::sync::atomic::Ordering::SeqCst) && check_anchor_chain(&block, &links, chain).is_ok() {
        remember_checked_height(chain, vb.height);
    }
}

/// Finalized blocks never go back: an anchor below the highest height already
/// verified on that chain is a node replaying old state, not a newer balance.
fn remember_height(chain_id: u64, height: u64) -> R<()> {
    let mut seen = VERIFIED_HEIGHT.lock().expect("verified height lock");
    match seen.iter_mut().find(|(c, _)| *c == chain_id) {
        Some((_, best)) if height < *best => Err(WalletError::Verification(format!(
            "the node served block {height}, but this wallet already verified block {best} of chain {chain_id}: finalized blocks never go back"
        ))),
        Some((_, best)) => {
            *best = (*best).max(height);
            Ok(())
        }
        None => {
            seen.push((chain_id, height));
            Ok(())
        }
    }
}

/// Every chain id a certified block commits to: the headers of its transactions
/// and any committee-signed upgrade. The certificate covers the block's digest,
/// so these are signed — a mismatch is another network's state, not a
/// mislabeled answer. A block with no transaction and no upgrade carries no
/// chain id, and is bound only by the pinned identity.
fn committed_chain_ids(block: &[u8]) -> R<Vec<u64>> {
    use commonware_codec::Decode;
    let b = aether_light::block::Block::decode_cfg(block, &aether_light::block::Block::codec_config(aether_light::MAX_BLOCK_BYTES))
        .map_err(|e| WalletError::Verification(format!("block: {e}")))?;
    let payload = b.payload().ok_or_else(|| WalletError::Verification("block payload".into()))?;
    let mut ids: Vec<u64> = payload.txs.iter().map(|tx| tx.header.chain_id).collect();
    ids.extend(payload.upgrade.iter().map(|u| u.upgrade.chain_id));
    Ok(ids)
}

/// The anchor (and the links leading to its certificate) must be for the chain
/// this wallet is configured for.
fn check_anchor_chain(block: &[u8], links: &[Vec<u8>], chain_id: u64) -> R<()> {
    for b in std::iter::once(block).chain(links.iter().map(|l| l.as_slice())) {
        for id in committed_chain_ids(b)? {
            if id != chain_id {
                return Err(WalletError::Verification(format!("the certificate is for chain {id}, but this wallet is for chain {chain_id}")));
            }
        }
    }
    Ok(())
}

static COMMITTEE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static NODES: std::sync::Mutex<Option<Vec<aether_net::EndpointId>>> = std::sync::Mutex::new(None);
/// The chain's consensus group (0 today): certificates verify under its
/// namespace and every block must carry it.
static GROUP: std::sync::Mutex<u16> = std::sync::Mutex::new(0);

/// Fixed follower and validator addresses, when discovery must not run
/// (tests pin fake servers on both sides). Set by `pin_servers`.
static PINNED_SERVERS: std::sync::Mutex<Option<(Vec<aether_net::EndpointAddr>, Vec<aether_net::EndpointAddr>)>> = std::sync::Mutex::new(None);

/// Dev mode, on only if someone asked for it (`use_devnet_keys`, or
/// `"devnet": true` in network.json): the public devnet committee key may then
/// stand in for a pinned identity. Never the silent default.
static DEVNET_KEYS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The chain every signature is for: from network.json, else the public devnet.
/// Never taken from a node: a malicious node could otherwise collect signatures
/// valid on another network where the same account holds funds.
static CHAIN_ID: std::sync::Mutex<u64> = std::sync::Mutex::new(7_777);

/// Highest anchor height verified so far, one entry per chain.
static VERIFIED_HEIGHT: std::sync::Mutex<Vec<(u64, u64)>> = std::sync::Mutex::new(Vec::new());

/// Trust the public devnet committee key (reproducible from a fixed seed, so it
/// proves nothing about any real network). Explicit opt-in for development.
#[uniffi::export]
pub fn use_devnet_keys() {
    DEVNET_KEYS.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn devnet_keys() -> bool {
    DEVNET_KEYS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The highest block height this process verified a certificate for, on the
/// chain this wallet is configured for (0 before the first one).
#[uniffi::export]
pub fn verified_height() -> u64 {
    let chain = *CHAIN_ID.lock().expect("chain id lock");
    VERIFIED_HEIGHT.lock().expect("verified height lock").iter().find(|(c, _)| *c == chain).map(|(_, h)| *h).unwrap_or(0)
}

/// A recent height proven by a validator's finality certificate, queried
/// through the remote client even while wallet reads use the local node.
/// The watchdog must never compare a local height with another local status
/// response, nor with a remote peer's unauthenticated numeric claim.
#[uniffi::export]
pub fn authenticated_remote_height() -> R<Option<u64>> {
    // A local development network has no remote validator set to compare with.
    if devnet_keys() && LOCAL_NODE.lock().expect("local node lock").is_some() {
        return Ok(None);
    }
    let remote = net()?;
    let chain = *CHAIN_ID.lock().expect("chain id lock");
    if let Some(height) = CHECKED_HEIGHT.lock().expect("checked height lock")
        .iter()
        .find(|(id, _, at)| *id == chain && at.elapsed() < Duration::from_secs(8))
        .map(|(_, height, _)| *height) {
        return Ok(Some(height));
    }
    // Bounded, because the route decision waits on this every poll: a cold
    // network (the incident of 2026-10-05) answers "unknown" in seconds, and
    // the wallet proceeds on its local node with the check marked pending
    // instead of camping on "Connecting" for minutes.
    if remote
        .rt
        .block_on(async {
            tokio::time::timeout(budgets().remote_check, refresh_finalized_height(&remote.client, remote.generation)).await
        })
        .is_err()
    {
        note_remote_failure();
    }
    Ok(recent_checked_height(chain))
}

fn recent_checked_height(chain: u64) -> Option<u64> {
    CHECKED_HEIGHT.lock().expect("checked height lock")
        .iter()
        .find(|(id, _, at)| *id == chain && at.elapsed() < Duration::from_secs(30))
        .map(|(_, height, _)| *height)
}

fn expected_chain(status: &Value) -> R<u64> {
    let want = *CHAIN_ID.lock().expect("chain id lock");
    match status["chain_id"].as_u64() {
        Some(c) if c == want => Ok(want),
        Some(c) => Err(WalletError::Verification(format!("the node reports chain {c}, but this wallet is for chain {want}"))),
        None => Err(WalletError::Verification(format!("the node does not say which chain it is on; this wallet is for chain {want}"))),
    }
}

/// Configure from network.json: the validators' node ids (looked up in the
/// Mainline DHT) and the committee identity to pin. Call before anything else.
/// The identity is required outside dev mode: without one there is nothing to
/// verify a certificate against, so every verification API refuses to run.
#[uniffi::export]
pub fn configure_network(network_json: String) -> R<u32> {
    let v: Value = serde_json::from_str(&network_json).map_err(|e| WalletError::Invalid(format!("network.json: {e}")))?;
    let group = match v.get("group") {
        None | Some(serde_json::Value::Null) => 0,
        Some(value) => value.as_u64().and_then(|g| u16::try_from(g).ok())
            .ok_or_else(|| WalletError::Invalid("network.json: group".into()))?,
    };
    let nodes = v["validators"]
        .as_array()
        .ok_or_else(|| WalletError::Invalid("network.json: validators".into()))?
        .iter()
        .map(|m| m["node"].as_str().unwrap_or_default().parse::<aether_net::EndpointId>().map_err(|e| WalletError::Invalid(format!("node id: {e}"))))
        .collect::<R<Vec<_>>>()?;
    let devnet = v["devnet"].as_bool().unwrap_or(false);
    let identity = v["identity"].as_str();
    if identity.is_none() && !devnet {
        return Err(WalletError::Invalid(
            "network.json: \"identity\" (the committee key, printed by `aether dkg`) is required; local development needs \"devnet\": true".into(),
        ));
    }
    let chain = v["chain_id"].as_u64().ok_or_else(|| WalletError::Invalid("network.json: chain_id".into()))?;
    if let Some(id) = identity {
        ValidatorSet::from_hex(id).map_err(|e| WalletError::Invalid(format!("identity: {e}")))?;
    }
    // The old client, certificate floor and local route belong to the old
    // configuration. In particular, a devnet key must not survive a return
    // to the bundled network.
    let mut cached = NET.lock().expect("network lock");
    NETWORK_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *cached = None;
    *COMMITTEE.lock().expect("committee lock") = identity.map(str::to_owned);
    DEVNET_KEYS.store(devnet, std::sync::atomic::Ordering::Relaxed);
    *CHAIN_ID.lock().expect("chain id lock") = chain;
    *LOCAL_NODE.lock().expect("local node lock") = None;
    VERIFIED_HEIGHT.lock().expect("verified height lock").clear();
    CHECKED_HEIGHT.lock().expect("checked height lock").clear();
    *GROUP.lock().expect("group lock") = group;
    let n = nodes.len() as u32;
    *NODES.lock().expect("nodes lock") = Some(nodes);
    drop(cached);
    Ok(n)
}

/// Pin the committee identity (hex, printed by `aether dkg`) that finality
/// certificates must verify under. Required outside dev mode (see `use_devnet_keys`).
#[uniffi::export]
pub fn set_committee_identity(identity_hex: String) -> R<()> {
    ValidatorSet::from_hex(&identity_hex).map_err(|e| WalletError::Invalid(format!("identity: {e}")))?;
    *COMMITTEE.lock().expect("committee lock") = Some(identity_hex);
    Ok(())
}

fn trusted_set(validators: u32) -> R<ValidatorSet> {
    let group = *GROUP.lock().expect("group lock");
    match COMMITTEE.lock().expect("committee lock").clone() {
        Some(hex) => ValidatorSet::from_hex(&hex).map_err(|e| WalletError::Invalid(format!("identity: {e}"))).map(|s| s.with_group(group)),
        // Dev mode only, and only because someone asked for it: the devnet key
        // is public, so a chain built with it proves nothing by itself.
        None if devnet_keys() => Ok(ValidatorSet::devnet(validators as u64).with_group(group)),
        None => Err(WalletError::Verification(
            "no committee identity is pinned, so nothing can be verified: pass network.json with \"identity\" to configure_network (development: use_devnet_keys)".into(),
        )),
    }
}

/// Balance and nonce, verified against a validator-signed state root.
#[uniffi::export]
pub fn verified_account(address: String, validators: u32) -> R<VerifiedAccount> {
    demote_on_failure(|| verified_account_at(address, validators))
}

fn verified_account_at(address: String, validators: u32) -> R<VerifiedAccount> {
    let a: Address = address.parse().map_err(|_| WalletError::Invalid("address".into()))?;
    let set = trusted_set(validators)?;
    let v = call("aether_getAccount", json!([a]))?;
    let proof: Proof = parse(&v["proof"], "proof")?;
    let height = v["height"].as_u64().unwrap_or_default();
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

/// A contract call or deployment a web page asked for (`aether://call`),
/// signed by the Secure Enclave key. `to` empty deploys `data` as init code.
#[uniffi::export]
pub fn prepare_call(p256_public_key: Vec<u8>, to: String, value_wei: String, data_hex: String, gas_limit: u64) -> R<PreparedTx> {
    const MAX_GAS: u64 = 10_000_000;
    let to: Option<Address> = if to.is_empty() { None } else { Some(to.parse().map_err(|_| WalletError::Invalid("contract address".into()))?) };
    let value: U256 = if value_wei.is_empty() { U256::ZERO } else { value_wei.parse().map_err(|_| WalletError::Invalid("value".into()))? };
    let input = alloy_primitives::hex::decode(data_hex.trim_start_matches("0x")).map_err(|_| WalletError::Invalid("call data is not hex".into()))?;
    let gas_limit = if gas_limit == 0 { 3_000_000 } else { gas_limit.min(MAX_GAS) };
    prepare(&p256_public_key, |_| Ok(EvmCall { to, value, input: input.clone().into(), gas_limit, delegate: None }))
}

/// One recipient of a batch.
#[derive(uniffi::Record)]
pub struct Payment {
    pub to: String,
    pub value_wei: String,
}

/// Several payments, all or nothing, under ONE signature (one Touch ID).
/// The account delegates to EastSeaAccount (EIP-7702) in the same tx the first
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
    let balance_hex = call("eth_getBalance", json!([from]))?;
    let balance = balance_hex.as_str().and_then(|h| U256::from_str_radix(h.trim_start_matches("0x"), 16).ok());
    let (max_fee, tip) = fee_caps(&status, balance);
    check_paid_state_balance(max_fee.state, balance)?;
    let nonce_hex = call("eth_getTransactionCount", json!([from]))?;
    let nonce = u64::from_str_radix(nonce_hex.as_str().unwrap_or("0x0").trim_start_matches("0x"), 16).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let call_body = body(from)?;
    let payload = call_body.encode();
    let group = *GROUP.lock().expect("group lock");
    let header = TxHeader {
        chain_id,
        sender: from,
        nonce,
        // Every interpreted instruction costs at least 1 gas, so prove steps <= gas_limit.
        gas: GasVector {
            exec: call_body.gas_limit,
            state: aether_execution::recommended_state_budget(&call_body, balance, max_fee.state),
            prove: call_body.gas_limit,
        },
        max_fee,
        tip,
        payload_commitment: aether_execution::tx::payload_commitment(&payload),
        scheme: SignerScheme::P256,
        // Group 0 keeps Secure Enclave v2 signing bytes; a split group signs
        // the group's v3 envelope.
        group: (group != 0).then_some(group),
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

/// How long a prepared free-lane registration stays valid (2 h of 1 s blocks):
/// the block that carries it must not be past this height.
const REGISTRATION_TTL: u64 = 7_200;

/// Register this Mac as a voting-node candidate, operated by the wallet's account.
/// `device_token` is Apple's DeviceCheck token (base64): one Mac, one candidate.
/// `ownership` is the voting key's own signature (`aether candidate-info
/// --operator`), so nobody can register a voting key they do not hold.
/// The registrar (a validator holding the network's DeviceCheck key) attests.
///
/// On a network with the free registration lane (`aether_status`'s
/// `free_registration`, docs/design/22-gas-pool.md 2층) the returned
/// `envelope_json` is a lane item instead of a transaction: no gas, no balance
/// needed, one signature from this wallet (the Secure Enclave signs the same
/// `signing_message` field). Older networks get the paid contract call.
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
    let pk = p256_key(&p256_public_key)?;
    let operator = address_of(&pk).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let a = registrar_call(json!([device_token, operator, hex_lower(&key), hex_lower(&node), beaconer, ownership]))?;
    let (r, s) = (hex32(a["r"].as_str().unwrap_or_default(), "attestation r")?, hex32(a["s"].as_str().unwrap_or_default(), "attestation s")?);
    let attestation = [r, s].concat();
    let status = call("aether_status", json!([]))?;
    if status["free_registration"].as_bool().unwrap_or(false) {
        let chain_id = expected_chain(&status)?;
        let height = status["height"].as_u64().unwrap_or_default();
        let nonce = call("aether_registrationNonce", json!([operator]))?.as_u64().unwrap_or_default();
        let expiry = height.saturating_add(REGISTRATION_TTL);
        let signing_message = aether_execution::registry::relay_message(chain_id, operator, &key, &node, beaconer, &attestation, nonce, expiry);
        // The item's canonical serialization (what the node re-parses and the
        // block carries); the signature field is filled by `submit_signed`.
        let item = aether_light::block::NodeRegistration {
            operator,
            validator_key: key.into(),
            node_id: node.into(),
            beaconer,
            attestation: Bytes::from(attestation).0,
            signature: Default::default(),
            operator_key: Bytes::from(pk.bytes.clone()).0,
            nonce,
            expiry,
        };
        return Ok(PreparedTx {
            from: operator.to_checksum(None),
            nonce,
            signing_message,
            envelope_json: json!({ "free_registration": { "chain_id": chain_id, "item": item } }).to_string(),
        });
    }
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

/// Attach a Secure Enclave signature (raw r‖s, 64 bytes) and submit: a signed
/// transaction, or the wallet's signature on a prepared free-lane registration
/// (the `envelope_json` of `prepare_register_node` on a lane network).
#[uniffi::export]
pub fn submit_signed(envelope_json: String, signature: Vec<u8>, p256_public_key: Vec<u8>) -> R<String> {
    if let Ok(v) = serde_json::from_str::<Value>(&envelope_json) {
        if v.get("free_registration").is_some() {
            return submit_registration(v, signature, p256_public_key);
        }
    }
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

/// Sign a prepared free-lane registration with the wallet key and send it to
/// the lane (`aether_sendRegistration`): the same checks `submit_signed` does,
/// then the item goes out with its signature attached. Returns the item's id —
/// `receipt` polls it like a tx hash.
fn submit_registration(prepared: Value, signature: Vec<u8>, p256_public_key: Vec<u8>) -> R<String> {
    let lane = &prepared["free_registration"];
    let chain_id = lane["chain_id"].as_u64().ok_or_else(|| WalletError::Invalid("registration chain id".into()))?;
    let mut item: aether_light::block::NodeRegistration =
        serde_json::from_value(lane["item"].clone()).map_err(|e| WalletError::Invalid(format!("registration: {e}")))?;
    let pk = p256_key(&p256_public_key)?;
    if address_of(&pk).map_err(|e| WalletError::Invalid(e.to_string()))? != item.operator {
        return Err(WalletError::Invalid("this key is not the registration's operator".into()));
    }
    let sig = p256::ecdsa::Signature::from_slice(&signature).map_err(|_| WalletError::Invalid("signature must be 64-byte r‖s".into()))?;
    // Secure Enclave may return high-s; the chain only accepts low-s.
    let bytes = sig.normalize_s().to_bytes().to_vec();
    let msg = aether_execution::registry::relay_message(
        chain_id,
        item.operator,
        &item.validator_key.0,
        &item.node_id.0,
        item.beaconer,
        &item.attestation,
        item.nonce,
        item.expiry,
    );
    aether_crypto::verify(&pk, &msg, &bytes).map_err(|e| WalletError::Invalid(format!("signature does not match key: {e}")))?;
    item.signature = Bytes::from(bytes).0;
    let v = call("aether_sendRegistration", json!([item]))?;
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
        state_fee_wei: u256_of_json(&v["receipt"]["state_fee"]).unwrap_or(U256::ZERO).to_string(),
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

/// Node-sourced, paginated account activity. Balances remain certificate
/// verified; this JSON is display data and includes `history_start`.
#[uniffi::export]
pub fn account_history(address: String, cursor: Option<String>, limit: u32) -> R<String> {
    let address: Address = address.parse().map_err(|_| WalletError::Invalid("address".into()))?;
    if !(1..=200).contains(&limit) { return Err(WalletError::Invalid("limit must be 1..200".into())); }
    let result = call("aether_accountHistory", json!([address, cursor, limit]))?;
    Ok(result.to_string())
}

/// One page of an address's reward records (`aether_rewardsPage`), newest
/// first, with the cursor that continues older and the total count — so the
/// wallet can load every reward and say "N of M" instead of silently keeping
/// only the newest 1,000.
#[uniffi::export]
pub fn rewards_page(address: String, cursor: Option<String>, limit: u32) -> R<String> {
    let address: Address = address.parse().map_err(|_| WalletError::Invalid("address".into()))?;
    if !(1..=10_000).contains(&limit) { return Err(WalletError::Invalid("limit must be 1..10000".into())); }
    let result = call("aether_rewardsPage", json!([address, cursor, limit]))?;
    Ok(result.to_string())
}

/// One account-history row, as far as the balance breakdown cares. Fields the
/// node adds later are ignored; fields the wallet needs default to nothing.
#[derive(serde::Deserialize)]
struct HistoryRow {
    #[serde(default)]
    direction: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    value_wei: String,
    #[serde(default)]
    fee_wei: String,
    success: bool,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    native_received_wei: Option<String>,
    #[serde(default)]
    native_payout_source: Option<String>,
}

fn wei_of(s: &str, what: &str) -> R<U256> {
    U256::from_str_radix(s.trim(), 10).map_err(|_| WalletError::Invalid(format!("history row {what} is not wei: {s:?}")))
}

fn same_address(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// "Where does my balance come from?", answered in exact wei. `entries_json`
/// is every account-history row of one address (the wallet pages until the
/// cursor is exhausted), `balance_wei` the certificate-verified balance, and
/// `faucet`/`waeth` the addresses this network's grants and wrapped-coin
/// payouts come from (nil when there is no such contract). Returns per-source
/// totals and the difference between the itemized net and the verified
/// balance: zero when every coin is accounted for, the honest remainder
/// (pruned history, rows not yet loaded) when it is not. Integer math only —
/// no floating point anywhere in the reconciliation.
#[uniffi::export]
pub fn balance_sources(entries_json: String, balance_wei: String, faucet: Option<String>, waeth: Option<String>) -> R<String> {
    let rows: Vec<HistoryRow> = serde_json::from_str(&entries_json)
        .map_err(|e| WalletError::Invalid(format!("history rows are not readable: {e}")))?;
    let balance = wei_of(&balance_wei, "balance")?;
    let mut proof_rewards = U256::ZERO;
    let mut node_rewards = U256::ZERO;
    let mut faucet_in = U256::ZERO;
    let mut received = U256::ZERO;
    let mut unwrapped = U256::ZERO;
    let mut sent = U256::ZERO;
    let mut fees = U256::ZERO;
    for row in &rows {
        let fee = wei_of(&row.fee_wei, "fee")?;
        fees = fees.saturating_add(fee);
        match row.kind.as_str() {
            "proof_reward" => {
                proof_rewards = proof_rewards.saturating_add(wei_of(&row.value_wei, "value")?);
                continue;
            }
            "node_reward" => {
                node_rewards = node_rewards.saturating_add(wei_of(&row.value_wei, "value")?);
                continue;
            }
            _ => {}
        }
        // A wrapped-coin payout the wallet's own contract list vouches for.
        // Anything else claiming to pay native coin stays out of the sums, so
        // it surfaces in the not-yet-itemized difference instead of silently
        // padding "received".
        if let (Some(amount), Some(source)) = (row.native_received_wei.as_deref(), row.native_payout_source.as_deref()) {
            if waeth.as_deref().is_some_and(|w| same_address(source, w)) {
                unwrapped = unwrapped.saturating_add(wei_of(amount, "native payout")?);
            }
        }
        let value = wei_of(&row.value_wei, "value")?;
        if !row.success {
            continue; // Nothing moved; the fee above is all it cost.
        }
        if row.direction == "in" && row.kind == "native_transfer" {
            if faucet.as_deref().is_some_and(|f| row.from.as_deref().is_some_and(|from| same_address(from, f))) {
                faucet_in = faucet_in.saturating_add(value);
            } else {
                received = received.saturating_add(value);
            }
        } else if row.direction == "out" {
            sent = sent.saturating_add(value);
        }
    }
    let total_in = proof_rewards.saturating_add(node_rewards).saturating_add(faucet_in).saturating_add(received).saturating_add(unwrapped);
    let total_out = sent.saturating_add(fees);
    // balance + total_out − total_in, kept exact even when the rows over- or
    // under-count the balance (pruned history under, mid-index over).
    let difference;
    let mut itemizes = false;
    if total_in <= balance.saturating_add(total_out) {
        difference = balance.saturating_add(total_out).saturating_sub(total_in).to_string();
        itemizes = difference == "0";
    } else {
        difference = format!("-{}", total_in.saturating_sub(balance).saturating_sub(total_out));
    }
    Ok(json!({
        "proof_rewards_wei": proof_rewards.to_string(),
        "node_rewards_wei": node_rewards.to_string(),
        "faucet_wei": faucet_in.to_string(),
        "received_wei": received.to_string(),
        "unwrapped_wei": unwrapped.to_string(),
        "sent_wei": sent.to_string(),
        "fees_wei": fees.to_string(),
        "total_in_wei": total_in.to_string(),
        "total_out_wei": total_out.to_string(),
        "balance_wei": balance.to_string(),
        "difference_wei": difference,
        "itemizes_completely": itemizes,
        "rows": rows.len(),
    })
    .to_string())
}

#[cfg(test)]
mod account_history_tests {
    use super::*;

    #[test]
    fn history_is_a_follower_read_and_rejects_bad_inputs_locally() {
        assert!(!needs_a_validator("aether_accountHistory"));
        assert!(!needs_a_validator("aether_rewardsPage"));
        assert!(account_history("not an address".into(), None, 50).is_err());
        assert!(account_history(format!("{:#x}", Address::ZERO), None, 201).is_err());
        assert!(rewards_page(format!("{:#x}", Address::ZERO), None, 0).is_err());
        assert!(rewards_page(format!("{:#x}", Address::ZERO), None, 10_001).is_err());
    }

    /// The founder's testnet balance, in miniature: rewards, a faucet grant,
    /// a transfer out and its fee must add up to the verified balance exactly.
    #[test]
    fn balance_sources_itemizes_every_wei() {
        let entries = r#"[
            {"kind":"proof_reward","direction":"in","value_wei":"1000000000000000000","fee_wei":"0","success":true},
            {"kind":"proof_reward","direction":"in","value_wei":"500000000000000000","fee_wei":"0","success":true},
            {"kind":"node_reward","direction":"in","value_wei":"2000000000000000000","fee_wei":"0","success":true},
            {"kind":"native_transfer","direction":"in","from":"0x00000000000000000000000000000000000fauc0","value_wei":"1000000000000000000","fee_wei":"0","success":true},
            {"kind":"native_transfer","direction":"in","from":"0x1111000000000000000000000000000000001111","value_wei":"300000000000000000","fee_wei":"0","success":true},
            {"kind":"native_transfer","direction":"out","to":"0x2222000000000000000000000000000000002222","value_wei":"100000000000000000","fee_wei":"1500000000000000","success":true},
            {"kind":"contract_call","direction":"out","to":"0x3333000000000000000000000000000000003333","value_wei":"50000000000000000","fee_wei":"2500000000000000","success":true},
            {"kind":"native_transfer","direction":"out","to":"0x4444000000000000000000000000000000004444","value_wei":"900000000000000000","fee_wei":"750000000000000","success":false},
            {"kind":"native_transfer","direction":"in","from":"0xwaethcontract0000000000000000000000000000","value_wei":"0","fee_wei":"0","native_received_wei":"700000000000000000","native_payout_source":"0xwaethcontract0000000000000000000000000000","success":true}
        ]"#;
        let out = balance_sources(
            entries.to_string(),
            // 1.0 + 0.5 + 2.0 + 1.0 + 0.3 + 0.7 − 0.1 − 0.05 − fees(0.0015+0.0025+0.00075)
            "5345250000000000000".to_string(),
            Some("0x00000000000000000000000000000000000fauc0".into()),
            Some("0xwaethcontract0000000000000000000000000000".into()),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["proof_rewards_wei"], "1500000000000000000");
        assert_eq!(v["node_rewards_wei"], "2000000000000000000");
        assert_eq!(v["faucet_wei"], "1000000000000000000");
        assert_eq!(v["received_wei"], "300000000000000000");
        assert_eq!(v["unwrapped_wei"], "700000000000000000");
        assert_eq!(v["sent_wei"], "150000000000000000");
        assert_eq!(v["fees_wei"], "4750000000000000");
        assert_eq!(v["difference_wei"], "0");
        assert_eq!(v["itemizes_completely"], true);
        assert_eq!(v["rows"], 9);
    }

    /// Pruned or not-yet-loaded history leaves an honest gap, never a silent one.
    #[test]
    fn balance_sources_reports_what_is_not_itemized() {
        let entries = r#"[
            {"kind":"proof_reward","direction":"in","value_wei":"1000000000000000000","fee_wei":"0","success":true}
        ]"#;
        let out = balance_sources(entries.to_string(), "3000000000000000000".to_string(), None, None).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["difference_wei"], "2000000000000000000");
        assert_eq!(v["itemizes_completely"], false);
        // Rows that claim more than the balance has report a negative gap
        // rather than wrapping around.
        let over = balance_sources(entries.to_string(), "100000000000000000".to_string(), None, None).unwrap();
        let v: Value = serde_json::from_str(&over).unwrap();
        assert_eq!(v["difference_wei"], "-900000000000000000");
        assert_eq!(v["itemizes_completely"], false);
    }

    #[test]
    fn balance_sources_rejects_inputs_it_cannot_sum_exactly() {
        assert!(balance_sources("not json".into(), "1".into(), None, None).is_err());
        assert!(balance_sources("[]".into(), "0x10".into(), None, None).is_err());
        assert!(balance_sources(r#"[{"kind":"native_transfer","direction":"in","value_wei":"1.5","fee_wei":"0","success":true}]"#.into(), "2".into(), None, None).is_err());
    }
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

    #[test]
    fn paid_state_transactions_require_a_native_balance_before_signing() {
        assert!(check_paid_state_balance(1_000_000_000_000, Some(U256::ZERO)).is_err());
        assert!(check_paid_state_balance(1_000_000_000_000, None).is_err());
        assert!(check_paid_state_balance(0, Some(U256::ZERO)).is_ok(), "legacy free transfers remain possible");
        assert!(check_paid_state_balance(0, None).is_ok(), "legacy missing-balance behavior remains unchanged");
        assert!(check_paid_state_balance(1_000_000_000_000, Some(U256::from(1))).is_ok());
    }

    #[test]
    fn wallet_plain_transfer_fits_persistent_byte_quote() {
        use aether_types::Canonical;
        let signer = aether_crypto::P256Signer::from_seed(&[7u8; 32]).unwrap();
        let call = EvmCall {
            to: Some(Address::repeat_byte(0x42)),
            value: U256::from(u128::MAX),
            input: Bytes::new(),
            gas_limit: 21_000,
            delegate: None,
        };
        let tx = aether_execution::sign_call_group(&signer, u64::MAX, u64::MAX, u128::MAX, u16::MAX, &call).unwrap();
        assert!(tx.to_canonical_bytes().len() + 128 <= 32 * 32);
    }

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
    fn eth_call_params_check_inputs() {
        let p = eth_call_params("0x6BC5DED76CCBDC8DF35E7CD28B68FED245A74416", "0x70a08231").unwrap();
        assert_eq!(p[0]["to"], "0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416");
        assert_eq!(p[0]["data"], "0x70a08231");
        assert_eq!(p[1], "latest");
        assert_eq!(eth_call_params("0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416", "dbb80e42").unwrap()[0]["data"], "0xdbb80e42");
        assert!(eth_call_params("0x1234", "0x").is_err());
        assert!(eth_call_params("0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416", "0xabc").is_err());
        assert!(eth_call_params("0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416", "0xzz").is_err());
    }

    #[test]
    fn address_is_same_for_compressed_and_uncompressed_keys() {
        let sk = p256::ecdsa::SigningKey::from_slice(&[9u8; 32]).unwrap();
        let c = sk.verifying_key().to_sec1_point(true).as_bytes().to_vec();
        let u = sk.verifying_key().to_sec1_point(false).as_bytes().to_vec();
        assert_eq!(account_address(c).unwrap(), account_address(u).unwrap());
    }

    // ---------------- network configuration and verification guards ----------------

    /// The wallet's configuration is process-global, so these tests take turns.
    fn config() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().expect("test lock")
    }

    fn reset_network() {
        *COMMITTEE.lock().expect("committee lock") = None;
        *GROUP.lock().expect("group lock") = 0;
        DEVNET_KEYS.store(false, std::sync::atomic::Ordering::Relaxed);
        *CHAIN_ID.lock().expect("chain id lock") = 7_777;
        VERIFIED_HEIGHT.lock().expect("verified height lock").clear();
        CHECKED_HEIGHT.lock().expect("checked height lock").clear();
        // The client, its pinned servers and its failure bookkeeping are as
        // process-global as the rest: every test starts from none of them.
        *NET.lock().expect("network lock") = None;
        *PINNED_SERVERS.lock().expect("pinned servers lock") = None;
        *NODES.lock().expect("nodes lock") = None;
        *LOCAL_NODE.lock().expect("local node lock") = None;
        REMOTE_FAILURES.store(0, std::sync::atomic::Ordering::Relaxed);
        *LAST_REBUILD.lock().expect("rebuild lock") = None;
        REBUILDS.store(0, std::sync::atomic::Ordering::Relaxed);
        *BUDGETS.lock().expect("budgets lock") = None;
    }

    /// A validator that never answers: TEST-NET-1 (packets go nowhere), so
    /// every connect attempt runs its full budget — the incident network of
    /// 2026-10-05, where the validators had all restarted on new addresses.
    fn dead_validator() -> PinnedServer {
        PinnedServer { node: "ae7b4bb59d9c18f830fed15a23fafec37029ee1de4df398b2154f312b9c1a23e".into(), socket: "192.0.2.1:1".into() }
    }

    /// The incident of 2026-10-05, reproduced: no validator answers, and the
    /// read still returns in its budget instead of parking the caller's
    /// thread inside the FFI call for minutes (`recent_blocks`,
    /// `connection`, `verified_account` were all stuck exactly there).
    #[test]
    fn remote_reads_are_bounded_when_nothing_answers() {
        let _g = config();
        reset_network();
        pin_servers(vec![], vec![dead_validator()]).unwrap();
        *BUDGETS.lock().expect("budgets lock") = Some(Budgets {
            net_build: Duration::from_secs(5),
            read: Duration::from_millis(700),
            remote_check: Duration::from_millis(500),
            rebuild_after: u32::MAX,
            rebuild_every: Duration::from_secs(30),
        });
        let t = std::time::Instant::now();
        let err = chain_status().err().expect("a dead network fails the read").to_string();
        assert!(
            t.elapsed() < Duration::from_secs(8),
            "a read against a dead network returned in {:?}, not minutes",
            t.elapsed()
        );
        assert!(err.contains("timed out") || err.contains("connect"), "{err}");
    }

    /// A client that keeps failing is stuck on stale addresses: after enough
    /// consecutive failures it is dropped and rebuilt — a different client,
    /// which redoes discovery — rather than re-dialing the same dead list
    /// forever. (Rebuild disabled in the bounded test above so the two stay
    /// independent; here it is the thing under test.)
    #[test]
    fn a_stuck_client_is_rebuilt_after_repeated_failures() {
        let _g = config();
        reset_network();
        pin_servers(vec![], vec![dead_validator()]).unwrap();
        *BUDGETS.lock().expect("budgets lock") = Some(Budgets {
            net_build: Duration::from_secs(5),
            read: Duration::from_millis(300),
            remote_check: Duration::from_millis(300),
            rebuild_after: 2,
            rebuild_every: Duration::from_millis(50),
        });
        let before = net().unwrap();
        for _ in 0..2 {
            assert!(chain_status().is_err(), "a dead network fails the read");
        }
        assert_eq!(
            REBUILDS.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "two consecutive failed reads rebuilt the stuck client"
        );
        let after = net().unwrap();
        assert!(
            !std::sync::Arc::ptr_eq(&before, &after),
            "the rebuild is a fresh client, not the same one re-dialed"
        );
    }

    /// The watchdog's remote view answers quickly even when the validators
    /// are unreachable: the route decision polls it, so it must say "unknown"
    /// in seconds — the wallet then reads through its local node with the
    /// check marked pending instead of camping on "Connecting".
    #[test]
    fn authenticated_remote_height_answers_fast_when_validators_are_gone() {
        let _g = config();
        reset_network();
        use_devnet_keys(); // so the refresh gets as far as the network call
        pin_servers(vec![], vec![dead_validator()]).unwrap();
        *BUDGETS.lock().expect("budgets lock") = Some(Budgets {
            net_build: Duration::from_secs(5),
            read: Duration::from_millis(500),
            remote_check: Duration::from_millis(400),
            rebuild_after: u32::MAX,
            rebuild_every: Duration::from_secs(30),
        });
        let t = std::time::Instant::now();
        assert_eq!(authenticated_remote_height().unwrap(), None);
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "the remote check took {:?} against a dead network",
            t.elapsed()
        );
    }

    fn network_json(identity: Option<&str>, devnet: bool) -> String {
        json!({
            "chain_id": 7_780,
            "validators": [{ "node": "ae7b4bb59d9c18f830fed15a23fafec37029ee1de4df398b2154f312b9c1a23e" }],
            "identity": identity,
            "devnet": devnet,
        })
        .to_string()
    }

    /// Without a committee identity nothing can be verified: the public devnet
    /// key is never a silent fallback (audit 3), only an explicit dev mode.
    #[test]
    fn verification_refuses_to_run_without_an_identity() {
        let _g = config();
        reset_network();
        let err = trusted_set(4).map(|_| ()).unwrap_err().to_string();
        assert!(err.contains("identity"), "{err}");
        // An explicit dev mode brings the devnet key back, nothing else does.
        use_devnet_keys();
        assert_eq!(trusted_set(4).unwrap().identity_hex(), ValidatorSet::devnet(4).identity_hex());
        reset_network();
        assert!(trusted_set(4).is_err());
        // A pinned identity is used whatever the mode.
        set_committee_identity(ValidatorSet::devnet(3).identity_hex()).unwrap();
        assert_eq!(trusted_set(4).unwrap().identity_hex(), ValidatorSet::devnet(3).identity_hex());
        assert!(set_committee_identity("not hex".into()).is_err(), "a bad identity is refused");
    }

    /// `configure_network` refuses a network.json with no identity outside dev
    /// mode, and accepts the devnet key only behind the explicit flag.
    #[test]
    fn configure_network_requires_an_identity_outside_dev_mode() {
        let _g = config();
        reset_network();
        let err = configure_network(network_json(None, false)).unwrap_err().to_string();
        assert!(err.contains("identity"), "{err}");
        assert_eq!(configure_network(network_json(None, true)).unwrap(), 1);
        assert!(devnet_keys(), "\"devnet\": true switches dev mode on");
        assert!(trusted_set(4).is_ok());
        reset_network();
        let id = ValidatorSet::devnet(4).identity_hex();
        assert_eq!(configure_network(network_json(Some(&id), false)).unwrap(), 1);
        assert_eq!(*CHAIN_ID.lock().expect("chain id lock"), 7_780, "the chain id from network.json is applied");
        assert_eq!(trusted_set(4).unwrap().identity_hex(), id);
        assert!(!devnet_keys());
        assert!(configure_network("{\"validators\": []}".to_string()).is_err(), "no validators either");
    }

    #[test]
    fn reconfiguration_sets_and_resets_the_consensus_group() {
        let _g = config();
        reset_network();
        let mut network: Value = serde_json::from_str(&network_json(None, true)).unwrap();
        network["group"] = json!(7);
        configure_network(network.to_string()).unwrap();
        assert_eq!(trusted_set(4).unwrap().group(), 7);
        network["group"] = json!(u16::MAX as u64 + 1);
        assert!(configure_network(network.to_string()).is_err());
        assert_eq!(trusted_set(4).unwrap().group(), 7, "invalid configuration changes nothing");
        configure_network(network_json(None, true)).unwrap();
        assert_eq!(trusted_set(4).unwrap().group(), 0);
    }

    #[test]
    fn reconfiguration_drops_old_identity_and_verification_state() {
        let _g = config();
        reset_network();
        configure_network(network_json(None, true)).unwrap();
        remember_height(7_780, 20).unwrap();
        use_local_node(Some(18545));
        assert_eq!(authenticated_remote_height().unwrap(), None);
        let id = ValidatorSet::devnet(4).identity_hex();
        configure_network(network_json(Some(&id), false)).unwrap();
        assert!(!devnet_keys());
        assert_eq!(verified_height(), 0);
        assert_eq!(*LOCAL_NODE.lock().unwrap(), None);
        assert_eq!(trusted_set(4).unwrap().identity_hex(), id);
    }

    /// Finalized heights never go back: an anchor below the highest verified is
    /// a replay of old state (audit 3), whatever certificate it comes with.
    #[test]
    fn verified_heights_only_move_forward() {
        let _g = config();
        reset_network();
        assert_eq!(verified_height(), 0);
        remember_height(7_777, 10).unwrap();
        remember_height(7_777, 10).unwrap(); // the same height again is fine (a paused chain)
        remember_height(7_777, 11).unwrap();
        assert_eq!(verified_height(), 11);
        let err = remember_height(7_777, 9).unwrap_err().to_string();
        assert!(err.contains("never go back"), "{err}");
        // Each chain keeps its own floor.
        remember_height(7_780, 3).unwrap();
        assert_eq!(verified_height(), 11);
        assert!(remember_height(7_780, 2).is_err());
    }

    /// Red team #2/#17: a stale certificate cannot drive a local-node
    /// restart or keep the wallet on an obsolete local route.
    #[test]
    fn remote_watchdog_height_requires_a_recent_certificate() {
        let _g = config();
        let chain = 9_990_017;
        remember_checked_height(chain, 42);
        assert_eq!(recent_checked_height(chain), Some(42));
        let mut checked = CHECKED_HEIGHT.lock().unwrap();
        let (_, _, at) = checked.iter_mut().find(|(id, _, _)| *id == chain).unwrap();
        *at = std::time::Instant::now() - Duration::from_secs(31);
        drop(checked);
        assert_eq!(recent_checked_height(chain), None);
    }

    /// The 10-minute recency guard, kept as an extra check on top of the above.
    #[test]
    fn stale_anchors_are_refused() {
        let now = now_ms();
        check_freshness(now).unwrap();
        check_freshness(now - MAX_ANCHOR_AGE_MS).unwrap();
        let err = check_freshness(now - MAX_ANCHOR_AGE_MS - 1).unwrap_err().to_string();
        assert!(err.contains("stale"), "{err}");
        check_freshness(now + 60_000).unwrap(); // a node's clock a little ahead, not our problem
    }

    /// An anchor far behind the newest finalized height a validator's own
    /// certificate showed is refused, however fresh its timestamp looks (a
    /// follower can be slow but never certify anything newer than the
    /// committee did); a small natural lag is fine, another chain is not
    /// bound by it, and an aged checked height enforces nothing.
    #[test]
    fn anchors_far_behind_a_verified_finalized_height_are_stale() {
        let _g = config();
        reset_network();
        remember_checked_height(7_777, 1_000);
        check_lag(7_777, 1_000).unwrap();
        check_lag(7_777, 1_000 - MAX_FOLLOWER_LAG).unwrap();
        let err = check_lag(7_777, 1_000 - MAX_FOLLOWER_LAG - 1).unwrap_err().to_string();
        assert!(err.contains("stale"), "{err}");
        check_lag(7_780, 0).unwrap();

        // The checked height never goes back: a validator replaying an old
        // certificate cannot lower the bar.
        remember_checked_height(7_777, 900);
        assert!(check_lag(7_777, 900 - MAX_FOLLOWER_LAG - 1).is_err(), "still the height of block 1,000");

        // Too old to trust the bar by: nothing is enforced on it.
        CHECKED_HEIGHT.lock().expect("checked height lock").iter_mut().for_each(|(_, _, at)| *at = std::time::Instant::now() - CHECKED_HEIGHT_MAX_AGE - Duration::from_secs(1));
        check_lag(7_777, 0).unwrap();
    }

    /// The chain id a certified block commits to (its transactions) is checked
    /// against the wallet's: another network's state must not pass as verified,
    /// even under a committee key that also verifies there.
    #[test]
    fn anchor_blocks_must_carry_the_configured_chain() {
        use aether_light::block::Block;
        use commonware_codec::Encode;
        let empty = Block::genesis(7, alloy_primitives::B256::repeat_byte(1));
        assert!(committed_chain_ids(&empty.encode()).unwrap().is_empty(), "an empty block carries no chain id");
        let with_tx = |chain_id: u64| {
            let mut p = empty.payload().unwrap();
            p.txs.push(TxEnvelope {
                header: TxHeader {
                    chain_id,
                    sender: Address::ZERO,
                    nonce: 0,
                    gas: GasVector::default(),
                    max_fee: FeeVector::default(),
                    tip: 0,
                    payload_commitment: aether_execution::tx::payload_commitment(&[]),
                    scheme: SignerScheme::P256,
                    group: None,
                },
                payload: TxPayload::Plain(Bytes::new()),
                signature: Bytes::new(),
            });
            Block::new(empty.context.clone(), empty.parent, empty.height, empty.timestamp, p.to_bytes()).encode().to_vec()
        };
        let (mine, other) = (with_tx(7), with_tx(8));
        check_anchor_chain(&mine, &[], 7).unwrap();
        assert_eq!(committed_chain_ids(&other).unwrap(), vec![8]);
        let err = check_anchor_chain(&other, &[], 7).unwrap_err().to_string();
        assert!(err.contains("chain 8"), "{err}");
        // The blocks a certificate reaches back through (links) are checked too.
        let err = check_anchor_chain(&mine, &[other], 7).unwrap_err().to_string();
        assert!(err.contains("chain 8"), "{err}");
    }

    // ---------------- transfer fee quotes (audit 6, A6-7) ----------------

    /// A plain transfer's displayed fee (audit 6, A6-7) carries, on a
    /// paid-state genesis, a possible new recipient account, a possible
    /// first-use sender account and the persisted bytes (A6-1, A6-2); it is
    /// always a maximum, and a recipient with a certified account drops only
    /// its account charge. The legacy 7780 chain (no state price) charges none.
    #[test]
    fn transfer_quotes_price_possible_new_accounts_and_persisted_bytes() {
        let fee = |q: &TransferQuote| q.fee_wei.parse::<u128>().unwrap();
        let legacy = json!({ "base_fee": { "exec": "0", "prove": "0" } });
        let q = quote_from_status(&legacy, None);
        assert_eq!(fee(&q), 0);
        assert_eq!(q.new_recipient_charge_wei, "0");

        let paid = json!({ "base_fee": { "exec": "0", "state": "1000000000000", "prove": "0" } });
        let q = quote_from_status(&paid, None);
        assert_eq!(fee(&q), 232_000_000_000_000, "recipient 100 + sender 100 + bytes 32 units");
        assert_eq!(q.new_recipient_charge_wei.parse::<u128>().unwrap(), 100_000_000_000_000);
        assert!(q.fee_is_maximum);
        let q = quote_from_status(&paid, Some(true));
        assert_eq!(fee(&q), 132_000_000_000_000, "an existing recipient pays no account charge");
        assert_eq!(q.new_recipient_charge_wei, "0");
        assert!(q.fee_is_maximum, "sender account and bytes stay upper bounds");

        let busy = json!({ "base_fee": { "exec": "1000000000", "state": "1000000000000", "prove": "0" } });
        assert_eq!(fee(&quote_from_status(&busy, Some(false))), 42_000_000_000_000 + 232_000_000_000_000);
    }

    /// The signed caps must accept the state budget a funded transfer needs
    /// (audit 6, A6-7): the state cap at the fixed unit price, so a new
    /// recipient's 100-unit reserve passes `check_budget`'s price floor.
    #[test]
    fn fee_caps_reserve_the_fixed_state_price_for_new_recipients() {
        use aether_execution::{fees::STATE_UNIT_PRICE, fees::STATE_ACCOUNT_UNITS, recommended_state_budget};
        let status = json!({ "base_fee": { "exec": "0", "prove": "0" } });
        let (caps, _tip) = fee_caps(&status, Some(U256::from(1_000u64)));
        assert_eq!(caps.state, STATE_UNIT_PRICE, "the state cap must meet the fixed state price floor");
        // The budget a funded transfer then signs (tx.rs) covers a new account.
        let call = EvmCall {
            to: Some(Address::ZERO),
            value: U256::from(1u64),
            input: Bytes::new(),
            gas_limit: 21_000,
            delegate: None,
        };
        // (A6-1/A6-2 add the possible first-use sender account and persisted bytes on top.)
        assert!(recommended_state_budget(&call, Some(U256::from(1u64)), caps.state) >= STATE_ACCOUNT_UNITS);
    }

    /// The receipt's actual burned state fee, so the app can replace the
    /// "maximum" quote with what the send really cost.
    #[test]
    fn receipt_state_fee_parses_decimal_and_hex_strings() {
        assert_eq!(u256_of_json(&json!("100000000000000")), Some(U256::from(100_000_000_000_000u64)));
        assert_eq!(u256_of_json(&json!("0x5af3107a4000")), Some(U256::from(100_000_000_000_000u64)));
        assert_eq!(u256_of_json(&json!("0")), Some(U256::ZERO));
        assert_eq!(u256_of_json(&json!(0)), Some(U256::ZERO));
        assert_eq!(u256_of_json(&Value::Null), None);
        assert_eq!(u256_of_json(&json!("zz")), None);
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
    demote_on_failure(|| verified_slot_at(account, slot, set))
}

fn verified_slot_at(account: Address, slot: U256, set: &ValidatorSet) -> R<U256> {
    let v = call("aether_getStorage", json!([account, slot]))?;
    let proof: Proof = parse(&v["proof"], "proof")?;
    let height = v["height"].as_u64().unwrap_or_default();
    let anchor = anchor(height, set)?;
    aether_light::verify_storage(&anchor, &account, slot, &proof).map_err(|e| WalletError::Verification(format!("account storage: {e}")))
}

/// ReleaseLog's `Entry[] public entries` starts at storage slot 0. Each
/// fixed-size Entry occupies four slots at keccak256(slot 0) + index * 4.
fn release_slots(index: u64) -> R<[U256; 5]> {
    let offset = index.checked_mul(4).ok_or_else(|| WalletError::Invalid("release index too large".into()))?;
    let base = U256::from_be_bytes(alloy_primitives::keccak256([0u8; 32]).0) + U256::from(offset);
    Ok([U256::ZERO, base, base + U256::from(1), base + U256::from(2), base + U256::from(3)])
}

fn release_metadata(meta: U256, height: u64) -> R<(u64, u64, bool)> {
    let published_block = (meta & U256::from(u64::MAX)).to::<u64>();
    let published_at = ((meta >> 64usize) & U256::from(u64::MAX)).to::<u64>();
    let emergency_byte = ((meta >> 128usize) & U256::from(0xff)).to::<u8>();
    if published_block == 0 || published_block > height || published_at == 0 || emergency_byte > 1 || meta >> 136usize != U256::ZERO {
        return Err(WalletError::Verification("invalid release publication metadata".into()));
    }
    Ok((published_block, published_at, emergency_byte == 1))
}

/// Read a ReleaseLog entry. Every slot must come from one state height, and
/// every EIP-7864 proof is checked under the same finality certificate.
#[uniffi::export]
pub fn verified_release(contract: String, code_hash: String, index: u64, validators: u32) -> R<VerifiedRelease> {
    // The index comes from an untrusted distribution sidecar. A missing entry
    // must not park an honest follower as a liar for six hours.
    verified_release_at(contract, code_hash, index, validators)
}

fn verified_release_at(contract: String, code_hash: String, index: u64, validators: u32) -> R<VerifiedRelease> {
    let address: Address = contract.parse().map_err(|_| WalletError::Invalid("release log address".into()))?;
    let expected_hash: aether_types::B256 = code_hash.parse().map_err(|_| WalletError::Invalid("release log code hash".into()))?;
    let set = trusted_set(validators)?;
    let slots = release_slots(index)?;
    let replies = slots.iter().map(|s| call("aether_getStorage", json!([address, s]))).collect::<R<Vec<_>>>()?;
    let code = call("aether_getCodeHash", json!([address]))?;
    let height = replies[0]["height"].as_u64().ok_or_else(|| WalletError::Verification("release state height missing".into()))?;
    if replies.iter().any(|v| v["height"].as_u64() != Some(height)) || code["height"].as_u64() != Some(height) {
        return Err(WalletError::Verification("release storage slots came from different blocks".into()));
    }
    let anchor = anchor(height, &set)?;
    let code_proof: Proof = parse(&code["proof"], "code hash proof")?;
    if aether_light::verify_code_hash(&anchor, &address, &code_proof)
        .map_err(|e| WalletError::Verification(format!("release code proof: {e}")))? != Some(expected_hash) {
        return Err(WalletError::Verification("the ReleaseLog runtime code hash is not pinned".into()));
    }
    let values = replies.iter().zip(slots.iter()).map(|(v, slot)| {
        let proof: Proof = parse(&v["proof"], "proof")?;
        aether_light::verify_storage(&anchor, &address, *slot, &proof)
            .map_err(|e| WalletError::Verification(format!("release storage proof: {e}")))
    }).collect::<R<Vec<_>>>()?;
    if U256::from(index) >= values[0] {
        return Err(WalletError::Verification("release entry does not exist".into()));
    }
    let (published_block, published_at, emergency) = release_metadata(values[4], height)?;
    Ok(VerifiedRelease {
        manifest_sha256: format!("{:064x}", values[1]),
        archive_sha256: format!("{:064x}", values[2]),
        signatures_sha256: format!("{:064x}", values[3]),
        published_block,
        published_at,
        emergency,
        state_height: height,
        certified_block: anchor.height,
        certified_timestamp_ms: anchor.timestamp_ms,
    })
}

#[cfg(test)]
mod release_tests {
    use super::*;

    #[test]
    fn release_slots_follow_solidity_dynamic_array_layout() {
        let first = release_slots(0).unwrap();
        let second = release_slots(1).unwrap();
        assert_eq!(first[0], U256::ZERO);
        assert_eq!(first[1], U256::from_be_bytes(alloy_primitives::keccak256([0u8; 32]).0));
        assert_eq!(second[1], first[1] + U256::from(4));
        assert_eq!(second[4], second[1] + U256::from(3));
        assert!(release_slots(u64::MAX).is_err());
    }

    #[test]
    fn release_metadata_rejects_future_and_noncanonical_values() {
        let meta = U256::from(50) | (U256::from(1_000_000) << 64usize) | (U256::from(1) << 128usize);
        assert_eq!(release_metadata(meta, 50).unwrap(), (50, 1_000_000, true));
        assert!(release_metadata(meta, 49).is_err());
        assert!(release_metadata(meta | (U256::from(2) << 128usize), 50).is_err());
        assert!(release_metadata(meta | (U256::from(1) << 200usize), 50).is_err());
    }

    #[test]
    fn release_storage_rejects_a_tampered_rpc_proof() {
        let fixture: Value = serde_json::from_str(include_str!("../../light/tests/fixtures/devnet4.json")).unwrap();
        let block = from_hex(fixture["anchor_block"].as_str().unwrap()).unwrap();
        let finalization = from_hex(fixture["anchor_finalization"].as_str().unwrap()).unwrap();
        let anchor = verify_finalized_chain(&ValidatorSet::devnet(4), &block, &finalization, &[]).unwrap();
        let address: Address = fixture["address"].as_str().unwrap().parse().unwrap();
        let wrong_proof: Proof = serde_json::from_value(fixture["proof"].clone()).unwrap();
        assert!(aether_light::verify_storage(&anchor, &address, release_slots(0).unwrap()[1], &wrong_proof).is_err());
        assert!(aether_light::verify_code_hash(&anchor, &address, &wrong_proof).is_err());
    }

    #[test]
    fn release_list_slots_verify_against_the_certified_state_root() {
        use aether_state::layout::{code_hash_key, storage_slot_key};
        use aether_state::StateRepository;
        let address: Address = "0x0000000000000000000000000000000000007704".parse().unwrap();
        let mut state = aether_execution::WorldState::default();
        state.set_code(address, vec![0x00].into()).unwrap();
        let slots = release_slots(0).unwrap();
        let expected = [U256::from(1), U256::from(11), U256::from(22), U256::from(33),
            U256::from(100) | (U256::from(1_000_000) << 64usize)];
        for (slot, value) in slots.iter().zip(expected) {
            state.set_storage(address, *slot, value);
        }
        let anchor = VerifiedBlock { height: 201, digest: String::new(), timestamp_ms: 1_000_001_000,
            parent_state_root: state.root(), history_root: aether_types::B256::ZERO };
        let repo = state.repo();
        let code = repo.prove(&[code_hash_key(repo.hasher(), &address)]).remove(0);
        assert_eq!(aether_light::verify_code_hash(&anchor, &address, &code).unwrap(), Some(state.code_hash(&address)));
        for (slot, value) in slots.iter().zip(expected) {
            let proof = repo.prove(&[storage_slot_key(repo.hasher(), &address, *slot)]).remove(0);
            assert_eq!(aether_light::verify_storage(&anchor, &address, *slot, &proof).unwrap(), value);
            let mut tampered = proof.clone();
            tampered.value = Some(U256::from(999).to_be_bytes::<32>());
            assert!(aether_light::verify_storage(&anchor, &address, *slot, &tampered).is_err());
        }
    }
}

/// Make the device with `recovery_code` able to recover this account after a
/// 48-hour delay that this account can cancel (delegates to EastSeaAccount first
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

/// Revoke every session of this account with one owner-authenticated tx.
#[uniffi::export]
pub fn prepare_stop_sessions(owner_public_key: Vec<u8>, validators: u32) -> R<PreparedTx> {
    let owner = address_of(&p256_key(&owner_public_key)?).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let set = trusted_set(validators)?;
    let count = verified_slot(owner, slots::session_count(), &set)?.to::<u64>();
    if count == 0 { return Err(WalletError::Invalid("no active session".into())); }
    let calls: Vec<aether_execution::AccountCall> = (0..count).map(|_| (owner, U256::ZERO, acct::encode_remove_session(0))).collect();
    prepare(&owner_public_key, |from| Ok(EvmCall {
        to: Some(from), value: U256::ZERO, input: aether_execution::encode_execute(&calls),
        gas_limit: 200_000 + 70_000 * count, delegate: Some(aether_execution::AETHER_ACCOUNT),
    }))
}

#[derive(uniffi::Record)]
pub struct SessionTokenStatus {
    pub per_payment: String,
    pub per_day: String,
    pub left_now: String,
}

#[uniffi::export]
pub fn session_token_status(account: String, token: String, validators: u32) -> R<SessionTokenStatus> {
    let a: Address = account.parse().map_err(|_| WalletError::Invalid("account address".into()))?;
    let token: Address = token.parse().map_err(|_| WalletError::Invalid("token address".into()))?;
    let set = trusted_set(validators)?;
    if verified_slot(a, slots::session_count(), &set)?.is_zero() { return Err(WalletError::Invalid("no session".into())); }
    let id = verified_slot(a, slots::session(0) + U256::from(7u64), &set)?.to::<u64>();
    let base = slots::session_token(id, token);
    let (per_payment, per_day) = slots::unpack_limits(verified_slot(a, base, &set)?);
    let usage = verified_slot(a, base + U256::from(1u64), &set)?;
    let bytes = usage.to_be_bytes::<32>();
    let day = u64::from_be_bytes(bytes[24..32].try_into().expect("day"));
    let spent = u128::from_be_bytes(bytes[8..24].try_into().expect("spent"));
    let prev = verified_slot(a, base + U256::from(2u64), &set)?.to::<u128>();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
    Ok(SessionTokenStatus { per_payment: per_payment.to_string(), per_day: per_day.to_string(),
        left_now: slots::left_now(per_day, day, spent, prev, now).to_string() })
}

/// Add or replace a listed token's on-chain caps without renewing the session.
#[uniffi::export]
pub fn prepare_set_session_token(owner_public_key: Vec<u8>, token: String, per_payment: String, per_day: String, validators: u32) -> R<PreparedTx> {
    let t: Address = token.parse().map_err(|_| WalletError::Invalid("token address".into()))?;
    let p = per_payment.parse::<u128>().map_err(|_| WalletError::Invalid("per payment".into()))?;
    let d = per_day.parse::<u128>().map_err(|_| WalletError::Invalid("per day".into()))?;
    if p == 0 || p > d { return Err(WalletError::Invalid("need 0 < per-payment <= per-day".into())); }
    let owner = address_of(&p256_key(&owner_public_key)?).map_err(|e| WalletError::Invalid(e.to_string()))?;
    let set = trusted_set(validators)?;
    if verified_slot(owner, slots::session_count(), &set)?.is_zero() { return Err(WalletError::Invalid("no session".into())); }
    let calls = [(owner, U256::ZERO, acct::encode_set_session_token(0, t, p, d))];
    prepare(&owner_public_key, |from| Ok(EvmCall { to: Some(from), value: U256::ZERO,
        input: aether_execution::encode_execute(&calls), gas_limit: 300_000, delegate: Some(aether_execution::AETHER_ACCOUNT) }))
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

#[derive(uniffi::Record)]
pub struct SessionTokenRequest {
    pub account: String,
    pub token: String,
    pub to: String,
    pub amount: String,
    pub nonce: u64,
    pub message: Vec<u8>,
}

fn token_call(token: &str, to: &str, amount: &str) -> R<aether_execution::AccountCall> {
    let t: Address = token.parse().map_err(|_| WalletError::Invalid("token".into()))?;
    let recipient: Address = to.parse().map_err(|_| WalletError::Invalid("recipient".into()))?;
    let value: U256 = amount.parse().map_err(|_| WalletError::Invalid("token amount".into()))?;
    if value.is_zero() { return Err(WalletError::Invalid("token amount".into())); }
    Ok((t, U256::ZERO, acct::encode_token_transfer(recipient, value)))
}

#[uniffi::export]
pub fn prepare_session_token_payment(account: String, token: String, to: String, amount: String, validators: u32) -> R<SessionTokenRequest> {
    let a: Address = account.parse().map_err(|_| WalletError::Invalid("account".into()))?;
    let calls = [token_call(&token, &to, &amount)?];
    let set = trusted_set(validators)?;
    if verified_slot(a, slots::session_count(), &set)?.is_zero() { return Err(WalletError::Invalid("no session".into())); }
    let nonce = verified_slot(a, slots::session(0) + U256::from(6u64), &set)?.to::<u64>();
    let id = verified_slot(a, slots::session(0) + U256::from(7u64), &set)?.to::<u64>();
    let chain_id = expected_chain(&call("aether_status", json!([]))?)?;
    Ok(SessionTokenRequest { account, token, to, amount, nonce, message: acct::session_message(chain_id, a, id, nonce, &calls) })
}

#[uniffi::export]
pub fn prepare_session_token_submit(session_public_key: Vec<u8>, request: SessionTokenRequest, session_signature: Vec<u8>) -> R<PreparedTx> {
    let a: Address = request.account.parse().map_err(|_| WalletError::Invalid("account".into()))?;
    let calls = [token_call(&request.token, &request.to, &request.amount)?];
    let sig = normalize_p256(&session_signature)?;
    let input = acct::encode_session_execute(&calls, 0, sig[..32].try_into().expect("32"), sig[32..64].try_into().expect("32"));
    prepare(&session_public_key, |_| Ok(EvmCall { to: Some(a), value: U256::ZERO, input, gas_limit: 200_000, delegate: None }))
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
