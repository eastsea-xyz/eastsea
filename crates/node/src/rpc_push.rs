//! Bounded, node-local finality hints. Nothing here grants authority to spend
//! or install: wallet consumers still verify accounts/receipts/release approvals.
use crate::{chain::Chain, rpc::RpcState};
use aether_types::{Address, TxHash, B256};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};
use tokio::sync::{broadcast, watch, Notify};

pub const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 2;
pub const MAX_SUBSCRIPTIONS_PER_NODE: usize = 128;
pub const MAX_CONNECTIONS: usize = 128;
pub const MAX_FRAME_BYTES: usize = 256 * 1024;
pub const MAX_QUEUED_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_QUEUED_FRAMES: usize = 1024;
pub const MAX_CLIENT_FRAMES: usize = 32;
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const FINALITY_REFERENCES: usize = 64;
type RpcError = (i64, String);

pub struct Hub {
    finalized: broadcast::Sender<u64>,
    queues: Mutex<Queues>,
    sequence: AtomicU64,
    bytes: AtomicUsize,
    frames: AtomicUsize,
}
#[derive(Default)]
struct Queues {
    clients: BTreeMap<u64, Client>,
    subscriptions: usize,
    connections: usize,
}
struct Client {
    queue: VecDeque<Frame>,
    wake: Arc<Notify>,
    dropped: watch::Sender<Option<DropNotice>>,
    subscriptions: BTreeMap<String, bool>,
    height: u64,
    bytes: usize,
}
struct Frame {
    text: Option<String>,
    hub: Weak<Hub>,
    bytes: usize,
    end: bool,
}
#[derive(Clone)]
struct DropNotice {
    reason: &'static str,
    height: u64,
    subscriptions: BTreeMap<String, bool>,
}
impl Drop for Frame {
    fn drop(&mut self) {
        drop(self.text.take());
        if let Some(hub) = self.hub.upgrade() {
            // An in-flight frame keeps its lease even after eviction. Dropping
            // a queue never re-enters its mutex or prematurely credits bytes.
            hub.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
            hub.frames.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
impl Default for Hub {
    fn default() -> Self {
        Self {
            finalized: broadcast::channel(FINALITY_REFERENCES).0,
            queues: Mutex::new(Queues::default()),
            sequence: AtomicU64::new(1),
            bytes: AtomicUsize::new(0),
            frames: AtomicUsize::new(0),
        }
    }
}
impl Queues {
    fn remove(&mut self, id: u64, reason: &'static str) -> VecDeque<Frame> {
        if let Some(c) = self.clients.remove(&id) {
            self.subscriptions -= c.subscriptions.len();
            c.dropped.send_replace(Some(DropNotice {
                reason,
                height: c.height,
                subscriptions: c.subscriptions,
            }));
            c.wake.notify_one();
            c.queue
        } else {
            VecDeque::new()
        }
    }
}
impl Hub {
    /// Only a height reference, sent after the durable finalized commit.
    pub(crate) fn publish(&self, height: u64) {
        let _ = self.finalized.send(height);
    }
    fn connect(&self) -> Option<(u64, Arc<Notify>, watch::Receiver<Option<DropNotice>>)> {
        let mut q = self.queues.lock().unwrap();
        if q.connections >= MAX_CONNECTIONS {
            return None;
        }
        q.connections += 1;
        let id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let wake = Arc::new(Notify::new());
        let (dropped, receiver) = watch::channel(None);
        q.clients.insert(
            id,
            Client {
                queue: VecDeque::new(),
                wake: wake.clone(),
                dropped,
                subscriptions: BTreeMap::new(),
                height: 0,
                bytes: 0,
            },
        );
        Some((id, wake, receiver))
    }
    fn disconnect(&self, id: u64, reason: &'static str) {
        let frames = self.queues.lock().unwrap().remove(id, reason);
        drop(frames);
    }
    fn release_connection(&self, id: u64) {
        self.disconnect(id, "disconnected");
        // Closing writers still occupy a connection until their task finishes.
        self.queues.lock().unwrap().connections -= 1;
    }
    fn subscribe(&self, id: u64, wallet: bool) -> Result<String, RpcError> {
        let mut q = self.queues.lock().unwrap();
        let c = q
            .clients
            .get(&id)
            .ok_or_else(|| (-32002, "connection closed".into()))?;
        if c.subscriptions.len() >= MAX_SUBSCRIPTIONS_PER_CONNECTION
            || q.subscriptions >= MAX_SUBSCRIPTIONS_PER_NODE
        {
            return Err((-32002, "subscription limit (2/connection, 128/node)".into()));
        }
        let key = format!("0x{:x}", self.sequence.fetch_add(1, Ordering::Relaxed));
        q.clients
            .get_mut(&id)
            .unwrap()
            .subscriptions
            .insert(key.clone(), wallet);
        q.subscriptions += 1;
        Ok(key)
    }
    fn unsubscribe(&self, id: u64, key: &str) {
        let mut q = self.queues.lock().unwrap();
        if let Some(c) = q.clients.get_mut(&id) {
            if c.subscriptions.remove(key).is_some() {
                q.subscriptions -= 1;
            }
        }
    }
    fn pop(&self, id: u64) -> Option<Frame> {
        let mut q = self.queues.lock().unwrap();
        let c = q.clients.get_mut(&id)?;
        let frame = c.queue.pop_front()?;
        c.bytes -= frame.bytes;
        Some(frame)
    }
    /// Never awaits: evict the largest queued backlog, freeing only frames
    /// actually destroyed. In-flight writes remain charged until they finish.
    fn enqueue(self: &Arc<Self>, id: u64, value: Value) -> bool {
        self.enqueue_frame(id, value, false)
    }
    fn finish(self: &Arc<Self>, id: u64, value: Value) -> bool {
        self.enqueue_frame(id, value, true)
    }
    fn enqueue_frame(self: &Arc<Self>, id: u64, value: Value, end: bool) -> bool {
        let height = value["params"]["result"]["height"].as_u64();
        let text = value.to_string();
        if text.len() > MAX_FRAME_BYTES {
            self.disconnect(id, "frame_too_large");
            return false;
        }
        let mut q = self.queues.lock().unwrap();
        while self.bytes.load(Ordering::Acquire) + text.len() + FINALITY_REFERENCES * 8
            > MAX_QUEUED_BYTES
            || self.frames.load(Ordering::Acquire) + FINALITY_REFERENCES >= MAX_QUEUED_FRAMES
        {
            let Some(slowest) = q
                .clients
                .iter()
                .filter(|(_, c)| !c.queue.is_empty())
                .max_by_key(|(_, c)| (c.bytes, c.queue.len()))
                .map(|(id, _)| *id)
            else {
                let frames = q.remove(id, "queue_overflow");
                drop(q);
                drop(frames);
                return false;
            };
            let frames = q.remove(slowest, "queue_overflow");
            drop(q);
            drop(frames);
            q = self.queues.lock().unwrap();
        }
        let Some(c) = q.clients.get_mut(&id) else {
            return false;
        };
        if c.queue.len() >= MAX_CLIENT_FRAMES {
            let frames = q.remove(id, "slow_reader");
            drop(q);
            drop(frames);
            return false;
        }
        if let Some(height) = height {
            c.height = height;
        }
        let size = text.len();
        self.bytes.fetch_add(size, Ordering::AcqRel);
        self.frames.fetch_add(1, Ordering::AcqRel);
        c.bytes += size;
        c.queue.push_back(Frame {
            text: Some(text),
            hub: Arc::downgrade(self),
            bytes: size,
            end,
        });
        c.wake.notify_one();
        true
    }
}

pub(crate) async fn upgrade(State(st): State<RpcState>, ws: WebSocketUpgrade) -> Response {
    let hub = st.chain.lock().push.clone();
    let Some((client, frames, dropped)) = hub.connect() else {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "RPC stream connection limit",
        )
            .into_response();
    };
    // Guard also releases the slot if the HTTP upgrade fails before its task.
    let guard = Connection { hub, client };
    ws.max_message_size(MAX_FRAME_BYTES)
        .max_frame_size(MAX_FRAME_BYTES)
        .read_buffer_size(8 * 1024)
        .write_buffer_size(0)
        .max_write_buffer_size(MAX_FRAME_BYTES + 1024)
        .on_upgrade(move |socket| run(socket, st, guard, frames, dropped))
}
struct Connection {
    hub: Arc<Hub>,
    client: u64,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.hub.release_connection(self.client);
    }
}

#[derive(Default)]
struct LogFilter {
    addresses: Vec<Address>,
    topics: Vec<Option<Vec<B256>>>,
}
fn hex_value<T: serde::de::DeserializeOwned>(v: &Value, bytes: usize) -> Result<T, RpcError> {
    if !v
        .as_str()
        .is_some_and(|s| s.starts_with("0x") && s.len() == 2 + bytes * 2)
    {
        return Err((-32602, format!("expected {bytes}-byte 0x hex value")));
    }
    serde_json::from_value(v.clone()).map_err(|e| (-32602, e.to_string()))
}
impl LogFilter {
    fn parse(value: &Value) -> Result<Self, RpcError> {
        let obj = value
            .as_object()
            .ok_or_else(|| (-32602, "logs filter must be an object".into()))?;
        if obj.keys().any(|k| k != "address" && k != "topics") {
            return Err((
                -32602,
                "live logs filters support address/topics only".into(),
            ));
        }
        let mut filter = Self::default();
        match obj.get("address") {
            None | Some(Value::Null) => {}
            Some(Value::Array(values)) if values.len() <= 64 => {
                for v in values {
                    filter.addresses.push(hex_value(v, 20)?);
                }
            }
            Some(Value::String(_)) => filter.addresses.push(hex_value(&obj["address"], 20)?),
            _ => return Err((-32602, "at most 64 log addresses".into())),
        }
        match obj.get("topics") {
            None | Some(Value::Null) => {}
            Some(Value::Array(values)) if values.len() <= 4 => {
                for v in values {
                    filter.topics.push(match v {
                        Value::Null => None,
                        Value::String(_) => Some(vec![hex_value(v, 32)?]),
                        Value::Array(alternatives) if alternatives.len() <= 64 => {
                            if alternatives.iter().any(Value::is_null) {
                                None
                            } else {
                                Some(
                                    alternatives
                                        .iter()
                                        .map(|v| hex_value(v, 32))
                                        .collect::<Result<_, _>>()?,
                                )
                            }
                        }
                        _ => return Err((-32602, "at most 64 alternatives per log topic".into())),
                    });
                }
            }
            _ => return Err((-32602, "at most four positional log topics".into())),
        }
        Ok(filter)
    }
    fn matches(&self, address: Address, topics: &[B256]) -> bool {
        (self.addresses.is_empty() || self.addresses.contains(&address))
            && self.topics.iter().enumerate().all(|(i, want)| match want {
                None => true,
                Some(want) => topics.get(i).is_some_and(|topic| want.contains(topic)),
            })
    }
}
struct WalletFilter {
    address: Address,
    transactions: Vec<TxHash>,
    after: Option<u64>,
}
impl WalletFilter {
    fn parse(v: &Value) -> Result<Self, RpcError> {
        let obj = v
            .as_object()
            .ok_or_else(|| (-32602, "wallet filter must be an object".into()))?;
        if obj
            .keys()
            .any(|k| !["address", "transactions", "after"].contains(&k.as_str()))
        {
            return Err((-32602, "unsupported wallet filter".into()));
        }
        let address = hex_value(obj.get("address").unwrap_or(&Value::Null), 20)?;
        let transactions = match obj.get("transactions") {
            None => vec![],
            Some(Value::Array(values)) if values.len() <= 32 => values
                .iter()
                .map(|v| hex_value(v, 32))
                .collect::<Result<_, _>>()?,
            _ => return Err((-32602, "at most 32 tracked transaction hashes".into())),
        };
        let after = obj
            .get("after")
            .map(|v| {
                v.as_u64()
                    .ok_or_else(|| (-32602, "after must be an unsigned height".into()))
            })
            .transpose()?;
        Ok(Self {
            address,
            transactions,
            after,
        })
    }
}
enum Kind {
    Heads,
    Logs(LogFilter),
    Wallet(WalletFilter, Option<Value>, Option<u64>),
}
struct Subscription {
    kind: Kind,
    start: u64,
}
fn notification(id: &str, wallet: bool, result: Value) -> Value {
    json!({"jsonrpc":"2.0", "method": if wallet {"aether_subscription"} else {"eth_subscription"},
        "params":{"subscription":id, "result":result}})
}
fn gap(height: u64, reason: &str) -> Value {
    json!({"kind":"gap", "height":height, "reason":reason})
}

fn heads(chain: &Chain, height: u64) -> Option<Value> {
    let g = chain.lock();
    let b = g.blocks.get(&height)?;
    Some(
        json!({"number":format!("0x{height:x}"), "hash":format!("0x{}", b.hash),
        "parentHash":format!("0x{}", b.parent), "stateRoot":b.state_root,
        "timestamp":format!("0x{:x}", b.timestamp_ms / 1000),
        "gasUsed":format!("0x{:x}", b.gas_used), "gasLimit":format!("0x{:x}", g.cfg.limits.exec),
        "baseFeePerGas":format!("0x{:x}", b.base_fee.exec)}),
    )
}
fn logs(chain: &Chain, height: u64, filter: &LogFilter) -> Result<Vec<Value>, &'static str> {
    let b = chain
        .lock()
        .blocks
        .get(&height)
        .ok_or("history_unavailable")?
        .clone();
    let executed = chain.executed_at(height).ok_or("history_unavailable")?;
    let receipts = &executed.receipts;
    let mut out = Vec::new();
    let mut log_index = 0;
    let mut bytes = 0;
    for (ti, receipt) in receipts.iter().enumerate() {
        for event in &receipt.events {
            let index = log_index;
            log_index += 1;
            if !filter.matches(event.address, &event.topics) {
                continue;
            }
            // Bound before hex encoding and building an aggregate of matches.
            bytes += event.data.len() * 2 + event.topics.len() * 68 + 512;
            if event.data.len() * 2 + 1024 > MAX_FRAME_BYTES
                || bytes > MAX_QUEUED_BYTES
                || out.len() >= MAX_QUEUED_FRAMES
            {
                return Err("frame_too_large");
            }
            out.push(json!({"address":event.address, "topics":event.topics, "data":format!("0x{}", hex::encode(&event.data)),
                "blockNumber":format!("0x{height:x}"), "blockHash":format!("0x{}", b.hash),
                "transactionHash":b.txs[ti], "transactionIndex":format!("0x{ti:x}"),
                "logIndex":format!("0x{index:x}"), "removed":false}));
        }
    }
    Ok(out)
}
fn wallet(
    chain: &Chain,
    height: u64,
    filter: &WalletFilter,
    previous: &mut Option<Value>,
    release_height: &mut Option<u64>,
    force: bool,
) -> Value {
    let mut g = chain.lock();
    let f = g.finalized.clone();
    let transactions = filter
        .transactions
        .iter()
        .map(|h| {
            let state = if let Some((height, r)) = g.receipts.get(h) {
                json!({"state":"included", "height":height, "success":r.success})
            } else if let Some(tx) = g.mempool.get(h) {
                json!({"state":"pending", "waiting":crate::chain::pending_facts(&g, tx).reason()})
            } else if let Some(reason) = g.tombstones.get(h) {
                json!({"state":"dropped", "reason":reason})
            } else {
                Value::Null
            };
            json!({"hash":h, "status":state})
        })
        .collect::<Vec<_>>();
    let notices = g.upgrade_notices.clone();
    drop(g);
    let release = json!({"schedule":f.schedule.iter().map(|a| json!([a.protocol,a.at])).collect::<Vec<_>>(), "notices":notices});
    let published_topic = alloy_primitives::keccak256(b"Published(uint256,bytes32,bytes,bytes)");
    let executed = chain.executed_at(height);
    let published = executed.as_ref().is_some_and(|block| {
        block.receipts.iter().any(|r| {
            r.events.iter().any(|e| {
                e.address == aether_execution::release_log::ADDRESS
                    && e.topics.first() == Some(&published_topic)
            })
        })
    });
    let transfer_topic = alloy_primitives::keccak256(b"Transfer(address,address,uint256)");
    // Candidate dirtiness only: token recognition and balances still come
    // from the wallet's existing asset policy and readers.
    let token_transfer = executed.as_ref().is_some_and(|block| {
        block.receipts.iter().any(|r| {
            r.events.iter().any(|e| {
                e.topics.len() == 3
                    && e.data.len() == 32
                    && e.topics[0] == transfer_topic
                    && e.topics[1..].iter().any(|t| {
                        t.as_slice()[..12] == [0; 12]
                            && t.as_slice()[12..] == filter.address.as_slice()[..]
                    })
            })
        })
    });
    let stamp = json!({"balance":f.state.balance(&filter.address).to_string(), "nonce":f.state.nonce(&filter.address), "transactions":transactions, "release":release});
    let mut topics = vec!["head"];
    let old = previous.as_ref();
    if force
        || token_transfer
        || old.is_none_or(|o| o["balance"] != stamp["balance"] || o["nonce"] != stamp["nonce"])
    {
        topics.push("balance");
    }
    if force || old.is_none_or(|o| o["transactions"] != stamp["transactions"]) {
        topics.push("tx_status");
    }
    let changed_release = published || old.is_some_and(|o| o["release"] != stamp["release"]);
    if changed_release {
        *release_height = Some(height);
    }
    // Approval state anchored by the next certified block remains a dirty hint.
    if force || old.is_none() || changed_release || release_height.is_some_and(|h| height == h + 1)
    {
        topics.push("release");
    }
    *previous = Some(stamp);
    json!({"kind":"wallet", "height":height, "observed_height":f.height, "topics":topics})
}

async fn deliver(
    chain: &Chain,
    hub: &Arc<Hub>,
    client: u64,
    id: &str,
    sub: &mut Subscription,
    height: u64,
    force: bool,
) -> bool {
    match &mut sub.kind {
        Kind::Heads => match heads(chain, height) {
            Some(head) => hub.enqueue(client, notification(id, false, head)),
            None => {
                hub.disconnect(client, "history_unavailable");
                false
            }
        },
        Kind::Logs(filter) => match logs(chain, height, filter) {
            Ok(logs) => {
                for (i, log) in logs.into_iter().enumerate() {
                    if !hub.enqueue(client, notification(id, false, log)) {
                        return false;
                    }
                    if i % 8 == 7 {
                        tokio::task::yield_now().await;
                    }
                }
                true
            }
            Err(reason) => {
                hub.disconnect(client, reason);
                false
            }
        },
        Kind::Wallet(filter, stamp, release_height) => hub.enqueue(
            client,
            notification(
                id,
                true,
                wallet(chain, height, filter, stamp, release_height, force),
            ),
        ),
    }
}

async fn request(
    st: &RpcState,
    conn: &Connection,
    subscriptions: &mut BTreeMap<String, Subscription>,
    req: Value,
) {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = req.get("params").cloned().unwrap_or_else(|| json!([]));
    let subscribing =
        method == "eth_subscribe" || method == "aether_subscribe" || method == "eastsea_subscribe";
    if !subscribing {
        let answer = if [
            "eth_unsubscribe",
            "aether_unsubscribe",
            "eastsea_unsubscribe",
        ]
        .contains(&method)
        {
            match params
                .as_array()
                .filter(|p| p.len() == 1)
                .and_then(|p| p[0].as_str())
            {
                Some(key) => {
                    let removed = subscriptions.remove(key).is_some();
                    if removed {
                        conn.hub.unsubscribe(conn.client, key);
                    }
                    json!({"jsonrpc":"2.0", "id":id, "result":removed})
                }
                None => {
                    json!({"jsonrpc":"2.0", "id":id,"error":{"code":-32602,"message":"unsubscribe requires one subscription id"}})
                }
            }
        } else {
            crate::rpc::handle_value(st, req).await
        };
        conn.hub.enqueue(conn.client, answer);
        return;
    }
    let result = (|| -> Result<Kind, RpcError> {
        let p = params
            .as_array()
            .ok_or_else(|| (-32602, "subscription params must be an array".into()))?;
        match (method, p.first().and_then(Value::as_str), p.len()) {
            ("eth_subscribe", Some("newHeads"), 1) => Ok(Kind::Heads),
            ("eth_subscribe", Some("logs"), 1) => Ok(Kind::Logs(LogFilter::default())),
            ("eth_subscribe", Some("logs"), 2) => Ok(Kind::Logs(LogFilter::parse(&p[1])?)),
            ("aether_subscribe" | "eastsea_subscribe", Some("wallet"), 2)
                if !st.public_read_only =>
            {
                Ok(Kind::Wallet(WalletFilter::parse(&p[1])?, None, None))
            }
            _ => Err((
                -32602,
                "unsupported subscription (newHeads/logs or private wallet)".into(),
            )),
        }
    })();
    let result = result.and_then(|kind| {
        conn.hub
            .subscribe(conn.client, matches!(kind, Kind::Wallet(..)))
            .map(|id| (id, kind))
    });
    let (key, kind) = match result {
        Ok(v) => v,
        Err((code, message)) => {
            conn.hub.enqueue(
                conn.client,
                json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message}}),
            );
            return;
        }
    };
    let (head, floor) = {
        let g = st.chain.lock();
        (g.finalized.height, g.pruned_below.max(g.cache_below))
    };
    if !conn
        .hub
        .enqueue(conn.client, json!({"jsonrpc":"2.0", "id":id, "result":key}))
    {
        return;
    }
    let after = match &kind {
        Kind::Wallet(filter, ..) => filter.after,
        _ => None,
    };
    let mut sub = Subscription { kind, start: head };
    if let Some(after) = after {
        if after < floor || after > head || head - after > MAX_QUEUED_FRAMES as u64 {
            conn.hub.finish(
                conn.client,
                notification(&key, true, gap(head, "history_unavailable")),
            );
            return;
        }
        for height in after.saturating_add(1)..=head {
            if !st.chain.lock().blocks.contains_key(&height) {
                conn.hub.finish(
                    conn.client,
                    notification(&key, true, gap(head, "history_unavailable")),
                );
                return;
            }
            if !deliver(
                &st.chain,
                &conn.hub,
                conn.client,
                &key,
                &mut sub,
                height,
                true,
            )
            .await
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    }
    if matches!(sub.kind, Kind::Wallet(..)) && after.is_none_or(|h| h == head) {
        if !deliver(
            &st.chain,
            &conn.hub,
            conn.client,
            &key,
            &mut sub,
            head,
            true,
        )
        .await
        {
            return;
        }
    }
    subscriptions.insert(key, sub);
}

async fn run(
    socket: WebSocket,
    st: RpcState,
    conn: Connection,
    wake: Arc<Notify>,
    mut dropped: watch::Receiver<Option<DropNotice>>,
) {
    // This registration and finalized watermark share the commit lock.
    let (mut finalized, mut last_height) = {
        let g = st.chain.lock();
        (g.push.finalized.subscribe(), g.finalized.height)
    };
    let (mut sink, mut source) = socket.split();
    let mut writer_drop = dropped.clone();
    let hub = conn.hub.clone();
    let client = conn.client;
    let mut writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = writer_drop.changed() => {
                    let notice = writer_drop.borrow().clone();
                    if let Some(notice) = notice {
                        for (id, wallet) in notice.subscriptions {
                            let message = notification(&id, wallet, gap(notice.height, notice.reason)).to_string();
                            let _ = tokio::time::timeout(WRITE_TIMEOUT, sink.send(Message::Text(message.into()))).await;
                        }
                    }
                    let _ = tokio::time::timeout(WRITE_TIMEOUT, sink.close()).await;
                    return;
                },
                _ = wake.notified() => loop {
                    let Some(mut frame) = hub.pop(client) else { break };
                        let text = frame.text.take().unwrap();
                        if !matches!(tokio::time::timeout(WRITE_TIMEOUT, sink.send(Message::Text(text.into()))).await, Ok(Ok(()))) {
                            hub.disconnect(client, "slow_reader");
                            return;
                        }
                        if frame.end {
                            let _ = tokio::time::timeout(WRITE_TIMEOUT, sink.close()).await;
                            return;
                        }
                },
            }
        }
    });
    let mut subscriptions = BTreeMap::new();
    loop {
        tokio::select! {
            _ = &mut writer => return,
            _ = dropped.changed() => break,
            incoming = source.next() => match incoming {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<Value>(&text) {
                    Ok(Value::Array(batch)) if batch.is_empty() || batch.len() > crate::rpc::PUBLIC_MAX_BATCH => {
                        conn.hub.enqueue(conn.client, json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"stream batch requires 1..8 calls"}}));
                    },
                    Ok(Value::Array(batch)) => {
                        let changes_subscription = batch.iter().any(|req| matches!(req["method"].as_str(),
                            Some("eth_subscribe" | "eth_unsubscribe" | "aether_subscribe" | "aether_unsubscribe" | "eastsea_subscribe" | "eastsea_unsubscribe")));
                        if changes_subscription {
                            let errors = batch.iter().map(|req| json!({"jsonrpc":"2.0", "id":req.get("id").cloned().unwrap_or(Value::Null),
                                "error":{"code":-32602,"message":"subscription changes require individual requests"}})).collect::<Vec<_>>();
                            conn.hub.enqueue(conn.client, json!(errors));
                        } else {
                            let response = crate::rpc::handle_value(&st, json!(batch)).await;
                            conn.hub.enqueue(conn.client, response);
                        }
                    },
                    Ok(req) => request(&st, &conn, &mut subscriptions, req).await,
                    Err(_) => { conn.hub.enqueue(conn.client, json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"invalid JSON"}})); },
                },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                _ => break,
            },
            event = finalized.recv() => match event {
                Ok(height) if height <= last_height => {},
                Ok(height) => {
                    if height != last_height.saturating_add(1) { conn.hub.disconnect(conn.client, "finality_lag"); break; }
                    last_height = height;
                    for (id, sub) in &mut subscriptions {
                        if height > sub.start && !deliver(&st.chain, &conn.hub, conn.client, id, sub, height, false).await { break; }
                    }
                },
                Err(_) => { conn.hub.disconnect(conn.client, "finality_lag"); break; },
            },
        }
    }
    conn.hub.disconnect(conn.client, "disconnected");
    if tokio::time::timeout(WRITE_TIMEOUT * 2, &mut writer)
        .await
        .is_err()
    {
        writer.abort();
        let _ = writer.await;
    }
}

/// Deterministic queue qualification used inside the 4-validator integration
/// test. Compiled out of shipped binaries with the existing test-seam feature.
#[cfg(feature = "test-seam")]
pub fn assert_backpressure_isolated() {
    let hub = Arc::new(Hub::default());
    let (slow, _, slow_drop) = hub.connect().unwrap();
    let (fast, _, fast_drop) = hub.connect().unwrap();
    hub.subscribe(slow, true).unwrap();
    hub.subscribe(fast, true).unwrap();
    for _ in 0..MAX_CLIENT_FRAMES {
        assert!(hub.enqueue(slow, json!({"payload":"x".repeat(1024)})));
        assert!(hub.enqueue(fast, json!({"height":1})));
        drop(hub.pop(fast).unwrap());
    }
    assert!(!hub.enqueue(slow, json!({"height":2})));
    assert_eq!(slow_drop.borrow().as_ref().unwrap().reason, "slow_reader");
    assert!(fast_drop.borrow().is_none());
    assert!(hub.enqueue(fast, json!({"height":3})));
    drop(hub.pop(fast).unwrap());
    assert_eq!(hub.bytes.load(Ordering::Acquire), 0);
    assert_eq!(hub.frames.load(Ordering::Acquire), 0);
    assert_eq!(hub.queues.lock().unwrap().subscriptions, 1);

    let (largest, _, largest_drop) = hub.connect().unwrap();
    let (medium, _, medium_drop) = hub.connect().unwrap();
    for _ in 0..25 {
        assert!(hub.enqueue(largest, json!("x".repeat(120_000))));
    }
    for _ in 0..8 {
        assert!(hub.enqueue(medium, json!("x".repeat(120_000))));
    }
    assert!(hub.enqueue(fast, json!("x".repeat(250_000))));
    assert_eq!(
        largest_drop.borrow().as_ref().unwrap().reason,
        "queue_overflow"
    );
    assert!(medium_drop.borrow().is_none());
    assert!(fast_drop.borrow().is_none());
    assert!(hub.bytes.load(Ordering::Acquire) + FINALITY_REFERENCES * 8 <= MAX_QUEUED_BYTES);

    // Evicting an in-flight writer neither credits its memory early nor
    // admits an extra connection before its lifetime guard is released.
    let in_flight = hub.pop(fast).unwrap();
    let before = hub.bytes.load(Ordering::Acquire);
    let connections = hub.queues.lock().unwrap().connections;
    hub.disconnect(fast, "slow_reader");
    assert_eq!(hub.bytes.load(Ordering::Acquire), before);
    assert_eq!(hub.queues.lock().unwrap().connections, connections);
    drop(in_flight);
    assert_eq!(hub.bytes.load(Ordering::Acquire), before - 250_002);
    hub.release_connection(fast);
    assert_eq!(hub.queues.lock().unwrap().connections, connections - 1);
    hub.release_connection(slow);
    hub.release_connection(largest);
    hub.release_connection(medium);
    assert_eq!(hub.bytes.load(Ordering::Acquire), 0);

    let mut receiver = hub.finalized.subscribe();
    for height in 0..FINALITY_REFERENCES + 4 {
        hub.publish(height as u64);
    }
    assert!(matches!(
        receiver.try_recv(),
        Err(broadcast::error::TryRecvError::Lagged(_))
    ));
    assert_eq!(hub.frames.load(Ordering::Acquire), 0);
}
